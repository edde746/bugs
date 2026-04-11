use crate::config::Config;
use crate::db::DbPool;
use crate::sentry_protocol::types::SentryEvent;
use crate::util::time::now_iso;

/// Extract tags from a processed event and update tag/stats tables.
pub async fn index_event(
    db: &DbPool,
    config: &Config,
    event_row_id: i64,
    project_id: i64,
    _issue_id: i64,
    _timestamp: &str,
    event: &SentryEvent,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let now = now_iso();
    let max_values = config.ingest.max_tag_values_per_key;

    // Collect all tags: explicit event tags + auto-derived tags
    let mut tags: Vec<(String, String)> = Vec::new();

    // 1. Extract explicit tags from event.tags
    if let Some(tags_val) = &event.tags {
        extract_tags_from_value(tags_val, &mut tags);
    }

    // 2. Auto-tags
    if let Some(level) = &event.level {
        tags.push(("level".to_string(), level.clone()));
    }
    if let Some(env) = &event.environment {
        tags.push(("environment".to_string(), env.clone()));
    }
    if let Some(rel) = &event.release {
        tags.push(("release".to_string(), rel.clone()));
    }
    if let Some(platform) = &event.platform {
        tags.push(("platform".to_string(), platform.clone()));
    }
    if let Some(transaction) = &event.transaction {
        tags.push(("transaction".to_string(), transaction.clone()));
    }
    if let Some(server_name) = &event.server_name {
        tags.push(("server_name".to_string(), server_name.clone()));
    }
    if let Some(logger) = &event.logger
        && !logger.is_empty()
    {
        tags.push(("logger".to_string(), logger.clone()));
    }

    // Auto-tags from contexts: browser, os, url
    if let Some(contexts) = &event.contexts {
        if let Some(browser) = contexts.get("browser")
            && let Some(name) = browser.get("name").and_then(|v| v.as_str())
        {
            let version = browser
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if version.is_empty() {
                tags.push(("browser".to_string(), name.to_string()));
            } else {
                tags.push(("browser".to_string(), format!("{name} {version}")));
            }
            tags.push(("browser.name".to_string(), name.to_string()));
        }
        if let Some(os) = contexts.get("os")
            && let Some(name) = os.get("name").and_then(|v| v.as_str())
        {
            let version = os.get("version").and_then(|v| v.as_str()).unwrap_or("");
            if version.is_empty() {
                tags.push(("os".to_string(), name.to_string()));
            } else {
                tags.push(("os".to_string(), format!("{name} {version}")));
            }
            tags.push(("os.name".to_string(), name.to_string()));
        }
    }

    // Auto-tag from request URL
    if let Some(request) = &event.request
        && let Some(url) = request.get("url").and_then(|v| v.as_str())
    {
        tags.push(("url".to_string(), url.to_string()));
    }

    // Auto-tag from user
    if let Some(user) = &event.user {
        if let Some(email) = user.get("email").and_then(|v| v.as_str()) {
            tags.push(("user.email".to_string(), email.to_string()));
        }
        if let Some(username) = user.get("username").and_then(|v| v.as_str()) {
            tags.push(("user.username".to_string(), username.to_string()));
        }
    }

    // Deduplicate tags (keep first occurrence of each key)
    let mut seen_keys = std::collections::HashSet::new();
    tags.retain(|(k, v)| {
        if v.is_empty() {
            return false;
        }
        let key_val = format!("{k}:{v}");
        seen_keys.insert(key_val)
    });

    // Insert tags into DB
    for (key, value) in &tags {
        // Insert into event_tags
        sqlx::query(
            "INSERT INTO event_tags (event_id, project_id, key, value) VALUES (?, ?, ?, ?)",
        )
        .bind(event_row_id)
        .bind(project_id)
        .bind(key)
        .bind(value)
        .execute(db.writer())
        .await?;

        // UPSERT tag_keys: increment values_seen only on new tag_values row
        // We do the tag_values upsert first to know if it's a new value
        // But we can just always upsert tag_keys since values_seen is an approximation
        // managed by counting actual distinct values.

        // Check cardinality cap: query current values_seen for this key
        let current_values: Option<(i64,)> = sqlx::query_as(
            "SELECT values_seen FROM tag_keys WHERE project_id = ? AND key = ?",
        )
        .bind(project_id)
        .bind(key)
        .fetch_optional(db.reader())
        .await?;

        let values_seen = current_values.map(|r| r.0).unwrap_or(0);

        // UPSERT tag_values (with cardinality cap)
        if values_seen < max_values as i64 {
            // Insert or update the tag value
            sqlx::query(
                "INSERT INTO tag_values (project_id, key, value, times_seen, last_seen) \
                 VALUES (?, ?, ?, 1, ?) \
                 ON CONFLICT(project_id, key, value) DO UPDATE SET \
                    times_seen = times_seen + 1, \
                    last_seen = excluded.last_seen",
            )
            .bind(project_id)
            .bind(key)
            .bind(value)
            .bind(&now)
            .execute(db.writer())
            .await?;
        } else {
            // Over cardinality cap: only update existing values, don't create new ones
            sqlx::query(
                "UPDATE tag_values SET times_seen = times_seen + 1, last_seen = ? \
                 WHERE project_id = ? AND key = ? AND value = ?",
            )
            .bind(&now)
            .bind(project_id)
            .bind(key)
            .bind(value)
            .execute(db.writer())
            .await?;
        }

        // UPSERT tag_keys: recount distinct values
        sqlx::query(
            "INSERT INTO tag_keys (project_id, key, values_seen) \
             VALUES (?, ?, (SELECT COUNT(*) FROM tag_values WHERE project_id = ? AND key = ?)) \
             ON CONFLICT(project_id, key) DO UPDATE SET \
                values_seen = (SELECT COUNT(*) FROM tag_values WHERE project_id = excluded.project_id AND key = excluded.key)",
        )
        .bind(project_id)
        .bind(key)
        .bind(project_id)
        .bind(key)
        .execute(db.writer())
        .await?;
    }

    Ok(())
}

/// Extract tags from a serde_json::Value.
/// Sentry tags can be either an object {"key": "value"} or an array of [key, value] pairs.
fn extract_tags_from_value(val: &serde_json::Value, tags: &mut Vec<(String, String)>) {
    match val {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                if let Some(s) = v.as_str() {
                    tags.push((k.clone(), s.to_string()));
                } else if let Some(n) = v.as_i64() {
                    tags.push((k.clone(), n.to_string()));
                } else if let Some(b) = v.as_bool() {
                    tags.push((k.clone(), b.to_string()));
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                if let serde_json::Value::Array(pair) = item
                    && pair.len() == 2
                    && let (Some(k), Some(v)) = (pair[0].as_str(), pair[1].as_str())
                {
                    tags.push((k.to_string(), v.to_string()));
                }
            }
        }
        _ => {}
    }
}
