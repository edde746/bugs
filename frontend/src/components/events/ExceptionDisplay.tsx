import { For, Show } from "solid-js";
import StacktraceViewer from "./StacktraceViewer";
import type { StackFrame } from "./StacktraceViewer";

export interface ExceptionValue {
  type?: string;
  value?: string;
  module?: string;
  mechanism?: {
    type?: string;
    handled?: boolean;
    description?: string;
  };
  stacktrace?: {
    frames?: StackFrame[];
  };
}

interface ExceptionDisplayProps {
  exceptions: ExceptionValue[];
}

export default function ExceptionDisplay(props: ExceptionDisplayProps) {
  return (
    <div class="space-y-4">
      <For each={props.exceptions}>
        {(exception) => (
          <div>
            <div class="mb-2">
              <span class="text-lg font-bold text-[var(--color-text-primary)]">
                {exception.type ?? "Error"}
              </span>
              <Show when={exception.value}>
                <p class="mt-0.5 text-sm text-[var(--color-text-secondary)]">
                  {exception.value}
                </p>
              </Show>
              <Show when={exception.mechanism}>
                <p class="mt-0.5 text-xs text-[var(--color-text-secondary)]">
                  Mechanism: {exception.mechanism!.type ?? "generic"}
                  {exception.mechanism!.handled === false && (
                    <span class="ml-2 text-red-600 dark:text-red-400 font-medium">
                      (unhandled)
                    </span>
                  )}
                </p>
              </Show>
            </div>
            <Show when={exception.stacktrace?.frames && exception.stacktrace!.frames!.length > 0}>
              <StacktraceViewer frames={exception.stacktrace!.frames!} />
            </Show>
          </div>
        )}
      </For>
    </div>
  );
}
