//! JSON file reads and crash-safe writes.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use serde_json::{Map, Value};

use crate::error::{Error, Result};

#[derive(Clone, Copy)]
pub enum Style {
    Compact,
    Pretty,
}

/// Windows refuses to open or replace a file while another process holds it without FILE_SHARE_DELETE, which
/// Claude Code and concurrent ccam processes briefly do. 40 x 25 ms rides out those windows.
const SHARING_RETRIES: u32 = 40;
const SHARING_BACKOFF: Duration = Duration::from_millis(25);

/// Reads a JSON file, returning `None` when it does not exist.
pub fn read(path: &Path) -> Result<Option<Value>> {
    match with_sharing_retry(|| fs::read(path)) {
        Ok(bytes) => {
            // PowerShell 5.1 writes UTF-8 with a byte order mark.
            let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
            serde_json::from_slice(bytes).map(Some).map_err(|source| Error::Json { path: path.into(), source })
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::io(path, e)),
    }
}

/// Writes JSON through a uniquely named temp file and a rename, so readers see the old or the new file, never
/// a torn one, even with several writers.
///
/// A new file is created owner-only on Unix when `private` is set. An existing file keeps its permissions.
pub fn write_atomic(path: &Path, value: &Value, style: Style, private: bool) -> Result<()> {
    let mut bytes = match style {
        Style::Compact => serde_json::to_vec(value),
        Style::Pretty => serde_json::to_vec_pretty(value),
    }
    .map_err(|source| Error::Json { path: path.into(), source })?;
    if matches!(style, Style::Pretty) {
        bytes.push(b'\n');
    }

    let tmp = temp_path(path);
    let result = write_synced(&tmp, &bytes, path, private)
        .map_err(|e| Error::io(&tmp, e))
        .and_then(|()| with_sharing_retry(|| fs::rename(&tmp, path)).map_err(|e| Error::io(path, e)));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Applies `change` to the top-level object of a JSON file, creating the file when missing. Returns whether the
/// content changed; an unchanged file is not rewritten.
pub fn edit_object(
    path: &Path,
    style: Style,
    private: bool,
    change: impl FnOnce(&mut Map<String, Value>) -> Result<()>,
) -> Result<bool> {
    let original = read(path)?;
    let mut map = match &original {
        Some(Value::Object(map)) => map.clone(),
        None => Map::new(),
        Some(_) => return Err(Error::Malformed { path: path.into(), reason: "top level is not a JSON object" }),
    };
    change(&mut map)?;
    let updated = Value::Object(map);
    let unchanged = match &original {
        Some(original) => *original == updated,
        None => updated == Value::Object(Map::new()),
    };
    if unchanged {
        return Ok(false);
    }
    write_atomic(path, &updated, style, private)?;
    Ok(true)
}

fn temp_path(path: &Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let unique = format!("{}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed));
    path.with_file_name(format!(".{file_name}.{unique}.ccam-tmp"))
}

fn write_synced(tmp: &Path, bytes: &[u8], target: &Path, private: bool) -> io::Result<()> {
    if let Some(parent) = tmp.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Owner-only from the first byte for new private files; existing files get their mode copied below.
        options.mode(if private { 0o600 } else { 0o644 });
    }
    let mut file = options.open(tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    #[cfg(unix)]
    {
        // Copy the exact mode (not umask-filtered) so a 0600 file such as .claude.json never widens.
        use std::os::unix::fs::PermissionsExt;
        if let Ok(existing) = fs::metadata(target) {
            let copied = fs::set_permissions(tmp, fs::Permissions::from_mode(existing.permissions().mode() & 0o777));
            // A file this process just created can always be chmod-ed on a Linux filesystem. EPERM only comes from
            // a Windows drive seen from WSL (DrvFs mounted without the user's uid), which stores no Linux modes, so
            // refusing the write there would lose a login over a permission that does not exist.
            match copied {
                Err(e) if e.raw_os_error() == Some(libc::EPERM) => {}
                other => other?,
            }
        }
    }
    #[cfg(not(unix))]
    let _ = (target, private);
    Ok(())
}

fn with_sharing_retry<T>(mut op: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let mut attempt = 0;
    loop {
        match op() {
            // From WSL, a Windows process holding a file in the shared vault shows up as the same error.
            Err(e)
                if e.kind() == io::ErrorKind::PermissionDenied
                    && (cfg!(windows) || crate::presence::running_in_wsl())
                    && attempt < SHARING_RETRIES =>
            {
                attempt += 1;
                thread::sleep(SHARING_BACKOFF);
            }
            other => return other,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use super::*;

    #[test]
    fn concurrent_writers_never_expose_a_torn_or_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = Arc::new(dir.path().join("login.json"));
        write_atomic(&path, &json!({"n": 0}), Style::Pretty, true).unwrap();

        let writers: Vec<_> = (0..8)
            .map(|i| {
                let path = Arc::clone(&path);
                thread::spawn(move || {
                    for j in 0..40 {
                        write_atomic(&path, &json!({"n": i * 100 + j, "pad": "x".repeat(2000)}), Style::Pretty, true)
                            .unwrap();
                    }
                })
            })
            .collect();
        for _ in 0..400 {
            assert!(read(&path).unwrap().is_some_and(|v| v["n"].is_number()));
        }
        for w in writers {
            w.join().unwrap();
        }
        let leftovers = fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(leftovers, 1, "temp files must not be left behind");
    }

    #[cfg(unix)]
    #[test]
    fn rewriting_keeps_the_original_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".claude.json");
        fs::write(&path, "{}").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

        write_atomic(&path, &json!({"a": 1}), Style::Pretty, false).unwrap();

        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn reads_files_with_a_byte_order_mark() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bom.json");
        fs::write(&path, b"\xEF\xBB\xBF{\"a\":1}").unwrap();
        assert_eq!(read(&path).unwrap(), Some(json!({"a": 1})));
    }

    #[test]
    fn edit_object_rejects_non_objects_and_skips_no_op_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.json");
        fs::write(&path, "[1]").unwrap();
        assert!(matches!(edit_object(&path, Style::Pretty, false, |_| Ok(())), Err(Error::Malformed { .. })));

        fs::write(&path, r#"{"a":1}"#).unwrap();
        assert!(!edit_object(&path, Style::Pretty, false, |_| Ok(())).unwrap());
        assert_eq!(fs::read_to_string(&path).unwrap(), r#"{"a":1}"#);
    }
}
