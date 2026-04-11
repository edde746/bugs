import { A, useParams } from "@solidjs/router";
import { createQuery } from "@tanstack/solid-query";
import { createSignal, For, Show } from "solid-js";
import { api } from "~/api/client";
import { queryKeys } from "~/queries/keys";
import type { Project, ProjectKey } from "~/lib/sentry-types";
import Button from "~/components/ui/Button";
import LoadingSkeleton from "~/components/ui/LoadingSkeleton";

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

  const [copiedId, setCopiedId] = createSignal<string | null>(null);

  const copyToClipboard = (text: string, id: string) => {
    navigator.clipboard.writeText(text).then(() => {
      setCopiedId(id);
      setTimeout(() => setCopiedId(null), 2000);
    });
  };

  return (
    <div class="p-6">
      <div class="mb-4">
        <A
          href="/settings/projects"
          class="text-sm text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)]"
        >
          &larr; Back to Projects
        </A>
      </div>

      <Show when={projectQuery.data} fallback={<LoadingSkeleton rows={6} />}>
        {(project) => (
          <>
            <div class="mb-6">
              <h1 class="text-2xl font-bold text-[var(--color-text-primary)]">
                {project().name}
              </h1>
              <p class="mt-1 text-sm text-[var(--color-text-secondary)]">
                Slug: {project().slug} | Platform: {project().platform ?? "N/A"}{" "}
                | Created: {new Date(project().created_at).toLocaleDateString()}
              </p>
            </div>

            {/* DSN Keys */}
            <div class="rounded-lg border border-[var(--color-border)] overflow-hidden">
              <div class="border-b border-[var(--color-border)] bg-[var(--color-surface-1)] px-4 py-3">
                <h2 class="text-sm font-medium text-[var(--color-text-primary)]">
                  DSN Keys
                </h2>
                <p class="mt-0.5 text-xs text-[var(--color-text-secondary)]">
                  Use these DSN values to configure your Sentry SDK client.
                </p>
              </div>
              <div class="p-4">
                <Show
                  when={keysQuery.data}
                  fallback={<LoadingSkeleton rows={2} />}
                >
                  {(keys) => (
                    <Show
                      when={keys().length > 0}
                      fallback={
                        <p class="text-sm text-[var(--color-text-secondary)]">
                          No DSN keys configured for this project.
                        </p>
                      }
                    >
                      <div class="space-y-4">
                        <For each={keys()}>
                          {(key) => (
                            <div class="rounded-md border border-[var(--color-border)] bg-[var(--color-surface-0)] p-3">
                              <div class="flex items-center justify-between mb-2">
                                <span class="text-sm font-medium text-[var(--color-text-primary)]">
                                  {key.label || "Default"}
                                </span>
                                <span
                                  class={`text-xs ${key.is_active ? "text-green-600 dark:text-green-400" : "text-red-600 dark:text-red-400"}`}
                                >
                                  {key.is_active ? "Active" : "Inactive"}
                                </span>
                              </div>
                              <div class="space-y-2">
                                <div>
                                  <label class="text-xs text-[var(--color-text-secondary)]">
                                    Public Key
                                  </label>
                                  <div class="flex items-center gap-2 mt-0.5">
                                    <code class="flex-1 rounded bg-[var(--color-surface-2)] px-2 py-1 text-xs font-mono text-[var(--color-text-primary)] break-all">
                                      {key.public_key}
                                    </code>
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
                                        ? "Copied!"
                                        : "Copy"}
                                    </Button>
                                  </div>
                                </div>
                                <div>
                                  <label class="text-xs text-[var(--color-text-secondary)]">
                                    DSN
                                  </label>
                                  <div class="flex items-center gap-2 mt-0.5">
                                    <code class="flex-1 rounded bg-[var(--color-surface-2)] px-2 py-1 text-xs font-mono text-[var(--color-text-primary)] break-all">
                                      {key.dsn}
                                    </code>
                                    <Button
                                      variant="ghost"
                                      size="sm"
                                      onClick={() =>
                                        copyToClipboard(
                                          key.dsn,
                                          `dsn-${key.id}`,
                                        )
                                      }
                                    >
                                      {copiedId() === `dsn-${key.id}`
                                        ? "Copied!"
                                        : "Copy"}
                                    </Button>
                                  </div>
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
