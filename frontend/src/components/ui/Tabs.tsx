import { clsx } from "clsx";
import { For } from "solid-js";

interface TabItem {
  label: string;
  value: string;
}

interface TabsProps {
  items: TabItem[];
  activeValue: string;
  onChange: (value: string) => void;
}

export default function Tabs(props: TabsProps) {
  return (
    <div class="flex gap-1 border-b border-[var(--color-border)]">
      <For each={props.items}>
        {(item) => (
          <button
            class={clsx(
              "px-3 py-2 text-sm font-medium transition-colors",
              props.activeValue === item.value
                ? "border-b-2 border-indigo-500 text-indigo-600 dark:text-indigo-400"
                : "text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)]",
            )}
            onClick={() => props.onChange(item.value)}
          >
            {item.label}
          </button>
        )}
      </For>
    </div>
  );
}
