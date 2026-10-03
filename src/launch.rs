//! Starting an interactive Claude Code in the current terminal.

use std::path::PathBuf;
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

/// Runs `claude <args>` attached to this terminal and returns its exit code.
///
/// Claude Code marks its child processes with CLAUDE* variables; a Claude Code started with them believes it is
/// a nested child session and does not save its transcript. `CLAUDE_CONFIG_DIR` is configuration, not a marker.
pub fn claude(args: &[String]) -> std::io::Result<i32> {
    let mut command = Command::new(claude_binary());
    command.args(args);
    for (key, _) in std::env::vars_os() {
        let upper = key.to_string_lossy().to_ascii_uppercase();
        if upper.starts_with("CLAUDE") && upper != "CLAUDE_CONFIG_DIR" {
            command.env_remove(key);
        }
    }
    Ok(command.status()?.code().unwrap_or(1))
}
