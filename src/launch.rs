//! Starting an interactive Claude Code in the current terminal.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Overrides which binary is run, for tests. Production use never sets it.
pub const CLAUDE_BIN_ENV: &str = "CCAM_CLAUDE_BIN";

/// The Claude Code binary: the override, else the one that started this process, else `claude` on PATH.
pub fn claude_binary() -> PathBuf {
    std::env::var_os(CLAUDE_BIN_ENV)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("CLAUDE_CODE_EXECPATH").map(PathBuf::from).filter(|p| p.exists()))
        .unwrap_or_else(|| PathBuf::from("claude"))
}

/// `claude` with this process's environment minus Claude Code's session markers. `config_dir` replaces
/// `CLAUDE_CONFIG_DIR`, for a lane.
///
/// Claude Code marks its child processes with CLAUDE* variables; a Claude Code started with them believes it is
/// a nested child session and does not save its transcript. `CLAUDE_CONFIG_DIR` is configuration, not a marker.
pub fn command(config_dir: Option<&Path>) -> Command {
    let mut command = Command::new(claude_binary());
    for (key, _) in std::env::vars_os() {
        let upper = key.to_string_lossy().to_ascii_uppercase();
        if upper.starts_with("CLAUDE") && upper != "CLAUDE_CONFIG_DIR" {
            command.env_remove(key);
        }
    }
    if let Some(dir) = config_dir {
        command.env("CLAUDE_CONFIG_DIR", dir);
    }
    command
}

/// Runs `claude <args>` attached to this terminal and returns its exit code.
pub fn claude(args: &[String], config_dir: Option<&Path>) -> std::io::Result<i32> {
    Ok(command(config_dir).args(args).status()?.code().unwrap_or(1))
}
