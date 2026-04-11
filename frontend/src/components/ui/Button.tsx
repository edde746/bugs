import { clsx } from "clsx";
import type { JSX } from "solid-js";

interface ButtonProps extends JSX.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "primary" | "secondary" | "danger" | "ghost";
  size?: "sm" | "md";
}

export default function Button(props: ButtonProps) {
  const variant = () => props.variant ?? "primary";
  const size = () => props.size ?? "md";

  return (
    <button
      {...props}
      class={clsx(
        "inline-flex items-center justify-center rounded-md font-medium transition-colors focus:outline-none focus:ring-2 focus:ring-offset-2 disabled:pointer-events-none disabled:opacity-50",
        {
          "bg-indigo-600 text-white hover:bg-indigo-700 focus:ring-indigo-500":
            variant() === "primary",
          "border border-gray-300 bg-white text-gray-700 hover:bg-gray-50 focus:ring-indigo-500 dark:border-gray-600 dark:bg-gray-800 dark:text-gray-200 dark:hover:bg-gray-700":
            variant() === "secondary",
          "bg-red-600 text-white hover:bg-red-700 focus:ring-red-500":
            variant() === "danger",
          "text-gray-600 hover:bg-gray-100 hover:text-gray-900 dark:text-gray-400 dark:hover:bg-gray-800 dark:hover:text-gray-200":
            variant() === "ghost",
        },
        {
          "px-2.5 py-1.5 text-xs": size() === "sm",
          "px-4 py-2 text-sm": size() === "md",
        },
        props.class,
      )}
    >
      {props.children}
    </button>
  );
}
