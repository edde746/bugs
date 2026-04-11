import { For, Show } from "solid-js";
import { clsx } from "clsx";
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

const LEVEL_STYLES: Record<string, { dot: string; badge: string }> = {
  error: {
    dot: "bg-red-500",
    badge: "bg-red-50 text-red-700 dark:bg-red-900/20 dark:text-red-400",
  },
  warning: {
    dot: "bg-amber-500",
    badge:
      "bg-amber-50 text-amber-700 dark:bg-amber-900/20 dark:text-amber-400",
  },
  info: {
    dot: "bg-blue-500",
    badge: "bg-blue-50 text-blue-700 dark:bg-blue-900/20 dark:text-blue-400",
  },
  debug: {
    dot: "bg-gray-400",
    badge: "bg-gray-50 text-gray-600 dark:bg-gray-800/40 dark:text-gray-400",
  },
};

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
    <div class="rounded-lg border border-[var(--color-border)] overflow-hidden">
      <div class="border-b border-[var(--color-border)] bg-[var(--color-surface-1)] px-4 py-2">
        <h3 class="text-sm font-medium text-[var(--color-text-primary)]">
          Breadcrumbs
        </h3>
      </div>
      <div class="max-h-96 overflow-y-auto">
        <div class="divide-y divide-[var(--color-border)]">
          <For each={sorted()}>
            {(crumb) => {
              const level = () => crumb.level ?? "info";
              const styles = () => LEVEL_STYLES[level()] ?? LEVEL_STYLES["info"]!;

              return (
                <div class="flex items-start gap-3 px-4 py-2 text-sm">
                  <div class="flex flex-col items-center pt-1.5">
                    <div
                      class={clsx("h-2 w-2 rounded-full", styles().dot)}
                    />
                  </div>
                  <div class="min-w-0 flex-1">
                    <div class="flex items-center gap-2">
                      <Show when={crumb.category}>
                        <span
                          class={clsx(
                            "inline-flex rounded px-1.5 py-0.5 text-[10px] font-medium",
                            styles().badge,
                          )}
                        >
                          {crumb.category}
                        </span>
                      </Show>
                      <span class="text-xs text-[var(--color-text-secondary)]">
                        {formatBreadcrumbTime(crumb.timestamp)}
                      </span>
                    </div>
                    <Show when={crumb.message}>
                      <p class="mt-0.5 text-xs text-[var(--color-text-primary)] break-all">
                        {crumb.message}
                      </p>
                    </Show>
                    <Show when={crumb.data && Object.keys(crumb.data!).length > 0}>
                      <pre class="mt-1 text-[10px] text-[var(--color-text-secondary)] break-all whitespace-pre-wrap">
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
    </div>
  );
}
