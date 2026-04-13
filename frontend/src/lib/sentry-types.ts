export interface Project {
  id: number;
  org_id: number;
  name: string;
  slug: string;
  platform: string | null;
  created_at: string;
}

export interface ProjectKey {
  id: number;
  project_id: number;
  public_key: string;
  label: string;
  is_active: boolean;
  dsn: string;
  created_at: string;
}

export interface Issue {
  id: number;
  project_id: number;
  fingerprint: string;
  title: string;
  culprit: string | null;
  level: string;
  status: string;
  first_seen: string;
  last_seen: string;
  event_count: number;
  metadata: string | null;
}

export interface IssueListResponse {
  issues: Issue[];
  nextCursor: string | null;
}

export interface Event {
  id: number;
  event_id: string;
  project_id: number;
  issue_id: number | null;
  timestamp: string;
  received_at: string;
  level: string;
  platform: string | null;
  release: string | null;
  environment: string | null;
  transaction_name: string | null;
  trace_id: string | null;
  message: string | null;
  title: string | null;
  exception_values: string | null;
  stacktrace_functions: string | null;
  data: string;
}

export interface EventListResponse {
  events: Event[];
  nextCursor: string | null;
}

export interface SearchResponse {
  results: Event[];
}

export interface AlertCondition {
  type: string;
  threshold?: number;
  window_seconds?: number;
  attribute?: string;
  match_type?: string;
  value?: string;
}

export interface AlertAction {
  type: string;
  url?: string;
  webhook_url?: string;
  to?: string;
  path?: string;
}

export interface AlertRuleResponse {
  id: number;
  project_id: number;
  name: string;
  enabled: boolean;
  conditions: AlertCondition[];
  actions: AlertAction[];
  frequency: number;
  last_fired: string | null;
  created_at: string;
}

export interface IssueFilterOptions {
  releases: string[];
  environments: string[];
  levels: string[];
}

export interface ProjectReleaseSummary {
  version: string;
  created_at: string;
  file_count: number;
}

export interface CreateProjectInput {
  name: string;
  slug: string;
  platform?: string;
  public_key?: string;
}

export interface UpdateIssueInput {
  status: string;
  snoozeUntil?: string;
  snoozeEventCount?: number;
}

export interface BulkUpdateIssuesInput extends UpdateIssueInput {
  ids: number[];
}

export interface BulkDeleteIssuesInput {
  ids: number[];
}

export interface IssueComment {
  id: number;
  issue_id: number;
  text: string;
  created_at: string;
}

export interface IssueActivity {
  id: number;
  issue_id: number;
  kind: string;
  data: string | null;
  created_at: string;
}

export interface ProjectStats {
  timeseries: [string, number][];
}
