import { A, useParams } from "@solidjs/router";
import { createQuery } from "@tanstack/solid-query";
import { Show } from "solid-js";
import { api } from "~/api/client";
import { queryKeys } from "~/queries/keys";
import type { Event as SentryEvent } from "~/lib/sentry-types";
import { relativeTime } from "~/lib/formatters";
import Badge from "~/components/ui/Badge";
import LoadingSkeleton from "~/components/ui/LoadingSkeleton";

export default function EventDetail() {
  const params = useParams<{
    project: string;
    issueId: string;
    eventId: string;
  }>();

  const eventQuery = createQuery(() => ({
    queryKey: queryKeys.events.detail(params.eventId),
    queryFn: () => api.get<SentryEvent>(`/internal/events/${params.eventId}`),
  }));

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
        {(event) => {
          let parsedData: unknown;
          try {
            parsedData = JSON.parse(event().data);
          } catch {
            parsedData = event().data;
          }

          return (
            <>
              <div class="mb-6">
                <div class="mb-2 flex items-center gap-2">
                  <Badge level={event().level} />
                  <span class="text-sm text-[var(--color-text-secondary)]">
                    {relativeTime(event().timestamp)}
                  </span>
                </div>
                <h1 class="text-xl font-bold text-[var(--color-text-primary)]">
                  {event().title ?? event().message ?? "Event"}
                </h1>
                <div class="mt-2 flex flex-wrap gap-x-4 gap-y-1 text-sm text-[var(--color-text-secondary)]">
                  <span>Event ID: {event().event_id}</span>
                  {event().platform && <span>Platform: {event().platform}</span>}
                  {event().environment && (
                    <span>Environment: {event().environment}</span>
                  )}
                  {event().release && <span>Release: {event().release}</span>}
                </div>
              </div>

              <div class="rounded-lg border border-[var(--color-border)]">
                <div class="border-b border-[var(--color-border)] bg-[var(--color-surface-1)] px-4 py-3">
                  <h2 class="text-sm font-medium text-[var(--color-text-primary)]">
                    Event Data
                  </h2>
                </div>
                <pre class="overflow-auto p-4 text-xs text-[var(--color-text-primary)]">
                  {JSON.stringify(parsedData, null, 2)}
                </pre>
              </div>
            </>
          );
        }}
      </Show>
    </div>
  );
}
