import { createSignal, For, Show } from "solid-js";

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
      <div class="card">
        <div class="tabs" style={{ background: "var(--color-surface-1)", "overflow-x": "auto", "border-bottom": "1px solid var(--color-border)" }}>
          <For each={tabs()}>
            {(tab, index) => (
              <button
                class="tab"
                data-active={activeTab() === index()}
                onClick={() => setActiveTab(index())}
              >
                {tab.label}
              </button>
            )}
          </For>
        </div>
        <Show when={tabs()[activeTab()]}>
          <div class="card__body">
            <table class="data-table data-table--compact">
              <tbody>
                <For each={Object.entries(tabs()[activeTab()]!.data)}>
                  {([key, value]) => (
                    <tr>
                      <td class="text-secondary" style={{ "white-space": "nowrap", "vertical-align": "top", "padding-right": "16px", "font-family": "var(--font-sans)", "font-weight": "500" }}>
                        {key}
                      </td>
                      <td style={{ "word-break": "break-all" }}>
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
