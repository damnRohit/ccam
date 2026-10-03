use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Every location ccam reads or writes, resolved once from the environment the way Claude Code resolves them.
#[derive(Debug, Clone)]
pub struct Paths {
    /// `$CLAUDE_CONFIG_DIR`, or `~/.claude`.
    pub config_home: PathBuf,
    /// The raw `CLAUDE_CONFIG_DIR` value. The macOS Keychain service name is derived from the unresolved string.
    pub config_dir_env: Option<String>,
    pub credentials_file: PathBuf,
    /// `.claude.json`, which holds `oauthAccount` among much else.
    pub global_config: PathBuf,
    pub settings_file: PathBuf,
    /// ccam's saved accounts. Deliberately not `accounts/`, which belongs to the PowerShell prototype: ccam
    /// never writes there, so a failed or abandoned ccam trial cannot damage the prototype's accounts.
    pub vault_dir: PathBuf,
    /// The PowerShell prototype's accounts. Read once by `import-legacy`, never written.
    pub prototype_vault_dir: PathBuf,
    /// ccam's own lock around vault changes. Taken before any of Claude Code's locks.
    pub vault_lock: PathBuf,
    /// Claude Code takes this lock, then `legacy_refresh_lock`, around every token refresh.
    pub refresh_lock: PathBuf,
    pub legacy_refresh_lock: PathBuf,
    /// Claude Code takes this lock around writes to `global_config`.
    pub global_config_lock: PathBuf,
}

impl Paths {
    pub fn from_env() -> Result<Self> {
        Self::for_config_dir(std::env::var("CLAUDE_CONFIG_DIR").ok())
    }

    /// Like `from_env`, with the config dir given explicitly. Empty means unset, as for Claude Code.
    pub fn for_config_dir(config_dir: Option<String>) -> Result<Self> {
        let home = std::env::home_dir().ok_or(Error::NoHome)?;
        Ok(Self::resolve(&home, config_dir.filter(|v| !v.is_empty())))
    }

    pub fn resolve(home: &Path, config_dir_env: Option<String>) -> Self {
        let config_home = config_dir_env.as_deref().map(PathBuf::from).unwrap_or_else(|| home.join(".claude"));

        // Claude Code prefers a legacy `.config.json` inside the config home when one exists.
        let legacy_config = config_home.join(".config.json");
        let global_config = if legacy_config.exists() {
            legacy_config
        } else if config_dir_env.is_some() {
            config_home.join(".claude.json")
        } else {
            home.join(".claude.json")
        };

        Self {
            credentials_file: config_home.join(".credentials.json"),
            settings_file: config_home.join("settings.json"),
            vault_dir: config_home.join("ccam").join("accounts"),
            vault_lock: config_home.join("ccam").join("accounts").join(crate::vault::LOCK_DIR),
            prototype_vault_dir: config_home.join("accounts"),
            refresh_lock: config_home.join(".oauth_refresh.lock"),
            legacy_refresh_lock: with_suffix(&config_home, ".lock"),
            global_config_lock: with_suffix(&global_config, ".lock"),
            global_config,
            config_home,
            config_dir_env,
        }
    }
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_layout_matches_claude_code() {
        let p = Paths::resolve(Path::new("/h"), None);
        assert_eq!(p.credentials_file, Path::new("/h/.claude/.credentials.json"));
        assert_eq!(p.global_config, Path::new("/h/.claude.json"));
        assert_eq!(p.legacy_refresh_lock, Path::new("/h/.claude.lock"));
        assert_eq!(p.global_config_lock, Path::new("/h/.claude.json.lock"));
        assert_eq!(p.vault_dir, Path::new("/h/.claude/ccam/accounts"));
        assert_eq!(p.prototype_vault_dir, Path::new("/h/.claude/accounts"));
    }

    #[test]
    fn config_dir_moves_everything_including_the_global_config() {
        let p = Paths::resolve(Path::new("/h"), Some("/work".into()));
        assert_eq!(p.credentials_file, Path::new("/work/.credentials.json"));
        assert_eq!(p.global_config, Path::new("/work/.claude.json"));
        assert_eq!(p.legacy_refresh_lock, Path::new("/work.lock"));
    }
}
