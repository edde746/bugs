import { useParams } from "@solidjs/router";
import { createQuery } from "@tanstack/solid-query";
import { For, Show } from "solid-js";
import { api } from "~/api/client";
import type { ProjectReleaseSummary } from "~/lib/sentry-types";
import { relativeTime } from "~/lib/formatters";
import LoadingSpinner from "~/components/ui/LoadingSpinner";
import EmptyState from "~/components/ui/EmptyState";
import ErrorState from "~/components/ui/ErrorState";

export default function ProjectReleases() {
  const params = useParams<{ project: string }>();

  const releasesQuery = createQuery(() => ({
    queryKey: ["releases", params.project],
    queryFn: () =>
      api.get<ProjectReleaseSummary[]>(
        `/internal/projects/${params.project}/releases`,
      ),
  }));

  return (
    <div class="page">
      <div class="page__header">
        <h1 class="page__title">Releases</h1>
      </div>

      <Show when={!releasesQuery.isPending} fallback={<LoadingSpinner />}>
        <Show
          when={!releasesQuery.isError}
          fallback={<ErrorState title="Couldn't load releases" error={releasesQuery.error} />}
        >
        <Show
          when={releasesQuery.data && releasesQuery.data.length > 0}
          fallback={
            <EmptyState
              title="No releases found"
              description="No releases have been tracked for this project yet."
            />
          }
        >
          <div class="card">
            <table class="data-table">
              <thead>
                <tr>
                  <th>Version</th>
                  <th>Created</th>
                  <th data-align="right">Files</th>
                </tr>
              </thead>
              <tbody>
                <For each={releasesQuery.data}>
                  {(release) => (
                    <tr>
                      <td>
                        <span class="text-mono" style={{ "font-weight": "500" }}>
                          {release.version}
                        </span>
                      </td>
                      <td class="text-secondary">
                        {relativeTime(release.created_at)}
                      </td>
                      <td data-align="right" class="text-secondary">
                        {release.file_count}
                      </td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </div>
        </Show>
        </Show>
      </Show>
    </div>
  );
}
