//! Local HTTP server for Planie: serves the built frontend and mediates PeopleSoft reads/writes.

mod demo;
mod log;
mod ps;
mod seats;
mod write;

use axum::body::Body;
use axum::extract::{FromRequestParts, Query, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_SECURITY_POLICY, COOKIE, HOST, ORIGIN, SET_COOKIE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, Method, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use ps::Creds;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tower_http::services::{ServeDir, ServeFile};

const DEFAULT_PORT: u16 = 6767;
const JAR_COOKIE: &str = "planie_jar";
const ACCOUNT_HEADER: &str = "x-planie-account";
const MAX_JARS: usize = 1024;
const MAX_ACCOUNTS_PER_JAR: usize = 8;
const MAX_LOGIN_COOLDOWNS: usize = 4096;
const LOGIN_COOLDOWN: Duration = Duration::from_secs(10);
const JAR_IDLE_TTL: Duration = Duration::from_secs(2 * 60 * 60);
const CRED_IDLE_TTL: Duration = Duration::from_secs(2 * 60 * 60);

const CSP_HEADER: &str = "default-src 'self'; img-src 'self' data:; style-src 'self'; script-src 'self'; connect-src 'self'; frame-src https://files.gamezhero.com https://subwaysurfers76.github.io; frame-ancestors 'none'; base-uri 'none'; form-action 'none'";

#[derive(Debug)]
pub struct ApiErr(pub StatusCode, pub String);

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

pub type App = Arc<Inner>;

pub struct Inner {
    pub creds: Mutex<Option<Creds>>,
    pub write: Arc<Mutex<write::WriteState>>,
    pub demo: bool,
    pub confirm_phrase: bool,
    pub run_lock: tokio::sync::Mutex<()>,
    pub snap: Mutex<write::Snap>,
    pub stop: Arc<AtomicBool>,
    pub shutdown: Arc<AtomicBool>,
    pub parallel_reads: usize,
    pub hosts: HashSet<String>,
    pub origins: HashSet<String>,
    pub job: Arc<Mutex<Option<Value>>>,
    pub fired: Arc<Mutex<Vec<Value>>>,
    pub prestaged: Mutex<Vec<(String, String, Instant, write::WriteState)>>,
    pub said: Arc<Mutex<Vec<(String, String)>>>,
    pub demo_state: Arc<Mutex<demo::DemoState>>,
    pub last_seen: Mutex<Instant>,
}

impl Inner {
    fn new(
        demo: bool,
        confirm_phrase: bool,
        parallel_reads: usize,
        hosts: HashSet<String>,
        origins: HashSet<String>,
        creds: Option<Creds>,
    ) -> App {
        let app = Arc::new(Self {
            creds: Mutex::new(creds),
            write: Arc::new(Mutex::new(write::WriteState::default())),
            demo,
            confirm_phrase,
            run_lock: tokio::sync::Mutex::new(()),
            snap: Mutex::new(write::Snap::default()),
            stop: Arc::new(AtomicBool::new(false)),
            shutdown: Arc::new(AtomicBool::new(false)),
            parallel_reads,
            hosts,
            origins,
            job: Arc::new(Mutex::new(None)),
            fired: Arc::new(Mutex::new(Vec::new())),
            prestaged: Mutex::new(Vec::new()),
            said: Arc::new(Mutex::new(Vec::new())),
            demo_state: Arc::new(Mutex::new(demo::DemoState::new())),
            last_seen: Mutex::new(Instant::now()),
        });
        if tokio::runtime::Handle::try_current().is_ok() {
            write::start_idle_watch(Arc::clone(&app));
        }
        app
    }
}

pub fn current_creds(app: &App) -> Result<Creds, ApiErr> {
    app.creds
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .ok_or_else(|| ApiErr(StatusCode::UNAUTHORIZED, "Sign in first.".into()))
}

struct AccountSlot {
    id: String,
    user_key: String,
    app: App,
}

struct Jar {
    slots: Vec<AccountSlot>,
    active: Option<String>,
    last_seen: Instant,
}

pub struct Shared {
    demo: bool,
    confirm_phrase: bool,
    parallel_reads: usize,
    preset_creds: Option<Creds>,
    hosts: HashSet<String>,
    origins: HashSet<String>,
    jars: Mutex<HashMap<String, Jar>>,
    login_cooldowns: Mutex<HashMap<String, Instant>>,
}

impl Shared {
    fn new(
        demo: bool,
        confirm_phrase: bool,
        parallel_reads: usize,
        preset_creds: Option<Creds>,
        hosts: HashSet<String>,
        origins: HashSet<String>,
    ) -> Arc<Self> {
        Arc::new(Self {
            demo,
            confirm_phrase,
            parallel_reads,
            preset_creds,
            hosts,
            origins,
            jars: Mutex::new(HashMap::new()),
            login_cooldowns: Mutex::new(HashMap::new()),
        })
    }

    fn new_app(&self, creds: Option<Creds>) -> App {
        Inner::new(
            self.demo,
            self.confirm_phrase,
            self.parallel_reads,
            self.hosts.clone(),
            self.origins.clone(),
            creds,
        )
    }
}

fn rand_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::getrandom(&mut buf).expect("CSPRNG failure");
    let mut out = String::with_capacity(bytes * 2);
    for b in buf {
        out.push(char::from_digit(u32::from(b >> 4), 16).unwrap());
        out.push(char::from_digit(u32::from(b & 0x0f), 16).unwrap());
    }
    out
}

fn parse_cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    for val in headers.get_all(COOKIE) {
        let Ok(s) = val.to_str() else { continue };
        for part in s.split(';') {
            let trimmed = part.trim();
            if let Some((k, v)) = trimmed.split_once('=') {
                if k.trim() == name && !v.is_empty() && v.len() <= 128 && v.bytes().all(|b| b.is_ascii_alphanumeric()) {
                    return Some(v.trim());
                }
            }
        }
    }
    None
}

fn header_account_id(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(ACCOUNT_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'))
}

fn jar_cookie_header(jar_id: &str, headers: &HeaderMap, clear: bool) -> Result<HeaderValue, ApiErr> {
    let secure = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|p| p.eq_ignore_ascii_case("https"));
    let mut s = if clear {
        format!("{JAR_COOKIE}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0")
    } else {
        format!("{JAR_COOKIE}={jar_id}; HttpOnly; SameSite=Strict; Path=/")
    };
    if secure {
        s.push_str("; Secure");
    }
    HeaderValue::from_str(&s).map_err(|_| ApiErr(StatusCode::INTERNAL_SERVER_ERROR, "internal error".into()))
}

fn host_allowed(hosts: &HashSet<String>, host_hdr: &str) -> bool {
    let h = host_hdr.trim().to_ascii_lowercase();
    hosts.contains(&h)
}

fn check_request_origin(shared: &Shared, method: &Method, headers: &HeaderMap, path: &str) -> Result<(), StatusCode> {
    let host = headers.get(HOST).and_then(|v| v.to_str().ok()).unwrap_or("");
    if host.is_empty() || !host_allowed(&shared.hosts, host) {
        return Err(StatusCode::FORBIDDEN);
    }

    if let Some(sfs) = headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()) {
        if sfs.eq_ignore_ascii_case("cross-site") {
            return Err(StatusCode::FORBIDDEN);
        }
    }

    let origin = headers.get(ORIGIN).and_then(|v| v.to_str().ok()).map(|s| s.trim().to_ascii_lowercase());
    let mutating = !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS);
    if path.starts_with("/api") && mutating {
        match origin.as_deref() {
            Some(o) if shared.origins.contains(o) => {}
            _ => return Err(StatusCode::FORBIDDEN),
        }
        let marker = headers.get("x-planner-request").and_then(|v| v.to_str().ok());
        if marker != Some("1") {
            return Err(StatusCode::FORBIDDEN);
        }
    } else if let Some(o) = origin.as_deref() {
        if !shared.origins.contains(o) {
            return Err(StatusCode::FORBIDDEN);
        }
    }
    Ok(())
}

fn apply_security_headers(res: &mut Response, is_api: bool) {
    let h = res.headers_mut();
    h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    h.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    h.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    h.insert(CONTENT_SECURITY_POLICY, HeaderValue::from_static(CSP_HEADER));
    if is_api {
        h.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
}

async fn guard(State(shared): State<Arc<Shared>>, req: Request<Body>, next: Next) -> Response {
    let path = req.uri().path().to_string();
    let is_api = path.starts_with("/api");
    if let Err(code) = check_request_origin(&shared, req.method(), req.headers(), &path) {
        let mut r = (code, Json(json!({ "error": "Request origin not allowed." }))).into_response();
        apply_security_headers(&mut r, is_api);
        return r;
    }
    let mut res = next.run(req).await;
    apply_security_headers(&mut res, is_api);
    res
}

fn evict_stale_jars(jars: &mut HashMap<String, Jar>) {
    jars.retain(|_, j| {
        let keep = j.last_seen.elapsed() <= JAR_IDLE_TTL;
        if !keep {
            for slot in &j.slots {
                slot.app.shutdown.store(true, Ordering::SeqCst);
                write::drop_session_now(&slot.app);
                *slot.app.creds.lock().unwrap_or_else(|e| e.into_inner()) = None;
            }
        }
        keep
    });
    if jars.len() >= MAX_JARS {
        if let Some(oldest_key) = jars
            .iter()
            .min_by_key(|(_, j)| j.last_seen)
            .map(|(k, _)| k.clone())
        {
            if let Some(j) = jars.remove(&oldest_key) {
                for slot in &j.slots {
                    slot.app.shutdown.store(true, Ordering::SeqCst);
                    write::drop_session_now(&slot.app);
                    *slot.app.creds.lock().unwrap_or_else(|e| e.into_inner()) = None;
                }
            }
        }
    }
}

fn slot_user_hint(app: &App) -> Option<String> {
    if app.demo {
        return Some("demo".into());
    }
    app.creds
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(Creds::hint)
}

fn pick_slot<'a>(jar: &'a mut Jar, requested_acct: Option<&str>) -> Option<&'a AccountSlot> {
    jar.last_seen = Instant::now();
    let target = match requested_acct {
        // An explicit x-planie-account header must match that exact account; never silently fall back to another account.
        Some(id) => jar.slots.iter().position(|s| s.id == id),
        None => jar
            .active
            .as_deref()
            .and_then(|id| jar.slots.iter().position(|s| s.id == id))
            .or_else(|| if jar.slots.is_empty() { None } else { Some(0) }),
    };
    target.map(|i| {
        let slot = &jar.slots[i];
        *slot.app.last_seen.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
        slot
    })
}

fn status_payload_for(app: &App, slot_id: Option<&str>) -> Value {
    let mut v = write::status_json(app);
    let signed_in = app.demo || app.creds.lock().unwrap_or_else(|e| e.into_inner()).is_some();
    if let Some(obj) = v.as_object_mut() {
        obj.insert("signedIn".into(), json!(signed_in));
        obj.insert("userHint".into(), json!(if signed_in { slot_user_hint(app) } else { None }));
        obj.insert("accountId".into(), json!(slot_id));
        obj.insert("parallelReads".into(), json!(app.parallel_reads));
    }
    v
}

struct AuthApp(pub App);

impl FromRequestParts<Arc<Shared>> for AuthApp {
    type Rejection = ApiErr;

    async fn from_request_parts(parts: &mut Parts, shared: &Arc<Shared>) -> Result<Self, Self::Rejection> {
        let jar_id = parse_cookie(&parts.headers, JAR_COOKIE)
            .ok_or_else(|| ApiErr(StatusCode::UNAUTHORIZED, "Sign in first.".into()))?;
        let acct = header_account_id(&parts.headers);
        let app = {
            let mut jars = shared.jars.lock().unwrap_or_else(|e| e.into_inner());
            let jar = jars
                .get_mut(jar_id)
                .ok_or_else(|| ApiErr(StatusCode::UNAUTHORIZED, "Sign in first.".into()))?;
            let slot = pick_slot(jar, acct).ok_or_else(|| ApiErr(StatusCode::UNAUTHORIZED, "Sign in first.".into()))?;
            Arc::clone(&slot.app)
        };
        if !app.demo && app.creds.lock().unwrap_or_else(|e| e.into_inner()).is_none() {
            return Err(ApiErr(StatusCode::UNAUTHORIZED, "Sign in first.".into()));
        }
        Ok(AuthApp(app))
    }
}

async fn status_handler(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Result<Response, ApiErr> {
    let req_acct = header_account_id(&headers);
    if let Some(jar_id) = parse_cookie(&headers, JAR_COOKIE) {
        let found = {
            let mut jars = shared.jars.lock().unwrap_or_else(|e| e.into_inner());
            jars.get_mut(jar_id)
                .and_then(|j| pick_slot(j, req_acct).map(|s| (s.id.clone(), Arc::clone(&s.app))))
        };
        if let Some((slot_id, app)) = found {
            return Ok(Json(status_payload_for(&app, Some(&slot_id))).into_response());
        }
    }

    // In --demo or --credentials-file mode, bootstrap an automatic session jar on initial status read.
    if shared.demo || shared.preset_creds.is_some() {
        let app = shared.new_app(shared.preset_creds.clone());
        let jar_id = rand_hex(16);
        let slot_id = rand_hex(8);
        let user_key = shared
            .preset_creds
            .as_ref()
            .map(|c| c.user.trim().to_ascii_lowercase())
            .unwrap_or_else(|| "demo".into());
        {
            let mut jars = shared.jars.lock().unwrap_or_else(|e| e.into_inner());
            evict_stale_jars(&mut jars);
            jars.insert(
                jar_id.clone(),
                Jar {
                    slots: vec![AccountSlot {
                        id: slot_id.clone(),
                        user_key,
                        app: Arc::clone(&app),
                    }],
                    active: Some(slot_id.clone()),
                    last_seen: Instant::now(),
                },
            );
        }
        let mut res = Json(status_payload_for(&app, Some(&slot_id))).into_response();
        res.headers_mut().insert(SET_COOKIE, jar_cookie_header(&jar_id, &headers, false)?);
        return Ok(res);
    }

    // Unauthenticated live status allocates zero state and sets no cookie.
    let tmp = Inner::new(
        false,
        shared.confirm_phrase,
        shared.parallel_reads,
        HashSet::new(),
        HashSet::new(),
        None,
    );
    tmp.shutdown.store(true, Ordering::SeqCst);
    Ok(Json(status_payload_for(&tmp, None)).into_response())
}

#[derive(Deserialize)]
struct LoginReq {
    #[serde(alias = "username")]
    user: String,
    #[serde(alias = "password")]
    pass: String,
}

fn check_cooldown(shared: &Shared, user_key: &str) -> Result<(), ApiErr> {
    let mut map = shared.login_cooldowns.lock().unwrap_or_else(|e| e.into_inner());
    map.retain(|_, t| t.elapsed() < LOGIN_COOLDOWN);
    if let Some(t) = map.get(user_key) {
        let rem = LOGIN_COOLDOWN.saturating_sub(t.elapsed()).as_secs().max(1);
        return Err(ApiErr(
            StatusCode::TOO_MANY_REQUESTS,
            format!("Wait {rem} seconds before trying to sign in again."),
        ));
    }
    Ok(())
}

fn record_failed_login(shared: &Shared, user_key: String) {
    let mut map = shared.login_cooldowns.lock().unwrap_or_else(|e| e.into_inner());
    map.retain(|_, t| t.elapsed() < LOGIN_COOLDOWN);
    if map.len() >= MAX_LOGIN_COOLDOWNS {
        map.clear();
    }
    map.insert(user_key, Instant::now());
}

fn clear_failed_login(shared: &Shared, user_key: &str) {
    if let Ok(mut map) = shared.login_cooldowns.lock() {
        map.remove(user_key);
    }
}

async fn login_handler(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(req): Json<LoginReq>,
) -> Result<Response, ApiErr> {
    let user_key = req.user.trim().to_ascii_lowercase();
    if user_key.is_empty() || req.pass.is_empty() {
        return Err(ApiErr(StatusCode::BAD_REQUEST, "Enter both your student number and password.".into()));
    }
    if req.user.len() > 64 || req.pass.len() > 256 {
        return Err(ApiErr(StatusCode::BAD_REQUEST, "Credentials are too long.".into()));
    }
    if !shared.demo {
        check_cooldown(&shared, &user_key)?;
    }

    let creds = Creds {
        user: req.user.trim().to_string(),
        pass: req.pass,
    };
    let app = shared.new_app(Some(creds.clone()));
    if !shared.demo {
        if let Err(e) = write::sign_in(&app, creds).await {
            app.shutdown.store(true, Ordering::SeqCst);
            record_failed_login(&shared, user_key);
            return Err(e);
        }
        clear_failed_login(&shared, &user_key);
    }

    let existing_jar = parse_cookie(&headers, JAR_COOKIE).map(str::to_string);
    let slot_id = rand_hex(8);
    let (jar_id, effective_slot_id, need_cookie) = {
        let mut jars = shared.jars.lock().unwrap_or_else(|e| e.into_inner());
        evict_stale_jars(&mut jars);
        if let Some(jid) = existing_jar.filter(|id| jars.contains_key(id)) {
            let jar = jars.get_mut(&jid).unwrap();
            jar.last_seen = Instant::now();
            if let Some(prev) = jar.slots.iter_mut().find(|s| s.user_key == user_key) {
                prev.app.shutdown.store(true, Ordering::SeqCst);
                write::drop_session_now(&prev.app);
                prev.app = Arc::clone(&app);
                let sid = prev.id.clone();
                jar.active = Some(sid.clone());
                (jid, sid, false)
            } else {
                if jar.slots.len() >= MAX_ACCOUNTS_PER_JAR {
                    let removed = jar.slots.remove(0);
                    removed.app.shutdown.store(true, Ordering::SeqCst);
                    write::drop_session_now(&removed.app);
                }
                jar.slots.push(AccountSlot {
                    id: slot_id.clone(),
                    user_key,
                    app: Arc::clone(&app),
                });
                jar.active = Some(slot_id.clone());
                (jid, slot_id, false)
            }
        } else {
            let jid = rand_hex(16);
            jars.insert(
                jid.clone(),
                Jar {
                    slots: vec![AccountSlot {
                        id: slot_id.clone(),
                        user_key,
                        app: Arc::clone(&app),
                    }],
                    active: Some(slot_id.clone()),
                    last_seen: Instant::now(),
                },
            );
            (jid, slot_id, true)
        }
    };

    log::event("login_ok", json!({ "demo": shared.demo }));
    let mut res = Json(status_payload_for(&app, Some(&effective_slot_id))).into_response();
    if need_cookie {
        res.headers_mut().insert(SET_COOKIE, jar_cookie_header(&jar_id, &headers, false)?);
    }
    Ok(res)
}

#[derive(Deserialize, Default)]
struct LogoutQuery {
    all: Option<u8>,
}

async fn logout_handler(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(q): Query<LogoutQuery>,
) -> Result<Response, ApiErr> {
    let Some(jar_id) = parse_cookie(&headers, JAR_COOKIE).map(str::to_string) else {
        return Ok(Json(json!({ "ok": true, "signedIn": false })).into_response());
    };
    let req_acct = header_account_id(&headers).map(str::to_string);
    let mut cleared_cookie = false;
    {
        let mut jars = shared.jars.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(jar) = jars.get_mut(&jar_id) {
            if q.all == Some(1) || jar.slots.len() <= 1 {
                for slot in jar.slots.drain(..) {
                    slot.app.shutdown.store(true, Ordering::SeqCst);
                    write::drop_session_now(&slot.app);
                    *slot.app.creds.lock().unwrap_or_else(|e| e.into_inner()) = None;
                }
                jars.remove(&jar_id);
                cleared_cookie = true;
            } else {
                let target = req_acct.as_deref().or(jar.active.as_deref());
                if let Some(pos) = target.and_then(|id| jar.slots.iter().position(|s| s.id == id)) {
                    let removed = jar.slots.remove(pos);
                    removed.app.shutdown.store(true, Ordering::SeqCst);
                    write::drop_session_now(&removed.app);
                    *removed.app.creds.lock().unwrap_or_else(|e| e.into_inner()) = None;
                }
                jar.active = jar.slots.first().map(|s| s.id.clone());
            }
        }
    }
    let mut res = Json(json!({ "ok": true, "signedIn": !cleared_cookie })).into_response();
    if cleared_cookie {
        res.headers_mut().insert(SET_COOKIE, jar_cookie_header("", &headers, true)?);
    }
    Ok(res)
}

async fn accounts_handler(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Json<Value> {
    let Some(jar_id) = parse_cookie(&headers, JAR_COOKIE) else {
        return Json(json!({ "accounts": [], "activeId": null }));
    };
    let mut jars = shared.jars.lock().unwrap_or_else(|e| e.into_inner());
    let Some(jar) = jars.get_mut(jar_id) else {
        return Json(json!({ "accounts": [], "activeId": null }));
    };
    jar.last_seen = Instant::now();
    let active = jar.active.clone();
    let list: Vec<Value> = jar
        .slots
        .iter()
        .filter(|s| s.app.demo || s.app.creds.lock().unwrap_or_else(|e| e.into_inner()).is_some())
        .map(|s| {
            json!({
                "id": s.id,
                "userHint": slot_user_hint(&s.app).unwrap_or_else(|| "••••".into()),
                "active": active.as_deref() == Some(s.id.as_str()),
                "demo": s.app.demo,
            })
        })
        .collect();
    Json(json!({ "accounts": list, "activeId": active }))
}

#[derive(Deserialize)]
struct SwitchAccountReq {
    id: String,
}

async fn switch_account_handler(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(req): Json<SwitchAccountReq>,
) -> Result<Json<Value>, ApiErr> {
    let jar_id = parse_cookie(&headers, JAR_COOKIE)
        .ok_or_else(|| ApiErr(StatusCode::UNAUTHORIZED, "Sign in first.".into()))?;
    let mut jars = shared.jars.lock().unwrap_or_else(|e| e.into_inner());
    let jar = jars
        .get_mut(jar_id)
        .ok_or_else(|| ApiErr(StatusCode::UNAUTHORIZED, "Sign in first.".into()))?;
    let slot = jar
        .slots
        .iter()
        .find(|s| s.id == req.id)
        .ok_or_else(|| ApiErr(StatusCode::NOT_FOUND, "Account not found in this session.".into()))?;
    let app = Arc::clone(&slot.app);
    let slot_id = slot.id.clone();
    jar.active = Some(slot_id.clone());
    jar.last_seen = Instant::now();
    *app.last_seen.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
    Ok(Json(status_payload_for(&app, Some(&slot_id))))
}

#[derive(Deserialize)]
struct SectionsReq {
    rows: Vec<u32>,
    #[serde(default)]
    known: HashMap<String, Value>,
}

async fn classes_handler(AuthApp(app): AuthApp) -> Result<Json<Value>, ApiErr> {
    write::classes(&app).await.map(Json)
}

async fn sections_handler(AuthApp(app): AuthApp, Json(req): Json<SectionsReq>) -> Result<Json<Value>, ApiErr> {
    write::sections(&app, req.rows, req.known).await.map(Json)
}

async fn refresh_handler(AuthApp(app): AuthApp, body: Json<write::reads::RefreshReq>) -> Result<Json<Value>, ApiErr> {
    write::refresh(State(app), body).await
}

async fn bootstrap_handler(AuthApp(app): AuthApp, body: Json<write::reads::RefreshReq>) -> Result<Json<Value>, ApiErr> {
    write::bootstrap(State(app), body).await
}

async fn compatible_sections_handler(
    AuthApp(app): AuthApp,
    body: Json<write::reads::CompatibleSectionsReq>,
) -> Result<Json<Value>, ApiErr> {
    write::compatible_sections(State(app), body).await
}

async fn write_status_handler(AuthApp(app): AuthApp) -> Json<Value> {
    write::status(State(app)).await
}

async fn arm_handler(AuthApp(app): AuthApp, body: Json<write::state::ArmReq>) -> Result<Json<Value>, ApiErr> {
    write::arm(State(app), body).await
}

async fn disarm_handler(AuthApp(app): AuthApp) -> Json<Value> {
    write::disarm(State(app)).await
}

async fn read_handler(AuthApp(app): AuthApp, body: Json<write::planning::ToolReq>) -> Result<Json<Value>, ApiErr> {
    write::read(State(app), body).await
}

async fn held_options_handler(AuthApp(app): AuthApp, body: Json<write::reads::HeldReq>) -> Result<Json<Value>, ApiErr> {
    write::held_options(State(app), body).await
}

async fn preview_handler(AuthApp(app): AuthApp, body: Json<write::planning::ToolReq>) -> Result<Json<Value>, ApiErr> {
    write::preview(State(app), body).await
}

async fn confirm_handler(AuthApp(app): AuthApp, body: Json<write::apply::ConfirmReq>) -> Result<Json<Value>, ApiErr> {
    write::confirm(State(app), body).await
}

async fn apply_start_handler(AuthApp(app): AuthApp, body: Json<write::planning::ApplyReq>) -> Result<Json<Value>, ApiErr> {
    write::apply_start(State(app), body).await
}

async fn apply_progress_handler(AuthApp(app): AuthApp) -> Result<Json<Value>, ApiErr> {
    write::apply_progress(State(app)).await
}

async fn chess_puzzle_handler() -> Result<Json<Value>, ApiErr> {
    let out = tokio::process::Command::new("curl")
        .args([
            "-sS",
            "--max-time",
            "6",
            "-H",
            "Accept: application/json",
            "-H",
            "User-Agent: PlanieSchedulePlanner/1.0",
            "https://lichess.org/api/puzzle/next",
        ])
        .output()
        .await
        .map_err(|_| ApiErr(StatusCode::SERVICE_UNAVAILABLE, "Lichess offline".into()))?;
    if !out.status.success() {
        return Err(ApiErr(StatusCode::SERVICE_UNAVAILABLE, "Lichess unreachable".into()));
    }
    let body: Value = serde_json::from_slice(&out.stdout)
        .map_err(|_| ApiErr(StatusCode::BAD_GATEWAY, "Invalid Lichess response".into()))?;
    let p = body.get("puzzle").ok_or_else(|| ApiErr(StatusCode::BAD_GATEWAY, "Missing puzzle payload".into()))?;
    Ok(Json(json!({
        "id": p["id"],
        "rating": p["rating"],
        "fen": p["fen"],
        "lastMove": p["lastMove"],
        "solution": p["solution"],
        "themes": p["themes"],
    })))
}

fn build_router(shared: Arc<Shared>, dist: PathBuf) -> Router {
    let api = Router::new()
        .route("/api/status", get(status_handler))
        .route("/api/games/chess-puzzle", get(chess_puzzle_handler))
        .route("/api/session", post(login_handler))
        .route("/api/session", delete(logout_handler))
        .route("/api/accounts", get(accounts_handler))
        .route("/api/accounts/switch", post(switch_account_handler))
        .route("/api/classes", get(classes_handler))
        .route("/api/sections", post(sections_handler))
        .route("/api/refresh", post(refresh_handler))
        .route("/api/bootstrap", post(bootstrap_handler))
        .route("/api/compatible_sections", post(compatible_sections_handler))
        .route("/api/write/status", get(write_status_handler))
        .route("/api/write/arm", post(arm_handler))
        .route("/api/write/disarm", post(disarm_handler))
        .route("/api/write/read", post(read_handler))
        .route("/api/write/held_options", post(held_options_handler))
        .route("/api/write/preview", post(preview_handler))
        .route("/api/write/confirm", post(confirm_handler))
        .route("/api/write/plan/apply", post(apply_start_handler))
        .route("/api/write/plan/progress", get(apply_progress_handler));

    let index = dist.join("index.html");
    let static_files = ServeDir::new(&dist).fallback(ServeFile::new(index));

    api.fallback_service(static_files)
        .layer(middleware::from_fn_with_state(Arc::clone(&shared), guard))
        .with_state(shared)
}

fn allowed_hosts_and_origins(port: u16, dev_origins: Option<&str>) -> (HashSet<String>, HashSet<String>) {
    let mut hosts = HashSet::new();
    let mut origins = HashSet::new();
    for h in ["localhost", "127.0.0.1", "[::1]"] {
        hosts.insert(h.to_string());
        hosts.insert(format!("{h}:{port}"));
        origins.insert(format!("http://{h}:{port}"));
        origins.insert(format!("https://{h}:{port}"));
        origins.insert(format!("http://{h}"));
        origins.insert(format!("https://{h}"));
    }
    if let Some(extra) = dev_origins {
        for raw in extra.split(',') {
            let o = raw.trim().trim_end_matches('/').to_ascii_lowercase();
            if o.starts_with("http://") || o.starts_with("https://") {
                origins.insert(o.clone());
                if let Some(host_part) = o.split("://").nth(1) {
                    hosts.insert(host_part.to_string());
                }
            }
        }
    }
    (hosts, origins)
}

fn start_jar_reaper(shared: Arc<Shared>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(60));
        loop {
            tick.tick().await;
            let mut jars = shared.jars.lock().unwrap_or_else(|e| e.into_inner());
            jars.retain(|_, j| {
                for slot in &j.slots {
                    let idle = slot.app.last_seen.lock().unwrap_or_else(|e| e.into_inner()).elapsed();
                    if !slot.app.demo && idle > CRED_IDLE_TTL {
                        let mut c = slot.app.creds.lock().unwrap_or_else(|e| e.into_inner());
                        if c.is_some() {
                            *c = None;
                            write::drop_session_now(&slot.app);
                        }
                    }
                }
                let keep = j.last_seen.elapsed() <= JAR_IDLE_TTL;
                if !keep {
                    for slot in &j.slots {
                        slot.app.shutdown.store(true, Ordering::SeqCst);
                        write::drop_session_now(&slot.app);
                        *slot.app.creds.lock().unwrap_or_else(|e| e.into_inner()) = None;
                    }
                }
                keep
            });
        }
    });
}

fn dist_dir() -> PathBuf {
    let candidates = [
        PathBuf::from("../frontend/dist"),
        PathBuf::from("frontend/dist"),
        PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../frontend/dist")),
    ];
    candidates.into_iter().find(|p| p.join("index.html").is_file()).unwrap_or_else(|| PathBuf::from("../frontend/dist"))
}

fn read_credentials_file(path: &str) -> Result<Creds, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| format!("Cannot read {path}: {e}"))?;
    let mut lines = raw.lines().map(str::trim).filter(|l| !l.is_empty());
    let user = lines.next().ok_or("Credentials file is empty")?.to_string();
    let pass = lines.next().ok_or("Credentials file needs two lines: student number then password")?.to_string();
    Ok(Creds { user, pass })
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut demo = false;
    let mut cred_file: Option<String> = None;
    let mut port: u16 = std::env::var("PORT").ok().and_then(|s| s.parse().ok()).unwrap_or(DEFAULT_PORT);

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--demo" => demo = true,
            "--credentials-file" => {
                i += 1;
                cred_file = args.get(i).cloned();
            }
            "--port" => {
                i += 1;
                if let Some(p) = args.get(i).and_then(|s| s.parse().ok()) {
                    port = p;
                }
            }
            other => {
                eprintln!("Unknown option: {other}");
                std::process::exit(2);
            }
        }
        i += 1;
    }

    let preset_creds = match cred_file {
        Some(path) => match read_credentials_file(&path) {
            Ok(c) => Some(c),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(2);
            }
        },
        None => None,
    };

    let confirm_phrase = std::env::var("PLANNER_CONFIRM").ok().as_deref() == Some("phrase");
    let parallel_reads = std::env::var("PLANNER_PARALLEL_READS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .map(|n| n.min(4))
        .unwrap_or(4);
    let dev_origins = std::env::var("PLANNER_DEV_ORIGINS").ok();
    let (hosts, origins) = allowed_hosts_and_origins(port, dev_origins.as_deref());

    let shared = Shared::new(demo, confirm_phrase, parallel_reads, preset_creds, hosts, origins);
    start_jar_reaper(Arc::clone(&shared));

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let _ = log::init(&PathBuf::from("logs"));
    let listener = tokio::net::TcpListener::bind(addr).await.expect("bind address");
    log::event("server_start", json!({ "port": port, "demo": demo }));
    eprintln!("Planie running at http://127.0.0.1:{port}{}", if demo { " (--demo)" } else { "" });

    let router = build_router(shared, dist_dir());
    axum::serve(listener, router).await.expect("serve");
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use tower::ServiceExt;

    fn test_router(demo: bool) -> (Arc<Shared>, Router) {
        let (hosts, origins) = allowed_hosts_and_origins(8787, None);
        let shared = Shared::new(demo, false, 2, None, hosts, origins);
        let router = build_router(Arc::clone(&shared), PathBuf::from("../frontend/dist"));
        (shared, router)
    }

    fn base_req(method: Method, uri: &str, host: &str, origin: Option<&str>, marker: bool) -> Request<Body> {
        let mut b = Request::builder().method(method).uri(uri).header(HOST, host);
        if let Some(o) = origin {
            b = b.header(ORIGIN, o);
        }
        if marker {
            b = b.header("x-planner-request", "1");
        }
        b.body(Body::empty()).unwrap()
    }

    #[tokio::test]
    async fn guard_blocks_bad_host_origin_cross_site_and_missing_marker_before_session_allocation() {
        let (shared, app) = test_router(true);

        // Bad Host
        let r = app.clone().oneshot(base_req(Method::GET, "/api/status", "evil.com:8787", None, false)).await.unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        assert_eq!(shared.jars.lock().unwrap().len(), 0);

        // Bad Origin
        let r = app.clone().oneshot(base_req(Method::GET, "/api/status", "localhost:8787", Some("https://evil.com"), false)).await.unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        assert_eq!(shared.jars.lock().unwrap().len(), 0);

        // Sec-Fetch-Site: cross-site
        let mut req = base_req(Method::GET, "/api/status", "localhost:8787", None, false);
        req.headers_mut().insert("sec-fetch-site", HeaderValue::from_static("cross-site"));
        let r = app.clone().oneshot(req).await.unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        assert_eq!(shared.jars.lock().unwrap().len(), 0);

        // POST without x-planner-request header
        let r = app.clone().oneshot(base_req(Method::POST, "/api/session", "localhost:8787", Some("http://localhost:8787"), false)).await.unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        assert_eq!(shared.jars.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn live_status_without_cookie_allocates_zero_jars_and_sets_strict_security_headers() {
        let (shared, app) = test_router(false);
        let r = app.oneshot(base_req(Method::GET, "/api/status", "127.0.0.1:8787", None, false)).await.unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert!(r.headers().get(SET_COOKIE).is_none());
        assert_eq!(r.headers().get("x-frame-options").unwrap(), "DENY");
        assert_eq!(r.headers().get("x-content-type-options").unwrap(), "nosniff");
        assert_eq!(r.headers().get(CACHE_CONTROL).unwrap(), "no-store");
        let csp = r.headers().get(CONTENT_SECURITY_POLICY).unwrap().to_str().unwrap();
        assert!(!csp.contains("unsafe-inline"), "CSP must not allow unsafe-inline: {csp}");
        assert_eq!(shared.jars.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn apply_progress_requires_session_and_isolates_jars() {
        let (_shared, app) = test_router(true);
        // Unauthenticated progress fails with 401
        let r = app.clone().oneshot(base_req(Method::GET, "/api/write/plan/progress", "localhost:8787", None, false)).await.unwrap();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);

        // Two distinct browser jars get distinct cookies
        let r1 = app.clone().oneshot(base_req(Method::GET, "/api/status", "localhost:8787", None, false)).await.unwrap();
        let c1 = r1.headers().get(SET_COOKIE).unwrap().to_str().unwrap().to_string();
        assert!(c1.contains("HttpOnly") && c1.contains("SameSite=Strict"));
        let cookie1 = c1.split(';').next().unwrap().to_string();

        let r2 = app.clone().oneshot(base_req(Method::GET, "/api/status", "localhost:8787", None, false)).await.unwrap();
        let c2 = r2.headers().get(SET_COOKIE).unwrap().to_str().unwrap().to_string();
        let cookie2 = c2.split(';').next().unwrap().to_string();
        assert_ne!(cookie1, cookie2);

        // Accounts list shows accountId for cookie1, and cookie2 cannot switch to cookie1's accountId
        let mut req_accts = base_req(Method::GET, "/api/accounts", "localhost:8787", None, false);
        req_accts.headers_mut().insert(COOKIE, HeaderValue::from_str(&cookie1).unwrap());
        let accts_res = app.clone().oneshot(req_accts).await.unwrap();
        let accts_bytes = to_bytes(accts_res.into_body(), 1024 * 64).await.unwrap();
        let accts_json: Value = serde_json::from_slice(&accts_bytes).unwrap();
        let acct1_id = accts_json["activeId"].as_str().unwrap().to_string();

        let sw = Request::builder()
            .method(Method::POST)
            .uri("/api/accounts/switch")
            .header(HOST, "localhost:8787")
            .header(ORIGIN, "http://localhost:8787")
            .header("x-planner-request", "1")
            .header("content-type", "application/json")
            .header(COOKIE, &cookie2)
            .body(Body::from(json!({ "id": acct1_id }).to_string()))
            .unwrap();
        let sw_res = app.clone().oneshot(sw).await.unwrap();
        assert_eq!(sw_res.status(), StatusCode::NOT_FOUND);

        // Explicitly specifying a non-existent or expired x-planie-account header must NEVER fall back to acct1.
        let bad_acct_req = Request::builder()
            .method(Method::GET)
            .uri("/api/classes")
            .header(HOST, "localhost:8787")
            .header(COOKIE, &cookie1)
            .header("x-planie-account", "expired_slot_id")
            .body(Body::empty())
            .unwrap();
        let bad_acct_res = app.oneshot(bad_acct_req).await.unwrap();
        assert_eq!(bad_acct_res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn global_login_cooldown_persists_across_new_requests() {
        let (shared, app) = test_router(false);
        record_failed_login(&shared, "60099999".into());
        let req = Request::builder()
            .method(Method::POST)
            .uri("/api/session")
            .header(HOST, "localhost:8787")
            .header(ORIGIN, "http://localhost:8787")
            .header("x-planner-request", "1")
            .header("content-type", "application/json")
            .body(Body::from(json!({ "user": "60099999", "pass": "wrong" }).to_string()))
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    }
}
