import { useParams } from "@solidjs/router";
import { createQuery } from "@tanstack/solid-query";
import { For, Show } from "solid-js";
import { api } from "~/api/client";
import { relativeTime } from "~/lib/formatters";
import LoadingSkeleton from "~/components/ui/LoadingSkeleton";
import EmptyState from "~/components/ui/EmptyState";

interface TransactionGroup {
  id: number;
  project_id: number;
  transaction_name: string;
  op: string;
  method: string;
  count: number;
  error_count: number;
  sum_duration_ms: number;
  min_duration_ms: number | null;
  max_duration_ms: number | null;
  p50_duration_ms: number | null;
  p95_duration_ms: number | null;
  last_seen: string;
}

function formatDuration(ms: number | null | undefined): string {
  if (ms == null) return "\u2014";
  if (ms < 1) return `${(ms * 1000).toFixed(0)}\u00b5s`;
  if (ms < 1000) return `${ms.toFixed(0)}ms`;
  return `${(ms / 1000).toFixed(2)}s`;
}

function formatErrorRate(errorCount: number, total: number): string {
  if (total === 0) return "0%";
  return `${((errorCount / total) * 100).toFixed(1)}%`;
}

export default function ProjectPerformance() {
  const params = useParams<{ project: string }>();

  const query = createQuery(() => ({
    queryKey: ["performance", params.project],
    queryFn: () =>
      api.get<TransactionGroup[]>(
        `/internal/projects/${params.project}/transactions?limit=100`,
      ),
  }));

  return (
    <div class="page">
      <div class="page__header">
        <h1 class="page__title">Performance</h1>
      </div>

      <Show when={!query.isPending} fallback={<LoadingSkeleton rows={6} />}>
        <Show
          when={query.data && query.data.length > 0}
          fallback={
            <EmptyState
              title="No transactions yet"
              description="Transaction data will appear here once your application sends performance data."
            />
          }
        >
          <div class="card">
            <table class="data-table">
              <thead>
                <tr>
                  <th>Transaction</th>
                  <th>Op</th>
                  <th data-align="right">Count</th>
                  <th data-align="right">Avg</th>
                  <th data-align="right">P50</th>
                  <th data-align="right">P95</th>
                  <th data-align="right">Error Rate</th>
                  <th data-align="right">Last Seen</th>
                </tr>
              </thead>
              <tbody>
                <For each={query.data}>
                  {(group) => (
                    <tr>
                      <td style={{ "font-weight": "500", "max-width": "300px", overflow: "hidden", "text-overflow": "ellipsis" }}>
                        {group.method ? `${group.method} ` : ""}{group.transaction_name}
                      </td>
                      <td class="text-secondary">{group.op || "\u2014"}</td>
                      <td data-align="right">{group.count.toLocaleString()}</td>
                      <td data-align="right" class="text-mono">
                        {formatDuration(group.count > 0 ? group.sum_duration_ms / group.count : null)}
                      </td>
                      <td data-align="right" class="text-mono">{formatDuration(group.p50_duration_ms)}</td>
                      <td data-align="right" class="text-mono">{formatDuration(group.p95_duration_ms)}</td>
                      <td data-align="right" class={group.error_count > 0 ? "text-error" : "text-secondary"}>
                        {formatErrorRate(group.error_count, group.count)}
                      </td>
                      <td data-align="right" class="text-secondary">{relativeTime(group.last_seen)}</td>
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
