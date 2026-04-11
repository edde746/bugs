import { createSignal, createEffect } from "solid-js";

export function useDebounce<T>(value: () => T, delay: number): () => T {
  const [debounced, setDebounced] = createSignal(value());
  let timer: ReturnType<typeof setTimeout>;
  createEffect(() => {
    const v = value();
    clearTimeout(timer);
    timer = setTimeout(() => setDebounced(() => v), delay);
  });
  return debounced;
}
