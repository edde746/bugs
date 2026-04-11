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

export interface CreateProjectInput {
  name: string;
  slug: string;
  platform?: string;
}

export interface UpdateIssueInput {
  status: string;
}

export interface ProjectStats {
  timeseries: [string, number][];
}
