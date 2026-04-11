use axum::{Router, extract::Path, http::{StatusCode, header}, response::IntoResponse, routing::get};
use rust_embed::Embed;

use crate::AppState;

#[derive(Embed)]
#[folder = "frontend/dist/"]
struct Asset;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/assets/{*path}", get(serve_asset))
        .fallback(get(serve_spa))
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
            ).into_response()
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
            "<html><body><h1>Bugs</h1><p>Frontend not built. Run: cd frontend && npm run build</p></body></html>".as_bytes().to_vec(),
        ).into_response(),
    }
}
