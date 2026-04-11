import { A, useParams, useSearchParams } from "@solidjs/router";
import { createQuery } from "@tanstack/solid-query";
import { For, Show } from "solid-js";
import { api } from "~/api/client";
import { queryKeys } from "~/queries/keys";
import type { EventListResponse } from "~/lib/sentry-types";
import { relativeTime } from "~/lib/formatters";
import Badge from "~/components/ui/Badge";
import Button from "~/components/ui/Button";
import LoadingSkeleton from "~/components/ui/LoadingSkeleton";
import EmptyState from "~/components/ui/EmptyState";

export default function IssueEvents() {
  const params = useParams<{ project: string; issueId: string }>();
  const [searchParams, setSearchParams] = useSearchParams<{
    cursor?: string;
  }>();

  const cursor = () => searchParams.cursor ?? "";

  const eventsQuery = createQuery(() => ({
    queryKey: [...queryKeys.events.list(params.issueId), "all", cursor()],
    queryFn: () => {
      let url = `/internal/issues/${params.issueId}/events?limit=50`;
      if (cursor()) url += `&cursor=${cursor()}`;
      return api.get<EventListResponse>(url);
    },
  }));

  const hasPrev = () => !!cursor();
  const hasNext = () => !!eventsQuery.data?.nextCursor;

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

      <div class="mb-6">
        <h1 class="text-2xl font-bold text-[var(--color-text-primary)]">
          Events
        </h1>
        <p class="mt-1 text-sm text-[var(--color-text-secondary)]">
          All events for issue #{params.issueId}
        </p>
      </div>

      <Show when={!eventsQuery.isPending} fallback={<LoadingSkeleton rows={8} />}>
        <Show
          when={eventsQuery.data && eventsQuery.data.events.length > 0}
          fallback={
            <EmptyState
              title="No events found"
              description="No events have been recorded for this issue yet."
            />
          }
        >
          <div class="overflow-hidden rounded-lg border border-[var(--color-border)]">
            <table class="w-full">
              <thead>
                <tr class="border-b border-[var(--color-border)] bg-[var(--color-surface-1)]">
                  <th class="px-4 py-2 text-left text-xs font-medium text-[var(--color-text-secondary)]">
                    Event ID
                  </th>
                  <th class="px-4 py-2 text-left text-xs font-medium text-[var(--color-text-secondary)]">
                    Timestamp
                  </th>
                  <th class="px-4 py-2 text-left text-xs font-medium text-[var(--color-text-secondary)]">
                    Level
                  </th>
                </tr>
              </thead>
              <tbody>
                <For each={eventsQuery.data?.events}>
                  {(event) => (
                    <tr class="border-b border-[var(--color-border)] transition-colors hover:bg-[var(--color-surface-1)]">
                      <td class="px-4 py-3">
                        <A
                          href={`/${params.project}/issues/${params.issueId}/events/${event.id}`}
                          class="font-mono text-sm text-indigo-600 hover:text-indigo-800 dark:text-indigo-400 dark:hover:text-indigo-300"
                        >
                          {event.event_id}
                        </A>
                      </td>
                      <td class="px-4 py-3 text-sm text-[var(--color-text-secondary)]">
                        {relativeTime(event.timestamp)}
                      </td>
                      <td class="px-4 py-3">
                        <Badge level={event.level} />
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
              Showing {eventsQuery.data?.events.length ?? 0} events
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
                    cursor: String(eventsQuery.data!.nextCursor),
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
