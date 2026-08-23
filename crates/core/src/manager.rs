//! The document manager: session lifecycle, rendering and caching.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::cache::PageCache;
use crate::error::DocBunkerError;
use docbunker_renderer_api::limits as render_limits;
use docbunker_renderer_api::{DocumentInfo, RenderOptions, RenderedPage};
use docbunker_sandbox::{DocumentId, DocumentInput, SandboxBackend, SandboxConfig, SandboxSession};

const MAX_OPEN_DOCUMENTS: usize = 4;

/// Opaque handle to an open document, returned to the UI layer.
///
/// The UI never sees paths or worker ids; it only round-trips this handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DocumentHandle {
    /// Manager-local session id.
    pub session: u64,
    /// Backend-assigned document id.
    pub document: u64,
}

struct OpenDocument {
    session: SandboxSession,
    document_id: DocumentId,
    info: DocumentInfo,
}

struct Inner {
    backend: Box<dyn SandboxBackend>,
    sessions: HashMap<u64, OpenDocument>,
    cache: PageCache,
    next_id: u64,
}

/// Orchestrates one sandbox backend.
///
/// All operations are serialized through an internal mutex (the backend
/// requires `&mut self`). Backend calls are synchronous; the Tauri layer runs
/// them on blocking threads so the UI never blocks (see `docs/architecture.md`).
pub struct DocumentManager {
    inner: Mutex<Inner>,
}

impl DocumentManager {
    pub fn new(backend: Box<dyn SandboxBackend>) -> Result<Self, DocBunkerError> {
        let mut backend = backend;
        backend.initialize()?;
        Ok(Self {
            inner: Mutex::new(Inner {
                backend,
                sessions: HashMap::new(),
                cache: PageCache::default(),
                next_id: 0,
            }),
        })
    }

    /// Open a document from raw bytes and return an opaque handle.
    pub fn open(&self, bytes: Vec<u8>) -> Result<DocumentHandle, DocBunkerError> {
        if bytes.is_empty() {
            return Err(DocBunkerError::InvalidDocument);
        }
        let mut inner = self.lock()?;
        if inner.sessions.len() >= MAX_OPEN_DOCUMENTS {
            return Err(DocBunkerError::ResourceLimitExceeded);
        }
        let config = SandboxConfig::default();
        let mut session = inner.backend.start_session(config)?;

        match inner
            .backend
            .send_document(&mut session, DocumentInput::new(bytes))
        {
            Ok(document_id) => {
                let info = match inner.backend.get_document_info(&mut session, document_id) {
                    Ok(info) => info,
                    Err(e) => {
                        let _ = inner.backend.close_session(session);
                        return Err(e.into());
                    }
                };
                inner.next_id += 1;
                let session_id = inner.next_id;
                inner.sessions.insert(
                    session_id,
                    OpenDocument {
                        session,
                        document_id,
                        info: info.clone(),
                    },
                );
                Ok(DocumentHandle {
                    session: session_id,
                    document: document_id.0,
                })
            }
            Err(e) => {
                // Best-effort cleanup of the half-open session.
                let _ = inner.backend.close_session(session);
                Err(e.into())
            }
        }
    }

    /// Return the cached metadata for an open document.
    pub fn get_document_info(
        &self,
        handle: &DocumentHandle,
    ) -> Result<DocumentInfo, DocBunkerError> {
        let inner = self.lock()?;
        inner
            .sessions
            .get(&handle.session)
            .filter(|open| open.document_id.0 == handle.document)
            .map(|open| open.info.clone())
            .ok_or(DocBunkerError::NoDocumentOpen)
    }

    /// Render a page, using the bounded LRU cache.
    pub fn render_page(
        &self,
        handle: &DocumentHandle,
        page: u32,
        options: RenderOptions,
    ) -> Result<RenderedPage, DocBunkerError> {
        validate_render_options(&options)?;

        let mut inner = self.lock()?;
        let open = inner
            .sessions
            .get(&handle.session)
            .filter(|open| open.document_id.0 == handle.document)
            .ok_or(DocBunkerError::NoDocumentOpen)?;

        let page_count = open.info.page_count;
        let document_id = open.document_id;
        let session = open.session.clone();

        if page >= page_count {
            return Err(DocBunkerError::InvalidPage);
        }

        if let Some(cached) = inner.cache.get(
            handle.document,
            page,
            options.target_width,
            options.target_height,
        ) {
            return Ok(cached);
        }

        let mut session = session;
        let rendered = inner
            .backend
            .render_page(&mut session, document_id, page, options)?;
        // Defense in depth: re-validate the backend's output in the host.
        rendered.validate()?;

        inner.cache.put(
            handle.document,
            page,
            options.target_width,
            options.target_height,
            rendered.clone(),
        );
        Ok(rendered)
    }

    /// Close a document and destroy its sandbox session.
    pub fn close(&self, handle: &DocumentHandle) -> Result<(), DocBunkerError> {
        let mut inner = self.lock()?;
        let matches_document = inner
            .sessions
            .get(&handle.session)
            .is_some_and(|open| open.document_id.0 == handle.document);
        if !matches_document {
            return Err(DocBunkerError::NoDocumentOpen);
        }
        let open = inner
            .sessions
            .remove(&handle.session)
            .expect("session was checked above");
        inner.cache.remove_document(handle.document);
        inner.backend.close_session(open.session)?;
        Ok(())
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Inner>, DocBunkerError> {
        self.inner.lock().map_err(|_| DocBunkerError::InternalError)
    }
}

/// Best-effort teardown of every open session when the manager goes away.
///
/// Each backend also cleans up on its own drop, but closing explicitly here
/// makes the guarantee visible (no session can outlive the manager) and keeps
/// page-cache buffers tied to document lifetimes: once the manager is gone,
/// nothing retains rendered pages.
impl Drop for DocumentManager {
    fn drop(&mut self) {
        if let Ok(mut inner) = self.inner.lock() {
            let sessions: Vec<SandboxSession> = inner
                .sessions
                .drain()
                .map(|(_, open)| open.session)
                .collect();
            for session in sessions {
                let _ = inner.backend.close_session(session);
            }
        }
    }
}

/// Validate a render target before it reaches the backend (checked math).
pub fn validate_render_options(options: &RenderOptions) -> Result<(), DocBunkerError> {
    render_limits::validate_render_target(options.target_width, options.target_height)
        .map_err(DocBunkerError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use docbunker_sandbox::{
        DocumentId, DocumentInput, MockBackend, SandboxBackend, SandboxConfig, SandboxSession,
    };

    fn manager() -> DocumentManager {
        DocumentManager::new(Box::new(MockBackend::new())).unwrap()
    }

    fn open(manager: &DocumentManager) -> DocumentHandle {
        manager
            .open(b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR".to_vec())
            .unwrap()
    }

    /// A delegating backend that counts session starts/closes, so lifecycle
    /// guarantees can be asserted from the manager's public API.
    struct CountingBackend {
        inner: Box<dyn SandboxBackend>,
        started: std::sync::Arc<std::sync::Mutex<usize>>,
        closed: std::sync::Arc<std::sync::Mutex<usize>>,
    }

    impl CountingBackend {
        fn new() -> (
            Self,
            std::sync::Arc<std::sync::Mutex<usize>>,
            std::sync::Arc<std::sync::Mutex<usize>>,
        ) {
            let started = std::sync::Arc::new(std::sync::Mutex::new(0));
            let closed = std::sync::Arc::new(std::sync::Mutex::new(0));
            (
                Self {
                    inner: Box::new(MockBackend::new()),
                    started: started.clone(),
                    closed: closed.clone(),
                },
                started,
                closed,
            )
        }
    }

    impl SandboxBackend for CountingBackend {
        fn initialize(&mut self) -> Result<(), docbunker_sandbox::SandboxError> {
            self.inner.initialize()
        }

        fn start_session(
            &mut self,
            config: SandboxConfig,
        ) -> Result<SandboxSession, docbunker_sandbox::SandboxError> {
            *self.started.lock().unwrap() += 1;
            self.inner.start_session(config)
        }

        fn send_document(
            &mut self,
            session: &mut SandboxSession,
            document: DocumentInput,
        ) -> Result<DocumentId, docbunker_sandbox::SandboxError> {
            self.inner.send_document(session, document)
        }

        fn get_document_info(
            &mut self,
            session: &mut SandboxSession,
            document_id: DocumentId,
        ) -> Result<DocumentInfo, docbunker_sandbox::SandboxError> {
            self.inner.get_document_info(session, document_id)
        }

        fn render_page(
            &mut self,
            session: &mut SandboxSession,
            document_id: DocumentId,
            page: u32,
            options: RenderOptions,
        ) -> Result<RenderedPage, docbunker_sandbox::SandboxError> {
            self.inner.render_page(session, document_id, page, options)
        }

        fn close_session(
            &mut self,
            session: SandboxSession,
        ) -> Result<(), docbunker_sandbox::SandboxError> {
            *self.closed.lock().unwrap() += 1;
            self.inner.close_session(session)
        }
    }

    #[test]
    fn each_document_gets_its_own_sandbox_session() {
        let manager = manager();
        let first = open(&manager);
        let second = open(&manager);
        // If documents shared a session, a compromised worker for one could
        // observe the other (threat model A16). They must never share.
        assert_ne!(first.session, second.session);
        manager.close(&first).unwrap();
        manager.close(&second).unwrap();
    }

    #[test]
    fn closing_one_document_isolates_other_sessions() {
        let (backend, _, _) = CountingBackend::new();
        let manager = DocumentManager::new(Box::new(backend)).unwrap();
        let first = open(&manager);
        let second = open(&manager);

        let page = manager
            .render_page(
                &second,
                0,
                RenderOptions {
                    target_width: 40,
                    target_height: 40,
                },
            )
            .unwrap();
        assert!(page.bytes.len() == 40 * 40 * 4);

        manager.close(&first).unwrap();
        // The surviving session still renders (its sandbox was not torn down
        // with the sibling)…
        let page = manager
            .render_page(
                &second,
                0,
                RenderOptions {
                    target_width: 40,
                    target_height: 40,
                },
            )
            .unwrap();
        assert!(page.bytes.len() == 40 * 40 * 4);
        // …and the closed handle is gone for good.
        assert!(matches!(
            manager.get_document_info(&first),
            Err(DocBunkerError::NoDocumentOpen)
        ));
        manager.close(&second).unwrap();
    }

    #[test]
    fn dropping_the_manager_closes_every_session() {
        let (backend, started, closed) = CountingBackend::new();
        let manager = DocumentManager::new(Box::new(backend)).unwrap();
        let _ = open(&manager);
        let _ = open(&manager);
        let _ = open(&manager);
        assert_eq!(*started.lock().unwrap(), 3);
        assert_eq!(*closed.lock().unwrap(), 0);
        drop(manager);
        assert_eq!(*closed.lock().unwrap(), 3);
    }

    #[test]
    fn close_is_idempotent_at_the_manager_level() {
        let manager = manager();
        let handle = open(&manager);
        manager.close(&handle).unwrap();
        assert!(matches!(
            manager.close(&handle),
            Err(DocBunkerError::NoDocumentOpen)
        ));
    }

    #[test]
    fn caps_simultaneously_open_sandboxes() {
        let manager = manager();
        let handles: Vec<_> = (0..MAX_OPEN_DOCUMENTS).map(|_| open(&manager)).collect();
        assert!(matches!(
            manager.open(b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR".to_vec()),
            Err(DocBunkerError::ResourceLimitExceeded)
        ));
        for handle in handles {
            manager.close(&handle).unwrap();
        }
    }

    #[test]
    fn open_info_render_close() {
        let manager = manager();
        let handle = open(&manager);

        let info = manager.get_document_info(&handle).unwrap();
        assert_eq!(info.page_count, 3);

        let page = manager
            .render_page(
                &handle,
                0,
                RenderOptions {
                    target_width: 100,
                    target_height: 120,
                },
            )
            .unwrap();
        assert_eq!(page.width, 100);
        assert_eq!(page.height, 120);
        assert_eq!(page.bytes.len(), 100 * 120 * 4);

        manager.close(&handle).unwrap();
        assert!(matches!(
            manager.get_document_info(&handle),
            Err(DocBunkerError::NoDocumentOpen)
        ));
    }

    #[test]
    fn invalid_page_rejected() {
        let manager = manager();
        let handle = open(&manager);
        assert!(matches!(
            manager.render_page(
                &handle,
                3,
                RenderOptions {
                    target_width: 100,
                    target_height: 100,
                },
            ),
            Err(DocBunkerError::InvalidPage)
        ));
    }

    #[test]
    fn absurd_render_target_rejected() {
        let manager = manager();
        let handle = open(&manager);
        assert!(matches!(
            manager.render_page(
                &handle,
                0,
                RenderOptions {
                    target_width: u32::MAX,
                    target_height: u32::MAX,
                },
            ),
            Err(DocBunkerError::ResourceLimitExceeded)
        ));
    }

    #[test]
    fn operations_on_unknown_handle_fail() {
        let manager = manager();
        let handle = DocumentHandle {
            session: 999,
            document: 999,
        };
        assert!(matches!(
            manager.get_document_info(&handle),
            Err(DocBunkerError::NoDocumentOpen)
        ));
        assert!(matches!(
            manager.close(&handle),
            Err(DocBunkerError::NoDocumentOpen)
        ));
    }

    #[test]
    fn mismatched_document_handle_cannot_access_or_close_session() {
        let manager = manager();
        let handle = open(&manager);
        let mismatched = DocumentHandle {
            session: handle.session,
            document: handle.document + 1,
        };

        assert!(matches!(
            manager.get_document_info(&mismatched),
            Err(DocBunkerError::NoDocumentOpen)
        ));
        assert!(matches!(
            manager.render_page(
                &mismatched,
                0,
                RenderOptions {
                    target_width: 100,
                    target_height: 100,
                },
            ),
            Err(DocBunkerError::NoDocumentOpen)
        ));
        assert!(matches!(
            manager.close(&mismatched),
            Err(DocBunkerError::NoDocumentOpen)
        ));

        assert!(manager.get_document_info(&handle).is_ok());
        manager.close(&handle).unwrap();
    }

    #[test]
    fn empty_document_rejected() {
        let manager = manager();
        assert!(matches!(
            manager.open(Vec::new()),
            Err(DocBunkerError::InvalidDocument)
        ));
    }

    #[test]
    fn cache_hits_same_buffer() {
        let manager = manager();
        let handle = open(&manager);
        let options = RenderOptions {
            target_width: 80,
            target_height: 100,
        };
        let first = manager.render_page(&handle, 0, options).unwrap();
        let second = manager.render_page(&handle, 0, options).unwrap();
        assert_eq!(first.bytes, second.bytes);
    }
}
