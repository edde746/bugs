/// Build a DSN string from components
pub fn build_dsn(scheme: &str, host: &str, public_key: &str, project_id: i64) -> String {
    format!("{scheme}://{public_key}@{host}/{project_id}")
}
