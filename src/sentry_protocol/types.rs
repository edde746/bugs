use std::collections::HashMap;

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
    #[serde(default, deserialize_with = "deserialize_message")]
    pub message: Option<String>,
    #[serde(default)]
    pub logentry: Option<LogEntry>,
    #[serde(default)]
    pub exception: Option<ExceptionInterface>,
    #[serde(default)]
    pub threads: Option<ThreadsInterface>,
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
    #[serde(default, rename = "type")]
    pub event_type: Option<String>,
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LogEntry {
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub params: Option<serde_json::Value>,
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExceptionInterface {
    #[serde(default)]
    pub values: Vec<ExceptionValue>,
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExceptionValue {
    #[serde(default, rename = "type")]
    pub exception_type: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub module: Option<String>,
    #[serde(default)]
    pub stacktrace: Option<Stacktrace>,
    #[serde(default)]
    pub mechanism: Option<Mechanism>,
    #[serde(default)]
    pub thread_id: Option<serde_json::Value>,
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Mechanism {
    #[serde(default, rename = "type")]
    pub mechanism_type: Option<String>,
    #[serde(default)]
    pub handled: Option<bool>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
    #[serde(default)]
    pub meta: Option<serde_json::Value>,
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Stacktrace {
    #[serde(default)]
    pub frames: Vec<StackFrame>,
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
    #[serde(default)]
    pub instruction_addr: Option<String>,
    #[serde(default)]
    pub symbol_addr: Option<String>,
    #[serde(default)]
    pub image_addr: Option<String>,
    #[serde(default)]
    pub package: Option<String>,
    #[serde(default)]
    pub addr_mode: Option<String>,
    #[serde(default)]
    pub trust: Option<String>,
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ThreadsInterface {
    #[serde(default)]
    pub values: Vec<ThreadValue>,
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ThreadValue {
    #[serde(default)]
    pub id: Option<serde_json::Value>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub crashed: Option<bool>,
    #[serde(default)]
    pub current: Option<bool>,
    #[serde(default)]
    pub stacktrace: Option<Stacktrace>,
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BreadcrumbsInterface {
    #[serde(default)]
    pub values: Vec<Breadcrumb>,
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
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
            Ok(Some(BreadcrumbsInterface {
                values,
                ..Default::default()
            }))
        }
        Some(serde_json::Value::Object(map)) => {
            let iface: BreadcrumbsInterface =
                serde_json::from_value(serde_json::Value::Object(map)).unwrap_or_default();
            Ok(Some(iface))
        }
        Some(_) => Ok(None),
    }
}

/// Deserialize `message` from either a bare string or the message-interface
/// object `{"formatted": ..., "message": ..., "params": [...]}` that the Dart
/// SDK sends for `captureMessage`. Sentry treats the attribute as an alias of
/// `logentry`; the formatted text wins over the unformatted template because
/// that is what the event is titled with. Any other shape carries no message.
fn deserialize_message<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value: Option<serde_json::Value> = Option::deserialize(deserializer)?;
    Ok(match value {
        Some(serde_json::Value::String(message)) => Some(message),
        Some(serde_json::Value::Object(mut map)) => {
            ["formatted", "message"]
                .into_iter()
                .find_map(|key| match map.remove(key) {
                    Some(serde_json::Value::String(message)) => Some(message),
                    _ => None,
                })
        }
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message_of(event: serde_json::Value) -> Option<String> {
        serde_json::from_value::<SentryEvent>(event)
            .unwrap()
            .message
    }

    #[test]
    fn object_message_reads_as_its_formatted_text() {
        let dart = serde_json::json!({
            "level": "warning",
            "message": {"formatted": "Previous Android application exit"},
        });
        let plain = serde_json::json!({
            "level": "warning",
            "message": "Previous Android application exit",
        });
        assert_eq!(message_of(dart), message_of(plain));
    }

    #[test]
    fn object_message_prefers_formatted_over_template() {
        let both = serde_json::json!({"message": {
            "message": "Retry %s",
            "formatted": "Retry 3",
            "params": ["3"],
        }});
        assert_eq!(message_of(both).as_deref(), Some("Retry 3"));

        let template_only =
            serde_json::json!({"message": {"message": "Retry %s", "params": ["3"]}});
        assert_eq!(message_of(template_only).as_deref(), Some("Retry %s"));
    }

    #[test]
    fn unusable_message_shapes_keep_the_event() {
        for message in [serde_json::json!({"params": ["3"]}), serde_json::json!(42)] {
            assert_eq!(message_of(serde_json::json!({"message": message})), None);
        }
    }
}
