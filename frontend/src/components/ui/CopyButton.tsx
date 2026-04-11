import { Show } from "solid-js";
import { useClipboard } from "~/hooks/useClipboard";
import Button from "~/components/ui/Button";

interface CopyButtonProps {
  text: string;
  label?: string;
  class?: string;
}

export default function CopyButton(props: CopyButtonProps) {
  const { copied, copy } = useClipboard();

  return (
    <Button
      variant="ghost"
      size="sm"
      class={props.class}
      onClick={() => copy(props.text)}
    >
      <Show when={copied()} fallback={<>{props.label ?? "Copy"}</>}>
        Copied!
      </Show>
    </Button>
  );
}
