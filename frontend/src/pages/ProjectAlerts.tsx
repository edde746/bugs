import { useParams } from "@solidjs/router";
import { createQuery, createMutation, useQueryClient } from "@tanstack/solid-query";
import { createSignal, For, Show } from "solid-js";
import { api } from "~/api/client";
import { relativeTime } from "~/lib/formatters";
import Button from "~/components/ui/Button";
import LoadingSkeleton from "~/components/ui/LoadingSkeleton";
import EmptyState from "~/components/ui/EmptyState";

interface AlertRule {
  id: number;
  name: string;
  condition_type: string;
  webhook_url: string;
  created_at: string;
}

const CONDITION_TYPES = [
  { value: "new_issue", label: "New Issue" },
  { value: "event_frequency", label: "Event Frequency" },
  { value: "issue_regression", label: "Issue Regression" },
] as const;

export default function ProjectAlerts() {
  const params = useParams<{ project: string }>();
  const queryClient = useQueryClient();

  const [name, setName] = createSignal("");
  const [conditionType, setConditionType] = createSignal("new_issue");
  const [webhookUrl, setWebhookUrl] = createSignal("");
  const [showForm, setShowForm] = createSignal(false);

  const alertsQuery = createQuery(() => ({
    queryKey: ["alerts", params.project],
    queryFn: () =>
      api.get<AlertRule[]>(`/internal/projects/${params.project}/alerts`),
  }));

  const createMut = createMutation(() => ({
    mutationFn: (input: {
      name: string;
      condition_type: string;
      webhook_url: string;
    }) => api.post<AlertRule>(`/internal/projects/${params.project}/alerts`, input),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: ["alerts", params.project],
      });
      setName("");
      setConditionType("new_issue");
      setWebhookUrl("");
      setShowForm(false);
    },
  }));

  const handleSubmit = (e: SubmitEvent) => {
    e.preventDefault();
    createMut.mutate({
      name: name(),
      condition_type: conditionType(),
      webhook_url: webhookUrl(),
    });
  };

  return (
    <div class="p-6">
      <div class="mb-6 flex items-center justify-between">
        <h1 class="text-2xl font-bold text-[var(--color-text-primary)]">
          Alerts
        </h1>
        <Button
          variant="primary"
          size="sm"
          onClick={() => setShowForm(!showForm())}
        >
          {showForm() ? "Cancel" : "Create Alert Rule"}
        </Button>
      </div>

      {/* Create form */}
      <Show when={showForm()}>
        <form
          onSubmit={handleSubmit}
          class="mb-6 rounded-lg border border-[var(--color-border)] bg-[var(--color-surface-1)] p-4"
        >
          <h2 class="mb-4 text-lg font-medium text-[var(--color-text-primary)]">
            New Alert Rule
          </h2>
          <div class="space-y-4">
            <div>
              <label class="mb-1 block text-sm font-medium text-[var(--color-text-primary)]">
                Name
              </label>
              <input
                type="text"
                value={name()}
                onInput={(e) => setName(e.currentTarget.value)}
                required
                class="w-full rounded-md border border-[var(--color-border)] bg-[var(--color-surface-0)] px-3 py-2 text-sm text-[var(--color-text-primary)] focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
                placeholder="Alert rule name"
              />
            </div>
            <div>
              <label class="mb-1 block text-sm font-medium text-[var(--color-text-primary)]">
                Condition Type
              </label>
              <select
                value={conditionType()}
                onChange={(e) => setConditionType(e.currentTarget.value)}
                class="w-full rounded-md border border-[var(--color-border)] bg-[var(--color-surface-0)] px-3 py-2 text-sm text-[var(--color-text-primary)] focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
              >
                <For each={CONDITION_TYPES}>
                  {(ct) => <option value={ct.value}>{ct.label}</option>}
                </For>
              </select>
            </div>
            <div>
              <label class="mb-1 block text-sm font-medium text-[var(--color-text-primary)]">
                Webhook URL
              </label>
              <input
                type="url"
                value={webhookUrl()}
                onInput={(e) => setWebhookUrl(e.currentTarget.value)}
                required
                class="w-full rounded-md border border-[var(--color-border)] bg-[var(--color-surface-0)] px-3 py-2 text-sm text-[var(--color-text-primary)] focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
                placeholder="https://hooks.example.com/webhook"
              />
            </div>
            <div class="flex justify-end">
              <Button
                type="submit"
                variant="primary"
                size="sm"
                disabled={createMut.isPending}
              >
                {createMut.isPending ? "Creating..." : "Create Rule"}
              </Button>
            </div>
          </div>
        </form>
      </Show>

      <Show when={!alertsQuery.isPending} fallback={<LoadingSkeleton rows={4} />}>
        <Show
          when={alertsQuery.data && alertsQuery.data.length > 0}
          fallback={
            <EmptyState
              title="No alert rules"
              description="Create an alert rule to get notified when events occur."
            />
          }
        >
          <div class="overflow-hidden rounded-lg border border-[var(--color-border)]">
            <table class="w-full">
              <thead>
                <tr class="border-b border-[var(--color-border)] bg-[var(--color-surface-1)]">
                  <th class="px-4 py-2 text-left text-xs font-medium text-[var(--color-text-secondary)]">
                    Name
                  </th>
                  <th class="px-4 py-2 text-left text-xs font-medium text-[var(--color-text-secondary)]">
                    Condition
                  </th>
                  <th class="px-4 py-2 text-left text-xs font-medium text-[var(--color-text-secondary)]">
                    Webhook URL
                  </th>
                  <th class="px-4 py-2 text-right text-xs font-medium text-[var(--color-text-secondary)]">
                    Created
                  </th>
                </tr>
              </thead>
              <tbody>
                <For each={alertsQuery.data}>
                  {(rule) => (
                    <tr class="border-b border-[var(--color-border)] transition-colors hover:bg-[var(--color-surface-1)]">
                      <td class="px-4 py-3 text-sm font-medium text-[var(--color-text-primary)]">
                        {rule.name}
                      </td>
                      <td class="px-4 py-3 text-sm text-[var(--color-text-secondary)]">
                        {CONDITION_TYPES.find((ct) => ct.value === rule.condition_type)?.label ?? rule.condition_type}
                      </td>
                      <td class="px-4 py-3 font-mono text-xs text-[var(--color-text-secondary)]">
                        {rule.webhook_url}
                      </td>
                      <td class="px-4 py-3 text-right text-sm text-[var(--color-text-secondary)]">
                        {relativeTime(rule.created_at)}
                      </td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </div>
        </Show>
      </Show>
    </div>
  );
}
