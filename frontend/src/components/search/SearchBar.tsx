import { createSignal, Show, For, onCleanup, createEffect } from "solid-js";
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
  // Arrow-key selection index. -1 = no keyboard focus, use mouse only.
  const [activeIndex, setActiveIndex] = createSignal(-1);

  let debounceTimer: ReturnType<typeof setTimeout> | undefined;

  onCleanup(() => {
    if (debounceTimer) clearTimeout(debounceTimer);
  });

  // Reset the keyboard-focus index any time results change so we don't
  // land on a stale row.
  createEffect(() => {
    results();
    setActiveIndex(-1);
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
    setActiveIndex(-1);
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

  const visibleResults = () => results().slice(0, 10);

  const handleKeyDown = (e: KeyboardEvent) => {
    const rows = visibleResults();
    if (rows.length === 0) return;
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        setActiveIndex((i) => (i + 1) % rows.length);
        if (!isOpen()) setIsOpen(true);
        break;
      case "ArrowUp":
        e.preventDefault();
        setActiveIndex((i) => (i <= 0 ? rows.length - 1 : i - 1));
        if (!isOpen()) setIsOpen(true);
        break;
      case "Enter": {
        const idx = activeIndex();
        if (isOpen() && idx >= 0 && idx < rows.length) {
          e.preventDefault();
          handleSelect(rows[idx]);
        }
        break;
      }
      case "Escape":
        setIsOpen(false);
        setActiveIndex(-1);
        break;
    }
  };

  return (
    <div class="search-bar">
      <input
        type="text"
        value={query()}
        onInput={(e) => handleInput(e.currentTarget.value)}
        onFocus={() => results().length > 0 && setIsOpen(true)}
        onBlur={handleBlur}
        onKeyDown={handleKeyDown}
        placeholder="Search events..."
        class="search-bar__input"
        role="combobox"
        aria-expanded={isOpen()}
        aria-controls="search-bar-results"
        aria-activedescendant={
          activeIndex() >= 0 ? `search-bar-result-${activeIndex()}` : undefined
        }
      />
      <Show when={loading()}>
        <div class="search-bar__spinner" />
      </Show>
      <Show when={isOpen() && visibleResults().length > 0}>
        <div
          class="search-bar__dropdown"
          id="search-bar-results"
          role="listbox"
        >
          <For each={visibleResults()}>
            {(result, i) => (
              <button
                id={`search-bar-result-${i()}`}
                class="search-bar__result"
                classList={{ "search-bar__result--active": activeIndex() === i() }}
                role="option"
                aria-selected={activeIndex() === i()}
                onMouseDown={() => handleSelect(result)}
                onMouseEnter={() => setActiveIndex(i())}
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
