//! Shared domain types and traits for DocBunker renderers.
//!
//! This crate is deliberately pure: no I/O, no serialization, no parsing of
//! untrusted data. It defines the seam between the trusted host and the
//! untrusted renderer implementations ([`DocumentRenderer`]), together with the
//! hard limits that every renderer and every protocol message must respect.
//!
//! See `docs/architecture.md` and `docs/protocol.md` for the surrounding design.

pub mod error;
pub mod format;
pub mod limits;
pub mod mock;
pub mod types;

pub use error::RenderError;
pub use types::{
    DocumentFormat, DocumentInfo, DocumentRenderer, PixelFormat, RenderOptions, RenderedPage,
};
