//! Swiggy phone+OTP authentication tool.
//!
//! Handles the MCP OAuth flow for all Swiggy services (food, instamart, dineout)
//! via direct API calls — no browser required. Designed for Telegram-based users.
//!
//! # Auth flow
//!
//! 1. Agent calls `start_auth(phone, country_code)` → IronClaw calls Swiggy
//!    `/auth/send-otp`, stores PKCE state and session in memory.
//! 2. User receives OTP on phone and shares it in Telegram.
//! 3. Agent calls `complete_auth(otp)` → IronClaw calls `/auth/verify-otp` then
//!    `/auth/token`, stores the resulting token for all three Swiggy MCP servers
//!    in the SecretsStore.
//! 4. MCP client finds the token in SecretsStore and uses it as Bearer for
//!    `POST https://mcp.swiggy.com/food`, `/im`, `/dineout`.
//!
//! # Pending state
//!
//! The PKCE verifier + Swiggy session data is held in an in-memory map keyed by
//! user_id. This is ephemeral — if the server restarts between start_auth and
//! complete_auth the user needs to start over, which is acceptable since OTP
//! sessions are short-lived anyway.
//!
//! # Token naming
//!
//! Tokens are stored under the keys that `McpServerConfig::token_secret_name()`
//! produces: `mcp_swiggy-food_access_token`, `mcp_swiggy-instamart_access_token`,
//! `mcp_swiggy-dineout_access_token` — one auth covers all three services.

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
use crate::tools::tool::{Tool, ToolError, ToolOutput, require_str};

// ── Constants ──────────────────────────────────────────────────────────────────

const SWIGGY_MCP_BASE: &str = "https://mcp.swiggy.com";
const SWIGGY_CLIENT_ID: &str = "swiggy-mcp";
/// Redirect URI whitelisted by Swiggy for third-party MCP clients.
const SWIGGY_REDIRECT_URI: &str = "http://localhost/callback";
/// OTP sessions older than 10 minutes are considered expired.
const OTP_SESSION_TIMEOUT_SECS: i64 = 600;
/// Server names covered by a single Swiggy auth session.
const SWIGGY_SERVERS: &[&str] = &["swiggy-food", "swiggy-instamart", "swiggy-dineout"];

// ── Pending auth state ─────────────────────────────────────────────────────────

/// Temporary state held between OTP send and verify.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct PendingAuth {
    swiggy_user_id: String,
    session_info: String,
    pkce_verifier: String,
    pkce_challenge: String,
    phone_digits: String,
    country_code: String,
    created_at_unix: i64,
}

// ── Tool struct ────────────────────────────────────────────────────────────────

/// Manages Swiggy OAuth authentication via phone + OTP.
///
/// `pending` maps `user_id → PendingAuth` and holds transient state between
/// the two-step auth flow. State is memory-only; clears on restart.
pub struct SwiggyAuthTool {
    secrets: Arc<dyn SecretsStore + Send + Sync>,
    pending: Arc<Mutex<HashMap<String, PendingAuth>>>,
}

impl SwiggyAuthTool {
    pub fn new(secrets: Arc<dyn SecretsStore + Send + Sync>) -> Self {
        Self {
            secrets,
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

// ── Swiggy API response types ─────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct SendOtpData {
    #[serde(rename = "userId")]
    user_id: String,
    #[serde(rename = "sessionInfo")]
    session_info: String,
}

#[derive(Debug, Deserialize)]
struct SendOtpResponse {
    success: bool,
    data: Option<SendOtpData>,
    #[serde(default)]
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct VerifyOtpData {
    authorization_code: String,
}

#[derive(Debug, Deserialize)]
struct VerifyOtpResponse {
    success: bool,
    data: Option<VerifyOtpData>,
    #[serde(default)]
    message: Option<String>,
}

/// Swiggy's token endpoint returns standard OAuth fields OR an `opaque_code`.
///
/// When called from a browser with `credentials: include` (cookie mode), the
/// server returns `{opaque_code}` which is later redirected to the client.
/// When called server-side (IronClaw), the server may return `{access_token}`
/// directly, or still return `{opaque_code}` which doubles as the Bearer token.
#[derive(Debug, Deserialize)]
struct SwiggyTokenResponse {
    access_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    opaque_code: Option<String>,
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

// ── HTTP client ─────────────────────────────────────────────────────────────────

fn http_client() -> Result<reqwest::Client, ToolError> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| ToolError::ExecutionFailed(format!("Failed to build HTTP client: {}", e)))
}

// ── Tool implementation ────────────────────────────────────────────────────────

#[async_trait]
impl Tool for SwiggyAuthTool {
    fn name(&self) -> &str {
        "swiggy_auth"
    }

    fn description(&self) -> &str {
        "Authenticate with Swiggy using phone number and OTP. \
        Use action='start_auth' to send OTP to the user's phone, \
        action='complete_auth' to verify the OTP and store credentials, \
        and action='status' to check if Swiggy is already connected. \
        One authentication covers food ordering, Instamart groceries, and Dineout table booking."
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
        // Token data arrives from Swiggy's servers.
        true
    }
}

// ── Action implementations ─────────────────────────────────────────────────────

impl SwiggyAuthTool {
    /// Send OTP to the user's phone, hold pending auth state in memory.
    async fn start_auth(
        &self,
        params: &serde_json::Value,
        user_id: &str,
        start: Instant,
    ) -> Result<ToolOutput, ToolError> {
        // LLMs frequently send numeric-looking values as JSON numbers despite
        // the schema declaring "type": "string". Coerce both types.
        let phone_raw = params
            .get("phone")
            .map(|v| match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                _ => String::new(),
            })
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                ToolError::InvalidParameters("missing 'phone' parameter".to_string())
            })?;

        let country_code_raw = params
            .get("country_code")
            .map(|v| match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => format!("+{}", n),
                _ => String::new(),
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "+91".to_string());
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
        let client = http_client()?;

        let body = serde_json::json!({
            "phone": phone_digits,
            "countryCode": country_code,
            "codeChallenge": pkce_challenge,
            "redirectUri": SWIGGY_REDIRECT_URI
        });

        let resp = client
            .post(format!("{SWIGGY_MCP_BASE}/auth/send-otp"))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Could not reach Swiggy: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            return Err(ToolError::ExecutionFailed(format!(
                "Swiggy send-OTP failed (HTTP {}): {}",
                status, body_text
            )));
        }

        let otp_resp: SendOtpResponse = resp.json().await.map_err(|e| {
            ToolError::ExecutionFailed(format!("Invalid response from Swiggy: {}", e))
        })?;

        if !otp_resp.success {
            return Err(ToolError::ExecutionFailed(
                otp_resp
                    .message
                    .unwrap_or_else(|| "Swiggy failed to send OTP.".to_string()),
            ));
        }

        let data = otp_resp.data.ok_or_else(|| {
            ToolError::ExecutionFailed(
                "Missing session data in Swiggy send-OTP response.".to_string(),
            )
        })?;

        let pending = PendingAuth {
            swiggy_user_id: data.user_id,
            session_info: data.session_info,
            pkce_verifier,
            pkce_challenge,
            phone_digits: phone_digits.clone(),
            country_code: country_code.to_string(),
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

    /// Verify the OTP, exchange for token, store credentials for all Swiggy services.
    async fn complete_auth(
        &self,
        params: &serde_json::Value,
        user_id: &str,
        start: Instant,
    ) -> Result<ToolOutput, ToolError> {
        let otp_raw = params
            .get("otp")
            .map(|v| match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                _ => String::new(),
            })
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                ToolError::InvalidParameters("missing 'otp' parameter".to_string())
            })?;
        let otp: String = otp_raw.chars().filter(|c| c.is_ascii_digit()).collect();

        if otp.len() < 4 || otp.len() > 8 {
            return Err(ToolError::InvalidParameters(
                "OTP must be 4–8 digits.".to_string(),
            ));
        }

        // Load and remove pending auth state.
        let pending = self.pending.lock().await.remove(user_id).ok_or_else(|| {
            ToolError::ExecutionFailed(
                "No pending Swiggy auth found. Please call start_auth first.".to_string(),
            )
        })?;

        let age = Utc::now().timestamp() - pending.created_at_unix;
        if age > OTP_SESSION_TIMEOUT_SECS {
            return Err(ToolError::ExecutionFailed(
                "OTP session expired (10-minute limit). Please start over.".to_string(),
            ));
        }

        let client = http_client()?;

        // Step 1: Verify OTP → authorization_code.
        let verify_body = serde_json::json!({
            "userId": pending.swiggy_user_id,
            "sessionInfo": pending.session_info,
            "otp": otp,
            "codeChallenge": pending.pkce_challenge,
            "redirectUri": SWIGGY_REDIRECT_URI
        });

        let verify_resp = client
            .post(format!("{SWIGGY_MCP_BASE}/auth/verify-otp"))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(&verify_body)
            .send()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Could not reach Swiggy: {}", e)))?;

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

        if !verify.success {
            return Err(ToolError::ExecutionFailed(
                verify
                    .message
                    .unwrap_or_else(|| "Invalid OTP. Please try again.".to_string()),
            ));
        }

        let verify_data = verify.data.ok_or_else(|| {
            ToolError::ExecutionFailed("Missing authorization code in OTP response.".to_string())
        })?;

        // Step 2: Exchange authorization_code → access token.
        let token_body = serde_json::json!({
            "grant_type": "authorization_code",
            "code": verify_data.authorization_code,
            "code_verifier": pending.pkce_verifier,
            "client_id": SWIGGY_CLIENT_ID,
            "redirect_uri": SWIGGY_REDIRECT_URI
        });

        let token_resp = client
            .post(format!("{SWIGGY_MCP_BASE}/auth/token"))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(&token_body)
            .send()
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("Could not reach Swiggy token endpoint: {}", e))
            })?;

        if !token_resp.status().is_success() {
            let status = token_resp.status();
            let body_text = token_resp.text().await.unwrap_or_default();
            return Err(ToolError::ExecutionFailed(format!(
                "Token exchange failed (HTTP {}): {}",
                status, body_text
            )));
        }

        let token: SwiggyTokenResponse = token_resp.json().await.map_err(|e| {
            ToolError::ExecutionFailed(format!("Invalid token response from Swiggy: {}", e))
        })?;

        // Prefer standard OAuth `access_token`; fall back to Swiggy's `opaque_code`.
        let access_token = token.access_token.or(token.opaque_code).ok_or_else(|| {
            ToolError::ExecutionFailed(
                "No access token received from Swiggy. The token format may have changed."
                    .to_string(),
            )
        })?;

        // Store the token for every Swiggy MCP server under the names that
        // McpServerConfig::token_secret_name() produces.
        for &server in SWIGGY_SERVERS {
            let secret_name = format!("mcp_{server}_access_token");
            let mut create_params = CreateSecretParams::new(&secret_name, &access_token)
                .with_provider(format!("mcp:{server}"));

            if let Some(secs) = token.expires_in {
                let expires_at = Utc::now() + chrono::Duration::seconds(secs as i64);
                create_params = create_params.with_expiry(expires_at);
            }

            self.secrets
                .create(user_id, create_params)
                .await
                .map_err(|e| {
                    ToolError::ExecutionFailed(format!("Failed to store {server} token: {e}"))
                })?;

            if let Some(ref refresh) = token.refresh_token {
                let refresh_name = format!("mcp_{server}_access_token_refresh_token");
                let refresh_params = CreateSecretParams::new(&refresh_name, refresh)
                    .with_provider(format!("mcp:{server}"));
                self.secrets
                    .create(user_id, refresh_params)
                    .await
                    .map_err(|e| {
                        ToolError::ExecutionFailed(format!(
                            "Failed to store {server} refresh token: {e}"
                        ))
                    })?;
            }
        }

        Ok(ToolOutput::text(
            "Swiggy connected! You can now order food, shop on Instamart, and book restaurant tables.",
            start.elapsed(),
        ))
    }

    /// Check whether the user has valid Swiggy credentials stored.
    async fn check_status(&self, user_id: &str, start: Instant) -> Result<ToolOutput, ToolError> {
        match self
            .secrets
            .get_decrypted(user_id, "mcp_swiggy-food_access_token")
            .await
        {
            Ok(_) => Ok(ToolOutput::text(
                "Swiggy is connected. Food, Instamart, and Dineout tools are available.",
                start.elapsed(),
            )),
            Err(_) => Ok(ToolOutput::text(
                "Swiggy is not connected. Use action='start_auth' with your phone number.",
                start.elapsed(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pkce_verifier_is_url_safe_base64() {
        let (verifier, challenge) = generate_pkce();
        assert!(
            verifier
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_'),
            "verifier must be URL-safe base64"
        );
        assert!(
            challenge
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_'),
            "challenge must be URL-safe base64"
        );
    }

    #[test]
    fn test_pkce_challenge_is_s256_of_verifier() {
        let (verifier, challenge) = generate_pkce();
        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        let expected = URL_SAFE_NO_PAD.encode(hasher.finalize());
        assert_eq!(challenge, expected, "challenge must equal S256(verifier)");
    }

    #[test]
    fn test_pkce_pairs_are_unique() {
        let (v1, c1) = generate_pkce();
        let (v2, c2) = generate_pkce();
        assert_ne!(v1, v2, "verifiers should be random and unique");
        assert_ne!(c1, c2, "challenges should differ for different verifiers");
    }
}
