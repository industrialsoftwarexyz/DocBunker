//! Deterministic newc (cpio) packer for the guest initramfs.
//!
//! The kernel requires `/init` (and the guest binaries) to be executable in
//! the archive; host tools like bsdtar on Windows cannot express that, and
//! reproducible builds need fixed metadata anyway. So packing happens here:
//! every directory is written 0755, every regular file 0755, uid/gid 0,
//! mtime 0, in sequential order. Symlinks are not supported and fail loudly.
//!
//! Usage: `pack-initramfs <input-dir> <output.cpio>`

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const NEWC_MAGIC: &[u8; 6] = b"070701";

struct Entry {
    name: String,
    mode: u32,
    data: Vec<u8>,
}

fn collect(dir: &Path, base: &Path, out: &mut Vec<Entry>) -> Result<(), String> {
    let mut entries = fs::read_dir(dir)
        .map_err(|e| format!("read_dir {}: {e}", dir.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    entries.sort_by_key(|e| e.file_name());

    let rel = if base == dir {
        PathBuf::new()
    } else {
        dir.strip_prefix(base)
            .map_err(|e| e.to_string())?
            .to_path_buf()
    };

    out.push(Entry {
        name: rel.to_string_lossy().replace('\\', "/"),
        mode: 0o040755,
        data: Vec::new(),
    });

    for entry in entries {
        let path = entry.path();
        let child_rel = rel.join(entry.file_name());
        let file_type = entry.file_type().map_err(|e| e.to_string())?;
        if file_type.is_dir() {
            collect(&path, base, out)?;
        } else if file_type.is_file() {
            let data = fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
            out.push(Entry {
                name: child_rel.to_string_lossy().replace('\\', "/"),
                mode: 0o100755,
                data,
            });
        } else {
            return Err(format!(
                "unsupported entry type (symlink/device) in initramfs: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn hex8(value: u32) -> Vec<u8> {
    format!("{value:08X}").into_bytes()
}

/// Kernel `N_ALIGN` for the name field: `((len + 1) & ~3) + 2`.
///
/// The Linux initramfs unpacker (`init/initramfs.c`) lays out each record as
/// `110 + N_ALIGN(namesize) + ALIGN(filesize, 4)` — the name is *not* simply
/// padded to 4. See `N_ALIGN` in the kernel source.
fn n_align(len: usize) -> usize {
    ((len + 1) & !3) + 2
}

fn write_entry(
    w: &mut impl Write,
    ino: u32,
    name: &str,
    mode: u32,
    data: &[u8],
) -> Result<(), String> {
    let mut header = Vec::with_capacity(110);
    header.extend_from_slice(NEWC_MAGIC);
    header.extend_from_slice(&hex8(ino));
    header.extend_from_slice(&hex8(mode));
    header.extend_from_slice(&hex8(0)); // uid
    header.extend_from_slice(&hex8(0)); // gid
    header.extend_from_slice(&hex8(1)); // nlink
    header.extend_from_slice(&hex8(0)); // mtime
    header.extend_from_slice(&hex8(data.len() as u32));
    header.extend_from_slice(&hex8(0)); // devmajor
    header.extend_from_slice(&hex8(0)); // devminor
    header.extend_from_slice(&hex8(0)); // rdevmajor
    header.extend_from_slice(&hex8(0)); // rdevminor
    header.extend_from_slice(&hex8((name.len() + 1) as u32)); // namesize incl. NUL
    header.extend_from_slice(&hex8(0)); // check
    assert_eq!(header.len(), 110);

    w.write_all(&header).map_err(|e| e.to_string())?;
    let mut name_bytes = name.as_bytes().to_vec();
    name_bytes.push(0);
    let name_len = name_bytes.len();
    name_bytes.resize(n_align(name_len), 0);
    w.write_all(&name_bytes).map_err(|e| e.to_string())?;
    write_padded(w, data)?;
    Ok(())
}

fn write_padded(w: &mut impl Write, data: &[u8]) -> Result<(), String> {
    w.write_all(data).map_err(|e| e.to_string())?;
    let pad = (4 - (data.len() % 4)) % 4;
    if pad > 0 {
        w.write_all(&[0; 4][..pad]).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let input = args
        .next()
        .ok_or("usage: pack-initramfs <input-dir> <output.cpio>")?;
    let output = args
        .next()
        .ok_or("usage: pack-initramfs <input-dir> <output.cpio>")?;

    let input = PathBuf::from(input);
    let output = PathBuf::from(output);
    if !input.is_dir() {
        return Err(format!("input is not a directory: {}", input.display()));
    }

    let mut entries = Vec::new();
    collect(&input, &input, &mut entries)?;

    let file =
        fs::File::create(&output).map_err(|e| format!("create {}: {e}", output.display()))?;
    let mut w = std::io::BufWriter::new(file);

    let mut ino: u32 = 0;
    for entry in &entries {
        ino += 1;
        let name = if entry.name.is_empty() {
            "."
        } else {
            &entry.name
        };
        write_entry(&mut w, ino, name, entry.mode, &entry.data)?;
    }
    ino += 1;
    write_entry(&mut w, ino, "TRAILER!!!", 0, &[])?;

    w.flush().map_err(|e| e.to_string())?;
    println!("packed {} entries -> {}", entries.len(), output.display());
    Ok(())
}
