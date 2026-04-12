import { useParams } from "@solidjs/router";
import { createQuery, createMutation, useQueryClient } from "@tanstack/solid-query";
import { createSignal, For, Show } from "solid-js";
import { api } from "~/api/client";
import type { AlertRuleResponse } from "~/lib/sentry-types";
import { relativeTime } from "~/lib/formatters";
import Button from "~/components/ui/Button";
import Modal from "~/components/ui/Modal";
import LoadingSpinner from "~/components/ui/LoadingSpinner";
import EmptyState from "~/components/ui/EmptyState";

const CONDITION_TYPES = [
  { value: "NewIssue", label: "New Issue" },
  { value: "FrequencyThreshold", label: "Event Frequency" },
  { value: "RegressionEvent", label: "Issue Regression" },
] as const;

const ACTION_TYPES = [
  { value: "Webhook", label: "Webhook", urlField: "url" },
  { value: "Slack", label: "Slack", urlField: "webhook_url" },
  { value: "Discord", label: "Discord", urlField: "webhook_url" },
  { value: "Email", label: "Email", urlField: "to" },
  { value: "LogFile", label: "Log File", urlField: "path" },
] as const;

function conditionLabel(type: string): string {
  return CONDITION_TYPES.find((ct) => ct.value === type)?.label ?? type;
}

function actionLabel(type: string): string {
  return ACTION_TYPES.find((at) => at.value === type)?.label ?? type;
}

function firstActionUrl(rule: AlertRuleResponse): string {
  const action = rule.actions[0];
  if (!action) return "";
  return action.url ?? action.webhook_url ?? action.to ?? action.path ?? "";
}

export default function ProjectAlerts() {
  const params = useParams<{ project: string }>();
  const queryClient = useQueryClient();

  const [name, setName] = createSignal("");
  const [conditionType, setConditionType] = createSignal("NewIssue");
  const [actionType, setActionType] = createSignal("Webhook");
  const [webhookUrl, setWebhookUrl] = createSignal("");
  const [showModal, setShowModal] = createSignal(false);

  const urlFieldLabel = () => {
    const at = ACTION_TYPES.find((a) => a.value === actionType());
    if (at?.urlField === "path") return "File Path";
    if (at?.urlField === "to") return "Email Address";
    return `${at?.label ?? "Webhook"} URL`;
  };

  const buildAction = () => {
    const type = actionType();
    const at = ACTION_TYPES.find((a) => a.value === type);
    const field = at?.urlField ?? "url";
    return { type, [field]: webhookUrl() };
  };

  const alertsQuery = createQuery(() => ({
    queryKey: ["alerts", params.project],
    queryFn: () =>
      api.get<AlertRuleResponse[]>(`/internal/projects/${params.project}/alerts`),
  }));

  const createMut = createMutation(() => ({
    mutationFn: (input: {
      name: string;
      conditions: { type: string; threshold?: number; window_seconds?: number }[];
      actions: Record<string, string>[];
    }) => api.post<AlertRuleResponse>(`/internal/projects/${params.project}/alerts`, input),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: ["alerts", params.project],
      });
      setName("");
      setConditionType("NewIssue");
      setActionType("Webhook");
      setWebhookUrl("");
      setShowModal(false);
    },
  }));

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
                  actions: [buildAction()],
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
            <label class="field-label">Action Type</label>
            <select
              value={actionType()}
              onChange={(e) => setActionType(e.currentTarget.value)}
              class="input"
            >
              <For each={ACTION_TYPES}>
                {(at) => <option value={at.value}>{at.label}</option>}
              </For>
            </select>
          </div>
          <div class="form-field">
            <label class="field-label">{urlFieldLabel()}</label>
            <input
              type={actionType() === "Email" ? "email" : actionType() === "LogFile" ? "text" : "url"}
              value={webhookUrl()}
              onInput={(e) => setWebhookUrl(e.currentTarget.value)}
              class="input"
              placeholder={
                actionType() === "LogFile" ? "/var/log/bugs-alerts.log"
                : actionType() === "Email" ? "alerts@example.com"
                : "https://hooks.example.com/webhook"
              }
            />
          </div>
        </div>
      </Modal>

      <Show when={!alertsQuery.isPending} fallback={<LoadingSpinner />}>
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
                  <th>Action</th>
                  <th>Target</th>
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
                      <td class="text-secondary">
                        {rule.actions.length > 0 ? actionLabel(rule.actions[0].type) : "\u2014"}
                      </td>
                      <td class="text-secondary text-mono" style={{ "font-size": "12px" }}>
                        {firstActionUrl(rule)}
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
