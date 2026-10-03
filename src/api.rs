//! Anthropic OAuth endpoints used by Claude Code.
//!
//! None of these are documented public API. They are the endpoints Claude Code itself calls, as identified by
//! claude-swap (`src/claude_swap/oauth.py`), and they can change without notice. Every caller degrades
//! gracefully when they fail.

use std::time::Duration;

use serde_json::{Value, json};

use crate::login::{OAuthLogin, UnixMillis};

/// Claude Code's public OAuth client id.
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const OAUTH_BETA: &str = "oauth-2025-04-20";
const USER_AGENT: &str = concat!("ccam/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone)]
pub struct Endpoints {
    pub usage: String,
    pub profile: String,
    pub token: String,
}

impl Endpoints {
    /// The real endpoints, or all three under `CCAM_API_BASE` when set. The override exists for tests and local
    /// mock servers; production use never sets it.
    pub fn from_env() -> Self {
        match std::env::var("CCAM_API_BASE") {
            Ok(base) if !base.is_empty() => {
                let base = base.trim_end_matches('/');
                Self {
                    usage: format!("{base}/api/oauth/usage"),
                    profile: format!("{base}/api/oauth/profile"),
                    token: format!("{base}/v1/oauth/token"),
                }
            }
            _ => Self::default(),
        }
    }
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            usage: "https://api.anthropic.com/api/oauth/usage".into(),
            profile: "https://api.anthropic.com/api/oauth/profile".into(),
            token: "https://platform.claude.com/v1/oauth/token".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ApiError {
    #[error("the token was rejected")]
    Unauthorized,
    #[error("login expired, run /login in Claude Code and save the account again")]
    LoginExpired,
    #[error("rate limited, try again in a minute")]
    RateLimited,
    #[error("timed out")]
    Timeout,
    #[error("unexpected response ({0})")]
    Status(u16),
    #[error("unexpected response body")]
    Malformed,
    #[error("network error: {0}")]
    Transport(String),
}

/// The Anthropic OAuth calls ccam makes. A trait so account logic can be tested without a network.
pub trait OAuthClient: Send + Sync {
    /// The account that owns an access token.
    fn account_uuid(&self, access_token: &str) -> Result<String, ApiError>;
    /// Exchanges the login's refresh token for a new generation. The old refresh token is spent the moment the
    /// server answers, so the caller must persist the result before doing anything else.
    fn refresh(&self, login: &OAuthLogin) -> Result<OAuthLogin, ApiError>;
    /// Raw usage JSON for the account owning an access token.
    fn usage(&self, access_token: &str) -> Result<Value, ApiError>;
}

pub struct Api {
    agent: ureq::Agent,
    endpoints: Endpoints,
}

impl Api {
    pub fn new(endpoints: Endpoints, timeout: Duration) -> Self {
        // A build without TLS cannot reach Anthropic; failing at startup beats a confusing error later.
        #[cfg(not(feature = "tls"))]
        if endpoints.usage.starts_with("https://") {
            panic!("this ccam build has no TLS support; rebuild with the default features");
        }
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .http_status_as_error(false)
            .user_agent(USER_AGENT)
            .build()
            .into();
        Self { agent, endpoints }
    }

    fn get_with_token(&self, url: &str, access_token: &str, on_rejected: ApiError) -> Result<Value, ApiError> {
        let response = self
            .agent
            .get(url)
            .header("Authorization", &format!("Bearer {access_token}"))
            .header("anthropic-beta", OAUTH_BETA)
            .call();
        json_body(response, on_rejected)
    }
}

impl OAuthClient for Api {
    fn account_uuid(&self, access_token: &str) -> Result<String, ApiError> {
        let body = self.get_with_token(&self.endpoints.profile, access_token, ApiError::Unauthorized)?;
        body["account"]["uuid"].as_str().filter(|uuid| !uuid.is_empty()).map(str::to_string).ok_or(ApiError::Malformed)
    }

    fn refresh(&self, login: &OAuthLogin) -> Result<OAuthLogin, ApiError> {
        let response = self.agent.post(&self.endpoints.token).send_json(json!({
            "grant_type": "refresh_token",
            "refresh_token": login.refresh_token(),
            "client_id": CLIENT_ID,
        }));
        // The token endpoint answers a spent or revoked refresh token with 400 or 401.
        let body = json_body(response, ApiError::LoginExpired)?;
        let access_token = body["access_token"].as_str().filter(|t| !t.is_empty()).ok_or(ApiError::Malformed)?;
        let expires_in = body["expires_in"].as_i64().ok_or(ApiError::Malformed)?;
        let expires_at = UnixMillis::now().plus(Duration::from_secs(expires_in.max(0).unsigned_abs()));
        Ok(login.refreshed(access_token, body["refresh_token"].as_str(), expires_at))
    }

    fn usage(&self, access_token: &str) -> Result<Value, ApiError> {
        self.get_with_token(&self.endpoints.usage, access_token, ApiError::Unauthorized)
    }
}

fn json_body(
    response: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    on_rejected: ApiError,
) -> Result<Value, ApiError> {
    let mut response = response.map_err(|e| match e {
        ureq::Error::Timeout(_) => ApiError::Timeout,
        other => ApiError::Transport(other.to_string()),
    })?;
    match response.status().as_u16() {
        200..=299 => response.body_mut().read_json().map_err(|_| ApiError::Malformed),
        400 | 401 | 403 => Err(on_rejected),
        429 => Err(ApiError::RateLimited),
        status => Err(ApiError::Status(status)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testserver::{Reply, TestServer};

    fn api(server: &TestServer) -> Api {
        Api::new(
            Endpoints { usage: server.url("/usage"), profile: server.url("/profile"), token: server.url("/token") },
            Duration::from_millis(500),
        )
    }

    fn login() -> OAuthLogin {
        OAuthLogin::parse(json!({
            "accessToken": "old-at", "refreshToken": "old-rt", "expiresAt": 1, "refreshTokenExpiresAt": 99, "scopes": ["a"]
        }))
        .unwrap()
    }

    #[test]
    fn usage_sends_the_bearer_token_and_beta_header() {
        let server =
            TestServer::start(vec![("/usage", Reply::Json(200, r#"{"five_hour":{"utilization":27}}"#.into()))]);

        let body = api(&server).usage("tok").unwrap();

        assert_eq!(body["five_hour"]["utilization"], 27);
        let request = &server.received()[0];
        assert_eq!(request.headers["authorization"], "Bearer tok");
        assert_eq!(request.headers["anthropic-beta"], OAUTH_BETA);
    }

    #[test]
    fn usage_maps_rejections_rate_limits_and_timeouts() {
        let server = TestServer::start(vec![
            ("/usage", Reply::Json(401, "{}".into())),
            ("/limited", Reply::Json(429, "{}".into())),
            ("/hang", Reply::Hang),
        ]);
        let mut endpoints = Endpoints { usage: server.url("/usage"), profile: String::new(), token: String::new() };
        assert_eq!(Api::new(endpoints.clone(), Duration::from_millis(500)).usage("t"), Err(ApiError::Unauthorized));
        endpoints.usage = server.url("/limited");
        assert_eq!(Api::new(endpoints.clone(), Duration::from_millis(500)).usage("t"), Err(ApiError::RateLimited));
        endpoints.usage = server.url("/hang");
        assert_eq!(Api::new(endpoints, Duration::from_millis(300)).usage("t"), Err(ApiError::Timeout));
    }

    #[test]
    fn refresh_rotates_tokens_and_keeps_the_rest_of_the_login() {
        let server = TestServer::start(vec![(
            "/token",
            Reply::Json(200, r#"{"access_token":"new-at","refresh_token":"new-rt","expires_in":3600}"#.into()),
        )]);

        let refreshed = api(&server).refresh(&login()).unwrap();

        assert_eq!(refreshed.access_token(), "new-at");
        assert_eq!(refreshed.refresh_token(), "new-rt");
        assert!(refreshed.expires_at() > UnixMillis::now());
        assert_eq!(refreshed.to_value()["refreshTokenExpiresAt"], 99);
        assert_eq!(refreshed.to_value()["scopes"], json!(["a"]));

        assert_eq!(server.received()[0].method, "POST");
        let sent: Value = serde_json::from_str(&server.received()[0].body).unwrap();
        assert_eq!(sent, json!({"grant_type": "refresh_token", "refresh_token": "old-rt", "client_id": CLIENT_ID}));
    }

    #[test]
    fn refresh_reports_a_dead_login() {
        let server = TestServer::start(vec![("/token", Reply::Json(400, r#"{"error":"invalid_grant"}"#.into()))]);
        assert_eq!(api(&server).refresh(&login()), Err(ApiError::LoginExpired));
    }

    #[test]
    fn profile_lookup_returns_the_account_uuid() {
        let server = TestServer::start(vec![(
            "/profile",
            Reply::Json(200, r#"{"account":{"uuid":"acc-1","email":"x"}}"#.into()),
        )]);
        assert_eq!(api(&server).account_uuid("tok").unwrap(), "acc-1");
    }
}
