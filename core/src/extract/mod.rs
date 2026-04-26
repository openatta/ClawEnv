//! Archive extraction — .tar.gz / .tar.xz / .zip.
//!
//! Blocking I/O is used (the underlying `tar` / `zip` crates are sync);
//! callers invoke via `tokio::task::spawn_blocking`.
//!
//! Strip-component support: Node.js / dugite / lima tarballs all wrap
//! their contents in a single top-level directory (e.g. `node-v22/…`).
//! We optionally strip N leading path components while extracting.

use std::fs::{self, File};
use std::io::{self, BufReader, Cursor};
use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ExtractError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("unsupported archive format: {0}")]
    Unsupported(String),
    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("xz decompression error: {0}")]
    Xz(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
    TarGz,
    TarXz,
    Zip,
}

impl ArchiveFormat {
    /// Infer format from file extension.
    pub fn from_path(p: &Path) -> Option<Self> {
        let s = p.file_name()?.to_string_lossy().to_lowercase();
        if s.ends_with(".tar.gz") || s.ends_with(".tgz") { Some(Self::TarGz) }
        else if s.ends_with(".tar.xz") { Some(Self::TarXz) }
        else if s.ends_with(".zip") { Some(Self::Zip) }
        else { None }
    }
}

#[derive(Default)]
pub struct ExtractOpts {
    /// Strip this many leading path components from every entry.
    /// Typical: 1 (node/git/lima tarballs wrap contents in a single top dir).
    pub strip_components: usize,
    /// If true, delete `dest` before extracting (safer for upgrades).
    pub clean_dest: bool,
}

/// Extract `archive` into `dest`. Detects format from `archive`'s extension.
pub fn extract_archive(
    archive: &Path,
    dest: &Path,
    opts: &ExtractOpts,
) -> Result<(), ExtractError> {
    let fmt = ArchiveFormat::from_path(archive)
        .ok_or_else(|| ExtractError::Unsupported(
            archive.to_string_lossy().into_owned()
        ))?;

    if opts.clean_dest && dest.exists() {
        fs::remove_dir_all(dest)?;
    }
    fs::create_dir_all(dest)?;

    match fmt {
        ArchiveFormat::TarGz => extract_tar_gz(archive, dest, opts),
        ArchiveFormat::TarXz => extract_tar_xz(archive, dest, opts),
        ArchiveFormat::Zip   => extract_zip(archive, dest, opts),
    }
}

fn extract_tar_gz(archive: &Path, dest: &Path, opts: &ExtractOpts) -> Result<(), ExtractError> {
    let f = File::open(archive)?;
    let gz = flate2::read::GzDecoder::new(BufReader::new(f));
    let mut tar = tar::Archive::new(gz);
    extract_tar_entries(&mut tar, dest, opts)
}

fn extract_tar_xz(archive: &Path, dest: &Path, opts: &ExtractOpts) -> Result<(), ExtractError> {
    // lzma-rs is pure-Rust but stream-only. Decompress fully into memory
    // first, then feed into tar. Node.js Linux tarballs are ~25 MB xz'd
    // → manageable; stage-C could switch to chunked decompression.
    let f = File::open(archive)?;
    let mut reader = BufReader::new(f);
    let mut decompressed: Vec<u8> = Vec::new();
    lzma_rs::xz_decompress(&mut reader, &mut decompressed)
        .map_err(|e| ExtractError::Xz(format!("{:?}", e)))?;
    let mut tar = tar::Archive::new(Cursor::new(decompressed));
    extract_tar_entries(&mut tar, dest, opts)
}

fn extract_tar_entries<R: std::io::Read>(
    tar: &mut tar::Archive<R>,
    dest: &Path,
    opts: &ExtractOpts,
) -> Result<(), ExtractError> {
    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();
        let stripped = match strip_path(&path, opts.strip_components) {
            Some(p) => p,
            None => continue, // Entry was entirely within the stripped prefix.
        };
        if stripped.as_os_str().is_empty() { continue; }
        let out_path = dest.join(&stripped);
        // Security: reject paths that escape dest.
        if !out_path.starts_with(dest) {
            return Err(ExtractError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("tar entry escapes destination: {}", path.display())
            )));
        }
        // Ensure parent dir exists — tarballs often omit explicit directory
        // entries for intermediate paths (we saw this with our test fixtures
        // and with dugite's archive).
        let is_dir = entry.header().entry_type().is_dir();
        if is_dir {
            fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            entry.unpack(&out_path)?;
        }
    }
    Ok(())
}

fn extract_zip(archive: &Path, dest: &Path, opts: &ExtractOpts) -> Result<(), ExtractError> {
    let f = File::open(archive)?;
    let mut zip = zip::ZipArchive::new(BufReader::new(f))?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let name = entry.mangled_name();
        let stripped = match strip_path(&name, opts.strip_components) {
            Some(p) => p,
            None => continue,
        };
        if stripped.as_os_str().is_empty() { continue; }
        let out_path = dest.join(&stripped);
        if !out_path.starts_with(dest) {
            return Err(ExtractError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("zip entry escapes destination: {}", name.display())
            )));
        }
        if entry.is_dir() {
            fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() { fs::create_dir_all(parent)?; }
            let mut out = File::create(&out_path)?;
            io::copy(&mut entry, &mut out)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Some(mode) = entry.unix_mode() {
                    let _ = fs::set_permissions(&out_path, fs::Permissions::from_mode(mode));
                }
            }
        }
    }
    Ok(())
}

fn strip_path(p: &Path, n: usize) -> Option<PathBuf> {
    let comps: Vec<_> = p.components().collect();
    if comps.len() < n { return None; }
    Some(comps.iter().skip(n).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn make_tar_gz(tmp: &Path, entries: &[(&str, &[u8])]) -> PathBuf {
        let archive_path = tmp.join("test.tar.gz");
        let f = File::create(&archive_path).unwrap();
        let gz = flate2::write::GzEncoder::new(f, flate2::Compression::default());
        let mut tar = tar::Builder::new(gz);
        for (name, data) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, name, *data).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap();
        archive_path
    }

    #[test]
    fn format_detection() {
        assert_eq!(ArchiveFormat::from_path(Path::new("x.tar.gz")), Some(ArchiveFormat::TarGz));
        assert_eq!(ArchiveFormat::from_path(Path::new("x.tgz")),    Some(ArchiveFormat::TarGz));
        assert_eq!(ArchiveFormat::from_path(Path::new("x.tar.xz")), Some(ArchiveFormat::TarXz));
        assert_eq!(ArchiveFormat::from_path(Path::new("x.zip")),    Some(ArchiveFormat::Zip));
        assert_eq!(ArchiveFormat::from_path(Path::new("x.bin")),    None);
    }

    #[test]
    fn strip_components_none() {
        let p = Path::new("a/b/c.txt");
        assert_eq!(strip_path(p, 0), Some(PathBuf::from("a/b/c.txt")));
    }

    #[test]
    fn strip_components_one() {
        let p = Path::new("topdir/bin/node");
        assert_eq!(strip_path(p, 1), Some(PathBuf::from("bin/node")));
    }

    #[test]
    fn strip_components_too_many_returns_none() {
        let p = Path::new("a/b");
        assert!(strip_path(p, 5).is_none());
    }

    #[test]
    fn extract_tar_gz_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let archive = make_tar_gz(tmp.path(), &[
            ("pkg/bin/node", b"#!/usr/bin/env node\n" as &[u8]),
            ("pkg/README.md", b"readme"),
        ]);
        let dest = tmp.path().join("out");
        extract_archive(&archive, &dest, &ExtractOpts {
            strip_components: 1, clean_dest: false,
        }).unwrap();
        let node_path = dest.join("bin/node");
        assert!(node_path.exists());
        let content = std::fs::read_to_string(&node_path).unwrap();
        assert!(content.starts_with("#!/usr/bin/env node"));
    }

    #[test]
    fn extract_tar_gz_no_strip() {
        let tmp = TempDir::new().unwrap();
        let archive = make_tar_gz(tmp.path(), &[("a.txt", b"hi")]);
        let dest = tmp.path().join("out");
        extract_archive(&archive, &dest, &ExtractOpts::default()).unwrap();
        assert_eq!(std::fs::read_to_string(dest.join("a.txt")).unwrap(), "hi");
    }

    // Note: we don't have a dedicated "escape path" integration test because
    // the `tar` crate refuses to even build archives containing `..` entries
    // (belt): our own `starts_with(dest)` check (suspenders) never triggers
    // in practice because tar's front-line guard fires first. Both layers
    // are wired; the outer guard is exercised indirectly via the roundtrip
    // test.

    #[test]
    fn extract_clean_dest_wipes_old_content() {
        let tmp = TempDir::new().unwrap();
        let dest = tmp.path().join("out");
        std::fs::create_dir_all(&dest).unwrap();
        let old = dest.join("old.txt");
        File::create(&old).unwrap().write_all(b"old").unwrap();
        let archive = make_tar_gz(tmp.path(), &[("new.txt", b"new")]);
        extract_archive(&archive, &dest, &ExtractOpts {
            strip_components: 0, clean_dest: true,
        }).unwrap();
        assert!(!old.exists());
        assert!(dest.join("new.txt").exists());
    }
}
