//! Cooperative locks compatible with the `proper-lockfile` locks Claude Code takes.
//!
//! A lock is a directory: `mkdir` either creates it or fails, atomically, on every platform. The holder keeps
//! touching its mtime so waiters can tell a live holder from one that crashed.
//!
//! A holder can lose its lock without noticing: if the machine sleeps longer than the stale timeout, another
//! process may take the lock over. Each guard therefore remembers the identity of the directory it created and
//! checks it before writes, before heartbeats and before removing the directory.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime};

use filetime::FileTime;

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy)]
pub struct LockTiming {
    /// A lock untouched for this long belongs to a dead process and may be taken over. Must match the value
    /// Claude Code uses for the same lock, or one side would steal the other's live lock.
    pub stale_after: Duration,
    /// How long to wait for a live holder before giving up.
    pub wait: Duration,
    /// How often the holder refreshes the mtime. Must be well under `stale_after`.
    pub heartbeat: Duration,
    pub poll: Duration,
}

impl LockTiming {
    /// Claude Code's credential refresh locks: stale after 60 s. Claude Code touches them every 5 s; ccam uses
    /// 3 s for margin. Waiting 9 s covers a normal refresh; it gives up long before a crashed holder's lock
    /// turns stale, so a command never hangs for a minute.
    pub const CREDENTIALS: Self = Self {
        stale_after: Duration::from_secs(60),
        wait: Duration::from_secs(9),
        heartbeat: Duration::from_secs(3),
        poll: Duration::from_millis(200),
    };

    /// Waits past the stale timeout. Used after ccam itself terminated Claude Code, which may have died
    /// holding the lock.
    pub const CREDENTIALS_AFTER_KILL: Self = Self { wait: Duration::from_secs(70), ..Self::CREDENTIALS };

    /// Claude Code's `.claude.json` write lock: stale after 10 s.
    pub const CONFIG: Self = Self {
        stale_after: Duration::from_secs(10),
        wait: Duration::from_secs(9),
        heartbeat: Duration::from_secs(3),
        poll: Duration::from_millis(200),
    };

    /// ccam's own vault lock. Held across network calls, so it waits longer than Claude Code's locks.
    pub const VAULT: Self = Self {
        stale_after: Duration::from_secs(60),
        wait: Duration::from_secs(30),
        heartbeat: Duration::from_secs(3),
        poll: Duration::from_millis(200),
    };
}

/// A heartbeat must land several times within the stale window, or a live lock could be taken over.
const fn heartbeat_fits(t: &LockTiming) -> bool {
    t.heartbeat.as_millis() * 3 <= t.stale_after.as_millis() && t.poll.as_millis() < t.wait.as_millis()
}
const _: () = assert!(heartbeat_fits(&LockTiming::CREDENTIALS));
const _: () = assert!(heartbeat_fits(&LockTiming::CREDENTIALS_AFTER_KILL));
const _: () = assert!(heartbeat_fits(&LockTiming::CONFIG));
const _: () = assert!(heartbeat_fits(&LockTiming::VAULT));
// After a kill the lock may be a dead process's; waiting must outlast the stale window to take it over.
const _: () = assert!(
    LockTiming::CREDENTIALS_AFTER_KILL.wait.as_millis() > LockTiming::CREDENTIALS_AFTER_KILL.stale_after.as_millis()
);

/// Identity of a lock directory, so a lock deleted and recreated under the same name is told apart: volume, file
/// number, and on Unix the birth time, where the filesystem records one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DirId(u64, u64, Option<u128>);

#[cfg(unix)]
fn dir_id(path: &Path) -> Option<DirId> {
    use std::os::unix::fs::MetadataExt;
    // ext4 and others hand a freed inode number straight to the next directory created, so a lock recreated by
    // another process often has the same (dev, ino). Its birth time differs. ctime cannot stand in where birth
    // time is missing: the heartbeat's mtime touch changes it.
    fs::metadata(path).ok().map(|m| {
        let born = m.created().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos());
        DirId(m.dev(), m.ino(), born)
    })
}

#[cfg(windows)]
fn dir_id(path: &Path) -> Option<DirId> {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, GetFileInformationByHandle, OPEN_EXISTING,
    };

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: `wide` is NUL-terminated; the handle is checked and always closed; the info struct is written by
    // the API before it is read. Creation time is not used as identity because Windows "tunnels" it onto a
    // directory recreated under the same name.
    unsafe {
        let handle = CreateFileW(
            wide.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        );
        if handle == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut info: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();
        let ok = GetFileInformationByHandle(handle, &mut info) != 0;
        CloseHandle(handle);
        ok.then(|| {
            DirId(
                u64::from(info.dwVolumeSerialNumber),
                (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
                None,
            )
        })
    }
}

#[derive(Debug)]
#[must_use = "the lock is released as soon as this guard is dropped"]
pub struct LockGuard {
    path: PathBuf,
    id: Option<DirId>,
    stop: Arc<AtomicBool>,
    heartbeat: Option<JoinHandle<()>>,
}

pub fn acquire(path: &Path, timing: LockTiming) -> Result<LockGuard> {
    let started = Instant::now();
    loop {
        match fs::create_dir(path) {
            Ok(()) => return Ok(LockGuard::start(path.to_path_buf(), timing.heartbeat)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                if let Some(stale) = stale_lock(path, timing.stale_after) {
                    // Re-check right before removing, so a lock another waiter just took over is left alone.
                    // proper-lockfile has the same small window; this keeps it as narrow as possible.
                    if stale_lock(path, timing.stale_after) == Some(stale) {
                        let _ = fs::remove_dir(path);
                    }
                    continue;
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
                }
                continue;
            }
            Err(e) => return Err(Error::io(path, e)),
        }
        if started.elapsed() >= timing.wait {
            return Err(Error::LockTimeout { path: path.to_path_buf(), waited_secs: timing.wait.as_secs() });
        }
        thread::sleep(timing.poll);
    }
}

/// The identity of the lock at `path` when it exists and has not been touched within `stale_after`.
fn stale_lock(path: &Path, stale_after: Duration) -> Option<DirId> {
    let modified = fs::metadata(path).and_then(|m| m.modified()).ok()?;
    let age = SystemTime::now().duration_since(modified).unwrap_or_default();
    if age > stale_after { dir_id(path) } else { None }
}

impl LockGuard {
    fn start(path: PathBuf, interval: Duration) -> Self {
        let id = dir_id(&path);
        let stop = Arc::new(AtomicBool::new(false));
        let heartbeat = {
            let (path, stop) = (path.clone(), Arc::clone(&stop));
            thread::spawn(move || {
                let tick = Duration::from_millis(50).min(interval);
                let mut since_touch = Duration::ZERO;
                while !stop.load(Ordering::Relaxed) {
                    thread::sleep(tick);
                    since_touch += tick;
                    if since_touch >= interval {
                        if id.is_none() || dir_id(&path) != id {
                            return;
                        }
                        let _ = filetime::set_file_mtime(&path, FileTime::now());
                        since_touch = Duration::ZERO;
                    }
                }
            })
        };
        Self { path, id, stop, heartbeat: Some(heartbeat) }
    }

    /// Fails when the lock was taken over since it was acquired. Call before every write it protects.
    pub fn ensure_held(&self) -> Result<()> {
        if self.id.is_some() && dir_id(&self.path) == self.id {
            Ok(())
        } else {
            Err(Error::LockLost(self.path.clone()))
        }
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.heartbeat.take() {
            let _ = handle.join();
        }
        if self.id.is_some() && dir_id(&self.path) == self.id {
            let _ = fs::remove_dir(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Generous staleness so a loaded test machine cannot make a live lock look abandoned.
    const FAST: LockTiming = LockTiming {
        stale_after: Duration::from_secs(5),
        wait: Duration::from_millis(300),
        heartbeat: Duration::from_millis(100),
        poll: Duration::from_millis(20),
    };

    #[test]
    fn a_held_lock_blocks_others_until_released() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.lock");

        let guard = acquire(&path, FAST).unwrap();
        let second = acquire(&path, FAST);
        assert!(matches!(second, Err(Error::LockTimeout { .. })), "{second:?}");

        drop(guard);
        assert!(!path.exists());
        let _reacquired = acquire(&path, FAST).unwrap();
    }

    #[test]
    fn heartbeat_keeps_a_long_held_lock_from_going_stale() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.lock");

        let timing = LockTiming { stale_after: Duration::from_millis(800), ..FAST };
        let _guard = acquire(&path, timing).unwrap();
        thread::sleep(Duration::from_millis(1200));
        assert!(matches!(acquire(&path, timing), Err(Error::LockTimeout { .. })));
    }

    #[test]
    fn a_crashed_holders_lock_is_taken_over_once_stale() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.lock");
        fs::create_dir(&path).unwrap();
        filetime::set_file_mtime(&path, FileTime::from_unix_time(0, 0)).unwrap();

        let _taken_over = acquire(&path, FAST).unwrap();
    }

    #[test]
    fn a_holder_notices_its_lock_was_taken_over_and_leaves_the_new_one_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.lock");
        let guard = acquire(&path, FAST).unwrap();
        guard.ensure_held().unwrap();

        // Another process decided the lock was stale (say, after this machine slept) and recreated it.
        fs::remove_dir(&path).unwrap();
        fs::create_dir(&path).unwrap();

        assert!(matches!(guard.ensure_held(), Err(Error::LockLost(_))));
        drop(guard);
        assert!(path.exists(), "the new holder's lock must survive");
    }

    #[test]
    fn missing_parent_directories_are_created() {
        let dir = tempfile::tempdir().unwrap();
        let _guard = acquire(&dir.path().join("a/b/x.lock"), FAST).unwrap();
    }
}
