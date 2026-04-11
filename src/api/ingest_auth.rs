use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use base64::Engine;

/// Extracted Sentry auth info from ingest requests
#[derive(Debug)]
pub struct SentryAuth {
    pub sentry_key: String,
}

#[derive(Debug)]
pub struct SentryAuthRejection(String);

impl IntoResponse for SentryAuthRejection {
    fn into_response(self) -> Response {
        (StatusCode::UNAUTHORIZED, self.0).into_response()
    }
}

impl<S> FromRequestParts<S> for SentryAuth
where
    S: Send + Sync,
{
    type Rejection = SentryAuthRejection;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // 1. X-Sentry-Auth header
        if let Some(val) = parts.headers.get("x-sentry-auth")
            && let Ok(s) = val.to_str()
            && let Some(auth) = parse_sentry_auth(s)
        {
            return Ok(auth);
        }

        // 2. Authorization header
        if let Some(val) = parts.headers.get("authorization")
            && let Ok(s) = val.to_str()
        {
            if (s.starts_with("Sentry ") || s.starts_with("DSN "))
                && let Some(auth) = parse_sentry_auth(s)
            {
                return Ok(auth);
            }
            if let Some(encoded) = s.strip_prefix("Basic ")
                && let Ok(decoded) =
                    base64::engine::general_purpose::STANDARD.decode(encoded.trim())
                && let Ok(text) = String::from_utf8(decoded)
                && let Some((key, _)) = text.split_once(':')
            {
                return Ok(SentryAuth {
                    sentry_key: key.to_string(),
                });
            }
        }

        // 3. Query parameter
        if let Some(query) = parts.uri.query() {
            for pair in query.split('&') {
                if let Some(key) = pair.strip_prefix("sentry_key=") {
                    return Ok(SentryAuth {
                        sentry_key: key.to_string(),
                    });
                }
            }
        }

        Err(SentryAuthRejection("Missing Sentry authentication".into()))
    }
}

fn parse_sentry_auth(header: &str) -> Option<SentryAuth> {
    let content = header
        .trim_start_matches("Sentry ")
        .trim_start_matches("DSN ")
        .trim_start_matches("sentry ");

    for pair in content.split(',') {
        let pair = pair.trim();
        if let Some((k, v)) = pair.split_once('=')
            && k.trim() == "sentry_key"
        {
            return Some(SentryAuth {
                sentry_key: v.trim().to_string(),
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_sentry_auth_standard() {
        let auth = parse_sentry_auth(
            "Sentry sentry_key=abc123, sentry_version=7, sentry_client=raven-js/1.0",
        )
        .unwrap();
        assert_eq!(auth.sentry_key, "abc123");
    }

    #[test]
    fn test_parse_sentry_auth_minimal() {
        let auth = parse_sentry_auth("Sentry sentry_key=mykey").unwrap();
        assert_eq!(auth.sentry_key, "mykey");
    }

    #[test]
    fn test_parse_sentry_auth_dsn_prefix() {
        let auth = parse_sentry_auth("DSN sentry_key=key456").unwrap();
        assert_eq!(auth.sentry_key, "key456");
    }

    #[test]
    fn test_parse_sentry_auth_no_key() {
        assert!(parse_sentry_auth("Sentry sentry_version=7").is_none());
    }

    #[test]
    fn test_parse_sentry_auth_empty() {
        assert!(parse_sentry_auth("").is_none());
    }
}
