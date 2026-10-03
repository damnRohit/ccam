//! Where Claude Code keeps the live login.

use std::path::PathBuf;

use serde_json::Value;

use crate::error::Result;
use crate::jsonio::{self, Style};
use crate::login::OAuthLogin;
use crate::paths::Paths;

const LOGIN_KEY: &str = "claudeAiOauth";

/// Overrides store detection. Tests set it to `file` so they can never reach a real macOS Keychain, which is
/// keyed by user name and not isolated by a temporary `HOME`.
pub const STORE_OVERRIDE_ENV: &str = "CCAM_CREDENTIAL_STORE";

/// Read and replace the live `claudeAiOauth` login. Implementations must preserve every other key in the
/// underlying document, notably `mcpOAuth`, which is machine state shared by all accounts.
pub trait LoginStore: Send + Sync {
    /// The raw login object, unparsed: an unreadable live login must be reported, not mistaken for none.
    fn read(&self) -> Result<Option<Value>>;
    fn write(&self, login: &OAuthLogin) -> Result<()>;
}

/// Picks the store Claude Code uses on this platform.
pub fn for_platform(paths: &Paths) -> Result<Box<dyn LoginStore>> {
    let file = || Box::new(FileStore::new(paths.credentials_file.clone())) as Box<dyn LoginStore>;
    match std::env::var(STORE_OVERRIDE_ENV).as_deref() {
        Ok("file") => return Ok(file()),
        #[cfg(target_os = "macos")]
        Ok("keychain") => return Ok(Box::new(keychain::KeychainStore::new(paths))),
        _ => {}
    }
    #[cfg(target_os = "macos")]
    {
        let keychain = keychain::KeychainStore::new(paths);
        // Claude Code falls back to the file only when the Keychain refuses writes, so a file with no Keychain
        // item means the fallback is in use. A locked Keychain is an error, not "no item".
        if !keychain.exists()? && paths.credentials_file.exists() {
            return Ok(file());
        }
        Ok(Box::new(keychain))
    }
    #[cfg(not(target_os = "macos"))]
    Ok(file())
}

fn document_with_login(existing: Option<Value>, login: &OAuthLogin) -> Value {
    let mut map = match existing {
        Some(Value::Object(map)) => map,
        _ => serde_json::Map::new(),
    };
    map.insert(LOGIN_KEY.into(), login.to_value());
    Value::Object(map)
}

/// `.credentials.json`, used on Windows and Linux, and on macOS when the Keychain is unavailable.
pub struct FileStore {
    path: PathBuf,
}

impl FileStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl LoginStore for FileStore {
    fn read(&self) -> Result<Option<Value>> {
        Ok(jsonio::read(&self.path)?.and_then(|mut doc| doc.get_mut(LOGIN_KEY).map(Value::take)))
    }

    fn write(&self, login: &OAuthLogin) -> Result<()> {
        let doc = document_with_login(jsonio::read(&self.path)?, login);
        jsonio::write_atomic(&self.path, &doc, Style::Compact, true)
    }
}

#[cfg(target_os = "macos")]
mod keychain {
    use std::io::Write;
    use std::process::{Command, Stdio};

    use sha2::{Digest, Sha256};

    use super::*;
    use crate::error::Error;

    const SECURITY: &str = "/usr/bin/security";
    const NOT_FOUND: i32 = 44;
    /// `security -i` reads each command into a 4096-byte buffer.
    const INTERACTIVE_LINE_LIMIT: usize = 4032;

    pub struct KeychainStore {
        service: String,
        account: String,
    }

    impl KeychainStore {
        pub fn new(paths: &Paths) -> Self {
            // Claude Code keys the item to the raw CLAUDE_CONFIG_DIR string, not the resolved path.
            let service = match &paths.config_dir_env {
                Some(dir) => {
                    let digest = hex::encode(Sha256::digest(dir.as_bytes()));
                    format!("Claude Code-credentials-{}", &digest[..8])
                }
                None => "Claude Code-credentials".to_string(),
            };
            let account = std::env::var("USER").unwrap_or_else(|_| "claude-code-user".into());
            Self { service, account }
        }

        pub fn exists(&self) -> Result<bool> {
            let status = Command::new(SECURITY)
                .args(["find-generic-password", "-a", &self.account, "-s", &self.service])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map_err(|e| Error::Keychain(format!("cannot run {SECURITY}: {e}")))?;
            match status.code() {
                Some(0) => Ok(true),
                Some(NOT_FOUND) => Ok(false),
                _ => Err(Error::Keychain("locked or access denied".into())),
            }
        }

        fn read_document(&self) -> Result<Option<Value>> {
            let output = Command::new(SECURITY)
                .args(["find-generic-password", "-a", &self.account, "-w", "-s", &self.service])
                .output()
                .map_err(|e| Error::Keychain(format!("cannot run {SECURITY}: {e}")))?;
            match output.status.code() {
                Some(0) => {
                    let text = String::from_utf8_lossy(&output.stdout);
                    serde_json::from_str(text.trim())
                        .map(Some)
                        .map_err(|e| Error::Keychain(format!("item is not JSON: {e}")))
                }
                Some(NOT_FOUND) => Ok(None),
                // Must not be mistaken for "logged out".
                _ => Err(Error::Keychain("locked or access denied".into())),
            }
        }
    }

    fn quote(s: &str) -> String {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    }

    impl LoginStore for KeychainStore {
        fn read(&self) -> Result<Option<Value>> {
            Ok(self.read_document()?.and_then(|mut doc| doc.get_mut(LOGIN_KEY).map(Value::take)))
        }

        fn write(&self, login: &OAuthLogin) -> Result<()> {
            let doc = document_with_login(self.read_document()?, login);
            let secret = hex::encode(serde_json::to_vec(&doc).expect("JSON values always serialize"));
            let line = format!(
                "add-generic-password -U -a {} -s {} -X {}\n",
                quote(&self.account),
                quote(&self.service),
                secret
            );

            // Sending the secret on stdin keeps it out of the process list. An item too large for the interactive
            // buffer has to go through argv, which is what Claude Code itself does in that case.
            let spawn_error = |e| Error::Keychain(format!("cannot run {SECURITY}: {e}"));
            if line.len() <= INTERACTIVE_LINE_LIMIT {
                let mut child = Command::new(SECURITY)
                    .arg("-i")
                    .stdin(Stdio::piped())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(spawn_error)?;
                child.stdin.take().expect("stdin is piped").write_all(line.as_bytes()).map_err(spawn_error)?;
                child.wait().map_err(spawn_error)?;
            } else {
                Command::new(SECURITY)
                    .args(["add-generic-password", "-U", "-a", &self.account, "-s", &self.service, "-X", &secret])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .map_err(spawn_error)?;
            }

            // `security -i` exits 0 even when the command inside it failed, so the only proof is reading it back.
            if self.read()?.as_ref() == Some(&login.to_value()) {
                Ok(())
            } else {
                Err(Error::Keychain("the write did not take effect".into()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn file_store_replaces_the_login_and_keeps_sibling_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".credentials.json");
        std::fs::write(&path, r#"{"mcpOAuth":{"server":{"token":"m"}},"claudeAiOauth":{"accessToken":"old"}}"#)
            .unwrap();
        let new = json!({"accessToken": "new", "refreshToken": "r", "expiresAt": 1});

        let store = FileStore::new(path.clone());
        store.write(&OAuthLogin::parse(new.clone()).unwrap()).unwrap();

        let doc = jsonio::read(&path).unwrap().unwrap();
        assert_eq!(doc, json!({"mcpOAuth": {"server": {"token": "m"}}, "claudeAiOauth": new}));
        assert_eq!(store.read().unwrap(), Some(new));
    }

    #[test]
    fn file_store_reads_nothing_when_logged_out() {
        let dir = tempfile::tempdir().unwrap();
        assert!(FileStore::new(dir.path().join("missing.json")).read().unwrap().is_none());
    }
}
