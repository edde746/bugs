import { A, useParams, useSearchParams } from "@solidjs/router";
import { createQuery } from "@tanstack/solid-query";
import { clsx } from "clsx";
import { For, Show } from "solid-js";
import { api } from "~/api/client";
import { queryKeys } from "~/queries/keys";
import type { IssueListResponse } from "~/lib/sentry-types";
import { formatNumber } from "~/lib/formatters";
import { STATUS_LABELS } from "~/lib/constants";
import Badge from "~/components/ui/Badge";
import RelativeTime from "~/components/ui/RelativeTime";
import LoadingSkeleton from "~/components/ui/LoadingSkeleton";
import EmptyState from "~/components/ui/EmptyState";

const STATUSES = ["unresolved", "resolved", "ignored"] as const;

export default function ProjectIssues() {
  const params = useParams<{ project: string }>();
  const [searchParams, setSearchParams] = useSearchParams<{ status?: string }>();

  const status = () => searchParams.status ?? "unresolved";
  const filters = () => ({ status: status() });

  const issuesQuery = createQuery(() => ({
    queryKey: queryKeys.issues.list(params.project, filters()),
    queryFn: () =>
      api.get<IssueListResponse>(
        `/internal/projects/${params.project}/issues?status=${status()}`,
      ),
  }));

  return (
    <div class="p-6">
      <div class="mb-6">
        <h1 class="text-2xl font-bold text-[var(--color-text-primary)]">
          Issues
        </h1>
      </div>

      {/* Status filter tabs */}
      <div class="mb-4 flex gap-1 border-b border-[var(--color-border)]">
        <For each={STATUSES}>
          {(s) => (
            <button
              class={clsx(
                "px-3 py-2 text-sm font-medium transition-colors",
                status() === s
                  ? "border-b-2 border-indigo-500 text-indigo-600 dark:text-indigo-400"
                  : "text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)]",
              )}
              onClick={() => setSearchParams({ status: s })}
            >
              {STATUS_LABELS[s] ?? s}
            </button>
          )}
        </For>
      </div>

      <Show when={!issuesQuery.isPending} fallback={<LoadingSkeleton rows={8} />}>
        <Show
          when={issuesQuery.data && issuesQuery.data.issues.length > 0}
          fallback={
            <EmptyState
              title="No issues found"
              description={`No ${status()} issues in this project yet.`}
            />
          }
        >
          <div class="overflow-hidden rounded-lg border border-[var(--color-border)]">
            <table class="w-full">
              <thead>
                <tr class="border-b border-[var(--color-border)] bg-[var(--color-surface-1)]">
                  <th class="px-4 py-2 text-left text-xs font-medium text-[var(--color-text-secondary)]">
                    Level
                  </th>
                  <th class="px-4 py-2 text-left text-xs font-medium text-[var(--color-text-secondary)]">
                    Error
                  </th>
                  <th class="px-4 py-2 text-right text-xs font-medium text-[var(--color-text-secondary)]">
                    Events
                  </th>
                  <th class="px-4 py-2 text-right text-xs font-medium text-[var(--color-text-secondary)]">
                    Last Seen
                  </th>
                </tr>
              </thead>
              <tbody>
                <For each={issuesQuery.data?.issues}>
                  {(issue) => (
                    <tr class="border-b border-[var(--color-border)] transition-colors hover:bg-[var(--color-surface-1)]">
                      <td class="px-4 py-3">
                        <Badge level={issue.level} />
                      </td>
                      <td class="px-4 py-3">
                        <A
                          href={`/${params.project}/issues/${issue.id}`}
                          class="font-medium text-[var(--color-text-primary)] hover:text-indigo-600 dark:hover:text-indigo-400"
                        >
                          {issue.title}
                        </A>
                        {issue.culprit && (
                          <div class="mt-0.5 text-xs text-[var(--color-text-secondary)]">
                            {issue.culprit}
                          </div>
                        )}
                      </td>
                      <td class="px-4 py-3 text-right text-sm text-[var(--color-text-secondary)]">
                        {formatNumber(issue.event_count)}
                      </td>
                      <td class="px-4 py-3 text-right text-sm text-[var(--color-text-secondary)]">
                        <RelativeTime date={issue.last_seen} />
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
