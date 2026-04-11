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
    <div class="tabs">
      <For each={props.items}>
        {(item) => (
          <button
            class="tab"
            data-active={props.activeValue === item.value}
            onClick={() => props.onChange(item.value)}
          >
            {item.label}
          </button>
        )}
      </For>
    </div>
  );
}
