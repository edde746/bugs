import type { JSX } from "solid-js";
import IconInbox from "~icons/lucide/inbox";

interface EmptyStateProps {
  title: string;
  description?: string;
  children?: JSX.Element;
}

export default function EmptyState(props: EmptyStateProps) {
  return (
    <div class="empty-state">
      <div class="empty-state__icon">
        <IconInbox />
      </div>
      <h3 class="empty-state__title">{props.title}</h3>
      {props.description && (
        <p class="empty-state__description">{props.description}</p>
      )}
      {props.children && <div class="empty-state__actions">{props.children}</div>}
    </div>
  );
}
