//! `ccam usage`: gathers per-account usage and renders it as cards.

use jiff::Timestamp;
use serde::Serialize;
use serde_json::Value;

use crate::error::Result;
use crate::login::UnixMillis;
use crate::manager::{LiveState, Manager};
use crate::name::AccountName;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Window {
    pub label: String,
    pub percent: f64,
    pub resets_at: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum CardState {
    Ok {
        windows: Vec<Window>,
    },
    /// A saved account whose access token expired and was not refreshed.
    Stale,
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Card {
    pub name: AccountName,
    pub current: bool,
    pub plan: Option<String>,
    #[serde(flatten)]
    pub state: CardState,
}

/// Reads the windows ccam knows about and ignores the rest of the response.
pub fn parse_windows(body: &Value) -> Vec<Window> {
    let window = |label: String, percent: Option<f64>, resets_at: &Value| {
        percent.map(|percent| Window { label, percent, resets_at: resets_at.as_str().and_then(|s| s.parse().ok()) })
    };

    let mut windows = Vec::new();
    for (key, label) in [("five_hour", "5 hour"), ("seven_day", "7 day")] {
        let entry = &body[key];
        windows.extend(window(label.into(), entry["utilization"].as_f64(), &entry["resets_at"]));
    }
    for limit in body["limits"].as_array().into_iter().flatten() {
        if let Some(model) = limit["scope"]["model"]["display_name"].as_str() {
            windows.extend(window(format!("7 day {model}"), limit["percent"].as_f64(), &limit["resets_at"]));
        }
    }
    windows
}

/// One card per saved account, fetched concurrently.
///
/// The live account always uses the live access token and is never refreshed here: Claude Code owns that login,
/// and rotating it underneath a running session would log the session out.
pub fn collect(manager: &Manager, refresh: bool) -> Result<Vec<Card>> {
    let overview = manager.overview()?;
    let live_name = overview.live_account().map(|a| a.name.clone());
    let live_login = match &overview.live {
        LiveState::Present(live) => Some(live.login.clone()),
        _ => None,
    };
    let now = UnixMillis::now();

    Ok(std::thread::scope(|scope| {
        let workers: Vec<_> = overview
            .listing
            .accounts
            .iter()
            .map(|account| {
                let current = live_name.as_ref() == Some(&account.name);
                let login = if current {
                    live_login.clone().unwrap_or_else(|| account.login.clone())
                } else {
                    account.login.clone()
                };
                scope.spawn(move || {
                    let token = if login.usable_at(now) {
                        Ok(login.access_token().to_string())
                    } else if current {
                        Err(CardState::Error {
                            message: "access token expired, send any message in Claude to renew it".into(),
                        })
                    } else if !refresh {
                        Err(CardState::Stale)
                    } else {
                        manager
                            .refresh_saved(&account.name, now)
                            .map(|refreshed| refreshed.login.access_token().to_string())
                            .map_err(|e| CardState::Error { message: e.to_string() })
                    };
                    let state = match token.map(|t| manager.client().usage(&t)) {
                        Ok(Ok(body)) => CardState::Ok { windows: parse_windows(&body) },
                        Ok(Err(e)) => CardState::Error { message: e.to_string() },
                        Err(state) => state,
                    };
                    Card {
                        name: account.name.clone(),
                        current,
                        plan: login.subscription_type().map(str::to_string),
                        state,
                    }
                })
            })
            .collect();
        workers.into_iter().map(|w| w.join().expect("usage worker panicked")).collect()
    }))
}

/// Narrowest card interior; wide enough for a label, a bar and a countdown.
const MIN_INNER_WIDTH: usize = 56;
const BAR_CELLS: usize = 20;
const MIN_LABEL_WIDTH: usize = 12;

mod ansi {
    pub const BOLD: &str = "1";
    pub const DIM: &str = "2";
    pub const GREEN: &str = "38;5;114";
    pub const YELLOW: &str = "38;5;221";
    pub const RED: &str = "38;5;203";
    pub const CYAN: &str = "38;5;117";
    pub const GREY: &str = "38;5;244";
    pub const DARK_GREY: &str = "38;5;240";
}

struct Palette {
    enabled: bool,
}

impl Palette {
    fn paint(&self, code: &str, text: &str) -> String {
        if self.enabled { format!("\x1b[{code}m{text}\x1b[0m") } else { text.to_string() }
    }

    fn severity(&self, percent: f64, text: &str) -> String {
        let code = match percent {
            p if p >= 90.0 => ansi::RED,
            p if p >= 70.0 => ansi::YELLOW,
            _ => ansi::GREEN,
        };
        self.paint(code, text)
    }
}

/// Renders every card at one shared width, so the boxes line up.
pub fn render(cards: &[Card], now: Timestamp, color: bool) -> String {
    let p = Palette { enabled: color };
    let label_width = cards
        .iter()
        .filter_map(|c| match &c.state {
            CardState::Ok { windows } => windows.iter().map(|w| w.label.chars().count()).max(),
            _ => None,
        })
        .fold(MIN_LABEL_WIDTH, usize::max);

    let bodies: Vec<(String, String, Vec<String>)> = cards
        .iter()
        .map(|card| {
            let (head, tail) = title(card, &p);
            (head, tail, body_lines(card, now, label_width, &p))
        })
        .collect();
    // The top border reads "╭─ <title> ─…─ <plan> ─╮": its fixed parts take the space of the "│ " and " │" sides.
    let width = bodies
        .iter()
        .flat_map(|(head, tail, lines)| {
            lines.iter().map(|l| visible_len(l)).chain([visible_len(head) + visible_len(tail) + 1 - SIDES])
        })
        .fold(MIN_INNER_WIDTH, usize::max);

    bodies.iter().map(|(head, tail, lines)| draw(head, tail, lines, width)).collect::<Vec<_>>().join("\n")
}

/// Columns taken by the left and right borders of a body line, "│ " and " │".
const SIDES: usize = 4;

fn title(card: &Card, p: &Palette) -> (String, String) {
    let marker = if card.current { p.paint(ansi::CYAN, "●") } else { p.paint(ansi::GREY, "○") };
    let mut title = format!("{marker} {}", p.paint(ansi::BOLD, card.name.as_str()));
    if card.current {
        title.push_str(&p.paint(ansi::CYAN, "  current"));
    }
    let tail = match &card.plan {
        Some(plan) => format!(" {} ─╮", p.paint(ansi::DIM, plan)),
        None => "─╮".into(),
    };
    (format!("╭─ {title} "), tail)
}

fn body_lines(card: &Card, now: Timestamp, label_width: usize, p: &Palette) -> Vec<String> {
    match &card.state {
        CardState::Ok { windows } if windows.is_empty() => vec![p.paint(ansi::DIM, "no usage reported")],
        CardState::Ok { windows } => windows.iter().map(|w| window_line(w, now, label_width, p)).collect(),
        CardState::Stale => vec![p.paint(ansi::DIM, "stale login, run: ccam usage --refresh")],
        CardState::Error { message } => vec![p.paint(ansi::RED, &format!("✗ {message}"))],
    }
}

fn draw(head: &str, tail: &str, lines: &[String], width: usize) -> String {
    let rule = width + SIDES - 2;
    let mut out = format!("{head}{}{tail}\n", "─".repeat(rule + 2 - visible_len(head) - visible_len(tail)));
    for line in lines {
        out.push_str(&format!("│ {line}{} │\n", " ".repeat(width - visible_len(line))));
    }
    out.push_str(&format!("╰{}╯\n", "─".repeat(rule)));
    out
}

fn window_line(window: &Window, now: Timestamp, label_width: usize, p: &Palette) -> String {
    let percent = window.percent.clamp(0.0, 100.0);
    let filled = ((percent / 100.0) * BAR_CELLS as f64).round() as usize;
    let bar = p.severity(percent, &"█".repeat(filled)) + &p.paint(ansi::DARK_GREY, &"░".repeat(BAR_CELLS - filled));
    let percent_text = p.severity(percent, &format!("{:>4}", format!("{}%", percent.round())));
    let reset = window.resets_at.map(|at| p.paint(ansi::DIM, &reset_text(now, at))).unwrap_or_default();
    format!("{:<label_width$} {bar} {percent_text}  {reset}", window.label)
}

fn reset_text(now: Timestamp, at: Timestamp) -> String {
    let minutes = ((at.as_millisecond() - now.as_millisecond()) as f64 / 60_000.0).ceil() as i64;
    if minutes <= 0 {
        return "resetting".into();
    }
    let (days, hours, mins) = (minutes / 1440, minutes % 1440 / 60, minutes % 60);
    let span = match (days, hours) {
        (0, 0) => format!("{mins}m"),
        (0, _) => format!("{hours}h {mins}m"),
        _ => format!("{days}d {hours}h"),
    };
    format!("resets in {span}")
}

/// Display width, ignoring ANSI colour codes. Every glyph ccam draws is one column wide.
fn visible_len(s: &str) -> usize {
    let mut len = 0;
    let mut in_escape = false;
    for c in s.chars() {
        match (in_escape, c) {
            (false, '\x1b') => in_escape = true,
            (true, 'm') => in_escape = false,
            (true, _) => {}
            (false, _) => len += 1,
        }
    }
    len
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Mutex;

    use serde_json::json;

    use super::*;
    use crate::api::{ApiError, OAuthClient};
    use crate::login::OAuthLogin;
    use crate::paths::Paths;
    use crate::store::FileStore;

    fn name(s: &str) -> AccountName {
        s.parse().unwrap()
    }

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    #[test]
    fn parses_session_weekly_and_per_model_windows() {
        let body = json!({
            "five_hour": {"utilization": 27.0, "resets_at": "2026-09-17T16:00:00.123456+00:00"},
            "seven_day": {"utilization": 46, "resets_at": null},
            "limits": [
                {"scope": {"model": {"display_name": "Fable"}}, "percent": 12.5, "resets_at": "2026-09-20T00:00:00Z"},
                {"scope": {}, "percent": 99},
            ],
            "extra_usage": {"utilization": 3},
        });

        assert_eq!(
            parse_windows(&body),
            [
                Window { label: "5 hour".into(), percent: 27.0, resets_at: Some(ts("2026-09-17T16:00:00.123456Z")) },
                Window { label: "7 day".into(), percent: 46.0, resets_at: None },
                Window { label: "7 day Fable".into(), percent: 12.5, resets_at: Some(ts("2026-09-20T00:00:00Z")) },
            ]
        );
    }

    #[test]
    fn renders_cards_with_bars_countdowns_and_problems() {
        let now = ts("2026-09-17T12:00:00Z");
        let cards = [
            Card {
                name: name("main"),
                current: true,
                plan: Some("pro".into()),
                state: CardState::Ok {
                    windows: vec![
                        Window { label: "5 hour".into(), percent: 27.0, resets_at: Some(ts("2026-09-17T16:04:00Z")) },
                        Window { label: "7 day".into(), percent: 46.0, resets_at: Some(ts("2026-09-22T21:00:00Z")) },
                    ],
                },
            },
            Card { name: name("second"), current: false, plan: Some("pro".into()), state: CardState::Stale },
            Card {
                name: name("third"),
                current: false,
                plan: None,
                state: CardState::Error { message: "timed out".into() },
            },
        ];

        let expected = "\
╭─ ● main  current ────────────────────────────────── pro ─╮
│ 5 hour       █████░░░░░░░░░░░░░░░  27%  resets in 4h 4m  │
│ 7 day        █████████░░░░░░░░░░░  46%  resets in 5d 9h  │
╰──────────────────────────────────────────────────────────╯

╭─ ○ second ───────────────────────────────────────── pro ─╮
│ stale login, run: ccam usage --refresh                   │
╰──────────────────────────────────────────────────────────╯

╭─ ○ third ────────────────────────────────────────────────╮
│ ✗ timed out                                              │
╰──────────────────────────────────────────────────────────╯
";
        assert_eq!(render(&cards, now, false), expected);
    }

    #[test]
    fn long_names_and_labels_widen_every_card_equally() {
        let now = ts("2026-09-17T12:00:00Z");
        let window = Window { label: "7 day Some Long Model Name".into(), percent: 1.0, resets_at: None };
        let cards = [
            Card {
                name: name(&"n".repeat(64)),
                current: true,
                plan: Some("max".into()),
                state: CardState::Ok { windows: vec![window] },
            },
            Card { name: name("b"), current: false, plan: None, state: CardState::Stale },
        ];

        let out = render(&cards, now, false);
        let widths: std::collections::BTreeSet<_> =
            out.lines().filter(|l| !l.is_empty()).map(|l| l.chars().count()).collect();
        assert_eq!(widths.len(), 1, "{out}");
    }

    #[test]
    fn past_or_imminent_resets_say_resetting() {
        let now = ts("2026-09-17T12:00:00Z");
        assert_eq!(reset_text(now, ts("2026-09-17T11:59:00Z")), "resetting");
        assert_eq!(reset_text(now, ts("2026-09-17T12:00:20Z")), "resets in 1m");
    }

    #[test]
    fn colour_marks_severity() {
        let card = |percent| Card {
            name: name("a"),
            current: false,
            plan: None,
            state: CardState::Ok { windows: vec![Window { label: "5 hour".into(), percent, resets_at: None }] },
        };
        let now = ts("2026-09-17T12:00:00Z");
        assert!(render(&[card(10.0)], now, true).contains(&format!("\x1b[{}m", ansi::GREEN)));
        assert!(render(&[card(75.0)], now, true).contains(&format!("\x1b[{}m", ansi::YELLOW)));
        assert!(render(&[card(95.0)], now, true).contains(&format!("\x1b[{}m", ansi::RED)));
    }

    #[test]
    fn json_output_is_tagged_by_status() {
        let card =
            Card { name: name("a"), current: false, plan: None, state: CardState::Error { message: "x".into() } };
        assert_eq!(
            serde_json::to_value(&card).unwrap(),
            json!({"name": "a", "current": false, "plan": null, "status": "error", "message": "x"})
        );
    }

    /// Usage by access token; refreshes rotate to `fresh-<old token>`.
    #[derive(Default)]
    struct FakeUsage {
        usage_tokens: Mutex<Vec<String>>,
    }

    impl OAuthClient for std::sync::Arc<FakeUsage> {
        fn account_uuid(&self, access_token: &str) -> std::result::Result<String, ApiError> {
            Ok(access_token.split('-').nth(1).unwrap_or_default().to_string())
        }

        fn refresh(&self, login: &OAuthLogin) -> std::result::Result<OAuthLogin, ApiError> {
            if login.access_token().contains("dead") {
                return Err(ApiError::LoginExpired);
            }
            let fresh = format!("fresh-{}", login.access_token());
            Ok(login.refreshed(&fresh, Some("rt-fresh"), UnixMillis::now().plus(std::time::Duration::from_secs(3600))))
        }

        fn usage(&self, access_token: &str) -> std::result::Result<Value, ApiError> {
            self.usage_tokens.lock().unwrap().push(access_token.to_string());
            Ok(json!({"seven_day": {"utilization": 80}}))
        }
    }

    struct Setup {
        _home: tempfile::TempDir,
        paths: Paths,
        manager: Manager,
        client: std::sync::Arc<FakeUsage>,
    }

    /// Saves `(name, uuid, access token, expires_at)` accounts, leaving the first one logged in.
    fn setup(accounts: &[(&str, &str, &str, i64)]) -> Setup {
        let home = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(home.path(), None);
        fs::create_dir_all(&paths.config_home).unwrap();
        let client = std::sync::Arc::new(FakeUsage::default());
        let store = Box::new(FileStore::new(paths.credentials_file.clone()));
        let manager = Manager::new(paths.clone(), store, Box::new(std::sync::Arc::clone(&client)));
        for (account, uuid, token, expires_at) in accounts.iter().rev() {
            let login = json!({"accessToken": token, "refreshToken": format!("rt-{token}"), "expiresAt": expires_at, "subscriptionType": "pro"});
            fs::write(&paths.credentials_file, json!({"claudeAiOauth": login}).to_string()).unwrap();
            fs::write(&paths.global_config, json!({"oauthAccount": {"accountUuid": uuid}}).to_string()).unwrap();
            manager.save(&name(account)).unwrap();
        }
        Setup { _home: home, paths, manager, client }
    }

    fn future() -> i64 {
        Timestamp::now().as_millisecond() + 3_600_000
    }

    #[test]
    fn saved_accounts_with_expired_tokens_are_stale_without_refresh() {
        let s = setup(&[("live", "L", "at-L", future()), ("old", "O", "at-O", 1)]);

        let cards = collect(&s.manager, false).unwrap();

        assert!(cards[0].current);
        assert_eq!(cards[1].state, CardState::Stale);
        assert_eq!(*s.client.usage_tokens.lock().unwrap(), ["at-L"]);
    }

    #[test]
    fn the_live_login_is_never_refreshed() {
        let s = setup(&[("live", "L", "at-L", 1)]);

        let cards = collect(&s.manager, true).unwrap();

        assert!(matches!(&cards[0].state, CardState::Error { message } if message.contains("send any message")));
        assert!(s.client.usage_tokens.lock().unwrap().is_empty());
    }

    #[test]
    fn refresh_saves_the_rotated_login_and_uses_it() {
        let s = setup(&[("live", "L", "at-L", future()), ("old", "O", "at-O", 1)]);

        let cards = collect(&s.manager, true).unwrap();

        let saved = crate::jsonio::read(&s.paths.vault_dir.join("old/login.json")).unwrap().unwrap();
        assert_eq!(saved["claudeAiOauth"]["refreshToken"], "rt-fresh");
        assert!(matches!(&cards[1].state, CardState::Ok { windows } if windows[0].percent == 80.0));
        assert!(s.client.usage_tokens.lock().unwrap().contains(&"fresh-at-O".to_string()));
    }

    #[test]
    fn a_dead_saved_login_is_reported_and_left_untouched() {
        let s = setup(&[("live", "L", "at-L", future()), ("old", "O", "at-O-dead", 1)]);

        let cards = collect(&s.manager, true).unwrap();

        assert_eq!(cards[1].state, CardState::Error { message: ApiError::LoginExpired.to_string() });
        let saved = crate::jsonio::read(&s.paths.vault_dir.join("old/login.json")).unwrap().unwrap();
        assert_eq!(saved["claudeAiOauth"]["refreshToken"], "rt-at-O-dead");
    }
}
