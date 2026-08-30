//! Sandbox management: the [`SandboxBackend`] abstraction, transports, the
//! Phase 1 mock backend and the platform stubs for future real backends.
//!
//! Trust boundary: everything in this crate is **trusted host code**. It
//! manages untrusted worker processes but never parses document content
//! itself. The only interaction with untrusted code is the binary protocol
//! (`docbunker-protocol`), and every message is validated on the way in and out.

pub mod backend;
pub mod config;
pub mod error;
pub mod mock;
pub mod platforms;
pub mod process;
pub mod process_backend;
pub mod runsc_bundle;
pub mod session;
pub mod transport;
pub mod version;

pub use backend::SandboxBackend;
pub use config::SandboxConfig;
pub use error::SandboxError;
pub use mock::MockBackend;
pub use process::ProcessTransport;
pub use process_backend::SubprocessBackend;
pub use session::{DocumentId, DocumentInput, SandboxKind, SandboxSession};
pub use transport::{InProcessTransport, NullTransport, SandboxTransport};
