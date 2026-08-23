//! Real image rendering: PNG, JPEG and WebP.
//!
//! This crate runs **inside the untrusted worker**. It decodes images with
//! maintained libraries (`png`, `jpeg-decoder`, `webp`/libwebp), enforces the
//! shared hard limits before any large allocation, and renders scaled RGBA
//! buffers per [`DocumentRenderer`].
//!
//! Format detection is content-based (see `docbunker-renderer-api::format`);
//! filenames and MIME types are never trusted.

pub mod image;
pub mod scaling;

pub use image::{decode_embedded, ImageRenderer};
