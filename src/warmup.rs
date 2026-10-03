//! `ccam start`: open the 5-hour usage window on every saved account with one minimal message each.
//!
//! Each message goes through the official `claude` binary in a throwaway config dir, with the account's access
//! token passed in the environment for that one run. The live login is never switched, nothing is written to
//! the real config, no session is saved, and Claude Code's own context (system prompt, tools, skills, CLAUDE.md,
//! MCP) is stripped so a message costs a few hundred tokens instead of thousands.
//!
//! Anthropic's current pages do not document when a window starts, so success is judged by re-reading the usage
//! endpoint, not assumed. Sending inside a running window does not move its reset time, so those accounts are
//! skipped before anything is sent.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use jiff::Timestamp;
use serde_json::Value;

use crate::error::Result;
use crate::login::UnixMillis;
use crate::manager::{LiveState, Manager};
use crate::name::AccountName;
use crate::vault::Account;

/// The usage endpoint reports a new window a few seconds after the message.
pub const SETTLE: Duration = Duration::from_secs(8);
const SETTLE_RETRIES: u32 = 4;
const CLAUDE_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// A window was already running; nothing was sent.
    AlreadyRunning {
        resets_at: Timestamp,
    },
    /// One message was sent and a window is now running.
    Started {
        resets_at: Timestamp,
        cost_usd: Option<f64>,
    },
    /// The message was sent but no window showed up within the settle time.
    Unconfirmed {
        cost_usd: Option<f64>,
    },
    /// Nothing was sent.
    Skipped(String),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub name: AccountName,
    pub outcome: Outcome,
}

/// Runs the minimal `claude -p` for one account. A trait so the orchestration can be tested without Claude Code.
pub trait Sender: Send + Sync {
    fn send(&self, access_token: &str) -> std::result::Result<SendResult, String>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct SendResult {
    pub reply: String,
    pub cost_usd: Option<f64>,
}

/// The window reported by the usage endpoint, if one is running.
fn running_window(usage: &Value) -> Option<Timestamp> {
    usage["five_hour"]["resets_at"].as_str().and_then(|s| s.parse().ok())
}

pub fn start(manager: &Manager, sender: &dyn Sender, refresh: bool, settle: Duration) -> Result<Vec<Report>> {
    let overview = manager.overview()?;
    let live_name = overview.live_account().map(|a| a.name.clone());
    let live_login = match &overview.live {
        LiveState::Present(live) => Some(live.login.clone()),
        _ => None,
    };
    let now = UnixMillis::now();

    let mut reports = Vec::new();
    for account in &overview.listing.accounts {
        let is_live = live_name.as_ref() == Some(&account.name);
        let outcome = start_one(manager, sender, account, is_live, live_login.as_ref(), refresh, now, settle);
        manager.log(&format!("start '{}': {}", account.name, describe(&outcome)));
        reports.push(Report { name: account.name.clone(), outcome });
    }
    Ok(reports)
}

#[allow(clippy::too_many_arguments)]
fn start_one(
    manager: &Manager,
    sender: &dyn Sender,
    account: &Account,
    is_live: bool,
    live_login: Option<&crate::login::OAuthLogin>,
    refresh: bool,
    now: UnixMillis,
    settle: Duration,
) -> Outcome {
    // The live account uses Claude Code's own token and is never renewed here.
    let login =
        if is_live { live_login.cloned().unwrap_or_else(|| account.login.clone()) } else { account.login.clone() };
    let token = if login.usable_at(now) {
        login.access_token().to_string()
    } else if is_live {
        return Outcome::Skipped("access token expired; send any message in Claude to renew it".into());
    } else if !refresh {
        return Outcome::Skipped("saved login expired; run with --refresh to renew it".into());
    } else {
        match manager.refresh_saved(&account.name, now) {
            Ok(refreshed) => refreshed.login.access_token().to_string(),
            Err(e) => return Outcome::Failed(e.to_string()),
        }
    };

    let usage = match manager.client().usage(&token) {
        Ok(usage) => usage,
        Err(e) => return Outcome::Failed(format!("could not read usage: {e}")),
    };
    if let Some(resets_at) = running_window(&usage) {
        return Outcome::AlreadyRunning { resets_at };
    }

    let sent = match sender.send(&token) {
        Ok(sent) => sent,
        Err(e) => return Outcome::Failed(e),
    };

    for _ in 0..SETTLE_RETRIES {
        thread::sleep(settle);
        if let Ok(usage) = manager.client().usage(&token)
            && let Some(resets_at) = running_window(&usage)
        {
            return Outcome::Started { resets_at, cost_usd: sent.cost_usd };
        }
    }
    Outcome::Unconfirmed { cost_usd: sent.cost_usd }
}

pub fn describe(outcome: &Outcome) -> String {
    let until = |t: &Timestamp| t.to_zoned(jiff::tz::TimeZone::system()).strftime("%H:%M").to_string();
    let cost = |c: &Option<f64>| c.map(|c| format!(", ${c:.4}")).unwrap_or_default();
    match outcome {
        Outcome::AlreadyRunning { resets_at } => format!("window already running, resets {}", until(resets_at)),
        Outcome::Started { resets_at, cost_usd } => format!("started, resets {}{}", until(resets_at), cost(cost_usd)),
        Outcome::Unconfirmed { cost_usd } => {
            format!("message sent but no window reported yet{}; check: ccam usage", cost(cost_usd))
        }
        Outcome::Skipped(reason) => format!("skipped: {reason}"),
        Outcome::Failed(reason) => format!("failed: {reason}"),
    }
}

/// Sends through the real `claude` binary.
pub struct ClaudeSender {
    claude: PathBuf,
}

impl ClaudeSender {
    pub fn new() -> Self {
        Self { claude: crate::launch::claude_binary() }
    }
}

impl Default for ClaudeSender {
    fn default() -> Self {
        Self::new()
    }
}

/// Environment variables that could outrank the per-run token, redirect the request to another provider, or
/// make Claude Code believe it is a nested session.
fn is_hijacking_env(key: &str) -> bool {
    let k = key.to_ascii_uppercase();
    k.starts_with("ANTHROPIC_")
        || k.starts_with("CLAUDE")
        || k.starts_with("AWS_")
        || k.contains("VERTEX")
        || k.contains("BEDROCK")
        || k.contains("FOUNDRY")
}

impl Sender for ClaudeSender {
    fn send(&self, access_token: &str) -> std::result::Result<SendResult, String> {
        let scratch = tempfile::Builder::new().prefix("ccam-start-").tempdir().map_err(|e| e.to_string())?;
        let config = scratch.path().join("config");
        let cwd = scratch.path().join("cwd");
        std::fs::create_dir_all(&config).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&cwd).map_err(|e| e.to_string())?;
        // Without this, `claude` opens its first-run onboarding instead of answering.
        std::fs::write(config.join(".claude.json"), r#"{"hasCompletedOnboarding":true}"#).map_err(|e| e.to_string())?;

        let mut command = Command::new(&self.claude);
        command.args([
            "-p",
            "hi",
            "--model",
            "haiku",
            "--system-prompt",
            "Reply with one word.",
            "--tools",
            "",
            "--strict-mcp-config",
            "--disable-slash-commands",
            "--max-turns",
            "1",
            "--no-session-persistence",
            "--output-format",
            "json",
        ]);
        command.current_dir(&cwd).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        for (key, _) in std::env::vars_os() {
            if is_hijacking_env(&key.to_string_lossy()) {
                command.env_remove(key);
            }
        }
        command
            .env("CLAUDE_CONFIG_DIR", &config)
            .env("CLAUDE_CODE_OAUTH_TOKEN", access_token)
            .env("CLAUDE_CODE_DISABLE_CLAUDE_MDS", "1")
            .env("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1")
            .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1");

        let output = run_with_timeout(command, CLAUDE_TIMEOUT)?;
        let body: Value = serde_json::from_slice(&output).map_err(|_| "claude did not return JSON".to_string())?;
        if body["is_error"].as_bool() == Some(true) {
            return Err(body["result"].as_str().unwrap_or("claude reported an error").to_string());
        }
        Ok(SendResult {
            reply: body["result"].as_str().unwrap_or_default().to_string(),
            cost_usd: body["total_cost_usd"].as_f64(),
        })
    }
}

fn run_with_timeout(mut command: Command, timeout: Duration) -> std::result::Result<Vec<u8>, String> {
    let mut child = command.spawn().map_err(|e| format!("cannot start claude: {e}"))?;
    let stdout = child.stdout.take().expect("stdout is piped");
    let reader = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut std::io::BufReader::new(stdout), &mut buf);
        buf
    });
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let out = reader.join().unwrap_or_default();
                if status.success() || !out.is_empty() {
                    return Ok(out);
                }
                return Err(format!("claude exited with {status}"));
            }
            Ok(None) if started.elapsed() > timeout => {
                let _ = child.kill();
                return Err("claude did not finish in time".into());
            }
            Ok(None) => thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::{Arc, Mutex};

    use serde_json::json;

    use super::*;
    use crate::api::{ApiError, OAuthClient};
    use crate::login::OAuthLogin;
    use crate::paths::Paths;
    use crate::store::FileStore;

    /// Usage per token, mutable so a test can make a window appear after a send.
    #[derive(Default)]
    struct FakeApi {
        windows: Mutex<std::collections::HashMap<String, Option<String>>>,
    }

    impl OAuthClient for Arc<FakeApi> {
        fn account_uuid(&self, access_token: &str) -> std::result::Result<String, ApiError> {
            Ok(access_token.split('-').nth(1).unwrap_or_default().to_string())
        }
        fn refresh(&self, login: &OAuthLogin) -> std::result::Result<OAuthLogin, ApiError> {
            let fresh = format!("fresh-{}", login.access_token().split('-').nth(1).unwrap_or_default());
            Ok(login.refreshed(&fresh, Some("rt-fresh"), UnixMillis::now().plus(Duration::from_secs(3600))))
        }
        fn usage(&self, access_token: &str) -> std::result::Result<Value, ApiError> {
            let windows = self.windows.lock().unwrap();
            let resets_at = windows.get(access_token).cloned().flatten();
            Ok(json!({"five_hour": {"utilization": 0, "resets_at": resets_at}}))
        }
    }

    /// Records sends; each send makes a window appear for that token.
    struct FakeSender {
        api: Arc<FakeApi>,
        sent: Mutex<Vec<String>>,
        starts_window: bool,
    }

    impl Sender for FakeSender {
        fn send(&self, access_token: &str) -> std::result::Result<SendResult, String> {
            self.sent.lock().unwrap().push(access_token.to_string());
            if self.starts_window {
                self.api.windows.lock().unwrap().insert(access_token.to_string(), Some("2026-09-19T16:09:59Z".into()));
            }
            Ok(SendResult { reply: "Hi".into(), cost_usd: Some(0.0005) })
        }
    }

    struct Setup {
        _home: tempfile::TempDir,
        manager: Manager,
        api: Arc<FakeApi>,
    }

    /// Saves `(name, uuid, expires_at)` accounts; the first stays logged in.
    fn setup(accounts: &[(&str, &str, i64)]) -> Setup {
        let home = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(home.path(), None);
        fs::create_dir_all(&paths.config_home).unwrap();
        let api = Arc::new(FakeApi::default());
        let store = Box::new(FileStore::new(paths.credentials_file.clone()));
        let manager = Manager::new(paths.clone(), store, Box::new(Arc::clone(&api)));
        for (name, uuid, expires_at) in accounts.iter().rev() {
            let login = json!({"accessToken": format!("at-{uuid}"), "refreshToken": format!("rt-{uuid}"), "expiresAt": expires_at, "refreshTokenExpiresAt": 1});
            fs::write(&paths.credentials_file, json!({"claudeAiOauth": login}).to_string()).unwrap();
            fs::write(&paths.global_config, json!({"oauthAccount": {"accountUuid": uuid}}).to_string()).unwrap();
            manager.save(&name.parse().unwrap()).unwrap();
        }
        Setup { _home: home, manager, api }
    }

    fn future() -> i64 {
        UnixMillis::now().plus(Duration::from_secs(3600)).as_millis()
    }

    fn sender(s: &Setup, starts_window: bool) -> FakeSender {
        FakeSender { api: Arc::clone(&s.api), sent: Mutex::new(Vec::new()), starts_window }
    }

    #[test]
    fn sends_once_per_account_without_a_window_and_reports_the_new_reset() {
        let s = setup(&[("live", "L", future()), ("other", "O", future())]);
        let sender = sender(&s, true);

        let reports = start(&s.manager, &sender, false, Duration::ZERO).unwrap();

        assert_eq!(sender.sent.lock().unwrap().as_slice(), ["at-L", "at-O"]);
        for r in &reports {
            assert!(matches!(r.outcome, Outcome::Started { cost_usd: Some(_), .. }), "{r:?}");
        }
    }

    #[test]
    fn accounts_with_a_running_window_are_not_sent_to() {
        let s = setup(&[("live", "L", future()), ("other", "O", future())]);
        s.api.windows.lock().unwrap().insert("at-O".into(), Some("2026-09-19T16:09:59Z".into()));
        let sender = sender(&s, true);

        let reports = start(&s.manager, &sender, false, Duration::ZERO).unwrap();

        assert_eq!(sender.sent.lock().unwrap().as_slice(), ["at-L"]);
        assert!(matches!(reports[1].outcome, Outcome::AlreadyRunning { .. }));
    }

    #[test]
    fn expired_saved_logins_are_skipped_unless_refresh_is_allowed_and_the_live_one_never_renewed() {
        let s = setup(&[("live", "L", 1), ("other", "O", 1)]);
        let sender = sender(&s, true);

        let reports = start(&s.manager, &sender, false, Duration::ZERO).unwrap();
        assert!(matches!(&reports[0].outcome, Outcome::Skipped(m) if m.contains("send any message")));
        assert!(matches!(&reports[1].outcome, Outcome::Skipped(m) if m.contains("--refresh")));
        assert!(sender.sent.lock().unwrap().is_empty());

        let reports = start(&s.manager, &sender, true, Duration::ZERO).unwrap();
        assert!(matches!(&reports[0].outcome, Outcome::Skipped(_)));
        assert!(matches!(&reports[1].outcome, Outcome::Started { .. }));
        assert_eq!(sender.sent.lock().unwrap().as_slice(), ["fresh-O"]);
    }

    #[test]
    fn a_send_that_starts_no_window_is_reported_as_unconfirmed() {
        let s = setup(&[("live", "L", future())]);
        let sender = sender(&s, false);

        let reports = start(&s.manager, &sender, false, Duration::ZERO).unwrap();

        assert!(matches!(reports[0].outcome, Outcome::Unconfirmed { .. }));
    }

    #[test]
    fn hijacking_variables_are_recognised() {
        for k in ["ANTHROPIC_API_KEY", "CLAUDE_CONFIG_DIR", "CLAUDECODE", "AWS_PROFILE", "CLAUDE_CODE_USE_VERTEX"] {
            assert!(is_hijacking_env(k), "{k}");
        }
        assert!(!is_hijacking_env("PATH"));
    }
}
