import { useParams } from "@solidjs/router";
import { createQuery } from "@tanstack/solid-query";
import { createSignal, Show } from "solid-js";
import { api } from "~/api/client";
import { queryKeys } from "~/queries/keys";
import type { Event as SentryEvent } from "~/lib/sentry-types";
import { relativeTime } from "~/lib/formatters";
import Badge from "~/components/ui/Badge";
import LoadingSpinner from "~/components/ui/LoadingSpinner";
import ExceptionDisplay from "~/components/events/ExceptionDisplay";
import BreadcrumbsTimeline from "~/components/events/BreadcrumbsTimeline";
import ContextPanels from "~/components/events/ContextPanels";
import TagsTable from "~/components/events/TagsTable";
import ThreadsDisplay from "~/components/events/ThreadsDisplay";
import IconEye from "~icons/lucide/eye";
import IconEyeOff from "~icons/lucide/eye-off";

export default function DirectEventDetail() {
  const params = useParams<{ eventId: string }>();

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

  const threads = () => {
    const data = parsedData();
    return data?.threads?.values ?? [];
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
    <div class="page">
      <Show when={eventQuery.data} fallback={<LoadingSpinner />}>
        {(event) => (
          <div class="section-gap">
            <div>
              <div class="inline-gap" style={{ "margin-bottom": "8px" }}>
                <Badge level={event().level} />
                <Show when={event().platform}>
                  <span class="meta-tag">{event().platform}</span>
                </Show>
                <span class="text-sm text-secondary">
                  {relativeTime(event().timestamp)}
                </span>
              </div>
              <h1 class="page__title" style={{ "font-size": "20px" }}>
                {event().title ?? event().message ?? "Event"}
              </h1>
              <div class="meta-row" style={{ "margin-top": "8px" }}>
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

            <Show when={exceptions().length > 0}>
              <ExceptionDisplay exceptions={exceptions()} />
            </Show>

            <Show when={threads().length > 0}>
              <ThreadsDisplay threads={threads()} />
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

            <div class="raw-json">
              <button
                class="raw-json__toggle"
                onClick={() => setShowRaw(!showRaw())}
              >
                <span class="raw-json__toggle-label">Raw JSON</span>
                <span class="raw-json__toggle-icon">
                  {showRaw() ? <IconEyeOff /> : <IconEye />}
                </span>
              </button>
              <Show when={showRaw()}>
                <pre class="raw-json__content">
                  {JSON.stringify(parsedData(), null, 2)}
                </pre>
              </Show>
            </div>
          </div>
        )}
      </Show>
    </div>
  );
}
