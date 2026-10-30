//! `ccam usage`: gathers per-account usage and renders it as cards.

use jiff::Timestamp;
use serde::Serialize;
use serde_json::Value;

use crate::error::Result;
use crate::login::UnixMillis;
use crate::manager::{LiveState, Manager};
use crate::name::AccountName;
use crate::store::LoginStore;

pub mod info;

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
        #[serde(skip_serializing_if = "Option::is_none")]
        extra_usage: Option<ExtraUsage>,
    },
    /// A saved account whose access token expired and was not refreshed.
    Stale,
    Error {
        message: String,
    },
}

/// The usage response's `extra_usage`: spend beyond the plan this billing period, in minor units of `currency`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExtraUsage {
    pub is_enabled: bool,
    pub used_credits: Option<f64>,
    /// None means no monthly limit.
    pub monthly_limit: Option<f64>,
    pub currency: Option<String>,
    pub decimal_places: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Card {
    pub name: AccountName,
    pub current: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lane: Option<String>,
    pub plan: Option<String>,
    pub plan_label: Option<String>,
    /// The account's email address. Filled only for `ccam usage --email` and `--info`, the only places ccam shows
    /// emails.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(flatten)]
    pub state: CardState,
    /// Filled only for `ccam usage --info`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub info: Option<info::Info>,
}

/// The plan as Claude Code names it, from the login's `subscriptionType` and `rateLimitTier`. Max 5x and Max 20x
/// differ only in the tier.
pub fn plan_label(plan: &str, rate_limit_tier: Option<&str>) -> String {
    match (plan, rate_limit_tier) {
        ("pro", _) => "Pro",
        ("max", Some("default_claude_max_5x")) => "Max 5x",
        ("max", Some("default_claude_max_20x")) => "Max 20x",
        ("max", _) => "Max",
        ("team", _) => "Team",
        ("enterprise", _) => "Enterprise",
        (other, _) => other,
    }
    .to_string()
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

pub fn parse_extra_usage(body: &Value) -> Option<ExtraUsage> {
    let entry = body.get("extra_usage").filter(|e| e.is_object())?;
    Some(ExtraUsage {
        is_enabled: entry["is_enabled"].as_bool().unwrap_or(false),
        used_credits: entry["used_credits"].as_f64(),
        monthly_limit: entry["monthly_limit"].as_f64(),
        currency: currency_code(&entry["currency"]),
        decimal_places: entry["decimal_places"].as_u64().and_then(|d| u32::try_from(d).ok()),
    })
}

impl ExtraUsage {
    /// "used / limit" while extra usage is on.
    fn spend(&self) -> Option<String> {
        if !self.is_enabled {
            return None;
        }
        // Claude Code documents the amounts as cents for USD, so two places when the response omits them.
        let amount = |minor| money(minor, self.decimal_places.unwrap_or(2), self.currency.as_deref());
        let limit = self.monthly_limit.map_or_else(|| "unlimited".to_string(), amount);
        Some(format!("{} / {limit}", amount(self.used_credits.unwrap_or(0.0))))
    }
}

/// An ISO 4217 code, `^[A-Z]{3}$`. Anything else is dropped, so its amounts print without a currency.
pub fn currency_code(value: &Value) -> Option<String> {
    value.as_str().filter(|c| c.len() == 3 && c.bytes().all(|b| b.is_ascii_uppercase())).map(str::to_string)
}

/// `minor` units of `currency` with `decimal_places` digits: "$12.40" for USD, "12.40 EUR" otherwise.
pub fn money(minor: f64, decimal_places: u32, currency: Option<&str>) -> String {
    let places = decimal_places.min(9);
    let amount = format!("{:.*}", places as usize, minor / 10f64.powi(places as i32));
    match currency {
        Some("USD") => format!("${amount}"),
        Some(code) => format!("{amount} {code}"),
        None => amount,
    }
}

/// One card per saved account, fetched concurrently.
///
/// The live account always uses the live access token and is never refreshed here: Claude Code owns that login,
/// and rotating it underneath a running session would log the session out.
///
/// With `with_info`, each account with a usable token also gets its profile and, when Claude Code's cache has no
/// recent answer, its guest pass eligibility, both read with the token its usage was read with.
pub fn collect(manager: &Manager, refresh: bool, with_info: bool) -> Result<Vec<Card>> {
    let overview = manager.overview()?;
    let live_name = overview.live_account().map(|a| a.name.clone());
    let live_login = match &overview.live {
        LiveState::Present(live) => Some(live.login.clone()),
        _ => None,
    };
    let clock = Timestamp::now();
    let now = UnixMillis::from_millis(clock.as_millisecond());
    // Claude Code caches guest pass eligibility for every organization it ran as in the main global config.
    let global_config = match with_info {
        true => crate::jsonio::read(&manager.paths().global_config).ok().flatten().unwrap_or_default(),
        false => Value::Null,
    };
    let global_config = &global_config;

    let mut lane_logins: Vec<(AccountName, String, crate::login::OAuthLogin)> = Vec::new();
    for id in crate::lane::all(manager.paths()) {
        if let Ok(paths) = crate::lane::paths(manager.paths(), &id)
            && let Ok(Some(raw)) = crate::store::FileStore::new(paths.credentials_file).read()
            && let Ok(login) = crate::login::OAuthLogin::parse(raw)
            && let Ok(Some(oauth_account)) = crate::config::GlobalConfig::new(paths.global_config).read_oauth_account()
            && let Some(identity) = crate::login::Identity::from_oauth_account(&oauth_account)
            && let Some(account) = overview.listing.find(&identity)
        {
            lane_logins.push((account.name.clone(), id.to_string(), login));
        }
    }

    Ok(std::thread::scope(|scope| {
        let workers: Vec<_> = overview
            .listing
            .accounts
            .iter()
            .map(|account| {
                let current = live_name.as_ref() == Some(&account.name);
                let (login, in_lane) = if current {
                    (live_login.clone().unwrap_or_else(|| account.login.clone()), None)
                } else if let Some((_, lane_id, login)) = lane_logins.iter().find(|(n, _, _)| n == &account.name) {
                    (login.clone(), Some(lane_id.clone()))
                } else {
                    (account.login.clone(), None)
                };
                scope.spawn(move || {
                    let token = if login.usable_at(now) {
                        Ok(login.access_token().to_string())
                    } else if current {
                        Err(CardState::Error {
                            message: "access token expired, send any message in Claude to renew it".into(),
                        })
                    } else if let Some(lane_id) = &in_lane {
                        Err(CardState::Error {
                            message: format!("access token expired, send any message in lane {lane_id} to renew it"),
                        })
                    } else if !refresh {
                        Err(CardState::Stale)
                    } else {
                        manager
                            .refresh_saved(&account.name, now)
                            .map(|refreshed| refreshed.login.access_token().to_string())
                            .map_err(|e| CardState::Error { message: e.to_string() })
                    };
                    let usage = token.as_deref().map(|t| manager.client().usage(t));
                    let state = match &usage {
                        Ok(Ok(body)) => {
                            CardState::Ok { windows: parse_windows(body), extra_usage: parse_extra_usage(body) }
                        }
                        Ok(Err(e)) => CardState::Error { message: e.to_string() },
                        Err(state) => (*state).clone(),
                    };
                    let info = with_info.then(|| {
                        let passes = (account.identity.organization_uuid.as_deref())
                            .filter(|_| info::has_guest_passes(login.subscription_type()))
                            .map(|organization_uuid| info::PassesSource { organization_uuid, global_config });
                        let usage = usage.as_ref().ok().and_then(|u| u.as_ref().ok());
                        info::gather(manager.client(), token.as_deref().ok(), usage, passes, clock)
                    });
                    Card {
                        name: account.name.clone(),
                        current,
                        lane: in_lane,
                        plan: login.subscription_type().map(str::to_string),
                        plan_label: login.subscription_type().map(|plan| plan_label(plan, login.rate_limit_tier())),
                        email: None,
                        state,
                        info,
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
            CardState::Ok { windows, .. } => windows.iter().map(|w| w.label.chars().count()).max(),
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
    let marker =
        if card.current || card.lane.is_some() { p.paint(ansi::CYAN, "●") } else { p.paint(ansi::GREY, "○") };
    let mut title = format!("{marker} {}", p.paint(ansi::BOLD, card.name.as_str()));
    if let Some(email) = &card.email {
        title.push_str(&format!("  {email}"));
    }
    if card.current {
        title.push_str(&p.paint(ansi::CYAN, "  current"));
    } else if let Some(lane) = &card.lane {
        title.push_str(&p.paint(ansi::CYAN, &format!("  lane {lane}")));
    }
    let tail = match &card.plan_label {
        Some(plan) => format!(" {} ─╮", p.paint(ansi::DIM, plan)),
        None => "─╮".into(),
    };
    (format!("╭─ {title} "), tail)
}

fn body_lines(card: &Card, now: Timestamp, label_width: usize, p: &Palette) -> Vec<String> {
    let mut lines = usage_lines(card, now, label_width, p);
    if let Some(info) = &card.info {
        lines.push(String::new());
        lines.extend(info::lines(info, now));
    }
    lines
}

fn usage_lines(card: &Card, now: Timestamp, label_width: usize, p: &Palette) -> Vec<String> {
    match &card.state {
        CardState::Ok { windows, extra_usage } => {
            let mut lines: Vec<String> = windows.iter().map(|w| window_line(w, now, label_width, p)).collect();
            if lines.is_empty() {
                lines.push(p.paint(ansi::DIM, "no usage reported"));
            }
            let spend = extra_usage.as_ref().and_then(ExtraUsage::spend);
            lines.extend(spend.map(|spend| format!("{:<label_width$} {spend}", "Extra usage")));
            lines
        }
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
    fn extra_usage_is_read_beside_the_windows() {
        let body = json!({"extra_usage": {
            "is_enabled": false, "monthly_limit": null, "used_credits": 0.0, "currency": "USD", "decimal_places": 2,
            "disabled_reason": "out_of_credits", "utilization": null,
        }});
        assert_eq!(
            parse_extra_usage(&body),
            Some(ExtraUsage {
                is_enabled: false,
                used_credits: Some(0.0),
                monthly_limit: None,
                currency: Some("USD".into()),
                decimal_places: Some(2),
            })
        );
        for currency in ["usd", "US$", "https://example.test", "USDX"] {
            let body = json!({"extra_usage": {"is_enabled": true, "used_credits": 1240, "currency": currency}});
            let extra = parse_extra_usage(&body).unwrap();
            assert_eq!(extra.currency, None, "{currency}");
            assert_eq!(extra.spend().as_deref(), Some("12.40 / unlimited"), "{currency}");
        }
        assert_eq!(parse_extra_usage(&json!({"five_hour": {}})), None);
        assert_eq!(parse_extra_usage(&json!({"extra_usage": null})), None);
    }

    #[test]
    fn money_is_formatted_from_minor_units() {
        assert_eq!(money(1240.0, 2, Some("USD")), "$12.40");
        assert_eq!(money(5000.0, 2, Some("USD")), "$50.00");
        assert_eq!(money(1240.0, 0, Some("USD")), "$1240");
        assert_eq!(money(1240.0, 2, Some("EUR")), "12.40 EUR");
        assert_eq!(money(7.0, 2, None), "0.07");
    }

    #[test]
    fn the_extra_usage_line_shows_only_when_enabled() {
        let now = ts("2026-09-17T12:00:00Z");
        let card = |extra_usage| Card {
            name: name("a"),
            current: false,
            lane: None,
            plan: None,
            plan_label: None,
            email: None,
            state: CardState::Ok {
                windows: vec![Window { label: "5 hour".into(), percent: 1.0, resets_at: None }],
                extra_usage: Some(extra_usage),
            },
            info: None,
        };
        let on = ExtraUsage {
            is_enabled: true,
            used_credits: Some(1240.0),
            monthly_limit: None,
            currency: Some("EUR".into()),
            decimal_places: None,
        };
        let off = ExtraUsage { is_enabled: false, ..on.clone() };

        assert!(render(&[card(on)], now, false).contains("│ Extra usage  12.40 EUR / unlimited "));
        assert!(!render(&[card(off)], now, false).contains("Extra usage"));
    }

    #[test]
    fn renders_cards_with_bars_countdowns_and_problems() {
        let now = ts("2026-09-17T12:00:00Z");
        let cards = [
            Card {
                name: name("main"),
                current: true,
                lane: None,
                plan: Some("pro".into()),
                plan_label: Some("Pro".into()),
                email: None,
                state: CardState::Ok {
                    windows: vec![
                        Window { label: "5 hour".into(), percent: 27.0, resets_at: Some(ts("2026-09-17T16:04:00Z")) },
                        Window { label: "7 day".into(), percent: 46.0, resets_at: Some(ts("2026-09-22T21:00:00Z")) },
                    ],
                    extra_usage: Some(ExtraUsage {
                        is_enabled: true,
                        used_credits: Some(1240.0),
                        monthly_limit: Some(5000.0),
                        currency: Some("USD".into()),
                        decimal_places: Some(2),
                    }),
                },
                info: None,
            },
            Card {
                name: name("second"),
                current: false,
                lane: None,
                plan: Some("pro".into()),
                plan_label: Some("Pro".into()),
                email: None,
                state: CardState::Stale,
                info: None,
            },
            Card {
                name: name("third"),
                current: false,
                lane: None,
                plan: None,
                plan_label: None,
                email: None,
                state: CardState::Error { message: "timed out".into() },
                info: None,
            },
        ];

        let expected = "\
╭─ ● main  current ────────────────────────────────── Pro ─╮
│ 5 hour       █████░░░░░░░░░░░░░░░  27%  resets in 4h 4m  │
│ 7 day        █████████░░░░░░░░░░░  46%  resets in 5d 9h  │
│ Extra usage  $12.40 / $50.00                             │
╰──────────────────────────────────────────────────────────╯

╭─ ○ second ───────────────────────────────────────── Pro ─╮
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
                lane: None,
                plan: Some("max".into()),
                plan_label: Some("Max".into()),
                email: None,
                state: CardState::Ok { windows: vec![window], extra_usage: None },
                info: None,
            },
            Card {
                name: name("b"),
                current: false,
                lane: None,
                plan: None,
                plan_label: None,
                email: None,
                state: CardState::Stale,
                info: None,
            },
        ];

        let out = render(&cards, now, false);
        let widths: std::collections::BTreeSet<_> =
            out.lines().filter(|l| !l.is_empty()).map(|l| l.chars().count()).collect();
        assert_eq!(widths.len(), 1, "{out}");
    }

    #[test]
    fn an_email_shows_after_the_name_only_when_the_card_has_one() {
        let now = ts("2026-09-17T12:00:00Z");
        let card = |email: Option<&str>| Card {
            name: name("work"),
            current: false,
            lane: None,
            plan: None,
            plan_label: None,
            email: email.map(str::to_string),
            state: CardState::Stale,
            info: None,
        };

        let first_line = |c: Card| render(&[c], now, false).lines().next().unwrap().to_string();
        assert!(first_line(card(Some("me@example.test"))).contains("work  me@example.test"));
        assert!(!first_line(card(None)).contains('@'));
    }

    #[test]
    fn plan_labels_name_the_max_tier_and_pass_unknown_plans_through() {
        assert_eq!(plan_label("pro", Some("default_claude_ai")), "Pro");
        assert_eq!(plan_label("max", Some("default_claude_max_5x")), "Max 5x");
        assert_eq!(plan_label("max", Some("default_claude_max_20x")), "Max 20x");
        assert_eq!(plan_label("max", Some("something_new")), "Max");
        assert_eq!(plan_label("max", None), "Max");
        assert_eq!(plan_label("team", Some("default_claude_max_5x")), "Team");
        assert_eq!(plan_label("enterprise", None), "Enterprise");
        assert_eq!(plan_label("free", None), "free");
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
            lane: None,
            plan: None,
            plan_label: None,
            email: None,
            state: CardState::Ok {
                windows: vec![Window { label: "5 hour".into(), percent, resets_at: None }],
                extra_usage: None,
            },
            info: None,
        };
        let now = ts("2026-09-17T12:00:00Z");
        assert!(render(&[card(10.0)], now, true).contains(&format!("\x1b[{}m", ansi::GREEN)));
        assert!(render(&[card(75.0)], now, true).contains(&format!("\x1b[{}m", ansi::YELLOW)));
        assert!(render(&[card(95.0)], now, true).contains(&format!("\x1b[{}m", ansi::RED)));
    }

    #[test]
    fn json_output_is_tagged_by_status() {
        let card = Card {
            name: name("a"),
            current: false,
            lane: None,
            plan: None,
            plan_label: None,
            email: None,
            state: CardState::Error { message: "x".into() },
            info: None,
        };
        assert_eq!(
            serde_json::to_value(&card).unwrap(),
            json!({"name": "a", "current": false, "plan": null, "plan_label": null, "status": "error", "message": "x"})
        );
    }

    /// Usage by access token; refreshes rotate to `fresh-<old token>`. Profile and guest pass reads answer with
    /// `info::fixtures` and are recorded as `profile <token>` and `eligibility <org>`.
    #[derive(Default)]
    struct FakeUsage {
        usage_tokens: Mutex<Vec<String>>,
        info_requests: Mutex<Vec<String>>,
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

        fn profile(&self, access_token: &str) -> std::result::Result<Value, ApiError> {
            self.info_requests.lock().unwrap().push(format!("profile {access_token}"));
            Ok(info::fixtures::profile())
        }

        fn referral_eligibility(&self, _: &str, organization_uuid: &str) -> std::result::Result<Value, ApiError> {
            self.info_requests.lock().unwrap().push(format!("eligibility {organization_uuid}"));
            Ok(info::fixtures::eligibility())
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
            let oauth_account = json!({"accountUuid": uuid, "organizationUuid": org(uuid)});
            fs::write(&paths.global_config, json!({"oauthAccount": oauth_account}).to_string()).unwrap();
            manager.save(&name(account)).unwrap();
        }
        Setup { _home: home, paths, manager, client }
    }

    /// The organization `setup` gives an account: a uuid, as the real ones are.
    fn org(uuid: &str) -> String {
        format!("{:08x}-0000-4000-8000-000000000000", uuid.as_bytes()[0])
    }

    /// Adds Claude Code's guest pass cache to the main global config, one entry per `(org, fetched at)`.
    fn cache_passes(s: &Setup, entries: &[(String, Timestamp)]) {
        let mut config = crate::jsonio::read(&s.paths.global_config).unwrap().unwrap();
        for (org, at) in entries {
            let mut entry = info::fixtures::eligibility();
            entry["timestamp"] = at.as_millisecond().into();
            config["passesEligibilityCache"][org] = entry;
        }
        fs::write(&s.paths.global_config, config.to_string()).unwrap();
    }

    fn sorted(requests: &Mutex<Vec<String>>) -> Vec<String> {
        let mut requests = requests.lock().unwrap().clone();
        requests.sort();
        requests
    }

    fn future() -> i64 {
        Timestamp::now().as_millisecond() + 3_600_000
    }

    #[test]
    fn saved_accounts_with_expired_tokens_are_stale_without_refresh() {
        let s = setup(&[("live", "L", "at-L", future()), ("old", "O", "at-O", 1)]);

        let cards = collect(&s.manager, false, false).unwrap();

        assert!(cards[0].current);
        assert_eq!((cards[0].plan.as_deref(), cards[0].plan_label.as_deref()), (Some("pro"), Some("Pro")));
        assert_eq!(cards[1].state, CardState::Stale);
        assert_eq!(*s.client.usage_tokens.lock().unwrap(), ["at-L"]);
    }

    #[test]
    fn the_live_login_is_never_refreshed() {
        let s = setup(&[("live", "L", "at-L", 1)]);

        let cards = collect(&s.manager, true, false).unwrap();

        assert!(matches!(&cards[0].state, CardState::Error { message } if message.contains("send any message")));
        assert!(s.client.usage_tokens.lock().unwrap().is_empty());
    }

    #[test]
    fn refresh_saves_the_rotated_login_and_uses_it() {
        let s = setup(&[("live", "L", "at-L", future()), ("old", "O", "at-O", 1)]);

        let cards = collect(&s.manager, true, false).unwrap();

        let saved = crate::jsonio::read(&s.paths.vault_dir.join("old/login.json")).unwrap().unwrap();
        assert_eq!(saved["claudeAiOauth"]["refreshToken"], "rt-fresh");
        assert!(matches!(&cards[1].state, CardState::Ok { windows, .. } if windows[0].percent == 80.0));
        assert!(s.client.usage_tokens.lock().unwrap().contains(&"fresh-at-O".to_string()));
    }

    #[test]
    fn plain_usage_reads_usage_and_nothing_else() {
        let s = setup(&[("live", "L", "at-L", future()), ("old", "O", "at-O", future())]);

        let cards = collect(&s.manager, false, false).unwrap();

        assert_eq!(sorted(&s.client.usage_tokens), ["at-L", "at-O"]);
        assert!(s.client.info_requests.lock().unwrap().is_empty());
        assert!(cards.iter().all(|c| c.info.is_none()));
    }

    #[test]
    fn info_reads_each_profile_with_its_own_token_and_guest_passes_only_without_a_fresh_cache() {
        let s =
            setup(&[("live", "L", "at-L", future()), ("old", "O", "at-O", future()), ("new", "N", "at-N", future())]);
        let hours_ago = |h: i64| Timestamp::now() - jiff::SignedDuration::from_hours(h);
        cache_passes(&s, &[(org("L"), hours_ago(3)), (org("O"), hours_ago(25))]);

        let cards = collect(&s.manager, false, true).unwrap();

        assert_eq!(
            sorted(&s.client.info_requests),
            [
                format!("eligibility {}", org("N")),
                format!("eligibility {}", org("O")),
                "profile at-L".into(),
                "profile at-N".into(),
                "profile at-O".into()
            ]
        );
        assert_eq!(sorted(&s.client.usage_tokens), ["at-L", "at-N", "at-O"], "usage is read once per account");
        let passes = |i: usize| match &cards[i].info.as_ref().unwrap().guest_passes {
            Some(info::Fetched::Read(passes)) => passes.clone(),
            other => panic!("{other:?}"),
        };
        assert!(passes(0).cached_at.is_some(), "the fresh cache is used as it is");
        assert_eq!((passes(1).cached_at, passes(1).remaining_passes), (None, Some(2)));
        let profile = &cards[0].info.as_ref().unwrap().profile;
        assert!(matches!(profile, info::Fetched::Read(p) if p.subscription_status.as_deref() == Some("past_due")));
    }

    #[test]
    fn info_requests_nothing_for_an_account_without_a_usable_token_or_guest_passes() {
        let s = setup(&[("live", "L", "at-L", future()), ("old", "O", "at-O", 1)]);
        let live_login = s.paths.vault_dir.join("live/login.json");
        let mut saved = crate::jsonio::read(&live_login).unwrap().unwrap();
        saved["claudeAiOauth"]["subscriptionType"] = "team".into();
        fs::write(&live_login, saved.to_string()).unwrap();
        let mut live = crate::jsonio::read(&s.paths.credentials_file).unwrap().unwrap();
        live["claudeAiOauth"]["subscriptionType"] = "team".into();
        fs::write(&s.paths.credentials_file, live.to_string()).unwrap();

        let cards = collect(&s.manager, false, true).unwrap();

        assert_eq!(*s.client.info_requests.lock().unwrap(), ["profile at-L"], "a Team plan has no guest passes");
        let old = cards[1].info.as_ref().unwrap();
        assert_eq!(old.profile, info::Fetched::Unavailable { unavailable: "no usable access token".into() });
        assert_eq!(old.guest_passes, None);
        assert_eq!(cards[1].state, CardState::Stale);
    }

    #[test]
    fn info_lines_follow_the_usage_inside_the_card_and_no_secret_shows() {
        let s = setup(&[("live", "L", "at-L", future())]);
        let cards = collect(&s.manager, false, true).unwrap();

        let out = render(&cards, Timestamp::now(), false);
        let json = serde_json::to_string(&cards).unwrap();

        assert!(out.contains("│ 7 day        "), "{out}");
        assert!(out.contains("│ name: Test (Test Person) "), "{out}");
        assert!(out.contains("│ guest passes: 2 of 3 left, $10.00 reward "), "{out}");
        let widths: std::collections::BTreeSet<_> = out.lines().map(|l| l.chars().count()).collect();
        assert_eq!(widths.len(), 1, "{out}");
        assert!(!(out.clone() + &json).contains("SECRET"), "{out}{json}");
        assert!(!json.contains("http"), "{json}");
    }

    #[test]
    fn a_dead_saved_login_is_reported_and_left_untouched() {
        let s = setup(&[("live", "L", "at-L", future()), ("old", "O", "at-O-dead", 1)]);

        let cards = collect(&s.manager, true, false).unwrap();

        assert_eq!(cards[1].state, CardState::Error { message: ApiError::LoginExpired.to_string() });
        let saved = crate::jsonio::read(&s.paths.vault_dir.join("old/login.json")).unwrap().unwrap();
        assert_eq!(saved["claudeAiOauth"]["refreshToken"], "rt-at-O-dead");
    }
}
