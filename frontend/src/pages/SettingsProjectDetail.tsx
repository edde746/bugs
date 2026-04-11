import { A, useParams } from "@solidjs/router";
import { createQuery } from "@tanstack/solid-query";
import { createSignal, For, Show } from "solid-js";
import { api } from "~/api/client";
import { queryKeys } from "~/queries/keys";
import type { Project, ProjectKey } from "~/lib/sentry-types";
import Button from "~/components/ui/Button";
import LoadingSkeleton from "~/components/ui/LoadingSkeleton";
import IconArrowLeft from "~icons/lucide/arrow-left";
import IconClipboard from "~icons/lucide/clipboard-copy";
import IconCheck from "~icons/lucide/check";

export default function SettingsProjectDetail() {
  const params = useParams<{ projectId: string }>();

  const projectQuery = createQuery(() => ({
    queryKey: queryKeys.projects.detail(params.projectId),
    queryFn: () => api.get<Project>(`/internal/projects/${params.projectId}`),
  }));

  const keysQuery = createQuery(() => ({
    queryKey: queryKeys.projects.keys(params.projectId),
    queryFn: () =>
      api.get<ProjectKey[]>(`/internal/projects/${params.projectId}/keys`),
    enabled: !!projectQuery.data,
  }));

  const buildDsn = (publicKey: string, projectId: number) => {
    const { protocol, host } = window.location;
    return `${protocol}//${publicKey}@${host}/${projectId}`;
  };

  const [copiedId, setCopiedId] = createSignal<string | null>(null);

  const copyToClipboard = (text: string, id: string) => {
    navigator.clipboard.writeText(text).then(() => {
      setCopiedId(id);
      setTimeout(() => setCopiedId(null), 2000);
    });
  };

  return (
    <div class="page">
      <A href="/settings/projects" class="back-link">
        <IconArrowLeft /> Back to Projects
      </A>

      <Show when={projectQuery.data} fallback={<LoadingSkeleton rows={6} />}>
        {(project) => (
          <>
            <div style={{ "margin-bottom": "24px" }}>
              <h1 class="page__title">{project().name}</h1>
              <p class="page__subtitle">
                Slug: {project().slug} | Platform: {project().platform ?? "N/A"}{" "}
                | Created: {new Date(project().created_at).toLocaleDateString()}
              </p>
            </div>

            <div class="card">
              <div class="card__header" style={{ "flex-direction": "column", "align-items": "flex-start" }}>
                <h2>DSN Keys</h2>
                <p>Use these DSN values to configure your Sentry SDK client.</p>
              </div>
              <div class="card__body">
                <Show
                  when={keysQuery.data}
                  fallback={<LoadingSkeleton rows={2} />}
                >
                  {(keys) => (
                    <Show
                      when={keys().length > 0}
                      fallback={
                        <p class="text-secondary text-sm">
                          No DSN keys configured for this project.
                        </p>
                      }
                    >
                      <div style={{ display: "flex", "flex-direction": "column", gap: "16px" }}>
                        <For each={keys()}>
                          {(key) => (
                            <div class="dsn-card">
                              <div class="dsn-card__header">
                                <span class="dsn-card__label">
                                  {key.label || "Default"}
                                </span>
                                <span
                                  class="dsn-card__status"
                                  data-active={key.is_active}
                                >
                                  {key.is_active ? "Active" : "Inactive"}
                                </span>
                              </div>
                              <div class="dsn-card__field">
                                <label>Public Key</label>
                                <div class="dsn-card__field-value">
                                  <code>{key.public_key}</code>
                                  <Button
                                    variant="ghost"
                                    size="sm"
                                    onClick={() =>
                                      copyToClipboard(
                                        key.public_key,
                                        `key-${key.id}`,
                                      )
                                    }
                                  >
                                    {copiedId() === `key-${key.id}`
                                      ? <><IconCheck /> Copied!</>
                                      : <><IconClipboard /> Copy</>}
                                  </Button>
                                </div>
                              </div>
                              <div class="dsn-card__field">
                                <label>DSN</label>
                                <div class="dsn-card__field-value">
                                  <code>{buildDsn(key.public_key, key.project_id)}</code>
                                  <Button
                                    variant="ghost"
                                    size="sm"
                                    onClick={() =>
                                      copyToClipboard(
                                        buildDsn(key.public_key, key.project_id),
                                        `dsn-${key.id}`,
                                      )
                                    }
                                  >
                                    {copiedId() === `dsn-${key.id}`
                                      ? <><IconCheck /> Copied!</>
                                      : <><IconClipboard /> Copy</>}
                                  </Button>
                                </div>
                              </div>
                            </div>
                          )}
                        </For>
                      </div>
                    </Show>
                  )}
                </Show>
              </div>
            </div>
          </>
        )}
      </Show>
    </div>
  );
}
