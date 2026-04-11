import { clsx } from "clsx";

interface LoadingSkeletonProps {
  rows?: number;
  class?: string;
}

export default function LoadingSkeleton(props: LoadingSkeletonProps) {
  const rows = () => props.rows ?? 5;

  return (
    <div class={clsx("animate-pulse space-y-3", props.class)}>
      {Array.from({ length: rows() }).map((_, i) => (
        <div class="flex space-x-4">
          <div
            class="h-4 rounded bg-gray-200 dark:bg-gray-700"
            style={{ width: `${60 + ((i * 17) % 30)}%` }}
          />
        </div>
      ))}
    </div>
  );
}
