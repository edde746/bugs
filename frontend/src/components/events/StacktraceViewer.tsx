import { createSignal, For, Show } from "solid-js";
import { clsx } from "clsx";
import Button from "~/components/ui/Button";

export interface StackFrame {
  filename?: string;
  function?: string;
  module?: string;
  lineno?: number;
  colno?: number;
  abs_path?: string;
  in_app?: boolean;
  pre_context?: string[];
  context_line?: string;
  post_context?: string[];
}

interface StacktraceViewerProps {
  frames: StackFrame[];
}

export default function StacktraceViewer(props: StacktraceViewerProps) {
  const [expandedFrames, setExpandedFrames] = createSignal<Set<number>>(
    new Set([0]),
  );
  const [allExpanded, setAllExpanded] = createSignal(false);

  const reversedFrames = () => [...props.frames].reverse();

  const toggleFrame = (index: number) => {
    const current = new Set(expandedFrames());
    if (current.has(index)) {
      current.delete(index);
    } else {
      current.add(index);
    }
    setExpandedFrames(current);
  };

  const expandAll = () => {
    const all = new Set(reversedFrames().map((_, i) => i));
    setExpandedFrames(all);
    setAllExpanded(true);
  };

  const collapseAll = () => {
    setExpandedFrames(new Set<number>());
    setAllExpanded(false);
  };

  return (
    <div class="rounded-lg border border-[var(--color-border)] overflow-hidden">
      <div class="flex items-center justify-between border-b border-[var(--color-border)] bg-[var(--color-surface-1)] px-4 py-2">
        <h3 class="text-sm font-medium text-[var(--color-text-primary)]">
          Stack Trace
        </h3>
        <Button
          variant="ghost"
          size="sm"
          onClick={() => (allExpanded() ? collapseAll() : expandAll())}
        >
          {allExpanded() ? "Collapse All" : "Expand All"}
        </Button>
      </div>
      <div class="divide-y divide-[var(--color-border)]">
        <For each={reversedFrames()}>
          {(frame, index) => {
            const isExpanded = () => expandedFrames().has(index());
            const hasContext = () =>
              !!frame.context_line ||
              (frame.pre_context && frame.pre_context.length > 0) ||
              (frame.post_context && frame.post_context.length > 0);

            return (
              <div
                class={clsx(
                  frame.in_app
                    ? "bg-[var(--color-surface-1)]"
                    : "bg-[var(--color-surface-0)]",
                )}
              >
                <button
                  class="flex w-full items-center gap-2 px-4 py-2 text-left text-sm hover:bg-[var(--color-surface-2)] transition-colors"
                  onClick={() => toggleFrame(index())}
                >
                  <span class="text-[var(--color-text-secondary)] text-xs w-4">
                    {isExpanded() ? "\u25BC" : "\u25B6"}
                  </span>
                  <span class="font-mono text-xs text-[var(--color-text-primary)] font-medium">
                    {frame.function ?? "<anonymous>"}
                  </span>
                  <span class="text-xs text-[var(--color-text-secondary)] truncate">
                    {frame.filename ?? frame.abs_path ?? frame.module ?? "unknown"}
                    <Show when={frame.lineno}>
                      :{frame.lineno}
                      <Show when={frame.colno}>:{frame.colno}</Show>
                    </Show>
                  </span>
                  <Show when={frame.in_app}>
                    <span class="ml-auto text-[10px] font-medium text-indigo-600 dark:text-indigo-400 uppercase">
                      app
                    </span>
                  </Show>
                </button>
                <Show when={isExpanded() && hasContext()}>
                  <div class="mx-4 mb-2 overflow-x-auto rounded border border-[var(--color-border)] bg-[var(--color-surface-2)]">
                    <pre class="text-xs leading-5">
                      <For each={frame.pre_context ?? []}>
                        {(line, lineIdx) => {
                          const lineNum = () =>
                            (frame.lineno ?? 0) -
                            (frame.pre_context?.length ?? 0) +
                            lineIdx();
                          return (
                            <div class="flex">
                              <span class="inline-block w-12 flex-shrink-0 select-none pr-3 text-right text-[var(--color-text-secondary)] opacity-50">
                                {lineNum()}
                              </span>
                              <span class="text-[var(--color-text-primary)]">
                                {line}
                              </span>
                            </div>
                          );
                        }}
                      </For>
                      <Show when={frame.context_line}>
                        <div class="flex bg-yellow-100/50 dark:bg-yellow-900/20">
                          <span class="inline-block w-12 flex-shrink-0 select-none pr-3 text-right text-[var(--color-text-secondary)] font-medium">
                            {frame.lineno}
                          </span>
                          <span class="text-[var(--color-text-primary)] font-medium">
                            {frame.context_line}
                          </span>
                        </div>
                      </Show>
                      <For each={frame.post_context ?? []}>
                        {(line, lineIdx) => {
                          const lineNum = () => (frame.lineno ?? 0) + 1 + lineIdx();
                          return (
                            <div class="flex">
                              <span class="inline-block w-12 flex-shrink-0 select-none pr-3 text-right text-[var(--color-text-secondary)] opacity-50">
                                {lineNum()}
                              </span>
                              <span class="text-[var(--color-text-primary)]">
                                {line}
                              </span>
                            </div>
                          );
                        }}
                      </For>
                    </pre>
                  </div>
                </Show>
              </div>
            );
          }}
        </For>
      </div>
    </div>
  );
}
