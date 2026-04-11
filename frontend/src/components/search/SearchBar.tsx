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
    setTimeout(() => setIsOpen(false), 200);
  };

  return (
    <div class="search-bar">
      <input
        type="text"
        value={query()}
        onInput={(e) => handleInput(e.currentTarget.value)}
        onFocus={() => results().length > 0 && setIsOpen(true)}
        onBlur={handleBlur}
        placeholder="Search events..."
        class="search-bar__input"
      />
      <Show when={loading()}>
        <div class="search-bar__spinner" />
      </Show>
      <Show when={isOpen() && results().length > 0}>
        <div class="search-bar__dropdown">
          <For each={results().slice(0, 10)}>
            {(result) => (
              <button
                class="search-bar__result"
                onMouseDown={() => handleSelect(result)}
              >
                <Badge level={result.level} />
                <span class="search-bar__result-title">
                  {result.title ?? result.message ?? result.event_id}
                </span>
                <span class="search-bar__result-time">
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
