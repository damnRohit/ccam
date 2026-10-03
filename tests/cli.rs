//! End-to-end tests of the built binary against a temporary home directory and the file credential store.
//!
//! No test reaches a real network: `CCAM_API_BASE` points at an unroutable address, and the fake logins are
//! shaped so that no command needs to confirm ownership online.

use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::str::contains;
use serde_json::{Value, json};

struct Home {
    dir: tempfile::TempDir,
}

impl Home {
    /// Creates an isolated home and proves the binary uses it before any test mutates anything.
    fn new() -> Self {
        let home = Self { dir: tempfile::tempdir().unwrap() };
        fs::create_dir_all(home.claude()).unwrap();
        let out = home.stdout(&["list"]);
        assert!(out.contains("No saved accounts"), "ccam is not isolated to the temporary home; refusing to continue");
        home
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn claude(&self) -> PathBuf {
        self.path().join(".claude")
    }

    fn ccam(&self, args: &[&str]) -> Command {
        let mut cmd = Command::cargo_bin("ccam").unwrap();
        cmd.args(args)
            .env("HOME", self.path())
            .env("USERPROFILE", self.path())
            .env("CCAM_CREDENTIAL_STORE", "file")
            .env("CCAM_API_BASE", "http://127.0.0.1:9")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CLAUDE_PID")
            .env_remove("WT_SESSION")
            .env("NO_COLOR", "1");
        cmd
    }

    /// Logs in as `uuid` with an already-expired access token, so nothing tries the network. Renewals of one
    /// login share `refreshTokenExpiresAt`, which lets syncs and switches skip the ownership check.
    fn log_in(&self, uuid: &str, expires_at: i64) {
        let creds = json!({
            "claudeAiOauth": {
                "accessToken": format!("at-{uuid}-{expires_at}"),
                "refreshToken": format!("rt-{uuid}-{expires_at}"),
                "expiresAt": expires_at,
                "refreshTokenExpiresAt": 5,
                "subscriptionType": "pro",
            },
            "mcpOAuth": {"keep": true},
        });
        fs::write(self.claude().join(".credentials.json"), creds.to_string()).unwrap();
        let config = json!({"projects": {"x": 1}, "oauthAccount": {"accountUuid": uuid, "organizationUuid": "org"}});
        fs::write(self.path().join(".claude.json"), config.to_string()).unwrap();
    }

    fn read(&self, relative: &str) -> Value {
        serde_json::from_str(&fs::read_to_string(self.path().join(relative)).unwrap()).unwrap()
    }

    fn stdout(&self, args: &[&str]) -> String {
        String::from_utf8(self.ccam(args).assert().success().get_output().stdout.clone()).unwrap()
    }
}

#[test]
fn saved_accounts_can_be_listed_renamed_and_removed() {
    let home = Home::new();
    home.log_in("A", 1);
    home.stdout(&["save", "work-1"]);
    home.log_in("B", 1);
    home.stdout(&["save", "home.me@x"]);

    assert_eq!(home.stdout(&["list"]), "* home.me@x  (pro)\n  work-1  (pro)\n");

    home.stdout(&["rename", "work-1", "office"]);
    home.stdout(&["remove", "home.me@x"]);
    assert_eq!(
        home.stdout(&["list"]),
        "  office  (pro)\n\nThe logged-in account is not saved. Run: ccam save <name>\n"
    );
}

#[test]
fn switching_in_place_swaps_login_and_identity_and_keeps_the_rest() {
    let home = Home::new();
    home.log_in("A", 1);
    home.stdout(&["save", "a"]);
    home.log_in("B", 2);
    home.stdout(&["save", "b"]);

    let out = home.stdout(&["to", "a"]);

    assert!(out.starts_with("Switched from 'b' to 'a'."), "{out}");
    let creds = home.read(".claude/.credentials.json");
    assert_eq!(creds["claudeAiOauth"]["accessToken"], "at-A-1");
    assert_eq!(creds["mcpOAuth"], json!({"keep": true}));
    let config = home.read(".claude.json");
    assert_eq!(config["oauthAccount"]["accountUuid"], "A");
    assert_eq!(config["projects"], json!({"x": 1}));
}

#[test]
fn switching_back_and_forth_never_loses_a_renewed_login() {
    let home = Home::new();
    home.log_in("A", 10);
    home.stdout(&["save", "a"]);
    home.log_in("B", 20);
    home.stdout(&["save", "b"]);

    let mut expiry = 100;
    for target in ["a", "b", "a", "b"] {
        // Claude Code renews the live login before each switch.
        let live = home.read(".claude.json")["oauthAccount"]["accountUuid"].as_str().unwrap().to_string();
        home.log_in(&live, expiry);
        home.stdout(&["to", target]);
        let saved = home.read(&format!(".claude/ccam/accounts/{}/login.json", live.to_ascii_lowercase()));
        assert_eq!(saved["claudeAiOauth"]["expiresAt"], expiry, "the renewed login of '{live}' was saved");
        expiry += 100;
    }
}

#[test]
fn user_mistakes_exit_1_and_environment_failures_exit_2() {
    let home = Home::new();
    home.log_in("A", 1);

    home.ccam(&["save", "bad name"]).assert().code(1);
    home.ccam(&["to", "missing"]).assert().code(1).stderr(contains("no saved account named 'missing'"));
    home.stdout(&["save", "a"]);
    home.ccam(&["to", "A"]).assert().code(1).stderr(contains("already on 'a'"));

    fs::write(home.path().join(".claude.json"), "{not json").unwrap();
    home.ccam(&["to", "a"]).assert().code(2).stderr(contains("invalid JSON"));
}

#[test]
fn an_unreadable_live_login_is_never_replaced_or_saved() {
    let home = Home::new();
    home.log_in("A", 1);
    home.stdout(&["save", "a"]);
    let creds = home.claude().join(".credentials.json");
    fs::write(&creds, r#"{"claudeAiOauth":{"accessToken":"orphan"},"mcpOAuth":{"keep":true}}"#).unwrap();

    home.ccam(&["to", "a"]).assert().code(1).stderr(contains("could not be read"));
    home.ccam(&["save", "b"]).assert().code(1);
    assert_eq!(home.read(".claude/.credentials.json")["claudeAiOauth"]["accessToken"], "orphan");
    assert!(home.stdout(&["list"]).contains("could not be read"));
}

#[test]
fn setup_and_uninstall_manage_hooks_in_the_claude_settings() {
    let home = Home::new();
    fs::write(home.claude().join("settings.json"), r#"{"model":"opus"}"#).unwrap();

    home.stdout(&["setup"]);
    let settings = home.read(".claude/settings.json");
    assert_eq!(settings["model"], "opus");
    assert_eq!(settings["hooks"]["SessionStart"][0]["hooks"][0]["args"], json!(["hook", "session-start"]));
    assert!(home.stdout(&["setup"]).starts_with("Hooks already installed"));

    home.stdout(&["uninstall"]);
    assert_eq!(home.read(".claude/settings.json"), json!({"model": "opus"}));
}

#[test]
fn session_start_hook_registers_the_credentials_file_and_syncs_offline() {
    let home = Home::new();
    home.log_in("A", 1);
    home.stdout(&["save", "a"]);
    home.log_in("A", 9);

    let out = home.ccam(&["hook", "session-start"]).write_stdin("{}").assert().success().get_output().stdout.clone();

    let output: Value = serde_json::from_slice(&out).unwrap();
    let watched = output["hookSpecificOutput"]["watchPaths"][0].as_str().unwrap();
    assert!(watched.ends_with(".credentials.json"));
    assert_eq!(home.read(".claude/ccam/accounts/a/login.json")["claudeAiOauth"]["expiresAt"], 9);
}

#[test]
fn file_changed_hook_syncs_only_for_the_credentials_file() {
    let home = Home::new();
    home.log_in("A", 1);
    home.stdout(&["save", "a"]);
    home.log_in("A", 9);
    let creds = home.claude().join(".credentials.json");

    let other = json!({"file_path": home.path().join("other.json")}).to_string();
    home.ccam(&["hook", "file-changed"]).write_stdin(other).assert().success().stdout("");
    assert_eq!(home.read(".claude/ccam/accounts/a/login.json")["claudeAiOauth"]["expiresAt"], 1);

    let ours = json!({"file_path": creds.to_string_lossy().to_uppercase()}).to_string();
    home.ccam(&["hook", "file-changed"]).write_stdin(ours).assert().success().stdout("");
    assert_eq!(home.read(".claude/ccam/accounts/a/login.json")["claudeAiOauth"]["expiresAt"], 9);
}

#[test]
fn hooks_exit_cleanly_on_garbage_input() {
    let home = Home::new();
    home.ccam(&["hook", "file-changed"]).write_stdin("not json").assert().success().stdout("");
    home.ccam(&["hook", "auth-success"]).assert().success();
}

#[test]
fn usage_json_reports_expired_accounts_without_network() {
    let home = Home::new();
    home.log_in("A", 1);
    home.stdout(&["save", "a"]);
    home.log_in("B", 1);
    home.stdout(&["save", "b"]);

    let out: Value = serde_json::from_str(&home.stdout(&["usage", "--json"])).unwrap();

    assert_eq!(out[0]["name"], "a");
    assert_eq!(out[0]["status"], "stale");
    assert_eq!(out[1]["name"], "b");
    assert_eq!(out[1]["current"], true);
    assert_eq!(out[1]["status"], "error");
}

#[test]
fn import_legacy_converts_prototype_accounts_once() {
    let home = Home::new();
    let legacy = home.claude().join("accounts/work.main");
    fs::create_dir_all(&legacy).unwrap();
    let creds = json!({"claudeAiOauth": {"accessToken": "a", "refreshToken": "r", "expiresAt": 7}, "mcpOAuth": {}});
    fs::write(legacy.join(".credentials.json"), creds.to_string()).unwrap();
    fs::write(legacy.join("oauthAccount.json"), json!({"accountUuid": "L"}).to_string()).unwrap();

    assert!(home.stdout(&["list"]).contains("No saved accounts"), "prototype folders are not read automatically");
    assert!(home.stdout(&["import-legacy"]).contains("Imported 'work.main'"));
    assert_eq!(home.stdout(&["list"]), "  work.main\n");
    assert!(home.stdout(&["import-legacy"]).contains("Nothing to import"));
}

/// Every file under `dir`, relative path to bytes, so a test can prove a directory was not touched.
fn snapshot(dir: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    let mut files = std::collections::BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in fs::read_dir(&d).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path.strip_prefix(dir).unwrap().to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
                files.insert(rel, fs::read(&path).unwrap());
            }
        }
    }
    files
}

#[test]
fn ccam_never_writes_to_the_prototype_folder() {
    let home = Home::new();
    let legacy_root = home.claude().join("accounts");
    let legacy = legacy_root.join("work.main");
    fs::create_dir_all(&legacy).unwrap();
    let creds = json!({"claudeAiOauth": {"accessToken": "a", "refreshToken": "r", "expiresAt": 7}});
    fs::write(legacy.join(".credentials.json"), creds.to_string()).unwrap();
    fs::write(legacy.join("oauthAccount.json"), json!({"accountUuid": "L"}).to_string()).unwrap();
    fs::write(
        legacy_root.join("switch.log"),
        "prototype log
",
    )
    .unwrap();
    let before = snapshot(&legacy_root);

    home.stdout(&["import-legacy"]);
    home.log_in("A", 1);
    home.stdout(&["save", "a"]);
    home.log_in("B", 1);
    home.stdout(&["save", "b"]);
    home.stdout(&["to", "a"]);
    home.stdout(&["rename", "b", "bee"]);
    home.stdout(&["remove", "work.main"]);
    home.stdout(&["list"]);

    assert_eq!(snapshot(&legacy_root), before, "the prototype folder must be byte-for-byte unchanged");
    assert!(home.claude().join("ccam/accounts/a/login.json").exists());
}

#[test]
fn switching_from_a_plain_terminal_can_open_claude_afterwards() {
    let home = Home::new();
    home.log_in("A", 1);
    home.stdout(&["save", "a"]);
    home.log_in("B", 1);
    home.stdout(&["save", "b"]);

    // A fake `claude` on PATH records how it was invoked instead of starting Claude Code.
    let bin = home.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let log = home.path().join("claude-args.txt");
    #[cfg(windows)]
    fs::write(bin.join("claude.cmd"), format!("@echo args:%*> \"{}\"\r\n", log.display())).unwrap();
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        let script = bin.join("claude");
        fs::write(&script, format!("#!/bin/sh\necho \"args:$@\" > '{}'\n", log.display())).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let fake = bin.join(if cfg!(windows) { "claude.cmd" } else { "claude" });
    let run = |args: &[&str]| {
        home.ccam(args).env("CCAM_CLAUDE_BIN", &fake).assert().success();
        fs::read_to_string(&log).map(|s| s.trim().to_string()).unwrap_or_default()
    };

    // Case 1: switch only, nothing opened.
    let out = home.stdout(&["to", "a"]);
    assert!(out.contains("Open it with"), "{out}");
    assert!(!log.exists());

    // Case 2: switch and open a plain Claude.
    assert_eq!(run(&["to", "b", "--open"]), "args:");
    fs::remove_file(&log).unwrap();

    // Case 3: switch and open a specific chat, or continue the latest.
    assert_eq!(run(&["to", "a", "--open", "abc-123"]), "args:--resume abc-123");
    assert_eq!(run(&["to", "b", "--continue"]), "args:--continue");

    // Inside Claude Code, --open is refused: the session cannot be replaced from within.
    home.ccam(&["to", "a", "--open"]).env("CLAUDE_PID", "1").assert().code(2).stderr(contains("normal terminal"));
}

#[test]
fn auto_runs_in_the_background_reports_status_refuses_a_second_copy_and_stops() {
    let home = Home::new();
    home.log_in("A", 4_102_444_800_000);
    home.stdout(&["save", "a"]);

    assert!(home.stdout(&["auto", "status"]).contains("not running"));
    assert!(home.stdout(&["auto", "stop"]).contains("not running"));
    home.ccam(&["auto", "--threshold", "20"]).assert().code(1).stderr(contains("between 50 and 100"));

    /// Kills the background loop even when an assertion fails, so it cannot outlive the test and hold cargo's
    /// output pipe open.
    struct KillOnDrop(std::process::Child);
    impl Drop for KillOnDrop {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    let exe = assert_cmd::cargo::cargo_bin("ccam");
    let child = std::process::Command::new(exe)
        .args(["auto", "--no-desktop"])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("CCAM_CREDENTIAL_STORE", "file")
        .env("CCAM_API_BASE", "http://127.0.0.1:9")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CLAUDE_PID")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let mut child = KillOnDrop(child);

    let status_file = home.claude().join("ccam/accounts/.auto.status.json");
    for _ in 0..100 {
        if status_file.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let status: Value = serde_json::from_str(&home.stdout(&["auto", "status"])).unwrap();
    assert_eq!(status["mode"], "auto");
    home.ccam(&["auto", "--no-desktop"]).assert().code(2).stderr(contains("already running"));

    assert!(home.stdout(&["auto", "stop"]).contains("stopped"));
    let exited = (0..40).any(|_| {
        std::thread::sleep(std::time::Duration::from_millis(250));
        child.0.try_wait().unwrap().is_some()
    });
    assert!(exited, "ccam auto did not exit after stop");
}
