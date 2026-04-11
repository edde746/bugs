import { A, useParams } from "@solidjs/router";
import { createQuery } from "@tanstack/solid-query";
import { createSignal, Show } from "solid-js";
import { api } from "~/api/client";
import { queryKeys } from "~/queries/keys";
import type { Event as SentryEvent } from "~/lib/sentry-types";
import { relativeTime } from "~/lib/formatters";
import Badge from "~/components/ui/Badge";
import Button from "~/components/ui/Button";
import LoadingSkeleton from "~/components/ui/LoadingSkeleton";
import ExceptionDisplay from "~/components/events/ExceptionDisplay";
import BreadcrumbsTimeline from "~/components/events/BreadcrumbsTimeline";
import ContextPanels from "~/components/events/ContextPanels";
import TagsTable from "~/components/events/TagsTable";

export default function EventDetail() {
  const params = useParams<{
    project: string;
    issueId: string;
    eventId: string;
  }>();

  const [showRaw, setShowRaw] = createSignal(false);

  const eventQuery = createQuery(() => ({
    queryKey: queryKeys.events.detail(params.eventId),
    queryFn: () => api.get<SentryEvent>(`/internal/events/${params.eventId}`),
  }));

  const parsedData = () => {
    if (!eventQuery.data) return null;
    try {
      return JSON.parse(eventQuery.data.data);
    } catch {
      return null;
    }
  };

  const exceptions = () => {
    const data = parsedData();
    if (!data) return [];
    // Sentry stores exceptions in exception.values
    if (data.exception?.values) return data.exception.values;
    // Some events have it at top level
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

  return (
    <div class="p-6">
      <div class="mb-4">
        <A
          href={`/${params.project}/issues/${params.issueId}`}
          class="text-sm text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)]"
        >
          &larr; Back to Issue
        </A>
      </div>

      <Show when={eventQuery.data} fallback={<LoadingSkeleton rows={10} />}>
        {(event) => (
          <>
            {/* Event metadata header */}
            <div class="mb-6">
              <div class="mb-2 flex items-center gap-2">
                <Badge level={event().level} />
                <Show when={event().platform}>
                  <span class="rounded bg-[var(--color-surface-2)] px-1.5 py-0.5 text-xs font-mono text-[var(--color-text-secondary)]">
                    {event().platform}
                  </span>
                </Show>
                <span class="text-sm text-[var(--color-text-secondary)]">
                  {relativeTime(event().timestamp)}
                </span>
              </div>
              <h1 class="text-xl font-bold text-[var(--color-text-primary)]">
                {event().title ?? event().message ?? "Event"}
              </h1>
              <div class="mt-2 flex flex-wrap gap-x-4 gap-y-1 text-xs text-[var(--color-text-secondary)] font-mono">
                <span>ID: {event().event_id}</span>
                <Show when={event().environment}>
                  <span>Env: {event().environment}</span>
                </Show>
                <Show when={event().release}>
                  <span>Release: {event().release}</span>
                </Show>
                <Show when={event().transaction_name}>
                  <span>Transaction: {event().transaction_name}</span>
                </Show>
              </div>
            </div>

            {/* Exception Display + Stack Trace */}
            <Show when={exceptions().length > 0}>
              <div class="mb-6">
                <ExceptionDisplay exceptions={exceptions()} />
              </div>
            </Show>

            {/* Breadcrumbs */}
            <Show when={breadcrumbs().length > 0}>
              <div class="mb-6">
                <BreadcrumbsTimeline breadcrumbs={breadcrumbs()} />
              </div>
            </Show>

            {/* Tags */}
            <Show when={tags().length > 0}>
              <div class="mb-6">
                <TagsTable tags={tags()} />
              </div>
            </Show>

            {/* Context Panels */}
            <div class="mb-6">
              <ContextPanels
                contexts={contexts()}
                request={request()}
                user={user()}
              />
            </div>

            {/* Raw JSON */}
            <div class="rounded-lg border border-[var(--color-border)] overflow-hidden">
              <button
                class="flex w-full items-center justify-between border-b border-[var(--color-border)] bg-[var(--color-surface-1)] px-4 py-3 text-left"
                onClick={() => setShowRaw(!showRaw())}
              >
                <h2 class="text-sm font-medium text-[var(--color-text-primary)]">
                  Raw JSON
                </h2>
                <span class="text-xs text-[var(--color-text-secondary)]">
                  {showRaw() ? "Hide" : "Show"}
                </span>
              </button>
              <Show when={showRaw()}>
                <pre class="max-h-[600px] overflow-auto p-4 text-xs text-[var(--color-text-primary)] bg-[var(--color-surface-0)]">
                  {JSON.stringify(parsedData(), null, 2)}
                </pre>
              </Show>
            </div>
          </>
        )}
      </Show>
    </div>
  );
}
