interface BadgeProps {
  level: string;
  class?: string;
}

export default function Badge(props: BadgeProps) {
  return (
    <span class={`badge ${props.class ?? ""}`} data-level={props.level}>
      {props.level}
    </span>
  );
}
