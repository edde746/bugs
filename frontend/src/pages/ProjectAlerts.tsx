import { useParams } from "@solidjs/router";
import { createQuery, createMutation, useQueryClient } from "@tanstack/solid-query";
import { createSignal, For, Show } from "solid-js";
import { api } from "~/api/client";
import type { AlertRuleResponse } from "~/lib/sentry-types";
import { relativeTime } from "~/lib/formatters";
import Button from "~/components/ui/Button";
import Modal from "~/components/ui/Modal";
import LoadingSkeleton from "~/components/ui/LoadingSkeleton";
import EmptyState from "~/components/ui/EmptyState";

const CONDITION_TYPES = [
  { value: "NewIssue", label: "New Issue" },
  { value: "FrequencyThreshold", label: "Event Frequency" },
  { value: "RegressionEvent", label: "Issue Regression" },
] as const;

function conditionLabel(type: string): string {
  return CONDITION_TYPES.find((ct) => ct.value === type)?.label ?? type;
}

function firstWebhookUrl(rule: AlertRuleResponse): string {
  const webhook = rule.actions.find((a) => a.type === "Webhook");
  return webhook?.url ?? "";
}

export default function ProjectAlerts() {
  const params = useParams<{ project: string }>();
  const queryClient = useQueryClient();

  const [name, setName] = createSignal("");
  const [conditionType, setConditionType] = createSignal("NewIssue");
  const [webhookUrl, setWebhookUrl] = createSignal("");
  const [showModal, setShowModal] = createSignal(false);

  const alertsQuery = createQuery(() => ({
    queryKey: ["alerts", params.project],
    queryFn: () =>
      api.get<AlertRuleResponse[]>(`/internal/projects/${params.project}/alerts`),
  }));

  const createMut = createMutation(() => ({
    mutationFn: (input: {
      name: string;
      conditions: { type: string; threshold?: number; window_seconds?: number }[];
      actions: { type: string; url?: string }[];
    }) => api.post<AlertRuleResponse>(`/internal/projects/${params.project}/alerts`, input),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: ["alerts", params.project],
      });
      setName("");
      setConditionType("NewIssue");
      setWebhookUrl("");
      setShowModal(false);
    },
  }));

  const handleSubmit = (e: SubmitEvent) => {
    e.preventDefault();
    const ct = conditionType();
    const condition: { type: string; threshold?: number; window_seconds?: number } =
      ct === "FrequencyThreshold"
        ? { type: ct, threshold: 1, window_seconds: 3600 }
        : { type: ct };
    createMut.mutate({
      name: name(),
      conditions: [condition],
      actions: [{ type: "Webhook", url: webhookUrl() }],
    });
  };

  return (
    <div class="page">
      <div class="page__header">
        <h1 class="page__title">Alerts</h1>
        <Button
          variant="primary"
          size="sm"
          onClick={() => setShowModal(true)}
        >
          Create Alert Rule
        </Button>
      </div>

      <Modal
        open={showModal()}
        onClose={() => setShowModal(false)}
        title="New Alert Rule"
        description="Configure a condition and webhook to receive notifications."
        footer={
          <>
            <Button variant="secondary" size="sm" onClick={() => setShowModal(false)}>
              Cancel
            </Button>
            <Button
              type="submit"
              variant="primary"
              size="sm"
              disabled={createMut.isPending || !name().trim() || !webhookUrl().trim()}
              onClick={() => {
                const ct = conditionType();
                const condition: { type: string; threshold?: number; window_seconds?: number } =
                  ct === "FrequencyThreshold"
                    ? { type: ct, threshold: 1, window_seconds: 3600 }
                    : { type: ct };
                createMut.mutate({
                  name: name(),
                  conditions: [condition],
                  actions: [{ type: "Webhook", url: webhookUrl() }],
                });
              }}
            >
              {createMut.isPending ? "Creating..." : "Create Rule"}
            </Button>
          </>
        }
      >
        <div class="form-stack">
          <div class="form-field">
            <label class="field-label">Name</label>
            <input
              type="text"
              value={name()}
              onInput={(e) => setName(e.currentTarget.value)}
              class="input"
              placeholder="Alert rule name"
            />
          </div>
          <div class="form-field">
            <label class="field-label">Condition Type</label>
            <select
              value={conditionType()}
              onChange={(e) => setConditionType(e.currentTarget.value)}
              class="input"
            >
              <For each={CONDITION_TYPES}>
                {(ct) => <option value={ct.value}>{ct.label}</option>}
              </For>
            </select>
          </div>
          <div class="form-field">
            <label class="field-label">Webhook URL</label>
            <input
              type="url"
              value={webhookUrl()}
              onInput={(e) => setWebhookUrl(e.currentTarget.value)}
              class="input"
              placeholder="https://hooks.example.com/webhook"
            />
          </div>
        </div>
      </Modal>

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
          <div class="card">
            <table class="data-table">
              <thead>
                <tr>
                  <th>Name</th>
                  <th>Condition</th>
                  <th>Webhook URL</th>
                  <th data-align="right">Created</th>
                </tr>
              </thead>
              <tbody>
                <For each={alertsQuery.data}>
                  {(rule) => (
                    <tr>
                      <td style={{ "font-weight": "500" }}>{rule.name}</td>
                      <td class="text-secondary">
                        {rule.conditions.length > 0 ? conditionLabel(rule.conditions[0].type) : "\u2014"}
                      </td>
                      <td class="text-secondary text-mono" style={{ "font-size": "12px" }}>
                        {firstWebhookUrl(rule)}
                      </td>
                      <td data-align="right" class="text-secondary">
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
