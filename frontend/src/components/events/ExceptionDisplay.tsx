import { For, Show } from "solid-js";
import StacktraceViewer from "./StacktraceViewer";
import type { StackFrame } from "./StacktraceViewer";

export interface ExceptionValue {
  type?: string;
  value?: string;
  module?: string;
  thread_id?: number;
  mechanism?: {
    type?: string;
    handled?: boolean;
    description?: string;
    data?: Record<string, unknown>;
    meta?: Record<string, unknown>;
  };
  stacktrace?: {
    frames?: StackFrame[];
    snapshot?: boolean;
  };
}

export function formatMechanismDetails(mechanism: NonNullable<ExceptionValue["mechanism"]>): string {
  const parts: string[] = [];
  if (mechanism.data) {
    for (const [k, v] of Object.entries(mechanism.data)) {
      if (v != null) parts.push(`${k}: ${v}`);
    }
  }
  if (mechanism.meta) {
    for (const [k, v] of Object.entries(mechanism.meta)) {
      if (v && typeof v === "object") {
        const inner = Object.entries(v as Record<string, unknown>)
          .filter(([, val]) => val != null)
          .map(([ik, iv]) => `${ik}: ${iv}`)
          .join(", ");
        if (inner) parts.push(`${k}(${inner})`);
      }
    }
  }
  return parts.join(", ");
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
                <Show when={exception.module}>
                  <span class="exception__module"> ({exception.module})</span>
                </Show>
              </span>
              <Show when={exception.thread_id != null}>
                <span class="exception__thread-id">Thread #{exception.thread_id}</span>
              </Show>
              <Show when={exception.value}>
                <p class="exception__value">{exception.value}</p>
              </Show>
              <Show when={exception.mechanism}>
                <p class="exception__mechanism">
                  Mechanism: {exception.mechanism!.type ?? "generic"}
                  {exception.mechanism!.handled === false && (
                    <span class="exception__unhandled">(unhandled)</span>
                  )}
                  {(() => {
                    const details = formatMechanismDetails(exception.mechanism!);
                    return details ? ` — ${details}` : "";
                  })()}
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
