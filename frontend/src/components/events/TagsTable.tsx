import { For, Show } from "solid-js";

interface TagsTableProps {
  tags: Array<{ key: string; value: string }> | Record<string, string>;
}

export default function TagsTable(props: TagsTableProps) {
  const entries = (): Array<{ key: string; value: string }> => {
    if (Array.isArray(props.tags)) {
      return props.tags;
    }
    return Object.entries(props.tags).map(([key, value]) => ({ key, value }));
  };

  return (
    <Show when={entries().length > 0}>
      <div class="card">
        <div class="card__header">
          <h3>Tags</h3>
        </div>
        <table class="data-table data-table--compact data-table--striped">
          <thead>
            <tr>
              <th>Key</th>
              <th>Value</th>
            </tr>
          </thead>
          <tbody>
            <For each={entries()}>
              {(tag) => (
                <tr>
                  <td class="text-secondary" style={{ "font-family": "var(--font-sans)", "font-weight": "500" }}>{tag.key}</td>
                  <td>{tag.value}</td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>
    </Show>
  );
}
