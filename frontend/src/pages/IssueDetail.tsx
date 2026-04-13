import { A, useParams } from "@solidjs/router";
import { createQuery, createMutation, useQueryClient } from "@tanstack/solid-query";
import { createSignal, Show, For } from "solid-js";
import { api } from "~/api/client";
import { queryKeys } from "~/queries/keys";
import type { Issue, UpdateIssueInput, EventListResponse, IssueComment, IssueActivity } from "~/lib/sentry-types";
import { relativeTime, formatNumber } from "~/lib/formatters";
import { STATUS_LABELS } from "~/lib/constants";
import Badge from "~/components/ui/Badge";
import Button from "~/components/ui/Button";
import LoadingSpinner from "~/components/ui/LoadingSpinner";
import ExceptionDisplay from "~/components/events/ExceptionDisplay";
import BreadcrumbsTimeline from "~/components/events/BreadcrumbsTimeline";
import ContextPanels from "~/components/events/ContextPanels";
import TagsTable from "~/components/events/TagsTable";
import CopyButton from "~/components/ui/CopyButton";
import IconArrowLeft from "~icons/lucide/arrow-left";
import IconArrowRight from "~icons/lucide/arrow-right";
import IconEye from "~icons/lucide/eye";
import IconEyeOff from "~icons/lucide/eye-off";
import type { ExceptionValue } from "~/components/events/ExceptionDisplay";
import type { Breadcrumb } from "~/components/events/BreadcrumbsTimeline";

const ACTIVITY_LABELS: Record<string, string> = {
  first_seen: "Issue first seen",
  resolved: "Marked as resolved",
  unresolved: "Marked as unresolved",
  ignored: "Ignored",
  unignored: "Unignored",
  regression: "Regression detected",
};

function activityKindLabel(kind: string): string {
  return ACTIVITY_LABELS[kind] ?? kind;
}

export default function IssueDetail() {
  const params = useParams<{ project: string; issueId: string }>();
  const queryClient = useQueryClient();
  const [eventIndex, setEventIndex] = createSignal(0);
  const [showRaw, setShowRaw] = createSignal(false);

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

  // Comments
  const [commentText, setCommentText] = createSignal("");

  const commentsQuery = createQuery(() => ({
    queryKey: queryKeys.comments.list(params.issueId),
    queryFn: () => api.get<IssueComment[]>(`/internal/issues/${params.issueId}/comments`),
    enabled: !!issueQuery.data,
  }));

  const addCommentMutation = createMutation(() => ({
    mutationFn: (text: string) =>
      api.post<IssueComment>(`/internal/issues/${params.issueId}/comments`, { text }),
    onSuccess: () => {
      setCommentText("");
      queryClient.invalidateQueries({ queryKey: queryKeys.comments.list(params.issueId) });
    },
  }));

  const deleteCommentMutation = createMutation(() => ({
    mutationFn: (id: number) => api.delete(`/internal/comments/${id}`),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.comments.list(params.issueId) });
    },
  }));

  const handleAddComment = () => {
    const text = commentText().trim();
    if (text) addCommentMutation.mutate(text);
  };

  const activityQuery = createQuery(() => ({
    queryKey: queryKeys.activity.list(params.issueId),
    queryFn: () => api.get<IssueActivity[]>(`/internal/issues/${params.issueId}/activity`),
    enabled: !!issueQuery.data,
  }));

  const canGoNewer = () => eventIndex() > 0;
  const canGoOlder = () => {
    const events = eventsQuery.data?.events;
    return events ? eventIndex() < events.length - 1 : false;
  };

  const buildMarkdown = (): string => {
    const parts: string[] = [];
    const iss = issueQuery.data;
    if (!iss) return "";

    // Issue header
    parts.push(`# ${iss.title}`);
    parts.push("");
    parts.push(`- **Level:** ${iss.level}`);
    parts.push(`- **Status:** ${STATUS_LABELS[iss.status] ?? iss.status}`);
    if (iss.culprit) parts.push(`- **Culprit:** ${iss.culprit}`);
    parts.push(`- **Events:** ${formatNumber(iss.event_count)}`);
    parts.push(`- **First Seen:** ${iss.first_seen}`);
    parts.push(`- **Last Seen:** ${iss.last_seen}`);

    // Event metadata
    const ev = currentEvent();
    if (ev) {
      parts.push("");
      parts.push("## Event");
      parts.push("");
      parts.push(`- **Event ID:** ${ev.event_id}`);
      parts.push(`- **Timestamp:** ${ev.timestamp}`);
      if (ev.platform) parts.push(`- **Platform:** ${ev.platform}`);
    }

    // Exceptions + stack traces
    const excs = exceptions() as ExceptionValue[];
    for (const exc of excs) {
      parts.push("");
      parts.push(`## Exception: ${exc.type ?? "Error"}`);
      if (exc.value) {
        parts.push("");
        parts.push(exc.value);
      }
      if (exc.mechanism) {
        const handled = exc.mechanism.handled === false ? " (unhandled)" : "";
        parts.push("");
        parts.push(`- **Mechanism:** ${exc.mechanism.type ?? "generic"}${handled}`);
      }
      const frames = exc.stacktrace?.frames;
      if (frames && frames.length > 0) {
        parts.push("");
        parts.push("### Stack Trace");
        parts.push("");
        parts.push("| | Function | File |");
        parts.push("|---|---|---|");
        const reversed = [...frames].reverse();
        for (const frame of reversed) {
          const tag = frame.in_app ? "app" : "";
          const fn = frame.function ?? "<anonymous>";
          const file = frame.filename ?? frame.abs_path ?? frame.module ?? "unknown";
          let loc = file;
          if (frame.lineno != null) {
            loc += `:${frame.lineno}`;
            if (frame.colno != null) loc += `:${frame.colno}`;
          }
          parts.push(`| ${tag} | ${fn} | ${loc} |`);
        }
      }
    }

    // Breadcrumbs
    const crumbs = breadcrumbs() as Breadcrumb[];
    if (crumbs.length > 0) {
      const sorted = [...crumbs].sort((a, b) => {
        const aTs = typeof a.timestamp === "number" ? a.timestamp : new Date(a.timestamp ?? 0).getTime() / 1000;
        const bTs = typeof b.timestamp === "number" ? b.timestamp : new Date(b.timestamp ?? 0).getTime() / 1000;
        return aTs - bTs;
      });
      parts.push("");
      parts.push("## Breadcrumbs");
      parts.push("");
      parts.push("| Time | Category | Level | Message |");
      parts.push("|---|---|---|---|");
      for (const crumb of sorted) {
        const ts = crumb.timestamp != null ? String(crumb.timestamp) : "";
        const cat = crumb.category ?? "";
        const lvl = crumb.level ?? "info";
        const msg = crumb.message ?? "";
        parts.push(`| ${ts} | ${cat} | ${lvl} | ${msg} |`);
        if (crumb.data && Object.keys(crumb.data).length > 0) {
          parts.push("");
          parts.push("```json");
          parts.push(JSON.stringify(crumb.data, null, 2));
          parts.push("```");
          parts.push("");
        }
      }
    }

    // Tags
    const t = tags() as Array<{ key: string; value: string }>;
    if (t.length > 0) {
      parts.push("");
      parts.push("## Tags");
      parts.push("");
      parts.push("| Key | Value |");
      parts.push("|---|---|");
      for (const tag of t) {
        parts.push(`| ${tag.key} | ${tag.value} |`);
      }
    }

    // Context panels (same ordering as ContextPanels.tsx)
    const u = user() as Record<string, unknown> | null;
    if (u && Object.keys(u).length > 0) {
      parts.push("");
      parts.push("## User");
      parts.push("");
      parts.push("| Key | Value |");
      parts.push("|---|---|");
      for (const [key, value] of Object.entries(u)) {
        parts.push(`| ${key} | ${typeof value === "object" ? JSON.stringify(value) : String(value ?? "")} |`);
      }
    }

    const ctx = contexts() as Record<string, Record<string, unknown>>;
    if (ctx) {
      const order = ["browser", "os", "device", "runtime"];
      const rendered = new Set<string>();
      const renderContext = (key: string, data: Record<string, unknown>) => {
        if (rendered.has(key) || Object.keys(data).length === 0) return;
        rendered.add(key);
        const label = key.charAt(0).toUpperCase() + key.slice(1);
        parts.push("");
        parts.push(`## ${label}`);
        parts.push("");
        parts.push("| Key | Value |");
        parts.push("|---|---|");
        for (const [k, v] of Object.entries(data)) {
          parts.push(`| ${k} | ${typeof v === "object" ? JSON.stringify(v) : String(v ?? "")} |`);
        }
      };
      for (const key of order) {
        if (ctx[key]) renderContext(key, ctx[key]);
      }
      for (const [key, value] of Object.entries(ctx)) {
        if (!order.includes(key) && value) renderContext(key, value);
      }
    }

    // Request
    const req = request() as { method?: string; url?: string; headers?: Record<string, string>; query_string?: string; data?: unknown; env?: Record<string, string> } | null;
    if (req) {
      const reqEntries: [string, string][] = [];
      if (req.method) reqEntries.push(["method", req.method]);
      if (req.url) reqEntries.push(["url", req.url]);
      if (req.query_string) reqEntries.push(["query_string", req.query_string]);
      if (req.headers) {
        for (const [hk, hv] of Object.entries(req.headers)) {
          reqEntries.push([`header: ${hk}`, hv]);
        }
      }
      if (req.env) {
        for (const [ek, ev] of Object.entries(req.env)) {
          reqEntries.push([`env: ${ek}`, ev]);
        }
      }
      if (reqEntries.length > 0) {
        parts.push("");
        parts.push("## Request");
        parts.push("");
        parts.push("| Key | Value |");
        parts.push("|---|---|");
        for (const [k, v] of reqEntries) {
          parts.push(`| ${k} | ${v} |`);
        }
      }
    }

    return parts.join("\n");
  };

  return (
    <div class="page">
      <A href={`/${params.project}/issues`} class="back-link">
        <IconArrowLeft /> Back to Issues
      </A>

      <Show when={issueQuery.data} fallback={<LoadingSpinner />}>
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
                <CopyButton text={buildMarkdown()} label="Copy as Markdown" />
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
                <div class="stat-card__value">
                  {relativeTime(issue().first_seen)}
                </div>
              </div>
              <div class="stat-card">
                <div class="stat-card__label">Last Seen</div>
                <div class="stat-card__value">
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
              </div>
            </div>

            <Show when={!eventsQuery.isPending} fallback={<LoadingSpinner />}>
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
            </Show>

            {/* Activity timeline */}
            <Show when={activityQuery.data && activityQuery.data.length > 0}>
              <div class="card" style={{ "margin-top": "24px" }}>
                <div class="card__header">
                  <h3>Activity</h3>
                  <span class="text-xs text-secondary">{activityQuery.data!.length} events</span>
                </div>
                <div class="activity-timeline">
                  <For each={activityQuery.data}>
                    {(item) => (
                      <div class="activity-item">
                        <span class="activity-item__dot" data-kind={item.kind} />
                        <span class="activity-item__label">{activityKindLabel(item.kind)}</span>
                        <span class="activity-item__time text-secondary text-sm">{relativeTime(item.created_at)}</span>
                      </div>
                    )}
                  </For>
                </div>
              </div>
            </Show>

            {/* Comments section */}
            <div class="card" style={{ "margin-top": "24px" }}>
              <div class="card__header">
                <h3>Comments</h3>
                <Show when={commentsQuery.data}>
                  <span class="text-xs text-secondary">
                    {commentsQuery.data!.length} comment{commentsQuery.data!.length !== 1 ? "s" : ""}
                  </span>
                </Show>
              </div>

              <Show when={commentsQuery.data && commentsQuery.data.length > 0}>
                <div class="card__body comment-list">
                  <For each={commentsQuery.data}>
                    {(comment) => (
                      <div class="comment">
                        <div class="comment__text">{comment.text}</div>
                        <div class="comment__meta">
                          <span class="text-secondary text-sm">{relativeTime(comment.created_at)}</span>
                          <button
                            class="comment__delete"
                            onClick={() => deleteCommentMutation.mutate(comment.id)}
                          >
                            Delete
                          </button>
                        </div>
                      </div>
                    )}
                  </For>
                </div>
              </Show>

              <div class="comment-compose">
                <textarea
                  class="comment__input"
                  placeholder="Add a comment..."
                  value={commentText()}
                  onInput={(e) => setCommentText(e.currentTarget.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                      handleAddComment();
                    }
                  }}
                  rows={2}
                />
                <div class="comment-compose__actions">
                  <span class="text-xs text-secondary">Ctrl+Enter to post</span>
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={handleAddComment}
                    disabled={addCommentMutation.isPending || !commentText().trim()}
                  >
                    Post
                  </Button>
                </div>
              </div>
            </div>
          </>
        )}
      </Show>
    </div>
  );
}
