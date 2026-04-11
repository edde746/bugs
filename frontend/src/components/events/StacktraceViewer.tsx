import { createSignal, For, Show } from "solid-js";
import IconChevronDown from "~icons/lucide/chevron-down";
import IconChevronRight from "~icons/lucide/chevron-right";
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
    <div class="stacktrace">
      <div class="stacktrace__header">
        <h3>Stack Trace</h3>
        <Button
          variant="ghost"
          size="sm"
          onClick={() => (allExpanded() ? collapseAll() : expandAll())}
        >
          {allExpanded() ? "Collapse All" : "Expand All"}
        </Button>
      </div>
      <div class="stacktrace__frames">
        <For each={reversedFrames()}>
          {(frame, index) => {
            const isExpanded = () => expandedFrames().has(index());
            const hasContext = () =>
              !!frame.context_line ||
              (frame.pre_context && frame.pre_context.length > 0) ||
              (frame.post_context && frame.post_context.length > 0);

            return (
              <div class="stacktrace__frame" data-in-app={frame.in_app ?? false}>
                <button
                  class="stacktrace__frame-btn"
                  onClick={() => toggleFrame(index())}
                >
                  <span class="stacktrace__toggle-icon">
                    {isExpanded() ? <IconChevronDown /> : <IconChevronRight />}
                  </span>
                  <span class="stacktrace__fn-name">
                    {frame.function ?? "<anonymous>"}
                  </span>
                  <span class="stacktrace__file-name">
                    {frame.filename ?? frame.abs_path ?? frame.module ?? "unknown"}
                    <Show when={frame.lineno}>
                      :{frame.lineno}
                      <Show when={frame.colno}>:{frame.colno}</Show>
                    </Show>
                  </span>
                  <Show when={frame.in_app}>
                    <span class="stacktrace__app-tag">app</span>
                  </Show>
                </button>
                <Show when={isExpanded() && hasContext()}>
                  <div class="stacktrace__context">
                    <pre>
                      <For each={frame.pre_context ?? []}>
                        {(line, lineIdx) => {
                          const lineNum = () =>
                            (frame.lineno ?? 0) -
                            (frame.pre_context?.length ?? 0) +
                            lineIdx();
                          return (
                            <div class="stacktrace__context-line">
                              <span class="stacktrace__line-number">
                                {lineNum()}
                              </span>
                              <span class="stacktrace__line-content">
                                {line}
                              </span>
                            </div>
                          );
                        }}
                      </For>
                      <Show when={frame.context_line}>
                        <div class="stacktrace__context-line" data-highlight="true">
                          <span class="stacktrace__line-number">
                            {frame.lineno}
                          </span>
                          <span class="stacktrace__line-content">
                            {frame.context_line}
                          </span>
                        </div>
                      </Show>
                      <For each={frame.post_context ?? []}>
                        {(line, lineIdx) => {
                          const lineNum = () => (frame.lineno ?? 0) + 1 + lineIdx();
                          return (
                            <div class="stacktrace__context-line">
                              <span class="stacktrace__line-number">
                                {lineNum()}
                              </span>
                              <span class="stacktrace__line-content">
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
