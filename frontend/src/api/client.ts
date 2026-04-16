const BASE_URL = "/api";

export class ApiError extends Error {
  constructor(
    public status: number,
    public statusText: string,
    public body: unknown,
  ) {
    super(`API Error ${status}: ${statusText}`);
  }
}

// Ensures a 401 burst from concurrent in-flight requests only triggers a
// single redirect to /login. Without this guard, N parallel queries would
// each clear the token and navigate, causing jittery double-redirects and
// spurious error toasts.
let loggingOut = false;

function triggerLogout() {
  if (loggingOut) return;
  loggingOut = true;
  localStorage.removeItem("bugs_admin_token");
  // Race-free guard: if we're already on /login, do nothing.
  if (window.location.pathname !== "/login") {
    window.location.href = "/login";
  }
}

export interface ApiRequestOptions extends RequestInit {
  /**
   * Abort signal from the caller (typically tanstack-query's queryFn/mutationFn).
   * When the signal fires before the response lands, the underlying fetch is
   * cancelled and this request rejects with a DOMException('AbortError').
   */
  signal?: AbortSignal;
}

export async function apiRequest<T>(
  path: string,
  options: ApiRequestOptions = {},
): Promise<T> {
  const token = localStorage.getItem("bugs_admin_token");
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
    ...(options.headers as Record<string, string>),
  };
  if (token) headers["Authorization"] = `Bearer ${token}`;

  const response = await fetch(`${BASE_URL}${path}`, {
    ...options,
    headers,
    signal: options.signal,
  });
  if (!response.ok) {
    if (response.status === 401) {
      triggerLogout();
      throw new ApiError(response.status, response.statusText, null);
    }
    const body = await response.json().catch(() => null);
    throw new ApiError(response.status, response.statusText, body);
  }
  if (response.status === 204) return undefined as T;
  return response.json();
}

export const api = {
  get: <T>(path: string, signal?: AbortSignal) =>
    apiRequest<T>(path, { signal }),
  post: <T>(path: string, body: unknown, signal?: AbortSignal) =>
    apiRequest<T>(path, { method: "POST", body: JSON.stringify(body), signal }),
  put: <T>(path: string, body: unknown, signal?: AbortSignal) =>
    apiRequest<T>(path, { method: "PUT", body: JSON.stringify(body), signal }),
  delete: <T>(path: string, signal?: AbortSignal) =>
    apiRequest<T>(path, { method: "DELETE", signal }),
};
