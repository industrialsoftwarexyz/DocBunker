//! [`SubprocessBackend`]: a [`SandboxBackend`] over a bare worker child.
//!
//! **Security**: this backend provides **no isolation** — the worker runs with
//! the host's privileges. It exists so the real decoding pipeline can be
//! exercised end-to-end (and to carry the session logic the `runsc` backend
//! reuses in Phase 4). It must never be used with untrusted documents outside
//! development and tests.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::backend::SandboxBackend;
use crate::config::SandboxConfig;
use crate::error::SandboxError;
use crate::process::{ProcessTransport, WorkerSession};
use crate::session::{DocumentId, DocumentInput, SandboxKind, SandboxSession};
use docbunker_renderer_api::{DocumentInfo, RenderOptions, RenderedPage};

struct SubprocessSession {
    config: SandboxConfig,
    worker: WorkerSession,
}

/// Backend that renders documents in a bare worker subprocess (no isolation).
pub struct SubprocessBackend {
    worker_bin: PathBuf,
    next_session_id: u64,
    next_document_id: u64,
    sessions: HashMap<u64, SubprocessSession>,
}

impl SubprocessBackend {
    pub fn new(worker_bin: impl AsRef<Path>) -> Self {
        Self {
            worker_bin: worker_bin.as_ref().to_path_buf(),
            next_session_id: 0,
            next_document_id: 0,
            sessions: HashMap::new(),
        }
    }
}

impl SandboxBackend for SubprocessBackend {
    fn initialize(&mut self) -> Result<(), SandboxError> {
        if !self.worker_bin.is_file() {
            return Err(SandboxError::BackendUnsupported(
                "renderer worker binary not found",
            ));
        }
        tracing::warn!("SubprocessBackend: NO isolation; development use only");
        Ok(())
    }

    fn start_session(&mut self, config: SandboxConfig) -> Result<SandboxSession, SandboxError> {
        config
            .validate()
            .map_err(|msg| SandboxError::Internal(msg.into()))?;
        self.next_session_id += 1;
        let id = self.next_session_id;
        let transport = ProcessTransport::spawn(&self.worker_bin, config.operation_timeout)?;
        self.sessions.insert(
            id,
            SubprocessSession {
                config,
                worker: WorkerSession::new(transport),
            },
        );
        Ok(SandboxSession {
            id,
            kind: SandboxKind::Subprocess,
        })
    }

    fn send_document(
        &mut self,
        session: &mut SandboxSession,
        document: DocumentInput,
    ) -> Result<DocumentId, SandboxError> {
        let session_state = self
            .sessions
            .get_mut(&session.id)
            .ok_or(SandboxError::InvalidSession)?;

        if document.data.len() > session_state.config.max_document_size {
            return Err(docbunker_renderer_api::RenderError::DocumentTooLarge.into());
        }

        self.next_document_id += 1;
        let id = DocumentId(self.next_document_id);
        session_state.worker.send_document(document.data, id.0)?;
        Ok(id)
    }

    fn get_document_info(
        &mut self,
        session: &mut SandboxSession,
        document_id: DocumentId,
    ) -> Result<DocumentInfo, SandboxError> {
        let session_state = self
            .sessions
            .get_mut(&session.id)
            .ok_or(SandboxError::InvalidSession)?;
        session_state.worker.get_document_info(document_id.0)
    }

    fn render_page(
        &mut self,
        session: &mut SandboxSession,
        document_id: DocumentId,
        page: u32,
        options: RenderOptions,
    ) -> Result<RenderedPage, SandboxError> {
        let session_state = self
            .sessions
            .get_mut(&session.id)
            .ok_or(SandboxError::InvalidSession)?;

        if options.target_width > session_state.config.max_page_width
            || options.target_height > session_state.config.max_page_height
        {
            return Err(docbunker_renderer_api::RenderError::ResourceLimitExceeded.into());
        }
        session_state
            .worker
            .render_page(document_id.0, page, options)
    }

    fn close_session(&mut self, session: SandboxSession) -> Result<(), SandboxError> {
        let Some(mut session_state) = self.sessions.remove(&session.id) else {
            return Err(SandboxError::InvalidSession);
        };
        session_state.worker.close_all_documents();
        session_state.worker.shutdown();
        Ok(())
    }
}

impl Drop for SubprocessBackend {
    fn drop(&mut self) {
        for (_, mut session) in self.sessions.drain() {
            session.worker.terminate();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn worker_bin() -> PathBuf {
        let mut path = std::env::current_exe().unwrap();
        path.pop();
        path.pop();
        path.push(if cfg!(windows) {
            "renderer-worker.exe"
        } else {
            "renderer-worker"
        });
        path
    }

    fn png_fixture() -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(4, 3, image::Rgba([9, 8, 7, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut out, image::ImageFormat::Png)
            .expect("fixture encodes");
        out.into_inner()
    }

    /// A minimal docx container with two paragraphs.
    fn docx_fixture() -> Vec<u8> {
        use std::io::Write;
        let mut buffer = Vec::new();
        {
            let mut archive = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
            let options: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default();
            archive
                .start_file("[Content_Types].xml", options)
                .expect("content types");
            archive
                .write_all(
                    br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>"#,
                )
                .expect("content types body");
            archive
                .start_file("word/document.xml", options)
                .expect("document");
            archive
                .write_all(
                    br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:body>
  <w:p><w:r><w:t>Vista previa Office</w:t></w:r></w:p>
  <w:p><w:r><w:t>Segundo parrafo</w:t></w:r></w:p>
</w:body>
</w:document>"#,
                )
                .expect("document body");
            archive.finish().expect("archive");
        }
        buffer
    }

    #[test]
    fn full_lifecycle_with_real_decoding() {
        let mut backend = SubprocessBackend::new(worker_bin());
        backend.initialize().unwrap();
        let config = SandboxConfig {
            operation_timeout: Duration::from_secs(10),
            ..SandboxConfig::default()
        };
        let mut session = backend.start_session(config).unwrap();

        let id = backend
            .send_document(&mut session, DocumentInput::new(png_fixture()))
            .unwrap();
        let info = backend.get_document_info(&mut session, id).unwrap();
        assert_eq!(info.format, docbunker_renderer_api::DocumentFormat::Png);
        assert_eq!((info.width, info.height), (4, 3));

        let page = backend
            .render_page(
                &mut session,
                id,
                0,
                RenderOptions {
                    target_width: 32,
                    target_height: 24,
                },
            )
            .unwrap();
        page.validate().unwrap();
        assert_eq!(page.bytes.len(), 32 * 24 * 4);

        backend.close_session(session).unwrap();
    }

    #[test]
    fn missing_binary_fails_initialize() {
        let mut backend = SubprocessBackend::new("no-such-worker-binary");
        assert!(backend.initialize().is_err());
    }

    #[test]
    fn ooxml_docx_lifecycle_with_real_decoding() {
        let mut backend = SubprocessBackend::new(worker_bin());
        backend.initialize().unwrap();
        let config = SandboxConfig {
            operation_timeout: Duration::from_secs(10),
            ..SandboxConfig::default()
        };
        let mut session = backend.start_session(config).unwrap();

        let id = backend
            .send_document(&mut session, DocumentInput::new(docx_fixture()))
            .unwrap();
        let info = backend.get_document_info(&mut session, id).unwrap();
        assert_eq!(info.format, docbunker_renderer_api::DocumentFormat::Ooxml);
        assert_eq!(info.page_count, 1);

        let page = backend
            .render_page(
                &mut session,
                id,
                0,
                RenderOptions {
                    target_width: 620,
                    target_height: 877,
                },
            )
            .unwrap();
        page.validate().unwrap();
        assert_eq!(page.bytes.len(), 620 * 877 * 4);

        backend.close_session(session).unwrap();
    }
}
