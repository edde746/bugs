import { useParams } from "@solidjs/router";
import { createQuery } from "@tanstack/solid-query";
import { Show } from "solid-js";
import { api } from "~/api/client";
import { queryKeys } from "~/queries/keys";
import type { Event as SentryEvent } from "~/lib/sentry-types";
import LoadingSpinner from "~/components/ui/LoadingSpinner";
import ErrorState from "~/components/ui/ErrorState";
import EventDetailView from "~/components/events/EventDetailView";

export default function DirectEventDetail() {
  const params = useParams<{ eventId: string }>();

  const eventQuery = createQuery(() => ({
    queryKey: queryKeys.events.detail(params.eventId),
    queryFn: ({ signal }) =>
      api.get<SentryEvent>(`/internal/events/${params.eventId}`, signal),
  }));

  return (
    <div class="page">
      <Show when={!eventQuery.isError} fallback={<ErrorState error={eventQuery.error} />}>
        <Show when={eventQuery.data} fallback={<LoadingSpinner />}>
          {(event) => <EventDetailView event={event()} />}
        </Show>
      </Show>
    </div>
  );
}
