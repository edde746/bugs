import type { JSX } from "solid-js";

interface ButtonProps extends JSX.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "primary" | "secondary" | "danger" | "ghost";
  size?: "sm" | "md";
}

export default function Button(props: ButtonProps) {
  return (
    <button
      {...props}
      class={`btn ${props.class ?? ""}`}
      data-variant={props.variant ?? "primary"}
      data-size={props.size ?? "sm"}
    >
      {props.children}
    </button>
  );
}
