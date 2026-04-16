import { createSignal } from "solid-js";

/**
 * Clipboard hook that distinguishes a successful copy from a failure.
 *
 * The browser clipboard API can reject for several reasons (insecure
 * context, missing user gesture, iframe permission, etc.). The previous
 * version swallowed those rejections, so the UI flashed "Copied!" even
 * when nothing landed on the clipboard. We now surface `error` and keep
 * `copied` false on failure so callers can show an accurate state.
 */
export function useClipboard() {
  const [copied, setCopied] = createSignal(false);
  const [error, setError] = createSignal<Error | null>(null);

  const copy = async (text: string): Promise<boolean> => {
    try {
      if (!navigator.clipboard?.writeText) {
        throw new Error("Clipboard API unavailable in this context");
      }
      await navigator.clipboard.writeText(text);
      setError(null);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
      return true;
    } catch (e) {
      const err = e instanceof Error ? e : new Error(String(e));
      setError(err);
      setCopied(false);
      return false;
    }
  };

  return { copied, error, copy };
}
