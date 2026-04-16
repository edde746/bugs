// Centralized parser for the `event.data` JSON blob that's stored as a
// string on the backend. Every event-detail page was duplicating this
// parsing + accessor logic, so moving it here keeps behavior and error
// handling consistent and lets us surface parse failures instead of
// silently returning nothing.
//
// The Sentry event shape is extremely loose (different SDKs include
// different fields and structures), so we return `unknown`-typed blobs
// with narrow accessor helpers rather than a full type definition. Use
// the helpers below; avoid reading straight off `parsed.data`.

import type { Event as SentryEvent } from "./sentry-types";

export interface ParseSuccess {
  ok: true;
  // Raw parsed object. Prefer the accessors below over direct access.
  data: Record<string, unknown>;
}

export interface ParseFailure {
  ok: false;
  reason: "empty" | "invalid_json";
  error?: Error;
}

export type ParsedEvent = ParseSuccess | ParseFailure;

export function parseEventData(event: SentryEvent | undefined | null): ParsedEvent {
  if (!event) return { ok: false, reason: "empty" };
  if (!event.data) return { ok: false, reason: "empty" };
  try {
    const parsed = JSON.parse(event.data);
    if (parsed === null || typeof parsed !== "object") {
      return { ok: false, reason: "invalid_json" };
    }
    return { ok: true, data: parsed as Record<string, unknown> };
  } catch (e) {
    return {
      ok: false,
      reason: "invalid_json",
      error: e instanceof Error ? e : new Error(String(e)),
    };
  }
}

// --- Accessors ---------------------------------------------------------
// All accessors take the result of `parseEventData`; they return empty
// collections / nulls for failed parses so callers don't need to branch
// twice.

function asObject(v: unknown): Record<string, unknown> | null {
  return v && typeof v === "object" && !Array.isArray(v)
    ? (v as Record<string, unknown>)
    : null;
}

function asArray(v: unknown): unknown[] {
  return Array.isArray(v) ? v : [];
}

export function getExceptions(p: ParsedEvent): unknown[] {
  if (!p.ok) return [];
  const exception = asObject(p.data.exception);
  if (exception && Array.isArray(exception.values)) return exception.values;
  if (Array.isArray(p.data.exceptions)) return p.data.exceptions as unknown[];
  return [];
}

export function getBreadcrumbs(p: ParsedEvent): unknown[] {
  if (!p.ok) return [];
  const breadcrumbs = p.data.breadcrumbs;
  const obj = asObject(breadcrumbs);
  if (obj && Array.isArray(obj.values)) return obj.values;
  return asArray(breadcrumbs);
}

export function getContexts(p: ParsedEvent): Record<string, unknown> {
  if (!p.ok) return {};
  return asObject(p.data.contexts) ?? {};
}

export function getRequest(p: ParsedEvent): Record<string, unknown> | null {
  if (!p.ok) return null;
  return asObject(p.data.request);
}

export function getUser(p: ParsedEvent): Record<string, unknown> | null {
  if (!p.ok) return null;
  return asObject(p.data.user);
}

export function getThreads(p: ParsedEvent): unknown[] {
  if (!p.ok) return [];
  const threads = asObject(p.data.threads);
  if (threads && Array.isArray(threads.values)) return threads.values;
  return [];
}

export interface TagEntry {
  key: string;
  value: string;
}

export function getTags(p: ParsedEvent): TagEntry[] {
  if (!p.ok) return [];
  const tags = p.data.tags;
  if (!tags) return [];
  if (Array.isArray(tags)) {
    return tags
      .map((t) => {
        if (t && typeof t === "object" && "key" in t && "value" in t) {
          const entry = t as { key: unknown; value: unknown };
          return { key: String(entry.key), value: String(entry.value) };
        }
        if (Array.isArray(t) && t.length === 2) {
          return { key: String(t[0]), value: String(t[1]) };
        }
        return null;
      })
      .filter((t): t is TagEntry => t !== null);
  }
  const obj = asObject(tags);
  if (!obj) return [];
  return Object.entries(obj).map(([key, value]) => ({
    key,
    value: String(value),
  }));
}

export function getSdk(
  p: ParsedEvent,
): {
  name?: string;
  version?: string;
  integrations?: string[];
  packages?: Array<{ name: string; version: string }>;
} | null {
  if (!p.ok) return null;
  return (asObject(p.data.sdk) as never) ?? null;
}

export function getFingerprint(p: ParsedEvent): string[] {
  if (!p.ok) return [];
  return Array.isArray(p.data.fingerprint)
    ? (p.data.fingerprint as string[])
    : [];
}

export function getExtra(p: ParsedEvent): Record<string, unknown> | null {
  if (!p.ok) return null;
  return asObject(p.data.extra);
}

export function getMessage(p: ParsedEvent): string | null {
  if (!p.ok) return null;
  if (typeof p.data.message === "string") return p.data.message;
  const logentry = asObject(p.data.logentry);
  if (logentry) {
    if (typeof logentry.formatted === "string") return logentry.formatted;
    if (typeof logentry.message === "string") return logentry.message;
  }
  return null;
}
