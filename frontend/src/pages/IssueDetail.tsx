import { A, useParams } from "@solidjs/router";
import { createQuery, createMutation, useQueryClient } from "@tanstack/solid-query";
import { createSignal, Show } from "solid-js";
import { api } from "~/api/client";
import { queryKeys } from "~/queries/keys";
import type { Issue, UpdateIssueInput, EventListResponse } from "~/lib/sentry-types";
import { relativeTime, formatNumber } from "~/lib/formatters";
import { STATUS_LABELS } from "~/lib/constants";
import Badge from "~/components/ui/Badge";
import Button from "~/components/ui/Button";
import LoadingSkeleton from "~/components/ui/LoadingSkeleton";
import ExceptionDisplay from "~/components/events/ExceptionDisplay";
import BreadcrumbsTimeline from "~/components/events/BreadcrumbsTimeline";
import ContextPanels from "~/components/events/ContextPanels";
import TagsTable from "~/components/events/TagsTable";
import IconArrowLeft from "~icons/lucide/arrow-left";
import IconArrowRight from "~icons/lucide/arrow-right";

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
    <div class="page">
      <A href={`/${params.project}/issues`} class="back-link">
        <IconArrowLeft /> Back to Issues
      </A>

      <Show when={issueQuery.data} fallback={<LoadingSkeleton rows={6} />}>
        {(issue) => (
          <>
            <div style={{ display: "flex", "align-items": "flex-start", "justify-content": "space-between", "margin-bottom": "24px" }}>
              <div>
                <div class="inline-gap" style={{ "margin-bottom": "8px" }}>
                  <Badge level={issue().level} />
                  <span class="status-text" data-status={issue().status}>
                    {STATUS_LABELS[issue().status] ?? issue().status}
                  </span>
                </div>
                <h1 class="page__title">{issue().title}</h1>
                {issue().culprit && (
                  <p class="page__subtitle">{issue().culprit}</p>
                )}
              </div>
              <div class="inline-gap">
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

            <div class="stat-cards">
              <div class="stat-card">
                <div class="stat-card__label">Events</div>
                <div class="stat-card__value">
                  {formatNumber(issue().event_count)}
                </div>
              </div>
              <div class="stat-card">
                <div class="stat-card__label">First Seen</div>
                <div class="stat-card__value stat-card__value--sm">
                  {relativeTime(issue().first_seen)}
                </div>
              </div>
              <div class="stat-card">
                <div class="stat-card__label">Last Seen</div>
                <div class="stat-card__value stat-card__value--sm">
                  {relativeTime(issue().last_seen)}
                </div>
              </div>
            </div>

            <div class="inline-gap inline-gap--between" style={{ "margin-bottom": "16px" }}>
              <h2 class="text-sm" style={{ "font-weight": "500" }}>
                Event{" "}
                <Show when={eventsQuery.data}>
                  <span class="text-secondary">
                    ({eventIndex() + 1} of {eventsQuery.data!.events.length})
                  </span>
                </Show>
              </h2>
              <div class="inline-gap">
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => setEventIndex((i) => i - 1)}
                  disabled={!canGoNewer()}
                >
                  <IconArrowLeft /> Newer
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => setEventIndex((i) => i + 1)}
                  disabled={!canGoOlder()}
                >
                  Older <IconArrowRight />
                </Button>
                <Show when={currentEvent()}>
                  {(ev) => (
                    <A
                      href={`/${params.project}/issues/${params.issueId}/events/${ev().id}`}
                      class="link-accent"
                    >
                      View Full Event
                    </A>
                  )}
                </Show>
              </div>
            </div>

            <Show when={!eventsQuery.isPending} fallback={<LoadingSkeleton rows={8} />}>
              <Show
                when={currentEvent()}
                fallback={
                  <div class="text-secondary text-sm" style={{ padding: "32px 0", "text-align": "center" }}>
                    No events found for this issue.
                  </div>
                }
              >
                {(event) => (
                  <div class="section-gap">
                    <div class="meta-row">
                      <span>ID: {event().event_id}</span>
                      <span>{relativeTime(event().timestamp)}</span>
                      <Show when={event().platform}>
                        <span class="meta-tag">{event().platform}</span>
                      </Show>
                    </div>

                    <Show when={exceptions().length > 0}>
                      <ExceptionDisplay exceptions={exceptions()} />
                    </Show>

                    <Show when={breadcrumbs().length > 0}>
                      <BreadcrumbsTimeline breadcrumbs={breadcrumbs()} />
                    </Show>

                    <Show when={tags().length > 0}>
                      <TagsTable tags={tags()} />
                    </Show>

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
