use serde::{Deserialize, Serialize};

/// Core Sentry event as received from SDKs
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SentryEvent {
    #[serde(default)]
    pub event_id: Option<String>,
    #[serde(default)]
    pub timestamp: Option<serde_json::Value>,
    #[serde(default)]
    pub level: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub logger: Option<String>,
    #[serde(default)]
    pub release: Option<String>,
    #[serde(default)]
    pub dist: Option<String>,
    #[serde(default)]
    pub environment: Option<String>,
    #[serde(default, rename = "server_name")]
    pub server_name: Option<String>,
    #[serde(default)]
    pub transaction: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub logentry: Option<LogEntry>,
    #[serde(default)]
    pub exception: Option<ExceptionInterface>,
    #[serde(default)]
    pub breadcrumbs: Option<BreadcrumbsInterface>,
    #[serde(default)]
    pub tags: Option<serde_json::Value>,
    #[serde(default)]
    pub extra: Option<serde_json::Value>,
    #[serde(default)]
    pub contexts: Option<serde_json::Value>,
    #[serde(default)]
    pub user: Option<serde_json::Value>,
    #[serde(default)]
    pub request: Option<serde_json::Value>,
    #[serde(default)]
    pub sdk: Option<serde_json::Value>,
    #[serde(default)]
    pub fingerprint: Option<Vec<String>>,
    #[serde(default)]
    pub modules: Option<serde_json::Value>,
    #[serde(default)]
    pub debug_meta: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub message: Option<String>,
    pub params: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExceptionInterface {
    pub values: Vec<ExceptionValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExceptionValue {
    #[serde(rename = "type")]
    pub exception_type: Option<String>,
    pub value: Option<String>,
    pub module: Option<String>,
    pub stacktrace: Option<Stacktrace>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stacktrace {
    pub frames: Vec<StackFrame>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackFrame {
    pub filename: Option<String>,
    pub function: Option<String>,
    pub module: Option<String>,
    pub lineno: Option<u32>,
    pub colno: Option<u32>,
    pub abs_path: Option<String>,
    pub in_app: Option<bool>,
    pub context_line: Option<String>,
    pub pre_context: Option<Vec<String>>,
    pub post_context: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreadcrumbsInterface {
    pub values: Vec<Breadcrumb>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Breadcrumb {
    pub timestamp: Option<serde_json::Value>,
    pub category: Option<String>,
    #[serde(rename = "type")]
    pub breadcrumb_type: Option<String>,
    pub level: Option<String>,
    pub message: Option<String>,
    pub data: Option<serde_json::Value>,
}
