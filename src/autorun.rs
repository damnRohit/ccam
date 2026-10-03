//! The `ccam auto` loop: reads Claude Code's hook signals, checks usage on the adaptive schedule, keeps the other
//! accounts' usage cached once armed, and acts on `auto::decide`.
//!
//! One thread, a sleep between ticks, blocking HTTP only when the schedule says so. Idle Claude Code means no
//! turn-end signals and a 30-minute check at most.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use jiff::Timestamp;
use serde_json::{Value, json};

use crate::api::ApiError;
use crate::auto::{self, Decision, Mode, Reason, Schedule, Settings, State, Usage};
use crate::error::{Error, Result};
use crate::jsonio::{self, Style};
use crate::lock::{self, LockGuard, LockTiming};
use crate::login::UnixMillis;
use crate::manager::{LiveState, Manager};
use crate::name::AccountName;
use crate::notify::Notifier;

pub const TICK: Duration = Duration::from_secs(5);
/// Candidates are re-read at most this often while armed.
const CANDIDATE_REFRESH: Duration = Duration::from_secs(10 * 60);
/// A switch target's reading must be at least this fresh; older readings are re-read first.
const TARGET_FRESHNESS: Duration = Duration::from_secs(5 * 60);

/// Files the loop and the hooks share, in the vault under this side's key: Windows and WSL share the vault, but
/// each side's Claude Code has its own turns, limits and loop. `.auto` starts with '.', which is not a valid
/// account name, so the vault never mistakes it for an account.
pub struct Files {
    pub turn_end: PathBuf,
    pub limit_hit: PathBuf,
    pub stop: PathBuf,
    pub status: PathBuf,
    pub lock: PathBuf,
}

impl Files {
    pub fn of(manager: &Manager) -> Self {
        let dir = manager.paths().vault_dir.join(".auto").join(manager.side().key());
        Self {
            turn_end: dir.join("turn-end"),
            limit_hit: dir.join("limit-hit"),
            stop: dir.join("stop"),
            status: dir.join("status.json"),
            lock: dir.join("lock"),
        }
    }
}

/// Marks a hook event by touching its file. Called from the hooks, so it only writes a timestamp.
pub fn mark(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, Timestamp::now().to_string())
}

/// What one tick did, for tests and the log.
#[derive(Debug, Clone, PartialEq)]
pub enum Tick {
    Quiet,
    Switched { from: AccountName, to: AccountName, reason: Reason },
    Advised { to: AccountName },
    Exhausted,
    Failed(String),
}

pub struct Runner<'a> {
    manager: &'a Manager,
    notifier: &'a dyn Notifier,
    settings: Settings,
    files: Files,
    state: State,
    schedule: Schedule,
    live: Option<(AccountName, Usage)>,
    cache: HashMap<String, Usage>,
    seen_turn_end: Option<SystemTime>,
    seen_limit_hit: Option<SystemTime>,
    turn_since_poll: bool,
    turn_since_cross: bool,
    /// A limit signal arrived and has not been answered by a usage check yet.
    limit_since_poll: bool,
    limit_hit: bool,
    exhausted_noted: bool,
    requests: Vec<Timestamp>,
}

impl<'a> Runner<'a> {
    pub fn new(manager: &'a Manager, notifier: &'a dyn Notifier, settings: Settings) -> Self {
        let files = Files::of(manager);
        // Signals from before the loop started are history, not news.
        let seen_turn_end = modified(&files.turn_end);
        let seen_limit_hit = modified(&files.limit_hit);
        Self {
            manager,
            notifier,
            settings,
            files,
            state: State::default(),
            schedule: Schedule::default(),
            live: None,
            cache: HashMap::new(),
            seen_turn_end,
            seen_limit_hit,
            turn_since_poll: false,
            turn_since_cross: false,
            limit_since_poll: false,
            limit_hit: false,
            exhausted_noted: false,
            requests: Vec::new(),
        }
    }

    pub fn tick(&mut self, now: Timestamp) -> Tick {
        self.read_signals();
        let overview = match self.manager.overview() {
            Ok(o) => o,
            Err(e) => return Tick::Failed(e.to_string()),
        };
        let (live_name, live_login) = match (&overview.live, overview.live_account()) {
            (LiveState::Present(live), Some(account)) => (account.name.clone(), live.login.clone()),
            _ => {
                self.live = None;
                self.write_status(now, "logged-in account is not saved");
                return Tick::Quiet;
            }
        };
        // A manual switch since the last tick makes the cached reading someone else's.
        if self.live.as_ref().is_some_and(|(name, _)| name != &live_name) {
            self.live = None;
        }

        let level = self.live.as_ref().map(|(_, u)| u.level(now));
        let due = self.limit_since_poll || self.schedule.due(now, level, self.turn_since_poll, &self.settings);
        if due && live_login.usable_at(UnixMillis::from_millis(now.as_millisecond())) {
            self.limit_since_poll = false;
            match self.fetch(live_login.access_token(), now) {
                Ok(usage) => {
                    self.live = Some((live_name.clone(), usage));
                    self.schedule.polled(now);
                    self.turn_since_poll = false;
                }
                Err(ApiError::RateLimited) => self.schedule.rate_limited(now),
                Err(e) => {
                    // Counts as a check for spacing, so an unreachable endpoint is not hammered every tick.
                    self.schedule.polled(now);
                    let failure = format!("reading usage of '{live_name}': {e}");
                    self.write_status(now, &failure);
                    return Tick::Failed(failure);
                }
            }
        }
        let Some((_, live_usage)) = self.live.clone() else {
            self.write_status(now, "waiting for a usage reading");
            return Tick::Quiet;
        };

        // Accounts another install (Windows or WSL) is using are not candidates: switching to one is refused,
        // and renewing its saved copy would log that install out.
        let busy: Vec<_> = self.manager.marks().into_iter().map(|m| m.identity).collect();
        let accounts: Vec<_> =
            overview.listing.accounts.iter().filter(|a| !busy.contains(&a.identity)).cloned().collect();
        let level = live_usage.level(now);
        if level >= self.settings.arm_at || self.limit_hit {
            self.refresh_stalest_candidate(&accounts, &live_name, now, CANDIDATE_REFRESH);
        }
        let candidates: Vec<(AccountName, Usage)> = accounts
            .iter()
            .filter(|a| a.name != live_name)
            .filter_map(|a| self.cache.get(a.name.as_str()).map(|u| (a.name.clone(), u.clone())))
            .collect();

        let inputs = auto::Inputs {
            now,
            live: &live_usage,
            candidates: &candidates,
            limit_hit: self.limit_hit,
            turn_ended: self.turn_since_cross,
        };
        let decision = auto::decide(&inputs, &mut self.state, &self.settings);
        let tick = self.act(&decision, &live_name, &live_usage, &overview.listing.accounts, now);
        self.write_status(now, &describe(&decision));
        tick
    }

    fn act(
        &mut self,
        decision: &Decision,
        live_name: &AccountName,
        live_usage: &Usage,
        accounts: &[crate::vault::Account],
        now: Timestamp,
    ) -> Tick {
        let level = live_usage.level(now);
        if !matches!(decision, Decision::Exhausted { .. }) {
            self.exhausted_noted = false;
        }
        match decision {
            Decision::Idle | Decision::Armed => {
                self.turn_since_cross = false;
                self.limit_hit = false;
                Tick::Quiet
            }
            Decision::Pending { .. } | Decision::CoolingDown => Tick::Quiet,
            Decision::Advise { target } => {
                self.notifier.notify(
                    "ccam: switch account",
                    &format!(
                        "{live_name} is at {level:.0}%. Run: ccam to {target}{}",
                        self.target_summary(target, now)
                    ),
                );
                Tick::Advised { to: target.clone() }
            }
            Decision::Exhausted { next } => {
                if self.exhausted_noted {
                    return Tick::Quiet;
                }
                self.exhausted_noted = true;
                let when = match next {
                    Some((name, at)) => format!("next usable: {name} at {}", local_time(*at)),
                    None => "no reset time known yet".into(),
                };
                self.notifier
                    .notify("ccam: all accounts nearly full", &format!("{live_name} is at {level:.0}%; {when}"));
                Tick::Exhausted
            }
            Decision::Switch { target, reason } => {
                // Switch only on a recent reading of the target; an old one may hide a window that filled up since.
                let stale = self.cache.get(target.as_str()).is_none_or(|u| since(u.read_at, now) > TARGET_FRESHNESS);
                if stale {
                    self.refresh_stalest_candidate(
                        &accounts.iter().filter(|a| &a.name == target).cloned().collect::<Vec<_>>(),
                        live_name,
                        now,
                        Duration::ZERO,
                    );
                    return Tick::Quiet;
                }
                match self.manager.switch_to(target) {
                    Ok(_) => {
                        self.state.switched(live_name.clone(), now);
                        self.live = None;
                        self.turn_since_cross = false;
                        self.limit_hit = false;
                        let why = match reason {
                            Reason::Threshold => "turn finished",
                            Reason::Timeout => "no turn finished in 10 min",
                            Reason::LimitHit => "limit hit; send any message in Claude to continue",
                        };
                        self.notifier.notify(
                            "ccam switched account",
                            &format!(
                                "{live_name} -> {target} ({live_name} at {level:.0}%, {why}){}",
                                self.target_summary(target, now)
                            ),
                        );
                        self.manager
                            .log(&format!("auto: switched '{live_name}' -> '{target}' ({reason:?}, {level:.1}%)"));
                        Tick::Switched { from: live_name.clone(), to: target.clone(), reason: *reason }
                    }
                    Err(e) => {
                        self.notifier.notify("ccam: switch failed", &format!("{live_name} -> {target}: {e}"));
                        self.manager.log(&format!("auto: switch '{live_name}' -> '{target}' failed: {e}"));
                        Tick::Failed(e.to_string())
                    }
                }
            }
        }
    }

    fn target_summary(&self, target: &AccountName, now: Timestamp) -> String {
        self.cache
            .get(target.as_str())
            .map(|u| {
                let (five, seven) = u.at(now);
                format!(". {target}: 5h {five:.0}%, week {seven:.0}%")
            })
            .unwrap_or_default()
    }

    fn read_signals(&mut self) {
        if let Some(t) = modified(&self.files.turn_end)
            && self.seen_turn_end.is_none_or(|seen| t > seen)
        {
            self.seen_turn_end = Some(t);
            self.turn_since_poll = true;
            self.turn_since_cross = true;
        }
        if let Some(t) = modified(&self.files.limit_hit)
            && self.seen_limit_hit.is_none_or(|seen| t > seen)
        {
            self.seen_limit_hit = Some(t);
            self.limit_hit = true;
            self.limit_since_poll = true;
        }
    }

    /// Reads one other account whose cached reading is older than `max_age`, the stalest first. One per tick
    /// keeps requests spread out.
    fn refresh_stalest_candidate(
        &mut self,
        accounts: &[crate::vault::Account],
        live_name: &AccountName,
        now: Timestamp,
        max_age: Duration,
    ) {
        let stalest = accounts
            .iter()
            .filter(|a| &a.name != live_name)
            .map(|a| (a, self.cache.get(a.name.as_str()).map(|u| u.read_at)))
            .filter(|(_, read_at)| read_at.is_none_or(|r| since(r, now) >= max_age))
            .min_by_key(|(_, read_at)| *read_at);
        let Some((account, _)) = stalest else { return };

        let unix_now = UnixMillis::from_millis(now.as_millisecond());
        let token = if account.login.usable_at(unix_now) {
            account.login.access_token().to_string()
        } else {
            match self.manager.refresh_saved(&account.name, unix_now) {
                Ok(refreshed) => refreshed.login.access_token().to_string(),
                Err(e) => {
                    self.manager.log(&format!("auto: could not renew '{}': {e}", account.name));
                    self.remember_attempt(&account.name, now);
                    return;
                }
            }
        };
        match self.fetch(&token, now) {
            Ok(usage) => {
                self.cache.insert(account.name.as_str().to_string(), usage);
            }
            Err(_) => self.remember_attempt(&account.name, now),
        }
    }

    /// A failed read still counts as a read for spacing, so a broken account is not retried every tick. Its
    /// usage is recorded as full so it is never chosen until a real reading replaces it.
    fn remember_attempt(&mut self, name: &AccountName, now: Timestamp) {
        let full = Usage { five_hour: 100.0, seven_day: 100.0, five_resets: None, seven_resets: None, read_at: now };
        self.cache.entry(name.as_str().to_string()).or_insert(full).read_at = now;
    }

    fn fetch(&mut self, token: &str, now: Timestamp) -> std::result::Result<Usage, ApiError> {
        self.requests.retain(|t| since(*t, now) < Duration::from_secs(3600));
        self.requests.push(now);
        let body = self.manager.client().usage(token)?;
        Ok(parse_usage(&body, now))
    }

    fn write_status(&self, now: Timestamp, state: &str) {
        let usage_json = |u: &Usage| {
            let (five, seven) = u.at(now);
            json!({"five_hour": five, "seven_day": seven, "read_at": u.read_at.to_string()})
        };
        let status = json!({
            "mode": match self.settings.mode { Mode::Auto => "auto", Mode::Notify => "notify" },
            "threshold": self.settings.threshold,
            "state": state,
            "live": self.live.as_ref().map(|(n, _)| n.as_str()),
            "live_usage": self.live.as_ref().map(|(_, u)| usage_json(u)),
            "candidates": self.cache.iter().map(|(n, u)| (n.clone(), usage_json(u))).collect::<serde_json::Map<_, _>>(),
            "requests_last_hour": self.requests.len(),
            "pid": std::process::id(),
            "updated": now.to_string(),
        });
        let _ = jsonio::write_atomic(&self.files.status, &status, Style::Pretty, true);
    }
}

fn describe(decision: &Decision) -> String {
    match decision {
        Decision::Idle => "watching".into(),
        Decision::Armed => "armed: other accounts' usage is being kept fresh".into(),
        Decision::Pending { target } => format!("over threshold, will move to {target} when the turn ends"),
        Decision::Switch { target, .. } => format!("switching to {target}"),
        Decision::Advise { target } => format!("advised: ccam to {target}"),
        Decision::CoolingDown => "over threshold, waiting out the switch cooldown".into(),
        Decision::Exhausted { .. } => "every account is nearly full".into(),
    }
}

fn parse_usage(body: &Value, now: Timestamp) -> Usage {
    let pct = |key: &str| body[key]["utilization"].as_f64().unwrap_or(0.0);
    let resets = |key: &str| body[key]["resets_at"].as_str().and_then(|s| s.parse().ok());
    Usage {
        five_hour: pct("five_hour"),
        seven_day: pct("seven_day"),
        five_resets: resets("five_hour"),
        seven_resets: resets("seven_day"),
        read_at: now,
    }
}

fn since(then: Timestamp, now: Timestamp) -> Duration {
    Duration::from_millis(u64::try_from(now.as_millisecond() - then.as_millisecond()).unwrap_or(0))
}

fn local_time(t: Timestamp) -> String {
    t.to_zoned(jiff::tz::TimeZone::system()).strftime("%H:%M").to_string()
}

/// Runs the loop until `ccam auto stop`. Only one loop may run per vault.
pub fn run(manager: &Manager, notifier: &dyn Notifier, settings: Settings, echo: &dyn Notifier) -> Result<()> {
    let files = Files::of(manager);
    let timing = LockTiming { wait: Duration::from_secs(2), ..LockTiming::VAULT };
    let _lock: LockGuard = lock::acquire(&files.lock, timing)
        .map_err(|_| Error::Restart("ccam auto is already running (stop it with: ccam auto stop)".into()))?;
    let _ = std::fs::remove_file(&files.stop);
    let mode = match settings.mode {
        Mode::Auto => "switches automatically",
        Mode::Notify => "only notifies",
    };
    echo.notify("ccam auto", &format!("started: {mode} at {:.0}%. Stop with: ccam auto stop", settings.threshold));
    manager.log(&format!("auto: started ({mode}, threshold {:.0}%)", settings.threshold));

    let mut runner = Runner::new(manager, notifier, settings);
    loop {
        if files.stop.exists() {
            let _ = std::fs::remove_file(&files.stop);
            echo.notify("ccam auto", "stopped");
            manager.log("auto: stopped");
            return Ok(());
        }
        if let Tick::Failed(e) = runner.tick(Timestamp::now()) {
            manager.log(&format!("auto: {e}"));
        }
        std::thread::sleep(TICK);
    }
}

/// Asks a running loop to stop and waits for it.
pub fn stop(manager: &Manager) -> Result<bool> {
    let files = Files::of(manager);
    if !files.lock.exists() {
        return Ok(false);
    }
    mark(&files.stop).map_err(|e| Error::io(&files.stop, e))?;
    for _ in 0..30 {
        if !files.lock.exists() {
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(Error::Restart("ccam auto did not stop within 15 s; it may be busy switching. Try again.".into()))
}

/// The last status written by the loop, if it is running.
pub fn status(manager: &Manager) -> Result<Option<Value>> {
    let files = Files::of(manager);
    if !files.lock.exists() {
        return Ok(None);
    }
    jsonio::read(&files.status)
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::{Arc, Mutex};

    use serde_json::json;

    use super::*;
    use crate::api::OAuthClient;
    use crate::login::OAuthLogin;
    use crate::paths::Paths;
    use crate::store::FileStore;

    /// Usage by access token, adjustable during a test; counts requests.
    #[derive(Default)]
    struct FakeApi {
        usage: Mutex<HashMap<String, Value>>,
        requests: Mutex<Vec<String>>,
    }

    impl OAuthClient for Arc<FakeApi> {
        fn account_uuid(&self, access_token: &str) -> std::result::Result<String, ApiError> {
            Ok(access_token.trim_start_matches("at-").to_string())
        }
        fn refresh(&self, login: &OAuthLogin) -> std::result::Result<OAuthLogin, ApiError> {
            Ok(login.clone())
        }
        fn usage(&self, access_token: &str) -> std::result::Result<Value, ApiError> {
            self.requests.lock().unwrap().push(access_token.to_string());
            self.usage.lock().unwrap().get(access_token).cloned().ok_or(ApiError::Status(404))
        }
    }

    #[derive(Default)]
    struct Recorder(Mutex<Vec<String>>);

    impl Notifier for Recorder {
        fn notify(&self, title: &str, body: &str) {
            self.0.lock().unwrap().push(format!("{title}: {body}"));
        }
    }

    struct Setup {
        _home: tempfile::TempDir,
        paths: Paths,
        manager: Manager,
        api: Arc<FakeApi>,
    }

    /// Saves accounts named by their uuid; the first stays logged in. Tokens never expire during a test.
    fn setup(accounts: &[&str]) -> Setup {
        let home = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(home.path(), None);
        fs::create_dir_all(&paths.config_home).unwrap();
        let api = Arc::new(FakeApi::default());
        let store = Box::new(FileStore::new(paths.credentials_file.clone()));
        let manager = Manager::new(paths.clone(), store, Box::new(Arc::clone(&api)));
        let far = UnixMillis::now().plus(Duration::from_secs(86_400)).as_millis();
        for uuid in accounts.iter().rev() {
            let login = json!({"accessToken": format!("at-{uuid}"), "refreshToken": format!("rt-{uuid}"), "expiresAt": far, "refreshTokenExpiresAt": 1});
            fs::write(&paths.credentials_file, json!({"claudeAiOauth": login}).to_string()).unwrap();
            fs::write(&paths.global_config, json!({"oauthAccount": {"accountUuid": uuid}}).to_string()).unwrap();
            manager.save(&uuid.parse().unwrap()).unwrap();
        }
        Setup { _home: home, paths, manager, api }
    }

    fn set_usage(s: &Setup, uuid: &str, five: f64, seven: f64) {
        s.api.usage.lock().unwrap().insert(
            format!("at-{uuid}"),
            json!({"five_hour": {"utilization": five, "resets_at": null}, "seven_day": {"utilization": seven, "resets_at": null}}),
        );
    }

    fn live_uuid(s: &Setup) -> String {
        let config: Value = serde_json::from_str(&fs::read_to_string(&s.paths.global_config).unwrap()).unwrap();
        config["oauthAccount"]["accountUuid"].as_str().unwrap().to_string()
    }

    fn at(minutes: i64) -> Timestamp {
        Timestamp::now() + jiff::SignedDuration::from_mins(minutes)
    }

    fn requests(s: &Setup) -> usize {
        s.api.requests.lock().unwrap().len()
    }

    #[test]
    fn idle_low_usage_costs_one_request_per_half_hour() {
        let s = setup(&["a", "b"]);
        set_usage(&s, "a", 20.0, 10.0);
        set_usage(&s, "b", 0.0, 0.0);
        let recorder = Recorder::default();
        let mut runner = Runner::new(&s.manager, &recorder, Settings::new(Mode::Auto, 98.0));

        for minute in 0..30 {
            assert_eq!(runner.tick(at(minute)), Tick::Quiet);
        }
        assert_eq!(requests(&s), 1, "only the first check, and no candidate reads at 20%");
        runner.tick(at(30));
        assert_eq!(requests(&s), 2);
    }

    #[test]
    fn it_never_picks_an_account_another_install_is_using() {
        let s = setup(&["a", "b", "c"]);
        set_usage(&s, "a", 98.6, 30.0);
        set_usage(&s, "b", 0.0, 0.0);
        set_usage(&s, "c", 20.0, 20.0);
        let b = s.manager.overview().unwrap().listing.get(&"b".parse().unwrap()).unwrap().identity.clone();
        let wsl = crate::presence::Side::new(
            crate::presence::Platform::Wsl { distro: "Ubuntu".into() },
            "/ccam".into(),
            std::path::Path::new("/elsewhere"),
        );
        crate::presence::record(&s.paths.vault_dir, &wsl, Some(&b)).unwrap();
        let recorder = Recorder::default();
        let mut runner = Runner::new(&s.manager, &recorder, Settings::new(Mode::Auto, 98.0));

        runner.tick(at(0));
        mark(&Files::of(&s.manager).limit_hit).unwrap();
        let mut tick = runner.tick(at(1));
        for minute in 2..6 {
            if matches!(tick, Tick::Switched { .. }) {
                break;
            }
            tick = runner.tick(at(minute));
        }

        assert!(matches!(&tick, Tick::Switched { to, .. } if to.as_str() == "c"), "{tick:?}");
        assert!(!s.api.requests.lock().unwrap().iter().any(|t| t == "at-b"), "b's usage is not even read");
    }

    #[test]
    fn at_the_threshold_it_switches_after_the_turn_ends_and_says_so() {
        let s = setup(&["a", "b"]);
        set_usage(&s, "a", 98.6, 30.0);
        set_usage(&s, "b", 5.0, 10.0);
        let recorder = Recorder::default();
        let mut runner = Runner::new(&s.manager, &recorder, Settings::new(Mode::Auto, 98.0));

        assert_eq!(runner.tick(at(0)), Tick::Quiet, "waits for the turn to end");
        assert_eq!(live_uuid(&s), "a");

        mark(&Files::of(&s.manager).turn_end).unwrap();
        let tick = runner.tick(at(1));

        assert_eq!(
            tick,
            Tick::Switched { from: "a".parse().unwrap(), to: "b".parse().unwrap(), reason: Reason::Threshold }
        );
        assert_eq!(live_uuid(&s), "b");
        let notes = recorder.0.lock().unwrap();
        assert!(notes.iter().any(|n| n.contains("a -> b")), "{notes:?}");
    }

    #[test]
    fn notify_mode_advises_and_leaves_the_login_alone() {
        let s = setup(&["a", "b"]);
        set_usage(&s, "a", 99.0, 30.0);
        set_usage(&s, "b", 5.0, 10.0);
        let recorder = Recorder::default();
        let mut runner = Runner::new(&s.manager, &recorder, Settings::new(Mode::Notify, 98.0));

        assert_eq!(runner.tick(at(0)), Tick::Advised { to: "b".parse().unwrap() });
        assert_eq!(runner.tick(at(1)), Tick::Quiet, "advice is given once");
        assert_eq!(live_uuid(&s), "a");
        assert!(recorder.0.lock().unwrap()[0].contains("ccam to b"));
    }

    #[test]
    fn a_hit_limit_switches_at_once() {
        let s = setup(&["a", "b"]);
        set_usage(&s, "a", 100.0, 30.0);
        set_usage(&s, "b", 5.0, 10.0);
        let recorder = Recorder::default();
        let mut runner = Runner::new(&s.manager, &recorder, Settings::new(Mode::Auto, 98.0));
        mark(&Files::of(&s.manager).limit_hit).unwrap();

        assert!(matches!(runner.tick(at(0)), Tick::Switched { reason: Reason::LimitHit, .. }));
        assert_eq!(live_uuid(&s), "b");
    }

    #[test]
    fn when_nothing_has_room_it_says_so_once_and_stays() {
        let s = setup(&["a", "b"]);
        set_usage(&s, "a", 100.0, 30.0);
        set_usage(&s, "b", 99.0, 10.0);
        let recorder = Recorder::default();
        let mut runner = Runner::new(&s.manager, &recorder, Settings::new(Mode::Auto, 98.0));

        assert_eq!(runner.tick(at(0)), Tick::Exhausted);
        assert_eq!(runner.tick(at(3)), Tick::Quiet);
        assert_eq!(live_uuid(&s), "a");
        assert_eq!(recorder.0.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_limit_signal_forces_one_check_not_one_per_tick() {
        let s = setup(&["a", "b"]);
        set_usage(&s, "a", 100.0, 30.0);
        set_usage(&s, "b", 99.0, 10.0);
        let recorder = Recorder::default();
        let mut runner = Runner::new(&s.manager, &recorder, Settings::new(Mode::Auto, 98.0));
        mark(&Files::of(&s.manager).limit_hit).unwrap();

        // One start time: asking the clock again per tick would add the test's own run time to the simulated two
        // minutes, and on a slow machine that crosses into the next scheduled check.
        let start = at(0);
        runner.tick(start);
        let after_first = requests(&s);
        for second in 1..24 {
            runner.tick(start + jiff::SignedDuration::from_secs(second * 5));
        }
        assert_eq!(requests(&s), after_first, "no further checks within two minutes while stuck at the limit");
    }

    #[test]
    fn status_is_written_for_ccam_auto_status() {
        let s = setup(&["a", "b"]);
        set_usage(&s, "a", 40.0, 30.0);
        let recorder = Recorder::default();
        let mut runner = Runner::new(&s.manager, &recorder, Settings::new(Mode::Auto, 98.0));
        runner.tick(at(0));

        let status: Value = serde_json::from_str(&fs::read_to_string(Files::of(&s.manager).status).unwrap()).unwrap();
        assert_eq!(status["live"], "a");
        assert_eq!(status["mode"], "auto");
        assert_eq!(status["live_usage"]["five_hour"], 40.0);
    }
}
