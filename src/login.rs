use std::fmt;
use std::time::Duration;

use serde_json::{Map, Value};

/// A point in time as Claude Code stores it: milliseconds since the Unix epoch. A distinct type so expiries,
/// clocks and buffers cannot be mixed up as plain integers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct UnixMillis(i64);

impl UnixMillis {
    pub const fn from_millis(ms: i64) -> Self {
        Self(ms)
    }

    pub fn now() -> Self {
        Self(jiff::Timestamp::now().as_millisecond())
    }

    pub fn as_millis(self) -> i64 {
        self.0
    }

    pub fn plus(self, d: Duration) -> Self {
        Self(self.0.saturating_add(i64::try_from(d.as_millis()).unwrap_or(i64::MAX)))
    }

    fn minus(self, d: Duration) -> Self {
        Self(self.0.saturating_sub(i64::try_from(d.as_millis()).unwrap_or(i64::MAX)))
    }
}

/// Claude Code's `claudeAiOauth` login, parsed once where it is read.
///
/// The typed fields are what ccam reasons about; `raw` keeps the object verbatim so fields ccam does not know
/// about survive a round trip. A login without tokens or an expiry cannot be ordered against other
/// generations, so it does not parse: callers treat it as unknown and refuse to overwrite anything with it.
#[derive(Clone, PartialEq)]
pub struct OAuthLogin {
    access_token: String,
    refresh_token: String,
    expires_at: UnixMillis,
    session_expires_at: Option<i64>,
    subscription_type: Option<String>,
    raw: Map<String, Value>,
}

impl OAuthLogin {
    /// Claude Code keeps the login's shape but blanks both tokens when a renewal is refused. Nothing in it can be
    /// saved or lost, so it is a logged-out state rather than an unreadable one.
    pub fn is_cleared(value: &Value) -> bool {
        let blank = |key| value.get(key).and_then(Value::as_str).is_none_or(str::is_empty);
        blank("accessToken") && blank("refreshToken")
    }

    pub fn parse(value: Value) -> Result<Self, &'static str> {
        let Value::Object(raw) = value else { return Err("login is not a JSON object") };
        let text = |key: &str| raw.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
        Ok(Self {
            access_token: text("accessToken").ok_or("login has no access token")?,
            refresh_token: text("refreshToken").ok_or("login has no refresh token")?,
            expires_at: raw
                .get("expiresAt")
                .and_then(Value::as_i64)
                .map(UnixMillis::from_millis)
                .ok_or("login has no expiresAt")?,
            session_expires_at: raw.get("refreshTokenExpiresAt").and_then(Value::as_i64),
            subscription_type: text("subscriptionType"),
            raw,
        })
    }

    pub fn to_value(&self) -> Value {
        Value::Object(self.raw.clone())
    }

    pub fn access_token(&self) -> &str {
        &self.access_token
    }

    pub fn refresh_token(&self) -> &str {
        &self.refresh_token
    }

    pub fn subscription_type(&self) -> Option<&str> {
        self.subscription_type.as_deref()
    }

    /// Access tokens this close to expiry count as expired, so a request never races the deadline.
    pub const EXPIRY_BUFFER: Duration = Duration::from_secs(5 * 60);

    pub fn expires_at(&self) -> UnixMillis {
        self.expires_at
    }

    /// Whether the access token can still be sent at `now`, with the safety buffer applied.
    pub fn usable_at(&self, now: UnixMillis) -> bool {
        now < self.expires_at.minus(Self::EXPIRY_BUFFER)
    }

    /// Every refresh and every login issues a token that expires later than the one before, so `expiresAt`
    /// orders generations of the same account. Refresh tokens are single-use: an older generation is dead
    /// once a newer one exists, and must never replace it.
    pub fn is_newer_than(&self, other: &Self) -> bool {
        self.expires_at > other.expires_at
    }

    /// Whether both are renewals of the same `/login`. A renewal replaces the tokens but keeps the login's own
    /// expiry (`refreshTokenExpiresAt`); a new login gets a new one. Logins without the field never match.
    ///
    /// This is an observation of Claude Code's behaviour, not a documented contract. When it does not hold,
    /// ccam falls back to confirming ownership over the network, which is slower but still safe.
    pub fn same_session_as(&self, other: &Self) -> bool {
        matches!((self.session_expires_at, other.session_expires_at), (Some(a), Some(b)) if a == b)
    }

    /// A copy carrying the tokens from a refresh grant.
    pub fn refreshed(&self, access_token: &str, refresh_token: Option<&str>, expires_at: UnixMillis) -> Self {
        let mut raw = self.raw.clone();
        raw.insert("accessToken".into(), access_token.into());
        raw.insert("expiresAt".into(), expires_at.as_millis().into());
        if let Some(rt) = refresh_token {
            raw.insert("refreshToken".into(), rt.into());
        }
        Self::parse(Value::Object(raw)).expect("a parsed login with fresh tokens still parses")
    }
}

/// Tokens never reach logs or error messages.
impl fmt::Debug for OAuthLogin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuthLogin")
            .field("expires_at", &self.expires_at)
            .field("session_expires_at", &self.session_expires_at)
            .field("subscription_type", &self.subscription_type)
            .finish_non_exhaustive()
    }
}

/// Who an `oauthAccount` belongs to. Email is deliberately not part of identity: it is display data and one
/// email can belong to several organizations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub account_uuid: String,
    pub organization_uuid: Option<String>,
}

impl Identity {
    pub fn from_oauth_account(value: &Value) -> Option<Self> {
        let account_uuid = value.get("accountUuid")?.as_str().filter(|s| !s.is_empty())?.to_string();
        let organization_uuid = value.get("organizationUuid").and_then(Value::as_str).map(str::to_string);
        Some(Self { account_uuid, organization_uuid })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn login(expires_at: i64, session: Option<i64>) -> OAuthLogin {
        let mut v = json!({"accessToken": "secret-at", "refreshToken": "secret-rt", "expiresAt": expires_at});
        if let Some(s) = session {
            v["refreshTokenExpiresAt"] = s.into();
        }
        OAuthLogin::parse(v).unwrap()
    }

    #[test]
    fn logins_without_tokens_or_expiry_do_not_parse() {
        assert!(OAuthLogin::parse(json!({"accessToken": "a", "refreshToken": "r"})).is_err());
        assert!(OAuthLogin::parse(json!({"accessToken": "", "refreshToken": "r", "expiresAt": 1})).is_err());
        assert!(OAuthLogin::parse(json!({"refreshToken": "r", "expiresAt": 1})).is_err());
        assert!(OAuthLogin::parse(json!([1])).is_err());
    }

    #[test]
    fn later_expiry_is_newer_and_equal_expiry_is_not() {
        assert!(login(200, None).is_newer_than(&login(100, None)));
        assert!(!login(100, None).is_newer_than(&login(200, None)));
        assert!(!login(100, None).is_newer_than(&login(100, None)));
    }

    #[test]
    fn same_session_requires_matching_known_login_expiry() {
        assert!(login(1, Some(9)).same_session_as(&login(2, Some(9))));
        assert!(!login(1, Some(9)).same_session_as(&login(2, Some(8))));
        assert!(!login(1, None).same_session_as(&login(2, None)));
    }

    #[test]
    fn refreshing_replaces_tokens_and_keeps_unknown_fields() {
        let original =
            OAuthLogin::parse(json!({"accessToken": "a", "refreshToken": "r", "expiresAt": 1, "scopes": ["x"]}))
                .unwrap();
        let refreshed = original.refreshed("a2", Some("r2"), UnixMillis::from_millis(5));
        assert_eq!(
            (refreshed.access_token(), refreshed.refresh_token(), refreshed.expires_at()),
            ("a2", "r2", UnixMillis::from_millis(5))
        );
        assert_eq!(refreshed.to_value()["scopes"], json!(["x"]));
    }

    #[test]
    fn debug_output_never_contains_tokens() {
        let text = format!("{:?}", login(1, None));
        assert!(!text.contains("secret"));
    }

    #[test]
    fn identities_with_different_organizations_are_different_accounts() {
        let id = |org: Option<&str>| {
            Identity::from_oauth_account(&json!({"accountUuid": "u", "organizationUuid": org})).unwrap()
        };
        assert_eq!(id(Some("o1")), id(Some("o1")));
        assert_ne!(id(Some("o1")), id(Some("o2")));
        assert_ne!(id(Some("o1")), id(None));
        assert!(Identity::from_oauth_account(&json!({"accountUuid": ""})).is_none());
    }
}

#[cfg(test)]
mod time_tests {
    use std::time::Duration;

    use serde_json::json;

    use super::*;

    #[test]
    fn a_token_counts_as_expired_within_the_safety_buffer() {
        let login = OAuthLogin::parse(json!({"accessToken": "a", "refreshToken": "r", "expiresAt": 10_000})).unwrap();
        assert!(!login.usable_at(UnixMillis::from_millis(0)));
        assert!(login.usable_at(UnixMillis::from_millis(10_000 - OAuthLogin::EXPIRY_BUFFER.as_millis() as i64 - 1)));
        assert!(!login.usable_at(UnixMillis::from_millis(10_000 - OAuthLogin::EXPIRY_BUFFER.as_millis() as i64)));
        assert_eq!(UnixMillis::from_millis(5).plus(Duration::from_secs(1)), UnixMillis::from_millis(1_005));
    }
}
