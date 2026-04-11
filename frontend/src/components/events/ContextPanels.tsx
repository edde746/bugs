import { createSignal, For, Show } from "solid-js";
import { clsx } from "clsx";

interface ContextPanelsProps {
  contexts?: Record<string, Record<string, unknown>>;
  request?: {
    method?: string;
    url?: string;
    headers?: Record<string, string>;
    query_string?: string;
    data?: unknown;
    env?: Record<string, string>;
  };
  user?: Record<string, unknown>;
}

interface TabDef {
  key: string;
  label: string;
  data: Record<string, unknown>;
}

export default function ContextPanels(props: ContextPanelsProps) {
  const tabs = (): TabDef[] => {
    const result: TabDef[] = [];

    if (props.user && Object.keys(props.user).length > 0) {
      result.push({ key: "user", label: "User", data: props.user });
    }

    if (props.contexts) {
      const order = ["browser", "os", "device", "runtime"];
      for (const key of order) {
        if (props.contexts[key] && Object.keys(props.contexts[key]).length > 0) {
          result.push({
            key,
            label: key.charAt(0).toUpperCase() + key.slice(1),
            data: props.contexts[key],
          });
        }
      }
      // Any other contexts
      for (const [key, value] of Object.entries(props.contexts)) {
        if (!order.includes(key) && value && Object.keys(value).length > 0) {
          result.push({
            key,
            label: key.charAt(0).toUpperCase() + key.slice(1),
            data: value,
          });
        }
      }
    }

    if (props.request) {
      const reqData: Record<string, unknown> = {};
      if (props.request.method) reqData["method"] = props.request.method;
      if (props.request.url) reqData["url"] = props.request.url;
      if (props.request.query_string) reqData["query_string"] = props.request.query_string;
      if (props.request.headers) {
        for (const [hk, hv] of Object.entries(props.request.headers)) {
          reqData[`header: ${hk}`] = hv;
        }
      }
      if (props.request.env) {
        for (const [ek, ev] of Object.entries(props.request.env)) {
          reqData[`env: ${ek}`] = ev;
        }
      }
      if (Object.keys(reqData).length > 0) {
        result.push({ key: "request", label: "Request", data: reqData });
      }
    }

    return result;
  };

  const [activeTab, setActiveTab] = createSignal(0);

  return (
    <Show when={tabs().length > 0}>
      <div class="rounded-lg border border-[var(--color-border)] overflow-hidden">
        <div class="flex border-b border-[var(--color-border)] bg-[var(--color-surface-1)] overflow-x-auto">
          <For each={tabs()}>
            {(tab, index) => (
              <button
                class={clsx(
                  "px-4 py-2 text-xs font-medium whitespace-nowrap transition-colors",
                  activeTab() === index()
                    ? "border-b-2 border-indigo-500 text-indigo-600 dark:text-indigo-400"
                    : "text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)]",
                )}
                onClick={() => setActiveTab(index())}
              >
                {tab.label}
              </button>
            )}
          </For>
        </div>
        <Show when={tabs()[activeTab()]}>
          <div class="p-4">
            <table class="w-full text-sm">
              <tbody>
                <For each={Object.entries(tabs()[activeTab()]!.data)}>
                  {([key, value]) => (
                    <tr class="border-b border-[var(--color-border)] last:border-0">
                      <td class="py-1.5 pr-4 text-xs font-medium text-[var(--color-text-secondary)] whitespace-nowrap align-top">
                        {key}
                      </td>
                      <td class="py-1.5 text-xs font-mono text-[var(--color-text-primary)] break-all">
                        {typeof value === "object"
                          ? JSON.stringify(value)
                          : String(value ?? "")}
                      </td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </div>
        </Show>
      </div>
    </Show>
  );
}
