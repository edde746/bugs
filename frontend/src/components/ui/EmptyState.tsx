import type { JSX } from "solid-js";

interface EmptyStateProps {
  title: string;
  description?: string;
  children?: JSX.Element;
}

export default function EmptyState(props: EmptyStateProps) {
  return (
    <div class="flex flex-col items-center justify-center py-16 text-center">
      <div class="mb-3 text-4xl text-gray-300 dark:text-gray-600">
        &#128722;
      </div>
      <h3 class="text-lg font-medium text-gray-900 dark:text-gray-100">
        {props.title}
      </h3>
      {props.description && (
        <p class="mt-1 text-sm text-[var(--color-text-secondary)]">
          {props.description}
        </p>
      )}
      {props.children && <div class="mt-4">{props.children}</div>}
    </div>
  );
}
