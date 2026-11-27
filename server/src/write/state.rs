use super::error::{err, extract_said, gate_msg, needs_login, ConfirmFail, SAID_STAGES};
use crate::demo::{DemoBackend, DemoState};
use crate::log;
use crate::seats::SeatCheck;
use crate::{ApiErr, App, Creds};
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use frankie_auth::{AuthFailure, Credentials, Http, HttpRequest, HttpResponse, Method, ReqwestHttp};
use frankie_connector_peoplesoft::login::login;
use frankie_connector_peoplesoft::testkit::{MockPs, Scenario};
use frankie_connector_peoplesoft::tools::call;
use frankie_connector_peoplesoft::{ConfirmError, PeopleSoft, PeopleSoftConfig, PsError, Session};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const CONSENT_VERSION: &str = "2026-11-26.1";
pub const IDLE_LIMIT: Duration = Duration::from_secs(20 * 60);
pub const PS_TIMEOUT: Duration = Duration::from_secs(180);
pub const PREVIEW_KEEP: Duration = Duration::from_secs(10 * 60);
pub const PRESTAGE_TTL: Duration = Duration::from_secs(10 * 60);

pub type SaidBuf = Arc<Mutex<Vec<(String, String)>>>;

pub fn push_said(buf: &SaidBuf, stage: &str, sentences: &[String]) {
    if let Ok(mut v) = buf.lock() {
        for s in sentences {
            if !v.iter().any(|(st, x)| st == stage && x == s) {
                v.push((stage.to_string(), s.clone()));
            }
        }
        let extra = v.len().saturating_sub(60);
        v.drain(..extra);
    }
}

pub fn take_said(buf: &SaidBuf, stages: &[&str]) -> Vec<String> {
    let Ok(mut v) = buf.lock() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    v.retain(|(st, x)| {
        if stages.contains(&st.as_str()) {
            if !out.contains(x) {
                out.push(x.clone());
            }
            false
        } else {
            true
        }
    });
    out
}

/// Server-owned consent items acknowledged before real PeopleSoft writes execute.
pub fn statements() -> Vec<(&'static str, &'static str)> {
    vec![
        ("real", "Changes only happen when you press Confirm & Enroll on your real UDST PeopleSoft account."),
        ("undo", "Dropping or swapping a class may release your seat immediately; deadlines and fees depend on UDST."),
        ("mine", "You review and confirm the final courses, sections, and credits shown before enrolling."),
    ]
}

/// One live (or demo) PeopleSoft session.
pub trait Backend: Send {
    fn tool(&mut self, name: &str, args: &Value) -> Result<Value, String>;
    fn confirm(&mut self, token: &str, phrase: &str) -> Result<Value, ConfirmFail>;
    fn arm(&mut self);
    fn disarm(&mut self);
}

pub struct Live<H: Http>(pub PeopleSoft<H>);

impl<H: Http + Send> Backend for Live<H> {
    fn tool(&mut self, name: &str, args: &Value) -> Result<Value, String> {
        let r = call(&mut self.0, name, args);
        match (&r.data, &r.error) {
            (Some(d), _) => Ok(d.clone()),
            (_, Some(e)) => Err(e.message.clone()),
            _ => Err("PeopleSoft returned nothing.".into()),
        }
    }
    fn confirm(&mut self, token: &str, phrase: &str) -> Result<Value, ConfirmFail> {
        match self.0.confirm(token, phrase) {
            Ok(out) => Ok(json!({
                "kind": out.plan.kind.word(),
                "classNumbers": out.plan.class_numbers,
                "messages": out.messages,
                "detail": out.detail,
            })),
            Err(ConfirmError::Gate(g)) => Err(ConfirmFail { message: gate_msg(g), may_have_sent: false }),
            Err(ConfirmError::Ps(e)) => {
                let sent = !matches!(e, PsError::WriteRefused(_) | PsError::InvalidInput(_) | PsError::NotFound(_) | PsError::Origin(_));
                Err(ConfirmFail { message: e.to_contract("confirm").message, may_have_sent: sent })
            }
        }
    }
    fn arm(&mut self) {
        self.0.arm_writes();
    }
    fn disarm(&mut self) {
        self.0.disarm_writes();
    }
}

pub struct Timed<H: Http> {
    inner: H,
    last_end: Option<Instant>,
    said_buf: Option<SaidBuf>,
}

impl<H: Http> Timed<H> {
    pub fn new(inner: H) -> Self {
        Self { inner, last_end: None, said_buf: None }
    }
    pub fn with_said(inner: H, said_buf: SaidBuf) -> Self {
        Self { inner, last_end: None, said_buf: Some(said_buf) }
    }
}

impl<H: Http> Http for Timed<H> {
    fn send(&mut self, r: HttpRequest) -> Result<HttpResponse, AuthFailure> {
        let stage = r.stage;
        let method = if matches!(r.method, Method::Get) { "GET" } else { "POST" };
        let path = r.url.path().to_string();
        let gap = self.last_end.map(|t| (t.elapsed().as_secs_f64() * 1000.0).round() as u64);
        if method == "POST" && path.contains("/EMPLOYEE/SA/c/") {
            if let Some(b) = &r.body {
                let pairs: Vec<(String, String)> = b
                    .split('&')
                    .map(|kv| {
                        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
                        (form_decode(k), form_decode(v))
                    })
                    .collect();
                let shown: Vec<String> = pairs
                    .iter()
                    .filter(|(k, _)| {
                        matches!(k.as_str(), "ICAction" | "ICStateNum" | "ICAJAX" | "ICType" | "ICXPos" | "ICYPos" | "ICChanged" | "ICResubmit" | "PTAI" | "PTAI_POSTPROC")
                            || k.starts_with("P_SELECT")
                    })
                    .map(|(k, v)| format!("{k}={}", log::trunc(v, 60)))
                    .collect();
                let mut names: Vec<&str> = pairs.iter().map(|(k, _)| k.as_str()).collect();
                names.sort_unstable();
                log::event("ps_post_shape", json!({ "stage": stage, "path": path, "fields": pairs.len(), "shown": shown, "names": names }));
            }
        }
        let started = Instant::now();
        let res = self.inner.send(r);
        let ms = log::ms_since(started);
        self.last_end = Some(Instant::now());
        if let Ok(x) = &res {
            if SAID_STAGES.contains(&stage) {
                let said = extract_said(stage, &x.body);
                log::event("ps_said", json!({ "stage": stage, "method": method, "status": x.status, "said": said }));
                if let Some(buf) = &self.said_buf {
                    push_said(buf, stage, &said);
                } else {
                    log::note_said(stage, &said);
                }
            }
        }
        if stage == "drop_guide" {
            if let Ok(x) = &res {
                log::event(
                    "ps_guide_shape",
                    json!({
                        "status": x.status,
                        "bytes": x.body.len(),
                        "has_list_id": x.body.contains("PTAI_LIST_ID=SR_DROP_FL"),
                        "has_init": x.body.contains("ptgpPage.init("),
                        "has_select_step": x.body.contains("ptgpPage.selectStep("),
                        "refused": x.body.contains("cannot be created"),
                    }),
                );
            }
        }
        match &res {
            Ok(x) => log::event("ps_req", json!({ "stage": stage, "method": method, "path": path, "status": x.status, "ms": ms, "bytes": x.body.len(), "gap_ms": gap })),
            Err(e) => log::event("ps_req_err", json!({ "stage": stage, "method": method, "path": path, "ms": ms, "error": e.safe_code(), "gap_ms": gap })),
        }
        res
    }
}

pub fn form_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() && b[i + 1].is_ascii_hexdigit() && b[i + 2].is_ascii_hexdigit() => {
                out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap_or(b'?'));
                i += 2;
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn step<T, E: std::fmt::Display>(op: &str, name: &str, f: impl FnOnce() -> Result<T, E>) -> Result<T, E> {
    let t = Instant::now();
    let r = f();
    match &r {
        Ok(_) => log::event("step", json!({ "op": op, "step": name, "ms": log::ms_since(t), "ok": true })),
        Err(e) => log::event("step", json!({ "op": op, "step": name, "ms": log::ms_since(t), "ok": false, "error": log::trunc(&e.to_string(), 300) })),
    }
    r
}

pub fn ps_config() -> PeopleSoftConfig {
    let mut cfg = PeopleSoftConfig::udst();
    if let Some(ms) = gap_from(std::env::var("PLANNER_MIN_GAP_MS").ok().as_deref()) {
        cfg.min_interval_ms = ms;
    }
    cfg
}

pub fn gap_from(v: Option<&str>) -> Option<u64> {
    v.and_then(|x| x.trim().parse::<u64>().ok()).map(|ms| ms.clamp(200, 5000))
}

#[cfg(test)]
pub(crate) static TEST_BACKENDS: std::sync::Mutex<Option<Box<dyn Fn() -> Box<dyn Backend> + Send + Sync>>> = std::sync::Mutex::new(None);

pub fn build_live_with_said(creds: &Creds, said: Option<SaidBuf>) -> Result<Box<dyn Backend>, String> {
    #[cfg(test)]
    {
        if let Some(make) = TEST_BACKENDS.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            return Ok(make());
        }
    }
    let c = Credentials::new(creds.user.trim(), &creds.pass).map_err(|_| "Credentials not accepted.".to_string())?;
    let http = ReqwestHttp::new(PS_TIMEOUT).map_err(|f| format!("Could not start the HTTP client: {}", f.safe_code()))?;
    let timed = match said {
        Some(buf) => Timed::with_said(http, buf),
        None => Timed::new(http),
    };
    let mut ps = PeopleSoft::new(Session::new(timed, ps_config()));
    ps.set_preview_ttl(PREVIEW_KEEP);
    if let Err(e) = login(ps.session_mut(), &c) {
        return Err(match e {
            PsError::Auth(f) => format!("Sign-in stopped: {} (no retry was made).", f.safe_code()),
            other => format!("Sign-in stopped: {}", other.to_contract("sign_in").message),
        });
    }
    Ok(Box::new(Live(ps)))
}

#[allow(dead_code)]
pub fn build_live(creds: &Creds) -> Result<Box<dyn Backend>, String> {
    build_live_with_said(creds, None)
}

pub fn build_demo_with_state(state: Arc<Mutex<DemoState>>) -> Box<dyn Backend> {
    Box::new(DemoBackend::new(state))
}

#[allow(dead_code)]
pub fn build_demo() -> Box<dyn Backend> {
    let mut cfg = PeopleSoftConfig::udst();
    cfg.min_interval_ms = 0;
    Box::new(Live(PeopleSoft::new(Session::new(Timed::new(MockPs::signed_in(Scenario::default())), cfg))))
}

#[derive(Default)]
pub struct Snap {
    pub armed: bool,
    pub last_used: Option<Instant>,
    pub busy: Option<(String, Instant)>,
    pub queued: u32,
}

#[derive(Default)]
pub struct WriteState {
    pub backend: Option<Box<dyn Backend>>,
    pub armed: bool,
    pub last_used: Option<Instant>,
    pub phrases: HashMap<String, (String, Instant)>,
    pub list_cache: Option<(Instant, Value)>,
    pub checks: HashMap<String, Vec<SeatCheck>>,
    pub swap_checks: Option<Vec<SeatCheck>>,
    pub kinds: HashMap<String, String>,
    pub drop_ctx: HashMap<String, (u32, String, Option<String>)>,
    pub list_page_valid: bool,
    pub defer_verify: bool,
    pub pending_verify: Option<Vec<SeatCheck>>,
    pub parallel_off: bool,
    pub relogins: u32,
    pub said_buf: Option<SaidBuf>,
    pub demo_state: Option<Arc<Mutex<DemoState>>>,
}

impl WriteState {
    pub fn disarm(&mut self) {
        if let Some(b) = self.backend.as_mut() {
            b.disarm();
        }
        self.armed = false;
        self.last_used = None;
        self.phrases.clear();
        self.checks.clear();
        self.kinds.clear();
        self.drop_ctx.clear();
        self.swap_checks = None;
        self.list_page_valid = false;
    }

    pub fn drop_session(&mut self) {
        self.disarm();
        self.backend = None;
        self.list_cache = None;
        self.parallel_off = false;
    }

    pub fn expire_idle(&mut self) {
        if self.armed && self.last_used.is_some_and(|t| t.elapsed() > IDLE_LIMIT) {
            self.disarm();
        }
        self.phrases.retain(|_, (_, t)| t.elapsed() < PREVIEW_KEEP);
        let live: Vec<String> = self.phrases.keys().cloned().collect();
        self.checks.retain(|k, _| live.contains(k));
        self.kinds.retain(|k, _| live.contains(k));
        self.drop_ctx.retain(|k, _| live.contains(k));
    }

    pub fn touch(&mut self) {
        self.last_used = Some(Instant::now());
    }

    pub fn take_said(&self, stages: &[&str]) -> Vec<String> {
        match &self.said_buf {
            Some(buf) => take_said(buf, stages),
            None => log::take_said(stages),
        }
    }
}

pub fn drop_session_now(app: &App) {
    app.stop.store(true, Ordering::SeqCst);
    if let Ok(mut s) = app.snap.lock() {
        s.armed = false;
    }
    if let Ok(mut p) = app.prestaged.lock() {
        p.clear();
    }
    log::event("signout_requested", json!({}));
    let a = app.clone();
    tokio::spawn(async move {
        let _ = with_state(&a, "drop_session", |st| {
            st.drop_session();
            Ok(())
        })
        .await;
    });
}

pub fn disarm_now(app: &App) {
    app.stop.store(true, Ordering::SeqCst);
    if let Ok(mut s) = app.snap.lock() {
        s.armed = false;
    }
    if let Ok(mut p) = app.prestaged.lock() {
        p.clear();
    }
    log::event("disarm_requested", json!({}));
    let a = app.clone();
    tokio::spawn(async move {
        let _ = with_state(&a, "disarm", |st| {
            st.disarm();
            Ok(())
        })
        .await;
    });
}

pub fn start_idle_watch(app: App) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        loop {
            tick.tick().await;
            if app.shutdown.load(Ordering::SeqCst) {
                break;
            }
            if let Ok(mut g) = app.write.try_lock() {
                let was = g.armed;
                g.expire_idle();
                if was && !g.armed {
                    log::event("idle_disarm", json!({}));
                    if let Ok(mut s) = app.snap.lock() {
                        s.armed = false;
                    }
                }
            }
        }
    });
}

pub async fn with_state<T: Send + 'static>(
    app: &App,
    op: impl Into<String>,
    f: impl FnOnce(&mut WriteState) -> Result<T, ApiErr> + Send + 'static,
) -> Result<T, ApiErr> {
    let op: String = op.into();
    let queued_at = Instant::now();
    if let Ok(mut s) = app.snap.lock() {
        s.queued += 1;
    }
    let _run = app.run_lock.lock().await;
    let wait_ms = log::ms_since(queued_at);
    if let Ok(mut s) = app.snap.lock() {
        s.queued = s.queued.saturating_sub(1);
        s.busy = Some((op.clone(), Instant::now()));
    }
    let started = Instant::now();
    let st = app.write.clone();
    let app2 = app.clone();
    let stop = app.stop.clone();
    let said = app.said.clone();
    let demo_state = app.demo_state.clone();
    let op_for_log = op.clone();
    let handle = tokio::task::spawn_blocking(move || {
        let mut g = st.lock().map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "internal error"))?;
        if g.said_buf.is_none() {
            g.said_buf = Some(said);
        }
        if g.demo_state.is_none() {
            g.demo_state = Some(demo_state);
        }
        g.expire_idle();
        let mut r = f(&mut g);
        if let Err(e) = &r {
            if needs_login(&e.1) {
                g.drop_session();
                r = Err(err(StatusCode::CONFLICT, "PeopleSoft signed you out after being idle, so writes were switched off. Sign in again."));
            }
        }
        let run_ms = log::ms_since(started);
        if let Ok(mut s) = app2.snap.lock() {
            s.armed = g.armed && !stop.load(Ordering::SeqCst);
            s.last_used = g.last_used;
            s.busy = None;
        }
        match &r {
            Ok(_) => log::event("op_end", json!({ "op": op_for_log, "wait_ms": wait_ms, "ms": run_ms, "ok": true })),
            Err(e) => log::event("op_end", json!({ "op": op_for_log, "wait_ms": wait_ms, "ms": run_ms, "ok": false, "status": e.0.as_u16(), "error": log::trunc(&e.1, 300) })),
        }
        Ok::<_, ApiErr>(r)
    });
    let joined = handle.await;
    match joined {
        Ok(Ok(r)) => r,
        _ => {
            if let Ok(mut s) = app.snap.lock() {
                s.busy = None;
            }
            log::event("op_end", json!({ "op": op, "wait_ms": wait_ms, "ms": log::ms_since(started), "ok": false, "error": "internal error" }));
            Err(err(StatusCode::INTERNAL_SERVER_ERROR, "internal error"))
        }
    }
}

pub fn armed_backend(st: &mut WriteState) -> Result<&mut Box<dyn Backend>, ApiErr> {
    if !st.armed {
        return Err(err(StatusCode::CONFLICT, "Writes are off. Enable them first."));
    }
    st.touch();
    st.list_page_valid = false;
    st.backend.as_mut().ok_or_else(|| err(StatusCode::CONFLICT, "Writes are off. Enable them first."))
}

pub fn live<'a>(st: &'a mut WriteState, creds: &Option<Creds>, demo: bool) -> Result<&'a mut Box<dyn Backend>, ApiErr> {
    if st.backend.is_none() {
        st.list_cache = None;
        st.backend = Some(if demo {
            let ds = st.demo_state.clone().unwrap_or_else(|| Arc::new(Mutex::new(DemoState::default())));
            build_demo_with_state(ds)
        } else {
            let c = creds.as_ref().ok_or_else(|| err(StatusCode::UNAUTHORIZED, "Sign in first."))?;
            build_live_with_said(c, st.said_buf.clone()).map_err(|m| err(StatusCode::BAD_GATEWAY, m))?
        });
    }
    st.backend.as_mut().ok_or_else(|| err(StatusCode::INTERNAL_SERVER_ERROR, "no session"))
}

pub fn read_tool(st: &mut WriteState, creds: &Option<Creds>, demo: bool, tool: &str, args: &Value) -> Result<Value, ApiErr> {
    if tool != "plan_class_list" {
        st.list_page_valid = false;
    }
    let first = live(st, creds, demo)?.tool(tool, args);
    match first {
        Ok(v) => Ok(v),
        Err(m) if needs_login(&m) && !st.armed => {
            st.relogins += 1;
            st.backend = None;
            let again = live(st, creds, demo)?.tool(tool, args);
            again.map_err(|m| err(StatusCode::UNPROCESSABLE_ENTITY, m))
        }
        Err(m) => Err(err(StatusCode::UNPROCESSABLE_ENTITY, m)),
    }
}

pub async fn sign_in(app: &App, creds: Creds) -> Result<(), ApiErr> {
    let said = app.said.clone();
    with_state(app, "sign_in", move |st| {
        st.drop_session();
        st.said_buf = Some(said.clone());
        st.backend = Some(build_live_with_said(&creds, Some(said)).map_err(|m| err(StatusCode::UNAUTHORIZED, m))?);
        Ok(())
    })
    .await
}

pub fn status_json(app: &App) -> Value {
    let (armed, left, busy, queued) = {
        let s = app.snap.lock().unwrap_or_else(|e| e.into_inner());
        let left = s.last_used.filter(|_| s.armed).map(|t| IDLE_LIMIT.saturating_sub(t.elapsed()).as_secs());
        let armed = s.armed && left.is_some_and(|l| l > 0);
        let busy = s.busy.as_ref().map(|(op, t)| json!({ "op": op, "seconds": t.elapsed().as_secs() }));
        (armed, left, busy, s.queued)
    };
    json!({
        "demo": app.demo,
        "armed": armed,
        "idleSecondsLeft": left,
        "busy": busy,
        "queued": queued,
        "confirmMode": if app.confirm_phrase { "phrase" } else { "click" },
        "consent": {
            "version": CONSENT_VERSION,
            "statements": statements().iter().map(|(id, text)| json!({"id": id, "text": text})).collect::<Vec<_>>(),
        },
    })
}

pub async fn status(State(app): State<App>) -> Json<Value> {
    Json(status_json(&app))
}

#[derive(Deserialize)]
pub struct ArmReq {
    pub acks: Vec<String>,
    pub version: String,
}

pub async fn arm(State(app): State<App>, Json(req): Json<ArmReq>) -> Result<Json<Value>, ApiErr> {
    let need: BTreeSet<&str> = statements().iter().map(|s| s.0).collect();
    let got: BTreeSet<&str> = req.acks.iter().map(String::as_str).collect();
    if req.version != CONSENT_VERSION || need != got {
        return Err(err(StatusCode::BAD_REQUEST, "Accept the confirmation notice before applying changes."));
    }
    let demo = app.demo;
    let creds = if demo { None } else { Some(crate::current_creds(&app)?) };
    let stop = app.stop.clone();
    with_state(&app, "arm", move |st| {
        stop.store(false, Ordering::SeqCst);
        live(st, &creds, demo)?.arm();
        st.armed = true;
        st.touch();
        Ok(())
    })
    .await?;
    Ok(Json(status_json(&app)))
}

pub async fn disarm(State(app): State<App>) -> Json<Value> {
    disarm_now(&app);
    Json(status_json(&app))
}
