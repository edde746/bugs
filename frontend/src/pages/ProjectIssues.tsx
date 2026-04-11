import { A, useParams, useSearchParams } from "@solidjs/router";
import { createQuery } from "@tanstack/solid-query";
import { clsx } from "clsx";
import { createSignal, For, Show } from "solid-js";
import { api } from "~/api/client";
import { queryKeys } from "~/queries/keys";
import type { IssueListResponse } from "~/lib/sentry-types";
import { formatNumber } from "~/lib/formatters";
import { STATUS_LABELS } from "~/lib/constants";
import Badge from "~/components/ui/Badge";
import Button from "~/components/ui/Button";
import RelativeTime from "~/components/ui/RelativeTime";
import LoadingSkeleton from "~/components/ui/LoadingSkeleton";
import EmptyState from "~/components/ui/EmptyState";

const STATUSES = ["unresolved", "resolved", "ignored"] as const;
const SORT_OPTIONS = [
  { value: "last_seen", label: "Last Seen" },
  { value: "first_seen", label: "First Seen" },
  { value: "events", label: "Events" },
] as const;

export default function ProjectIssues() {
  const params = useParams<{ project: string }>();
  const [searchParams, setSearchParams] = useSearchParams<{
    status?: string;
    sort?: string;
    cursor?: string;
  }>();

  const status = () => searchParams.status ?? "unresolved";
  const sort = () => searchParams.sort ?? "last_seen";
  const cursor = () => searchParams.cursor ?? "";

  const [selectedIssues, setSelectedIssues] = createSignal<Set<number>>(
    new Set(),
  );

  const filters = () => ({ status: status(), sort: sort(), cursor: cursor() });

  const issuesQuery = createQuery(() => ({
    queryKey: queryKeys.issues.list(params.project, filters()),
    queryFn: () => {
      let url = `/internal/projects/${params.project}/issues?status=${status()}&sort=${sort()}`;
      if (cursor()) url += `&cursor=${cursor()}`;
      return api.get<IssueListResponse>(url);
    },
    refetchInterval: 30000,
    refetchIntervalInBackground: false,
  }));

  const toggleSelectAll = () => {
    const issues = issuesQuery.data?.issues ?? [];
    if (selectedIssues().size === issues.length) {
      setSelectedIssues(new Set<number>());
    } else {
      setSelectedIssues(new Set(issues.map((i) => i.id)));
    }
  };

  const toggleSelect = (id: number) => {
    const current = new Set(selectedIssues());
    if (current.has(id)) {
      current.delete(id);
    } else {
      current.add(id);
    }
    setSelectedIssues(current);
  };

  const hasPrev = () => !!cursor();
  const hasNext = () => !!issuesQuery.data?.nextCursor;

  return (
    <div class="p-6">
      <div class="mb-6">
        <h1 class="text-2xl font-bold text-[var(--color-text-primary)]">
          Issues
        </h1>
      </div>

      {/* Status filter tabs + sort */}
      <div class="mb-4 flex items-center justify-between border-b border-[var(--color-border)]">
        <div class="flex gap-1">
          <For each={STATUSES}>
            {(s) => (
              <button
                class={clsx(
                  "px-3 py-2 text-sm font-medium transition-colors",
                  status() === s
                    ? "border-b-2 border-indigo-500 text-indigo-600 dark:text-indigo-400"
                    : "text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)]",
                )}
                onClick={() => setSearchParams({ status: s, cursor: undefined })}
              >
                {STATUS_LABELS[s] ?? s}
              </button>
            )}
          </For>
        </div>
        <div class="flex items-center gap-2 pb-1">
          <label class="text-xs text-[var(--color-text-secondary)]">Sort:</label>
          <select
            value={sort()}
            onChange={(e) =>
              setSearchParams({ sort: e.currentTarget.value, cursor: undefined })
            }
            class="rounded border border-[var(--color-border)] bg-[var(--color-surface-0)] px-2 py-1 text-xs text-[var(--color-text-primary)] focus:border-indigo-500 focus:outline-none"
          >
            <For each={SORT_OPTIONS}>
              {(opt) => <option value={opt.value}>{opt.label}</option>}
            </For>
          </select>
        </div>
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
                  <th class="w-10 px-3 py-2">
                    <input
                      type="checkbox"
                      checked={
                        selectedIssues().size > 0 &&
                        selectedIssues().size ===
                          (issuesQuery.data?.issues.length ?? 0)
                      }
                      onChange={toggleSelectAll}
                      class="rounded border-gray-300"
                    />
                  </th>
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
                      <td class="w-10 px-3 py-3">
                        <input
                          type="checkbox"
                          checked={selectedIssues().has(issue.id)}
                          onChange={() => toggleSelect(issue.id)}
                          class="rounded border-gray-300"
                        />
                      </td>
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

          {/* Pagination */}
          <div class="mt-4 flex items-center justify-between">
            <div class="text-xs text-[var(--color-text-secondary)]">
              Showing {issuesQuery.data?.issues.length ?? 0} issues
            </div>
            <div class="flex gap-2">
              <Button
                variant="ghost"
                size="sm"
                disabled={!hasPrev()}
                onClick={() => setSearchParams({ cursor: undefined })}
              >
                &larr; Prev
              </Button>
              <Button
                variant="ghost"
                size="sm"
                disabled={!hasNext()}
                onClick={() =>
                  setSearchParams({
                    cursor: String(issuesQuery.data!.nextCursor),
                  })
                }
              >
                Next &rarr;
              </Button>
            </div>
          </div>
        </Show>
      </Show>
    </div>
  );
}
