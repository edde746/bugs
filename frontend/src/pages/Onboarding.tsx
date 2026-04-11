import { A } from "@solidjs/router";
import Button from "~/components/ui/Button";

export default function Onboarding() {
  return (
    <div class="center-page">
      <div class="center-page__content">
        <h1 class="center-page__title">Welcome to Bugs</h1>
        <p class="center-page__text">
          A lightweight, self-hosted error tracker. Create your first project to
          start capturing errors from your applications.
        </p>

        <A href="/settings/projects">
          <Button size="md">Create Your First Project</Button>
        </A>

        <div class="quickstart-card">
          <h3>Quick Start</h3>
          <ol>
            <li>1. Create a project in Settings</li>
            <li>2. Copy the DSN from the project keys</li>
            <li>3. Configure your Sentry SDK with the DSN</li>
          </ol>
          <div class="quickstart-card__code">
            <code>
              {"Sentry.init({ dsn: 'http://<key>@localhost:9000/<project_id>' })"}
            </code>
          </div>
        </div>
      </div>
    </div>
  );
}
