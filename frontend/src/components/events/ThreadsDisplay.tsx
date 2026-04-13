import { For, Show } from "solid-js";
import StacktraceViewer from "./StacktraceViewer";
import type { StackFrame } from "./StacktraceViewer";

export interface ThreadValue {
  id?: unknown;
  name?: string;
  crashed?: boolean;
  current?: boolean;
  stacktrace?: {
    frames?: StackFrame[];
  };
}

interface ThreadsDisplayProps {
  threads: ThreadValue[];
}

export default function ThreadsDisplay(props: ThreadsDisplayProps) {
  return (
    <div>
      <For each={props.threads}>
        {(thread) => (
          <div class="thread" data-crashed={thread.crashed ?? false}>
            <div class="thread__header">
              <span class="thread__name">
                Thread {thread.id != null ? `#${thread.id}` : ""}{thread.name ? ` — ${thread.name}` : ""}
              </span>
              <Show when={thread.crashed}>
                <span class="thread__crashed-tag">crashed</span>
              </Show>
              <Show when={thread.current}>
                <span class="thread__current-tag">current</span>
              </Show>
            </div>
            <Show when={thread.stacktrace?.frames && thread.stacktrace!.frames!.length > 0}>
              <StacktraceViewer frames={thread.stacktrace!.frames!} />
            </Show>
          </div>
        )}
      </For>
    </div>
  );
}
