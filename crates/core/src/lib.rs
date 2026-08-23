//! DocBunker trusted core.
//!
//! This crate is the orchestration layer between the Tauri shell and the
//! sandbox backend. It never parses documents; it manages sessions, renders
//! pages through the [`SandboxBackend`], keeps a bounded page cache and
//! encodes our own RGBA buffers to PNG for the UI (ADR-002).
//!
//! Trust boundary: everything here is trusted. Untrusted data only arrives as
//! already-validated `RenderedPage` values from the sandbox crate.

pub mod cache;
pub mod error;
pub mod imaging;
pub mod io;
pub mod manager;

pub use docbunker_renderer_api::{DocumentInfo, RenderOptions, RenderedPage};
pub use error::DocBunkerError;
pub use manager::{DocumentHandle, DocumentManager};
