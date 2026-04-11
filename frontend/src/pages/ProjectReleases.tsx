import { useParams } from "@solidjs/router";
import { createQuery } from "@tanstack/solid-query";
import { For, Show } from "solid-js";
import { api } from "~/api/client";
import { relativeTime } from "~/lib/formatters";
import LoadingSkeleton from "~/components/ui/LoadingSkeleton";
import EmptyState from "~/components/ui/EmptyState";

interface Release {
  version: string;
  dateCreated: string;
  dateReleased: string | null;
  shortVersion: string;
  newGroups: number;
}

export default function ProjectReleases() {
  const params = useParams<{ project: string }>();

  const releasesQuery = createQuery(() => ({
    queryKey: ["releases", params.project],
    queryFn: () =>
      api.get<Release[]>(
        `/0/organizations/default/releases?project=${params.project}`,
      ),
  }));

  return (
    <div class="p-6">
      <div class="mb-6">
        <h1 class="text-2xl font-bold text-[var(--color-text-primary)]">
          Releases
        </h1>
      </div>

      <Show when={!releasesQuery.isPending} fallback={<LoadingSkeleton rows={6} />}>
        <Show
          when={releasesQuery.data && releasesQuery.data.length > 0}
          fallback={
            <EmptyState
              title="No releases found"
              description="No releases have been tracked for this project yet."
            />
          }
        >
          <div class="overflow-hidden rounded-lg border border-[var(--color-border)]">
            <table class="w-full">
              <thead>
                <tr class="border-b border-[var(--color-border)] bg-[var(--color-surface-1)]">
                  <th class="px-4 py-2 text-left text-xs font-medium text-[var(--color-text-secondary)]">
                    Version
                  </th>
                  <th class="px-4 py-2 text-left text-xs font-medium text-[var(--color-text-secondary)]">
                    Date Created
                  </th>
                  <th class="px-4 py-2 text-right text-xs font-medium text-[var(--color-text-secondary)]">
                    New Issues
                  </th>
                </tr>
              </thead>
              <tbody>
                <For each={releasesQuery.data}>
                  {(release) => (
                    <tr class="border-b border-[var(--color-border)] transition-colors hover:bg-[var(--color-surface-1)]">
                      <td class="px-4 py-3">
                        <span class="font-mono text-sm font-medium text-[var(--color-text-primary)]">
                          {release.shortVersion ?? release.version}
                        </span>
                      </td>
                      <td class="px-4 py-3 text-sm text-[var(--color-text-secondary)]">
                        {relativeTime(release.dateCreated)}
                      </td>
                      <td class="px-4 py-3 text-right text-sm text-[var(--color-text-secondary)]">
                        {release.newGroups}
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
