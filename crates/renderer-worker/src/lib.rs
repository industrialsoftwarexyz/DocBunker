//! The untrusted `renderer-worker`.
//!
//! This crate is the code that runs *inside* the sandbox. It has no UI, no
//! network, no updater, no shell and no knowledge of host paths. Its only job
//! is to answer protocol requests (open / info / render / close / ping) and
//! return validated raster data.
//!
//! The `Worker` core is transport-agnostic (used by the `renderer-worker`
//! binary over stdin/stdout and by the host's `InProcessTransport`), which
//! keeps the exact same code path in tests, the mock backend and the future
//! `runsc` backend.

#[cfg(feature = "pdf-mupdf")]
pub mod pdf;
pub mod pdf_hayro;
pub mod shm;

pub mod worker;

pub use shm::{PageBufferStore, SharedMemStore};
pub use worker::{
    AnyRenderer, MockRendererFactory, RendererFactory, SharedStoreFactory, StandardRendererFactory,
    Worker,
};
