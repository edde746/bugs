import { A } from "@solidjs/router";
import { createQuery, createMutation, useQueryClient } from "@tanstack/solid-query";
import { createSignal, For, Show } from "solid-js";
import { api } from "~/api/client";
import { queryKeys } from "~/queries/keys";
import type { Project, CreateProjectInput } from "~/lib/sentry-types";
import { slugify } from "~/lib/formatters";
import Button from "~/components/ui/Button";
import Modal from "~/components/ui/Modal";
import LoadingSkeleton from "~/components/ui/LoadingSkeleton";
import EmptyState from "~/components/ui/EmptyState";

export default function SettingsProjects() {
  const queryClient = useQueryClient();
  const [name, setName] = createSignal("");
  const [slug, setSlug] = createSignal("");
  const [showModal, setShowModal] = createSignal(false);

  const projectsQuery = createQuery(() => ({
    queryKey: queryKeys.projects.all(),
    queryFn: () => api.get<Project[]>("/internal/projects"),
  }));

  const createMut = createMutation(() => ({
    mutationFn: (input: CreateProjectInput) =>
      api.post<Project>("/internal/projects", input),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.projects.all() });
      setName("");
      setSlug("");
      setShowModal(false);
    },
  }));

  const deleteMut = createMutation(() => ({
    mutationFn: (id: number) => api.delete<void>(`/internal/projects/${id}`),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.projects.all() });
    },
  }));

  const handleNameChange = (value: string) => {
    setName(value);
    setSlug(slugify(value));
  };

  return (
    <div class="page">
      <div class="page__header">
        <h1 class="page__title">Projects</h1>
        <Button variant="primary" size="sm" onClick={() => setShowModal(true)}>
          Create Project
        </Button>
      </div>

      <Modal
        open={showModal()}
        onClose={() => setShowModal(false)}
        title="Create New Project"
        description="Set up a new project to start tracking errors."
        footer={
          <>
            <Button variant="secondary" size="sm" onClick={() => setShowModal(false)}>
              Cancel
            </Button>
            <Button
              variant="primary"
              size="sm"
              disabled={createMut.isPending || !name().trim()}
              onClick={() => {
                if (!name().trim() || !slug().trim()) return;
                createMut.mutate({ name: name(), slug: slug() });
              }}
            >
              {createMut.isPending ? "Creating..." : "Create"}
            </Button>
          </>
        }
      >
        <div class="form-stack">
          <div class="form-field">
            <label class="field-label">Name</label>
            <input
              type="text"
              value={name()}
              onInput={(e) => handleNameChange(e.currentTarget.value)}
              placeholder="My Project"
              class="input"
            />
          </div>
          <div class="form-field">
            <label class="field-label">Slug</label>
            <input
              type="text"
              value={slug()}
              onInput={(e) => setSlug(e.currentTarget.value)}
              placeholder="my-project"
              class="input"
            />
          </div>
        </div>
      </Modal>

      <Show when={!projectsQuery.isPending} fallback={<LoadingSkeleton rows={4} />}>
        <Show
          when={projectsQuery.data && projectsQuery.data.length > 0}
          fallback={
            <EmptyState
              title="No projects yet"
              description="Create your first project to start tracking errors."
            />
          }
        >
          <div class="card">
            <table class="data-table">
              <thead>
                <tr>
                  <th>Name</th>
                  <th>Slug</th>
                  <th>Platform</th>
                  <th data-align="right">Actions</th>
                </tr>
              </thead>
              <tbody>
                <For each={projectsQuery.data}>
                  {(project) => (
                    <tr>
                      <td>
                        <A href={`/settings/projects/${project.id}`}>
                          {project.name}
                        </A>
                      </td>
                      <td class="text-secondary">{project.slug}</td>
                      <td class="text-secondary">{project.platform ?? "-"}</td>
                      <td data-align="right">
                        <Button
                          variant="danger"
                          size="sm"
                          onClick={() => {
                            if (confirm(`Delete project "${project.name}"?`)) {
                              deleteMut.mutate(project.id);
                            }
                          }}
                        >
                          Delete
                        </Button>
                      </td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </div>
        </Show>
      </Show>
    </div>
  );
}
