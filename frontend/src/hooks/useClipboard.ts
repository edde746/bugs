import { createSignal } from "solid-js";

export function useClipboard() {
  const [copied, setCopied] = createSignal(false);
  const copy = async (text: string) => {
    await navigator.clipboard.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };
  return { copied, copy };
}
