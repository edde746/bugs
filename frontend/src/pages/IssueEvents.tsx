import { A, useParams, useSearchParams } from "@solidjs/router";
import { createQuery } from "@tanstack/solid-query";
import { For, Show } from "solid-js";
import { api } from "~/api/client";
import { queryKeys } from "~/queries/keys";
import type { EventListResponse } from "~/lib/sentry-types";
import { relativeTime } from "~/lib/formatters";
import Badge from "~/components/ui/Badge";
import Button from "~/components/ui/Button";
import LoadingSpinner from "~/components/ui/LoadingSpinner";
import EmptyState from "~/components/ui/EmptyState";
import ErrorState from "~/components/ui/ErrorState";
import IconArrowLeft from "~icons/lucide/arrow-left";
import IconArrowRight from "~icons/lucide/arrow-right";

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
    <div class="page">
      <A href={`/${params.project}/issues/${params.issueId}`} class="back-link">
        <IconArrowLeft /> Back to Issue
      </A>

      <div style={{ "margin-bottom": "24px" }}>
        <h1 class="page__title">Events</h1>
        <p class="page__subtitle">
          All events for issue #{params.issueId}
        </p>
      </div>

      <Show when={!eventsQuery.isPending} fallback={<LoadingSpinner />}>
        <Show
          when={!eventsQuery.isError}
          fallback={<ErrorState title="Couldn't load events" error={eventsQuery.error} />}
        >
        <Show
          when={eventsQuery.data && eventsQuery.data.events.length > 0}
          fallback={
            <EmptyState
              title="No events found"
              description="No events have been recorded for this issue yet."
            />
          }
        >
          <div class="card">
            <table class="data-table">
              <thead>
                <tr>
                  <th>Event ID</th>
                  <th>Timestamp</th>
                  <th>Level</th>
                </tr>
              </thead>
              <tbody>
                <For each={eventsQuery.data?.events}>
                  {(event) => (
                    <tr>
                      <td>
                        <A
                          href={`/${params.project}/issues/${params.issueId}/events/${event.id}`}
                          class="link-mono"
                        >
                          {event.event_id}
                        </A>
                      </td>
                      <td class="text-secondary">
                        {relativeTime(event.timestamp)}
                      </td>
                      <td>
                        <Badge level={event.level} />
                      </td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </div>

          <div class="table-footer">
            <div class="table-footer__count">
              Showing {eventsQuery.data?.events.length ?? 0} events
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
                    cursor: String(eventsQuery.data!.nextCursor),
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
