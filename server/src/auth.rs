//! Passkey (WebAuthn) authentication for NoirTTY Web
//!
//! Single-user authentication with:
//! - Setup token for initial passkey registration
//! - Passkey-only authentication (no password fallback)
//! - File-based credential storage
//! - Session cookies

use anyhow::{Context, Result};
use axum::{
    extract::State,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Response},
    Json,
};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};
use webauthn_rs::prelude::*;

/// Session cookie name
const SESSION_COOKIE: &str = "noirtty_session";
/// Session validity duration (24 hours)
const SESSION_DURATION_SECS: i64 = 24 * 60 * 60;

/// Authentication state shared across handlers
#[derive(Clone)]
pub struct AuthState {
    inner: Arc<AuthStateInner>,
}

struct AuthStateInner {
    webauthn: Option<Webauthn>,
    credential_file: PathBuf,
    /// Current setup token (only valid when no passkey registered)
    setup_token: RwLock<Option<String>>,
    /// Stored passkey credential
    credential: RwLock<Option<StoredCredential>>,
    /// In-progress registration state
    reg_state: RwLock<Option<PasskeyRegistration>>,
    /// In-progress authentication state
    auth_state: RwLock<Option<PasskeyAuthentication>>,
    /// Active sessions (session_id -> expiry timestamp)
    sessions: RwLock<std::collections::HashMap<String, i64>>,
    /// Is this an IP-based (non-domain) setup?
    is_ip_mode: bool,
}

/// Stored passkey credential
#[derive(Clone, Serialize, Deserialize)]
struct StoredCredential {
    passkey: Passkey,
    registered_at: i64,
}

impl AuthState {
    /// Create new auth state
    pub fn new(rp_id: &str, rp_origin: &url::Url, data_dir: &Path) -> Result<Self> {
        // WebAuthn doesn't support IPs or .local hostnames.
        let mut is_ip_mode = rp_id.parse::<std::net::IpAddr>().is_ok();
        if rp_id.ends_with(".local") {
            is_ip_mode = true;
        }

        let mut webauthn = None;
        if is_ip_mode {
            info!("═══════════════════════════════════════════════════════");
            info!("  IP/LOCAL MODE - WebAuthn not available for IP/.local");
            info!("  Access is open on this host.");
            info!("═══════════════════════════════════════════════════════");
        } else {
            // Domain mode: use WebAuthn passkey
            match WebauthnBuilder::new(rp_id, rp_origin)
                .map(|b| b.rp_name("NoirTTY Web Terminal"))
                .and_then(|b| b.build())
            {
                Ok(instance) => {
                    webauthn = Some(instance);
                }
                Err(err) => {
                    warn!("WebAuthn disabled for host '{}': {}", rp_id, err);
                    warn!("Falling back to open access (IP mode).");
                    is_ip_mode = true;
                }
            }
        }

        let credential_file = data_dir.join("passkey.json");
        let credential = if !is_ip_mode && credential_file.exists() {
            let data = std::fs::read_to_string(&credential_file)?;
            Some(serde_json::from_str(&data)?)
        } else {
            None
        };

        // Generate setup token if no credential exists (domain mode only)
        let setup_token = if !is_ip_mode && credential.is_none() {
            let token = generate_token();
            info!("═══════════════════════════════════════════════════════");
            info!("  SETUP TOKEN: {}", token);
            info!("  Open: {}setup?token={}", rp_origin, token);
            info!("═══════════════════════════════════════════════════════");
            Some(token)
        } else if !is_ip_mode {
            info!("Passkey already registered. Authentication required.");
            None
        } else {
            None
        };

        Ok(Self {
            inner: Arc::new(AuthStateInner {
                webauthn,
                credential_file,
                setup_token: RwLock::new(setup_token),
                credential: RwLock::new(credential),
                reg_state: RwLock::new(None),
                auth_state: RwLock::new(None),
                sessions: RwLock::new(std::collections::HashMap::new()),
                is_ip_mode,
            }),
        })
    }

    /// Check if passkey is registered
    pub async fn is_registered(&self) -> bool {
        if self.inner.is_ip_mode {
            return false;
        }
        self.inner.credential.read().await.is_some()
    }

    pub fn is_ip_mode(&self) -> bool {
        self.inner.is_ip_mode
    }

    /// Check if session is valid
    pub async fn is_session_valid(&self, session_id: &str) -> bool {
        let sessions = self.inner.sessions.read().await;
        if let Some(&expiry) = sessions.get(session_id) {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;
            expiry > now
        } else {
            false
        }
    }

    /// Create a new session
    pub async fn create_session(&self) -> String {
        let session_id = generate_token();
        let expiry = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            + SESSION_DURATION_SECS;

        self.inner.sessions.write().await.insert(session_id.clone(), expiry);
        session_id
    }

    /// Validate setup token
    pub async fn validate_setup_token(&self, token: &str) -> bool {
        let setup_token = self.inner.setup_token.read().await;
        setup_token.as_ref().map(|t| t == token).unwrap_or(false)
    }

    /// Reset authentication (for backdoor/recovery)
    pub async fn reset_auth(&self) -> Result<()> {
        // Remove credential file
        if self.inner.credential_file.exists() {
            std::fs::remove_file(&self.inner.credential_file)?;
        }

        // Clear in-memory state
        *self.inner.credential.write().await = None;
        *self.inner.reg_state.write().await = None;
        *self.inner.auth_state.write().await = None;
        self.inner.sessions.write().await.clear();

        // Generate new setup token
        let token = generate_token();
        info!("═══════════════════════════════════════════════════════");
        info!("  AUTH RESET - NEW SETUP TOKEN: {}", token);
        info!("  Open: https://localhost:3000/setup?token={}", token);
        info!("═══════════════════════════════════════════════════════");
        *self.inner.setup_token.write().await = Some(token);

        Ok(())
    }

    fn webauthn(&self) -> Result<&Webauthn> {
        self.inner
            .webauthn
            .as_ref()
            .context("WebAuthn not available (IP mode)")
    }

    /// Start passkey registration
    pub async fn start_registration(&self) -> Result<CreationChallengeResponse> {
        let user_id = Uuid::new_v4();
        let webauthn = self.webauthn()?;
        let (ccr, reg_state) = webauthn.start_passkey_registration(
            user_id,
            "admin",
            "NoirTTY Admin",
            None,
        )?;

        *self.inner.reg_state.write().await = Some(reg_state);
        Ok(ccr)
    }

    /// Finish passkey registration
    pub async fn finish_registration(&self, reg: RegisterPublicKeyCredential) -> Result<()> {
        let reg_state = self.inner.reg_state.write().await.take()
            .context("No registration in progress")?;

        let webauthn = self.webauthn()?;
        let passkey = webauthn.finish_passkey_registration(&reg, &reg_state)?;

        let credential = StoredCredential {
            passkey,
            registered_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64,
        };

        // Save to file
        let json = serde_json::to_string_pretty(&credential)?;
        std::fs::write(&self.inner.credential_file, json)?;

        // Update in-memory state
        *self.inner.credential.write().await = Some(credential);
        *self.inner.setup_token.write().await = None;

        info!("Passkey registered successfully!");
        Ok(())
    }

    /// Start passkey authentication
    pub async fn start_authentication(&self) -> Result<RequestChallengeResponse> {
        let credential = self.inner.credential.read().await;
        let credential = credential.as_ref().context("No passkey registered")?;

        let webauthn = self.webauthn()?;
        let (rcr, auth_state) = webauthn.start_passkey_authentication(
            &[credential.passkey.clone()]
        )?;

        *self.inner.auth_state.write().await = Some(auth_state);
        Ok(rcr)
    }

    /// Finish passkey authentication
    pub async fn finish_authentication(&self, auth: PublicKeyCredential) -> Result<String> {
        let auth_state = self.inner.auth_state.write().await.take()
            .context("No authentication in progress")?;

        let webauthn = self.webauthn()?;
        let _auth_result = webauthn.finish_passkey_authentication(&auth, &auth_state)?;

        // Create session
        let session_id = self.create_session().await;
        info!("Passkey authentication successful, session created");
        Ok(session_id)
    }
}

/// Generate a random URL-safe token (alphanumeric only, 32 chars)
fn generate_token() -> String {
    const CHARSET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghjkmnpqrstuvwxyz23456789";
    let mut rng = rand::thread_rng();
    (0..32)
        .map(|_| {
            let idx = rng.gen_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}

/// Extract session ID from cookie header
pub fn get_session_from_headers(headers: &HeaderMap) -> Option<String> {
    let cookie_header = headers.get(header::COOKIE)?;
    let cookie_str = cookie_header.to_str().ok()?;

    for part in cookie_str.split(';') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix(&format!("{}=", SESSION_COOKIE)) {
            return Some(value.to_string());
        }
    }
    None
}

/// Check if request is authenticated
pub async fn check_auth_from_headers(auth: &AuthState, headers: &HeaderMap) -> bool {
    if auth.is_ip_mode() {
        return true;
    }
    // If no passkey registered, allow access (setup mode)
    if !auth.is_registered().await {
        return true;
    }

    // Check session cookie
    if let Some(session_id) = get_session_from_headers(headers) {
        return auth.is_session_valid(&session_id).await;
    }

    false
}

/// Create Set-Cookie header for session
fn create_session_cookie(session_id: &str) -> HeaderValue {
    let cookie = format!(
        "{}={}; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age={}",
        SESSION_COOKIE, session_id, SESSION_DURATION_SECS
    );
    HeaderValue::from_str(&cookie).unwrap()
}

/// Create Set-Cookie header to clear session
fn create_logout_cookie() -> HeaderValue {
    let cookie = format!(
        "{}=; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=0",
        SESSION_COOKIE
    );
    HeaderValue::from_str(&cookie).unwrap()
}

// ============================================================================
// HTTP Handlers
// ============================================================================

#[derive(Deserialize)]
pub struct SetupQuery {
    pub token: Option<String>,
}

/// Setup page handler
pub async fn setup_page(
    State(auth): State<AuthState>,
    axum::extract::Query(query): axum::extract::Query<SetupQuery>,
) -> Response {
    if auth.is_ip_mode() {
        return axum::response::Redirect::to("/").into_response();
    }
    // Check if already registered
    if auth.is_registered().await {
        return (StatusCode::FORBIDDEN, Html(r#"
            <!DOCTYPE html>
            <html>
            <head><title>NoirTTY - Already Configured</title></head>
            <body style="background:#1e1e1e;color:#e5e5e5;font-family:system-ui;display:flex;justify-content:center;align-items:center;height:100vh;margin:0;">
                <div style="text-align:center;">
                    <h1>Already Configured</h1>
                    <p>A passkey is already registered.</p>
                    <a href="/login" style="color:#4fc3f7;">Go to Login</a>
                </div>
            </body>
            </html>
        "#)).into_response();
    }

    // Validate token
    let token = query.token.unwrap_or_default();
    if !auth.validate_setup_token(&token).await {
        return (StatusCode::UNAUTHORIZED, Html(r#"
            <!DOCTYPE html>
            <html>
            <head><title>NoirTTY - Invalid Token</title></head>
            <body style="background:#1e1e1e;color:#e5e5e5;font-family:system-ui;display:flex;justify-content:center;align-items:center;height:100vh;margin:0;">
                <div style="text-align:center;">
                    <h1>Invalid Setup Token</h1>
                    <p>Check the server console for the correct token.</p>
                </div>
            </body>
            </html>
        "#)).into_response();
    }

    Html(SETUP_HTML).into_response()
}

/// Login page handler
pub async fn login_page(State(auth): State<AuthState>) -> Response {
    if auth.is_ip_mode() {
        return axum::response::Redirect::to("/").into_response();
    }
    if !auth.is_registered().await {
        return axum::response::Redirect::to("/").into_response();
    }
    Html(LOGIN_HTML).into_response()
}

/// Start registration API
pub async fn api_register_start(State(auth): State<AuthState>) -> Response {
    match auth.start_registration().await {
        Ok(ccr) => Json(ccr).into_response(),
        Err(e) => {
            warn!("Registration start failed: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

/// Finish registration API
pub async fn api_register_finish(
    State(auth): State<AuthState>,
    Json(reg): Json<RegisterPublicKeyCredential>,
) -> Response {
    match auth.finish_registration(reg).await {
        Ok(()) => {
            // Create session immediately after registration
            let session_id = auth.create_session().await;
            let mut headers = HeaderMap::new();
            headers.insert(header::SET_COOKIE, create_session_cookie(&session_id));
            (headers, Json(serde_json::json!({"ok": true}))).into_response()
        }
        Err(e) => {
            warn!("Registration finish failed: {}", e);
            (StatusCode::BAD_REQUEST, e.to_string()).into_response()
        }
    }
}

/// Start authentication API
pub async fn api_auth_start(State(auth): State<AuthState>) -> Response {
    match auth.start_authentication().await {
        Ok(rcr) => Json(rcr).into_response(),
        Err(e) => {
            warn!("Auth start failed: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

/// Finish authentication API
pub async fn api_auth_finish(
    State(auth): State<AuthState>,
    Json(cred): Json<PublicKeyCredential>,
) -> Response {
    match auth.finish_authentication(cred).await {
        Ok(session_id) => {
            let mut headers = HeaderMap::new();
            headers.insert(header::SET_COOKIE, create_session_cookie(&session_id));
            (headers, Json(serde_json::json!({"ok": true}))).into_response()
        }
        Err(e) => {
            warn!("Auth finish failed: {}", e);
            (StatusCode::UNAUTHORIZED, e.to_string()).into_response()
        }
    }
}

/// Logout handler
pub async fn logout() -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(header::SET_COOKIE, create_logout_cookie());
    (headers, axum::response::Redirect::to("/login")).into_response()
}

/// Lock system - invalidate ALL sessions (requires re-auth with passkey)
pub async fn lock_system(State(auth): State<AuthState>) -> Response {
    auth.inner.sessions.write().await.clear();
    info!("🔒 System locked - all sessions invalidated");
    Json(serde_json::json!({"ok": true, "message": "All sessions invalidated"})).into_response()
}

// ============================================================================
// HTML Templates
// ============================================================================

const SETUP_HTML: &str = r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>NoirTTY - Setup Passkey</title>
    <style>
        * { box-sizing: border-box; }
        body {
            background: #1e1e1e;
            color: #e5e5e5;
            font-family: system-ui, -apple-system, sans-serif;
            display: flex;
            justify-content: center;
            align-items: center;
            min-height: 100vh;
            margin: 0;
            padding: 20px;
        }
        .container {
            max-width: 400px;
            text-align: center;
        }
        h1 { color: #4fc3f7; margin-bottom: 10px; }
        p { color: #aaa; line-height: 1.6; }
        button {
            background: #4fc3f7;
            color: #1e1e1e;
            border: none;
            padding: 16px 32px;
            font-size: 18px;
            font-weight: 600;
            border-radius: 8px;
            cursor: pointer;
            margin-top: 20px;
            transition: background 0.2s;
        }
        button:hover { background: #81d4fa; }
        button:disabled { background: #555; cursor: not-allowed; }
        .status { margin-top: 20px; padding: 10px; border-radius: 4px; }
        .status.error { background: #5c2626; color: #f48fb1; }
        .status.success { background: #1b5e20; color: #a5d6a7; }
        .icon { font-size: 64px; margin-bottom: 20px; }
    </style>
</head>
<body>
    <div class="container">
        <div class="icon">🔐</div>
        <h1>Setup Passkey</h1>
        <p>Register a passkey to secure your NoirTTY terminal. This will use your device's biometric authentication (Face ID, Touch ID, Windows Hello, etc.)</p>
        <button id="register">Register Passkey</button>
        <div id="status" class="status" style="display:none;"></div>
    </div>
    <script>
        const btn = document.getElementById('register');
        const status = document.getElementById('status');

        function showStatus(msg, isError) {
            status.style.display = 'block';
            status.textContent = msg;
            status.className = 'status ' + (isError ? 'error' : 'success');
        }

        btn.addEventListener('click', async () => {
            btn.disabled = true;
            btn.textContent = 'Registering...';
            try {
                // Start registration
                const startResp = await fetch('/api/auth/register/start', { method: 'POST' });
                if (!startResp.ok) throw new Error(await startResp.text());
                const options = await startResp.json();

                // Convert base64url to ArrayBuffer
                options.publicKey.challenge = base64urlToBuffer(options.publicKey.challenge);
                options.publicKey.user.id = base64urlToBuffer(options.publicKey.user.id);
                if (options.publicKey.excludeCredentials) {
                    options.publicKey.excludeCredentials = options.publicKey.excludeCredentials.map(c => ({
                        ...c,
                        id: base64urlToBuffer(c.id)
                    }));
                }

                // Create credential
                const credential = await navigator.credentials.create(options);

                // Prepare response
                const response = {
                    id: credential.id,
                    rawId: bufferToBase64url(credential.rawId),
                    type: credential.type,
                    response: {
                        clientDataJSON: bufferToBase64url(credential.response.clientDataJSON),
                        attestationObject: bufferToBase64url(credential.response.attestationObject),
                    }
                };

                // Finish registration
                const finishResp = await fetch('/api/auth/register/finish', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify(response)
                });
                if (!finishResp.ok) throw new Error(await finishResp.text());

                showStatus('Passkey registered successfully! Redirecting...', false);
                setTimeout(() => window.location.href = '/', 1500);
            } catch (e) {
                console.error(e);
                showStatus('Error: ' + e.message, true);
                btn.disabled = false;
                btn.textContent = 'Register Passkey';
            }
        });

        function base64urlToBuffer(base64url) {
            const base64 = base64url.replace(/-/g, '+').replace(/_/g, '/');
            const padding = '='.repeat((4 - base64.length % 4) % 4);
            const binary = atob(base64 + padding);
            return Uint8Array.from(binary, c => c.charCodeAt(0)).buffer;
        }

        function bufferToBase64url(buffer) {
            const bytes = new Uint8Array(buffer);
            let binary = '';
            for (const byte of bytes) binary += String.fromCharCode(byte);
            return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=/g, '');
        }
    </script>
</body>
</html>"#;

const LOGIN_HTML: &str = r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>NoirTTY - Login</title>
    <style>
        * { box-sizing: border-box; }
        body {
            background: #1e1e1e;
            color: #e5e5e5;
            font-family: system-ui, -apple-system, sans-serif;
            display: flex;
            justify-content: center;
            align-items: center;
            min-height: 100vh;
            margin: 0;
            padding: 20px;
        }
        .container {
            max-width: 400px;
            text-align: center;
        }
        h1 { color: #4fc3f7; margin-bottom: 10px; }
        p { color: #aaa; line-height: 1.6; }
        button {
            background: #4fc3f7;
            color: #1e1e1e;
            border: none;
            padding: 16px 32px;
            font-size: 18px;
            font-weight: 600;
            border-radius: 8px;
            cursor: pointer;
            margin-top: 20px;
            transition: background 0.2s;
        }
        button:hover { background: #81d4fa; }
        button:disabled { background: #555; cursor: not-allowed; }
        .status { margin-top: 20px; padding: 10px; border-radius: 4px; }
        .status.error { background: #5c2626; color: #f48fb1; }
        .icon { font-size: 64px; margin-bottom: 20px; }
    </style>
</head>
<body>
    <div class="container">
        <div class="icon">👻</div>
        <h1>NoirTTY Web</h1>
        <p>Authenticate with your passkey to access the terminal.</p>
        <button id="login">Login with Passkey</button>
        <div id="status" class="status" style="display:none;"></div>
    </div>
    <script>
        const btn = document.getElementById('login');
        const status = document.getElementById('status');

        function showStatus(msg) {
            status.style.display = 'block';
            status.textContent = msg;
            status.className = 'status error';
        }

        async function authenticate() {
            btn.disabled = true;
            btn.textContent = 'Authenticating...';
            try {
                // Start authentication
                const startResp = await fetch('/api/auth/login/start', { method: 'POST' });
                if (!startResp.ok) throw new Error(await startResp.text());
                const options = await startResp.json();

                // Convert base64url to ArrayBuffer
                options.publicKey.challenge = base64urlToBuffer(options.publicKey.challenge);
                if (options.publicKey.allowCredentials) {
                    options.publicKey.allowCredentials = options.publicKey.allowCredentials.map(c => ({
                        ...c,
                        id: base64urlToBuffer(c.id)
                    }));
                }

                // Get credential
                const credential = await navigator.credentials.get(options);

                // Prepare response
                const response = {
                    id: credential.id,
                    rawId: bufferToBase64url(credential.rawId),
                    type: credential.type,
                    response: {
                        clientDataJSON: bufferToBase64url(credential.response.clientDataJSON),
                        authenticatorData: bufferToBase64url(credential.response.authenticatorData),
                        signature: bufferToBase64url(credential.response.signature),
                        userHandle: credential.response.userHandle ? bufferToBase64url(credential.response.userHandle) : null,
                    }
                };

                // Finish authentication
                const finishResp = await fetch('/api/auth/login/finish', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify(response)
                });
                if (!finishResp.ok) throw new Error(await finishResp.text());

                window.location.href = '/';
            } catch (e) {
                console.error(e);
                showStatus('Error: ' + e.message);
                btn.disabled = false;
                btn.textContent = 'Login with Passkey';
            }
        }

        btn.addEventListener('click', authenticate);

        // Auto-trigger on page load
        setTimeout(authenticate, 500);

        function base64urlToBuffer(base64url) {
            const base64 = base64url.replace(/-/g, '+').replace(/_/g, '/');
            const padding = '='.repeat((4 - base64.length % 4) % 4);
            const binary = atob(base64 + padding);
            return Uint8Array.from(binary, c => c.charCodeAt(0)).buffer;
        }

        function bufferToBase64url(buffer) {
            const bytes = new Uint8Array(buffer);
            let binary = '';
            for (const byte of bytes) binary += String.fromCharCode(byte);
            return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=/g, '');
        }
    </script>
</body>
</html>"#;
