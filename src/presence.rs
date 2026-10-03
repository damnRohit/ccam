//! Which account each Claude Code install sharing this vault is logged in as.
//!
//! Windows and WSL can share one vault (WSL's `~/.claude/ccam` linked to the Windows one). Each has its own live
//! login, and a login's refresh token works once: if both sides run the same account from one saved copy, the
//! first renewal kills the other side's login. Every side therefore records its live account in
//! `<vault>/.live/<side>.json`, and a switch refuses an account another side is using. A record also says how to
//! run that side's ccam, so `ccam to --force` can move the other side off the account first.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::jsonio::{self, Style};
use crate::login::Identity;
use crate::paths::Paths;

const DIR: &str = ".live";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Platform {
    Windows,
    Wsl { distro: String },
    Other,
}

/// One Claude Code install: a platform plus its config home.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Side {
    pub platform: Platform,
    /// This side's ccam, so another side can run it.
    pub exe: PathBuf,
    key: String,
}

impl Side {
    pub fn current(paths: &Paths) -> Self {
        let platform = if cfg!(windows) {
            Platform::Windows
        } else if running_in_wsl() {
            Platform::Wsl { distro: std::env::var("WSL_DISTRO_NAME").unwrap_or_else(|_| "Ubuntu".into()) }
        } else {
            Platform::Other
        };
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("ccam"));
        Self::new(platform, exe, &paths.credentials_file)
    }

    pub fn new(platform: Platform, exe: PathBuf, credentials_file: &Path) -> Self {
        // Two WSL distros share the same home path, so the distro is part of who a side is.
        let distro = match &platform {
            Platform::Wsl { distro } => distro.as_str(),
            _ => "",
        };
        let id = format!("{}|{distro}|{}", std::env::consts::OS, credentials_file.display());
        let key = hex::encode(&Sha256::digest(id.as_bytes())[..6]);
        Self { platform, exe, key }
    }

    /// A short stable name for this side, used for its files in the shared vault.
    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn label(&self) -> String {
        match &self.platform {
            Platform::Windows => "Windows".into(),
            Platform::Wsl { distro } => format!("WSL ({distro})"),
            Platform::Other => "another machine".into(),
        }
    }

    /// How to run this side's ccam from `here`, when that is possible from one machine.
    pub fn command_from(&self, here: &Side) -> Option<Command> {
        match (&self.platform, &here.platform) {
            (Platform::Wsl { distro }, Platform::Windows | Platform::Wsl { .. }) => {
                let mut cmd = Command::new("wsl.exe");
                cmd.args(["-d", distro, "--"]).arg(&self.exe);
                Some(cmd)
            }
            (Platform::Windows, Platform::Wsl { .. }) => Some(Command::new(windows_to_wsl(&self.exe)?)),
            _ => None,
        }
    }
}

/// Another side's record: who it runs and how to reach it.
#[derive(Debug, Clone)]
pub struct Mark {
    pub side: Side,
    pub identity: Identity,
    pub file: PathBuf,
}

pub fn record(vault_dir: &Path, side: &Side, live: Option<&Identity>) -> Result<()> {
    // A record with this side's platform and ccam but another key is this side's own, left from before its key
    // changed (a new key scheme or config dir). Left alone it would block this side's own account forever.
    for old in others(vault_dir, side) {
        if old.side.platform == side.platform && old.side.exe == side.exe {
            let _ = std::fs::remove_file(&old.file);
        }
    }
    let file = vault_dir.join(DIR).join(format!("{}.json", side.key));
    let Some(identity) = live else {
        return match std::fs::remove_file(&file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(Error::io(&file, e)),
            _ => Ok(()),
        };
    };
    let dir = vault_dir.join(DIR);
    std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
    let (platform, distro) = match &side.platform {
        Platform::Windows => ("windows", None),
        Platform::Wsl { distro } => ("wsl", Some(distro.as_str())),
        Platform::Other => ("other", None),
    };
    let doc = json!({
        "accountUuid": identity.account_uuid,
        "organizationUuid": identity.organization_uuid,
        "platform": platform,
        "distro": distro,
        "exe": side.exe,
        "key": side.key,
    });
    // Most syncs change nothing, and every write goes over drvfs from WSL.
    if jsonio::read(&file).ok().flatten().as_ref() == Some(&doc) {
        return Ok(());
    }
    jsonio::write_atomic(&file, &doc, Style::Pretty, false)
}

/// Every other side's record. Unreadable records are skipped: they cannot block a switch they cannot describe.
pub fn others(vault_dir: &Path, here: &Side) -> Vec<Mark> {
    let dir = vault_dir.join(DIR);
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };
    entries
        .flatten()
        .filter_map(|entry| {
            let file = entry.path();
            let doc = jsonio::read(&file).ok()??;
            let mark = parse(&doc, file)?;
            (mark.side.key != here.key).then_some(mark)
        })
        .collect()
}

fn parse(doc: &Value, file: PathBuf) -> Option<Mark> {
    let text = |key: &str| doc.get(key).and_then(Value::as_str);
    let platform = match text("platform")? {
        "windows" => Platform::Windows,
        "wsl" => Platform::Wsl { distro: text("distro")?.to_string() },
        _ => Platform::Other,
    };
    Some(Mark {
        side: Side { platform, exe: PathBuf::from(text("exe")?), key: text("key")?.to_string() },
        identity: Identity {
            account_uuid: text("accountUuid")?.to_string(),
            organization_uuid: text("organizationUuid").map(str::to_string),
        },
        file,
    })
}

pub(crate) fn running_in_wsl() -> bool {
    std::fs::read_to_string("/proc/version").is_ok_and(|v| v.to_ascii_lowercase().contains("microsoft"))
}

/// `C:\Users\me\ccam.exe` as WSL sees it: `/mnt/c/Users/me/ccam.exe`.
fn windows_to_wsl(path: &Path) -> Option<PathBuf> {
    let text = path.to_str()?;
    let (drive, rest) = text.split_once(":\\")?;
    (drive.len() == 1)
        .then(|| PathBuf::from(format!("/mnt/{}/{}", drive.to_ascii_lowercase(), rest.replace('\\', "/"))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(uuid: &str) -> Identity {
        Identity { account_uuid: uuid.into(), organization_uuid: Some("org".into()) }
    }

    fn wsl() -> Side {
        Side::new(Platform::Wsl { distro: "Ubuntu".into() }, "/home/me/.cargo/bin/ccam".into(), Path::new("/w"))
    }

    fn windows() -> Side {
        Side::new(Platform::Windows, r"C:\Users\me\.cargo\bin\ccam.exe".into(), Path::new("/x"))
    }

    #[test]
    fn a_side_sees_the_others_records_but_not_its_own() {
        let dir = tempfile::tempdir().unwrap();
        record(dir.path(), &windows(), Some(&identity("A"))).unwrap();
        record(dir.path(), &wsl(), Some(&identity("B"))).unwrap();

        let seen_from_windows = others(dir.path(), &windows());
        assert_eq!(seen_from_windows.len(), 1);
        assert_eq!(seen_from_windows[0].identity, identity("B"));
        assert_eq!(seen_from_windows[0].side.label(), "WSL (Ubuntu)");

        record(dir.path(), &wsl(), None).unwrap();
        assert!(others(dir.path(), &windows()).is_empty());
        record(dir.path(), &wsl(), None).unwrap();
    }

    #[test]
    fn a_side_replaces_its_own_record_left_under_an_older_key() {
        let dir = tempfile::tempdir().unwrap();
        let before = Side::new(Platform::Windows, r"C:\ccam.exe".into(), Path::new("/old-config"));
        let after = Side::new(Platform::Windows, r"C:\ccam.exe".into(), Path::new("/new-config"));
        record(dir.path(), &before, Some(&identity("A"))).unwrap();
        record(dir.path(), &wsl(), Some(&identity("B"))).unwrap();

        record(dir.path(), &after, Some(&identity("A"))).unwrap();

        let seen = others(dir.path(), &after);
        assert_eq!(seen.len(), 1, "only the WSL record is left: {seen:?}");
        assert_eq!(seen[0].identity, identity("B"));
    }

    #[test]
    fn two_distros_with_the_same_home_are_different_sides() {
        let dir = tempfile::tempdir().unwrap();
        let debian = Side::new(Platform::Wsl { distro: "Debian".into() }, "/ccam".into(), Path::new("/w"));
        record(dir.path(), &wsl(), Some(&identity("A"))).unwrap();
        record(dir.path(), &debian, Some(&identity("B"))).unwrap();

        assert_eq!(others(dir.path(), &debian)[0].identity, identity("A"));
        assert_eq!(others(dir.path(), &wsl())[0].identity, identity("B"));
    }

    #[test]
    fn each_side_can_reach_the_other() {
        let from_windows = format!("{:?}", wsl().command_from(&windows()).unwrap());
        assert!(from_windows.contains("wsl.exe") && from_windows.contains("Ubuntu"), "{from_windows}");
        let from_wsl = format!("{:?}", windows().command_from(&wsl()).unwrap());
        assert!(from_wsl.contains("/mnt/c/Users/me/.cargo/bin/ccam.exe"), "{from_wsl}");
    }
}
