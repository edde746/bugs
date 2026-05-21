import { createSignal, For, Show } from "solid-js";
import { displayValue } from "~/lib/formatters";

interface ContextPanelsProps {
  contexts?: Record<string, unknown>;
  request?: Record<string, unknown> | null;
  user?: Record<string, unknown> | null;
}

interface TabDef {
  key: string;
  label: string;
  data: Record<string, unknown>;
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function asString(value: unknown): string | null {
  return typeof value === "string" ? value : null;
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
        const value = asRecord(props.contexts[key]);
        if (value && Object.keys(value).length > 0) {
          result.push({
            key,
            label: key.charAt(0).toUpperCase() + key.slice(1),
            data: value,
          });
        }
      }
      for (const [key, value] of Object.entries(props.contexts)) {
        const data = asRecord(value);
        if (!order.includes(key) && data && Object.keys(data).length > 0) {
          result.push({
            key,
            label: key.charAt(0).toUpperCase() + key.slice(1),
            data,
          });
        }
      }
    }

    if (props.request) {
      const reqData: Record<string, unknown> = {};
      const method = asString(props.request.method);
      const url = asString(props.request.url);
      const queryString = asString(props.request.query_string);
      const headers = asRecord(props.request.headers);
      const env = asRecord(props.request.env);
      if (method) reqData["method"] = method;
      if (url) reqData["url"] = url;
      if (queryString) reqData["query_string"] = queryString;
      if (headers) {
        for (const [hk, hv] of Object.entries(headers)) {
          reqData[`header: ${hk}`] = hv;
        }
      }
      if (env) {
        for (const [ek, ev] of Object.entries(env)) {
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
        <div class="tabs" style={{ background: "var(--color-surface-1)", "overflow-x": "auto" }}>
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
                        {displayValue(value)}
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
