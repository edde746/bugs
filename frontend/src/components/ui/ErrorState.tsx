import type { JSX } from "solid-js";
import { Show } from "solid-js";
import IconAlertTriangle from "~icons/lucide/alert-triangle";

interface ErrorStateProps {
  title?: string;
  error?: unknown;
  description?: string;
  children?: JSX.Element;
}

// Compact inline error surface, shared across pages/tables. Keeps the UI
// honest about fetch failures instead of rendering nothing when a query
// rejects, which otherwise looks identical to an empty result.
export default function ErrorState(props: ErrorStateProps) {
  const message = () => {
    if (props.description) return props.description;
    const err = props.error;
    if (err instanceof Error) return err.message;
    if (typeof err === "string") return err;
    return "Something went wrong while loading this content.";
  };

  return (
    <div class="empty-state" role="alert">
      <div class="empty-state__icon">
        <IconAlertTriangle />
      </div>
      <h3 class="empty-state__title">{props.title ?? "Couldn't load"}</h3>
      <p class="empty-state__description">{message()}</p>
      <Show when={props.children}>
        <div class="empty-state__actions">{props.children}</div>
      </Show>
    </div>
  );
}
