//! Embedded MCP (Model Context Protocol) server.
//!
//! Exposes read + triage tools over the existing internal API handlers so
//! LLM agents can inspect and manage a running instance. Mounted at /mcp
//! behind the admin auth middleware when `config.mcp.enabled` is set
//! (BUGS_MCP__ENABLED=true). Tools call the axum handlers directly with
//! hand-built extractors, so behavior (filters, cursors, limits) is
//! identical to the internal HTTP API.

use std::time::Duration;

use crate::AppState;
use crate::api;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, Content, ErrorData, Implementation, ServerCapabilities, ServerInfo,
};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct BugsMcp {
    state: AppState,
}

/// Build the tower service mounted at /mcp. `cancel` must be cancelled at
/// shutdown so open SSE streams close and the HTTP drain can finish.
pub fn streamable_http_service(
    state: AppState,
    cancel: CancellationToken,
) -> StreamableHttpService<BugsMcp, LocalSessionManager> {
    StreamableHttpService::new(
        move || Ok(BugsMcp::new(state.clone())),
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default()
            // The default allowlist only accepts localhost Host headers and
            // 403s any remote deployment. The bearer token is the gate here,
            // same as the rest of the management API.
            .disable_allowed_hosts()
            // Periodic pings so reverse proxies don't reap idle streams.
            .with_sse_keep_alive(Some(Duration::from_secs(30)))
            .with_cancellation_token(cancel),
    )
}

/// Cap tool results so a large event payload (`data` can approach the 1 MB
/// ingest limit) doesn't flood the model's context.
const MAX_RESULT_BYTES: usize = 100 * 1024;

fn json_result<T: serde::Serialize>(value: &T) -> Result<CallToolResult, ErrorData> {
    let mut s = serde_json::to_string_pretty(value)
        .map_err(|e| ErrorData::internal_error(format!("serialization failed: {e}"), None))?;
    if s.len() > MAX_RESULT_BYTES {
        let mut cut = MAX_RESULT_BYTES;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
        s.push_str("\n... [truncated: result exceeded 100 KB]");
    }
    Ok(CallToolResult::success(vec![Content::text(s)]))
}

fn http_error(status: StatusCode, what: &str) -> ErrorData {
    match status {
        StatusCode::NOT_FOUND => ErrorData::resource_not_found(format!("{what}: not found"), None),
        StatusCode::BAD_REQUEST => {
            ErrorData::invalid_params(format!("{what}: invalid parameters"), None)
        }
        s => ErrorData::internal_error(format!("{what}: HTTP {s}"), None),
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct ListIssuesParams {
    /// Project slug, as returned by list_projects.
    pub project_slug: String,
    /// Issue status filter: "unresolved" (default), "resolved" or "ignored".
    pub status: Option<String>,
    /// Case-insensitive substring match on the issue title.
    pub query: Option<String>,
    /// Filter by level, e.g. "error", "warning", "info".
    pub level: Option<String>,
    /// Only issues with events from this release version.
    pub release: Option<String>,
    /// Only issues with events from this environment, e.g. "production".
    pub environment: Option<String>,
    /// Sort order: "last_seen" (default), "first_seen" or "events".
    pub sort: Option<String>,
    /// Opaque pagination cursor from a previous response's "nextCursor".
    pub cursor: Option<String>,
    /// Page size, 1-100 (default 25).
    pub limit: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
pub struct IssueIdParams {
    /// Numeric issue id (the `id` field from list_issues).
    pub issue_id: i64,
}

#[derive(Deserialize, JsonSchema)]
pub struct EventIdParams {
    /// Numeric event database id (the `id` field from list_events, not the
    /// `event_id` UUID).
    pub event_id: i64,
}

#[derive(Deserialize, JsonSchema)]
pub struct ListEventsParams {
    /// Numeric issue id.
    pub issue_id: i64,
    /// Page size, 1-100 (default 50).
    pub limit: Option<i64>,
    /// Opaque pagination cursor from a previous response's "nextCursor".
    pub cursor: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct SearchToolParams {
    /// Full-text query (min 2 chars). Matches message, title, exception text
    /// and stack frames.
    pub query: String,
    /// Optional project slug or numeric project id to scope the search.
    pub project: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum IssueStatus {
    Resolved,
    Ignored,
    Unresolved,
}

impl IssueStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Resolved => "resolved",
            Self::Ignored => "ignored",
            Self::Unresolved => "unresolved",
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct UpdateIssueStatusParams {
    /// Numeric issue id.
    pub issue_id: i64,
    /// New status for the issue.
    pub status: IssueStatus,
    /// Only meaningful with status=resolved: the release version that fixes
    /// the issue, or "__next__" for the next release.
    pub resolved_in_release: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct AddCommentParams {
    /// Numeric issue id.
    pub issue_id: i64,
    /// Comment text. Markdown is supported.
    pub text: String,
}

#[tool_router]
impl BugsMcp {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }

    fn state(&self) -> State<AppState> {
        State(self.state.clone())
    }

    #[tool(
        description = "List all projects with their ids, names and slugs. Call this first to discover project slugs for the other tools.",
        annotations(read_only_hint = true)
    )]
    async fn list_projects(&self) -> Result<CallToolResult, ErrorData> {
        let Json(projects) = api::projects::list_projects(self.state())
            .await
            .map_err(|s| http_error(s, "projects"))?;
        json_result(&projects)
    }

    #[tool(
        description = "List issues (grouped errors) for a project. Returns { issues, nextCursor }; pass nextCursor back as `cursor` to paginate. Defaults to unresolved issues sorted by last_seen.",
        annotations(read_only_hint = true)
    )]
    async fn list_issues(
        &self,
        Parameters(p): Parameters<ListIssuesParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let query = api::issues::IssueQuery {
            status: p.status,
            sort: p.sort,
            cursor: p.cursor,
            query: p.query,
            release: p.release,
            environment: p.environment,
            level: p.level,
            limit: p.limit.unwrap_or(25),
        };
        let Json(body) = api::issues::list_issues(self.state(), Path(p.project_slug), Query(query))
            .await
            .map_err(|s| http_error(s, "issues"))?;
        json_result(&body)
    }

    #[tool(
        description = "Get a single issue by id: title, culprit, level, status, first/last seen, event count and metadata.",
        annotations(read_only_hint = true)
    )]
    async fn get_issue(
        &self,
        Parameters(p): Parameters<IssueIdParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let Json(issue) = api::issues::get_issue(self.state(), Path(p.issue_id))
            .await
            .map_err(|s| http_error(s, "issue"))?;
        json_result(&issue)
    }

    #[tool(
        description = "List recent events for an issue as lightweight summaries (no full payload). Returns { events, nextCursor }. Use this for an overview before fetching a full event.",
        annotations(read_only_hint = true)
    )]
    async fn list_events(
        &self,
        Parameters(p): Parameters<ListEventsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let query = api::events::EventQuery {
            limit: p.limit.unwrap_or(50),
            cursor: p.cursor,
        };
        let Json(body) =
            api::events::list_events_for_issue(self.state(), Path(p.issue_id), Query(query))
                .await
                .map_err(|s| http_error(s, "events"))?;
        json_result(&body)
    }

    #[tool(
        description = "Get the most recent full event for an issue, including stack trace, local variables, breadcrumbs and request context. Large results are truncated at 100 KB.",
        annotations(read_only_hint = true)
    )]
    async fn get_latest_event(
        &self,
        Parameters(p): Parameters<IssueIdParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let Json(event) = api::events::latest_event(self.state(), Path(p.issue_id))
            .await
            .map_err(|s| http_error(s, "event"))?;
        json_result(&event)
    }

    #[tool(
        description = "Get one full event by its numeric id, including stack trace, local variables, breadcrumbs and request context. Large results are truncated at 100 KB.",
        annotations(read_only_hint = true)
    )]
    async fn get_event(
        &self,
        Parameters(p): Parameters<EventIdParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let Json(event) = api::events::get_event(self.state(), Path(p.event_id))
            .await
            .map_err(|s| http_error(s, "event"))?;
        json_result(&event)
    }

    #[tool(
        description = "Full-text search across error events (message, title, exception text, stack frames). Returns up to 50 events ranked by relevance.",
        annotations(read_only_hint = true)
    )]
    async fn search(
        &self,
        Parameters(p): Parameters<SearchToolParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = api::search::SearchParams {
            q: p.query,
            project: p.project,
        };
        let Json(body) = api::search::search_events(self.state(), Query(params))
            .await
            .map_err(|(s, msg)| match s {
                StatusCode::BAD_REQUEST => ErrorData::invalid_params(msg, None),
                StatusCode::NOT_FOUND => ErrorData::resource_not_found(msg, None),
                _ => ErrorData::internal_error(msg, None),
            })?;
        json_result(&body)
    }

    #[tool(
        description = "List the comments on an issue, oldest first.",
        annotations(read_only_hint = true)
    )]
    async fn list_comments(
        &self,
        Parameters(p): Parameters<IssueIdParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let Json(comments) = api::comments::list_comments(self.state(), Path(p.issue_id))
            .await
            .map_err(|s| http_error(s, "comments"))?;
        json_result(&comments)
    }

    #[tool(
        description = "List an issue's activity log (status changes such as resolved/ignored/unresolved, newest first).",
        annotations(read_only_hint = true)
    )]
    async fn list_activity(
        &self,
        Parameters(p): Parameters<IssueIdParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let Json(activity) = api::comments::list_activity(self.state(), Path(p.issue_id))
            .await
            .map_err(|s| http_error(s, "activity"))?;
        json_result(&activity)
    }

    #[tool(
        description = "Set an issue's status to resolved, ignored or unresolved. Optionally record the release that resolves it. Returns the updated issue."
    )]
    async fn update_issue_status(
        &self,
        Parameters(p): Parameters<UpdateIssueStatusParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let resolved_in_release = p
            .resolved_in_release
            .filter(|_| matches!(p.status, IssueStatus::Resolved));
        let input = crate::models::issue::UpdateIssue {
            status: Some(p.status.as_str().to_string()),
            snooze_until: None,
            snooze_event_count: None,
            resolved_in_release,
        };
        let Json(issue) = api::issues::update_issue(self.state(), Path(p.issue_id), Json(input))
            .await
            .map_err(|s| http_error(s, "issue"))?;
        json_result(&issue)
    }

    #[tool(
        description = "Add a comment to an issue, e.g. to record analysis findings or a triage decision. Returns the created comment."
    )]
    async fn add_comment(
        &self,
        Parameters(p): Parameters<AddCommentParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let input = crate::models::comment::CreateComment { text: p.text };
        let (_, Json(comment)) =
            api::comments::create_comment(self.state(), Path(p.issue_id), Json(input))
                .await
                .map_err(|s| http_error(s, "comment"))?;
        json_result(&comment)
    }
}

const INSTRUCTIONS: &str = "\
Bugs is a self-hosted, Sentry-compatible error tracker. These tools read and triage its data.

Typical workflow:
1. list_projects to discover project slugs.
2. list_issues(project_slug) to see open issues (default status=unresolved; paginate with nextCursor).
3. get_issue(issue_id) for one issue; list_events(issue_id) for its recent events (lightweight summaries).
4. get_latest_event(issue_id) or get_event(event_id) for the full payload: stack trace, breadcrumbs, \
request context, local variables. These can be large (truncated past 100 KB); prefer list_events \
summaries when you only need an overview.
5. search(query, project?) for full-text search across all events.
6. Triage: update_issue_status to resolve/ignore/reopen, add_comment to record findings, \
list_comments and list_activity for prior discussion and status history.

Notes: ids are numeric database ids (issue.id / event.id), not Sentry event UUIDs. \
Timestamps are UTC ISO-8601. An event's `data` field is the raw Sentry payload as a JSON string.";

#[tool_handler]
impl ServerHandler for BugsMcp {
    fn get_info(&self) -> ServerInfo {
        // ServerInfo and Implementation are #[non_exhaustive]; mutate
        // defaults instead of using struct literals.
        let mut info = ServerInfo::default();
        info.instructions = Some(INSTRUCTIONS.to_string());
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.server_info = Implementation::default();
        info.server_info.name = "bugs".to_string();
        info.server_info.version = env!("CARGO_PKG_VERSION").to_string();
        info
    }
}
