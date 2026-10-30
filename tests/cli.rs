//! End-to-end tests of the built binary against a temporary home directory and the file credential store.
//!
//! No test reaches a real network: `CCAM_API_BASE` points at an unroutable address, and the fake logins are
//! shaped so that no command needs to confirm ownership online. `ccam login` must confirm, so its tests point
//! `CCAM_API_BASE` at `profile_server` on 127.0.0.1 instead.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
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

    fn t3(&self) -> PathBuf {
        self.path().join(".t3")
    }

    fn ccam(&self, args: &[&str]) -> Command {
        let mut cmd = Command::cargo_bin("ccam").unwrap();
        cmd.args(args)
            .env("HOME", self.path())
            .env("USERPROFILE", self.path())
            .env("CCAM_CREDENTIAL_STORE", "file")
            .env("CCAM_API_BASE", "http://127.0.0.1:9")
            // Inside the temp home even for tests that do not care about T3 Code, so none can reach the real one.
            .env("T3CODE_HOME", self.t3())
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
fn usage_shows_email_addresses_only_when_asked() {
    let home = Home::new();
    for uuid in ["A", "B"] {
        home.log_in(uuid, 1);
        let mut config = home.read(".claude.json");
        config["oauthAccount"]["emailAddress"] = format!("{}@example.test", uuid.to_ascii_lowercase()).into();
        fs::write(home.path().join(".claude.json"), config.to_string()).unwrap();
        home.stdout(&["save", &uuid.to_ascii_lowercase()]);
    }

    assert!(!home.stdout(&["usage"]).contains("@example.test"), "emails stay hidden by default");
    let plain: Value = serde_json::from_str(&home.stdout(&["usage", "--json"])).unwrap();
    assert!(plain[0].get("email").is_none());

    let boxes = home.stdout(&["usage", "--email"]);
    assert!(boxes.contains("a@example.test") && boxes.contains("b@example.test"), "{boxes}");
    let out: Value = serde_json::from_str(&home.stdout(&["usage", "--json", "--email"])).unwrap();
    assert_eq!((out[0]["name"].as_str(), out[0]["email"].as_str()), (Some("a"), Some("a@example.test")));

    let log = fs::read_to_string(home.claude().join("ccam/accounts/switch.log")).unwrap();
    assert!(!log.contains("@example.test"), "emails never reach the log");
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
        .env("T3CODE_HOME", home.t3())
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

/// A fake `claude`: answers `--version` (from `FAKE_CLAUDE_VERSION`) and `agents --json` (from
/// `<config dir>/fake-agents.json`), and records the config dir and arguments of every other run.
///
/// `auth login` signs the config dir in as `FAKE_LOGIN_UUID` with expiry `FAKE_LOGIN_EXPIRES` (default 7), and
/// fails with 1 under `FAKE_LOGIN_FAIL`. Every `auth` run exits 9 without writing when it inherits a variable that
/// would point it at another login.
struct FakeClaude {
    bin: PathBuf,
    log: PathBuf,
}

impl FakeClaude {
    fn new(home: &Home) -> Self {
        let dir = home.path().join("bin");
        fs::create_dir_all(&dir).unwrap();
        let log = home.path().join("claude-runs.txt");
        #[cfg(windows)]
        let (bin, script) = (
            dir.join("claude.cmd"),
            [
                "@echo off",
                "if \"%~1\"==\"--version\" goto version",
                "if \"%~1\"==\"agents\" goto agents",
                &format!(">>\"{}\" echo %CLAUDE_CONFIG_DIR%^|%*", log.display()),
                "if \"%~1\"==\"auth\" goto auth",
                "exit /b 0",
                ":auth",
                "if defined CLAUDE_SECURESTORAGE_CONFIG_DIR exit /b 9",
                "if defined ANTHROPIC_API_KEY exit /b 9",
                "if defined ANTHROPIC_AUTH_TOKEN exit /b 9",
                "if defined FAKE_LOGIN_FAIL exit /b 1",
                "if not \"%~2\"==\"login\" exit /b 0",
                "if not defined FAKE_LOGIN_EXPIRES set FAKE_LOGIN_EXPIRES=7",
                ">\"%CLAUDE_CONFIG_DIR%\\.credentials.json\" echo {\"claudeAiOauth\":{\"accessToken\":\"at-%FAKE_LOGIN_UUID%-%FAKE_LOGIN_EXPIRES%\",\"refreshToken\":\"rt-%FAKE_LOGIN_UUID%-%FAKE_LOGIN_EXPIRES%\",\"expiresAt\":%FAKE_LOGIN_EXPIRES%,\"refreshTokenExpiresAt\":5,\"subscriptionType\":\"pro\"}}",
                ">\"%CLAUDE_CONFIG_DIR%\\.claude.json\" echo {\"oauthAccount\":{\"accountUuid\":\"%FAKE_LOGIN_UUID%\",\"organizationUuid\":\"org\"}}",
                "exit /b 0",
                ":version",
                "if defined FAKE_CLAUDE_VERSION (echo %FAKE_CLAUDE_VERSION%) else (echo 2.1.296 ^(Claude Code^))",
                "exit /b 0",
                ":agents",
                "if exist \"%CLAUDE_CONFIG_DIR%\\fake-agents.json\" (type \"%CLAUDE_CONFIG_DIR%\\fake-agents.json\") else (echo [])",
            ]
            .join("\r\n"),
        );
        #[cfg(not(windows))]
        let (bin, script) = (
            dir.join("claude"),
            [
                "#!/bin/sh",
                "case \"$1\" in",
                "  --version) echo \"${FAKE_CLAUDE_VERSION:-2.1.296 (Claude Code)}\"; exit 0;;",
                "  agents) cat \"$CLAUDE_CONFIG_DIR/fake-agents.json\" 2>/dev/null || echo '[]'; exit 0;;",
                "esac",
                &format!("echo \"$CLAUDE_CONFIG_DIR|$*\" >> '{}'", log.display()),
                "[ \"$1\" = auth ] || exit 0",
                "[ -n \"${CLAUDE_SECURESTORAGE_CONFIG_DIR+x}${ANTHROPIC_API_KEY+x}${ANTHROPIC_AUTH_TOKEN+x}\" ] && exit 9",
                "[ -n \"${FAKE_LOGIN_FAIL+x}\" ] && exit 1",
                "[ \"$2\" = login ] || exit 0",
                "U=\"$FAKE_LOGIN_UUID\"; E=\"${FAKE_LOGIN_EXPIRES:-7}\"",
                "printf '{\"claudeAiOauth\":{\"accessToken\":\"at-%s-%s\",\"refreshToken\":\"rt-%s-%s\",\"expiresAt\":%s,\"refreshTokenExpiresAt\":5,\"subscriptionType\":\"pro\"}}' \"$U\" \"$E\" \"$U\" \"$E\" \"$E\" > \"$CLAUDE_CONFIG_DIR/.credentials.json\"",
                "printf '{\"oauthAccount\":{\"accountUuid\":\"%s\",\"organizationUuid\":\"org\"}}' \"$U\" > \"$CLAUDE_CONFIG_DIR/.claude.json\"",
            ]
            .join("\n"),
        );
        fs::write(&bin, script).unwrap();
        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
        }
        Self { bin, log }
    }

    fn ccam(&self, home: &Home, args: &[&str]) -> Command {
        let mut cmd = home.ccam(args);
        cmd.env("CCAM_CLAUDE_BIN", &self.bin)
            .env_remove("FAKE_CLAUDE_VERSION")
            .env_remove("FAKE_LOGIN_UUID")
            .env_remove("FAKE_LOGIN_EXPIRES")
            .env_remove("FAKE_LOGIN_FAIL");
        cmd
    }

    fn stdout(&self, home: &Home, args: &[&str]) -> String {
        String::from_utf8(self.ccam(home, args).assert().success().get_output().stdout.clone()).unwrap()
    }

    /// The config dir and arguments of the last interactive run.
    fn last_run(&self) -> (PathBuf, String) {
        let runs = fs::read_to_string(&self.log).unwrap();
        let (dir, args) = runs.lines().last().unwrap().split_once('|').unwrap();
        (PathBuf::from(dir.trim()), args.trim().to_string())
    }
}

/// Saves accounts a, b and c and leaves the main login on c.
fn three_accounts() -> Home {
    let home = Home::new();
    for uuid in ["A", "B", "C"] {
        home.log_in(uuid, 1);
        home.stdout(&["save", &uuid.to_ascii_lowercase()]);
    }
    home
}

fn lane_login(home: &Home, lane: &str) -> Value {
    home.read(&format!(".claude/ccam/lanes/{lane}/.credentials.json"))
}

fn switch_log(home: &Home) -> String {
    fs::read_to_string(home.claude().join("ccam/accounts/switch.log")).unwrap()
}

#[test]
fn lanes_keep_their_accounts_while_the_main_login_moves() {
    let home = three_accounts();
    let claude = FakeClaude::new(&home);
    let lanes = home.claude().join("ccam").join("lanes");

    assert!(claude.stdout(&home, &["run", "a", "--continue"]).contains("Lane 1"));
    let (dir, args) = claude.last_run();
    assert_eq!(dir, lanes.join("1"));
    assert!(args.starts_with("--settings ") && args.ends_with(" --continue"), "{args}");
    assert_eq!(lane_login(&home, "1")["claudeAiOauth"]["accessToken"], "at-A-1");
    assert_eq!(home.read(".claude/ccam/lanes/1/.claude.json")["oauthAccount"]["accountUuid"], "A");
    assert_eq!(home.read(".claude/.credentials.json")["claudeAiOauth"]["accessToken"], "at-C-1", "main is untouched");

    // Auto memory is the main login's, reached without a link so Claude Code does not ask before each write.
    let settings_file = args["--settings ".len()..args.len() - " --continue".len()].trim_matches('"');
    let settings: Value = serde_json::from_str(&fs::read_to_string(settings_file).unwrap()).unwrap();
    let memory = Path::new(settings["autoMemoryDirectory"].as_str().unwrap());
    assert!(memory.starts_with(home.claude().join("projects")) && memory.ends_with("memory"), "{settings}");
    assert!(settings["permissions"]["allow"][0].as_str().unwrap().starts_with("Read(//"), "{settings}");

    // A second account gets the next lane; an account already in a lane joins it.
    claude.ccam(&home, &["run", "b"]).assert().success();
    assert_eq!(claude.last_run().0, lanes.join("2"));
    claude.ccam(&home, &["run", "a"]).assert().success();
    assert_eq!(claude.last_run().0, lanes.join("1"));
    claude.ccam(&home, &["run", "--lane", "2"]).assert().success();
    assert_eq!(claude.last_run().0, lanes.join("2"));
    claude.ccam(&home, &["run", "b", "--", "-p", "my secret plan", "--model=opus", "notes.md"]).assert().success();
    let (dir, args) = claude.last_run();
    assert_eq!(dir, lanes.join("2"));
    let passed = ["my secret plan", "--model=opus", "notes.md"].iter().all(|a| args.contains(a));
    assert!(passed, "claude gets every argument: {args}");
    claude.ccam(&home, &["run", "--lane", "7"]).assert().code(1).stderr(contains("no lane 7"));
    claude.ccam(&home, &["run", "c"]).assert().code(1).stderr(contains("main login"));
    claude.ccam(&home, &["run", "a", "--lane", "2"]).assert().code(1).stderr(contains("lane 1"));
    claude.ccam(&home, &["run", "--lane", "007"]).assert().code(1);

    // Conversations are shared with the main config home.
    fs::write(lanes.join("1").join("projects").join("chat.jsonl"), "x").unwrap();
    assert!(home.claude().join("projects").join("chat.jsonl").exists());

    // Accounts in lanes are off limits to the main login.
    let list = home.stdout(&["list"]);
    assert!(list.contains("a  (pro)  in use on lane 1"), "{list}");
    home.ccam(&["to", "a"]).assert().code(1).stderr(contains("ccam move 1"));

    // Every lane event is in the switch log, tagged with its lane, including sessions its hooks see start.
    let hook = json!({"session_id": "s9", "cwd": "/work/api", "hook_event_name": "SessionStart", "source": "startup"});
    let lane_dir = lanes.join("1").to_string_lossy().into_owned();
    home.ccam(&["hook", "session-start", "--config-dir", &lane_dir]).write_stdin(hook.to_string()).assert().success();
    let log = switch_log(&home);
    assert!(log.contains("[lane 1] session s9 startup in /work/api"), "{log}");
    assert!(log.contains("[lane 1] switch '(logged out)' -> 'a'"), "{log}");
    assert!(log.contains("[lane 1] run in "), "{log}");
    assert!(log.contains("[lane 2] run in "), "{log}");
    // Only flag names are logged: a prompt, a flag's value or a file name could be private.
    assert!(log.contains("args [--continue], settings "), "{log}");
    assert!(log.contains("args [-p, --model], settings "), "{log}");
    for private in ["my secret plan", "opus", "notes.md"] {
        assert!(!log.contains(private), "{private} reached the switch log: {log}");
    }
}

#[test]
fn a_lane_moves_to_another_account_and_is_released_only_when_idle() {
    let home = three_accounts();
    let claude = FakeClaude::new(&home);
    claude.ccam(&home, &["run", "a"]).assert().success();

    claude.ccam(&home, &["move", "1", "b"]).assert().code(1).stderr(contains("--yes"));
    assert_eq!(lane_login(&home, "1")["claudeAiOauth"]["accessToken"], "at-A-1", "unconfirmed moves change nothing");
    let out = claude.stdout(&home, &["move", "1", "b", "--yes"]);
    assert!(out.contains("Moved lane 1 from 'a' to 'b'"), "{out}");
    assert_eq!(lane_login(&home, "1")["claudeAiOauth"]["accessToken"], "at-B-1");
    let taken =
        claude.ccam(&home, &["move", "1", "c", "--yes"]).assert().code(1).stderr(contains("in use by Claude on"));
    assert!(!String::from_utf8_lossy(&taken.get_output().stderr).contains("--force"));
    claude.ccam(&home, &["move", "1", "b"]).assert().code(1).stderr(contains("already on 'b'"));
    claude.ccam(&home, &["move", "9", "c", "--yes"]).assert().code(1).stderr(contains("no lane 9"));

    // Claude Code renews the lane's login.
    let lane = home.claude().join("ccam").join("lanes").join("1");
    let mut renewed = lane_login(&home, "1");
    renewed["claudeAiOauth"]["accessToken"] = "at-B-2".into();
    renewed["claudeAiOauth"]["refreshToken"] = "rt-B-2".into();
    renewed["claudeAiOauth"]["expiresAt"] = 2.into();
    fs::write(lane.join(".credentials.json"), renewed.to_string()).unwrap();

    // A running session keeps the lane.
    let session =
        json!([{"kind": "interactive", "pid": 4242, "sessionId": "s1", "cwd": "/work/api", "status": "busy"}]);
    fs::write(lane.join("fake-agents.json"), session.to_string()).unwrap();
    let sessions = claude.stdout(&home, &["sessions"]);
    assert!(sessions.contains("lane 1") && sessions.contains("'b'") && sessions.contains("/work/api"), "{sessions}");
    assert!(sessions.contains("main") && sessions.contains("'c'"), "{sessions}");
    claude.ccam(&home, &["release", "1"]).assert().code(1).stderr(contains("--force"));

    let out = claude.stdout(&home, &["release", "1", "--force"]);
    assert!(out.contains("Released 'b' from lane 1"), "{out}");
    assert!(lane_login(&home, "1").get("claudeAiOauth").is_none());
    claude.ccam(&home, &["run", "--lane", "1"]).assert().code(1).stderr(contains("holds no account"));

    // The renewal made in the lane was saved, so the main login can take the account without /login.
    home.stdout(&["to", "b"]);
    assert_eq!(home.read(".claude/.credentials.json")["claudeAiOauth"]["accessToken"], "at-B-2");

    // A released lane can take an account again, and a lane can be named.
    claude.ccam(&home, &["run", "a", "--lane", "api"]).assert().success();
    assert_eq!(lane_login(&home, "api")["claudeAiOauth"]["accessToken"], "at-A-1");
    let log = switch_log(&home);
    assert!(log.contains("[lane 1] release 'b' (--force, 1 running session)"), "{log}");

    // Prune removes empty lane 1, while active lane api is kept.
    assert!(lane.exists());
    let prune_out = home.stdout(&["prune"]);
    assert!(prune_out.contains("Removed empty lane 1"), "{prune_out}");
    assert!(!lane.exists());
    assert!(home.claude().join("ccam").join("lanes").join("api").exists());

    // Release with --delete releases and removes the lane folder.
    let rel_out = home.stdout(&["release", "api", "--delete"]);
    assert!(rel_out.contains("Released 'a' from lane api"), "{rel_out}");
    assert!(rel_out.contains("Removed lane api"), "{rel_out}");
    assert!(!home.claude().join("ccam").join("lanes").join("api").exists());
}

#[test]
fn lanes_refuse_a_claude_code_too_old_to_share_history_safely() {
    let home = three_accounts();
    let claude = FakeClaude::new(&home);

    let old = claude.ccam(&home, &["run", "a"]).env("FAKE_CLAUDE_VERSION", "2.1.279 (Claude Code)").assert();
    old.code(1).stderr(contains("2.1.296"));
    assert!(!home.claude().join("ccam").join("lanes").exists());
}

/// Answers the profile endpoint with the account named by the bearer token, `at-<uuid>-<n>`, as Anthropic does
/// for a token it issued. Returns the base URL for `CCAM_API_BASE`.
fn profile_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(&stream);
            let mut request = String::new();
            let mut uuid = None;
            for _ in 0..64 {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                    break;
                }
                if let Some((key, value)) = line.split_once(':')
                    && key.eq_ignore_ascii_case("authorization")
                {
                    uuid = value
                        .trim()
                        .strip_prefix("Bearer at-")
                        .and_then(|t| t.rsplit_once('-'))
                        .map(|(u, _)| u.to_string());
                } else if request.is_empty() {
                    request = line;
                }
            }
            let (status, body) = match uuid {
                Some(uuid) if request.starts_with("GET /api/oauth/profile ") => {
                    ("200 OK", json!({"account": {"uuid": uuid}}).to_string())
                }
                _ => ("404 Not Found", "{}".to_string()),
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    base
}

#[test]
fn login_adds_an_account_without_touching_the_main_login() {
    let home = three_accounts();
    let claude = FakeClaude::new(&home);
    let root = home.claude().join("ccam").join("login");
    let leftovers = || fs::read_dir(&root).map(|d| d.count()).unwrap_or(0);
    fs::create_dir_all(root.join("old")).unwrap();
    fs::write(root.join("old").join(".credentials.json"), "{}").unwrap();
    fs::create_dir_all(root.join("old.lock")).unwrap();
    let api = profile_server();
    let login = |name: &str, uuid: &str| {
        let mut cmd = claude.ccam(&home, &["login", name]);
        cmd.env("FAKE_LOGIN_UUID", uuid).env("CCAM_API_BASE", &api);
        cmd
    };

    let out = login("fresh", "N")
        .env("CLAUDE_SECURESTORAGE_CONFIG_DIR", home.claude())
        .env("ANTHROPIC_API_KEY", "sk-x")
        .env("ANTHROPIC_AUTH_TOKEN", "t-x")
        .assert()
        .success();
    let out = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    assert!(out.contains("Saved 'fresh'"), "{out}");
    assert_eq!(home.read(".claude/ccam/accounts/fresh/login.json")["claudeAiOauth"]["accessToken"], "at-N-7");
    assert_eq!(home.read(".claude/.credentials.json")["claudeAiOauth"]["accessToken"], "at-C-1", "main is untouched");
    assert_eq!(home.read(".claude.json")["oauthAccount"]["accountUuid"], "C");
    assert_eq!(leftovers(), 0, "the sign-in dir and the crashed run's leftovers are deleted");
    assert!(switch_log(&home).contains("login 'fresh' (new)"));

    login("third", "A").assert().code(1).stderr(contains("private window"));
    assert!(!home.claude().join("ccam/accounts/third").exists());
    assert_eq!(leftovers(), 0);

    login("c", "C").assert().code(1).stderr(contains("in use on the main login"));
    assert_eq!(home.read(".claude/ccam/accounts/c/login.json")["claudeAiOauth"]["accessToken"], "at-C-1");

    let out = login("a", "A").env("FAKE_LOGIN_EXPIRES", "40").assert().success();
    assert!(String::from_utf8_lossy(&out.get_output().stdout).contains("Refreshed"));
    assert_eq!(home.read(".claude/ccam/accounts/a/login.json")["claudeAiOauth"]["expiresAt"], 40);

    login("other", "O").env("FAKE_LOGIN_FAIL", "1").assert().code(1).stderr(contains("did not finish"));
    assert!(!home.claude().join("ccam/accounts/other").exists());
    assert_eq!(leftovers(), 0);

    let runs = fs::read_to_string(&claude.log).unwrap();
    assert_eq!(runs.matches("|auth login").count(), 4, "an account in use is refused before signing in: {runs}");
    assert!(!runs.contains("logout"), "{runs}");
}

#[test]
fn login_saves_nothing_when_the_sign_in_cannot_be_confirmed() {
    let home = three_accounts();
    let claude = FakeClaude::new(&home);

    let refused = claude.ccam(&home, &["login", "x"]).env("FAKE_LOGIN_UUID", "X").assert();
    refused.code(3).stderr(contains("could not confirm which account you signed in to"));

    assert!(!home.claude().join("ccam/accounts/x").exists());
    let root = home.claude().join("ccam").join("login");
    assert_eq!(fs::read_dir(&root).map(|d| d.count()).unwrap_or(0), 0, "the sign-in dir is deleted");
    assert!(fs::read_to_string(&claude.log).unwrap().contains("|auth login"), "the sign-in ran");
}

#[test]
fn a_second_login_is_refused_while_one_is_signing_in_and_leaves_its_dir_alone() {
    let home = three_accounts();
    let claude = FakeClaude::new(&home);
    let ccam_dir = home.claude().join("ccam");
    // A live holder's lock: just created, so its mtime is fresh.
    fs::create_dir_all(ccam_dir.join("login.lock")).unwrap();
    let running = ccam_dir.join("login").join("1-1");
    fs::create_dir_all(&running).unwrap();
    fs::write(running.join(".credentials.json"), "{}").unwrap();

    let mut second = claude.ccam(&home, &["login", "x"]);
    second.env("FAKE_LOGIN_UUID", "X").env("CCAM_API_BASE", profile_server());
    second.assert().code(1).stderr(contains("another ccam login is signing in"));

    assert!(running.join(".credentials.json").exists(), "the running sign-in's dir is not swept");
    assert!(!ccam_dir.join("accounts/x").exists());
    assert!(!claude.log.exists(), "no sign-in was started");
    assert!(ccam_dir.join("login.lock").exists(), "the holder's lock is left alone");

    // A crashed login's lock stops being touched, and is taken over once stale.
    filetime::set_file_mtime(ccam_dir.join("login.lock"), filetime::FileTime::from_unix_time(0, 0)).unwrap();
    let mut after_crash = claude.ccam(&home, &["login", "x"]);
    after_crash.env("FAKE_LOGIN_UUID", "X").env("CCAM_API_BASE", profile_server());
    after_crash.assert().success().stdout(contains("Saved 'x'"));
    assert!(!running.exists(), "the crashed run's dir is swept");
    assert!(!ccam_dir.join("login.lock").exists(), "the lock is released");
}

#[test]
fn login_refuses_an_account_in_use_before_opening_the_browser() {
    let home = three_accounts();
    let claude = FakeClaude::new(&home);
    claude.stdout(&home, &["lane", "a"]);

    for (name, place) in [("c", "the main login"), ("A", "lane 1")] {
        let refused = claude.ccam(&home, &["login", name]).env("FAKE_LOGIN_UUID", name.to_ascii_uppercase()).assert();
        refused.code(1).stderr(contains(format!("in use on {place}")));
    }
    let runs = fs::read_to_string(&claude.log).unwrap_or_default();
    assert!(!runs.contains("auth"), "no sign-in was started: {runs}");
}

#[test]
fn lane_checks_an_account_out_without_launching_claude_and_refuses_what_run_refuses() {
    let home = three_accounts();
    let claude = FakeClaude::new(&home);
    let lanes = home.claude().join("ccam").join("lanes");

    let out = claude.stdout(&home, &["lane", "a"]);
    assert!(out.contains("Lane 1 holds 'a'."), "{out}");
    assert!(out.contains("T3 Code not found; skipped adding the lane to T3."), "{out}");
    assert!(!home.t3().exists());
    assert_eq!(lane_login(&home, "1")["claudeAiOauth"]["accessToken"], "at-A-1");
    assert_eq!(home.read(".claude/.credentials.json")["claudeAiOauth"]["accessToken"], "at-C-1", "main is untouched");
    assert!(lanes.join("1").join("projects").is_dir(), "the lane is prepared as for run");

    claude.ccam(&home, &["lane", "c"]).assert().code(1).stderr(contains("main login"));
    claude.ccam(&home, &["lane", "a", "--lane", "2"]).assert().code(1).stderr(contains("lane 1"));
    claude.ccam(&home, &["lane", "b", "--lane", "1"]).assert().code(1).stderr(contains("ccam move 1 b"));
    let in_lane = claude.ccam(&home, &["lane", "b"]).env("CLAUDE_CONFIG_DIR", lanes.join("1")).assert();
    in_lane.code(1).stderr(contains("in lane 1"));
    assert!(!lanes.join("2").exists(), "a refused lane command creates no lane");

    // T3 runs commands inside Claude Code, so unlike run, lane works there.
    let out = claude.ccam(&home, &["lane", "b"]).env("CLAUDE_PID", "1").assert().success();
    assert!(String::from_utf8_lossy(&out.get_output().stdout).contains("Lane 2 holds 'b'."));
    assert!(claude.stdout(&home, &["lane", "a"]).contains("Lane 1 holds 'a'."), "an account's lane is reused");

    assert!(!claude.log.exists(), "ccam lane never starts claude");
}

#[test]
fn lanes_appear_in_t3_code_and_follow_moves_releases_and_deletes() {
    let home = three_accounts();
    let claude = FakeClaude::new(&home);
    let lanes = home.claude().join("ccam").join("lanes");
    let userdata = home.t3().join("userdata");
    fs::create_dir_all(&userdata).unwrap();
    let file = userdata.join("settings.json");
    let user =
        json!({"providerInstances": {"codex": {"driver": "codex", "enabled": true}}, "projectSettingsFolded": true});
    fs::write(&file, user.to_string()).unwrap();
    let t3 = || -> Value { serde_json::from_slice(&fs::read(&file).unwrap()).unwrap() };
    let ids = || -> Vec<String> { t3()["providerInstances"].as_object().unwrap().keys().cloned().collect() };

    let out = claude.stdout(&home, &["lane", "a"]);
    assert!(out.contains("Lane 1 holds 'a'."), "{out}");
    assert!(out.contains("T3 Code: lane 1 is in the model picker as 'Claude · lane 1 · a'."), "{out}");
    let doc = t3();
    let lane1 = &doc["providerInstances"]["ccam-lane-1"];
    assert_eq!(lane1["driver"], "claudeAgent");
    assert_eq!(lane1["displayName"], "Claude · lane 1 · a");
    assert_eq!(lane1["enabled"], true);
    assert_eq!(Path::new(lane1["config"]["homePath"].as_str().unwrap()), lanes.join("1"));
    assert_eq!(doc["providerInstances"]["codex"], user["providerInstances"]["codex"]);
    assert_eq!(doc["projectSettingsFolded"], true);
    assert_eq!(doc.as_object().unwrap().keys().collect::<Vec<_>>(), ["providerInstances", "projectSettingsFolded"]);

    let written = fs::read(&file).unwrap();
    assert!(claude.stdout(&home, &["lane", "a"]).contains("model picker"));
    assert_eq!(fs::read(&file).unwrap(), written, "an unchanged lane does not rewrite T3's settings");

    claude.stdout(&home, &["move", "1", "b", "--yes"]);
    assert_eq!(t3()["providerInstances"]["ccam-lane-1"]["displayName"], "Claude · lane 1 · b");

    claude.stdout(&home, &["release", "1"]);
    let released = t3()["providerInstances"]["ccam-lane-1"].clone();
    assert_eq!(
        (released["displayName"].as_str(), released["enabled"].as_bool()),
        (Some("Claude · lane 1"), Some(false))
    );

    claude.stdout(&home, &["release", "1", "--delete"]);
    assert_eq!(ids(), ["codex"]);

    // run adds its lane too, and prune takes a removed lane's instance away.
    claude.ccam(&home, &["run", "a", "--lane", "api"]).assert().success();
    assert_eq!(t3()["providerInstances"]["ccam-lane-api"]["displayName"], "Claude · lane api · a");
    claude.stdout(&home, &["release", "api"]);
    assert!(claude.stdout(&home, &["prune"]).contains("Removed empty lane api"));
    assert_eq!(ids(), ["codex"]);
    assert!(switch_log(&home).contains("t3: updated"));

    // A settings file T3 itself could not read is left alone, and the lane command still succeeds.
    fs::write(&file, "{not json").unwrap();
    let out = claude.ccam(&home, &["lane", "a"]).assert().success();
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
    assert!(stderr.contains("could not update T3 Code"), "{stderr}");
    assert_eq!(fs::read_to_string(&file).unwrap(), "{not json");
    assert!(switch_log(&home).lines().any(|l| l.contains("t3: ") && l.contains("invalid JSON")));
}

#[test]
fn a_lane_session_start_refreshes_the_lanes_copy_of_the_main_settings() {
    let home = three_accounts();
    let claude = FakeClaude::new(&home);
    fs::write(home.claude().join("settings.json"), r#"{"model":"opus"}"#).unwrap();
    claude.stdout(&home, &["lane", "a"]);
    assert_eq!(home.read(".claude/ccam/lanes/1/settings.json")["model"], "opus");

    fs::write(home.claude().join("settings.json"), r#"{"model":"sonnet"}"#).unwrap();
    fs::write(home.claude().join("keybindings.json"), "[]").unwrap();
    let lane = home.claude().join("ccam").join("lanes").join("1").to_string_lossy().into_owned();
    let hook = json!({"session_id": "s1", "cwd": "/w", "hook_event_name": "SessionStart", "source": "startup"});
    let out = home.ccam(&["hook", "session-start", "--config-dir", &lane]).write_stdin(hook.to_string()).assert();
    let out: Value = serde_json::from_slice(&out.success().get_output().stdout).unwrap();
    assert!(out["hookSpecificOutput"]["watchPaths"][0].as_str().unwrap().ends_with(".credentials.json"));

    let settings = home.read(".claude/ccam/lanes/1/settings.json");
    assert_eq!(settings["model"], "sonnet");
    let args = settings["hooks"]["SessionStart"][0]["hooks"][0]["args"].to_string();
    assert!(args.contains("--config-dir") && args.contains("lanes"), "{args}");
    assert_eq!(fs::read_to_string(home.claude().join("ccam/lanes/1/keybindings.json")).unwrap(), "[]");
    assert!(switch_log(&home).contains("[lane 1] settings refreshed"));
}
