import { createMemo, createSignal, For, Show } from "solid-js";
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
  instruction_addr?: string;
  symbol_addr?: string;
  image_addr?: string;
  package?: string;
  native?: boolean;
  platform?: string;
  lock?: {
    address: string;
    class_name: string;
    package_name: string;
    type: number;
  };
  addr_mode?: string;
  trust?: string;
}

export interface DebugImage {
  type?: string;
  debug_id?: string;
  code_id?: string;
  code_file?: string;
  debug_file?: string;
  image_addr?: string | number;
  image_size?: string | number;
  image_vmaddr?: string | number;
  arch?: string;
}

function parseAddr(v: string | number | undefined): number | null {
  if (v == null) return null;
  if (typeof v === "number") return v;
  const s = v.trim().replace(/^0[xX]/, "");
  const n = Number.parseInt(s, 16);
  return Number.isFinite(n) ? n : null;
}

function findImage(
  frame: StackFrame,
  images: DebugImage[] | undefined,
): DebugImage | null {
  if (!images || images.length === 0) return null;
  const iaddr = parseAddr(frame.instruction_addr);
  if (iaddr == null) return null;
  for (const img of images) {
    const base = parseAddr(img.image_addr);
    const size = parseAddr(img.image_size) ?? 0;
    if (base == null) continue;
    if (iaddr >= base && (size === 0 || iaddr < base + size)) return img;
  }
  return null;
}

export function getFrameName(frame: StackFrame): string {
  return frame.function ?? frame.instruction_addr ?? "<anonymous>";
}

export function getFrameLocation(frame: StackFrame): string {
  if (frame.filename || frame.abs_path || frame.module) {
    let loc = frame.filename ?? frame.abs_path ?? frame.module ?? "";
    if (frame.lineno != null) {
      loc += `:${frame.lineno}`;
      if (frame.colno != null) loc += `:${frame.colno}`;
    }
    return loc;
  }
  return frame.package ?? "unknown";
}

interface StacktraceViewerProps {
  frames: StackFrame[];
  images?: DebugImage[];
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
            const image = createMemo(() => findImage(frame, props.images));
            const hasDebugInfo = () => {
              const img = image();
              return !!(
                frame.instruction_addr ||
                frame.image_addr ||
                img?.debug_id ||
                img?.code_id ||
                img?.code_file ||
                img?.arch
              );
            };
            const isExpandable = () => hasContext() || hasDebugInfo();
            const frameName = () => getFrameName(frame);
            const frameLocation = () => getFrameLocation(frame);

            const frameTags = () => (
              <>
                <Show when={frame.in_app}>
                  <span class="stacktrace__app-tag">app</span>
                </Show>
                <Show when={frame.native}>
                  <span class="stacktrace__native-tag">native</span>
                </Show>
                <Show when={frame.lock}>
                  <span class="stacktrace__lock-tag" title={`${frame.lock!.package_name}.${frame.lock!.class_name} @ ${frame.lock!.address}`}>
                    lock
                  </span>
                </Show>
              </>
            );

            return (
              <div class="stacktrace__frame" data-in-app={frame.in_app ?? false}>
                <Show when={isExpandable()} fallback={
                  <div class="stacktrace__frame-btn stacktrace__frame-btn--static">
                    <span class="stacktrace__fn-name">
                      {frameName()}
                    </span>
                    <span class="stacktrace__file-name">
                      {frameLocation()}
                    </span>
                    {frameTags()}
                  </div>
                }>
                  <button
                    class="stacktrace__frame-btn"
                    onClick={() => toggleFrame(index())}
                  >
                    <span class="stacktrace__toggle-icon">
                      {isExpanded() ? <IconChevronDown /> : <IconChevronRight />}
                    </span>
                    <span class="stacktrace__fn-name">
                      {frameName()}
                    </span>
                    <span class="stacktrace__file-name">
                      {frameLocation()}
                    </span>
                    {frameTags()}
                  </button>
                </Show>
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
                <Show when={isExpanded() && hasDebugInfo()}>
                  <dl class="stacktrace__debug-info">
                    <Show when={frame.instruction_addr}>
                      <dt>instruction_addr</dt>
                      <dd><code>{frame.instruction_addr}</code></dd>
                    </Show>
                    <Show when={frame.image_addr ?? image()?.image_addr}>
                      <dt>image_addr</dt>
                      <dd><code>{String(frame.image_addr ?? image()?.image_addr)}</code></dd>
                    </Show>
                    <Show when={image()?.debug_id}>
                      <dt>debug_id</dt>
                      <dd><code>{image()!.debug_id}</code></dd>
                    </Show>
                    <Show when={image()?.code_id}>
                      <dt>code_id</dt>
                      <dd><code>{image()!.code_id}</code></dd>
                    </Show>
                    <Show when={image()?.arch}>
                      <dt>arch</dt>
                      <dd>{image()!.arch}</dd>
                    </Show>
                    <Show when={image()?.code_file}>
                      <dt>code_file</dt>
                      <dd><code>{image()!.code_file}</code></dd>
                    </Show>
                  </dl>
                </Show>
              </div>
            );
          }}
        </For>
      </div>
    </div>
  );
}
