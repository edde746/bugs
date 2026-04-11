import { A, useParams } from "@solidjs/router";
import { createQuery } from "@tanstack/solid-query";
import { For, Show } from "solid-js";
import { api } from "~/api/client";
import { queryKeys } from "~/queries/keys";
import type { Project } from "~/lib/sentry-types";
import IconCircleDot from "~icons/lucide/circle-dot";
import IconTag from "~icons/lucide/tag";
import IconBell from "~icons/lucide/bell";
import IconGauge from "~icons/lucide/gauge";
import IconSettings from "~icons/lucide/settings";

export default function Sidebar() {
  const params = useParams<{ project?: string }>();

  const projectsQuery = createQuery(() => ({
    queryKey: queryKeys.projects.all(),
    queryFn: () => api.get<Project[]>("/internal/projects"),
  }));

  return (
    <aside class="sidebar">
      <div class="sidebar__brand">
        <span class="sidebar__brand-text">Bugs</span>
      </div>

      <nav class="sidebar__nav">
        <div class="sidebar__section-label">Projects</div>
        <Show when={projectsQuery.data} fallback={<div class="sidebar__link text-secondary">Loading...</div>}>
          {(projects) => (
            <For each={projects()}>
              {(project) => (
                <A
                  href={`/${project.slug}/issues`}
                  class="sidebar__link"
                  data-active={params.project === project.slug}
                >
                  <span class="sidebar__avatar">
                    {project.name.charAt(0).toUpperCase()}
                  </span>
                  {project.name}
                </A>
              )}
            </For>
          )}
        </Show>

        <Show when={params.project}>
          <div class="sidebar__section-label" style={{ "margin-top": "16px" }}>
            Navigation
          </div>
          <A
            href={`/${params.project}/issues`}
            class="sidebar__link"
          >
            <IconCircleDot /> Issues
          </A>
          <A
            href={`/${params.project}/releases`}
            class="sidebar__link"
          >
            <IconTag /> Releases
          </A>
          <A
            href={`/${params.project}/performance`}
            class="sidebar__link"
          >
            <IconGauge /> Performance
          </A>
          <A
            href={`/${params.project}/alerts`}
            class="sidebar__link"
          >
            <IconBell /> Alerts
          </A>
        </Show>
      </nav>

      <div class="sidebar__footer">
        <A href="/settings/projects" class="sidebar__link">
          <IconSettings /> Settings
        </A>
      </div>
    </aside>
  );
}
