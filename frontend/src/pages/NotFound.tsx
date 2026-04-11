import { A } from "@solidjs/router";
import Button from "~/components/ui/Button";

export default function NotFound() {
  return (
    <div class="center-page">
      <div class="center-page__content">
        <div class="center-page__404">404</div>
        <h1 class="center-page__title" style={{ "font-size": "20px" }}>
          Page Not Found
        </h1>
        <p class="center-page__text">
          The page you are looking for does not exist.
        </p>
        <A href="/">
          <Button variant="secondary">Go Home</Button>
        </A>
      </div>
    </div>
  );
}
