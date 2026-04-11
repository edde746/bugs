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
      <div class="rounded-lg border border-[var(--color-border)] overflow-hidden">
        <div class="border-b border-[var(--color-border)] bg-[var(--color-surface-1)] px-4 py-2">
          <h3 class="text-sm font-medium text-[var(--color-text-primary)]">
            Tags
          </h3>
        </div>
        <table class="w-full text-sm">
          <thead>
            <tr class="border-b border-[var(--color-border)] bg-[var(--color-surface-1)]">
              <th class="px-4 py-1.5 text-left text-xs font-medium text-[var(--color-text-secondary)]">
                Key
              </th>
              <th class="px-4 py-1.5 text-left text-xs font-medium text-[var(--color-text-secondary)]">
                Value
              </th>
            </tr>
          </thead>
          <tbody>
            <For each={entries()}>
              {(tag, index) => (
                <tr
                  class={
                    index() % 2 === 0
                      ? "bg-[var(--color-surface-0)]"
                      : "bg-[var(--color-surface-1)]"
                  }
                >
                  <td class="px-4 py-1.5 text-xs font-medium text-[var(--color-text-secondary)]">
                    {tag.key}
                  </td>
                  <td class="px-4 py-1.5 text-xs font-mono text-[var(--color-text-primary)]">
                    {tag.value}
                  </td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>
    </Show>
  );
}
