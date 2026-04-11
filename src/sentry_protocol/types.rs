use serde::{Deserialize, Deserializer, Serialize};

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
    #[serde(default, deserialize_with = "deserialize_breadcrumbs")]
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
    pub fingerprint: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    pub modules: Option<serde_json::Value>,
    #[serde(default)]
    pub debug_meta: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub params: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExceptionInterface {
    #[serde(default)]
    pub values: Vec<ExceptionValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExceptionValue {
    #[serde(default, rename = "type")]
    pub exception_type: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub module: Option<String>,
    #[serde(default)]
    pub stacktrace: Option<Stacktrace>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stacktrace {
    #[serde(default)]
    pub frames: Vec<StackFrame>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackFrame {
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default)]
    pub function: Option<String>,
    #[serde(default)]
    pub module: Option<String>,
    #[serde(default)]
    pub lineno: Option<u32>,
    #[serde(default)]
    pub colno: Option<u32>,
    #[serde(default)]
    pub abs_path: Option<String>,
    #[serde(default)]
    pub in_app: Option<bool>,
    #[serde(default)]
    pub context_line: Option<String>,
    #[serde(default)]
    pub pre_context: Option<Vec<String>>,
    #[serde(default)]
    pub post_context: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreadcrumbsInterface {
    #[serde(default)]
    pub values: Vec<Breadcrumb>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Breadcrumb {
    #[serde(default)]
    pub timestamp: Option<serde_json::Value>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default, rename = "type")]
    pub breadcrumb_type: Option<String>,
    #[serde(default)]
    pub level: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
}

/// Deserialize breadcrumbs from either `{"values": [...]}` or a bare `[...]`.
/// The Node.js SDK sends a bare array, Python SDK sends the wrapped form.
fn deserialize_breadcrumbs<'de, D>(
    deserializer: D,
) -> Result<Option<BreadcrumbsInterface>, D::Error>
where
    D: Deserializer<'de>,
{
    let value: Option<serde_json::Value> = Option::deserialize(deserializer)?;
    match value {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Array(arr)) => {
            let values: Vec<Breadcrumb> = arr
                .into_iter()
                .filter_map(|v| serde_json::from_value(v).ok())
                .collect();
            Ok(Some(BreadcrumbsInterface { values }))
        }
        Some(serde_json::Value::Object(map)) => {
            let iface: BreadcrumbsInterface =
                serde_json::from_value(serde_json::Value::Object(map))
                    .unwrap_or(BreadcrumbsInterface { values: vec![] });
            Ok(Some(iface))
        }
        Some(_) => Ok(None),
    }
}
