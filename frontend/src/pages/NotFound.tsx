import { A } from "@solidjs/router";
import Button from "~/components/ui/Button";

export default function NotFound() {
  return (
    <div class="flex min-h-screen items-center justify-center p-6">
      <div class="text-center">
        <div class="mb-4 text-6xl font-bold text-gray-200 dark:text-gray-700">
          404
        </div>
        <h1 class="mb-2 text-xl font-semibold text-[var(--color-text-primary)]">
          Page Not Found
        </h1>
        <p class="mb-6 text-[var(--color-text-secondary)]">
          The page you are looking for does not exist.
        </p>
        <A href="/">
          <Button variant="secondary">Go Home</Button>
        </A>
      </div>
    </div>
  );
}
