import { A } from "@solidjs/router";
import Button from "~/components/ui/Button";

export default function Onboarding() {
  return (
    <div class="flex min-h-screen items-center justify-center p-6">
      <div class="max-w-lg text-center">
        <h1 class="mb-4 text-3xl font-bold text-[var(--color-text-primary)]">
          Welcome to Bugs
        </h1>
        <p class="mb-6 text-[var(--color-text-secondary)]">
          A lightweight, self-hosted error tracker. Create your first project to
          start capturing errors from your applications.
        </p>

        <A href="/settings/projects">
          <Button>Create Your First Project</Button>
        </A>

        <div class="mt-10 rounded-lg border border-[var(--color-border)] bg-[var(--color-surface-1)] p-4 text-left">
          <h3 class="mb-2 text-sm font-medium text-[var(--color-text-primary)]">
            Quick Start
          </h3>
          <ol class="space-y-2 text-sm text-[var(--color-text-secondary)]">
            <li>1. Create a project in Settings</li>
            <li>2. Copy the DSN from the project keys</li>
            <li>3. Configure your Sentry SDK with the DSN</li>
          </ol>
          <div class="mt-3 rounded bg-[var(--color-surface-2)] p-3">
            <code class="text-xs text-[var(--color-text-primary)]">
              {"Sentry.init({ dsn: 'http://<key>@localhost:9000/<project_id>' })"}
            </code>
          </div>
        </div>
      </div>
    </div>
  );
}
