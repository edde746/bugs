import { relativeTime } from "~/lib/formatters";

interface RelativeTimeProps {
  date: string;
  class?: string;
}

export default function RelativeTime(props: RelativeTimeProps) {
  return (
    <time
      datetime={props.date}
      title={new Date(props.date).toLocaleString()}
      class={props.class}
    >
      {relativeTime(props.date)}
    </time>
  );
}
