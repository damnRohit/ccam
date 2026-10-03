//! `ccam to <name> --restart` on Windows: reopen the conversation in a new Windows Terminal tab on the new account.
//!
//! ```text
//! old tab (inside Claude Code)                  new tab
//! ────────────────────────────                  ───────
//! pre-check the switch
//! open tab ───────────────────────────────────▶ write ready file
//! wait for ready file (20 s)                    wait for old Claude Code to exit (60 s)
//! wait until 5 s after launch
//! take Claude Code's refresh locks
//! terminate Claude Code and its tab shell
//! release the locks once it has exited ───────▶ switch accounts (waits out a lock left by a crash)
//!                                               run `claude --continue`
//! ```
//!
//! The switch runs in the new tab, after the old process is gone, so the old process can never write its stale
//! login back. Holding the refresh locks while terminating means Claude Code is never killed between receiving
//! a rotated token and saving it, which would lose the only working login.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_INVALID_PARAMETER, GetLastError, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, TerminateProcess, WaitForSingleObject,
};

use crate::error::{Error, Result};
use crate::manager::{Manager, Switched};
use crate::name::AccountName;

const READY_TIMEOUT: Duration = Duration::from_secs(20);
/// Long enough for the user to see the new tab open before the old one disappears.
const OLD_TAB_GRACE: Duration = Duration::from_secs(5);
const OLD_CLAUDE_EXIT_TIMEOUT: Duration = Duration::from_secs(60);

/// Whether this terminal is a Windows Terminal tab, which is the only kind a new tab can be opened in.
pub fn available() -> bool {
    std::env::var_os("WT_SESSION").is_some()
}

/// Runs in the old tab. Returns once the old Claude Code process has been terminated.
pub fn launch(manager: &Manager, name: &AccountName) -> Result<()> {
    let claude_pid = calling_claude().ok_or_else(|| {
        Error::Restart("--restart must run inside Claude Code, e.g. `! ccam to <name> --restart`".into())
    })?;
    manager.check_switch(name)?;

    let exe = std::env::current_exe().map_err(|e| Error::Restart(format!("cannot locate ccam: {e}")))?;
    let cwd = std::env::current_dir().map_err(|e| Error::Restart(format!("cannot read the working directory: {e}")))?;
    let ready = manager.paths().vault_dir.join(format!(".ready-{claude_pid}"));
    std::fs::create_dir_all(&manager.paths().vault_dir).map_err(|e| Error::io(&manager.paths().vault_dir, e))?;
    let _ = std::fs::remove_file(&ready);

    // A tab opened in an existing window inherits that window's environment, not this process's, so the config
    // dir always travels as an argument, empty meaning unset.
    let config_dir: OsString = manager.paths().config_dir_env.clone().unwrap_or_default().into();
    let args: Vec<OsString> = vec![
        "-w".into(),
        "0".into(),
        "nt".into(),
        "-d".into(),
        cwd.into_os_string(),
        exe.into_os_string(),
        "resume".into(),
        name.as_str().into(),
        "--claude-pid".into(),
        claude_pid.to_string().into(),
        "--ready-file".into(),
        ready.clone().into_os_string(),
        "--claude-config-dir".into(),
        config_dir,
    ];
    // wt.exe splits its own command line on ';'. Names cannot contain it; paths that do are refused.
    if args.iter().any(|a| a.to_string_lossy().contains(';')) {
        return Err(Error::Restart("a path contains ';', which Windows Terminal cannot pass through".into()));
    }

    let started = Instant::now();
    without_claude_markers(Command::new("wt.exe").args(&args))
        .spawn()
        .map_err(|e| Error::Restart(format!("cannot start Windows Terminal: {e}")))?;
    while !ready.exists() {
        if started.elapsed() >= READY_TIMEOUT {
            return Err(Error::Restart("the new tab did not start, so nothing was switched".into()));
        }
        thread::sleep(Duration::from_millis(200));
    }
    let _ = std::fs::remove_file(&ready);
    if let Some(remaining) = OLD_TAB_GRACE.checked_sub(started.elapsed()) {
        thread::sleep(remaining);
    }

    let locks = manager.hold_refresh_locks()?;
    let shell = tab_shell(claude_pid);
    terminate(claude_pid);
    if let Some(shell) = shell {
        terminate(shell);
    }
    if !wait_for_exit(claude_pid, Duration::from_secs(5)) {
        manager.log(&format!("restart: claude pid {claude_pid} still running 5 s after termination"));
    }
    drop(locks);
    Ok(())
}

/// Runs in the new tab. Switches once the old Claude Code has exited, then runs `claude --continue` and returns
/// its exit code.
pub fn resume(manager: &Manager, name: &AccountName, claude_pid: u32, ready_file: &Path) -> Result<i32> {
    std::fs::write(ready_file, "ready").map_err(|e| Error::io(ready_file, e))?;
    if !wait_for_exit(claude_pid, OLD_CLAUDE_EXIT_TIMEOUT) {
        return Err(Error::Restart("the old Claude Code session is still running, so nothing was switched".into()));
    }
    let Switched { to, .. } = manager.switch_after_kill(name)?;
    println!("Now on '{to}'.");

    let claude = std::env::var_os("CLAUDE_CODE_EXECPATH").map(PathBuf::from).filter(|p| p.exists());
    let mut command = Command::new(claude.unwrap_or_else(|| PathBuf::from("claude")));
    command.arg("--continue");
    match &manager.paths().config_dir_env {
        Some(dir) => command.env("CLAUDE_CONFIG_DIR", dir),
        None => command.env_remove("CLAUDE_CONFIG_DIR"),
    };
    let status = without_claude_markers(&mut command)
        .status()
        .map_err(|e| Error::Restart(format!("cannot start claude: {e}")))?;
    Ok(status.code().unwrap_or(1))
}

/// The Claude Code process that ran this command, from the documented `CLAUDE_PID` variable (Claude Code
/// 2.1.214+). Walking up the process tree is unreliable: Git Bash hides the real parent of what it spawns.
fn calling_claude() -> Option<u32> {
    let pid: u32 = std::env::var("CLAUDE_PID").ok()?.parse().ok()?;
    process(pid)?.name.eq_ignore_ascii_case("claude.exe").then_some(pid)
}

/// The shell hosting Claude Code, when it is a plain shell tab in Windows Terminal. Terminating it closes the old
/// tab. Anything else (an IDE terminal, a nested shell) is left alone.
fn tab_shell(claude_pid: u32) -> Option<u32> {
    let shell_pid = process(claude_pid)?.parent_pid;
    let shell = process(shell_pid)?;
    let host = process(shell.parent_pid)?;
    let plain_shell = ["powershell.exe", "pwsh.exe", "cmd.exe"].iter().any(|s| shell.name.eq_ignore_ascii_case(s));
    (plain_shell && host.name.eq_ignore_ascii_case("WindowsTerminal.exe")).then_some(shell_pid)
}

/// Claude Code marks its child processes with CLAUDE* variables. A Claude Code started with them believes it is
/// a nested child session and does not save its transcript. `CLAUDE_CONFIG_DIR` is configuration, not a marker.
fn without_claude_markers(command: &mut Command) -> &mut Command {
    for (key, _) in std::env::vars_os() {
        let upper = key.to_string_lossy().to_ascii_uppercase();
        if upper.starts_with("CLAUDE") && upper != "CLAUDE_CONFIG_DIR" {
            command.env_remove(key);
        }
    }
    command
}

struct ProcessInfo {
    name: String,
    parent_pid: u32,
}

fn process(pid: u32) -> Option<ProcessInfo> {
    // SAFETY: the snapshot handle is checked before use and closed on every path; the entry is initialised with
    // its size as the API requires.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut found = None;
        let mut more = Process32FirstW(snapshot, &mut entry) != 0;
        while more {
            if entry.th32ProcessID == pid {
                let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
                found = Some(ProcessInfo {
                    name: String::from_utf16_lossy(&entry.szExeFile[..len]),
                    parent_pid: entry.th32ParentProcessID,
                });
                break;
            }
            more = Process32NextW(snapshot, &mut entry) != 0;
        }
        CloseHandle(snapshot);
        found
    }
}

/// True once the process has exited; false if it is still running after `timeout` or cannot be observed.
fn wait_for_exit(pid: u32, timeout: Duration) -> bool {
    // SAFETY: the handle is null-checked and closed.
    unsafe {
        let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if handle.is_null() {
            // Only "no such process" means it exited. Access denied (an elevated Claude Code) means it may still be
            // running, and switching under it would be unsafe.
            return GetLastError() == ERROR_INVALID_PARAMETER;
        }
        let result = WaitForSingleObject(handle, u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX));
        CloseHandle(handle);
        result == WAIT_OBJECT_0
    }
}

fn terminate(pid: u32) {
    // SAFETY: the handle is null-checked and closed.
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if !handle.is_null() {
            TerminateProcess(handle, 1);
            CloseHandle(handle);
        }
    }
}
