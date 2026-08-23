//! Cross-crate integration tests for DocBunker.
//!
//! This crate exists so the root `tests/` directory is a real, buildable unit
//! that exercises the crates *together* (core + sandbox + protocol + worker)
//! and ships a deliberately misbehaving `fake_worker` binary used to verify
//! host-side protocol validation.
//!
//! See `tests/README.md` for the test strategy.
