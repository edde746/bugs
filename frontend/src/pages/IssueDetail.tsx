import { A, useParams } from "@solidjs/router";
import { createQuery, createMutation, useQueryClient } from "@tanstack/solid-query";
import { Show } from "solid-js";
import { api } from "~/api/client";
import { queryKeys } from "~/queries/keys";
import type { Issue, Event as SentryEvent, UpdateIssueInput } from "~/lib/sentry-types";
import { relativeTime, formatNumber } from "~/lib/formatters";
import { STATUS_LABELS, STATUS_COLORS } from "~/lib/constants";
import Badge from "~/components/ui/Badge";
import Button from "~/components/ui/Button";
import LoadingSkeleton from "~/components/ui/LoadingSkeleton";

export default function IssueDetail() {
  const params = useParams<{ project: string; issueId: string }>();
  const queryClient = useQueryClient();

  const issueQuery = createQuery(() => ({
    queryKey: queryKeys.issues.detail(params.issueId),
    queryFn: () => api.get<Issue>(`/internal/issues/${params.issueId}`),
  }));

  const latestEventQuery = createQuery(() => ({
    queryKey: queryKeys.events.latest(params.issueId),
    queryFn: () =>
      api.get<SentryEvent>(`/internal/issues/${params.issueId}/events/latest`),
    enabled: !!issueQuery.data,
  }));

  const updateMutation = createMutation(() => ({
    mutationFn: (input: UpdateIssueInput) =>
      api.put<Issue>(`/internal/issues/${params.issueId}`, input),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.issues.detail(params.issueId) });
      queryClient.invalidateQueries({
        queryKey: queryKeys.issues.list(params.project, {}),
        exact: false,
      });
    },
  }));

  const handleStatusChange = (status: string) => {
    updateMutation.mutate({ status });
  };

  return (
    <div class="p-6">
      <div class="mb-4">
        <A
          href={`/${params.project}/issues`}
          class="text-sm text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)]"
        >
          &larr; Back to Issues
        </A>
      </div>

      <Show when={issueQuery.data} fallback={<LoadingSkeleton rows={6} />}>
        {(issue) => (
          <>
            <div class="mb-6 flex items-start justify-between">
              <div>
                <div class="mb-2 flex items-center gap-2">
                  <Badge level={issue().level} />
                  <span
                    class={`text-sm font-medium ${STATUS_COLORS[issue().status] ?? ""}`}
                  >
                    {STATUS_LABELS[issue().status] ?? issue().status}
                  </span>
                </div>
                <h1 class="text-2xl font-bold text-[var(--color-text-primary)]">
                  {issue().title}
                </h1>
                {issue().culprit && (
                  <p class="mt-1 text-sm text-[var(--color-text-secondary)]">
                    {issue().culprit}
                  </p>
                )}
              </div>
              <div class="flex gap-2">
                <Show when={issue().status !== "resolved"}>
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={() => handleStatusChange("resolved")}
                    disabled={updateMutation.isPending}
                  >
                    Resolve
                  </Button>
                </Show>
                <Show when={issue().status !== "ignored"}>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => handleStatusChange("ignored")}
                    disabled={updateMutation.isPending}
                  >
                    Ignore
                  </Button>
                </Show>
                <Show when={issue().status !== "unresolved"}>
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={() => handleStatusChange("unresolved")}
                    disabled={updateMutation.isPending}
                  >
                    Unresolve
                  </Button>
                </Show>
              </div>
            </div>

            {/* Stats */}
            <div class="mb-6 grid grid-cols-3 gap-4">
              <div class="rounded-lg border border-[var(--color-border)] bg-[var(--color-surface-1)] p-4">
                <div class="text-sm text-[var(--color-text-secondary)]">Events</div>
                <div class="mt-1 text-2xl font-semibold text-[var(--color-text-primary)]">
                  {formatNumber(issue().event_count)}
                </div>
              </div>
              <div class="rounded-lg border border-[var(--color-border)] bg-[var(--color-surface-1)] p-4">
                <div class="text-sm text-[var(--color-text-secondary)]">First Seen</div>
                <div class="mt-1 text-sm font-medium text-[var(--color-text-primary)]">
                  {relativeTime(issue().first_seen)}
                </div>
              </div>
              <div class="rounded-lg border border-[var(--color-border)] bg-[var(--color-surface-1)] p-4">
                <div class="text-sm text-[var(--color-text-secondary)]">Last Seen</div>
                <div class="mt-1 text-sm font-medium text-[var(--color-text-primary)]">
                  {relativeTime(issue().last_seen)}
                </div>
              </div>
            </div>

            {/* Latest Event */}
            <div class="rounded-lg border border-[var(--color-border)]">
              <div class="border-b border-[var(--color-border)] bg-[var(--color-surface-1)] px-4 py-3">
                <h2 class="text-sm font-medium text-[var(--color-text-primary)]">
                  Latest Event
                </h2>
              </div>
              <div class="p-4">
                <Show
                  when={latestEventQuery.data}
                  fallback={<LoadingSkeleton rows={4} />}
                >
                  {(event) => (
                    <div>
                      <div class="mb-2 flex items-center gap-3 text-sm text-[var(--color-text-secondary)]">
                        <span>Event ID: {event().event_id}</span>
                        <span>{relativeTime(event().timestamp)}</span>
                        <A
                          href={`/${params.project}/issues/${params.issueId}/events/${event().id}`}
                          class="text-indigo-600 hover:text-indigo-800 dark:text-indigo-400"
                        >
                          View Full Event
                        </A>
                      </div>
                      <pre class="max-h-96 overflow-auto rounded-md bg-[var(--color-surface-2)] p-4 text-xs text-[var(--color-text-primary)]">
                        {JSON.stringify(JSON.parse(event().data), null, 2)}
                      </pre>
                    </div>
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
