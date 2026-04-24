//! Debug Information File (DIF) upload for native symbolication.
//!
//! Accepts dSYM / PDB / ELF / Dart split-debug-info payloads (either raw
//! or ZIP-packed) and pre-processes each contained `Object` into a
//! `SymCache` stored on disk. A row in `artifact_debug_ids` indexes each
//! SymCache by `(debug_id, kind='native')` for per-project lookup at
//! ingest time.

use std::io::{Cursor, Read, Write};

use axum::{
    Json,
    extract::{Multipart, Path, State},
    http::StatusCode,
};
use serde::Serialize;
use symbolic::common::ByteView;
use symbolic::debuginfo::Archive;
use symbolic::symcache::SymCacheConverter;
use tracing::warn;

use crate::AppState;
use crate::models::release::Release;
use crate::util::id::normalize_debug_id;
use crate::worker::native_symbolication;

#[derive(Serialize)]
pub struct UploadedDif {
    pub debug_id: String,
    pub arch: String,
    pub code_id: Option<String>,
    pub source_name: String,
    pub size: usize,
}

#[derive(Serialize)]
pub struct UploadError {
    pub entry: String,
    pub reason: String,
}

#[derive(Serialize)]
pub struct UploadResponse {
    pub uploaded: Vec<UploadedDif>,
    pub errors: Vec<UploadError>,
}

struct ConvertedDif {
    debug_id: String,
    arch: String,
    code_id: Option<String>,
    source_name: String,
    symcache_bytes: Vec<u8>,
}

struct ConversionError {
    entry: String,
    reason: String,
}

pub async fn upload_dsym(
    State(state): State<AppState>,
    Path((_org, project_slug)): Path<(String, String)>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<UploadResponse>), (StatusCode, String)> {
    let project_id: i64 = {
        let row: Option<(i64,)> = sqlx::query_as("SELECT id FROM projects WHERE slug = ?")
            .bind(&project_slug)
            .fetch_optional(state.db.reader())
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        row.ok_or((StatusCode::NOT_FOUND, "project not found".to_string()))?
            .0
    };

    let mut file_content: Option<Vec<u8>> = None;
    let mut release_version: Option<String> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
    {
        match field.name().unwrap_or("") {
            "file" => {
                let data = field
                    .bytes()
                    .await
                    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
                file_content = Some(data.to_vec());
            }
            "release" => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
                if !text.is_empty() {
                    release_version = Some(text);
                }
            }
            _ => {}
        }
    }

    let file_content =
        file_content.ok_or((StatusCode::BAD_REQUEST, "Missing 'file' field".to_string()))?;
    if file_content.len() > state.config.uploads.max_bytes {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            "Upload too large".to_string(),
        ));
    }

    let release_id: Option<i64> = match release_version {
        Some(version) => {
            let row: Release = sqlx::query_as(
                "INSERT INTO releases (org_id, version) VALUES (1, ?) \
                 ON CONFLICT(org_id, version) DO UPDATE SET org_id=org_id \
                 RETURNING *",
            )
            .bind(&version)
            .fetch_one(state.db.writer())
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            sqlx::query(
                "INSERT OR IGNORE INTO release_projects (release_id, project_id) VALUES (?, ?)",
            )
            .bind(row.id)
            .bind(project_id)
            .execute(state.db.writer())
            .await
            .ok();
            Some(row.id)
        }
        None => None,
    };

    let max_bytes = state.config.uploads.max_bytes;
    let conversion = tokio::task::spawn_blocking(move || convert_upload(file_content, max_bytes))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let (converted, mut errors) = split_conversion(conversion);

    let mut uploaded = Vec::new();
    for dif in converted {
        let shard = &dif.debug_id[..2];
        let dir_path = format!(
            "{}/native/{}/{}",
            state.config.artifacts_dir, shard, dif.debug_id
        );
        let target = format!("{dir_path}/{}.symc", dif.arch);

        let size = dif.symcache_bytes.len();
        if let Err(e) = write_atomic(&dir_path, &target, dif.symcache_bytes).await {
            errors.push(UploadError {
                entry: format!("{} ({})", dif.source_name, dif.arch),
                reason: format!("write: {e}"),
            });
            continue;
        }
        native_symbolication::invalidate_symcache_path(&target);

        let result = sqlx::query(
            "INSERT INTO artifact_debug_ids \
                 (debug_id, project_id, release_id, file_path, source_name, arch, code_id, kind) \
             VALUES (?, ?, ?, ?, ?, ?, ?, 'native') \
             ON CONFLICT(debug_id, kind) DO UPDATE SET \
                 project_id = excluded.project_id, \
                 release_id = excluded.release_id, \
                 file_path = excluded.file_path, \
                 source_name = excluded.source_name, \
                 arch = excluded.arch, \
                 code_id = excluded.code_id",
        )
        .bind(&dif.debug_id)
        .bind(project_id)
        .bind(release_id)
        .bind(&target)
        .bind(&dif.source_name)
        .bind(&dif.arch)
        .bind(&dif.code_id)
        .execute(state.db.writer())
        .await;

        if let Err(e) = result {
            warn!("artifact_debug_ids insert failed: {e}");
            errors.push(UploadError {
                entry: format!("{} ({})", dif.source_name, dif.arch),
                reason: format!("db: {e}"),
            });
            continue;
        }

        uploaded.push(UploadedDif {
            debug_id: dif.debug_id,
            arch: dif.arch,
            code_id: dif.code_id,
            source_name: dif.source_name,
            size,
        });
    }

    Ok((
        StatusCode::CREATED,
        Json(UploadResponse { uploaded, errors }),
    ))
}

/// CPU-bound: parse the upload, extract objects, build SymCaches. Runs
/// off the tokio runtime. No I/O or DB access here.
fn convert_upload(
    file_content: Vec<u8>,
    max_bytes: usize,
) -> Vec<Result<ConvertedDif, ConversionError>> {
    let mut out = Vec::new();

    let is_zip = file_content.len() >= 4 && &file_content[..4] == b"PK\x03\x04";
    if is_zip {
        let cursor = Cursor::new(&file_content[..]);
        let mut zip = match zip::ZipArchive::new(cursor) {
            Ok(z) => z,
            Err(e) => {
                out.push(Err(ConversionError {
                    entry: "archive".to_string(),
                    reason: format!("zip: {e}"),
                }));
                return out;
            }
        };
        for i in 0..zip.len() {
            let entry = match zip.by_index(i) {
                Ok(e) => e,
                Err(e) => {
                    out.push(Err(ConversionError {
                        entry: format!("#{i}"),
                        reason: e.to_string(),
                    }));
                    continue;
                }
            };
            if !entry.is_file() {
                continue;
            }
            let name = entry.name().to_string();
            let declared = entry.size() as usize;
            if declared > max_bytes {
                out.push(Err(ConversionError {
                    entry: name,
                    reason: "entry exceeds uploads.max_bytes".to_string(),
                }));
                continue;
            }
            let mut buf = Vec::with_capacity(declared.min(1 << 20));
            if let Err(e) = entry.take(max_bytes as u64 + 1).read_to_end(&mut buf) {
                out.push(Err(ConversionError {
                    entry: name,
                    reason: e.to_string(),
                }));
                continue;
            }
            if buf.len() > max_bytes {
                out.push(Err(ConversionError {
                    entry: name,
                    reason: "entry exceeds uploads.max_bytes".to_string(),
                }));
                continue;
            }
            convert_one(&name, buf, &mut out);
        }
    } else {
        convert_one("upload", file_content, &mut out);
    }

    out
}

fn convert_one(
    source_name: &str,
    bytes: Vec<u8>,
    out: &mut Vec<Result<ConvertedDif, ConversionError>>,
) {
    let view = ByteView::from_vec(bytes);
    let archive = match Archive::parse(&view) {
        Ok(a) => a,
        Err(_) => {
            // Not every file in a ZIP is a DIF (e.g. Info.plist); silently skip.
            return;
        }
    };

    for object_result in archive.objects() {
        let object = match object_result {
            Ok(o) => o,
            Err(e) => {
                out.push(Err(ConversionError {
                    entry: source_name.to_string(),
                    reason: format!("object: {e}"),
                }));
                continue;
            }
        };

        let debug_id = normalize_debug_id(&object.debug_id().to_string());
        if debug_id.is_empty() {
            continue;
        }
        let arch = object.arch().name().to_string();
        let code_id = object.code_id().map(|c| c.to_string());

        let mut converter = SymCacheConverter::new();
        if let Err(e) = converter.process_object(&object) {
            out.push(Err(ConversionError {
                entry: format!("{source_name} ({arch})"),
                reason: format!("symcache process: {e}"),
            }));
            continue;
        }
        let mut symcache_bytes: Vec<u8> = Vec::new();
        if let Err(e) = converter.serialize(&mut symcache_bytes) {
            out.push(Err(ConversionError {
                entry: format!("{source_name} ({arch})"),
                reason: format!("symcache serialize: {e}"),
            }));
            continue;
        }

        out.push(Ok(ConvertedDif {
            debug_id,
            arch,
            code_id,
            source_name: source_name.to_string(),
            symcache_bytes,
        }));
    }
}

fn split_conversion(
    conv: Vec<Result<ConvertedDif, ConversionError>>,
) -> (Vec<ConvertedDif>, Vec<UploadError>) {
    let mut ok = Vec::new();
    let mut errs = Vec::new();
    for r in conv {
        match r {
            Ok(d) => ok.push(d),
            Err(ConversionError { entry, reason }) => errs.push(UploadError { entry, reason }),
        }
    }
    (ok, errs)
}

async fn write_atomic(dir: &str, target: &str, bytes: Vec<u8>) -> std::io::Result<()> {
    tokio::fs::create_dir_all(dir).await?;
    let tmp = format!("{target}.tmp.{}", std::process::id());
    let tmp_move = tmp.clone();
    // sync_all blocks on disk flush; must run off the tokio reactor.
    tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp_move)?;
        f.write_all(&bytes)?;
        f.sync_all()
    })
    .await
    .map_err(std::io::Error::other)??;
    tokio::fs::rename(&tmp, target).await?;
    Ok(())
}
