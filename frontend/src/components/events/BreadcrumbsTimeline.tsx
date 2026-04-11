import { For, Show } from "solid-js";
import { relativeTime } from "~/lib/formatters";

export interface Breadcrumb {
  type?: string;
  category?: string;
  message?: string;
  data?: Record<string, unknown>;
  level?: string;
  timestamp?: string | number;
}

interface BreadcrumbsTimelineProps {
  breadcrumbs: Breadcrumb[];
}

function formatBreadcrumbTime(ts: string | number | undefined): string {
  if (!ts) return "";
  const d = typeof ts === "number" ? new Date(ts * 1000) : new Date(ts);
  if (isNaN(d.getTime())) return "";
  return relativeTime(d.toISOString());
}

export default function BreadcrumbsTimeline(props: BreadcrumbsTimelineProps) {
  const sorted = () =>
    [...props.breadcrumbs].sort((a, b) => {
      const aTs = typeof a.timestamp === "number" ? a.timestamp : new Date(a.timestamp ?? 0).getTime() / 1000;
      const bTs = typeof b.timestamp === "number" ? b.timestamp : new Date(b.timestamp ?? 0).getTime() / 1000;
      return aTs - bTs;
    });

  return (
    <div class="breadcrumbs">
      <div class="breadcrumbs__header">
        <h3>Breadcrumbs</h3>
      </div>
      <div class="breadcrumbs__list">
        <For each={sorted()}>
          {(crumb) => {
            const level = () => crumb.level ?? "info";

            return (
              <div class="breadcrumb">
                <div class="breadcrumb__dot-col">
                  <div class="breadcrumb__dot" data-level={level()} />
                </div>
                <div class="breadcrumb__body">
                  <div class="breadcrumb__meta">
                    <Show when={crumb.category}>
                      <span class="breadcrumb__badge" data-level={level()}>
                        {crumb.category}
                      </span>
                    </Show>
                    <span class="breadcrumb__time">
                      {formatBreadcrumbTime(crumb.timestamp)}
                    </span>
                  </div>
                  <Show when={crumb.message}>
                    <p class="breadcrumb__message">{crumb.message}</p>
                  </Show>
                  <Show when={crumb.data && Object.keys(crumb.data!).length > 0}>
                    <pre class="breadcrumb__data">
                      {JSON.stringify(crumb.data, null, 2)}
                    </pre>
                  </Show>
                </div>
              </div>
            );
          }}
        </For>
      </div>
    </div>
  );
}
