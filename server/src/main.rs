//! NoirTTY Web Server - WebSocket Terminal Server

mod auth;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
        Query,
    },
    http::HeaderMap,
    response::{IntoResponse, Response, Redirect},
    routing::{get, post},
    Router,
};
use alacritty_terminal::{
    event::{Event, EventListener},
    grid::{Dimensions, Scroll},
    term::{cell::Flags as TermFlags, Term, Config as TermConfig},
};
use alacritty_terminal::vte::ansi::{Color, NamedColor, CursorShape, Processor, StdSyncHandler};
use rcgen::{generate_simple_self_signed, CertifiedKey};
use futures::{SinkExt, StreamExt};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::net::SocketAddr;
use tokio::sync::{mpsc, broadcast};
use tower_http::{cors::CorsLayer, services::ServeDir};
use tower_http::set_header::SetResponseHeaderLayer;
use axum::http::{header, HeaderValue, StatusCode};
use tower::ServiceBuilder;
use tracing::{debug, error, info, warn};
use uuid::Uuid;
use dashmap::DashMap;
use rustls::crypto::ring;
use gethostname::gethostname;
use bincode;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ClientMessage {
    #[serde(rename = "data")]
    Data { data: String },
    #[serde(rename = "resize")]
    Resize { cols: u16, rows: u16 },
    #[serde(rename = "scroll")]
    Scroll { delta: i32 },
    #[serde(rename = "quality")]
    Quality { min_interval_ms: u32 },
}

#[derive(Clone, Debug, Serialize)]
struct ServerCell {
    c: char,
    fg: [u8; 3],
    bg: [u8; 3],
    bold: bool,
    italic: bool,
    underline: bool,
    inverse: bool,
}

impl Default for ServerCell {
    fn default() -> Self {
        Self {
            c: ' ',
            fg: DEFAULT_FG,
            bg: DEFAULT_BG,
            bold: false,
            italic: false,
            underline: false,
            inverse: false,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct ServerFrame {
    cols: u16,
    rows: u16,
    cursor_col: u16,
    cursor_row: u16,
    cursor_visible: bool,
    cells: Vec<ServerCell>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type")]
enum ServerMessage {
    #[serde(rename = "frame")]
    Frame(ServerFrame),
}

#[derive(Clone)]
struct AppState {
    sessions: Arc<DashMap<String, Arc<Session>>>,
    auth: auth::AuthState,
    config_path: Arc<std::path::PathBuf>,
    debug_ui: bool,
}

#[derive(Clone)]
struct Session {
    id: String,
    pty_tx: mpsc::Sender<PtyCommand>,
    frame_tx: broadcast::Sender<ServerMessage>,
    last_frame: Arc<Mutex<Option<ServerMessage>>>,
    min_interval_ms: Arc<AtomicU64>,
}

#[derive(Debug, Deserialize)]
struct SessionQuery {
    session: Option<String>,
    format: Option<String>,
}

#[tokio::main]
async fn main() {
    init_logging();
    info!("Starting NoirTTY Web Server...");

    let (use_https, cert_hosts, reset_auth, rp_host) = parse_tls_args();
    let data_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../certs");
    std::fs::create_dir_all(&data_dir).expect("Failed to create data directory");

    // Determine the host for WebAuthn (use --host or first cert host, default localhost)
    let webauthn_host = rp_host.unwrap_or_else(|| "localhost".to_string());
    info!("WebAuthn RP ID: {}", webauthn_host);

    // Determine the origin URL for WebAuthn
    let origin = if use_https {
        url::Url::parse(&format!("https://{}:3000", webauthn_host)).unwrap()
    } else {
        url::Url::parse(&format!("http://{}:3000", webauthn_host)).unwrap()
    };

    // Initialize auth state
    let auth = auth::AuthState::new(&webauthn_host, &origin, &data_dir)
        .expect("Failed to initialize authentication");

    // Handle --reset-auth flag
    if reset_auth {
        auth.reset_auth().await.expect("Failed to reset auth");
    }

    let static_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../static");
    info!("Serving static files from: {:?}", static_dir);

    let debug_ui = std::env::var("NOIRTTY_DEBUG")
        .map(|v| v != "0")
        .unwrap_or(false);

    let state = AppState {
        sessions: Arc::new(DashMap::new()),
        auth: auth.clone(),
        config_path: Arc::new(static_dir.join("config.json")),
        debug_ui,
    };

    let static_service = ServiceBuilder::new()
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::PRAGMA,
            HeaderValue::from_static("no-cache"),
        ))
        .service(ServeDir::new(&static_dir).append_index_html_on_directories(true));

    // All routes with state
    let app = Router::new()
        // Auth routes (no auth required)
        .route("/setup", get(setup_page_handler))
        .route("/login", get(login_page_handler))
        .route("/logout", post(logout_handler))
        .route("/api/auth/register/start", post(register_start_handler))
        .route("/api/auth/register/finish", post(register_finish_handler))
        .route("/api/auth/login/start", post(auth_start_handler))
        .route("/api/auth/login/finish", post(auth_finish_handler))
        .route("/api/auth/lock", post(lock_handler))
        // Protected routes (auth checked in handler)
        .route("/", get(index_handler))
        .route("/ws", get(ws_handler_with_auth))
        .route("/health", get(|| async { "OK" }))
        .route("/config.json", get(config_handler))
        .fallback_service(static_service)
        .with_state(state.clone())
        .layer(CorsLayer::permissive());

    let addr: SocketAddr = "0.0.0.0:3000".parse().unwrap();
    if use_https {
        if ring::default_provider().install_default().is_err() {
            error!("Failed to install rustls ring crypto provider");
        }
        let (cert_path, key_path) = ensure_self_signed_cert(&data_dir, &cert_hosts)
            .expect("Failed to generate self-signed certificate");
        info!("TLS certificate: {:?}", cert_path);
        info!("Server listening on https://{}:3000", webauthn_host);
        let tls_config = axum_server::tls_rustls::RustlsConfig::from_pem_file(cert_path, key_path)
            .await
            .expect("Failed to load TLS config");
        axum_server::bind_rustls(addr, tls_config)
            .serve(app.into_make_service())
            .await
            .unwrap();
    } else {
        warn!("⚠️  INSECURE MODE: Running without TLS encryption!");
        warn!("⚠️  All terminal data transmitted in plaintext.");
        warn!("⚠️  Use only for local development. Run with HTTPS in production.");
        info!("Server listening on http://{}:3000", webauthn_host);
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        axum::serve(listener, app).await.unwrap();
    }
}


/// Index handler - serves terminal or redirects to login/setup
async fn index_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    // If no passkey registered, redirect to setup unless we're in IP mode.
    if !state.auth.is_registered().await && !state.auth.is_ip_mode() {
        return axum::response::Html(r#"
            <!DOCTYPE html>
            <html>
            <head><title>NoirTTY - Setup Required</title></head>
            <body style="background:#1e1e1e;color:#e5e5e5;font-family:system-ui;display:flex;justify-content:center;align-items:center;height:100vh;margin:0;">
                <div style="text-align:center;">
                    <h1>🔐 Setup Required</h1>
                    <p>Check the server console for the setup token.</p>
                    <p style="color:#888;">Then open <code>/setup?token=YOUR_TOKEN</code></p>
                </div>
            </body>
            </html>
        "#).into_response();
    }

    // Check auth
    if !auth::check_auth_from_headers(&state.auth, &headers).await {
        return Redirect::to("/login").into_response();
    }

    // Serve the terminal
    let index_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../static/index.html");
    match std::fs::read_to_string(&index_path) {
        Ok(content) => axum::response::Html(content).into_response(),
        Err(_) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to load index.html").into_response(),
    }
}

async fn config_handler(State(state): State<AppState>) -> impl IntoResponse {
    let raw = std::fs::read_to_string(&*state.config_path).unwrap_or_else(|_| "{}".to_string());
    let mut value: serde_json::Value = serde_json::from_str(&raw).unwrap_or_else(|_| {
        serde_json::json!({})
    });

    if state.debug_ui {
        let ui = value
            .as_object_mut()
            .and_then(|root| root.entry("ui").or_insert_with(|| serde_json::json!({})).as_object_mut());
        if let Some(ui) = ui {
            ui.insert("debug".to_string(), serde_json::json!(true));
        }
    }

    let body = serde_json::to_string(&value).unwrap_or(raw);
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "no-store"),
            (header::PRAGMA, "no-cache"),
        ],
        body,
    )
}

// Auth handler wrappers (use AppState instead of AuthState)
async fn setup_page_handler(
    State(state): State<AppState>,
    query: axum::extract::Query<auth::SetupQuery>,
) -> Response {
    info!("Setup page requested, token: {:?}", query.token);
    auth::setup_page(State(state.auth), query).await
}

async fn login_page_handler(State(state): State<AppState>) -> Response {
    auth::login_page(State(state.auth)).await
}

async fn logout_handler() -> Response {
    auth::logout().await
}

async fn register_start_handler(State(state): State<AppState>) -> Response {
    auth::api_register_start(State(state.auth)).await
}

async fn register_finish_handler(
    State(state): State<AppState>,
    json: axum::Json<webauthn_rs::prelude::RegisterPublicKeyCredential>,
) -> Response {
    auth::api_register_finish(State(state.auth), json).await
}

async fn auth_start_handler(State(state): State<AppState>) -> Response {
    auth::api_auth_start(State(state.auth)).await
}

async fn auth_finish_handler(
    State(state): State<AppState>,
    json: axum::Json<webauthn_rs::prelude::PublicKeyCredential>,
) -> Response {
    auth::api_auth_finish(State(state.auth), json).await
}

async fn lock_handler(State(state): State<AppState>) -> Response {
    auth::lock_system(State(state.auth)).await
}

/// WebSocket handler with auth check
async fn ws_handler_with_auth(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Query(query): Query<SessionQuery>,
    headers: HeaderMap,
) -> Response {
    // Check auth
    if !auth::check_auth_from_headers(&state.auth, &headers).await {
        return (axum::http::StatusCode::UNAUTHORIZED, "Authentication required").into_response();
    }

    let session_id = query.session.unwrap_or_else(|| Uuid::new_v4().to_string());
    let use_binary = matches!(
        query.format.as_deref(),
        Some("bincode") | Some("bin") | Some("binary")
    );
    let session = get_or_create_session(&state, &session_id);
    ws.on_upgrade(move |socket| handle_socket(socket, session, use_binary))
}

enum PtyCommand {
    Data(Vec<u8>),
    Resize(u16, u16),
    Scroll(i32),
}

enum TermCommand {
    Data(Vec<u8>),
    Resize(u16, u16),
    Scroll(i32),
}

#[derive(Clone)]
struct TermEventProxy {
    pty_tx: mpsc::Sender<PtyCommand>,
}

impl EventListener for TermEventProxy {
    fn send_event(&self, event: Event) {
        if let Event::PtyWrite(text) = event {
            let _ = self.pty_tx.blocking_send(PtyCommand::Data(text.into_bytes()));
        }
    }
}

#[derive(Clone, Copy)]
struct TermSize {
    cols: usize,
    rows: usize,
}

impl Dimensions for TermSize {
    fn total_lines(&self) -> usize {
        self.rows
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn columns(&self) -> usize {
        self.cols
    }
}

fn get_or_create_session(state: &AppState, session_id: &str) -> Arc<Session> {
    if let Some(existing) = state.sessions.get(session_id) {
        info!("Reusing session {}", session_id);
        return existing.clone();
    }

    let (pty_tx, pty_rx) = mpsc::channel::<PtyCommand>(1024);
    let (frame_tx, _) = broadcast::channel::<ServerMessage>(256);
    let last_frame = Arc::new(Mutex::new(None));
    let min_interval_ms = Arc::new(AtomicU64::new(0));

    let pty_tx_clone = pty_tx.clone();
    let frame_tx_clone = frame_tx.clone();
    let last_frame_clone = last_frame.clone();
    let min_interval_clone = min_interval_ms.clone();
    std::thread::spawn(move || {
        run_pty(frame_tx_clone, pty_rx, pty_tx_clone, last_frame_clone, min_interval_clone);
    });

    let session = Arc::new(Session {
        id: session_id.to_string(),
        pty_tx,
        frame_tx,
        last_frame,
        min_interval_ms,
    });
    state.sessions.insert(session_id.to_string(), session.clone());
    info!("Created new session {}", session_id);
    session
}

async fn handle_socket(socket: WebSocket, session: Arc<Session>, use_binary: bool) {
    info!("New WebSocket connection (session={})", session.id);

    let (mut ws_tx, mut ws_rx) = socket.split();
    let min_interval_ms = session.min_interval_ms.clone();

    // Task: PTY -> WebSocket
    let mut frame_rx = session.frame_tx.subscribe();
    let last = session
        .last_frame
        .lock()
        .ok()
        .and_then(|guard| guard.clone());
    if let Some(last) = last {
        if use_binary {
            if let Ok(bin) = bincode::serialize(&last) {
                let _ = ws_tx.send(Message::Binary(bin.into())).await;
            }
        } else if let Ok(json) = serde_json::to_string(&last) {
            let _ = ws_tx.send(Message::Text(json.into())).await;
        }
    }
    let min_interval_ms_send = min_interval_ms.clone();
    let send_task = tokio::spawn(async move {
        let mut last_sent = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(1))
            .unwrap_or_else(std::time::Instant::now);
        loop {
            match frame_rx.recv().await {
                Ok(msg) => {
                    let min_ms = min_interval_ms_send.load(Ordering::Relaxed);
                    if min_ms > 0 {
                        let now = std::time::Instant::now();
                        if now.duration_since(last_sent) < std::time::Duration::from_millis(min_ms) {
                            continue;
                        }
                        last_sent = now;
                    }
                    if use_binary {
                        if let Ok(bin) = bincode::serialize(&msg) {
                            if ws_tx.send(Message::Binary(bin.into())).await.is_err() {
                                break;
                            }
                        }
                    } else if let Ok(json) = serde_json::to_string(&msg) {
                        if ws_tx.send(Message::Text(json.into())).await.is_err() {
                            break;
                        }
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    });

    // Task: WebSocket -> PTY
    let pty_tx = session.pty_tx.clone();
    let min_interval_ms_recv = min_interval_ms.clone();
    let recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = ws_rx.next().await {
            match msg {
                Message::Text(text) => {
                    if let Ok(client_msg) = serde_json::from_str::<ClientMessage>(&text) {
                        match client_msg {
                            ClientMessage::Data { data } => {
                                let _ = pty_tx.send(PtyCommand::Data(data.into_bytes())).await;
                            }
                            ClientMessage::Resize { cols, rows } => {
                                let _ = pty_tx.send(PtyCommand::Resize(cols, rows)).await;
                            }
                            ClientMessage::Scroll { delta } => {
                                let _ = pty_tx.send(PtyCommand::Scroll(delta)).await;
                            }
                            ClientMessage::Quality { min_interval_ms } => {
                                min_interval_ms_recv.store(min_interval_ms as u64, Ordering::Relaxed);
                            }
                        }
                    } else {
                        warn!("Failed to parse client message: {}", text);
                    }
                }
                Message::Binary(data) => {
                    let _ = pty_tx.send(PtyCommand::Data(data.to_vec())).await;
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    });

    tokio::select! {
        _ = send_task => {},
        _ = recv_task => {},
    }

    info!("Connection closed");
}

fn run_pty(
    output_tx: broadcast::Sender<ServerMessage>,
    mut input_rx: mpsc::Receiver<PtyCommand>,
    pty_tx: mpsc::Sender<PtyCommand>,
    last_frame: Arc<Mutex<Option<ServerMessage>>>,
    min_interval_ms: Arc<AtomicU64>,
) {
    let pty_system = native_pty_system();

    let pair = match pty_system.openpty(PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    }) {
        Ok(p) => p,
        Err(e) => {
            error!("Failed to create PTY: {}", e);
            return;
        }
    };

    info!("PTY created successfully");

    let shell = resolve_shell();
    let mut cmd = CommandBuilder::new(&shell);
    configure_shell_command(&mut cmd, &shell);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("LANG", "en_US.UTF-8");
    // Explicitly set PATH to ensure standard commands are found
    if std::env::var("PATH").is_err() {
        cmd.env("PATH", "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin");
    }

    if let Err(e) = pair.slave.spawn_command(cmd) {
        error!("Failed to spawn shell: {}", e);
        return;
    }
    drop(pair.slave); // Close slave after spawn

    info!("Shell spawned");

    let master = pair.master;
    let mut reader = master.try_clone_reader().unwrap();
    let mut writer = master.take_writer().unwrap();

    let (term_cmd_tx, mut term_cmd_rx) = mpsc::channel::<TermCommand>(1024);

    // Terminal emulation thread (alacritty_terminal)
    let term_output_tx = output_tx.clone();
    let term_last_frame = last_frame.clone();
    let term_pty_tx = pty_tx.clone();
    std::thread::spawn(move || {
        let proxy = TermEventProxy { pty_tx: term_pty_tx };
        let mut processor = Processor::<StdSyncHandler>::new();
        let config = TermConfig::default();
        let size = TermSize { cols: 80, rows: 24 };
        let mut term = Term::new(config, &size, proxy);

        let mut last_sent = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(1))
            .unwrap_or_else(std::time::Instant::now);
        while let Some(cmd) = term_cmd_rx.blocking_recv() {
            match cmd {
                TermCommand::Data(data) => {
                    processor.advance(&mut term, &data);
                }
                TermCommand::Resize(cols, rows) => {
                    term.resize(TermSize {
                        cols: cols as usize,
                        rows: rows as usize,
                    });
                }
                TermCommand::Scroll(delta) => {
                    term.scroll_display(Scroll::Delta(delta));
                }
            }

            // Drain any additional queued commands to avoid rebuilding multiple frames.
            while let Ok(cmd) = term_cmd_rx.try_recv() {
                match cmd {
                    TermCommand::Data(data) => {
                        processor.advance(&mut term, &data);
                    }
                    TermCommand::Resize(cols, rows) => {
                        term.resize(TermSize {
                            cols: cols as usize,
                            rows: rows as usize,
                        });
                    }
                    TermCommand::Scroll(delta) => {
                        term.scroll_display(Scroll::Delta(delta));
                    }
                }
            }

            let min_ms = min_interval_ms.load(Ordering::Relaxed);
            if min_ms > 0 {
                let now = std::time::Instant::now();
                let elapsed = now.duration_since(last_sent);
                let min_dur = std::time::Duration::from_millis(min_ms);
                if elapsed < min_dur {
                    std::thread::sleep(min_dur - elapsed);
                }
                last_sent = std::time::Instant::now();
            } else {
                last_sent = std::time::Instant::now();
            }

            let frame = build_frame(&term);
            let msg = ServerMessage::Frame(frame);
            if let Ok(mut guard) = term_last_frame.lock() {
                *guard = Some(msg.clone());
            }
            let _ = term_output_tx.send(msg);
        }
    });

    // Reader thread
    let term_tx = term_cmd_tx.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if term_tx.blocking_send(TermCommand::Data(buf[..n].to_vec())).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    error!("Error reading from PTY: {}", e);
                    break;
                }
            }
        }
        debug!("PTY reader thread exited");
    });

    // Input/resize handler (same thread as master owner)
    while let Some(cmd) = input_rx.blocking_recv() {
        match cmd {
            PtyCommand::Data(data) => {
                if writer.write_all(&data).is_err() {
                    break;
                }
            }
            PtyCommand::Resize(cols, rows) => {
                debug!("Resize to {}x{}", cols, rows);
                let _ = master.resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                });
                let _ = term_cmd_tx.blocking_send(TermCommand::Resize(cols, rows));
            }
            PtyCommand::Scroll(delta) => {
                let _ = term_cmd_tx.blocking_send(TermCommand::Scroll(delta));
            }
        }
    }
    info!("PTY handler exited");
}

fn init_logging() {
    let verbose = std::env::args().any(|arg| arg == "-v" || arg == "--verbose");
    let default_level = if verbose { "debug" } else { "info" };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_level));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

fn parse_tls_args() -> (bool, Vec<String>, bool, Option<String>) {
    // SECURITY: HTTPS is enabled by default
    let mut use_https = true;
    let mut reset_auth = false;
    let mut rp_host: Option<String> = None;
    let mut hosts: BTreeSet<String> = ["localhost", "127.0.0.1", "::1"]
        .iter()
        .map(|s| s.to_string())
        .collect();

    // Allow disabling HTTPS via env var (for development only)
    if let Ok(val) = std::env::var("NOIRTTY_INSECURE") {
        if val == "1" || val.eq_ignore_ascii_case("true") {
            use_https = false;
        }
    }
    // Optional env var support (now inverted logic)
    if let Ok(val) = std::env::var("NOIRTTY_HTTPS") {
        if val == "0" || val.eq_ignore_ascii_case("false") {
            use_https = false;
        }
    }
    // Reset auth via env var
    if let Ok(val) = std::env::var("NOIRTTY_RESET_AUTH") {
        if val == "1" || val.eq_ignore_ascii_case("true") {
            reset_auth = true;
        }
    }
    // Host for WebAuthn RP ID
    if let Ok(val) = std::env::var("NOIRTTY_HOST") {
        rp_host = Some(val.clone());
        hosts.insert(val);
    }
    if let Ok(val) = std::env::var("NOIRTTY_CERT_HOSTS") {
        for host in val.split(',').map(|h| h.trim()).filter(|h| !h.is_empty()) {
            hosts.insert(host.to_string());
        }
    }

    for arg in std::env::args().skip(1) {
        if arg == "--insecure" || arg == "--http" {
            use_https = false;
        } else if arg == "--reset-auth" {
            reset_auth = true;
        } else if let Some(val) = arg.strip_prefix("--host=") {
            rp_host = Some(val.to_string());
            hosts.insert(val.to_string());
        } else if let Some(val) = arg.strip_prefix("--cert-hosts=") {
            for host in val.split(',').map(|h| h.trim()).filter(|h| !h.is_empty()) {
                hosts.insert(host.to_string());
            }
        }
    }

    if rp_host.is_none() {
        if let Some(hostname) = detect_hostname() {
            rp_host = Some(hostname.clone());
            hosts.insert(hostname);
        }
    }

    if rp_host.is_none() {
        for host in &hosts {
            if host == "localhost" || host == "127.0.0.1" || host == "::1" {
                continue;
            }
            rp_host = Some(host.clone());
            break;
        }
    }

    (use_https, hosts.into_iter().collect(), reset_auth, rp_host)
}

fn detect_hostname() -> Option<String> {
    let env_host = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("HOST"))
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .ok();
    if let Some(mut host) = env_host {
        host = host.trim().to_string();
        if host.is_empty() || host.eq_ignore_ascii_case("localhost") {
            return None;
        }
        return Some(normalize_hostname(host));
    }

    let os_host = gethostname();
    let host = os_host.to_string_lossy().trim().to_string();
    if host.is_empty() || host.eq_ignore_ascii_case("localhost") {
        return None;
    }
    Some(normalize_hostname(host))
}

fn normalize_hostname(host: String) -> String {
    if host.contains('.') {
        return host;
    }
    format!("{}.local", host)
}

fn ensure_self_signed_cert(cert_dir: &Path, hosts: &[String]) -> anyhow::Result<(std::path::PathBuf, std::path::PathBuf)> {
    std::fs::create_dir_all(cert_dir)?;
    let cert_pem = cert_dir.join("noirtty-selfsigned.cert.pem");
    let key_pem = cert_dir.join("noirtty-selfsigned.key.pem");

    if cert_pem.exists() && key_pem.exists() {
        return Ok((cert_pem, key_pem));
    }

    let CertifiedKey { cert, key_pair } = generate_simple_self_signed(hosts.to_vec())?;
    let cert_pem_str = cert.pem();
    let key_pem_str = key_pair.serialize_pem();

    std::fs::write(&cert_pem, cert_pem_str)?;
    std::fs::write(&key_pem, key_pem_str)?;

    Ok((cert_pem, key_pem))
}

const DEFAULT_FG: [u8; 3] = [229, 229, 229];
const DEFAULT_BG: [u8; 3] = [30, 30, 30];

const ANSI_16: [[u8; 3]; 16] = [
    [0, 0, 0],
    [205, 49, 49],
    [13, 188, 121],
    [229, 229, 16],
    [36, 114, 200],
    [188, 63, 188],
    [17, 168, 205],
    [229, 229, 229],
    [102, 102, 102],
    [241, 76, 76],
    [35, 209, 139],
    [245, 245, 67],
    [59, 142, 234],
    [214, 112, 214],
    [41, 184, 219],
    [255, 255, 255],
];

fn build_frame<T: EventListener>(term: &Term<T>) -> ServerFrame {
    let content = term.renderable_content();
    let cols = term.columns() as u16;
    let rows = term.screen_lines() as u16;
    let display_offset = content.display_offset as i32;

    let mut cells = vec![ServerCell::default(); (cols as usize) * (rows as usize)];

    for indexed in content.display_iter {
        let row = indexed.point.line.0 + display_offset;
        if row < 0 || row >= rows as i32 {
            continue;
        }
        let col = indexed.point.column.0;
        if col >= cols as usize {
            continue;
        }
        let idx = row as usize * cols as usize + col;
        cells[idx] = convert_cell(indexed.cell, content.colors);
    }

    let mut cursor_col = 0u16;
    let mut cursor_row = 0u16;
    let mut cursor_visible = content.cursor.shape != CursorShape::Hidden;

    if cursor_visible {
        let row = content.cursor.point.line.0 + display_offset;
        if row < 0 || row >= rows as i32 {
            cursor_visible = false;
        } else {
            cursor_row = row as u16;
            cursor_col = content.cursor.point.column.0 as u16;
        }
    }

    ServerFrame {
        cols,
        rows,
        cursor_col,
        cursor_row,
        cursor_visible,
        cells,
    }
}

fn convert_cell(cell: &alacritty_terminal::term::cell::Cell, colors: &alacritty_terminal::term::color::Colors) -> ServerCell {
    let flags = cell.flags;
    let mut fg = resolve_color(cell.fg, colors);
    let mut bg = resolve_color(cell.bg, colors);

    if flags.contains(TermFlags::INVERSE) {
        std::mem::swap(&mut fg, &mut bg);
    }

    let mut c = cell.c;
    if flags.contains(TermFlags::WIDE_CHAR_SPACER) {
        c = ' ';
    }

    ServerCell {
        c,
        fg,
        bg,
        bold: flags.contains(TermFlags::BOLD),
        italic: flags.contains(TermFlags::ITALIC),
        underline: flags.intersects(TermFlags::ALL_UNDERLINES),
        inverse: flags.contains(TermFlags::INVERSE),
    }
}

fn resolve_color(color: Color, colors: &alacritty_terminal::term::color::Colors) -> [u8; 3] {
    match color {
        Color::Spec(rgb) => [rgb.r, rgb.g, rgb.b],
        Color::Indexed(idx) => color_256(idx),
        Color::Named(named) => {
            if let Some(rgb) = colors[named] {
                [rgb.r, rgb.g, rgb.b]
            } else {
                resolve_named_color(named)
            }
        }
    }
}

fn resolve_named_color(named: NamedColor) -> [u8; 3] {
    match named {
        NamedColor::Foreground => DEFAULT_FG,
        NamedColor::Background => DEFAULT_BG,
        NamedColor::Cursor => DEFAULT_FG,
        NamedColor::BrightForeground => ANSI_16[15],
        NamedColor::DimForeground => dim_color(DEFAULT_FG),
        NamedColor::Black => ANSI_16[0],
        NamedColor::Red => ANSI_16[1],
        NamedColor::Green => ANSI_16[2],
        NamedColor::Yellow => ANSI_16[3],
        NamedColor::Blue => ANSI_16[4],
        NamedColor::Magenta => ANSI_16[5],
        NamedColor::Cyan => ANSI_16[6],
        NamedColor::White => ANSI_16[7],
        NamedColor::BrightBlack => ANSI_16[8],
        NamedColor::BrightRed => ANSI_16[9],
        NamedColor::BrightGreen => ANSI_16[10],
        NamedColor::BrightYellow => ANSI_16[11],
        NamedColor::BrightBlue => ANSI_16[12],
        NamedColor::BrightMagenta => ANSI_16[13],
        NamedColor::BrightCyan => ANSI_16[14],
        NamedColor::BrightWhite => ANSI_16[15],
        NamedColor::DimBlack => dim_color(ANSI_16[0]),
        NamedColor::DimRed => dim_color(ANSI_16[1]),
        NamedColor::DimGreen => dim_color(ANSI_16[2]),
        NamedColor::DimYellow => dim_color(ANSI_16[3]),
        NamedColor::DimBlue => dim_color(ANSI_16[4]),
        NamedColor::DimMagenta => dim_color(ANSI_16[5]),
        NamedColor::DimCyan => dim_color(ANSI_16[6]),
        NamedColor::DimWhite => dim_color(ANSI_16[7]),
    }
}

fn dim_color(color: [u8; 3]) -> [u8; 3] {
    let scale = 2u16;
    [
        ((color[0] as u16 * scale) / 3) as u8,
        ((color[1] as u16 * scale) / 3) as u8,
        ((color[2] as u16 * scale) / 3) as u8,
    ]
}

fn color_256(idx: u8) -> [u8; 3] {
    match idx {
        0 => [0, 0, 0],
        1 => [205, 49, 49],
        2 => [13, 188, 121],
        3 => [229, 229, 16],
        4 => [36, 114, 200],
        5 => [188, 63, 188],
        6 => [17, 168, 205],
        7 => [229, 229, 229],
        8 => [102, 102, 102],
        9 => [241, 76, 76],
        10 => [35, 209, 139],
        11 => [245, 245, 67],
        12 => [59, 142, 234],
        13 => [214, 112, 214],
        14 => [41, 184, 219],
        15 => [255, 255, 255],
        16..=231 => {
            let idx = idx - 16;
            let r = (idx / 36) * 51;
            let g = ((idx / 6) % 6) * 51;
            let b = (idx % 6) * 51;
            [r, g, b]
        }
        232..=255 => {
            let gray = (idx - 232) * 10 + 8;
            [gray, gray, gray]
        }
    }
}

fn resolve_shell() -> String {
    if let Ok(shell) = std::env::var("SHELL") {
        if Path::new(&shell).exists() {
            return shell;
        }
    }

    for candidate in [
        "/bin/zsh",
        "/usr/bin/zsh",
        "/bin/bash",
        "/usr/bin/bash",
        "/bin/sh",
        "/usr/bin/sh",
    ] {
        if Path::new(candidate).exists() {
            return candidate.to_string();
        }
    }

    "/bin/sh".to_string()
}

fn configure_shell_command(cmd: &mut CommandBuilder, shell: &str) {
    let name = Path::new(shell)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(shell);
    match name {
        "zsh" => {
            cmd.arg("-l");
            cmd.arg("-i");
        }
        "bash" => {
            cmd.arg("-l");
            cmd.arg("-i");
        }
        "sh" => {
            cmd.arg("-i");
        }
        "fish" => {
            cmd.arg("-l");
            cmd.arg("-i");
        }
        _ => {}
    }
}
