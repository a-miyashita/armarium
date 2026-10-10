//! File system helpers: atomic replacement and fingerprints.

use crate::error::{CoreError, Result};
use std::fs::{self, File};
use std::io::{BufWriter, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Writes a file through a temporary file in the same directory, and then replaces
/// the target file. If the write fails, the target file does not change.
pub fn atomic_write(
    path: &Path,
    write: impl FnOnce(&mut BufWriter<File>) -> Result<()>,
) -> Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path.file_name().ok_or_else(|| {
        CoreError::InvalidArgument(format!("not a file path: {}", path.display()))
    })?;
    let tmp = dir.join(format!(
        ".{}.armarium-{}-{}.tmp",
        name.to_string_lossy(),
        std::process::id(),
        uuid::Uuid::now_v7().simple()
    ));
    let result = (|| -> Result<()> {
        let file = File::create(&tmp).map_err(|e| CoreError::from_io(e, &tmp))?;
        let mut w = BufWriter::with_capacity(1 << 20, file);
        write(&mut w)?;
        let file = w.into_inner().map_err(|e| CoreError::Io(e.into_error()))?;
        file.sync_all()?;
        drop(file);
        replace_file(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(windows)]
fn replace_file(tmp: &Path, target: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{ReplaceFileW, REPLACEFILE_IGNORE_MERGE_ERRORS};
    if !target.exists() {
        return fs::rename(tmp, target).map_err(|e| CoreError::from_io(e, target));
    }
    let wide = |p: &Path| -> Vec<u16> { p.as_os_str().encode_wide().chain(Some(0)).collect() };
    let t = wide(target);
    let s = wide(tmp);
    // ReplaceFileW keeps the attributes and the ACL of the original file.
    let ok = unsafe {
        ReplaceFileW(
            t.as_ptr(),
            s.as_ptr(),
            std::ptr::null(),
            REPLACEFILE_IGNORE_MERGE_ERRORS,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if ok == 0 {
        let err = std::io::Error::last_os_error();
        return Err(CoreError::from_io(err, target));
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(tmp: &Path, target: &Path) -> Result<()> {
    if let Ok(meta) = fs::metadata(target) {
        let _ = fs::set_permissions(tmp, meta.permissions());
    }
    fs::rename(tmp, target).map_err(|e| CoreError::from_io(e, target))
}

/// A fingerprint of a file: size, time of the last change, and a hash of the
/// first and last 64 KB (06. Editing, section 5.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint {
    pub size: u64,
    pub modified_ns: i128,
    pub hash: u64,
}

const EDGE: u64 = 64 * 1024;

impl Fingerprint {
    pub fn of(path: &Path) -> Result<Fingerprint> {
        let mut file = File::open(path).map_err(|e| CoreError::from_io(e, path))?;
        let meta = file.metadata()?;
        let size = meta.len();
        let modified_ns = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as i128)
            .unwrap_or(0);
        let mut hasher = xxhash_rust::xxh3::Xxh3::new();
        let mut buf = vec![0u8; EDGE as usize];
        let n = read_up_to(&mut file, &mut buf)?;
        hasher.update(&buf[..n]);
        if size > EDGE {
            let start = size.saturating_sub(EDGE).max(EDGE);
            file.seek(SeekFrom::Start(start))?;
            let n = read_up_to(&mut file, &mut buf)?;
            hasher.update(&buf[..n]);
        }
        Ok(Fingerprint {
            size,
            modified_ns,
            hash: hasher.digest(),
        })
    }

    /// A fast check with the metadata only.
    pub fn metadata_differs(&self, path: &Path) -> bool {
        match fs::metadata(path) {
            Ok(meta) => {
                let m = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_nanos() as i128)
                    .unwrap_or(0);
                meta.len() != self.size || m != self.modified_ns
            }
            Err(_) => true,
        }
    }
}

fn read_up_to(file: &mut File, buf: &mut [u8]) -> Result<usize> {
    let mut total = 0;
    while total < buf.len() {
        let n = file.read(&mut buf[total..])?;
        if n == 0 {
            break;
        }
        total += n;
    }
    Ok(total)
}

/// Makes an absolute path without resolving links. Keeps the letter case.
pub fn absolute(path: &Path) -> Result<PathBuf> {
    let p = std::path::absolute(path)?;
    Ok(strip_verbatim(p))
}

/// Removes the `\\?\` prefix for display and storage.
fn strip_verbatim(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        p
    }
}

/// Compares two paths as the file system does (not case-sensitive on Windows).
pub fn same_path(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a.to_lowercase() == b.to_lowercase()
    } else {
        a == b
    }
}

/// The default directory for application data (`%APPDATA%\Armarium`).
pub fn config_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("ARMARIUM_CONFIG_DIR") {
        return PathBuf::from(d);
    }
    #[cfg(windows)]
    if let Some(d) = std::env::var_os("APPDATA") {
        return PathBuf::from(d).join("Armarium");
    }
    if let Some(d) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(d).join("armarium");
    }
    if let Some(h) = std::env::var_os("HOME") {
        return PathBuf::from(h).join(".config").join("armarium");
    }
    std::env::temp_dir().join("armarium")
}

/// The default directory for local data (`%LOCALAPPDATA%\Armarium`).
pub fn local_data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("ARMARIUM_DATA_DIR") {
        return PathBuf::from(d);
    }
    #[cfg(windows)]
    if let Some(d) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(d).join("Armarium");
    }
    if let Some(d) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(d).join("armarium");
    }
    if let Some(h) = std::env::var_os("HOME") {
        return PathBuf::from(h)
            .join(".local")
            .join("share")
            .join("armarium");
    }
    std::env::temp_dir().join("armarium")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn writes_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        fs::write(&p, "old").unwrap();
        atomic_write(&p, |w| Ok(w.write_all(b"new")?)).unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "new");
        let r = atomic_write(&p, |w| {
            w.write_all(b"partial")?;
            Err(CoreError::Internal("fail".into()))
        });
        assert!(r.is_err());
        assert_eq!(fs::read_to_string(&p).unwrap(), "new");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn fingerprint_changes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        fs::write(&p, "abc").unwrap();
        let a = Fingerprint::of(&p).unwrap();
        assert_eq!(a, Fingerprint::of(&p).unwrap());
        fs::write(&p, "abd").unwrap();
        let b = Fingerprint::of(&p).unwrap();
        assert_ne!(a.hash, b.hash);
    }
}
