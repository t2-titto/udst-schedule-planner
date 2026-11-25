//! Local-only backend for the schedule planner.
//!
//! * Listens on 127.0.0.1 only. Every request must carry a Host header that names this server, and an Origin
//!   (if present) that names this server or one you allow with PLANNER_DEV_ORIGINS. That defeats DNS rebinding
//!   and cross-site form posts to a password endpoint.
//! * Credentials are typed into the page and kept in this process's memory only, wiped on sign-out. They are never
//!   written to disk, logged or put in a URL. One signed-in connector session is kept in memory too, so a fetch does
//!   not pay for a new sign-in.
//! * Reads, previews and writes all go through the linked connector's own gate. Writes stay off until the consent
//!   screen has been accepted (see `write.rs`).

mod log;
mod ps;
mod seats;
mod write;

use axum::extract::{FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use ps::Creds;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tower_http::services::{ServeDir, ServeFile};

const LOGIN_COOLDOWN: Duration = Duration::from_secs(10);
const MAX_ROWS: usize = 12;

struct Inner {
    creds: Mutex<Option<Creds>>,
    /// The live connector session, consent state and issued previews. See `write.rs`.
    write: Arc<Mutex<write::WriteState>>,
    /// Demo mode: the connector's invented PeopleSoft, no network and no account.
    demo: bool,
    /// Ask for the typed phrase again at confirm time (PLANNER_CONFIRM=phrase). Default is one click.
    confirm_phrase: bool,
    last_failed_login: Mutex<Option<Instant>>,
    /// One connector operation at a time: the connector spaces its own requests, and so should we.
    run_lock: tokio::sync::Mutex<()>,
    /// What the status endpoints report. Only ever held for an instant, so a status check never waits for PeopleSoft.
    snap: Mutex<write::Snap>,
    /// Set by "Disable writes" and sign-out; long write operations look at it between their steps and stop.
    stop: Arc<std::sync::atomic::AtomicBool>,
    inflight: AtomicUsize,
    /// How many extra read-only sessions a cold fetch may open at once (PLANNER_PARALLEL_READS, default 3; 0 or 1 turns it off).
    parallel_reads: usize,
    hosts: HashSet<String>,
    origins: HashSet<String>,
}

type App = Arc<Inner>;

struct Shared {
    sessions: Mutex<HashMap<String, App>>,
    preset: Option<Creds>,
    demo: bool,
    confirm_phrase: bool,
    parallel_reads: usize,
    inflight: AtomicUsize,
    hosts: HashSet<String>,
    origins: HashSet<String>,
}

impl Shared {
    fn app_for(&self, sid: &str) -> App {
        let mut map = self.sessions.lock().unwrap();
        if let Some(app) = map.get(sid) {
            return app.clone();
        }
        // Only the headerless "default" slot inherits --credentials-file; every browser/private-window session starts fresh.
        let creds = if sid == "default" { self.preset.clone() } else { None };
        let app = Arc::new(Inner {
            creds: Mutex::new(creds),
            write: Arc::new(Mutex::new(write::WriteState::default())),
            demo: self.demo,
            confirm_phrase: self.confirm_phrase,
            last_failed_login: Mutex::new(None),
            run_lock: tokio::sync::Mutex::new(()),
            snap: Mutex::new(write::Snap::default()),
            stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            inflight: AtomicUsize::new(0),
            parallel_reads: self.parallel_reads,
            hosts: self.hosts.clone(),
            origins: self.origins.clone(),
        });
        write::start_idle_watch(app.clone());
        map.insert(sid.to_string(), app.clone());
        app
    }
}

type SharedState = Arc<Shared>;

struct SessionApp(App);

impl<S: Send + Sync> FromRequestParts<S> for SessionApp {
    type Rejection = ApiErr;
    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<App>()
            .cloned()
            .map(SessionApp)
            .ok_or_else(|| ApiErr(StatusCode::INTERNAL_SERVER_ERROR, "Missing session.".into()))
    }
}

#[derive(Debug)]
struct ApiErr(StatusCode, String);

impl std::fmt::Display for ApiErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.1)
    }
}

impl IntoResponse for ApiErr {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

async fn api_not_found() -> ApiErr {
    ApiErr(StatusCode::NOT_FOUND, "No such API route.".into())
}

fn current_creds(app: &App) -> Result<Creds, ApiErr> {
    app.creds
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| ApiErr(StatusCode::UNAUTHORIZED, "Sign in first.".into()))
}

// ---------------------------------------------------------------- guard

async fn guard(State(shared): State<SharedState>, mut req: Request, next: Next) -> Response {
    let t0 = Instant::now();
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let sid = match req.headers().get("x-planner-session").and_then(|v| v.to_str().ok()) {
        Some(s) if !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') => s.to_string(),
        Some(_) => return (StatusCode::BAD_REQUEST, "invalid x-planner-session header").into_response(),
        None => "default".to_string(),
    };
    let app = shared.app_for(&sid);
    req.extensions_mut().insert(app.clone());
    let inflight = shared.inflight.fetch_add(1, Ordering::SeqCst) + 1;
    let res = guard_inner(&app, req, next).await;
    shared.inflight.fetch_sub(1, Ordering::SeqCst);
    let ms = log::ms_since(t0);
    log::event("http", json!({ "method": method, "path": path, "status": res.status().as_u16(), "ms": ms, "inflight": inflight, "slow": ms > 1000.0 }));
    res
}

async fn guard_inner(app: &App, req: Request, next: Next) -> Response {
    let h = req.headers();
    let host_ok = h
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| app.hosts.contains(&v.to_ascii_lowercase()));
    if !host_ok {
        return (StatusCode::FORBIDDEN, "unexpected host").into_response();
    }
    if let Some(o) = h.get(header::ORIGIN) {
        let ok = o
            .to_str()
            .ok()
            .is_some_and(|v| app.origins.contains(&v.to_ascii_lowercase()));
        if !ok {
            return (StatusCode::FORBIDDEN, "unexpected origin").into_response();
        }
    }
    let is_api = req.uri().path().starts_with("/api");
    if is_api && !matches!(*req.method(), Method::GET | Method::HEAD) {
        // A custom header cannot be sent cross-site without a CORS preflight, which we never answer.
        let custom = h.get("x-planner-request").is_some_and(|v| v == "1");
        if !custom {
            return (StatusCode::FORBIDDEN, "missing x-planner-request header").into_response();
        }
    }
    let mut res = next.run(req).await;
    let hdrs = res.headers_mut();
    hdrs.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    hdrs.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    hdrs.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
        ),
    );
    if is_api {
        hdrs.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    res
}

// ---------------------------------------------------------------- handlers

async fn status(SessionApp(app): SessionApp) -> Json<Value> {
    if app.demo {
        return Json(json!({ "binaryFound": true, "signedIn": true, "userHint": "demo", "demo": true }));
    }
    let hint = app.creds.lock().unwrap().as_ref().map(|c| c.hint());
    Json(json!({
        "binaryFound": true,
        "signedIn": hint.is_some(),
        "userHint": hint,
        "demo": false,
    }))
}

#[derive(Deserialize)]
struct LoginReq {
    username: String,
    password: String,
}

async fn login(SessionApp(app): SessionApp, Json(r): Json<LoginReq>) -> Result<Json<Value>, ApiErr> {
    if app.demo {
        return Ok(Json(json!({ "signedIn": true, "userHint": "demo" })));
    }
    if let Some(t) = *app.last_failed_login.lock().unwrap() {
        let left = LOGIN_COOLDOWN.saturating_sub(t.elapsed());
        if !left.is_zero() {
            return Err(ApiErr(
                StatusCode::TOO_MANY_REQUESTS,
                format!("Wait {} s before trying again (protects your account from lock-outs).", left.as_secs() + 1),
            ));
        }
    }
    let creds = Creds::new(&r.username, &r.password).map_err(|m| ApiErr(StatusCode::BAD_REQUEST, m.into()))?;
    let hint = creds.hint();
    // The connector's own login runs here and the session stays in memory. A wrong password stops after one attempt.
    if let Err(e) = write::sign_in(&app, creds.clone()).await {
        *app.last_failed_login.lock().unwrap() = Some(Instant::now());
        return Err(e);
    }
    *app.creds.lock().unwrap() = Some(creds);
    Ok(Json(json!({ "signedIn": true, "userHint": hint })))
}

async fn logout(SessionApp(app): SessionApp) -> Json<Value> {
    *app.creds.lock().unwrap() = None; // Drop zeroizes
    write::drop_session_now(&app); // signing out also switches writes off and forgets the live session (never waits)
    Json(json!({ "signedIn": false }))
}

/// The Plan & Enrol class list: every course with its term and whether classes are available.
async fn classes(SessionApp(app): SessionApp) -> Result<Json<Value>, ApiErr> {
    Ok(Json(write::classes(&app).await?))
}

#[derive(Deserialize)]
struct SectionsReq {
    rows: Vec<u32>,
    /// Course keys the page remembers from an earlier fetch, by course code. Used only to skip the slow way in; the
    /// server checks every result against the class list, so a stale key can cost time but never give wrong data.
    #[serde(default)]
    known: HashMap<String, Value>,
}

/// Sections for the requested class-list rows, as the connector parsed them.
async fn sections(SessionApp(app): SessionApp, Json(r): Json<SectionsReq>) -> Result<Json<Value>, ApiErr> {
    let mut rows = r.rows.clone();
    rows.sort_unstable();
    rows.dedup();
    if rows.is_empty() || rows.len() > MAX_ROWS || rows.iter().any(|&x| x > 500) {
        return Err(ApiErr(StatusCode::BAD_REQUEST, format!("Pick between 1 and {MAX_ROWS} courses.")));
    }
    if r.known.len() > 60 || r.known.keys().any(|k| k.len() > 24) {
        return Err(ApiErr(StatusCode::BAD_REQUEST, "Too many cached keys.".into()));
    }
    Ok(Json(write::sections(&app, rows, r.known).await?))
}

// ---------------------------------------------------------------- main

fn env_list(name: &str) -> Vec<String> {
    std::env::var(name)
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

fn creds_from_file(path: &str) -> Result<Creds, String> {
    let text = std::fs::read_to_string(path).map_err(|_| "could not read the credentials file".to_string())?;
    let mut lines = text.lines();
    let user = lines.next().ok_or("credentials file needs two lines")?;
    let pass = lines.next().ok_or("credentials file needs two lines")?;
    Creds::new(user, pass.trim_end_matches('\r')).map_err(|m| m.to_string())
}

#[tokio::main]
async fn main() {
    let port: u16 = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(6767);
    let dist = PathBuf::from(std::env::var("PLANNER_DIST").unwrap_or_else(|_| "../frontend/dist".to_string()));
    let data_dir = PathBuf::from(std::env::var("PLANNER_DATA").unwrap_or_else(|_| "../data".to_string()));
    let log_path = match log::init(&data_dir.join("logs")) {
        Ok(p) => Some(p),
        Err(e) => {
            eprintln!("warning: could not open the log under {}: {e}", data_dir.display());
            None
        }
    };

    // Optional: skip typing by pointing at your own two-line credentials file at launch. It is read once, here, and
    // held in memory like a typed login; the session is created on first use.
    let mut preset: Option<Creds> = None;
    let mut demo = std::env::var("PLANNER_DEMO").is_ok_and(|v| v == "1");
    let confirm_phrase = std::env::var("PLANNER_CONFIRM").is_ok_and(|v| v == "phrase");
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        if a == "--demo" {
            demo = true;
        } else if a == "--credentials-file" {
            let Some(p) = it.next() else {
                eprintln!("--credentials-file needs a path");
                std::process::exit(2);
            };
            match creds_from_file(&p) {
                Ok(c) => preset = Some(c),
                Err(m) => {
                    eprintln!("{m}");
                    std::process::exit(2);
                }
            }
        } else {
            eprintln!("unknown argument {a} (use --credentials-file PATH or --demo)");
            std::process::exit(2);
        }
    }

    let hosts: HashSet<String> = [format!("localhost:{port}"), format!("127.0.0.1:{port}"), format!("[::1]:{port}")].into_iter().collect();
    let mut origins: HashSet<String> = hosts.iter().map(|h| format!("http://{h}")).collect();
    origins.extend(env_list("PLANNER_DEV_ORIGINS"));
    let mut host_set = hosts;
    for o in env_list("PLANNER_DEV_ORIGINS") {
        if let Some(h) = o.strip_prefix("http://") {
            host_set.insert(h.to_string());
        }
    }

    let shared: SharedState = Arc::new(Shared {
        sessions: Mutex::new(HashMap::new()),
        preset,
        demo,
        confirm_phrase,
        parallel_reads: std::env::var("PLANNER_PARALLEL_READS").ok().and_then(|v| v.parse().ok()).unwrap_or(3),
        inflight: AtomicUsize::new(0),
        hosts: host_set,
        origins,
    });

    let api = Router::new()
        .route("/status", get(status))
        .route("/session", post(login).delete(logout))
        .route("/classes", get(classes))
        .route("/sections", post(sections))
        .route("/refresh", post(|SessionApp(app): SessionApp, j| write::refresh(State(app), j)))
        .route("/write/status", get(|SessionApp(app): SessionApp| write::status(State(app))))
        .route("/write/arm", post(|SessionApp(app): SessionApp, j| write::arm(State(app), j)))
        .route("/write/disarm", post(|SessionApp(app): SessionApp| write::disarm(State(app))))
        .route("/write/read", post(|SessionApp(app): SessionApp, j| write::read(State(app), j)))
        .route("/write/held_options", post(|SessionApp(app): SessionApp, j| write::held_options(State(app), j)))
        .route("/write/preview", post(|SessionApp(app): SessionApp, j| write::preview(State(app), j)))
        .route("/write/confirm", post(|SessionApp(app): SessionApp, j| write::confirm(State(app), j)))
        .route("/write/plan/apply", post(|SessionApp(app): SessionApp, j| write::apply_start(State(app), j)))
        .route("/write/plan/progress", get(write::apply_progress))
        // Anything else under /api is a 404 with a JSON body, never the app shell (a mistyped route used to answer 200 with HTML).
        .fallback(api_not_found);

    let index = dist.join("index.html");
    let static_files = ServeDir::new(&dist).fallback(ServeFile::new(index));
    let router = Router::new()
        .nest("/api", api)
        .fallback_service(static_files)
        .layer(middleware::from_fn_with_state(shared.clone(), guard))
        .with_state(shared.clone());

    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("could not listen on 127.0.0.1:{port}: {e}");
            std::process::exit(1);
        }
    };
    if shared.demo {
        println!("Schedule planner: http://localhost:{port}  DEMO MODE (invented PeopleSoft, nothing real is touched)");
    } else {
        println!("Schedule planner: http://localhost:{port}  (local only; writes stay off until you acknowledge the statements on the sign-in screen)");
    }
    if let Some(p) = &log_path {
        println!("Timing log: {} (one JSON object per line; never any passwords, cookies or page bodies)", p.display());
    }
    log::event("start", json!({ "port": port, "parallel_reads": shared.parallel_reads, "min_gap_ms": std::env::var("PLANNER_MIN_GAP_MS").ok(), "demo": shared.demo, "confirm": if shared.confirm_phrase { "phrase" } else { "click" }, "pid": std::process::id() }));
    if !dist.join("index.html").is_file() {
        println!("note: {} has no build yet; run `npm run build` in frontend/ (or use the Vite dev server)", dist.display());
    }
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .ok();
}
