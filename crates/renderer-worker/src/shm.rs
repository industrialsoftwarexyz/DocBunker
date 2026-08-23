//! Shared-memory page-buffer store (ADR-009).
//!
//! The renderer worker publishes one named shared-memory region per handshake
//! when the host asks for it. The region holds **one page buffer** (host and
//! worker strictly alternate), sized to the absolute pixel-buffer cap.
//!
//! Implementation: a per-user temporary file (`docbunker_pagebuf_*`) written
//! at a fixed offset through the **safe** `std::os::*::FileExt` APIs
//! (`seek_write`/`seek_read` — no `unsafe`, satisfying the workspace
//! `unsafe_code = "forbid"` lint). The same primitive works on Windows and
//! POSIX hosts; shared `/dev/shm` segments would not be reachable from inside
//! `runsc`/QEMU anyway, and those backends never advertise shm (dev-only
//! optimization). Creating the region is best-effort: on failure the worker
//! falls back to in-frame bytes and the protocol negotiates that
//! automatically.

use std::fs::File;
use std::path::PathBuf;
use std::sync::atomic::{fence, AtomicU64, Ordering};

use docbunker_renderer_api::limits;

const REGION_PREFIX: &str = "docbunker_pagebuf_";
const REGION_EXTENSION: &str = ".bin";

fn next_region_id() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Positional write at `offset`, writing the whole buffer (unix `pwrite`).
/// POSIX allows short writes, which would leave stale bytes in the region the
/// host then reads as a page.
#[cfg(unix)]
fn write_at(file: &File, buffer: &[u8], offset: u64) -> std::io::Result<()> {
    use std::os::unix::fs::FileExt;
    let mut done = 0;
    while done < buffer.len() {
        let n = file.write_at(&buffer[done..], offset + done as u64)?;
        if n == 0 {
            return Err(std::io::ErrorKind::WriteZero.into());
        }
        done += n;
    }
    Ok(())
}

/// Positional write at `offset` (Windows `WriteFile` with offset).
#[cfg(windows)]
fn write_at(file: &File, buffer: &[u8], offset: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    let mut done = 0;
    while done < buffer.len() {
        let n = file.seek_write(&buffer[done..], offset + done as u64)?;
        if n == 0 {
            return Err(std::io::ErrorKind::WriteZero.into());
        }
        done += n;
    }
    Ok(())
}

/// Positional read at `offset`, filling `buffer` exactly (test helper).
#[cfg(all(unix, test))]
fn read_exact_at(file: &File, buffer: &mut [u8], offset: u64) -> std::io::Result<()> {
    use std::os::unix::fs::FileExt;
    let mut done = 0;
    while done < buffer.len() {
        let n = file.read_at(&mut buffer[done..], offset + done as u64)?;
        if n == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        done += n;
    }
    Ok(())
}

/// Positional read at `offset`, filling `buffer` exactly (test helper).
#[cfg(all(windows, test))]
fn read_exact_at(file: &File, buffer: &mut [u8], offset: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    let mut done = 0;
    while done < buffer.len() {
        let n = file.seek_read(&mut buffer[done..], offset + done as u64)?;
        if n == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        done += n;
    }
    Ok(())
}

/// A sink for rendered page buffers, shared with the trusted host.
pub trait PageBufferStore: Send {
    /// Copy a page into the shared region and publish it (release fence).
    fn store(&mut self, bytes: &[u8]) -> Result<(), String>;
    /// Base name of the region file, as advertised in `HelloOk`.
    fn name(&self) -> String;
    /// Region capacity in bytes (≤ `MAX_PIXEL_BUFFER`).
    fn capacity(&self) -> u64;
}

/// Real shared-memory store backed by a temp file.
pub struct SharedMemStore {
    file: File,
    name: String,
    path: PathBuf,
    capacity: u64,
}

impl SharedMemStore {
    /// Try to create a fresh region. Returns `None` when the OS cannot.
    pub fn try_create() -> Option<Self> {
        let capacity = limits::MAX_PIXEL_BUFFER as u64;
        for attempt in 0..3 {
            let name = format!(
                "{REGION_PREFIX}{}_{}{REGION_EXTENSION}",
                std::process::id(),
                next_region_id()
            );
            let path = std::env::temp_dir().join(&name);
            // Private to the creating user: page rasters may contain the
            // document's content and live in the shared system temp dir.
            #[cfg(unix)]
            let opened = {
                use std::os::unix::fs::OpenOptionsExt;
                let mut options = std::fs::OpenOptions::new();
                options.read(true).write(true).create_new(true).mode(0o600);
                options.open(&path)
            };
            #[cfg(not(unix))]
            let opened = {
                use std::fs::OpenOptions;
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .open(&path)
            };
            let file = opened;
            let Ok(file) = file else {
                tracing::warn!(%name, "shm create attempt {attempt} failed");
                continue;
            };
            if file.set_len(capacity).is_err() {
                let _ = std::fs::remove_file(&path);
                continue;
            }
            tracing::info!(%name, capacity, "worker: page buffer shared region created");
            return Some(Self {
                file,
                name,
                path,
                capacity,
            });
        }
        None
    }
}

impl PageBufferStore for SharedMemStore {
    fn store(&mut self, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() > self.capacity as usize {
            return Err("page does not fit the shared region".into());
        }
        write_at(&self.file, bytes, 0).map_err(|error| error.to_string())?;
        // The subsequent frame write on the pipe orders this store (ADR-009).
        fence(Ordering::SeqCst);
        Ok(())
    }

    fn name(&self) -> String {
        self.name.clone()
    }

    fn capacity(&self) -> u64 {
        self.capacity
    }
}

impl Drop for SharedMemStore {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_round_trips_within_cap() {
        let mut store = SharedMemStore::try_create().expect("shm available in test env");
        let payload = vec![0xAB; 4096];
        store.store(&payload).unwrap();
        let mut read_back = vec![0u8; payload.len()];
        read_exact_at(&store.file, &mut read_back, 0).unwrap();
        assert_eq!(read_back, payload);
        assert!(store.capacity() >= 4096);
        assert!(!store.name().is_empty());
    }

    #[test]
    fn store_rejects_oversized_payload() {
        let mut store = SharedMemStore::try_create().expect("shm available in test env");
        let oversized = vec![0u8; limits::MAX_PIXEL_BUFFER + 1];
        assert!(store.store(&oversized).is_err());
    }

    #[test]
    fn file_is_removed_on_drop() {
        let store = SharedMemStore::try_create().expect("shm available in test env");
        let path = store.path.clone();
        assert!(path.is_file());
        drop(store);
        assert!(!path.exists(), "region file must be removed on drop");
    }
}
