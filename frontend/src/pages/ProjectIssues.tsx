import { A, useParams, useSearchParams } from "@solidjs/router";
import { createQuery, createMutation, useQueryClient } from "@tanstack/solid-query";
import { createSignal, For, Show } from "solid-js";
import { api } from "~/api/client";
import { queryKeys } from "~/queries/keys";
import type { IssueListResponse, IssueFilterOptions, BulkUpdateIssuesInput, BulkDeleteIssuesInput } from "~/lib/sentry-types";
import { formatNumber } from "~/lib/formatters";
import { STATUS_LABELS } from "~/lib/constants";
import Badge from "~/components/ui/Badge";
import Button from "~/components/ui/Button";
import RelativeTime from "~/components/ui/RelativeTime";
import LoadingSpinner from "~/components/ui/LoadingSpinner";
import EmptyState from "~/components/ui/EmptyState";
import ErrorState from "~/components/ui/ErrorState";
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
  const queryClient = useQueryClient();
  const [searchParams, setSearchParams] = useSearchParams<{
    status?: string;
    sort?: string;
    cursor?: string;
    release?: string;
    environment?: string;
    level?: string;
  }>();

  const status = () => searchParams.status ?? "unresolved";
  const sort = () => searchParams.sort ?? "last_seen";
  const cursor = () => searchParams.cursor ?? "";
  const release = () => searchParams.release ?? "";
  const environment = () => searchParams.environment ?? "";
  const level = () => searchParams.level ?? "";

  const [selectedIssues, setSelectedIssues] = createSignal<Set<number>>(
    new Set(),
  );

  const filters = () => ({
    status: status(),
    sort: sort(),
    cursor: cursor(),
    release: release(),
    environment: environment(),
    level: level(),
  });

  const issuesQuery = createQuery(() => ({
    queryKey: queryKeys.issues.list(params.project, filters()),
    queryFn: () => {
      let url = `/internal/projects/${params.project}/issues?status=${status()}&sort=${sort()}`;
      if (cursor()) url += `&cursor=${cursor()}`;
      if (release()) url += `&release=${encodeURIComponent(release())}`;
      if (environment()) url += `&environment=${encodeURIComponent(environment())}`;
      if (level()) url += `&level=${encodeURIComponent(level())}`;
      return api.get<IssueListResponse>(url);
    },
    refetchInterval: 30000,
    refetchIntervalInBackground: false,
  }));

  const filtersQuery = createQuery(() => ({
    queryKey: queryKeys.issues.filters(params.project),
    queryFn: () =>
      api.get<IssueFilterOptions>(
        `/internal/projects/${params.project}/issues/filters`,
      ),
    staleTime: 60000,
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

  const clearSelection = () => setSelectedIssues(new Set<number>());

  const onBulkSuccess = () => {
    clearSelection();
    // Use the list prefix so we invalidate every filter variant of the
    // project's issues list, not just one with an empty filter object.
    queryClient.invalidateQueries({
      queryKey: queryKeys.issues.listPrefix(params.project),
    });
  };

  const bulkUpdateMutation = createMutation(() => ({
    mutationFn: (input: BulkUpdateIssuesInput) =>
      api.put<void>("/internal/issues/bulk", input),
    onSuccess: onBulkSuccess,
  }));

  const bulkDeleteMutation = createMutation(() => ({
    mutationFn: (input: BulkDeleteIssuesInput) =>
      api.post<void>("/internal/issues/bulk/delete", input),
    onSuccess: onBulkSuccess,
  }));

  const isBulkPending = () =>
    bulkUpdateMutation.isPending || bulkDeleteMutation.isPending;

  const handleBulkStatus = (newStatus: string) => {
    bulkUpdateMutation.mutate({ ids: [...selectedIssues()], status: newStatus });
  };

  const handleBulkDelete = () => {
    bulkDeleteMutation.mutate({ ids: [...selectedIssues()] });
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
                onClick={() => {
                  clearSelection();
                  setSearchParams({ status: s, cursor: undefined });
                }}
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

      <Show when={filtersQuery.data}>
        <div class="filter-bar__filters">
          <div class="filter-bar__group">
            <label class="filter-bar__sort-label">Release:</label>
            <select
              value={release()}
              onChange={(e) =>
                setSearchParams({
                  release: e.currentTarget.value || undefined,
                  cursor: undefined,
                })
              }
              class="select"
            >
              <option value="">All Releases</option>
              <For each={filtersQuery.data!.releases}>
                {(r) => <option value={r}>{r}</option>}
              </For>
            </select>
          </div>

          <div class="filter-bar__group">
            <label class="filter-bar__sort-label">Environment:</label>
            <select
              value={environment()}
              onChange={(e) =>
                setSearchParams({
                  environment: e.currentTarget.value || undefined,
                  cursor: undefined,
                })
              }
              class="select"
            >
              <option value="">All Environments</option>
              <For each={filtersQuery.data!.environments}>
                {(env) => <option value={env}>{env}</option>}
              </For>
            </select>
          </div>

          <div class="filter-bar__group">
            <label class="filter-bar__sort-label">Level:</label>
            <select
              value={level()}
              onChange={(e) =>
                setSearchParams({
                  level: e.currentTarget.value || undefined,
                  cursor: undefined,
                })
              }
              class="select"
            >
              <option value="">All Levels</option>
              <For each={filtersQuery.data!.levels}>
                {(l) => <option value={l}>{l}</option>}
              </For>
            </select>
          </div>
        </div>
      </Show>

      <Show when={selectedIssues().size > 0}>
        <div class="bulk-action-bar">
          <span class="text-secondary">{selectedIssues().size} selected</span>
          <div class="inline-gap">
            <Show when={status() !== "resolved"}>
              <Button
                variant="secondary"
                size="sm"
                disabled={isBulkPending()}
                onClick={() => handleBulkStatus("resolved")}
              >
                Resolve
              </Button>
            </Show>
            <Show when={status() !== "ignored"}>
              <Button
                variant="ghost"
                size="sm"
                disabled={isBulkPending()}
                onClick={() => handleBulkStatus("ignored")}
              >
                Ignore
              </Button>
            </Show>
            <Show when={status() !== "unresolved"}>
              <Button
                variant="secondary"
                size="sm"
                disabled={isBulkPending()}
                onClick={() => handleBulkStatus("unresolved")}
              >
                Unresolve
              </Button>
            </Show>
            <Button
              variant="danger"
              size="sm"
              disabled={isBulkPending()}
              onClick={handleBulkDelete}
            >
              Delete
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={clearSelection}
            >
              Clear
            </Button>
          </div>
        </div>
      </Show>

      <Show when={!issuesQuery.isPending} fallback={<LoadingSpinner />}>
        <Show
          when={!issuesQuery.isError}
          fallback={
            <ErrorState
              title="Couldn't load issues"
              error={issuesQuery.error}
            />
          }
        >
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
      </Show>
    </div>
  );
}
