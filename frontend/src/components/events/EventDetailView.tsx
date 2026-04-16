import { createSignal, Show } from "solid-js";
import type { Event as SentryEvent } from "~/lib/sentry-types";
import { relativeTime } from "~/lib/formatters";
import {
  getBreadcrumbs,
  getContexts,
  getExceptions,
  getRequest,
  getTags,
  getThreads,
  getUser,
  parseEventData,
} from "~/lib/eventData";
import Badge from "~/components/ui/Badge";
import ErrorState from "~/components/ui/ErrorState";
import ExceptionDisplay from "./ExceptionDisplay";
import BreadcrumbsTimeline from "./BreadcrumbsTimeline";
import ContextPanels from "./ContextPanels";
import TagsTable from "./TagsTable";
import ThreadsDisplay from "./ThreadsDisplay";
import IconEye from "~icons/lucide/eye";
import IconEyeOff from "~icons/lucide/eye-off";
import type { ExceptionValue } from "./ExceptionDisplay";
import type { Breadcrumb } from "./BreadcrumbsTimeline";
import type { ThreadValue } from "./ThreadsDisplay";

interface EventDetailViewProps {
  event: SentryEvent;
  /** Render the h1 title. Defaults to true; set false if the caller already has its own header. */
  showHeader?: boolean;
}

/**
 * Shared body of an event detail page. Both EventDetail and
 * DirectEventDetail render this with only the surrounding chrome (back
 * link, modal placement) differing. IssueDetail doesn't use this — it
 * has significantly more UI (event nav, comments, activity) around the
 * same event data.
 */
export default function EventDetailView(props: EventDetailViewProps) {
  const [showRaw, setShowRaw] = createSignal(false);
  const parsed = () => parseEventData(props.event);

  const exceptions = () => getExceptions(parsed()) as ExceptionValue[];
  const breadcrumbs = () => getBreadcrumbs(parsed()) as Breadcrumb[];
  const threads = () => getThreads(parsed()) as ThreadValue[];
  const contexts = () => getContexts(parsed());
  const request = () => getRequest(parsed());
  const user = () => getUser(parsed());
  const tags = () => getTags(parsed());

  return (
    <div class="section-gap">
      <Show when={props.showHeader !== false}>
        <div>
          <div class="inline-gap" style={{ "margin-bottom": "8px" }}>
            <Badge level={props.event.level} />
            <Show when={props.event.platform}>
              <span class="meta-tag">{props.event.platform}</span>
            </Show>
            <span class="text-sm text-secondary">
              {relativeTime(props.event.timestamp)}
            </span>
          </div>
          <h1 class="page__title" style={{ "font-size": "20px" }}>
            {props.event.title ?? props.event.message ?? "Event"}
          </h1>
          <div class="meta-row" style={{ "margin-top": "8px" }}>
            <span>ID: {props.event.event_id}</span>
            <Show when={props.event.environment}>
              <span>Env: {props.event.environment}</span>
            </Show>
            <Show when={props.event.release}>
              <span>Release: {props.event.release}</span>
            </Show>
            <Show when={props.event.transaction_name}>
              <span>Transaction: {props.event.transaction_name}</span>
            </Show>
          </div>
        </div>
      </Show>

      {/* Surface a parse failure instead of pretending the event is empty. */}
      <Show when={!parsed().ok}>
        <ErrorState
          title="Couldn't display event details"
          description="The event body could not be parsed. Reporting from the underlying SDK may be malformed or truncated."
        />
      </Show>

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

      <ContextPanels contexts={contexts()} request={request()} user={user()} />

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
            {parsed().ok
              ? JSON.stringify((parsed() as { data: unknown }).data, null, 2)
              : "(unable to parse event body)"}
          </pre>
        </Show>
      </div>
    </div>
  );
}
