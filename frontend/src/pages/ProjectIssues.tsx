import { A, useParams, useSearchParams } from "@solidjs/router";
import { createQuery } from "@tanstack/solid-query";
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
import IconArrowLeft from "~icons/lucide/arrow-left";
import IconArrowRight from "~icons/lucide/arrow-right";

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
    <div class="page">
      <div class="page__header">
        <h1 class="page__title">Issues</h1>
      </div>

      <div class="filter-bar">
        <div class="tabs" style={{ border: "none" }}>
          <For each={STATUSES}>
            {(s) => (
              <button
                class="tab"
                data-active={status() === s}
                onClick={() => setSearchParams({ status: s, cursor: undefined })}
              >
                {STATUS_LABELS[s] ?? s}
              </button>
            )}
          </For>
        </div>
        <div class="filter-bar__sort">
          <label class="filter-bar__sort-label">Sort:</label>
          <select
            value={sort()}
            onChange={(e) =>
              setSearchParams({ sort: e.currentTarget.value, cursor: undefined })
            }
            class="select"
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
          <div class="card">
            <table class="data-table">
              <thead>
                <tr>
                  <th style={{ width: "40px", "padding-left": "12px", "padding-right": "12px" }}>
                    <input
                      type="checkbox"
                      class="checkbox"
                      checked={
                        selectedIssues().size > 0 &&
                        selectedIssues().size ===
                          (issuesQuery.data?.issues.length ?? 0)
                      }
                      onChange={toggleSelectAll}
                    />
                  </th>
                  <th>Level</th>
                  <th>Error</th>
                  <th data-align="right">Events</th>
                  <th data-align="right">Last Seen</th>
                </tr>
              </thead>
              <tbody>
                <For each={issuesQuery.data?.issues}>
                  {(issue) => (
                    <tr>
                      <td style={{ width: "40px", "padding-left": "12px", "padding-right": "12px" }}>
                        <input
                          type="checkbox"
                          class="checkbox"
                          checked={selectedIssues().has(issue.id)}
                          onChange={() => toggleSelect(issue.id)}
                        />
                      </td>
                      <td>
                        <Badge level={issue.level} />
                      </td>
                      <td>
                        <A href={`/${params.project}/issues/${issue.id}`}>
                          {issue.title}
                        </A>
                        {issue.culprit && (
                          <div class="culprit">{issue.culprit}</div>
                        )}
                      </td>
                      <td data-align="right" class="text-secondary">
                        {formatNumber(issue.event_count)}
                      </td>
                      <td data-align="right" class="text-secondary">
                        <RelativeTime date={issue.last_seen} />
                      </td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </div>

          <div class="table-footer">
            <div class="table-footer__count">
              Showing {issuesQuery.data?.issues.length ?? 0} issues
            </div>
            <div class="pagination">
              <Button
                variant="ghost"
                size="sm"
                disabled={!hasPrev()}
                onClick={() => setSearchParams({ cursor: undefined })}
              >
                <IconArrowLeft /> Prev
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
                Next <IconArrowRight />
              </Button>
            </div>
          </div>
        </Show>
      </Show>
    </div>
  );
}
