import { A } from "@solidjs/router";
import { createQuery, createMutation, useQueryClient } from "@tanstack/solid-query";
import { createSignal, For, Show } from "solid-js";
import { api } from "~/api/client";
import { queryKeys } from "~/queries/keys";
import type { Project, CreateProjectInput } from "~/lib/sentry-types";
import { slugify } from "~/lib/formatters";
import Button from "~/components/ui/Button";
import LoadingSkeleton from "~/components/ui/LoadingSkeleton";
import EmptyState from "~/components/ui/EmptyState";

export default function SettingsProjects() {
  const queryClient = useQueryClient();
  const [name, setName] = createSignal("");
  const [slug, setSlug] = createSignal("");

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

  const handleSubmit = (e: SubmitEvent) => {
    e.preventDefault();
    if (!name().trim() || !slug().trim()) return;
    createMut.mutate({ name: name(), slug: slug() });
  };

  return (
    <div class="p-6">
      <h1 class="mb-6 text-2xl font-bold text-[var(--color-text-primary)]">
        Projects
      </h1>

      {/* Create form */}
      <div class="mb-8 rounded-lg border border-[var(--color-border)] bg-[var(--color-surface-1)] p-4">
        <h2 class="mb-3 text-sm font-medium text-[var(--color-text-primary)]">
          Create New Project
        </h2>
        <form onSubmit={handleSubmit} class="flex items-end gap-3">
          <div class="flex-1">
            <label class="mb-1 block text-xs text-[var(--color-text-secondary)]">
              Name
            </label>
            <input
              type="text"
              value={name()}
              onInput={(e) => handleNameChange(e.currentTarget.value)}
              placeholder="My Project"
              class="w-full rounded-md border border-[var(--color-border)] bg-[var(--color-surface-0)] px-3 py-2 text-sm text-[var(--color-text-primary)] placeholder:text-gray-400 focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
            />
          </div>
          <div class="flex-1">
            <label class="mb-1 block text-xs text-[var(--color-text-secondary)]">
              Slug
            </label>
            <input
              type="text"
              value={slug()}
              onInput={(e) => setSlug(e.currentTarget.value)}
              placeholder="my-project"
              class="w-full rounded-md border border-[var(--color-border)] bg-[var(--color-surface-0)] px-3 py-2 text-sm text-[var(--color-text-primary)] placeholder:text-gray-400 focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
            />
          </div>
          <Button type="submit" disabled={createMut.isPending || !name().trim()}>
            {createMut.isPending ? "Creating..." : "Create"}
          </Button>
        </form>
      </div>

      {/* Project list */}
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
          <div class="overflow-hidden rounded-lg border border-[var(--color-border)]">
            <table class="w-full">
              <thead>
                <tr class="border-b border-[var(--color-border)] bg-[var(--color-surface-1)]">
                  <th class="px-4 py-2 text-left text-xs font-medium text-[var(--color-text-secondary)]">
                    Name
                  </th>
                  <th class="px-4 py-2 text-left text-xs font-medium text-[var(--color-text-secondary)]">
                    Slug
                  </th>
                  <th class="px-4 py-2 text-left text-xs font-medium text-[var(--color-text-secondary)]">
                    Platform
                  </th>
                  <th class="px-4 py-2 text-right text-xs font-medium text-[var(--color-text-secondary)]">
                    Actions
                  </th>
                </tr>
              </thead>
              <tbody>
                <For each={projectsQuery.data}>
                  {(project) => (
                    <tr class="border-b border-[var(--color-border)]">
                      <td class="px-4 py-3 text-sm font-medium text-[var(--color-text-primary)]">
                        <A
                          href={`/settings/projects/${project.id}`}
                          class="hover:text-indigo-600 dark:hover:text-indigo-400"
                        >
                          {project.name}
                        </A>
                      </td>
                      <td class="px-4 py-3 text-sm text-[var(--color-text-secondary)]">
                        {project.slug}
                      </td>
                      <td class="px-4 py-3 text-sm text-[var(--color-text-secondary)]">
                        {project.platform ?? "-"}
                      </td>
                      <td class="px-4 py-3 text-right">
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
