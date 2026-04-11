import { A, useParams } from "@solidjs/router";
import { createQuery } from "@tanstack/solid-query";
import { clsx } from "clsx";
import { For, Show } from "solid-js";
import { api } from "~/api/client";
import { queryKeys } from "~/queries/keys";
import type { Project } from "~/lib/sentry-types";

export default function Sidebar() {
  const params = useParams<{ project?: string }>();

  const projectsQuery = createQuery(() => ({
    queryKey: queryKeys.projects.all(),
    queryFn: () => api.get<Project[]>("/internal/projects"),
  }));

  return (
    <aside class="flex h-screen w-56 flex-col border-r border-[var(--color-border)] bg-[var(--color-surface-1)]">
      <div class="flex h-14 items-center gap-2 border-b border-[var(--color-border)] px-4">
        <span class="text-lg font-bold text-[var(--color-text-primary)]">
          Bugs
        </span>
      </div>

      <nav class="flex-1 overflow-y-auto p-3">
        <div class="mb-2 px-2 text-xs font-semibold uppercase tracking-wider text-[var(--color-text-secondary)]">
          Projects
        </div>
        <Show when={projectsQuery.data} fallback={<div class="px-2 text-sm text-[var(--color-text-secondary)]">Loading...</div>}>
          {(projects) => (
            <For each={projects()}>
              {(project) => (
                <A
                  href={`/${project.slug}/issues`}
                  class={clsx(
                    "mb-0.5 flex items-center rounded-md px-2 py-1.5 text-sm transition-colors",
                    params.project === project.slug
                      ? "bg-indigo-50 text-indigo-700 dark:bg-indigo-900/20 dark:text-indigo-300"
                      : "text-[var(--color-text-secondary)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text-primary)]",
                  )}
                >
                  <span class="mr-2 flex h-5 w-5 items-center justify-center rounded bg-gray-200 text-xs font-medium text-gray-600 dark:bg-gray-700 dark:text-gray-300">
                    {project.name.charAt(0).toUpperCase()}
                  </span>
                  {project.name}
                </A>
              )}
            </For>
          )}
        </Show>
      </nav>

      <div class="border-t border-[var(--color-border)] p-3">
        <A
          href="/settings/projects"
          class="flex items-center rounded-md px-2 py-1.5 text-sm text-[var(--color-text-secondary)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text-primary)]"
        >
          Settings
        </A>
      </div>
    </aside>
  );
}
