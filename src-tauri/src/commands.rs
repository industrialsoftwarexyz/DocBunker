//! Tauri command layer.
//!
//! The UI talks to the backend exclusively through these commands. It never
//! receives paths or file contents directly; the file dialog and file reading
//! happen here in trusted Rust code.

use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use base64::Engine;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::app_config::PendingDocument;
use docbunker_core::{imaging, DocBunkerError, DocumentManager};
use docbunker_native_broker::has_supported_signature;
use docbunker_renderer_api::limits::MAX_DOCUMENT_SIZE;
use docbunker_renderer_api::{DocumentInfo, RenderOptions, RenderedPage};

/// Shared application state managed by Tauri.
pub struct AppState {
    pub manager: Option<Arc<DocumentManager>>,
    pub backend_isolated: bool,
}

pub struct StartupFiles(pub Mutex<VecDeque<PendingDocument>>);

impl AppState {
    fn manager(&self) -> Result<Arc<DocumentManager>, DocBunkerError> {
        self.manager
            .clone()
            .ok_or(DocBunkerError::SandboxStartupFailed)
    }
}

#[derive(Debug, Serialize)]
pub struct BackendStatusDto {
    pub available: bool,
    pub isolated: bool,
}

#[tauri::command]
pub fn get_backend_status(state: State<'_, AppState>) -> BackendStatusDto {
    BackendStatusDto {
        available: state.manager.is_some(),
        isolated: state.backend_isolated,
    }
}

/// Opaque handle the frontend round-trips; no paths ever reach the UI.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DocumentHandleDto {
    pub session: u64,
    pub document: u64,
}

impl DocumentHandleDto {
    fn from_handle(handle: docbunker_core::DocumentHandle) -> Self {
        Self {
            session: handle.session,
            document: handle.document,
        }
    }

    fn to_handle(self) -> docbunker_core::DocumentHandle {
        docbunker_core::DocumentHandle {
            session: self.session,
            document: self.document,
        }
    }
}

/// Result of opening a document: opaque handle + a display name (basename
/// only, truncated). The UI cannot read files with this string.
#[derive(Debug, Serialize)]
pub struct OpenResultDto {
    pub handle: DocumentHandleDto,
    pub file_name: String,
}

async fn open_file(
    manager: Arc<DocumentManager>,
    file: PathBuf,
) -> Result<OpenResultDto, DocBunkerError> {
    let file_name = file
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("document")
        .chars()
        .take(128)
        .collect::<String>();
    let handle = tauri::async_runtime::spawn_blocking(move || {
        let bytes = docbunker_core::io::read_document_file(&file, MAX_DOCUMENT_SIZE)?;
        manager.open(bytes)
    })
    .await
    .map_err(|_| DocBunkerError::InternalError)??;

    Ok(OpenResultDto {
        handle: DocumentHandleDto::from_handle(handle),
        file_name,
    })
}

#[derive(Debug, Serialize)]
pub struct DocumentInfoDto {
    pub page_count: u32,
    pub width: u32,
    pub height: u32,
    pub format: &'static str,
}

impl From<&DocumentInfo> for DocumentInfoDto {
    fn from(info: &DocumentInfo) -> Self {
        Self {
            page_count: info.page_count,
            width: info.width,
            height: info.height,
            format: info.format.label(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RenderedPageDto {
    pub width: u32,
    pub height: u32,
    /// PNG data URL of the rendered page (our own encoding, ADR-002).
    pub data_url: String,
}

fn rendered_to_dto(rendered: &RenderedPage) -> Result<RenderedPageDto, DocBunkerError> {
    let png = imaging::encode_rgba_to_png(rendered)?;
    let data_url = format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png)
    );
    Ok(RenderedPageDto {
        width: rendered.width,
        height: rendered.height,
        data_url,
    })
}

/// Open a file picked through the native dialog and render it in a sandbox
/// session. Returns an opaque handle.
#[tauri::command]
pub async fn open_document(state: State<'_, AppState>) -> Result<OpenResultDto, DocBunkerError> {
    // rfd's dialog is blocking; run it on a worker thread so the UI never
    // blocks. The frontend never sees the picked path (trust boundary).
    let file = tauri::async_runtime::spawn_blocking(|| {
        rfd::FileDialog::new()
            .add_filter(
                "Documents",
                &["pdf", "png", "jpg", "jpeg", "webp", "docx", "pptx", "xlsx"],
            )
            .pick_file()
    })
    .await
    .map_err(|_| DocBunkerError::InternalError)?
    .ok_or(DocBunkerError::Cancelled)?;

    let manager = state.manager()?;
    open_file(manager, file).await
}

const SUPPORTED_EXTENSIONS: [&str; 8] =
    ["pdf", "png", "jpg", "jpeg", "webp", "docx", "pptx", "xlsx"];

/// Open a document by its filesystem path (used by drag-and-drop).
#[tauri::command]
pub async fn open_document_by_path(
    state: State<'_, AppState>,
    path: String,
) -> Result<OpenResultDto, DocBunkerError> {
    let path = PathBuf::from(&path);
    let valid = path.is_file()
        && path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| {
                SUPPORTED_EXTENSIONS
                    .iter()
                    .any(|supported| ext.eq_ignore_ascii_case(supported))
            })
        && has_supported_signature(&path);
    if !valid {
        return Err(DocBunkerError::Cancelled);
    }
    let manager = state.manager()?;
    open_file(manager, path).await
}

/// Consume the file passed by an operating-system file association, if any.
#[tauri::command]
pub async fn open_startup_document(
    state: State<'_, AppState>,
    startup_files: State<'_, StartupFiles>,
) -> Result<Option<OpenResultDto>, DocBunkerError> {
    let pending = startup_files
        .0
        .lock()
        .map_err(|_| DocBunkerError::InternalError)?
        .pop_front();
    match pending {
        Some(pending) => {
            let result = open_file(state.manager()?, pending.path).await;
            if let Some(ack_path) = pending.ack_path {
                let status = if result.is_ok() { "ok" } else { "error" };
                // `create_new`: the ack file must not pre-exist. If a local
                // process planted a symlink/junction at `result` before this
                // write, the open fails instead of following it.
                let write = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&ack_path)
                    .and_then(|mut file| file.write_all(status.as_bytes()));
                if let Err(error) = write {
                    tracing::warn!(%error, "failed to acknowledge associated document");
                }
            }
            result.map(Some)
        }
        None => Ok(None),
    }
}

/// Return minimal metadata for an open document.
#[tauri::command]
pub async fn get_document_info(
    state: State<'_, AppState>,
    handle: DocumentHandleDto,
) -> Result<DocumentInfoDto, DocBunkerError> {
    let manager = state.manager()?;
    let info = tauri::async_runtime::spawn_blocking(move || {
        manager.get_document_info(&handle.to_handle())
    })
    .await
    .map_err(|_| DocBunkerError::InternalError)??;
    Ok(DocumentInfoDto::from(&info))
}

/// Render one page at the requested pixel size; returns a PNG data URL.
#[tauri::command]
pub async fn render_page(
    state: State<'_, AppState>,
    handle: DocumentHandleDto,
    page: u32,
    target_width: u32,
    target_height: u32,
) -> Result<RenderedPageDto, DocBunkerError> {
    let manager = state.manager()?;
    let rendered = tauri::async_runtime::spawn_blocking(move || {
        let options = RenderOptions {
            target_width,
            target_height,
        };
        manager.render_page(&handle.to_handle(), page, options)
    })
    .await
    .map_err(|_| DocBunkerError::InternalError)??;

    rendered_to_dto(&rendered)
}

/// Close a document and destroy its sandbox session.
#[tauri::command]
pub async fn close_document(
    state: State<'_, AppState>,
    handle: DocumentHandleDto,
) -> Result<(), DocBunkerError> {
    let manager = state.manager()?;
    tauri::async_runtime::spawn_blocking(move || manager.close(&handle.to_handle()))
        .await
        .map_err(|_| DocBunkerError::InternalError)?
}
