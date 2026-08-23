//! Bounded host-side file reading.
//!
//! Only the trusted Tauri layer calls this, right after the user picks a file
//! through the native dialog. The opened handle is read through a hard bound,
//! so a growing or replaced file cannot cause an oversized allocation.

use std::io::Read;
use std::path::Path;

use crate::error::DocBunkerError;

/// Read a document file, enforcing `max_size` on one opened file handle.
pub fn read_document_file(path: &Path, max_size: usize) -> Result<Vec<u8>, DocBunkerError> {
    let file = std::fs::File::open(path).map_err(DocBunkerError::from)?;
    let metadata = file.metadata().map_err(DocBunkerError::from)?;
    if !metadata.is_file() {
        return Err(DocBunkerError::InvalidDocument);
    }
    if metadata.len() > max_size as u64 {
        return Err(DocBunkerError::DocumentTooLarge);
    }
    let read_limit = max_size
        .checked_add(1)
        .ok_or(DocBunkerError::ResourceLimitExceeded)?;
    let mut data = Vec::with_capacity((metadata.len() as usize).min(max_size));
    file.take(read_limit as u64)
        .read_to_end(&mut data)
        .map_err(DocBunkerError::from)?;
    if data.len() > max_size {
        return Err(DocBunkerError::DocumentTooLarge);
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn reads_file_within_limit() {
        let dir = std::env::temp_dir();
        let mut path = dir.join("docbunker-io-test.tmp");
        path.set_extension(format!("{}", std::process::id()));
        {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(b"hello").unwrap();
        }
        let data = read_document_file(&path, 1024).unwrap();
        assert_eq!(data, b"hello");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn rejects_file_over_limit() {
        let dir = std::env::temp_dir();
        let mut path = dir.join("docbunker-io-test-big.tmp");
        path.set_extension(format!("{}", std::process::id()));
        {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(&[0u8; 2048]).unwrap();
        }
        let result = read_document_file(&path, 1024);
        assert!(matches!(result, Err(DocBunkerError::DocumentTooLarge)));
        std::fs::remove_file(&path).ok();
    }
}
