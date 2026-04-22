import { createMemo, createSignal, For, Show } from "solid-js";
import StacktraceViewer from "./StacktraceViewer";
import type { DebugImage, StackFrame } from "./StacktraceViewer";
import IconChevronDown from "~icons/lucide/chevron-down";
import IconChevronRight from "~icons/lucide/chevron-right";

export interface ThreadValue {
  id?: unknown;
  name?: string;
  crashed?: boolean;
  current?: boolean;
  main?: boolean;
  state?: string;
  held_locks?: Record<string, { address: string; class_name: string; package_name: string; type: number }>;
  stacktrace?: {
    frames?: StackFrame[];
    snapshot?: boolean;
  };
}

interface ThreadsDisplayProps {
  threads: ThreadValue[];
  images?: DebugImage[];
}

export default function ThreadsDisplay(props: ThreadsDisplayProps) {
  const sorted = createMemo(() => {
    const threads = [...props.threads];
    threads.sort((a, b) => {
      if (a.crashed && !b.crashed) return -1;
      if (!a.crashed && b.crashed) return 1;
      if (a.current && !b.current) return -1;
      if (!a.current && b.current) return 1;
      if (a.main && !b.main) return -1;
      if (!a.main && b.main) return 1;
      return 0;
    });
    return threads;
  });

  const [expandedThreads, setExpandedThreads] = createSignal<Set<number>>(
    new Set(sorted().map((t, i) => (t.crashed || t.current) ? i : -1).filter(i => i >= 0))
  );
  const [cardExpanded, setCardExpanded] = createSignal(false);

  const toggleThread = (index: number) => {
    const current = new Set(expandedThreads());
    if (current.has(index)) {
      current.delete(index);
    } else {
      current.add(index);
    }
    setExpandedThreads(current);
  };

  return (
    <div class="threads">
      <button
        classList={{ threads__header: true, "threads__header--open": cardExpanded() }}
        onClick={() => setCardExpanded(!cardExpanded())}
      >
        <span class="threads__toggle-icon">
          {cardExpanded() ? <IconChevronDown /> : <IconChevronRight />}
        </span>
        <h3>Threads ({props.threads.length})</h3>
      </button>
      <Show when={cardExpanded()}>
        <For each={sorted()}>
          {(thread, index) => {
            const isExpanded = () => expandedThreads().has(index());
            const hasFrames = () => thread.stacktrace?.frames && thread.stacktrace.frames.length > 0;
            const hasLocks = () => thread.held_locks && Object.keys(thread.held_locks).length > 0;
            const canExpand = () => hasFrames() || hasLocks();

            const threadLabel = () => (
              <>
                <span class="thread__name">
                  Thread {thread.id != null ? `#${thread.id}` : ""}{thread.name ? ` — ${thread.name}` : ""}
                </span>
                <Show when={thread.crashed}>
                  <span class="thread__crashed-tag">crashed</span>
                </Show>
                <Show when={thread.current}>
                  <span class="thread__current-tag">current</span>
                </Show>
                <Show when={thread.main}>
                  <span class="thread__main-tag">main</span>
                </Show>
                <Show when={thread.state}>
                  <span class="thread__state-tag">{thread.state}</span>
                </Show>
                <Show when={!hasFrames()}>
                  <span class="thread__no-frames">no frames</span>
                </Show>
              </>
            );

            return (
              <div class="thread" data-crashed={thread.crashed ?? false}>
                <Show
                  when={canExpand()}
                  fallback={
                    <div class="thread__header thread__header--static">
                      {threadLabel()}
                    </div>
                  }
                >
                  <button
                    class="thread__header"
                    onClick={() => toggleThread(index())}
                  >
                    <span class="thread__toggle-icon">
                      {isExpanded() ? <IconChevronDown /> : <IconChevronRight />}
                    </span>
                    {threadLabel()}
                  </button>
                </Show>
                <Show when={isExpanded() && canExpand()}>
                  <div class="thread__body">
                    <Show when={hasLocks()}>
                      <div class="thread__locks">
                        <For each={Object.values(thread.held_locks!)}>
                          {(lock) => (
                            <span class="thread__lock-badge" title={lock.address}>
                              holds {lock.package_name}.{lock.class_name}
                            </span>
                          )}
                        </For>
                      </div>
                    </Show>
                    <Show when={hasFrames()}>
                      <StacktraceViewer frames={thread.stacktrace!.frames!} images={props.images} />
                    </Show>
                  </div>
                </Show>
              </div>
            );
          }}
        </For>
      </Show>
    </div>
  );
}
