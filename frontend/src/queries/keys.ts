export const queryKeys = {
  projects: {
    all: () => ["projects"] as const,
    detail: (id: string) => ["projects", id] as const,
    keys: (id: string) => ["projects", id, "keys"] as const,
  },
  issues: {
    // Prefix used for invalidation; matches every `list` call for a
    // given project regardless of which filters were applied. Without
    // this, invalidateQueries({ queryKey: list(project, {}), exact: false })
    // fails to match stored keys whose 3rd segment is a non-empty filter
    // object because tanstack-query compares keys by deep equality.
    listPrefix: (project: string) => ["issues", "list", project] as const,
    list: (project: string, filters: Record<string, unknown>) =>
      ["issues", "list", project, filters] as const,
    detail: (id: string) => ["issues", id] as const,
    filters: (project: string) => ["issues", project, "filters"] as const,
  },
  events: {
    list: (issueId: string) => ["events", issueId] as const,
    detail: (id: string) => ["events", "detail", id] as const,
    latest: (issueId: string) => ["events", issueId, "latest"] as const,
  },
  comments: {
    list: (issueId: string) => ["comments", issueId] as const,
  },
  activity: {
    list: (issueId: string) => ["activity", issueId] as const,
  },
  stats: {
    project: (slug: string) => ["stats", slug] as const,
    issue: (id: string) => ["stats", "issue", id] as const,
  },
};
