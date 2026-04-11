/// Build a DSN string from components
pub fn build_dsn(scheme: &str, host: &str, public_key: &str, project_id: i64) -> String {
    format!("{scheme}://{public_key}@{host}/{project_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_dsn() {
        assert_eq!(
            build_dsn("https", "sentry.example.com", "abc123", 42),
            "https://abc123@sentry.example.com/42"
        );
    }

    #[test]
    fn test_build_dsn_with_port() {
        assert_eq!(
            build_dsn("http", "localhost:9000", "key123", 1),
            "http://key123@localhost:9000/1"
        );
    }
}
