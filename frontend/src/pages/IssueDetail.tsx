import { A, useParams } from "@solidjs/router";
import { createQuery, createMutation, useQueryClient } from "@tanstack/solid-query";
import { createSignal, Show } from "solid-js";
import { api } from "~/api/client";
import { queryKeys } from "~/queries/keys";
import type { Issue, UpdateIssueInput, EventListResponse } from "~/lib/sentry-types";
import { relativeTime, formatNumber } from "~/lib/formatters";
import { STATUS_LABELS, STATUS_COLORS } from "~/lib/constants";
import Badge from "~/components/ui/Badge";
import Button from "~/components/ui/Button";
import LoadingSkeleton from "~/components/ui/LoadingSkeleton";
import ExceptionDisplay from "~/components/events/ExceptionDisplay";
import BreadcrumbsTimeline from "~/components/events/BreadcrumbsTimeline";
import ContextPanels from "~/components/events/ContextPanels";
import TagsTable from "~/components/events/TagsTable";

export default function IssueDetail() {
  const params = useParams<{ project: string; issueId: string }>();
  const queryClient = useQueryClient();
  const [eventIndex, setEventIndex] = createSignal(0);

  const issueQuery = createQuery(() => ({
    queryKey: queryKeys.issues.detail(params.issueId),
    queryFn: () => api.get<Issue>(`/internal/issues/${params.issueId}`),
  }));

  const eventsQuery = createQuery(() => ({
    queryKey: queryKeys.events.list(params.issueId),
    queryFn: () =>
      api.get<EventListResponse>(
        `/internal/issues/${params.issueId}/events?limit=50`,
      ),
    enabled: !!issueQuery.data,
  }));

  const currentEvent = () => {
    const events = eventsQuery.data?.events;
    if (!events || events.length === 0) return null;
    const idx = Math.min(eventIndex(), events.length - 1);
    return events[idx];
  };

  const parsedData = () => {
    const event = currentEvent();
    if (!event) return null;
    try {
      return JSON.parse(event.data);
    } catch {
      return null;
    }
  };

  const exceptions = () => {
    const data = parsedData();
    if (!data) return [];
    if (data.exception?.values) return data.exception.values;
    if (data.exceptions) return data.exceptions;
    return [];
  };

  const breadcrumbs = () => {
    const data = parsedData();
    if (!data) return [];
    if (data.breadcrumbs?.values) return data.breadcrumbs.values;
    if (Array.isArray(data.breadcrumbs)) return data.breadcrumbs;
    return [];
  };

  const contexts = () => {
    const data = parsedData();
    return data?.contexts ?? {};
  };

  const request = () => {
    const data = parsedData();
    return data?.request ?? null;
  };

  const user = () => {
    const data = parsedData();
    return data?.user ?? null;
  };

  const tags = () => {
    const data = parsedData();
    if (!data?.tags) return [];
    if (Array.isArray(data.tags)) return data.tags;
    return Object.entries(data.tags).map(([key, value]) => ({
      key,
      value: String(value),
    }));
  };

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

  const canGoNewer = () => eventIndex() > 0;
  const canGoOlder = () => {
    const events = eventsQuery.data?.events;
    return events ? eventIndex() < events.length - 1 : false;
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

            {/* Event navigation */}
            <div class="mb-4 flex items-center justify-between">
              <h2 class="text-sm font-medium text-[var(--color-text-primary)]">
                Event{" "}
                <Show when={eventsQuery.data}>
                  <span class="text-[var(--color-text-secondary)]">
                    ({eventIndex() + 1} of {eventsQuery.data!.events.length})
                  </span>
                </Show>
              </h2>
              <div class="flex items-center gap-2">
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => setEventIndex((i) => i - 1)}
                  disabled={!canGoNewer()}
                >
                  &larr; Newer
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => setEventIndex((i) => i + 1)}
                  disabled={!canGoOlder()}
                >
                  Older &rarr;
                </Button>
                <Show when={currentEvent()}>
                  {(ev) => (
                    <A
                      href={`/${params.project}/issues/${params.issueId}/events/${ev().id}`}
                      class="text-xs text-indigo-600 hover:text-indigo-800 dark:text-indigo-400 ml-2"
                    >
                      View Full Event
                    </A>
                  )}
                </Show>
              </div>
            </div>

            {/* Event content */}
            <Show when={!eventsQuery.isPending} fallback={<LoadingSkeleton rows={8} />}>
              <Show
                when={currentEvent()}
                fallback={
                  <div class="text-sm text-[var(--color-text-secondary)] py-8 text-center">
                    No events found for this issue.
                  </div>
                }
              >
                {(event) => (
                  <div class="space-y-6">
                    {/* Event header */}
                    <div class="flex items-center gap-3 text-xs text-[var(--color-text-secondary)] font-mono">
                      <span>ID: {event().event_id}</span>
                      <span>{relativeTime(event().timestamp)}</span>
                      <Show when={event().platform}>
                        <span class="rounded bg-[var(--color-surface-2)] px-1.5 py-0.5">
                          {event().platform}
                        </span>
                      </Show>
                    </div>

                    {/* Exception Display + Stack Trace */}
                    <Show when={exceptions().length > 0}>
                      <ExceptionDisplay exceptions={exceptions()} />
                    </Show>

                    {/* Breadcrumbs */}
                    <Show when={breadcrumbs().length > 0}>
                      <BreadcrumbsTimeline breadcrumbs={breadcrumbs()} />
                    </Show>

                    {/* Tags */}
                    <Show when={tags().length > 0}>
                      <TagsTable tags={tags()} />
                    </Show>

                    {/* Context Panels */}
                    <ContextPanels
                      contexts={contexts()}
                      request={request()}
                      user={user()}
                    />
                  </div>
                )}
              </Show>
            </Show>
          </>
        )}
      </Show>
    </div>
  );
}
