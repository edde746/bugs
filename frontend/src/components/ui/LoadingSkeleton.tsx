interface LoadingSkeletonProps {
  rows?: number;
  class?: string;
}

export default function LoadingSkeleton(props: LoadingSkeletonProps) {
  const rows = () => props.rows ?? 5;

  return (
    <div class={`skeleton ${props.class ?? ""}`}>
      {Array.from({ length: rows() }).map((_, i) => (
        <div
          class="skeleton__line"
          style={{ width: `${60 + ((i * 17) % 30)}%` }}
        />
      ))}
    </div>
  );
}
