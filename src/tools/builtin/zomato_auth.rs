//! Zomato phone+OTP authentication tool.
//!
//! Handles the OAuth 2.1 PKCE flow for Zomato's MCP server via direct API calls
//! — no browser required. Designed for Telegram-based users.
//!
//! # Auth flow (4 steps)
//!
//! 1. Agent calls `start_auth(phone)` → IronClaw does:
//!    a. GET `/authorize` with PKCE challenge → captures `login_challenge` +
//!    `oauth2_authentication_csrf` cookie from redirect.
//!    b. POST `/login` with phone + challenge → OTP sent to user.
//! 2. User receives OTP on phone and shares it in Telegram.
//! 3. Agent calls `complete_auth(otp)` → IronClaw does:
//!    a. POST `/verify-otp` with OTP + challenge → returns auth code.
//!    b. POST `/token` with **form-encoded** body → returns access_token.
//! 4. Token stored under `mcp_zomato-mcp-server_access_token` in SecretsStore.
//!
//! # Pending state
//!
//! The PKCE verifier, login_challenge, CSRF cookie, state, and phone are held in
//! an in-memory map keyed by user_id. This is ephemeral — if the server restarts
//! between start_auth and complete_auth the user needs to start over, which is
//! acceptable since OTP sessions are short-lived.
//!
//! # Token naming
//!
//! Token stored under `mcp_zomato-mcp-server_access_token` — the key
//! `McpServerConfig::token_secret_name()` produces for the `zomato-mcp-server`
//! registry entry. The MCP client injects it as `Authorization: Bearer <token>`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::Utc;
use rand::RngCore;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::context::JobContext;
use crate::secrets::{CreateSecretParams, SecretsStore};
use crate::tools::tool::{Tool, ToolError, ToolOutput, require_str, require_str_coerced};

// ── Constants ──────────────────────────────────────────────────────────────────

const ZOMATO_AUTH_BASE: &str = "https://mcp-server.zomato.com";
const ZOMATO_CLIENT_ID: &str = "zomato-mcp-server";
/// Redirect URI whitelisted by Zomato for headless/server-side MCP clients.
const ZOMATO_REDIRECT_URI: &str = "https://vscode.dev/redirect";
/// OTP sessions older than 10 minutes are considered expired.
const OTP_SESSION_TIMEOUT_SECS: i64 = 600;
/// Secret name for the Zomato MCP access token.
const ZOMATO_SERVER_NAME: &str = "zomato-mcp-server";

// ── Pending auth state ─────────────────────────────────────────────────────────

/// Temporary state held between OTP send and verify.
#[derive(Debug, Clone)]
struct PendingAuth {
    login_challenge: String,
    csrf_cookie: String,
    pkce_verifier: String,
    state: String,
    phone_digits: String,
    created_at_unix: i64,
}

// ── Tool struct ────────────────────────────────────────────────────────────────

/// Manages Zomato OAuth authentication via phone + OTP.
///
/// `pending` maps `user_id → PendingAuth` and holds transient state between
/// the two-step auth flow. State is memory-only; clears on restart.
pub struct ZomatoAuthTool {
    secrets: Arc<dyn SecretsStore + Send + Sync>,
    pending: Arc<Mutex<HashMap<String, PendingAuth>>>,
}

impl ZomatoAuthTool {
    pub fn new(secrets: Arc<dyn SecretsStore + Send + Sync>) -> Self {
        Self {
            secrets,
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

// ── Zomato API response types ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct VerifyOtpResponse {
    redirect_uri: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ZomatoTokenResponse {
    access_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    error: Option<String>,
}

// ── PKCE helpers ────────────────────────────────────────────────────────────────

fn generate_pkce() -> (String, String) {
    let mut buf = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut buf);
    let verifier = URL_SAFE_NO_PAD.encode(buf);
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let challenge = URL_SAFE_NO_PAD.encode(hasher.finalize());
    (verifier, challenge)
}

fn generate_state() -> String {
    let mut buf = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

// ── HTTP client ─────────────────────────────────────────────────────────────────

/// Build an HTTP client that follows redirects manually (we need to inspect
/// the redirect Location header) and persists cookies across requests.
fn http_client() -> Result<reqwest::Client, ToolError> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| ToolError::ExecutionFailed(format!("Failed to build HTTP client: {}", e)))
}

// ── Tool implementation ────────────────────────────────────────────────────────

#[async_trait]
impl Tool for ZomatoAuthTool {
    fn name(&self) -> &str {
        "zomato_auth"
    }

    fn description(&self) -> &str {
        "Authenticate with Zomato using phone number and OTP. \
        Use action='start_auth' to send OTP to the user's phone, \
        action='complete_auth' to verify the OTP and store credentials, \
        and action='status' to check if Zomato is already connected."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["start_auth", "complete_auth", "status"],
                    "description": "Action: 'start_auth' sends OTP, 'complete_auth' verifies OTP and stores credentials, 'status' checks connection state"
                },
                "phone": {
                    "type": "string",
                    "description": "Mobile number digits only (required for start_auth)"
                },
                "country_code": {
                    "type": "string",
                    "description": "Country code with + prefix, e.g. '+91' (default: +91)",
                    "default": "+91"
                },
                "otp": {
                    "type": "string",
                    "description": "OTP received on the phone (required for complete_auth)"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        tracing::debug!(
            raw_phone = ?params.get("phone"),
            raw_cc = ?params.get("country_code"),
            "zomato_auth: execute() params"
        );
        let action = require_str(&params, "action")?;
        let user_id = ctx.user_id.clone();

        match action {
            "start_auth" => self.start_auth(&params, &user_id, start).await,
            "complete_auth" => self.complete_auth(&params, &user_id, start).await,
            "status" => self.check_status(&user_id, start).await,
            other => Err(ToolError::InvalidParameters(format!(
                "Unknown action '{}'. Use start_auth, complete_auth, or status.",
                other
            ))),
        }
    }

    fn requires_sanitization(&self) -> bool {
        true
    }

    fn sensitive_params(&self) -> &[&str] {
        &["phone", "otp"]
    }

    fn rate_limit_config(&self) -> Option<crate::tools::tool::ToolRateLimitConfig> {
        // Max 3 OTP requests per minute, 10 per hour — prevents runaway LLM retry loops
        Some(crate::tools::tool::ToolRateLimitConfig::new(3, 10))
    }
}

// ── Action implementations ─────────────────────────────────────────────────────

impl ZomatoAuthTool {
    /// Step 1+2: GET /authorize to get login_challenge + CSRF cookie, then
    /// POST /login to send OTP to the user's phone.
    async fn start_auth(
        &self,
        params: &serde_json::Value,
        user_id: &str,
        start: Instant,
    ) -> Result<ToolOutput, ToolError> {
        let phone_raw = require_str_coerced(params, "phone")?;

        let country_code_raw = match params.get("country_code") {
            Some(serde_json::Value::String(s)) if !s.is_empty() => {
                if s.starts_with('+') {
                    s.clone()
                } else {
                    format!("+{}", s)
                }
            }
            Some(serde_json::Value::Number(n)) => format!("+{}", n),
            _ => "+91".to_string(),
        };
        let country_code = country_code_raw.as_str();

        let phone_digits: String = phone_raw.chars().filter(|c| c.is_ascii_digit()).collect();

        if country_code == "+91" {
            if phone_digits.len() != 10 {
                return Err(ToolError::InvalidParameters(
                    "Indian phone number must be exactly 10 digits.".to_string(),
                ));
            }
            if !phone_digits.starts_with(|c: char| matches!(c, '6'..='9')) {
                return Err(ToolError::InvalidParameters(
                    "Indian mobile numbers start with 6, 7, 8, or 9.".to_string(),
                ));
            }
        } else if phone_digits.len() < 7 || phone_digits.len() > 15 {
            return Err(ToolError::InvalidParameters(
                "Phone number must be 7–15 digits.".to_string(),
            ));
        }

        let (pkce_verifier, pkce_challenge) = generate_pkce();
        let state = generate_state();
        let client = http_client()?;

        // Step 1: GET /authorize → redirect to /consent with login_challenge + CSRF cookie.
        let authorize_url = format!(
            "{ZOMATO_AUTH_BASE}/authorize?\
            response_type=code&\
            client_id={ZOMATO_CLIENT_ID}&\
            redirect_uri={}&\
            code_challenge={pkce_challenge}&\
            code_challenge_method=S256&\
            scope=offline+openid&\
            state={state}",
            urlencoding::encode(ZOMATO_REDIRECT_URI),
        );

        let authorize_resp =
            client.get(&authorize_url).send().await.map_err(|e| {
                ToolError::ExecutionFailed(format!("Could not reach Zomato: {}", e))
            })?;

        // Extract the oauth2_authentication_csrf cookie from Set-Cookie header.
        let csrf_cookie = authorize_resp
            .headers()
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .find_map(|val| {
                let s = val.to_str().ok()?;
                if s.starts_with("oauth2_authentication_csrf=") {
                    // Extract the cookie value (up to first `;`)
                    let after_eq = s.strip_prefix("oauth2_authentication_csrf=")?;
                    Some(after_eq.split(';').next().unwrap_or(after_eq).to_string())
                } else {
                    None
                }
            })
            .ok_or_else(|| {
                ToolError::ExecutionFailed(
                    "Zomato /authorize did not return oauth2_authentication_csrf cookie."
                        .to_string(),
                )
            })?;

        // Extract login_challenge from the Location redirect header.
        let location = authorize_resp
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                ToolError::ExecutionFailed(
                    "Zomato /authorize did not return a redirect Location.".to_string(),
                )
            })?;

        let login_challenge = url::Url::parse(&if location.starts_with('/') {
            format!("{ZOMATO_AUTH_BASE}{location}")
        } else if location.starts_with("./") {
            format!("{ZOMATO_AUTH_BASE}/{}", location.strip_prefix("./").unwrap_or(location))
        } else {
            location.to_string()
        })
        .ok()
        .and_then(|u| {
            u.query_pairs()
                .find(|(k, _)| k == "login_challenge")
                .map(|(_, v)| v.to_string())
        })
        .ok_or_else(|| {
            ToolError::ExecutionFailed(
                "Could not extract login_challenge from Zomato /authorize redirect.".to_string(),
            )
        })?;

        tracing::debug!(
            login_challenge = %login_challenge,
            csrf_cookie_len = csrf_cookie.len(),
            "zomato_auth: captured login_challenge and CSRF cookie"
        );

        // Step 2: POST /login to send OTP.
        let login_body = serde_json::json!({
            "id": phone_digits,
            "type": "phone",
            "login_challenge": login_challenge,
        });

        let login_resp = client
            .post(format!("{ZOMATO_AUTH_BASE}/login"))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(
                reqwest::header::COOKIE,
                format!("oauth2_authentication_csrf={csrf_cookie}"),
            )
            .json(&login_body)
            .send()
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("Could not reach Zomato /login: {}", e))
            })?;

        if !login_resp.status().is_success() {
            let status = login_resp.status();
            let body_text = login_resp.text().await.unwrap_or_default();
            return Err(ToolError::ExecutionFailed(format!(
                "Zomato /login failed (HTTP {}): {}",
                status, body_text
            )));
        }

        tracing::debug!(
            phone = %phone_digits,
            "zomato_auth: OTP request accepted by Zomato"
        );

        let pending = PendingAuth {
            login_challenge,
            csrf_cookie,
            pkce_verifier,
            state,
            phone_digits: phone_digits.clone(),
            created_at_unix: Utc::now().timestamp(),
        };

        self.pending
            .lock()
            .await
            .insert(user_id.to_string(), pending);

        Ok(ToolOutput::text(
            format!(
                "OTP sent to {}{} via SMS. Please share the code you received.",
                country_code, phone_digits
            ),
            start.elapsed(),
        ))
    }

    /// Step 3+4: POST /verify-otp to get auth code, then POST /token
    /// (form-encoded) to get access_token. Store in SecretsStore.
    async fn complete_auth(
        &self,
        params: &serde_json::Value,
        user_id: &str,
        start: Instant,
    ) -> Result<ToolOutput, ToolError> {
        let otp_raw = require_str_coerced(params, "otp")?;
        let otp: String = otp_raw.chars().filter(|c| c.is_ascii_digit()).collect();

        if otp.len() < 4 || otp.len() > 8 {
            return Err(ToolError::InvalidParameters(
                "OTP must be 4–8 digits.".to_string(),
            ));
        }

        // Peek (clone) the pending auth — do NOT remove it yet. If the
        // subsequent /verify-otp or /token calls fail, we need to keep the
        // login_challenge / csrf_cookie / pkce_verifier intact so the user
        // can retry with a fresh OTP without restarting the whole flow.
        // Only the expiry-cleanup path and the final success path remove it.
        let pending = {
            let guard = self.pending.lock().await;
            guard.get(user_id).cloned().ok_or_else(|| {
                ToolError::ExecutionFailed(
                    "No pending Zomato auth found. Please call start_auth first.".to_string(),
                )
            })?
        };

        let age = Utc::now().timestamp() - pending.created_at_unix;
        if age > OTP_SESSION_TIMEOUT_SECS {
            // Expired state is no longer useful — drop it so the next
            // start_auth call begins a clean session.
            self.pending.lock().await.remove(user_id);
            return Err(ToolError::ExecutionFailed(
                "OTP session expired (10-minute limit). Please start over.".to_string(),
            ));
        }

        let client = http_client()?;
        let cookie_header = format!("oauth2_authentication_csrf={}", pending.csrf_cookie);

        // Step 3: POST /verify-otp → redirect_uri containing auth code.
        let verify_body = serde_json::json!({
            "otp": otp,
            "id": pending.phone_digits,
            "type": "phone",
            "login_challenge": pending.login_challenge,
            "client_id": ZOMATO_CLIENT_ID,
            "redirect_uri": ZOMATO_REDIRECT_URI,
            "state": pending.state,
            "scope": "offline openid",
        });

        let verify_resp = client
            .post(format!("{ZOMATO_AUTH_BASE}/verify-otp"))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(reqwest::header::COOKIE, &cookie_header)
            .json(&verify_body)
            .send()
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("Could not reach Zomato /verify-otp: {}", e))
            })?;

        if !verify_resp.status().is_success() {
            let status = verify_resp.status();
            let body_text = verify_resp.text().await.unwrap_or_default();
            return Err(ToolError::ExecutionFailed(format!(
                "OTP verification failed (HTTP {}): {}",
                status, body_text
            )));
        }

        let verify: VerifyOtpResponse = verify_resp.json().await.map_err(|e| {
            ToolError::ExecutionFailed(format!("Invalid verify-OTP response: {}", e))
        })?;

        if let Some(ref err) = verify.error {
            return Err(ToolError::ExecutionFailed(format!(
                "OTP verification error: {}",
                err
            )));
        }

        // Extract auth code from the redirect_uri query string.
        let redirect_uri = verify.redirect_uri.ok_or_else(|| {
            ToolError::ExecutionFailed("No redirect_uri in Zomato verify-OTP response.".to_string())
        })?;

        let auth_code = url::Url::parse(&redirect_uri)
            .ok()
            .and_then(|u| {
                u.query_pairs()
                    .find(|(k, _)| k == "code")
                    .map(|(_, v)| v.to_string())
            })
            .ok_or_else(|| {
                ToolError::ExecutionFailed(
                    "Could not extract auth code from Zomato redirect_uri.".to_string(),
                )
            })?;

        // Step 4: POST /token with form-encoded body (NOT JSON).
        let token_form = [
            ("grant_type", "authorization_code"),
            ("code", &auth_code),
            ("code_verifier", &pending.pkce_verifier),
            ("client_id", ZOMATO_CLIENT_ID),
            ("redirect_uri", ZOMATO_REDIRECT_URI),
        ];

        let token_resp = client
            .post(format!("{ZOMATO_AUTH_BASE}/token"))
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .form(&token_form)
            .send()
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("Could not reach Zomato token endpoint: {}", e))
            })?;

        if !token_resp.status().is_success() {
            let status = token_resp.status();
            let body_text = token_resp.text().await.unwrap_or_default();
            return Err(ToolError::ExecutionFailed(format!(
                "Token exchange failed (HTTP {}): {}",
                status, body_text
            )));
        }

        let token: ZomatoTokenResponse = token_resp.json().await.map_err(|e| {
            ToolError::ExecutionFailed(format!("Invalid token response from Zomato: {}", e))
        })?;

        if let Some(ref err) = token.error {
            return Err(ToolError::ExecutionFailed(format!(
                "Zomato token error: {}",
                err
            )));
        }

        let access_token = token.access_token.ok_or_else(|| {
            ToolError::ExecutionFailed(
                "No access token received from Zomato. The token format may have changed."
                    .to_string(),
            )
        })?;

        // Store the token under the name McpServerConfig::token_secret_name() produces.
        let secret_name = format!("mcp_{ZOMATO_SERVER_NAME}_access_token");
        let mut create_params = CreateSecretParams::new(&secret_name, &access_token)
            .with_provider(format!("mcp:{ZOMATO_SERVER_NAME}"));

        if let Some(secs) = token.expires_in {
            let expires_at = Utc::now() + chrono::Duration::seconds(secs as i64);
            create_params = create_params.with_expiry(expires_at);
        }

        self.secrets
            .create(user_id, create_params)
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to store Zomato token: {e}"))
            })?;

        if let Some(ref refresh) = token.refresh_token {
            let refresh_name = format!("mcp_{ZOMATO_SERVER_NAME}_access_token_refresh_token");
            let refresh_params = CreateSecretParams::new(&refresh_name, refresh)
                .with_provider(format!("mcp:{ZOMATO_SERVER_NAME}"));
            self.secrets
                .create(user_id, refresh_params)
                .await
                .map_err(|e| {
                    ToolError::ExecutionFailed(format!("Failed to store Zomato refresh token: {e}"))
                })?;
        }

        // Auth succeeded end-to-end — now it's safe to drop the pending state.
        self.pending.lock().await.remove(user_id);

        Ok(ToolOutput::text(
            "Zomato connected! You can now order food from Zomato.",
            start.elapsed(),
        ))
    }

    /// Check whether the user has valid Zomato credentials stored.
    async fn check_status(&self, user_id: &str, start: Instant) -> Result<ToolOutput, ToolError> {
        let secret_name = format!("mcp_{ZOMATO_SERVER_NAME}_access_token");
        match self.secrets.get_decrypted(user_id, &secret_name).await {
            Ok(_) => Ok(ToolOutput::text(
                "Zomato is connected. Food ordering tools are available.",
                start.elapsed(),
            )),
            Err(_) => Ok(ToolOutput::text(
                "Zomato is not connected. Use action='start_auth' with your phone number.",
                start.elapsed(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::JobContext;
    use crate::secrets::InMemorySecretsStore;
    use crate::testing::credentials::TEST_CRYPTO_KEY;
    use secrecy::SecretString;

    fn test_secrets() -> Arc<dyn crate::secrets::SecretsStore + Send + Sync> {
        let crypto = Arc::new(
            crate::secrets::SecretsCrypto::new(SecretString::from(TEST_CRYPTO_KEY.to_string()))
                .expect("test crypto key"),
        );
        Arc::new(InMemorySecretsStore::new(crypto))
    }

    #[tokio::test]
    async fn start_auth_accepts_numeric_phone() {
        let tool = ZomatoAuthTool::new(test_secrets());
        let ctx = JobContext::with_user("test-user", "test", "test");

        // LLM sends phone as JSON number — must NOT fail with InvalidParameters
        let params = serde_json::json!({
            "action": "start_auth",
            "phone": 9289289123_u64,
            "country_code": 91
        });

        let result = tool.execute(params, &ctx).await;
        if let Err(ToolError::InvalidParameters(msg)) = &result {
            panic!("parameter parsing must not fail for numeric phone, got: {msg}");
        }
        // Any other error (ExecutionFailed from HTTP, etc.) is expected in tests
    }

    #[tokio::test]
    async fn start_auth_accepts_string_phone() {
        let tool = ZomatoAuthTool::new(test_secrets());
        let ctx = JobContext::with_user("test-user", "test", "test");

        let params = serde_json::json!({
            "action": "start_auth",
            "phone": "9289289123",
            "country_code": "+91"
        });

        let result = tool.execute(params, &ctx).await;
        if let Err(ToolError::InvalidParameters(msg)) = &result {
            panic!("parameter parsing must not fail for string phone, got: {msg}");
        }
    }

    #[tokio::test]
    async fn status_shows_not_connected() {
        let tool = ZomatoAuthTool::new(test_secrets());
        let ctx = JobContext::with_user("test-user", "test", "test");

        let params = serde_json::json!({ "action": "status" });
        let output = tool
            .execute(params, &ctx)
            .await
            .expect("status should not fail");
        let text = output.result.as_str().unwrap_or("");
        assert!(
            text.contains("not connected"),
            "expected 'not connected' message, got: {}",
            text
        );
    }

    /// Regression: if /verify-otp or /token fails, the PendingAuth state
    /// must remain so the user can retry with a fresh OTP without restarting
    /// the whole /authorize → /login flow. Previously `complete_auth`
    /// called `.remove(user_id)` at the top, destroying the state on any
    /// downstream HTTP failure.
    #[tokio::test]
    async fn complete_auth_preserves_pending_on_failure() {
        let tool = ZomatoAuthTool::new(test_secrets());
        let ctx = JobContext::with_user("test-user", "test", "test");

        // Seed a fake pending auth — the login_challenge / csrf_cookie are
        // bogus so /verify-otp will definitely fail (either network error
        // or upstream rejection).
        {
            let mut guard = tool.pending.lock().await;
            guard.insert(
                "test-user".to_string(),
                PendingAuth {
                    login_challenge: "fake-challenge".to_string(),
                    csrf_cookie: "fake-cookie".to_string(),
                    pkce_verifier: "fake-verifier".to_string(),
                    state: "fake-state".to_string(),
                    phone_digits: "9289289123".to_string(),
                    created_at_unix: Utc::now().timestamp(),
                },
            );
        }

        let params = serde_json::json!({
            "action": "complete_auth",
            "otp": "123456",
        });
        let result = tool.execute(params, &ctx).await;
        assert!(
            result.is_err(),
            "complete_auth with fake state must fail, got: {:?}",
            result
        );

        // The critical assertion: pending state must still be present after
        // the failure, so the user can share a new OTP and retry.
        let guard = tool.pending.lock().await;
        assert!(
            guard.contains_key("test-user"),
            "PendingAuth must be retained after /verify-otp or /token failure"
        );
    }

    /// Regression: expired PendingAuth entries should be evicted so the
    /// next start_auth begins cleanly.
    #[tokio::test]
    async fn complete_auth_clears_expired_pending() {
        let tool = ZomatoAuthTool::new(test_secrets());
        let ctx = JobContext::with_user("test-user", "test", "test");

        {
            let mut guard = tool.pending.lock().await;
            guard.insert(
                "test-user".to_string(),
                PendingAuth {
                    login_challenge: "fake".to_string(),
                    csrf_cookie: "fake".to_string(),
                    pkce_verifier: "fake".to_string(),
                    state: "fake".to_string(),
                    phone_digits: "9289289123".to_string(),
                    // More than OTP_SESSION_TIMEOUT_SECS in the past.
                    created_at_unix: Utc::now().timestamp() - (OTP_SESSION_TIMEOUT_SECS + 60),
                },
            );
        }

        let params = serde_json::json!({
            "action": "complete_auth",
            "otp": "123456",
        });
        let _ = tool.execute(params, &ctx).await;

        let guard = tool.pending.lock().await;
        assert!(
            !guard.contains_key("test-user"),
            "expired PendingAuth should be evicted on complete_auth"
        );
    }

    #[test]
    fn test_pkce_pairs_are_unique() {
        let (v1, c1) = generate_pkce();
        let (v2, c2) = generate_pkce();
        assert_ne!(v1, v2, "verifiers should be random and unique");
        assert_ne!(c1, c2, "challenges should differ for different verifiers");
    }
}
