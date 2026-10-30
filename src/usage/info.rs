//! `ccam usage --info`: who an account is, how it is billed, and its guest passes. Shown on the terminal and in
//! `--json` only, never logged.
//!
//! The profile and referral responses also hold uuids, the account email, an invoice link and the referral code
//! and links. The parsers here read display fields by name and nothing else, so those never reach a card.

use std::fmt;

use jiff::Timestamp;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::api::OAuthClient;

/// A part of the info that needs a request, or why it could not be read.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Fetched<T> {
    Read(T),
    Unavailable { unavailable: String },
}

/// The display fields of `GET /api/oauth/profile`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Profile {
    pub display_name: Option<String>,
    pub full_name: Option<String>,
    pub account_created_at: Option<Timestamp>,
    pub organization_name: Option<String>,
    pub organization_type: Option<String>,
    pub rate_limit_tier: Option<String>,
    pub billing_type: Option<String>,
    /// Undocumented and unread by Claude Code, so shown as returned when enum-like.
    pub subscription_status: Option<String>,
    pub subscription_created_at: Option<Timestamp>,
    pub claude_code_trial_ends_at: Option<Timestamp>,
    pub seat_tier: Option<String>,
    pub has_extra_usage_enabled: Option<bool>,
}

/// The dollar credit window Claude Code shows as "Credit" (`wattle_ember` in the usage response). Its `label` is
/// free text from the server and is never read.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Credit {
    pub used_dollars: Option<f64>,
    pub limit_dollars: Option<f64>,
    pub resets_at: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Reward {
    pub amount_minor_units: f64,
    pub currency: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GuestPasses {
    pub eligible: bool,
    pub remaining_passes: Option<i64>,
    pub limit: Option<i64>,
    pub reward: Option<Reward>,
    /// When Claude Code cached this answer in the global config; absent when ccam asked just now.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_at: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Info {
    #[serde(flatten)]
    pub profile: Fetched<Profile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_usage_disabled_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit: Option<Credit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guest_passes: Option<Fetched<GuestPasses>>,
    #[serde(skip_serializing_if = "Raw::is_empty")]
    pub raw: Raw,
}

/// The usage response's top-level keys ccam does not interpret, keeping only leaves that cannot carry a link,
/// email, id, code or token: numbers, booleans, nulls and strings that parse as ISO 8601 timestamps. Every other
/// string is dropped, and so is any key not shaped `^[a-z0-9_]{1,48}$` and any object or array left empty. JSON
/// only; `Debug` prints none of it.
#[derive(Clone, Default, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Raw(Map<String, Value>);

impl Raw {
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Raw {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Raw(..)")
    }
}

fn safe_object<'a>(entries: impl Iterator<Item = (&'a String, &'a Value)>) -> Map<String, Value> {
    entries.filter(|(key, _)| is_identifier(key)).filter_map(|(key, value)| Some((key.clone(), safe(value)?))).collect()
}

fn safe(value: &Value) -> Option<Value> {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => Some(value.clone()),
        Value::String(s) => s.parse::<Timestamp>().is_ok().then(|| value.clone()),
        Value::Array(items) => {
            let kept: Vec<Value> = items.iter().filter_map(safe).collect();
            (!kept.is_empty()).then_some(Value::Array(kept))
        }
        Value::Object(map) => {
            let kept = safe_object(map.iter());
            (!kept.is_empty()).then_some(Value::Object(kept))
        }
    }
}

fn is_identifier(s: &str) -> bool {
    (1..=48).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// An enum-like server string as returned when it is shaped `^[a-z0-9_]{1,48}$`, else "unrecognised", so free text
/// or an identifier in such a field never reaches a card.
fn enum_like(value: &Value) -> Option<String> {
    let text = value.as_str().filter(|s| !s.is_empty())?;
    Some(if is_identifier(text) { text } else { "unrecognised" }.to_string())
}

/// Where an account's guest passes can be found: Claude Code's cache in the main global config, keyed by
/// organization, and failing that the referral eligibility route for the same organization.
pub struct PassesSource<'a> {
    pub organization_uuid: &'a str,
    pub global_config: &'a Value,
}

/// Claude Code offers guest passes to Pro and Max subscribers only.
pub fn has_guest_passes(plan: Option<&str>) -> bool {
    matches!(plan, Some("pro" | "max"))
}

/// Claude Code refetches its cached eligibility after a day.
const PASSES_CACHE_TTL_MS: i64 = 24 * 60 * 60 * 1000;

/// One account's info: at most one profile request and one eligibility request, both with `token`, the access
/// token its usage was just read with. Without a usable token nothing is requested.
pub fn gather(
    client: &dyn OAuthClient,
    token: Option<&str>,
    usage: Option<&Value>,
    passes: Option<PassesSource<'_>>,
    now: Timestamp,
) -> Info {
    let profile = match token {
        Some(token) => client
            .profile(token)
            .map_or_else(|e| unavailable(&e.to_string()), |body| Fetched::Read(Profile::parse(&body))),
        None => unavailable("no usable access token"),
    };
    let guest_passes = passes.and_then(|source| {
        if let Some(cached) = fresh_cached_passes(source.global_config, source.organization_uuid, now) {
            return Some(Fetched::Read(cached));
        }
        let body = client.referral_eligibility(token?, source.organization_uuid);
        Some(match body.as_ref().map(GuestPasses::parse) {
            Ok(Some(passes)) => Fetched::Read(passes),
            Ok(None) => unavailable(&crate::api::ApiError::Malformed.to_string()),
            Err(e) => unavailable(&e.to_string()),
        })
    });
    let usage = usage.and_then(Value::as_object);
    Info {
        profile,
        extra_usage_disabled_reason: usage
            .and_then(|u| u.get("extra_usage"))
            .filter(|e| e["is_enabled"] == false)
            .and_then(|e| enum_like(&e["disabled_reason"])),
        credit: usage.and_then(|u| u.get("wattle_ember")).filter(|c| c.is_object()).map(|c| Credit {
            used_dollars: c["used_dollars"].as_f64(),
            limit_dollars: c["limit_dollars"].as_f64(),
            resets_at: timestamp(&c["resets_at"]),
        }),
        guest_passes,
        raw: Raw(safe_object(
            usage.into_iter().flatten().filter(|(key, _)| !INTERPRETED_USAGE_KEYS.contains(&key.as_str())),
        )),
    }
}

/// Usage keys ccam reads into windows, extra usage or the credit window; everything else goes to `raw`.
const INTERPRETED_USAGE_KEYS: [&str; 5] = ["five_hour", "seven_day", "limits", "extra_usage", "wattle_ember"];

fn unavailable<T>(reason: &str) -> Fetched<T> {
    Fetched::Unavailable { unavailable: reason.to_string() }
}

fn timestamp(value: &Value) -> Option<Timestamp> {
    value.as_str().and_then(|s| s.parse().ok())
}

impl Profile {
    pub fn parse(body: &Value) -> Self {
        let (account, org) = (&body["account"], &body["organization"]);
        let text = |value: &Value| value.as_str().filter(|s| !s.is_empty()).map(str::to_string);
        Self {
            display_name: text(&account["display_name"]),
            full_name: text(&account["full_name"]),
            account_created_at: timestamp(&account["created_at"]),
            organization_name: text(&org["name"]),
            organization_type: enum_like(&org["organization_type"]),
            rate_limit_tier: enum_like(&org["rate_limit_tier"]),
            billing_type: enum_like(&org["billing_type"]),
            subscription_status: enum_like(&org["subscription_status"]),
            subscription_created_at: timestamp(&org["subscription_created_at"]),
            claude_code_trial_ends_at: timestamp(&org["claude_code_trial_ends_at"]),
            seat_tier: enum_like(&org["seat_tier"]),
            has_extra_usage_enabled: org["has_extra_usage_enabled"].as_bool(),
        }
    }
}

impl GuestPasses {
    /// Reads the counts and the reward from an eligibility response or a cache entry, never the code or links.
    pub fn parse(value: &Value) -> Option<Self> {
        let reward = &value["referrer_reward"];
        Some(Self {
            eligible: value["eligible"].as_bool()?,
            remaining_passes: value["remaining_passes"].as_i64(),
            limit: value["limit"].as_i64(),
            reward: reward["amount_minor_units"].as_f64().map(|amount| Reward {
                amount_minor_units: amount,
                currency: super::currency_code(&reward["currency"]),
            }),
            cached_at: None,
        })
    }
}

/// The cached eligibility for an organization, when Claude Code fetched it less than a day ago.
pub fn fresh_cached_passes(global_config: &Value, organization_uuid: &str, now: Timestamp) -> Option<GuestPasses> {
    let entry = &global_config["passesEligibilityCache"][organization_uuid];
    let cached_at = Timestamp::from_millisecond(entry["timestamp"].as_i64()?).ok()?;
    if now.as_millisecond() - cached_at.as_millisecond() >= PASSES_CACHE_TTL_MS {
        return None;
    }
    GuestPasses::parse(entry).map(|passes| GuestPasses { cached_at: Some(cached_at), ..passes })
}

pub fn billing_channel(billing_type: &str) -> &str {
    match billing_type {
        "stripe_subscription" => "card (Stripe)",
        "apple_subscription" => "Apple",
        "google_play_subscription" => "Google Play",
        "aws_marketplace" => "AWS Marketplace",
        "usage_based" => "usage-based",
        other => other,
    }
}

/// The lines printed under a card's usage.
pub fn lines(info: &Info, now: Timestamp) -> Vec<String> {
    let mut out = match &info.profile {
        Fetched::Read(profile) => profile_lines(profile),
        Fetched::Unavailable { unavailable } => vec![format!("info unavailable: {unavailable}")],
    };
    out.extend(info.extra_usage_disabled_reason.as_ref().map(|reason| format!("extra usage disabled: {reason}")));
    if let Some(credit) = &info.credit {
        let dollars = |amount: Option<f64>| super::money(amount.unwrap_or(0.0) * 100.0, 2, Some("USD"));
        let reset = credit.resets_at.map(|at| format!(", {}", super::reset_text(now, at))).unwrap_or_default();
        out.push(format!("Credit: {} / {}{reset}", dollars(credit.used_dollars), dollars(credit.limit_dollars)));
    }
    out.extend(info.guest_passes.as_ref().map(|passes| match passes {
        Fetched::Read(passes) => format!("guest passes: {}", passes_text(passes, now)),
        Fetched::Unavailable { unavailable } => format!("guest passes unavailable: {unavailable}"),
    }));
    out
}

fn profile_lines(p: &Profile) -> Vec<String> {
    let date = |at: &Timestamp| at.strftime("%Y-%m-%d").to_string();
    let name = match (&p.display_name, &p.full_name) {
        (Some(display), Some(full)) if display != full => Some(format!("{display} ({full})")),
        (display, full) => display.clone().or_else(|| full.clone()),
    };
    let organization = p.organization_name.as_ref().map(|name| match &p.organization_type {
        Some(kind) => format!("{name} ({kind})"),
        None => name.clone(),
    });
    let trial = match &p.claude_code_trial_ends_at {
        Some(at) => format!("Claude Code trial ends {}", date(at)),
        None => "no Claude Code trial".into(),
    };
    [
        name.map(|n| format!("name: {n}")),
        organization.map(|o| format!("organization: {o}")),
        p.rate_limit_tier.as_ref().map(|t| format!("rate limit tier: {t}")),
        p.billing_type.as_deref().map(|b| format!("billing: {}", billing_channel(b))),
        p.subscription_status.as_ref().map(|s| format!("subscription status: {s}")),
        p.subscription_created_at.as_ref().map(|at| format!("subscription created: {}", date(at))),
        p.account_created_at.as_ref().map(|at| format!("account created: {}", date(at))),
        Some(trial),
        p.seat_tier.as_ref().map(|t| format!("seat tier: {t}")),
        p.has_extra_usage_enabled.map(|on| format!("extra usage: {}", if on { "on" } else { "off" })),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn passes_text(passes: &GuestPasses, now: Timestamp) -> String {
    let mut text = match (passes.eligible, passes.remaining_passes, passes.limit) {
        (false, _, _) => "not eligible".to_string(),
        (true, Some(left), Some(limit)) => format!("{left} of {limit} left"),
        (true, Some(left), None) => format!("{left} left"),
        (true, None, _) => "eligible".to_string(),
    };
    if let Some(reward) = passes.reward.as_ref().filter(|_| passes.eligible) {
        text.push_str(&format!(", {} reward", super::money(reward.amount_minor_units, 2, reward.currency.as_deref())));
    }
    if let Some(at) = passes.cached_at {
        text.push_str(&format!(" (cached {} ago)", age(now, at)));
    }
    text
}

fn age(now: Timestamp, at: Timestamp) -> String {
    let minutes = (now.as_millisecond() - at.as_millisecond()).max(0) / 60_000;
    match minutes {
        m if m < 60 => format!("{m}m"),
        m if m < 24 * 60 => format!("{}h", m / 60),
        m => format!("{}d", m / (24 * 60)),
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    use serde_json::{Value, json};

    /// Shaped like the live profile response (docs/research/billing.md). Every identifier, email and link holds
    /// `SECRET`, so a test can prove none of them is shown.
    pub fn profile() -> Value {
        json!({
            "account": {
                "uuid": "SECRET-account-uuid", "email": "SECRET@example.test", "full_name": "Test Person",
                "display_name": "Test", "created_at": "2024-03-01T10:00:00.123456Z",
                "has_claude_max": false, "has_claude_pro": true,
            },
            "organization": {
                "uuid": "SECRET-org-uuid", "name": "Test Person's Organization", "organization_type": "claude_pro",
                "billing_type": "stripe_subscription", "rate_limit_tier": "default_claude_ai", "seat_tier": null,
                "has_extra_usage_enabled": true, "subscription_status": "past_due",
                "subscription_created_at": "2024-03-01T10:00:11.000001Z",
                "claude_code_trial_ends_at": null, "claude_code_trial_duration_days": null,
                "payment_auth_hosted_invoice_url": "https://invoice.example.test/SECRET-invoice",
                "cc_onboarding_flags": {},
            },
            "application": {"uuid": "SECRET-app-uuid", "name": "Claude Code", "slug": "claude-code"},
            "enabled_plugins": [],
        })
    }

    pub fn eligibility() -> Value {
        json!({
            "eligible": true, "remaining_passes": 2, "limit": 3, "degraded": false,
            "referrer_reward": {"amount_minor_units": 1000, "currency": "USD"},
            "referral_code_details": {
                "code": "SECRET-CODE", "campaign": "claude_code_guest_pass_v2",
                "referral_link": "https://claude.ai/referral/SECRET-LINK",
            },
            "share_link": "https://claude.ai/share/SECRET-SHARE", "terms_url": "https://example.test/SECRET-terms",
        })
    }

    /// A usage response with extra usage off, a credit window and keys ccam does not interpret.
    pub fn usage() -> Value {
        json!({
            "five_hour": {"utilization": 27.0, "resets_at": null},
            "seven_day": {"utilization": 46.0, "resets_at": null},
            "limits": [],
            "extra_usage": {
                "is_enabled": false, "monthly_limit": null, "used_credits": 0.0, "currency": "USD",
                "decimal_places": 2, "disabled_reason": "out_of_credits",
            },
            "wattle_ember": {
                "label": null, "utilization": 32.0, "used_dollars": 3.2, "limit_dollars": 10,
                "remaining_dollars": 6.8, "resets_at": "2026-09-19T15:00:00Z",
            },
            "iguana_necktie": {"utilization": 1.5, "limit_dollars": 20, "used_dollars": 0.3},
            "seven_day_breakdown": {"as_of": "2026-09-17T11:00:00Z", "rows": [{"key": "k", "percent": 4}]},
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    const NOW: &str = "2026-09-17T12:00:00Z";

    fn cache(organization_uuid: &str, timestamp: Timestamp) -> Value {
        let mut entry = fixtures::eligibility();
        entry["timestamp"] = timestamp.as_millisecond().into();
        json!({"projects": {}, "passesEligibilityCache": {organization_uuid: entry}})
    }

    fn info() -> Info {
        Info {
            profile: Fetched::Read(Profile::parse(&fixtures::profile())),
            extra_usage_disabled_reason: Some("out_of_credits".into()),
            credit: Some(Credit {
                used_dollars: Some(3.2),
                limit_dollars: Some(10.0),
                resets_at: Some(ts("2026-09-19T15:00:00Z")),
            }),
            guest_passes: fresh_cached_passes(&cache("org-a", ts("2026-09-17T09:00:00Z")), "org-a", ts(NOW))
                .map(Fetched::Read),
            raw: Raw::default(),
        }
    }

    #[test]
    fn the_profile_yields_display_fields_only() {
        assert_eq!(
            Profile::parse(&fixtures::profile()),
            Profile {
                display_name: Some("Test".into()),
                full_name: Some("Test Person".into()),
                account_created_at: Some(ts("2024-03-01T10:00:00.123456Z")),
                organization_name: Some("Test Person's Organization".into()),
                organization_type: Some("claude_pro".into()),
                rate_limit_tier: Some("default_claude_ai".into()),
                billing_type: Some("stripe_subscription".into()),
                subscription_status: Some("past_due".into()),
                subscription_created_at: Some(ts("2024-03-01T10:00:11.000001Z")),
                claude_code_trial_ends_at: None,
                seat_tier: None,
                has_extra_usage_enabled: Some(true),
            }
        );
    }

    #[test]
    fn info_lines_show_dates_the_status_verbatim_the_trial_and_billing_in_words() {
        assert_eq!(
            lines(&info(), ts(NOW)),
            [
                "name: Test (Test Person)",
                "organization: Test Person's Organization (claude_pro)",
                "rate limit tier: default_claude_ai",
                "billing: card (Stripe)",
                "subscription status: past_due",
                "subscription created: 2024-03-01",
                "account created: 2024-03-01",
                "no Claude Code trial",
                "extra usage: on",
                "extra usage disabled: out_of_credits",
                "Credit: $3.20 / $10.00, resets in 2d 3h",
                "guest passes: 2 of 3 left, $10.00 reward (cached 3h ago)",
            ]
        );
    }

    #[test]
    fn a_trial_seat_tier_and_missing_names_change_only_their_lines() {
        let mut body = fixtures::profile();
        body["organization"]["claude_code_trial_ends_at"] = "2026-11-05T00:00:00Z".into();
        body["organization"]["seat_tier"] = "team_standard".into();
        body["account"]["full_name"] = "Test".into();
        body["organization"]["has_extra_usage_enabled"] = false.into();
        let info = Info {
            profile: Fetched::Read(Profile::parse(&body)),
            extra_usage_disabled_reason: None,
            credit: None,
            guest_passes: None,
            raw: Raw::default(),
        };

        let out = lines(&info, ts(NOW));

        assert!(out.contains(&"name: Test".to_string()), "{out:?}");
        assert!(out.contains(&"Claude Code trial ends 2026-11-05".to_string()), "{out:?}");
        assert!(out.contains(&"seat tier: team_standard".to_string()), "{out:?}");
        assert!(out.contains(&"extra usage: off".to_string()), "{out:?}");
        assert!(!out.iter().any(|l| l.contains("guest passes") || l.contains("Credit")), "{out:?}");
    }

    #[test]
    fn billing_types_read_as_words_and_unknown_ones_as_returned() {
        assert_eq!(billing_channel("stripe_subscription"), "card (Stripe)");
        assert_eq!(billing_channel("apple_subscription"), "Apple");
        assert_eq!(billing_channel("google_play_subscription"), "Google Play");
        assert_eq!(billing_channel("aws_marketplace"), "AWS Marketplace");
        assert_eq!(billing_channel("usage_based"), "usage-based");
        assert_eq!(billing_channel("c4e_consumption_trial"), "c4e_consumption_trial");
    }

    #[test]
    fn an_unreadable_profile_says_why() {
        let info = Info {
            profile: Fetched::Unavailable { unavailable: "no usable access token".into() },
            extra_usage_disabled_reason: None,
            credit: None,
            guest_passes: Some(Fetched::Unavailable { unavailable: "timed out".into() }),
            raw: Raw::default(),
        };
        assert_eq!(
            lines(&info, ts(NOW)),
            ["info unavailable: no usable access token", "guest passes unavailable: timed out"]
        );
    }

    #[test]
    fn a_cached_eligibility_counts_only_while_under_a_day_old() {
        let now = ts(NOW);
        let fresh = cache("org-a", ts("2026-09-16T12:00:01Z"));
        let passes = fresh_cached_passes(&fresh, "org-a", now).unwrap();
        assert_eq!(
            passes,
            GuestPasses {
                eligible: true,
                remaining_passes: Some(2),
                limit: Some(3),
                reward: Some(Reward { amount_minor_units: 1000.0, currency: Some("USD".into()) }),
                cached_at: Some(ts("2026-09-16T12:00:01Z")),
            }
        );

        assert_eq!(fresh_cached_passes(&cache("org-a", ts("2026-09-16T11:59:59Z")), "org-a", now), None, "stale");
        assert_eq!(fresh_cached_passes(&fresh, "org-b", now), None, "another organization");
        assert_eq!(fresh_cached_passes(&json!({}), "org-a", now), None, "no cache");
        let no_timestamp = json!({"passesEligibilityCache": {"org-a": fixtures::eligibility()}});
        assert_eq!(fresh_cached_passes(&no_timestamp, "org-a", now), None);
    }

    #[test]
    fn an_ineligible_cache_entry_reads_as_not_eligible() {
        let entry = json!({"eligible": false, "remaining_passes": null, "limit": null, "referrer_reward": null,
            "timestamp": ts("2026-09-17T11:58:00Z").as_millisecond()});
        let config = json!({"passesEligibilityCache": {"org-a": entry}});
        let info = Info {
            profile: Fetched::Unavailable { unavailable: "x".into() },
            extra_usage_disabled_reason: None,
            credit: None,
            guest_passes: fresh_cached_passes(&config, "org-a", ts(NOW)).map(Fetched::Read),
            raw: Raw::default(),
        };
        assert_eq!(lines(&info, ts(NOW))[1], "guest passes: not eligible (cached 2m ago)");
        assert_eq!(GuestPasses::parse(&json!({"limit": 3})), None, "eligibility is required");
    }

    #[test]
    fn guest_passes_are_for_pro_and_max_only() {
        assert!(has_guest_passes(Some("pro")) && has_guest_passes(Some("max")));
        assert!(!has_guest_passes(Some("team")) && !has_guest_passes(Some("enterprise")) && !has_guest_passes(None));
    }

    struct NoNetwork;
    impl OAuthClient for NoNetwork {
        fn account_uuid(&self, _: &str) -> Result<String, crate::api::ApiError> {
            unreachable!()
        }
        fn refresh(&self, _: &crate::login::OAuthLogin) -> Result<crate::login::OAuthLogin, crate::api::ApiError> {
            unreachable!()
        }
        fn usage(&self, _: &str) -> Result<Value, crate::api::ApiError> {
            unreachable!()
        }
        fn profile(&self, _: &str) -> Result<Value, crate::api::ApiError> {
            unreachable!("no token, no request")
        }
        fn referral_eligibility(&self, _: &str, _: &str) -> Result<Value, crate::api::ApiError> {
            unreachable!("no token, no request")
        }
    }

    #[test]
    fn raw_keeps_numbers_booleans_nulls_and_timestamps_and_drops_every_other_string() {
        const UUID: &str = "0a1b2c3d-0000-4000-8000-00000000000f";
        let mut usage = fixtures::usage();
        usage["future_window"] = json!({
            "resets_at": "2026-09-19T15:00:00Z", "used": 1.5, "locked": true, "cap": null,
            "portal": "https://billing.example.test/portal",
            "owner": {"email": "someone@example.test", "id": UUID},
            "codes": ["REFCODE-7Q2X", {"token": "sk-ant-oat01-abcdef"}, 7, "2026-01-01T00:00:00Z"],
            UUID: {"count": 1},
        });
        usage["future_link"] = "https://example.test/link".into();

        let info = gather(&NoNetwork, None, Some(&usage), None, ts(NOW));
        let json = serde_json::to_value(&info).unwrap();

        assert_eq!(
            json["raw"]["future_window"],
            json!({"resets_at": "2026-09-19T15:00:00Z", "used": 1.5, "locked": true, "cap": null,
                "codes": [7, "2026-01-01T00:00:00Z"]})
        );
        assert!(json["raw"].get("future_link").is_none(), "{json}");
        let debug = format!("{info:?}");
        assert!(!debug.contains("future_window") && !debug.contains("iguana_necktie"), "{debug}");
        let shown = json.to_string() + &debug;
        for secret in ["http", "@", UUID, "0a1b2c3d", "REFCODE", "sk-ant", "abcdef"] {
            assert!(!shown.contains(secret), "{secret} shown in {shown}");
        }
    }

    #[test]
    fn enum_like_fields_show_as_returned_only_when_they_look_like_identifiers() {
        let mut body = fixtures::profile();
        body["organization"]["organization_type"] = "Claude Pro".into();
        body["organization"]["rate_limit_tier"] = "https://example.test/tier".into();
        body["organization"]["billing_type"] = "someone@example.test".into();
        body["organization"]["subscription_status"] = "a".repeat(49).into();
        body["organization"]["seat_tier"] = "team-standard".into();
        let mut usage = fixtures::usage();
        usage["extra_usage"]["disabled_reason"] = "see https://example.test/why".into();
        let info = Info {
            profile: Fetched::Read(Profile::parse(&body)),
            ..gather(&NoNetwork, None, Some(&usage), None, ts(NOW))
        };

        let out = lines(&info, ts(NOW));

        for line in [
            "organization: Test Person's Organization (unrecognised)",
            "rate limit tier: unrecognised",
            "billing: unrecognised",
            "subscription status: unrecognised",
            "seat tier: unrecognised",
            "extra usage disabled: unrecognised",
        ] {
            assert!(out.contains(&line.to_string()), "missing {line:?} in {out:?}");
        }
        let json = serde_json::to_value(&info).unwrap().to_string();
        assert!(!json.contains("http") && !json.contains('@') && !json.contains("Claude Pro"), "{json}");

        body["organization"]["subscription_status"] = "a".repeat(48).into();
        body["organization"]["seat_tier"] = "team_2".into();
        let profile = Profile::parse(&body);
        assert_eq!(profile.subscription_status, Some("a".repeat(48)));
        assert_eq!(profile.seat_tier.as_deref(), Some("team_2"));
    }

    #[test]
    fn the_credit_window_is_always_called_credit() {
        let mut usage = fixtures::usage();
        usage["wattle_ember"]["label"] = "Visit https://example.test/claim".into();

        let info = gather(&NoNetwork, None, Some(&usage), None, ts(NOW));

        assert!(lines(&info, ts(NOW)).contains(&"Credit: $3.20 / $10.00, resets in 2d 3h".to_string()));
        let json = serde_json::to_value(&info).unwrap().to_string();
        assert!(!json.contains("http") && !json.contains("Visit"), "{json}");
    }

    #[test]
    fn a_reward_currency_that_is_not_an_iso_code_is_left_out() {
        for currency in ["usd", "US$", "https://example.test", "USDX"] {
            let mut body = fixtures::eligibility();
            body["referrer_reward"]["currency"] = currency.into();
            let passes = GuestPasses::parse(&body).unwrap();
            assert_eq!(passes_text(&passes, ts(NOW)), "2 of 3 left, 10.00 reward", "{currency}");
            let json = serde_json::to_value(&passes).unwrap().to_string();
            assert!(!json.contains(currency), "{json}");
        }
        let mut body = fixtures::eligibility();
        body["referrer_reward"]["currency"] = "EUR".into();
        assert_eq!(passes_text(&GuestPasses::parse(&body).unwrap(), ts(NOW)), "2 of 3 left, 10.00 EUR reward");
    }

    #[test]
    fn the_usage_response_gives_the_credit_window_the_disabled_reason_and_the_raw_rest() {
        let usage = fixtures::usage();
        let info = gather(&NoNetwork, None, Some(&usage), None, ts(NOW));

        assert_eq!(info.profile, Fetched::Unavailable { unavailable: "no usable access token".into() });
        assert_eq!(info.extra_usage_disabled_reason.as_deref(), Some("out_of_credits"));
        assert_eq!(
            info.credit,
            Some(Credit {
                used_dollars: Some(3.2),
                limit_dollars: Some(10.0),
                resets_at: Some(ts("2026-09-19T15:00:00Z")),
            })
        );
        assert_eq!(
            serde_json::to_value(&info.raw).unwrap(),
            json!({
                "iguana_necktie": usage["iguana_necktie"],
                "seven_day_breakdown": {"as_of": "2026-09-17T11:00:00Z", "rows": [{"percent": 4}]},
            })
        );
        assert_eq!(info.guest_passes, None);
    }

    #[test]
    fn json_info_is_flat_snake_case_and_holds_no_identifier_code_or_link() {
        let mut info = info();
        info.raw = Raw(Map::from_iter([("iguana_necktie".to_string(), json!({"used_dollars": 0.3}))]));
        let value = serde_json::to_value(&info).unwrap();

        assert_eq!(value["display_name"], "Test");
        assert_eq!(value["subscription_status"], "past_due");
        assert_eq!(value["guest_passes"]["remaining_passes"], 2);
        assert_eq!(value["raw"]["iguana_necktie"]["used_dollars"], 0.3);
        let text = value.to_string() + &lines(&info, ts(NOW)).join("\n");
        assert!(!text.contains("SECRET") && !text.contains("http"), "{text}");

        let unavailable = Info { profile: Fetched::Unavailable { unavailable: "timed out".into() }, ..info };
        assert_eq!(serde_json::to_value(&unavailable).unwrap()["unavailable"], "timed out");
    }
}
