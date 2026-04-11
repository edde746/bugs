import { clsx } from "clsx";
import { LEVEL_COLORS } from "~/lib/constants";

interface BadgeProps {
  level: string;
  class?: string;
}

export default function Badge(props: BadgeProps) {
  const colors = () => LEVEL_COLORS[props.level] ?? LEVEL_COLORS["info"]!;

  return (
    <span
      class={clsx(
        "inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium",
        colors().bg,
        colors().text,
        props.class,
      )}
    >
      {props.level}
    </span>
  );
}
