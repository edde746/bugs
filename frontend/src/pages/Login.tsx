import { createSignal } from "solid-js";
import { useNavigate } from "@solidjs/router";
import Button from "~/components/ui/Button";

export default function Login() {
  const navigate = useNavigate();
  const [token, setToken] = createSignal("");
  const [error, setError] = createSignal("");
  const [loading, setLoading] = createSignal(false);

  const handleSubmit = async (e: Event) => {
    e.preventDefault();
    setError("");
    setLoading(true);
    try {
      const res = await fetch("/api/internal/auth/check", {
        method: "POST",
        headers: { Authorization: `Bearer ${token()}` },
      });
      if (res.ok) {
        localStorage.setItem("bugs_admin_token", token());
        navigate("/", { replace: true });
      } else {
        setError("Invalid token");
      }
    } catch {
      setError("Connection error");
    } finally {
      setLoading(false);
    }
  };

  return (
    <div class="center-page">
      <div class="center-page__content">
        <h1 class="center-page__title">Bugs</h1>
        <p class="center-page__text">Enter your admin token to continue.</p>
        <form onSubmit={handleSubmit} class="form-stack">
          <input
            type="password"
            value={token()}
            onInput={(e) => setToken(e.currentTarget.value)}
            placeholder="Admin token"
            class="input"
            autocomplete="current-password"
          />
          {error() && (
            <p style={{ color: "var(--color-danger)", "font-size": "13px" }}>
              {error()}
            </p>
          )}
          <Button size="md" type="submit" disabled={loading() || !token()}>
            {loading() ? "Checking..." : "Sign In"}
          </Button>
        </form>
      </div>
    </div>
  );
}
