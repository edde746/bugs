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
    <div>
      <For each={props.exceptions}>
        {(exception) => (
          <div class="exception">
            <div>
              <span class="exception__type">
                {exception.type ?? "Error"}
              </span>
              <Show when={exception.value}>
                <p class="exception__value">{exception.value}</p>
              </Show>
              <Show when={exception.mechanism}>
                <p class="exception__mechanism">
                  Mechanism: {exception.mechanism!.type ?? "generic"}
                  {exception.mechanism!.handled === false && (
                    <span class="exception__unhandled">(unhandled)</span>
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
