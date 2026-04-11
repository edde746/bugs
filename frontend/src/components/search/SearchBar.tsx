import { createSignal, Show, For, onCleanup } from "solid-js";
import { useNavigate, useParams } from "@solidjs/router";
import { relativeTime } from "~/lib/formatters";
import { api } from "~/api/client";
import type { Event as SentryEvent, SearchResponse } from "~/lib/sentry-types";
import Badge from "~/components/ui/Badge";

export default function SearchBar() {
  const params = useParams<{ project?: string }>();
  const navigate = useNavigate();
  const [query, setQuery] = createSignal("");
  const [results, setResults] = createSignal<SentryEvent[]>([]);
  const [isOpen, setIsOpen] = createSignal(false);
  const [loading, setLoading] = createSignal(false);

  let debounceTimer: ReturnType<typeof setTimeout> | undefined;

  onCleanup(() => {
    if (debounceTimer) clearTimeout(debounceTimer);
  });

  const doSearch = async (q: string) => {
    if (q.trim().length < 2) {
      setResults([]);
      setIsOpen(false);
      return;
    }
    setLoading(true);
    try {
      const projectParam = params.project ? `&project=${params.project}` : "";
      const response = await api.get<SearchResponse>(
        `/internal/search?q=${encodeURIComponent(q)}${projectParam}`,
      );
      setResults(response.results ?? []);
      setIsOpen(true);
    } catch {
      setResults([]);
    } finally {
      setLoading(false);
    }
  };

  const handleInput = (value: string) => {
    setQuery(value);
    if (debounceTimer) clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => doSearch(value), 300);
  };

  const handleSelect = (result: SentryEvent) => {
    setIsOpen(false);
    setQuery("");
    const project = params.project ?? "default";
    if (result.issue_id) {
      navigate(`/${project}/issues/${result.issue_id}/events/${result.id}`);
    } else {
      navigate(`/${project}/issues`);
    }
  };

  const handleBlur = () => {
    // Delay closing so clicks on results register
    setTimeout(() => setIsOpen(false), 200);
  };

  return (
    <div class="relative w-full max-w-md">
      <input
        type="text"
        value={query()}
        onInput={(e) => handleInput(e.currentTarget.value)}
        onFocus={() => results().length > 0 && setIsOpen(true)}
        onBlur={handleBlur}
        placeholder="Search events..."
        class="w-full rounded-md border border-[var(--color-border)] bg-[var(--color-surface-0)] px-3 py-1.5 text-sm text-[var(--color-text-primary)] placeholder:text-gray-400 focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
      />
      <Show when={loading()}>
        <div class="absolute right-3 top-1/2 -translate-y-1/2">
          <div class="h-3 w-3 animate-spin rounded-full border-2 border-gray-300 border-t-indigo-500" />
        </div>
      </Show>
      <Show when={isOpen() && results().length > 0}>
        <div class="absolute top-full left-0 z-50 mt-1 w-full rounded-lg border border-[var(--color-border)] bg-[var(--color-surface-0)] shadow-lg overflow-hidden">
          <For each={results().slice(0, 10)}>
            {(result) => (
              <button
                class="flex w-full items-center gap-2 px-3 py-2 text-left text-sm hover:bg-[var(--color-surface-1)] transition-colors"
                onMouseDown={() => handleSelect(result)}
              >
                <Badge level={result.level} />
                <span class="flex-1 truncate text-[var(--color-text-primary)]">
                  {result.title ?? result.message ?? result.event_id}
                </span>
                <span class="text-xs text-[var(--color-text-secondary)]">
                  {relativeTime(result.timestamp)}
                </span>
              </button>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}
