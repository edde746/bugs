use axum::{
    Json, Router,
    extract::Path,
    http::{StatusCode, header},
    response::IntoResponse,
    routing::{any, get},
};
use rust_embed::Embed;
use serde_json::json;

use crate::AppState;

#[derive(Embed)]
#[folder = "frontend/dist/"]
struct Asset;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/assets/{*path}", get(serve_asset))
        // Unmatched /api/* requests return JSON 404 instead of falling
        // through to the SPA HTML — sentry-cli (and any other API client)
        // expects JSON and "200 OK text/html" surfaces as "not a JSON
        // response" or similar nonsense.
        .route("/api/{*rest}", any(api_not_found))
        .fallback(get(serve_spa))
}

async fn api_not_found() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "detail": "not found" })),
    )
}

async fn serve_asset(Path(path): Path<String>) -> impl IntoResponse {
    let asset_path = format!("assets/{path}");
    match Asset::get(&asset_path) {
        Some(content) => {
            let mime = mime_guess::from_path(&asset_path).first_or_octet_stream();
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, mime.as_ref().to_string())],
                content.data.into_owned(),
            )
                .into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn serve_spa() -> impl IntoResponse {
    match Asset::get("index.html") {
        Some(content) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html".to_string())],
            content.data.into_owned(),
        ).into_response(),
        None => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html".to_string())],
            "<html><body><h1>Bugs</h1><p>Frontend not built. Run: cd frontend && bun run build</p></body></html>".as_bytes().to_vec(),
        ).into_response(),
    }
}
