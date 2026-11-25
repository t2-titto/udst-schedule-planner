//! The write layer: plan changes, enrol, drop and swap on the real PeopleSoft account, behind one acknowledgement of the statements below.
//!
//! How it is kept safe, in order:
//! 1. Nothing here runs until the page has acknowledged every statement in [`statements`] for the current `CONSENT_VERSION` (the server
//!    owns the text, so the page cannot water it down; the sign-in screen shows it and asks for one acknowledgement) and the server
//!    has opened its own live session and armed the connector.
//! 2. Every write is a preview first. The connector's gate hands back a one-time token bound to that exact plan; the
//!    server only ever confirms a token it issued itself, with the phrase the connector itself required. The only writes that can be
//!    reached are a plan change (`/write/preview` + `/write/confirm`) and `/write/plan/apply` (drops, swaps, plans, enrolments); the
//!    browser names class numbers and a course in them, and the server re-reads PeopleSoft and checks them before anything is sent.
//!    The old one-request endpoints (bulk and parallel drop and enrol, swap apply, the lab) were removed in the red-team fixes.
//! 3. Before anything is staged, the class numbers PeopleSoft returned are compared with the ones the person chose.
//!    If they differ (the list moved since the fetch) nothing is sent. The seats are read fresh before every write step
//!    and again at confirm time (see `seats.rs`); a full, closed or vanished class stops the write before it is sent.
//! 4. Writes switch off after 20 idle minutes, on sign-out and on "Disable writes". Sign-out also drops the session.
//!
//! The same live session serves reads. It is created at sign-in and kept in memory, so a fetch does not pay for a
//! fresh sign-in, and `fetch_sections` warms it with one class-list visit, then reads each course directly by its key.
//!
//! Confirmation is one click by the person's own choice (`PLANNER_CONFIRM=phrase` brings back the typed phrase).
//! The connector's own CLI additionally demands a real terminal; a web page cannot offer that guarantee, so
//! anything on this machine that can reach the local server with the right headers can confirm. Keep that in mind.

use crate::log;
use crate::seats::{self, safe_key, Report, SeatCheck};
use crate::{ApiErr, App, Creds};
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use frankie_auth::{AuthFailure, Credentials, Http, HttpRequest, HttpResponse, Method, ReqwestHttp};
use frankie_connector_peoplesoft::login::login;
use frankie_connector_peoplesoft::testkit::{MockPs, Scenario};
use frankie_connector_peoplesoft::tools::call;
use frankie_connector_peoplesoft::{ConfirmError, GateError, PeopleSoft, PeopleSoftConfig, PsError, Session};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub const CONSENT_VERSION: &str = "2026-11-21.1";
const IDLE_LIMIT: Duration = Duration::from_secs(20 * 60);
/// How long one request to PeopleSoft may take before this server gives up on it. It was 30 s, and PeopleSoft's enrol
/// submit takes longer than that: the client hung up while PeopleSoft was still enrolling, the app reported a failure,
/// and the student was in fact enrolled. The time the submit really takes is now in the log (`ps_req`, stage
/// `enrol_submit`), so this can be tuned from evidence.
const PS_TIMEOUT: Duration = Duration::from_secs(180);
const PREVIEW_KEEP: Duration = Duration::from_secs(10 * 60);

/// What the person must accept. `(id, text)`. Every statement is required.
pub fn statements() -> Vec<(&'static str, &'static str)> {
    vec![
        ("real", "These actions change your real UDST PeopleSoft account: your course planning and your class enrolment records. They are not a simulation."),
        ("untested", "The write steps were built from recorded sessions and tested against invented pages. Plan changes, drops, enrolments and swaps have since been run on the live site, but not every case: a step can fail, time out, or behave differently from what this screen describes, and the first enrolment submit once timed out and had to be checked by hand."),
        ("enrol", "Enrolling happens in stages: your chosen classes are added to your Plan & Enrol list (nothing is enrolled yet), then the list is submitted and enrols you for real. Fees may apply, and the amount is only shown after the submit."),
        ("undo", "Enrolment may not be reversible. Whether you can undo it depends on drop deadlines and seat availability, which this app cannot read in advance, and a freed seat can be taken by someone else at once. A course planned for a term can also be impossible to move out of it once you have been enrolled in it."),
        ("drop", "Dropping removes you from a class. Deadlines, penalties and fees are not known to this app, and you may not get the seat back."),
        ("swap", "Confirming a swap gives up your current seat and tries to take the new one. If the new enrolment fails, your old seat may already be gone. The server refuses swaps that change only the lab."),
        ("oneclick", "One click sends it. Pressing Apply (or Save to PeopleSoft) reviews nothing further: the steps are sent one after another with no typed phrase and no second prompt."),
        ("mine", "You are responsible for the result. You will check the courses, terms and times on the review screen, and you are not treating this app's scores, ranking or warnings as advice."),
        ("expiry", "Writes switch off after 20 minutes of no use, when you sign out, when you press Disable writes, and when an Apply or a save finishes."),
    ]
}

// ------------------------------------------------------------------ backend

/// Why a confirm failed, and whether anything may have reached PeopleSoft before it did. A refusal by the gate or a bad
/// input means nothing was sent. A timeout, a dropped connection or an unexpected page means the write MAY have gone
/// through, and then the only honest answer is "unknown": look at what PeopleSoft itself says.
#[derive(Debug, Clone)]
pub struct ConfirmFail {
    pub message: String,
    pub may_have_sent: bool,
}

impl std::fmt::Display for ConfirmFail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl From<String> for ConfirmFail {
    fn from(message: String) -> Self {
        Self { message, may_have_sent: false }
    }
}

impl ConfirmFail {
    /// The message, with the warning that matters when the write may have gone through.
    pub fn text(&self) -> String {
        if self.may_have_sent {
            format!("{} This step may or may not have gone through on PeopleSoft's side. Check PeopleSoft before doing anything again.", self.message)
        } else {
            self.message.clone()
        }
    }
}

/// One live (or demo) PeopleSoft session. Object-safe so real and mock sessions share the server code.
pub trait Backend: Send {
    /// A read or preview. The connector itself refuses a confirm here.
    fn tool(&mut self, name: &str, args: &Value) -> Result<Value, String>;
    fn confirm(&mut self, token: &str, phrase: &str) -> Result<Value, ConfirmFail>;
    fn arm(&mut self);
    fn disarm(&mut self);
}

pub struct Live<H: Http>(pub PeopleSoft<H>);

fn gate_msg(g: GateError) -> String {
    match g {
        GateError::Disarmed => "Writes are off.".into(),
        GateError::UnknownToken => "That preview is no longer valid. Preview again.".into(),
        GateError::Expired => "The preview expired. Preview again.".into(),
        GateError::PhraseMismatch => "The phrase did not match.".into(),
        GateError::TooManyAttempts => "Too many wrong phrases. Preview again.".into(),
    }
}

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
                // Refused or invalid BEFORE anything was sent: certain. Anything else (timeout, dropped connection,
                // an unexpected page) may have happened after the request left.
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

/// The stages whose pages can carry PeopleSoft's own answer to a write ("Enrollment not successful", "Unable to drop ...").
/// The connector only looks at one of the pages in a chain and throws the rest away, so the sentences are read here, from
/// every page of these stages, with the connector's own extractor (result-like sentences only, never raw HTML).
const SAID_STAGES: [&str; 7] = ["drop_click", "drop_yes", "drop_result", "drop_review", "enrol_submit", "stage_option", "swap_commit"];

/// Pulls PeopleSoft's own outcome/error sentences from a write-stage response without logging raw HTML or personal data.
/// Starts with the connector's `outcome_hints`, then falls back to message/error fields and status-icon error text when
/// `outcome_hints` missed a refusal (e.g. "corequisite", "waitlist", "limit", or a message inside a `<div>` or `<p>`).
fn extract_said(stage: &str, body: &str) -> Vec<String> {
    let fs = frankie_connector_peoplesoft::pia::fields(body);
    let mut out: Vec<String> = frankie_connector_peoplesoft::pia::outcome_hints(&fs)
        .into_iter()
        .map(|h| log::trunc(&h, 200))
        .collect();
    let push_unique = |dst: &mut Vec<String>, s: &str| {
        let t = log::trunc(s, 200);
        if !t.is_empty() && !dst.contains(&t) {
            dst.push(t);
        }
    };
    const EXTRA_WORDS: [&str; 13] = [
        "corequisite", "co-requisite", "requisite", "waitlist", "wait list",
        "closed", "full", "limit", "maximum", "exceed", "credit", "reserved", "permission",
    ];
    let has_error_icon = body.contains("PS_CS_STATUS_ERROR_ICN");
    for f in &fs {
        if out.len() >= 6 { break; }
        if f.text.is_empty() || f.text.len() >= 400 { continue; }
        let name_upper = f.name.to_ascii_uppercase();
        if name_upper.contains("AMOUNT") || name_upper.contains("PAYMENT") || name_upper.contains("DESCR50") { continue; }
        let low = f.text.to_ascii_lowercase();
        let is_msg_field = name_upper.contains("MSG") || name_upper.contains("ERR") || name_upper.contains("DESCRLONG") || name_upper.contains("ALERT");
        if EXTRA_WORDS.iter().any(|w| low.contains(w)) || (is_msg_field && (has_error_icon || stage == "enrol_submit")) {
            push_unique(&mut out, &f.text);
        }
    }
    if out.is_empty() && has_error_icon {
        // PeopleSoft Fluid sometimes places the error text in a <div>/<p>/<img> alt/title next to PS_CS_STATUS_ERROR_ICN
        // rather than a <span id=...> matched by `pia::fields`. Extract short non-tag snippets around the error icon.
        let mut rest = body;
        while let Some(pos) = rest.find("PS_CS_STATUS_ERROR_ICN") {
            let after_icon = rest[pos..].find('>').map(|i| pos + i + 1).unwrap_or(pos + "PS_CS_STATUS_ERROR_ICN".len());
            let chunk = &rest[after_icon..rest.len().min(after_icon + 900)];
            let mut plain = String::with_capacity(200);
            let mut in_tag = false;
            for ch in chunk.chars() {
                match ch {
                    '<' => { in_tag = true; plain.push(' '); }
                    '>' => in_tag = false,
                    _ if !in_tag => plain.push(ch),
                    _ => {}
                }
            }
            let cleaned = plain.replace("&nbsp;", " ").replace("&amp;", "&").replace("&quot;", "\"").replace("&#39;", "'");
            let trimmed: String = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
            if !trimmed.is_empty() && trimmed.len() >= 8 {
                push_unique(&mut out, &trimmed);
                if out.len() >= 6 { break; }
            }
            rest = &rest[pos + "PS_CS_STATUS_ERROR_ICN".len()..];
        }
    }
    out.truncate(6);
    out
}

/// Times every request the connector sends to PeopleSoft and writes it to the log: the connector's fixed step label,
/// method, path (no query), status, milliseconds, bytes, and `gap_ms`, the idle time since the previous request ended
/// (the connector's 1 s pacing plus parsing). No body, header or cookie is ever logged.
pub struct Timed<H: Http> {
    inner: H,
    last_end: Option<Instant>,
}

impl<H: Http> Timed<H> {
    pub fn new(inner: H) -> Self {
        Self { inner, last_end: None }
    }
}

impl<H: Http> Http for Timed<H> {
    fn send(&mut self, r: HttpRequest) -> Result<HttpResponse, AuthFailure> {
        let stage = r.stage;
        let method = if matches!(r.method, Method::Get) { "GET" } else { "POST" };
        let path = r.url.path().to_string();
        let gap = self.last_end.map(|t| (t.elapsed().as_secs_f64() * 1000.0).round() as u64);
        // The shape of a PeopleSoft page POST, so it can be compared with what a browser sends: which action, and the NAMES of
        // every field. Values are logged only for a short allowlist of non-secret control fields. Never the sign-in (it is not
        // on a /SA/c/ page path), never ICSID, cookies or any other value.
        if method == "POST" && path.contains("/EMPLOYEE/SA/c/") {
            if let Some(b) = &r.body {
                let pairs: Vec<(String, String)> = b.split('&').map(|kv| { let (k, v) = kv.split_once('=').unwrap_or((kv, "")); (form_decode(k), form_decode(v)) }).collect();
                let shown: Vec<String> = pairs.iter().filter(|(k, _)| matches!(k.as_str(), "ICAction" | "ICStateNum" | "ICAJAX" | "ICType" | "ICXPos" | "ICYPos" | "ICChanged" | "ICResubmit" | "PTAI" | "PTAI_POSTPROC") || k.starts_with("P_SELECT")).map(|(k, v)| format!("{k}={}", log::trunc(v, 60))).collect();
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
                log::note_said(stage, &said);
            }
        }
        // The drop guide's grouplet answers: structural facts only (no page text), to see why an answer carried no guide ids.
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

/// Minimal url-encoded form decoding, for logging field names only.
fn form_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() + 0 && b[i + 1].is_ascii_hexdigit() && b[i + 2].is_ascii_hexdigit() => {
                out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap_or(b'?'));
                i += 2;
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Time one step of a long operation and log it, with the failure text if it failed.
fn step<T, E: std::fmt::Display>(op: &str, name: &str, f: impl FnOnce() -> Result<T, E>) -> Result<T, E> {
    let t = Instant::now();
    let r = f();
    match &r {
        Ok(_) => log::event("step", json!({ "op": op, "step": name, "ms": log::ms_since(t), "ok": true })),
        Err(e) => log::event("step", json!({ "op": op, "step": name, "ms": log::ms_since(t), "ok": false, "error": log::trunc(&e.to_string(), 300) })),
    }
    r
}

/// The connector's config, with the gap between two requests set from `PLANNER_MIN_GAP_MS` (default 1000, the connector's
/// own value; never below 200 or above 5000). The gap only matters when a request answers faster than the gap; it costs
/// about 0.6 s per quick request in a course's chain.
fn ps_config() -> PeopleSoftConfig {
    let mut cfg = PeopleSoftConfig::udst();
    if let Some(ms) = gap_from(std::env::var("PLANNER_MIN_GAP_MS").ok().as_deref()) {
        cfg.min_interval_ms = ms;
    }
    cfg
}

fn gap_from(v: Option<&str>) -> Option<u64> {
    v.and_then(|x| x.trim().parse::<u64>().ok()).map(|ms| ms.clamp(200, 5000))
}

/// Tests only: every new session is made by this instead of signing in to PeopleSoft, so the apply run (which opens its own
/// sessions for drops and enrolments) can be driven against the connector's pretend PeopleSoft.
#[cfg(test)]
pub(crate) static TEST_BACKENDS: std::sync::Mutex<Option<Box<dyn Fn() -> Box<dyn Backend> + Send + Sync>>> = std::sync::Mutex::new(None);

fn build_live(creds: &Creds) -> Result<Box<dyn Backend>, String> {
    #[cfg(test)]
    {
        if let Some(make) = TEST_BACKENDS.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            return Ok(make());
        }
    }
    let c = Credentials::new(creds.user.trim(), &creds.pass).map_err(|_| "Credentials not accepted.".to_string())?;
    let http = ReqwestHttp::new(PS_TIMEOUT).map_err(|f| format!("Could not start the HTTP client: {}", f.safe_code()))?;
    let mut ps = PeopleSoft::new(Session::new(Timed::new(http), ps_config()));
    // Apply stages an enrolment first and submits it after the drops, plans and swaps. The connector's own 120 s would expire the staged submit
    // mid-run (the committed live run was 97 s apart with one drop and no swap), after the drops had already happened. The server's own limits for
    // a staged submit are PREVIEW_KEEP and PRESTAGE_TTL (10 minutes): the connector is told the same, so the real limit is the one that is written.
    ps.set_preview_ttl(PREVIEW_KEEP);
    if let Err(e) = login(ps.session_mut(), &c) {
        return Err(match e {
            PsError::Auth(f) => format!("Sign-in stopped: {} (no retry was made).", f.safe_code()),
            other => format!("Sign-in stopped: {}", other.to_contract("sign_in").message),
        });
    }
    Ok(Box::new(Live(ps)))
}

/// The connector's own invented PeopleSoft. No network, no account.
pub fn build_demo() -> Box<dyn Backend> {
    let mut cfg = PeopleSoftConfig::udst();
    cfg.min_interval_ms = 0;
    Box::new(Live(PeopleSoft::new(Session::new(Timed::new(MockPs::signed_in(Scenario::default())), cfg))))
}

// ------------------------------------------------------------------ state

/// What the status endpoints report. It is updated at the end of every operation and is only ever locked for an
/// instant, so asking for the status never waits for PeopleSoft (the cause of the freeze: a status check used to wait
/// for the same lock the running operation held, and the page's polling piled up until every server thread was stuck).
#[derive(Default)]
pub struct Snap {
    pub armed: bool,
    pub last_used: Option<Instant>,
    /// The operation running now and when it started.
    pub busy: Option<(String, Instant)>,
    /// Operations waiting for their turn.
    pub queued: u32,
}

#[derive(Default)]
pub struct WriteState {
    pub backend: Option<Box<dyn Backend>>,
    pub armed: bool,
    pub last_used: Option<Instant>,
    /// token -> (phrase the connector requires, when issued). Only ever filled from the connector's own previews.
    pub phrases: HashMap<String, (String, Instant)>,
    /// The last class list and when it was read. Reading it warms PeopleSoft's session for direct course fetches.
    pub list_cache: Option<(Instant, Value)>,
    /// token -> the seat checks to run again, fresh, right before that preview is confirmed.
    pub checks: HashMap<String, Vec<SeatCheck>>,
    /// The seat checks of the swap that was staged, for the swap's final confirm.
    pub swap_checks: Option<Vec<SeatCheck>>,
    /// token -> the connector's kind of write ("submit_enrol", "drop", ...), so a failure can be handled by kind.
    pub kinds: HashMap<String, String>,
    /// token -> (term row, class number) of a previewed drop, so the drop can be proven from PeopleSoft's enrolled list.
    pub drop_ctx: HashMap<String, (u32, String, Option<String>)>,
    /// True only while the last thing done in PeopleSoft's session was reading the Plan & Enrol class list, so that page
    /// is still the one in front of the session and a course can be opened from it without reloading it (a reload is
    /// about 15 s). Any other navigation or write clears it.
    pub list_page_valid: bool,
    /// Fire path: skip the class-list read after an enrol submit (it takes 8 to 14 s) and hand the checks back in `pending_verify`,
    /// so the confirmation page's own answer can be reported at once and the list read done afterwards.
    pub defer_verify: bool,
    pub pending_verify: Option<Vec<SeatCheck>>,
    /// Set when parallel read sessions misbehaved (a helper failed, or opening them signed the main session out). The
    /// fetch then stays on the single session until sign-out.
    pub parallel_off: bool,
    /// How many times a lost session was repaired by signing in again.
    pub relogins: u32,
}

impl WriteState {
    /// Writes off and every preview forgotten. The signed-in session stays, so reads keep working.
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
    /// Sign-out, or PeopleSoft ended the session: nothing stays in memory.
    pub fn drop_session(&mut self) {
        self.disarm();
        self.backend = None;
        self.list_cache = None;
        self.parallel_off = false;
    }
    fn expire_idle(&mut self) {
        if self.armed && self.last_used.is_some_and(|t| t.elapsed() > IDLE_LIMIT) {
            self.disarm();
        }
        self.phrases.retain(|_, (_, t)| t.elapsed() < PREVIEW_KEEP);
        let live: Vec<String> = self.phrases.keys().cloned().collect();
        self.checks.retain(|k, _| live.contains(k));
        self.kinds.retain(|k, _| live.contains(k));
        self.drop_ctx.retain(|k, _| live.contains(k));
    }
    fn touch(&mut self) {
        self.last_used = Some(Instant::now());
    }
}

/// Sign-out: the page sees "off" at once, a long write in progress is told to stop between its steps, and the session is
/// dropped as soon as the current operation lets go of it. This never waits.
pub fn drop_session_now(app: &App) {
    app.stop.store(true, Ordering::SeqCst);
    if let Ok(mut s) = app.snap.lock() {
        s.armed = false;
    }
    prestaged_clear();
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

/// "Disable writes": the same, but the signed-in session is kept for reads. Never waits.
pub fn disarm_now(app: &App) {
    app.stop.store(true, Ordering::SeqCst);
    if let Ok(mut s) = app.snap.lock() {
        s.armed = false;
    }
    prestaged_clear();
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

/// Every 30 s, switch writes off if they have been idle too long, but only if nothing is running (never waits).
pub fn start_idle_watch(app: App) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        loop {
            tick.tick().await;
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

fn needs_login(m: &str) -> bool {
    m.contains("sign-in required")
}

fn err(code: StatusCode, m: impl Into<String>) -> ApiErr {
    ApiErr(code, m.into())
}

async fn with_state<T: Send + 'static>(
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
    let op_for_log = op.clone();
    // All the bookkeeping (busy, armed, the log line) happens INSIDE the blocking closure. If the HTTP request is dropped while awaiting
    // (the client went away), the closure still finishes and still clears `busy`; before, that code ran after the await and was skipped.
    let handle = tokio::task::spawn_blocking(move || {
        let mut g = st.lock().map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "internal error"))?;
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
            // A pending "disable writes" or sign-out already told the page writes are off; keep saying so until it has run.
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
    // A panic in the closure must not leave `busy` set either.
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

fn armed_backend(st: &mut WriteState) -> Result<&mut Box<dyn Backend>, ApiErr> {
    if !st.armed {
        return Err(err(StatusCode::CONFLICT, "Writes are off. Enable them first."));
    }
    st.touch();
    st.list_page_valid = false; // a write path is about to navigate
    st.backend.as_mut().ok_or_else(|| err(StatusCode::CONFLICT, "Writes are off. Enable them first."))
}

// ------------------------------------------------------------------ status, arm, disarm

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
    acks: Vec<String>,
    version: String,
}

pub async fn arm(State(app): State<App>, Json(req): Json<ArmReq>) -> Result<Json<Value>, ApiErr> {
    let need: BTreeSet<&str> = statements().iter().map(|s| s.0).collect();
    let got: BTreeSet<&str> = req.acks.iter().map(String::as_str).collect();
    if req.version != CONSENT_VERSION || need != got {
        return Err(err(StatusCode::BAD_REQUEST, "Tick every statement on the current consent screen to enable writes."));
    }
    let demo = app.demo;
    let creds = if demo { None } else { Some(crate::current_creds(&app)?) };
    let stop = app.stop.clone();
    with_state(&app, "arm", move |st| {
        // Cleared here, once everything queued before this arming has finished, and not when the request arrived: an earlier
        // "Disable writes" must still reach the operation it was meant to stop.
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

// ------------------------------------------------------------------ reads, previews, confirm

const READ_TOOLS: [&str; 14] = ["catalog_search", "catalog_detail", "catalog_sections", "degree_tree", "degree_requirement", "degree_course_list", "plan_overview", "plan_expand_requirement", "plan_class_list", "plan_select_class", "drop_terms", "enrolled_classes", "swap_candidates", "swap_options"];
/// Enrol stage and submit are deliberately NOT here: they can only be reached through `prepare_enrol`, which reads the
/// seats fresh before each step. A preview that skipped those reads would also skip the check at confirm time.
const PREVIEW_TOOLS: [&str; 4] = ["preview_drop", "preview_swap_stage", "preview_swap_commit", "preview_plan_term"];

#[derive(Deserialize)]
pub struct ToolReq {
    tool: String,
    #[serde(default)]
    args: Value,
    /// Class numbers the person chose. If the preview names different ones, nothing is offered for confirmation.
    #[serde(default)]
    expect: Option<Vec<String>>,
    /// The course, key, chosen class numbers and the seats the person saw. Required for a swap's new class.
    #[serde(default)]
    check: Option<SeatCheck>,
    /// For `preview_plan_term`: the credit total of the destination term after the move, as the page counted it.
    #[serde(default)]
    credits: Option<CreditsReq>,
}

/// A plan write is course planning only. PeopleSoft's Save accepts an over-limit plan (seen live), so this server is the one that refuses:
/// no credit check, a limit it does not agree with, unknown credits or an over-limit term all stop the preview before anything is sent.
/// Un-planning (a blank term) is refused too: a blank term plus Save did not remove a course when tried live.
pub fn check_plan_request(args: &Value, credits: Option<&CreditsReq>) -> Result<(), String> {
    match args["term_code"].as_str() {
        Some(t) if !t.is_empty() => {}
        _ => return Err("Removing a course from the plan is not supported yet: PeopleSoft did not accept it when tried.".into()),
    }
    let Some(c) = credits else {
        return Err("This page is out of date: it did not check the credit limit. Reload it and try again.".into());
    };
    let limit = credit_limit(&c.term);
    if (c.limit - limit).abs() > 1e-9 {
        return Err(format!("The page and the server disagree about the credit limit for {} ({} against {}). Reload the page.", c.term, c.limit, limit));
    }
    if !c.unknown.is_empty() {
        return Err(format!("The credits of {} are unknown, so the term's total cannot be trusted. Nothing was sent.", c.unknown.join(", ")));
    }
    if !(c.after.is_finite() && c.after >= 0.0) {
        return Err("The credit total is not a number. Nothing was sent.".into());
    }
    if c.after > limit + 1e-9 {
        return Err(format!("{} credits is over the {} credits PeopleSoft allows in {}. PeopleSoft would save it but refuse to enrol later, so nothing was sent.", c.after, limit, c.term));
    }
    Ok(())
}

/// "Fall 2025" is order 6075, "Winter 2026" 6076, "Spring 2026" 6077, "Fall 2026" 6078: the same arithmetic as the page's `termOrder`.
fn term_order(label: &str) -> Option<i64> {
    let (season, year) = label.split_once(' ')?;
    let y: i64 = year.parse().ok()?;
    match season {
        "Fall" => Some(y * 3),
        "Winter" => Some((y - 1) * 3 + 1),
        "Spring" => Some((y - 1) * 3 + 2),
        _ => None,
    }
}

fn term_from_order(n: i64) -> String {
    match n % 3 {
        0 => format!("Fall {}", n / 3),
        1 => format!("Winter {}", (n - 1) / 3 + 1),
        _ => format!("Spring {}", (n - 2) / 3 + 1),
    }
}

/// The degree's own order of terms: its first Semester is the term of the first taken course in it, each next Semester the next term.
/// A course's Semester block therefore says which term it belongs to, whatever term PeopleSoft has it planned in.
fn curriculum_term(rows: &[Value], semester: u64) -> Option<String> {
    let first = rows.iter().filter_map(|r| r["semester"].as_u64()).min()?;
    let base = rows.iter().find(|r| r["semester"].as_u64() == Some(first) && r["term"].as_str().is_some_and(|t| !t.is_empty())).and_then(|r| term_order(r["term"].as_str()?))?;
    Some(term_from_order(base + (semester as i64 - first as i64)))
}

fn compact_code(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

/// The term a tree row is planned or taken in: its plain term, or the label of its selected dropdown entry.
fn row_term(r: &Value) -> Option<String> {
    if let Some(t) = r["term"].as_str().filter(|t| !t.is_empty()) {
        return Some(t.to_string());
    }
    let c = r["term_choice"].as_object()?;
    let sel = c.get("selected")?.as_str()?;
    c.get("options")?.as_array()?.iter().find(|o| o["code"].as_str() == Some(sel)).and_then(|o| o["label"].as_str()).map(String::from)
}

/// The credits PeopleSoft's own tree puts in `term` (taken, in progress or planned), skipping the courses in `skip` (compact codes).
/// Courses inside Course Lists are not rows of the tree, so this is a floor: the page's own total may be higher, never lower.
/// With `set_aside`, the planning-semester rule applies: when the total is over the term's limit, a planned (not taken) course that belongs to
/// another term in the degree's order is not counted. The rule was never verified against PeopleSoft's own Enrol click, so a plan WRITE never uses it
/// (a plan saved on its strength could leave the student unable to enrol); only the enrol-only Apply does, where PeopleSoft's Enrol click is the judge.
fn term_floor(rows: &[Value], term: &str, skip: &BTreeSet<String>, set_aside: bool, extra: f64) -> Result<f64, String> {
    let units = |r: &Value| r["units"].as_str().and_then(|u| u.trim().parse::<f64>().ok());
    let mut total = extra;
    let mut elsewhere = 0.0;
    for r in rows {
        let code = r["code"].as_str().unwrap_or("");
        if code.is_empty() || skip.contains(&compact_code(code)) || row_term(r).as_deref() != Some(term) {
            continue;
        }
        let u = units(r).ok_or_else(|| format!("The credits of {code} are unknown, so {term}'s total cannot be trusted."))?;
        total += u;
        let planned_only = r["term_choice"].is_object();
        let home = r["semester"].as_u64().and_then(|s| curriculum_term(rows, s));
        if planned_only && home.as_deref().is_some_and(|h| h != term) {
            elsewhere += u;
        }
    }
    Ok(if set_aside && total > credit_limit(term) + 1e-9 { total - elsewhere } else { total })
}

/// The destination term's total after the move, recomputed from a fresh read of the degree tree (not from what the page claims):
/// every tree course in that term plus the moved course. See `term_floor` for why a plan write is strict (`set_aside` false).
pub fn fresh_tree_total(tree: &Value, dest: &str, moved_compact: &str, set_aside: bool) -> Result<f64, String> {
    let rows = tree["rows"].as_array().ok_or("The degree tree could not be read again.")?;
    let moved = rows.iter().find(|r| r["code"].as_str().is_some_and(|c| compact_code(c) == moved_compact)).ok_or("The course being planned was not found in a fresh read of the tree.")?;
    let moved_units = moved["units"].as_str().and_then(|u| u.trim().parse::<f64>().ok()).ok_or("The credits of the course being planned are unknown.")?;
    let skip: BTreeSet<String> = [moved_compact.to_string()].into_iter().collect();
    // The moved course counts in the destination exactly once: it is skipped in the rows and passed as `extra`, so the limit test sees it.
    term_floor(rows, dest, &skip, set_aside, moved_units)
}

/// Enrol-only Apply: PeopleSoft's own tree is read once more right before anything is sent (as a plan write does), so a stale or wrong page
/// total cannot start an enrolment that PeopleSoft's Enrol click will refuse. Courses this Apply drops are not counted.
pub fn apply_recount(st: &mut WriteState, req: &ApplyReq) -> Result<(), ApiErr> {
    if req.enrols.is_empty() {
        return Ok(());
    }
    let c = req.credits.as_ref().ok_or_else(|| err(StatusCode::UNPROCESSABLE_ENTITY, "This page did not check the credit limit. Nothing was sent."))?;
    let tree = armed_backend(st)?.tool("degree_tree", &json!({})).map_err(|m| err(StatusCode::UNPROCESSABLE_ENTITY, format!("Could not read PeopleSoft's plan to re-check the credits: {m} Nothing was sent.")))?;
    let rows = tree["rows"].as_array().ok_or_else(|| err(StatusCode::UNPROCESSABLE_ENTITY, "PeopleSoft's plan could not be read again. Nothing was sent."))?;
    let dropped: BTreeSet<String> = req.drops.iter().map(|d| compact_code(&d.course)).collect();
    let floor = term_floor(rows, &req.term, &dropped, true, 0.0).map_err(|m| err(StatusCode::UNPROCESSABLE_ENTITY, format!("{m} Nothing was sent.")))?;
    let limit = credit_limit(&req.term);
    if floor > limit + 1e-9 {
        return Err(err(StatusCode::UNPROCESSABLE_ENTITY, format!("PeopleSoft's own plan has {floor} credits in {} after your drops, over the {limit} allowed. Nothing was sent.", req.term)));
    }
    if floor > c.after + 1e-9 {
        return Err(err(StatusCode::CONFLICT, format!("The page counted {} credits for {} but PeopleSoft's plan has at least {floor}. The page is out of date: reload it and try again. Nothing was sent.", c.after, req.term)));
    }
    Ok(())
}

/// The preview must be for the very term the page counted credits for.
pub fn plan_term_matches(out: &Value, credits: &CreditsReq) -> bool {
    out["plan"]["term"].as_str() == Some(credits.term.as_str())
}

const ARG_NAMES: [&str; 14] = ["row", "term_row", "class_nbr", "candidate_row", "option_row", "new_class_nbrs", "year", "index", "term_code", "keyword", "crse_id", "offer_nbr", "strm", "effdt"];

fn clean_args(v: &Value) -> Result<Value, ApiErr> {
    match v {
        Value::Null => Ok(json!({})),
        Value::Object(m)
            if m.len() <= 4
                // Only the argument names the connector's tools read; anything else is refused instead of being passed along.
                && m.keys().all(|k| ARG_NAMES.contains(&k.as_str()))
                && m.iter().all(|(k, x)| {
                    // a catalog keyword: a course code or subject (letters, digits, spaces, hyphens)
                    if k == "keyword" {
                        x.as_str().is_some_and(|s| !s.trim().is_empty() && s.len() <= 40 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '-'))
                    } else if k == "effdt" {
                        x.as_str().is_some_and(|s| s.len() == 10 && s.as_bytes()[4] == b'-' && s.as_bytes()[7] == b'-' && s.bytes().enumerate().all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit()))
                    } else {
                    x.is_u64()
                        || x.as_str().is_some_and(|s| s.len() <= 12 && s.bytes().all(|b| b.is_ascii_digit()))
                        // a list of class numbers (a swap names its new option this way)
                        || x.as_array().is_some_and(|a| !a.is_empty() && a.len() <= 6 && a.iter().all(|e| e.as_str().is_some_and(|s| !s.is_empty() && s.len() <= 10 && s.bytes().all(|b| b.is_ascii_digit()))))
                    }
                }) =>
        {
            Ok(v.clone())
        }
        _ => Err(err(StatusCode::BAD_REQUEST, "Unexpected arguments.")),
    }
}

/// Reads through the live session (armed). The connector refuses nothing here because nothing here changes anything.
pub async fn read(State(app): State<App>, Json(req): Json<ToolReq>) -> Result<Json<Value>, ApiErr> {
    if !READ_TOOLS.contains(&req.tool.as_str()) {
        return Err(err(StatusCode::BAD_REQUEST, "Unknown read."));
    }
    let args = clean_args(&req.args)?;
    let demo = app.demo;
    let creds = app.creds.lock().unwrap().clone();
    let op = format!("read:{}", req.tool);
    let v = with_state(&app, op, move |st| {
        if st.armed {
            st.touch();
        }
        read_tool(st, &creds, demo, &req.tool, &args)
    })
    .await?;
    Ok(Json(v))
}

/// Refuse a preview whose class numbers differ from the ones the person chose. The token is never stashed, so the
/// server can never confirm it.
fn check_expected(data: &Value, expect: Option<&[String]>) -> Result<(), ApiErr> {
    let Some(exp) = expect else { return Ok(()) };
    let got = nums_of(&data["plan"]["class_numbers"]);
    if sorted(&got) != sorted(exp) {
        return Err(err(
            StatusCode::CONFLICT,
            format!("PeopleSoft's plan names class {} but you chose {}. Nothing was changed.", got.join(", "), exp.join(", ")),
        ));
    }
    Ok(())
}

/// Remember the phrase the connector required for this token, and hand the browser a preview without the CLI-only hint.
fn stash_preview(st: &mut WriteState, mut data: Value) -> Value {
    if let (Some(tok), Some(phrase)) = (data["token"].as_str(), data["plan"]["required_phrase"].as_str()) {
        st.phrases.insert(tok.to_string(), (phrase.to_string(), Instant::now()));
        if let Some(kind) = data["plan"]["kind"].as_str() {
            st.kinds.insert(tok.to_string(), kind.to_string());
        }
    }
    if let Some(o) = data.as_object_mut() {
        o.remove("instruction");
    }
    data
}

/// Like [`stash_preview`], and also remember the seat checks to repeat at confirm time, and show the fresh seats.
fn stash_checked(st: &mut WriteState, data: Value, checks: Vec<SeatCheck>, reports: &[Report]) -> Value {
    let mut data = stash_preview(st, data);
    if let Some(tok) = data["token"].as_str().map(String::from) {
        if !checks.is_empty() {
            st.checks.insert(tok, checks);
        }
    }
    if !reports.is_empty() {
        data["seats"] = seats::reports_json(reports);
    }
    data
}

fn seat_block(m: String) -> ApiErr {
    err(StatusCode::CONFLICT, m)
}

/// A drop or swap preview. A swap's new class is read fresh BEFORE the preview is asked for, so the preview shows
/// seats that are seconds old, and a full class never gets a token.
pub fn preview_checked(st: &mut WriteState, tool: &str, args: &Value, expect: Option<&[String]>, check: Option<&SeatCheck>) -> Result<Value, ApiErr> {
    let (checks, reports, must_match): (Vec<SeatCheck>, Vec<Report>, Option<Vec<String>>) = match tool {
        "preview_swap_stage" => {
            let c = check.ok_or_else(|| err(StatusCode::BAD_REQUEST, "A swap needs the new class's course key and seats. Fetch the course from PeopleSoft again."))?;
            c.valid().map_err(|m| err(StatusCode::BAD_REQUEST, m))?;
            let be = armed_backend(st)?;
            let reports = seats::check_all(be.as_mut(), std::slice::from_ref(c)).map_err(seat_block)?;
            (vec![c.clone()], reports, Some(c.class_nbrs.clone()))
        }
        "preview_swap_commit" => {
            let checks = st.swap_checks.clone().unwrap_or_default();
            if checks.is_empty() {
                // The swap was not staged through this arming's checked path (or writes were switched off since): there is no
                // fresh seat read to rely on, so there is no commit preview either.
                return Err(err(StatusCode::CONFLICT, "This swap was not staged in this session of writes. Stage it again first."));
            }
            let be = armed_backend(st)?;
            let reports = seats::check_all(be.as_mut(), &checks).map_err(seat_block)?;
            (checks, reports, None)
        }
        _ => (vec![], vec![], None),
    };
    let data = armed_backend(st)?.tool(tool, args).map_err(|m| err(StatusCode::UNPROCESSABLE_ENTITY, m))?;
    check_expected(&data, expect)?;
    check_expected(&data, must_match.as_deref())?;
    let out = stash_checked(st, data, checks, &reports);
    if tool == "preview_drop" {
        if let (Some(tok), Some(row), Some(nbr)) = (out["token"].as_str(), args["term_row"].as_u64(), args["class_nbr"].as_str()) {
            st.drop_ctx.insert(tok.to_string(), (row as u32, nbr.to_string(), plan_term(&out)));
        }
    }
    Ok(out)
}

/// A plan write's preview with every check: the page's credit numbers, the term, and PeopleSoft's own tree read again right now.
pub fn plan_preview(st: &mut WriteState, args: &Value, credits: &CreditsReq, expect_course: &str) -> Result<Value, ApiErr> {
    check_plan_request(args, Some(credits)).map_err(|m| err(StatusCode::UNPROCESSABLE_ENTITY, m))?;
    let v = preview_checked(st, "preview_plan_term", args, None, None)?;
    // The page addresses a course by (year, position). If its copy of the tree is out of date that position can be another course: the
    // course the student chose must be the one PeopleSoft's own tree has at that position.
    if v["plan"]["class_numbers"][0].as_str() != Some(compact_code(expect_course).as_str()) {
        return Err(err(StatusCode::CONFLICT, "The course at that place in PeopleSoft's degree is not the one you chose, so the page's copy is out of date. Nothing was changed. Reload and try again."));
    }
    if !plan_term_matches(&v, credits) {
        // The token was made but is not handed out; it expires by itself.
        return Err(err(StatusCode::CONFLICT, "The term PeopleSoft would plan into is not the one the page checked. Reload and try again."));
    }
    // Re-check the credits from PeopleSoft itself, right before anything can be sent: the page's numbers may be minutes old.
    let moved = v["plan"]["class_numbers"][0].as_str().unwrap_or("").to_string();
    let tree = st.backend.as_mut().ok_or_else(|| err(StatusCode::CONFLICT, "Writes are off. Enable them first."))?.tool("degree_tree", &json!({})).map_err(|m| err(StatusCode::UNPROCESSABLE_ENTITY, m))?;
    let fresh = fresh_tree_total(&tree, &credits.term, &moved, false).map_err(|m| err(StatusCode::UNPROCESSABLE_ENTITY, m))?;
    let limit = credit_limit(&credits.term);
    if fresh > limit + 1e-9 {
        return Err(err(StatusCode::UNPROCESSABLE_ENTITY, format!("PeopleSoft's own plan now has {fresh} credits in {} with this course, over the {limit} allowed. Nothing was sent.", credits.term)));
    }
    if fresh > credits.after + 1e-9 {
        return Err(err(StatusCode::CONFLICT, format!("The page counted {} credits for {} but PeopleSoft's plan now has at least {fresh}. The page is out of date: reload it and try again.", credits.after, credits.term)));
    }
    Ok(v)
}

pub async fn preview(State(app): State<App>, Json(req): Json<ToolReq>) -> Result<Json<Value>, ApiErr> {
    if !PREVIEW_TOOLS.contains(&req.tool.as_str()) {
        return Err(err(StatusCode::BAD_REQUEST, "Unknown preview."));
    }
    let args = clean_args(&req.args)?;
    // A plan preview must say which course it is for (`expect`: its code) and carry its credit check; neither is optional.
    let mut plan_course = String::new();
    if req.tool == "preview_plan_term" {
        check_plan_request(&args, req.credits.as_ref()).map_err(|m| err(StatusCode::UNPROCESSABLE_ENTITY, m))?;
        match req.expect.as_deref() {
            Some([c]) if !c.is_empty() && c.len() <= 12 && c.bytes().all(|b| b.is_ascii_alphanumeric()) => plan_course = c.clone(),
            _ => return Err(err(StatusCode::BAD_REQUEST, "A plan change must name the course it is for. Reload the page.")),
        }
    }
    let expect = req.expect.clone();
    let op = format!("preview:{}", req.tool);
    let credits = req.credits.clone();
    let v = with_state(&app, op, move |st| match (req.tool == "preview_plan_term", credits.as_ref()) {
        (true, Some(c)) => plan_preview(st, &args, c, &plan_course),
        _ => preview_checked(st, &req.tool, &args, expect.as_deref(), req.check.as_ref()),
    }).await?;
    Ok(Json(v))
}

#[derive(Deserialize)]
pub struct ConfirmReq {
    token: String,
    /// Only used when the server runs with PLANNER_CONFIRM=phrase.
    #[serde(default)]
    phrase: Option<String>,
}

/// After an enrol, read PeopleSoft's own class list and report what it now says about each chosen course. This is the only
/// real proof of the result; the connector's own answer is just the page's text.
fn verify_enrolled(be: &mut dyn Backend, checks: &[SeatCheck]) -> Value {
    let t = Instant::now();
    match be.tool("plan_class_list", &json!({})) {
        Ok(list) => {
            let rows: Vec<Value> = checks
                .iter()
                .map(|c| {
                    let status = list_row(&list, &c.course).and_then(|r| r["status"].as_str()).unwrap_or("not on the list");
                    // A waitlist pick is "done" when the list says Waitlisted: that is NOT an enrolment, and is reported apart.
                    let waitlisted = c.waitlist && seats::is_wait_status(status);
                    json!({ "course": c.course, "status": status, "enrolled": !c.waitlist && status.eq_ignore_ascii_case("enrolled"), "waitlisted": waitlisted })
                })
                .collect();
            log::event("verify_enrolled", json!({ "ms": log::ms_since(t), "rows": rows }));
            json!({ "readAtMs": log::now_ms(), "rows": rows })
        }
        Err(m) => {
            log::event("verify_enrolled", json!({ "ms": log::ms_since(t), "error": log::trunc(&m, 200) }));
            json!({ "error": m })
        }
    }
}

/// What PeopleSoft's own pages say about one dropped class.
#[derive(Debug)]
enum Proof {
    /// The class is not on the term's enrolled list (the list left: these classes remain). `term_left`: the whole term left
    /// the drop page because that was its last class.
    Absent { remaining: BTreeSet<String>, term_left: bool },
    /// The class is still on the term's enrolled list.
    Present { remaining: BTreeSet<String> },
    /// Nothing can be concluded.
    Unreadable(String),
}

/// The class numbers of an enrolled-list answer. `None` unless the answer really has a `classes` array: a missing or
/// mis-shaped list is "unknown", never "empty".
fn listed_numbers(l: &Value) -> Option<BTreeSet<String>> {
    let a = l["classes"].as_array()?;
    Some(a.iter().filter_map(|c| c["class_nbr"].as_str().map(String::from)).collect())
}

/// The term name PeopleSoft's drop preview reported. Row numbers shift when a term's last class is dropped (the term then
/// leaves the drop page), so the name is what identifies the term afterwards.
fn plan_term(preview: &Value) -> Option<String> {
    preview["plan"]["term"].as_str().filter(|t| !t.trim().is_empty()).map(String::from)
}

/// True only when PeopleSoft's drop page was read successfully, still lists at least one other term, and no longer lists
/// this one: a term disappears from that page when its last enrolled class goes. An unreadable or empty page proves nothing.
fn term_left_drop_page(be: &mut dyn Backend, name: &Option<String>) -> bool {
    let Some(name) = name else { return false };
    match be.tool("drop_terms", &json!({})) {
        Ok(v) => match v.as_array() {
            Some(a) if !a.is_empty() => !a.iter().any(|t| t["term"].as_str() == Some(name.as_str())),
            _ => false,
        },
        Err(_) => false,
    }
}

/// Read PeopleSoft's enrolled list for the term through the normal read path and say what it proves about `class_nbr`. The
/// connector's own "after the drop" page is not used: on the real site it was a 29 KB page that is not the enrolled list, and
/// it parsed as an empty list. The connector now refuses a page that is not the term's list, and a list answer without a
/// `classes` array is refused here too.
fn read_proof(be: &mut dyn Backend, op: &str, step_name: &str, term_row: u32, class_nbr: &str, term_name: &Option<String>) -> Proof {
    let t = Instant::now();
    let read = step(op, step_name, || be.tool("enrolled_classes", &json!({ "row": term_row })));
    let proof = match read {
        Ok(l) => match listed_numbers(&l) {
            Some(set) if set.contains(class_nbr) => Proof::Present { remaining: set },
            Some(set) => Proof::Absent { remaining: set, term_left: false },
            None => Proof::Unreadable("PeopleSoft's answer had no enrolled list in it".into()),
        },
        Err(m) => {
            if term_left_drop_page(be, term_name) {
                Proof::Absent { remaining: BTreeSet::new(), term_left: true }
            } else {
                Proof::Unreadable(m)
            }
        }
    };
    log::event(
        "verify_dropped",
        json!({
            "ms": log::ms_since(t), "class_nbr": class_nbr,
            "result": match &proof { Proof::Absent { term_left: true, .. } => "absent_term_left", Proof::Absent { .. } => "absent", Proof::Present { .. } => "present", Proof::Unreadable(_) => "unreadable" },
        }),
    );
    proof
}

/// The single-drop check as JSON for the page: `state` is `absent`, `still_present` or `unknown`.
fn proof_json(class_nbr: &str, p: &Proof) -> Value {
    match p {
        Proof::Absent { remaining, term_left } => json!({ "readAtMs": log::now_ms(), "classNbr": class_nbr, "state": "absent", "termLeft": term_left, "stillListed": false, "remaining": remaining }),
        Proof::Present { remaining } => json!({ "readAtMs": log::now_ms(), "classNbr": class_nbr, "state": "still_present", "stillListed": true, "remaining": remaining }),
        Proof::Unreadable(m) => json!({ "classNbr": class_nbr, "state": "unknown", "error": log::trunc(m, 200) }),
    }
}

pub fn confirm_token(st: &mut WriteState, token: &str, typed: Option<&str>, phrase_mode: bool, stop: &AtomicBool) -> Result<Value, ApiErr> {
    let off = || err(StatusCode::CONFLICT, "Writes were switched off before this was sent. Nothing was sent.");
    if stop.load(Ordering::SeqCst) {
        // Spend the preview: whoever switched writes off did not mean for it to be usable later.
        st.phrases.remove(token);
        st.checks.remove(token);
        return Err(off());
    }
    if !st.armed {
        return Err(err(StatusCode::CONFLICT, "Writes are off. Enable them first."));
    }
    let required = st
        .phrases
        .get(token)
        .map(|(p, _)| p.clone())
        .ok_or_else(|| err(StatusCode::CONFLICT, "That preview is no longer valid. Preview again."))?;
    // A wrong typed phrase goes straight to the connector's gate (which counts the failure); no seat read is spent on it.
    let will_pass = !phrase_mode || typed == Some(required.as_str());
    let phrase = if phrase_mode { typed.ok_or_else(|| err(StatusCode::BAD_REQUEST, "Type the phrase shown."))?.to_string() } else { required };
    st.touch();

    // The person has pressed confirm: read the seats fresh once more, and stop before anything is sent if a chosen class
    // is full, no longer open, or gone. The token is spent, so a new preview (with new numbers) is needed.
    if will_pass {
        if let Some(checks) = st.checks.get(token).cloned().filter(|c| !c.is_empty()) {
            let t = Instant::now();
            let verdict = match st.backend.as_mut() {
                Some(be) => seats::check_all(be.as_mut(), &checks),
                None => return Err(err(StatusCode::CONFLICT, "Writes are off. Enable them first.")),
            };
            log::event("confirm_seat_check", json!({ "ms": log::ms_since(t), "courses": checks.iter().map(|c| c.course.clone()).collect::<Vec<_>>(), "blocked": verdict.is_err() }));
            if let Err(msg) = verdict {
                st.phrases.remove(token);
                st.checks.remove(token);
                return Err(seat_block(msg));
            }
        }
    }

    // The seat read above takes seconds. Look again at the very last moment before anything is sent.
    if stop.load(Ordering::SeqCst) {
        st.phrases.remove(token);
        st.checks.remove(token);
        return Err(off());
    }
    let t_confirm = Instant::now();
    let out = st
        .backend
        .as_mut()
        .ok_or_else(|| err(StatusCode::CONFLICT, "Writes are off. Enable them first."))?
        .confirm(token, &phrase);
    let confirm_ms = log::ms_since(t_confirm);
    match out {
        Ok(mut v) => {
            log::event(
                "confirm",
                json!({
                    "kind": v["kind"], "class_numbers": v["classNumbers"], "ms": confirm_ms, "ok": true,
                    "messages": v["messages"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).take(5).map(|m| log::trunc(m, 200)).collect::<Vec<_>>()),
                    "detail": log::trunc(&v["detail"].to_string(), 400),
                }),
            );
            st.phrases.remove(token);
            st.kinds.remove(token);
            let drop_ctx = st.drop_ctx.remove(token);
            let spent = st.checks.remove(token);
            if v["kind"].as_str() == Some("DROP") {
                if let (Some(be), Some((term_row, nbr, term))) = (st.backend.as_mut(), drop_ctx.as_ref()) {
                    v["verify"] = proof_json(nbr, &read_proof(be.as_mut(), "drop", "verify_enrolled_list", *term_row, nbr, term));
                }
            }
            v["said"] = json!(log::take_said(&["drop_click", "drop_yes", "drop_result", "enrol_submit", "stage_option", "swap_commit"]));
            if v["kind"].as_str() == Some("ENROLL") {
                if st.defer_verify {
                    st.pending_verify = spent.clone();
                    v["verify_deferred"] = json!(true);
                } else if let (Some(be), Some(checks)) = (st.backend.as_mut(), spent.as_ref()) {
                    v["verify"] = verify_enrolled(be.as_mut(), checks);
                }
            }
            match v["kind"].as_str() {
                // Several swaps are staged before one commit: keep every course's check for that commit.
                Some("SWAPSTAGE") => {
                    let mut all = st.swap_checks.take().unwrap_or_default();
                    all.extend(spent.unwrap_or_default());
                    st.swap_checks = Some(all);
                }
                Some("SWAP") => st.swap_checks = None,
                _ => {}
            }
            Ok(v)
        }
        Err(fail) => {
            log::event("confirm", json!({ "ms": confirm_ms, "ok": false, "error": log::trunc(&fail.message, 300), "may_have_sent": fail.may_have_sent }));
            let keep = fail.message.contains("did not match"); // a wrong typed phrase leaves the preview usable
            let kind = st.kinds.get(token).cloned().unwrap_or_default();
            let drop_ctx = if keep { None } else { st.drop_ctx.get(token).cloned() };
            let spent = if keep {
                None
            } else {
                st.phrases.remove(token);
                st.kinds.remove(token);
                st.drop_ctx.remove(token);
                st.checks.remove(token)
            };
            if fail.may_have_sent && kind == "submit_enrol" {
                // The submit may have gone through: the first time this timed out, PeopleSoft carried on and enrolled the
                // student after the client had given up. So this is not a failure, it is "unknown", and the only proof is
                // PeopleSoft's own list. Read it and say what it says.
                let verify = match (st.backend.as_mut(), spent.as_ref()) {
                    (Some(be), Some(checks)) => verify_enrolled(be.as_mut(), checks),
                    _ => json!({ "error": "PeopleSoft's list could not be read afterwards." }),
                };
                log::event("enrol_unknown", json!({ "error": log::trunc(&fail.message, 300), "rows": verify["rows"] }));
                let said = log::take_said(&["enrol_submit"]);
                return Ok(json!({ "kind": "ENROLL", "unknown": true, "error": fail.message, "classNumbers": [], "messages": [], "detail": null, "verify": verify, "said": said }));
            }
            if fail.may_have_sent && v_kind_is_drop(&kind, &drop_ctx) {
                // A drop whose answer was lost may have been applied. Never show that as a plain error (it invites a retry):
                // read PeopleSoft's list and say `absent`, `still_present` or `unknown`.
                let (term_row, nbr, term) = drop_ctx.clone().unwrap_or_default();
                let proof = match st.backend.as_mut() {
                    Some(be) => read_proof(be.as_mut(), "drop", "enrolled_after_unclear_answer", term_row, &nbr, &term),
                    None => Proof::Unreadable("writes are off".into()),
                };
                let said = log::take_said(&["drop_click", "drop_yes", "drop_result"]);
                log::event("drop_unknown", json!({ "error": log::trunc(&fail.message, 300), "state": proof_json(&nbr, &proof)["state"] }));
                return Ok(json!({ "kind": "DROP", "unknown": true, "error": fail.message, "classNumbers": [nbr], "messages": [], "detail": null, "verify": proof_json(&nbr, &proof), "said": said }));
            }
            Err(err(StatusCode::UNPROCESSABLE_ENTITY, fail.text()))
        }
    }
}

fn v_kind_is_drop(kind: &str, ctx: &Option<(u32, String, Option<String>)>) -> bool {
    let _ = kind; // only a drop preview records a drop context
    ctx.is_some()
}

pub async fn confirm(State(app): State<App>, Json(req): Json<ConfirmReq>) -> Result<Json<Value>, ApiErr> {
    let phrase_mode = app.confirm_phrase;
    let stop = app.stop.clone();
    let v = with_state(&app, "confirm", move |st| confirm_token(st, &req.token, req.phrase.as_deref(), phrase_mode, &stop)).await?;
    Ok(Json(v))
}

// ------------------------------------------------------------------ enrol, stage then submit preview

/// A term row the page may send as -1 when the term is not on PeopleSoft's drop page (nothing enrolled in it). It is only used to
/// drop, and then there is nothing to drop, so any negative number reads as 0 instead of failing the whole request with a 422.
/// `null`, absent or negative: not known (None). A number: that row. Anything else is a parse error.
fn optional_row<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<u32>, D::Error> {
    let n = Option::<i64>::deserialize(d)?;
    Ok(n.filter(|n| *n >= 0).map(|n| u32::try_from(n).unwrap_or(u32::MAX)))
}

fn lenient_row<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    let n = i64::deserialize(d)?;
    Ok(u32::try_from(n.max(0)).unwrap_or(u32::MAX))
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct Pick {
    /// Row of the course in the Plan & Enrol class list.
    pub row: u32,
    /// Row of the chosen option on the course page.
    #[serde(rename = "optionRow")]
    pub option_row: u32,
    pub course: String,
    #[serde(rename = "classNbrs")]
    pub class_nbrs: Vec<String>,
    /// PeopleSoft's key for the course and term, from the live fetch. Needed to read the seats fresh.
    #[serde(default)]
    pub key: Value,
    /// Open seats per class number when the person fetched the course.
    #[serde(default)]
    pub seen: HashMap<String, Option<u32>>,
    /// Join the course's waitlist instead of enrolling (unverified; see `seats::SeatCheck::waitlist`).
    #[serde(default)]
    pub waitlist: bool,
}

impl Pick {
    fn to_check(&self) -> SeatCheck {
        SeatCheck { course: self.course.clone(), key: self.key.clone(), class_nbrs: self.class_nbrs.clone(), seen: self.seen.clone(), waitlist: self.waitlist }
    }
}

fn sorted(v: &[String]) -> Vec<String> {
    let mut s = v.to_vec();
    s.sort();
    s
}

fn nums_of(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
}

/// A course's row in a class list, by course code.
fn list_row<'a>(list: &'a Value, course: &str) -> Option<&'a Value> {
    list["rows"].as_array()?.iter().find(|r| code_of(r["course_label"].as_str().unwrap_or("")) == course)
}

/// Did PeopleSoft actually record the selection? The connector remembers what it staged, but that memory is not
/// PeopleSoft. A row that still says "Classes available" and shows no selected class did not take.
fn verify_selected(list: &Value, course: &str) -> Result<(), String> {
    let Some(row) = list_row(list, course) else {
        return Err(format!("{course} is no longer on PeopleSoft's class list."));
    };
    let status = row["status"].as_str().unwrap_or("").trim();
    let summary = row["schedule_summary"].as_str().unwrap_or("").trim();
    let selected = !(status.eq_ignore_ascii_case("classes available") && summary.is_empty());
    log::event("verify_selected", json!({ "course": course, "status": log::trunc(status, 60), "has_selected_class": !summary.is_empty(), "ok": selected }));
    if selected {
        Ok(())
    } else {
        Err(format!("PeopleSoft does not show {course} as selected (it still says \"{status}\" with no selected class), so the stage step did not take effect. Nothing was submitted."))
    }
}

/// Does a row of PeopleSoft's Plan & Enrol list have a class chosen that the next submit would enrol? The live site's words
/// (read from the real list on 2026-11-16): a course with a class chosen says "Classes available" and shows that class's
/// schedule; one with nothing chosen says "Classes available" with no schedule, or "No classes available"; one already
/// enrolled says "Enrolled" and is not part of the submit.
fn is_chosen(status: &str, summary: &str) -> bool {
    status.trim().eq_ignore_ascii_case("classes available") && !summary.trim().is_empty()
}

/// Courses chosen on the live list that are not among `picks`: [{course, title, schedule}].
fn live_leftovers(list: &Value, picks: &[Pick]) -> Vec<Value> {
    let want: BTreeSet<String> = picks.iter().map(|p| code_of(&p.course)).collect();
    list["rows"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter(|r| is_chosen(r["status"].as_str().unwrap_or(""), r["schedule_summary"].as_str().unwrap_or("")))
                .filter_map(|r| {
                    let label = r["course_label"].as_str().unwrap_or("");
                    let code = code_of(label);
                    (!want.contains(&code)).then(|| json!({ "course": code, "title": label.split_once(':').map(|x| x.1.trim()).unwrap_or(""), "schedule": log::trunc(r["schedule_summary"].as_str().unwrap_or(""), 120) }))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Compare the courses PeopleSoft's live Plan & Enrol list shows as selected (each row is [label, status, selected-class
/// summary]) with the courses chosen. `None` = they are exactly the same.
fn live_cart_mismatch(live: &[Value], picks: &[Pick]) -> Option<String> {
    let live_selected: BTreeSet<String> = live
        .iter()
        .filter(|r| {
            let status = r[1].as_str().unwrap_or("");
            let summary = r[2].as_str().unwrap_or("");
            is_chosen(status, summary)
        })
        .map(|r| code_of(r[0].as_str().unwrap_or("")))
        .collect();
    let want_codes: BTreeSet<String> = picks.iter().map(|p| code_of(&p.course)).collect();
    if live_selected == want_codes {
        return None;
    }
    let extra: Vec<String> = live_selected.difference(&want_codes).cloned().collect();
    let missing: Vec<String> = want_codes.difference(&live_selected).cloned().collect();
    Some(format!(
        "PeopleSoft's live list does not show exactly the courses you chose as selected (extra: {}; missing: {}). Nothing was submitted. Clear the Plan & Enrol list in PeopleSoft and try again.",
        if extra.is_empty() { "none".into() } else { extra.join(", ") },
        if missing.is_empty() { "none".into() } else { missing.join(", ") },
    ))
}

/// Add each chosen option to the Plan & Enrol list, then preview the submit. Stops at the first surprise and says what
/// was already staged. Nothing is staged for a pick whose class numbers do not match what the person chose.
#[cfg(test)]
pub fn prepare_enrol(st: &mut WriteState, picks: &[Pick]) -> Result<Value, ApiErr> {
    prepare_enrol_stop(st, picks, &AtomicBool::new(false))
}

/// The same, and it looks at `stop` before each course and again just before each write, so "Disable writes" or
/// sign-out ends a long enrol between steps instead of after it. Every step is timed in the log.
pub fn prepare_enrol_stop(st: &mut WriteState, picks: &[Pick], stop: &AtomicBool) -> Result<Value, ApiErr> {
    if picks.is_empty() || picks.len() > 10 {
        return Err(err(StatusCode::BAD_REQUEST, "Pick between 1 and 10 courses."));
    }
    // Everything the seat checks need is validated before the first request, so a bad pick stops everything cleanly.
    let checks: Vec<SeatCheck> = picks.iter().map(Pick::to_check).collect();
    for c in &checks {
        c.valid().map_err(|m| err(StatusCode::BAD_REQUEST, m))?;
    }
    let mut staged: Vec<String> = Vec::new();
    let stopped = |staged: &Vec<String>, at: &str, m: &str| {
        let tail = if staged.is_empty() {
            "Nothing was added to your Plan & Enrol list.".to_string()
        } else {
            format!("Already on your Plan & Enrol list: {}. They are not enrolled; remove them in PeopleSoft or try again.", staged.join(", "))
        };
        err(StatusCode::CONFLICT, format!("Stopped at {at}: {m} {tail}"))
    };
    let off = "writes were switched off while this was running.";
    let t_all = Instant::now();
    let mut previous: Option<String> = None;
    let mut stage_reports: Vec<Report> = Vec::new();
    for (p, check) in picks.iter().zip(&checks) {
        let at = p.course.clone();
        let op = format!("enrol:{at}");
        if stop.load(Ordering::SeqCst) {
            return Err(stopped(&staged, &at, off));
        }
        let be = armed_backend(st)?;
        let list = step(&op, "class_list", || be.tool("plan_class_list", &json!({}))).map_err(|m| stopped(&staged, &at, &m))?;
        // The list is read before every course anyway; use it to check that the PREVIOUS course really was selected.
        if let Some(prev) = &previous {
            verify_selected(&list, prev).map_err(|m| stopped(&staged, prev, &m))?;
        } else {
            // Before anything is sent: PeopleSoft keeps what is chosen on this list across restarts and earlier runs, and the
            // submit enrols all of it. Anything chosen that is not in this enrol is shown to the person and nothing is written.
            let left = live_leftovers(&list, picks);
            if !left.is_empty() {
                log::event("enrol_blocked_leftovers", json!({ "count": left.len(), "courses": left.iter().map(|l| l["course"].clone()).collect::<Vec<_>>() }));
                return Ok(json!({
                    "blocked": "leftovers",
                    "leftovers": left,
                    "message": "Nothing was sent. These courses are already chosen on your Plan & Enrol list in PeopleSoft, and submitting would enrol you in them too. Remove them in PeopleSoft, or add them to your picks if you want them.",
                }));
            }
        }
        // Seats before staging come from the page that is opened next. It is a fresh answer from PeopleSoft (a POST and a GET,
        // never memory), and it is the very page the stage step is built from, so it is both fresher than a separate read
        // a few seconds earlier and free. (The old separate read cost 1.4 to 2.7 s per course. A read taken AFTER opening
        // would replace the connector's remembered course page and change what the confirm sends, so there is none.)
        // A course that was just dropped has no row the page could know yet (the list reorders): find it by its code now.
        let row = if p.row == u32::MAX {
            list_row(&list, &p.course).and_then(|r| r["row"].as_u64()).map(|x| x as u32).ok_or_else(|| stopped(&staged, &at, "the course is not on PeopleSoft's Plan & Enrol list."))?
        } else {
            p.row
        };
        let opened = step(&op, "open_course", || be.tool("plan_select_class", &json!({ "row": row }))).map_err(|m| stopped(&staged, &at, &m))?;
        if opened["sections"]["course"].as_str() != Some(p.course.as_str()) {
            return Err(stopped(&staged, &at, "PeopleSoft's class list has changed since you fetched it (a different course is on that row). Fetch again."));
        }
        // Option rows differ between the plan page and the swap page, so a course whose combination was read through the swap page names
        // it by its class numbers.
        let option_row = if p.option_row == u32::MAX {
            opened["sections"]["options"]
                .as_array()
                .and_then(|o| o.iter().find(|x| sorted(&nums_of(&x["components"].as_array().map(|c| c.iter().map(|y| y["class_nbr"].clone()).collect::<Vec<_>>()).map(Value::Array).unwrap_or(Value::Null))) == sorted(&p.class_nbrs)))
                .and_then(|x| x["row"].as_u64())
                .map(|x| x as u32)
                .ok_or_else(|| stopped(&staged, &at, "PeopleSoft's page no longer offers the combination you chose. Fetch again."))?
        } else {
            p.option_row
        };
        let live_nums = opened["sections"]["options"]
            .as_array()
            .and_then(|o| o.iter().find(|x| x["row"].as_u64() == Some(u64::from(option_row))))
            .map(|o| {
                o["components"].as_array().map(|c| c.iter().filter_map(|x| x["class_nbr"].as_str().map(String::from)).collect::<Vec<_>>()).unwrap_or_default()
            })
            .unwrap_or_default();
        if sorted(&live_nums) != sorted(&p.class_nbrs) {
            return Err(stopped(&staged, &at, "the option on PeopleSoft no longer has the class numbers you chose. Fetch again."));
        }
        // The page the stage step is built from shows seats too. Judge it as well; this costs no extra request.
        let on_page = seats::evaluate(check, &opened["sections"], seats::now_ms());
        log::event("seat_on_open_page", json!({ "course": at, "readings": on_page.readings.iter().map(|r| json!({ "class": r.class_nbr, "open": r.open, "status": r.option_status, "blocked": !r.ok() })).collect::<Vec<_>>() }));
        if !on_page.blockers().is_empty() {
            return Err(stopped(&staged, &at, &format!("Not sent. {} Nothing was changed by this step, and it will not retry by itself.", on_page.blockers().join(" "))));
        }
        stage_reports.push(on_page);
        if stop.load(Ordering::SeqCst) {
            return Err(stopped(&staged, &at, off));
        }
        let prev = step(&op, "preview_stage", || be.tool("preview_stage_enrol", &json!({ "option_row": option_row }))).map_err(|m| stopped(&staged, &at, &m))?;
        if sorted(&nums_of(&prev["plan"]["class_numbers"])) != sorted(&p.class_nbrs) {
            return Err(stopped(&staged, &at, "the connector's plan names different class numbers than the ones you chose."));
        }
        let (token, phrase) = match (prev["token"].as_str(), prev["plan"]["required_phrase"].as_str()) {
            (Some(t), Some(ph)) => (t.to_string(), ph.to_string()),
            _ => return Err(stopped(&staged, &at, "no preview token was issued.")),
        };
        if stop.load(Ordering::SeqCst) {
            return Err(stopped(&staged, &at, off));
        }
        let out = step(&op, "stage_confirm", || be.confirm(&token, &phrase)).map_err(|f| stopped(&staged, &at, &f.text()))?;
        log::event(
            "stage_outcome",
            json!({
                "course": at,
                "class_numbers": p.class_nbrs,
                "messages": out["messages"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).take(5).map(|m| log::trunc(m, 200)).collect::<Vec<_>>()),
                "detail": log::trunc(&out["detail"].to_string(), 300),
            }),
        );
        staged.extend(p.class_nbrs.iter().cloned());
        previous = Some(at);
    }
    if stop.load(Ordering::SeqCst) {
        return Err(stopped(&staged, "the final step", off));
    }
    let be = armed_backend(st)?;
    // The preview shows each course's seats as read from its opened page while staging, with how long ago that was. The
    // fresh read that guards the submit is the one made when the person presses confirm (`confirm_token`), immediately
    // before the submit is sent; a read here, minutes before that, would only cost about 11 s to show numbers that are
    // about to be re-read anyway.
    let reports = stage_reports;
    let sub = step("enrol", "preview_submit", || be.tool("preview_submit_enrol", &json!({}))).map_err(|m| stopped(&staged, "the final step", &m))?;
    // The Plan & Enrol list can still hold classes staged earlier. Offer "Enrol now" only for exactly what was chosen.
    let want: BTreeSet<String> = picks.iter().flat_map(|p| p.class_nbrs.iter().cloned()).collect();
    let got: BTreeSet<String> = nums_of(&sub["plan"]["class_numbers"]).into_iter().collect();
    if got != want {
        let extra: Vec<&String> = got.difference(&want).collect();
        let missing: Vec<&String> = want.difference(&got).collect();
        return Err(stopped(
            &staged,
            "the final step",
            &format!(
                "submitting would not enrol exactly what you chose (extra on the list: {}; missing: {}). Nothing was submitted. Clear the Plan & Enrol list in PeopleSoft and try again.",
                if extra.is_empty() { "none".into() } else { extra.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ") },
                if missing.is_empty() { "none".into() } else { missing.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ") },
            ),
        ));
    }
    // The check above compares the connector's own memory of what it staged. This one reads the live Plan & Enrol page the
    // submit preview just read: the courses it shows as selected must be exactly the chosen ones. A course selected earlier
    // (by hand, or by a run that stopped) shows up here and is not enrolled by accident.
    let live = sub["plan"]["live_rows"].as_array().ok_or_else(|| stopped(&staged, "the final step", "PeopleSoft's class list could not be read, so the final list cannot be proved. Nothing was submitted."))?;
    if let Some(why) = live_cart_mismatch(live, picks) {
        return Err(stopped(&staged, "the final step", &why));
    }
    let courses: Vec<&str> = picks.iter().map(|p| p.course.as_str()).collect();
    let sub = stash_checked(st, sub, checks, &reports);
    log::event("enrol_prepared", json!({ "ms": log::ms_since(t_all), "courses": courses, "staged": staged }));
    Ok(json!({ "staged": staged, "submit": sub }))
}

// ------------------------------------------------------------------ drop in bulk

/// The phrase that confirms a bulk drop in phrase mode, naming every class number.
pub fn bulk_drop_phrase(nums: &[String]) -> String {
    format!("CONFIRM DROP {}", nums.join(" "))
}

/// A drop that PeopleSoft answered but whose effect has not been read back yet. The proof for it comes from the NEXT
/// course's preview (a fresh, validated read of the term's enrolled list, made before that course's confirm), or from one
/// explicit read when no next preview happens.
struct PendingDrop {
    idx: usize,
    nbr: String,
}

/// Record what the real list says about the pending drop. Returns true if the batch may go on.
fn settle(results: &mut [Value], p: &PendingDrop, proof: Proof, halted: &mut Option<String>, remaining: &mut Option<BTreeSet<String>>) -> bool {
    let n = &p.nbr;
    let row = &mut results[p.idx];
    match proof {
        Proof::Absent { remaining: r, term_left } => {
            row["status"] = json!("dropped");
            row["note"] = if term_left { json!("PeopleSoft no longer lists this term on its drop page: no enrolled classes are left in it.") } else { Value::Null };
            *remaining = Some(r);
            true
        }
        Proof::Present { remaining: r } => {
            row["status"] = json!("still_listed");
            row["note"] = json!("PeopleSoft still lists this class as enrolled after the drop, so the drop did not take effect.");
            halted.get_or_insert(format!("stopped: class {n} is still on your enrolled list after its drop."));
            *remaining = Some(r);
            false
        }
        Proof::Unreadable(m) => {
            row["status"] = json!("dropped_unverified");
            row["note"] = json!(format!("PeopleSoft's enrolled list could not be read afterwards ({}), so this is not confirmed.", log::trunc(&m, 200)));
            halted.get_or_insert(format!("stopped: class {n} could not be checked against PeopleSoft's enrolled list."));
            false
        }
    }
}

/// Drop several enrolled classes of one term, one after another, each through the connector's own preview and gate.
/// The person's single confirmation covers the batch. It stops at the first problem (nothing is retried), skips the rest
/// and says exactly what happened to each class. A class that PeopleSoft removed together with a linked class (a lab
/// that goes with its lecture) is reported as gone, not as a failure. Every step is timed in the log.
///
/// Proof: a drop counts as `dropped` only when PeopleSoft's own enrolled list (a page the connector checked to be that term's
/// list) no longer shows the class. For every course but the last, that list is the one the NEXT course's preview reads
/// anyway, so no separate read is spent, and the next confirm is still never sent before the previous drop is proved. The
/// last course gets one explicit read.
pub fn drop_bulk(st: &mut WriteState, term_row: u32, nums: &[String], stop: &AtomicBool, phrase_mode: bool, typed: Option<&str>) -> Result<Value, ApiErr> {
    if nums.is_empty() || nums.len() > 12 {
        return Err(err(StatusCode::BAD_REQUEST, "Pick between 1 and 12 classes to drop."));
    }
    let mut seen = BTreeSet::new();
    for n in nums {
        if n.is_empty() || n.len() > 10 || !n.bytes().all(|b| b.is_ascii_digit()) || !seen.insert(n.clone()) {
            return Err(err(StatusCode::BAD_REQUEST, "Class numbers must be distinct digits."));
        }
    }
    if phrase_mode && typed != Some(bulk_drop_phrase(nums).as_str()) {
        return Err(err(StatusCode::BAD_REQUEST, format!("Type the phrase shown: {}", bulk_drop_phrase(nums))));
    }
    armed_backend(st)?; // writes must be on
    let t_all = Instant::now();
    let mut results: Vec<Value> = Vec::new();
    let mut remaining: Option<BTreeSet<String>> = None;
    let mut halted: Option<String> = None;
    // The term's name, from the first preview's plan: row numbers shift when a term's last class goes.
    let mut term_name: Option<String> = None;
    let mut pending: Option<PendingDrop> = None;

    for n in nums {
        // Before anything else this round: if the batch is ending (a problem, or "Disable writes"), prove the last drop first.
        if halted.is_none() && stop.load(Ordering::SeqCst) {
            halted = Some("writes were switched off while this was running.".to_string());
        }
        if let Some(why) = halted.clone() {
            if let Some(p) = pending.take() {
                let be = armed_backend(st)?;
                let proof = read_proof(&mut **be, &format!("drop:{}", p.nbr), "verify_enrolled_list", term_row, &p.nbr, &term_name);
                settle(&mut results, &p, proof, &mut halted, &mut remaining);
            }
            results.push(json!({ "classNbr": n, "status": "skipped", "note": why }));
            continue;
        }
        if remaining.as_ref().is_some_and(|r| !r.contains(n)) {
            results.push(json!({ "classNbr": n, "status": "gone", "note": "PeopleSoft already removed it, together with a linked class." }));
            continue;
        }
        let op = format!("drop:{n}");
        let _ = log::take_said(&["drop_click", "drop_yes", "drop_result", "drop_review"]);
        let t = Instant::now();
        let be = armed_backend(st)?;
        let preview = match step(&op, "preview", || be.tool("preview_drop", &json!({ "term_row": term_row, "class_nbr": n }))) {
            Ok(p) => p,
            Err(m) => {
                // Not previewable. If the cause is the previous drop (this class went with it), say so; either way the previous
                // drop is proved from a read now, before anything else happens.
                if let Some(p) = pending.take() {
                    let proof = read_proof(&mut **be, &format!("drop:{}", p.nbr), "verify_enrolled_list", term_row, &p.nbr, &term_name);
                    let ok = settle(&mut results, &p, proof, &mut halted, &mut remaining);
                    if ok && remaining.as_ref().is_some_and(|r| !r.contains(n)) {
                        results.push(json!({ "classNbr": n, "status": "gone", "note": "PeopleSoft already removed it, together with a linked class." }));
                        continue;
                    }
                }
                halted.get_or_insert(format!("stopped after class {n} could not be previewed."));
                results.push(json!({ "classNbr": n, "status": "failed", "note": log::trunc(&m, 300) }));
                continue;
            }
        };
        if term_name.is_none() {
            term_name = plan_term(&preview);
        }
        // This preview read the term's enrolled list. If it is a recognised list, it also proves the previous drop.
        if let Some(p) = pending.take() {
            let proof = match preview["plan"]["listed_classes"].as_array() {
                Some(a) => {
                    let set: BTreeSet<String> = a.iter().filter_map(|x| x.as_str().map(String::from)).collect();
                    log::event("verify_dropped", json!({ "class_nbr": p.nbr, "result": if set.contains(&p.nbr) { "present" } else { "absent" }, "via": "next_preview" }));
                    if set.contains(&p.nbr) { Proof::Present { remaining: set } } else { Proof::Absent { remaining: set, term_left: false } }
                }
                None => read_proof(&mut **be, &format!("drop:{}", p.nbr), "verify_enrolled_list", term_row, &p.nbr, &term_name),
            };
            if !settle(&mut results, &p, proof, &mut halted, &mut remaining) {
                results.push(json!({ "classNbr": n, "status": "skipped", "note": halted }));
                continue;
            }
            if remaining.as_ref().is_some_and(|r| !r.contains(n)) {
                // Cannot happen after a successful preview of n (it was on the list), but never confirm what is not listed.
                results.push(json!({ "classNbr": n, "status": "gone", "note": "PeopleSoft already removed it, together with a linked class." }));
                continue;
            }
        }
        if nums_of(&preview["plan"]["class_numbers"]) != vec![n.clone()] {
            halted = Some(format!("stopped: PeopleSoft's plan for class {n} named different classes."));
            results.push(json!({ "classNbr": n, "status": "failed", "note": "PeopleSoft's plan names different class numbers than the one you chose. Nothing was dropped for it." }));
            continue;
        }
        let (token, phrase) = match (preview["token"].as_str(), preview["plan"]["required_phrase"].as_str()) {
            (Some(t), Some(p)) => (t.to_string(), p.to_string()),
            _ => {
                halted = Some(format!("stopped: no preview was issued for class {n}."));
                results.push(json!({ "classNbr": n, "status": "failed", "note": "no preview token was issued" }));
                continue;
            }
        };
        if stop.load(Ordering::SeqCst) {
            halted = Some("writes were switched off while this was running.".to_string());
            results.push(json!({ "classNbr": n, "status": "skipped", "note": halted }));
            continue;
        }
        match step(&op, "confirm", || be.confirm(&token, &phrase)) {
            Err(f) if f.may_have_sent => {
                // The drop may have gone through. Do not guess: read PeopleSoft's enrolled list and say what it shows.
                match read_proof(&mut **be, &op, "enrolled_after_unclear_answer", term_row, n, &term_name) {
                    Proof::Absent { remaining: r, term_left } => {
                        let note = if term_left { "PeopleSoft's answer was cut off, but its drop page no longer lists this term: no enrolled classes are left in it." } else { "PeopleSoft's answer was cut off, but its enrolled list no longer shows this class." };
                        results.push(json!({ "classNbr": n, "status": "dropped", "note": note, "messages": [] }));
                        remaining = Some(r);
                    }
                    Proof::Present { remaining: r } => {
                        halted = Some(format!("stopped: PeopleSoft's answer for class {n} was unclear and it is still listed as enrolled."));
                        results.push(json!({ "classNbr": n, "status": "unknown", "note": f.text(), "messages": [] }));
                        remaining = Some(r);
                    }
                    Proof::Unreadable(m2) => {
                        halted = Some(format!("stopped: the answer for class {n} was unclear and PeopleSoft's list could not be read."));
                        results.push(json!({ "classNbr": n, "status": "unknown", "note": format!("{} Could not read the list afterwards: {}", f.text(), log::trunc(&m2, 200)) }));
                    }
                }
            }
            Err(f) => {
                halted = Some(format!("stopped after class {n} failed."));
                results.push(json!({ "classNbr": n, "status": "failed", "note": log::trunc(&f.message, 300) }));
            }
            Ok(out) => {
                let messages: Vec<String> = out["messages"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).take(5).map(|m| log::trunc(m, 200)).collect()).unwrap_or_default();
                let said = log::take_said(&["drop_click", "drop_yes", "drop_result"]);
                log::event("drop_outcome", json!({ "class_nbr": n, "ms": log::ms_since(t), "messages": messages, "detail": log::trunc(&out["detail"].to_string(), 400) }));
                if out["detail"]["stopped_before_yes"] == true {
                    halted = Some(format!("stopped: PeopleSoft reported a problem for class {n} before the final yes."));
                    results.push(json!({ "classNbr": n, "status": "stopped", "note": "PeopleSoft reported a problem before the final yes, so nothing was dropped for this class.", "said": said, "messages": messages }));
                    continue;
                }
                // The connector's own "after the drop" page is NOT proof (on the real site it parsed as an empty list). The proof is
                // PeopleSoft's real enrolled list, read back by the next course's preview or, for the last course, by one read.
                // Until then the row says so.
                results.push(json!({ "classNbr": n, "status": "dropped_unverified", "note": "Not read back yet.", "said": said, "messages": messages, "ms": log::ms_since(t) }));
                pending = Some(PendingDrop { idx: results.len() - 1, nbr: n.clone() });
            }
        }
    }
    if let Some(p) = pending.take() {
        let be = armed_backend(st)?;
        let proof = read_proof(&mut **be, &format!("drop:{}", p.nbr), "verify_enrolled_list", term_row, &p.nbr, &term_name);
        settle(&mut results, &p, proof, &mut halted, &mut remaining);
    }
    let count = |s: &str| results.iter().filter(|r| r["status"] == s).count();
    let (dropped, gone, failed, skipped) = (count("dropped"), count("gone"), count("failed") + count("stopped") + count("still_listed"), count("skipped"));
    let unproved = count("unknown") + count("dropped_unverified");
    log::event("drop_bulk_done", json!({ "ms": log::ms_since(t_all), "asked": nums.len(), "dropped": dropped, "gone": gone, "failed": failed, "skipped": skipped, "unproved": unproved, "halted": halted }));
    debug_assert_eq!(dropped + gone + failed + skipped + unproved, results.len(), "every row is counted exactly once");
    Ok(json!({ "results": results, "halted": halted, "remaining": remaining.map(|r| r.into_iter().collect::<Vec<_>>()) }))
}

// ------------------------------------------------------------------ the live session, reads, and the fast fetch

/// The signed-in session, created on first use from the credentials held in memory (or the demo mock).
fn live<'a>(st: &'a mut WriteState, creds: &Option<Creds>, demo: bool) -> Result<&'a mut Box<dyn Backend>, ApiErr> {
    if st.backend.is_none() {
        st.list_cache = None;
        st.backend = Some(if demo {
            build_demo()
        } else {
            let c = creds.as_ref().ok_or_else(|| err(StatusCode::UNAUTHORIZED, "Sign in first."))?;
            build_live(c).map_err(|m| err(StatusCode::BAD_GATEWAY, m))?
        });
    }
    st.backend.as_mut().ok_or_else(|| err(StatusCode::INTERNAL_SERVER_ERROR, "no session"))
}

/// One read through the session. If PeopleSoft has signed the session out (idle) and no write is in progress,
/// sign in again once and retry. While writes are armed a lost session is never repaired silently: the staged
/// state would be gone, so the error reaches the person instead.
fn read_tool(st: &mut WriteState, creds: &Option<Creds>, demo: bool, tool: &str, args: &Value) -> Result<Value, ApiErr> {
    if tool != "plan_class_list" {
        st.list_page_valid = false; // every other read moves PeopleSoft's session off the list page
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

/// Sign in for real (the connector's own login), replacing any earlier session.
pub async fn sign_in(app: &App, creds: Creds) -> Result<(), ApiErr> {
    with_state(app, "sign_in", move |st| {
        st.drop_session();
        st.backend = Some(build_live(&creds).map_err(|m| err(StatusCode::UNAUTHORIZED, m))?);
        Ok(())
    })
    .await
}

/// The Plan & Enrol class list, read fresh. It is also the one slow page that warms the session for direct fetches.
pub async fn classes(app: &App) -> Result<Value, ApiErr> {
    let demo = app.demo;
    let creds = app.creds.lock().unwrap().clone();
    if !demo && creds.is_none() {
        return Err(err(StatusCode::UNAUTHORIZED, "Sign in first."));
    }
    with_state(app, "classes", move |st| {
        let v = read_tool(st, &creds, demo, "plan_class_list", &json!({}))?;
        empty_list_is_not_trusted(st.list_cache.as_ref().map(|(_, p)| p), &v)?;
        st.list_cache = Some((Instant::now(), v.clone()));
        st.list_page_valid = true;
        Ok(v)
    })
    .await
}

/// A class list that comes back empty although the last good one had courses is a bad page (maintenance notice, empty answer), not a
/// student who suddenly has nothing: it is refused instead of being believed. (The connector's own fix is in the frankie working tree;
/// this guard covers the pinned build.)
fn empty_list_is_not_trusted(prev: Option<&Value>, now: &Value) -> Result<(), ApiErr> {
    let n = |v: &Value| v["rows"].as_array().map_or(0, |a| a.len());
    if n(now) == 0 && prev.is_some_and(|p| n(p) > 0) {
        return Err(err(StatusCode::BAD_GATEWAY, "PeopleSoft returned an empty class list. It was not trusted, so nothing changed here. Try again in a moment."));
    }
    Ok(())
}

const LIST_FRESH: Duration = Duration::from_secs(10 * 60);

fn code_of(label: &str) -> String {
    label.split(':').next().unwrap_or("").trim().to_string()
}

fn course_row(list: &Value, row: u32) -> Option<&Value> {
    list["rows"].as_array().and_then(|a| a.iter().find(|r| r["row"].as_u64() == Some(u64::from(row))))
}

/// Sections for the requested class-list rows.
///
/// Measured against the real site: the class list takes about 16 s to load and a course page about 2 s, and the slow way
/// (re-list before every course) pays for the list once per course. So: visit the list once (it warms the session),
/// then read every course whose key is already known directly. A course without a usable key is opened the slow way.
/// A direct result is accepted only if it names the same course and term as the list row; otherwise it is opened.
/// Extra read-only sessions are used only when asked for (2 or more), only while writes are OFF (a new sign-in is never made
/// in the middle of a write session), and only until one has misbehaved.
pub fn parallel_allowed(helpers: usize, writes_armed: bool, switched_off: bool) -> bool {
    helpers >= 2 && !writes_armed && !switched_off
}

/// Makes a fresh, signed-in, read-only session for a helper thread.
pub type HelperMaker<'a> = &'a (dyn Fn() -> Result<Box<dyn Backend>, String> + Sync);

/// The most extra sessions opened at once.
const MAX_HELPERS: usize = 4;

/// Open one course on its own short-lived, read-only session: sign in, read the class list, open the course. It never
/// arms writes and is dropped afterwards.
fn open_on_helper(make: HelperMaker, row: u32, code: &str) -> Result<Value, String> {
    let t = Instant::now();
    let r = (|| -> Result<Value, String> {
        let mut be = make()?;
        be.tool("plan_class_list", &json!({}))?;
        let v = be.tool("plan_select_class", &json!({ "row": row }))?;
        if v["sections"]["course"].as_str() != Some(code) {
            return Err(format!("a different course is on row {row}"));
        }
        Ok(v)
    })();
    log::event("parallel_open", json!({ "course": code, "ms": log::ms_since(t), "ok": r.is_ok(), "error": r.as_ref().err().map(|e| log::trunc(e, 200)) }));
    r
}

pub fn fetch_sections(st: &mut WriteState, creds: &Option<Creds>, demo: bool, rows: &[u32], known: &HashMap<String, Value>, helpers: usize, make: Option<HelperMaker>) -> Result<Value, ApiErr> {
    let t0 = Instant::now();
    let mut list_visited = false;
    let list = match &st.list_cache {
        Some((t, v)) if t.elapsed() < LIST_FRESH => v.clone(),
        _ => {
            let v = step("sections", "class_list", || read_tool(st, creds, demo, "plan_class_list", &json!({})))?;
            st.list_cache = Some((Instant::now(), v.clone()));
            st.list_page_valid = true;
            list_visited = true;
            v
        }
    };

    // Courses with no remembered key can only be opened the slow way, which is about 15 s each when done one after another.
    // Open up to `helpers` of them AT THE SAME TIME, each on its own short-lived read-only session, while this session reads
    // the others directly. Reads only: nothing here can change anything on the account, and if anything misbehaves the
    // fetch falls back to the single session and parallel reads are switched off.
    let cold: Vec<(u32, String)> = rows
        .iter()
        .filter_map(|&row| {
            let code = code_of(course_row(&list, row)?["course_label"].as_str().unwrap_or(""));
            known.get(&code).and_then(safe_key).is_none().then_some((row, code))
        })
        .collect();
    let jobs: Vec<(u32, String)> = match make {
        Some(_) if helpers >= 2 && cold.len() >= 2 && !st.parallel_off => cold.iter().take(helpers.min(MAX_HELPERS)).cloned().collect(),
        _ => Vec::new(),
    };
    let relogins_before = st.relogins;
    let mut helper_out: HashMap<u32, Result<Value, String>> = HashMap::new();

    let mut results: HashMap<u32, Value> = HashMap::new();
    let mut slow: Vec<(u32, String)> = Vec::new();
    std::thread::scope(|sc| {
        let handles: Vec<_> = match make {
            Some(mk) => jobs.iter().map(|(row, code)| sc.spawn(move || (*row, open_on_helper(mk, *row, code)))).collect(),
            None => Vec::new(),
        };
        for &row in rows {
            let Some(r) = course_row(&list, row) else {
                results.insert(row, json!({ "row": row, "error": "That row is not on PeopleSoft's class list any more. Load the class list again." }));
                continue;
            };
            let code = code_of(r["course_label"].as_str().unwrap_or(""));
            let term = r["term"].as_str().unwrap_or("").to_string();
            let direct = known.get(&code).and_then(safe_key).and_then(|key| {
                let d = step("sections", &format!("direct:{code}"), || read_tool(st, creds, demo, "course_sections", &key)).ok()?;
                let same = d["course"].as_str() == Some(code.as_str()) && (term.is_empty() || d["term"].as_str() == Some(term.as_str())) && d["options"].is_array();
                same.then(|| json!({ "row": row, "key": key, "sections": d, "via": "direct" }))
            });
            match direct {
                Some(v) => {
                    results.insert(row, v);
                }
                None => slow.push((row, code)),
            }
        }
        for h in handles {
            if let Ok((row, r)) = h.join() {
                helper_out.insert(row, r);
            }
        }
    });
    let mut helper_failed = false;
    for (row, r) in helper_out {
        match r {
            Ok(v) => {
                results.insert(row, json!({ "row": row, "key": v["course_key"], "sections": v["sections"], "via": "parallel" }));
                slow.retain(|(r2, _)| *r2 != row);
            }
            Err(_) => helper_failed = true, // that course stays in `slow` and is opened on the main session below
        }
    }
    if !jobs.is_empty() && (helper_failed || st.relogins > relogins_before) {
        st.parallel_off = true;
        log::event("parallel_disabled", json!({ "helper_failed": helper_failed, "main_session_lost": st.relogins > relogins_before }));
    }

    // The slow way. Opening a course moves PeopleSoft's page on, so the list is read again before the next one. The first
    // course is the exception: if the list page was the last thing read (no direct reads, writes or other navigation since),
    // it is still in front of the session and the course can be opened from it. That saves a ~15 s reload. If PeopleSoft
    // refuses, fall back to the reload and try once more; nothing here writes, so the retry is only a read.
    let mut page_fresh = list_visited || st.list_page_valid;
    for (row, code) in slow {
        let skipped_reload = page_fresh && !list_visited && st.list_page_valid;
        if !page_fresh {
            let v = step("sections", "class_list_again", || read_tool(st, creds, demo, "plan_class_list", &json!({})))?;
            st.list_cache = Some((Instant::now(), v));
        }
        page_fresh = false;
        if skipped_reload {
            log::event("skip_relist", json!({ "course": code, "saved": "one class-list reload" }));
        }
        let mut opened = step("sections", &format!("open:{code}"), || read_tool(st, creds, demo, "plan_select_class", &json!({ "row": row })));
        if opened.is_err() && skipped_reload {
            log::event("skip_relist_failed", json!({ "course": code, "then": "reload the list and try once more" }));
            let v = step("sections", "class_list_fallback", || read_tool(st, creds, demo, "plan_class_list", &json!({})))?;
            st.list_cache = Some((Instant::now(), v));
            opened = step("sections", &format!("open_retry:{code}"), || read_tool(st, creds, demo, "plan_select_class", &json!({ "row": row })));
        }
        let v = match opened {
            Ok(v) if v["sections"]["course"].as_str() == Some(code.as_str()) => json!({ "row": row, "key": v["course_key"], "sections": v["sections"], "via": "open" }),
            Ok(_) => json!({ "row": row, "error": "A different course is on that row now. Load the class list again." }),
            Err(e) => json!({ "row": row, "error": e.1 }),
        };
        results.insert(row, v);
    }

    let courses: Vec<Value> = rows.iter().filter_map(|r| results.remove(r)).collect();
    let count = |via: &str| courses.iter().filter(|c| c["via"] == via).count();
    let stats = json!({
        "direct": count("direct"),
        "opened": count("open"),
        "parallel": count("parallel"),
        "helpers": jobs.len(),
        "listVisited": list_visited,
        "ms": t0.elapsed().as_millis() as u64,
    });
    Ok(json!({ "courses": courses, "stats": stats }))
}

// ------------------------------------------------------------------ refresh: class list and enrolled list at the same time

/// The enrolled list of one term on the given session: the term's row on the drop page, then its classes.
fn read_enrolled_on(be: &mut dyn Backend, term: &str) -> Result<Value, String> {
    let terms = be.tool("drop_terms", &json!({}))?;
    if terms.as_array().is_some_and(|a| a.is_empty()) {
        return Err("PeopleSoft's drop page listed no terms. It was not trusted.".into());
    }
    let row = terms.as_array().and_then(|a| a.iter().find(|t| t["term"].as_str().is_some_and(|x| x.trim().eq_ignore_ascii_case(term.trim())))).and_then(|t| t["row"].as_u64());
    match row {
        None => Ok(json!({ "termRow": -1, "classes": [] })),
        Some(r) => {
            let l = be.tool("enrolled_classes", &json!({ "row": r }))?;
            Ok(json!({ "termRow": r, "classes": l["classes"] }))
        }
    }
}

/// Everything a refresh needs from the two quick pages: PeopleSoft's class list (on the main session) and, at the same time on
/// one extra read-only session, the term's enrolled list. Without a helper (or after one failed) the enrolled list is read on the main
/// session after the class list. Reads only.
pub fn fetch_refresh(st: &mut WriteState, creds: &Option<Creds>, demo: bool, term: Option<&str>, helpers: usize, make: Option<HelperMaker>) -> Result<Value, ApiErr> {
    let t0 = Instant::now();
    let relogins_before = st.relogins;
    let use_helper = term.is_some() && helpers >= 2 && !st.parallel_off && make.is_some();
    let mut helper_res: Option<Result<Value, String>> = None;
    let mut list_ms = 0u64;
    let list = std::thread::scope(|sc| {
        let handle = match (use_helper, make, term) {
            (true, Some(mk), Some(t)) => Some(sc.spawn(move || {
                let t1 = Instant::now();
                let r = mk().and_then(|mut be| read_enrolled_on(&mut *be, t));
                log::event("parallel_refresh", json!({ "ms": log::ms_since(t1), "ok": r.is_ok(), "error": r.as_ref().err().map(|e| log::trunc(e, 200)) }));
                r
            })),
            _ => None,
        };
        let t1 = Instant::now();
        let list = step("refresh", "class_list", || read_tool(st, creds, demo, "plan_class_list", &json!({})));
        list_ms = t1.elapsed().as_millis() as u64;
        if let Some(h) = handle {
            helper_res = Some(h.join().unwrap_or_else(|_| Err("the helper stopped".into())));
        }
        list
    })?;
    empty_list_is_not_trusted(st.list_cache.as_ref().map(|(_, p)| p), &list)?;
    st.list_cache = Some((Instant::now(), list.clone()));
    st.list_page_valid = true;
    let mut via = "parallel";
    let enrolled = match (term, helper_res) {
        (None, _) => Value::Null,
        (Some(_), Some(Ok(v))) => v,
        (Some(t), other) => {
            // No helper, or it failed: read it on the main session now.
            if matches!(other, Some(Err(_))) || st.relogins > relogins_before {
                st.parallel_off = true;
                log::event("parallel_disabled", json!({ "helper_failed": matches!(other, Some(Err(_))), "main_session_lost": st.relogins > relogins_before, "where": "refresh" }));
            }
            via = "main";
            let t = t.to_string();
            let terms = step("refresh", "drop_terms", || read_tool(st, creds, demo, "drop_terms", &json!({})))?;
            let row = terms.as_array().and_then(|a| a.iter().find(|x| x["term"].as_str().is_some_and(|y| y.trim().eq_ignore_ascii_case(t.trim())))).and_then(|x| x["row"].as_u64());
            match row {
                None => json!({ "termRow": -1, "classes": [] }),
                Some(r) => {
                    let l = step("refresh", "enrolled_classes", || read_tool(st, creds, demo, "enrolled_classes", &json!({ "row": r })))?;
                    json!({ "termRow": r, "classes": l["classes"] })
                }
            }
        }
    };
    Ok(json!({ "classes": list, "enrolled": enrolled, "stats": { "via": via, "ms": t0.elapsed().as_millis() as u64, "listMs": list_ms } }))
}

#[derive(Deserialize)]
pub struct RefreshReq {
    #[serde(default)]
    term: Option<String>,
}

pub async fn refresh(State(app): State<App>, Json(r): Json<RefreshReq>) -> Result<Json<Value>, ApiErr> {
    if r.term.as_ref().is_some_and(|t| t.len() > 40 || !t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b' ')) {
        return Err(err(StatusCode::BAD_REQUEST, "Unexpected term."));
    }
    let demo = app.demo;
    let creds = app.creds.lock().unwrap().clone();
    if !demo && creds.is_none() {
        return Err(err(StatusCode::UNAUTHORIZED, "Sign in first."));
    }
    let helpers = app.parallel_reads + 1;
    let term = r.term;
    let v = with_state(&app, "refresh", move |st| {
        let make = || -> Result<Box<dyn Backend>, String> {
            match &creds {
                Some(c) => build_live(c),
                None => Ok(build_demo()),
            }
        };
        let par: Option<HelperMaker> = if parallel_allowed(helpers, st.armed, st.parallel_off) { Some(&make) } else { None };
        fetch_refresh(st, &creds, demo, term.as_deref(), helpers, par)
    })
    .await?;
    Ok(Json(v))
}

// ------------------------------------------------------------------ other combinations of the enrolled courses

type HeldRead = (Value, u64);

/// The course code at the start of a swap candidate's name ("INFS 2201 Database Management Systems" -> "INFS 2201").
fn swap_code_of(label: &str) -> String {
    label.split_whitespace().take(2).collect::<Vec<_>>().join(" ")
}

/// Read a share of the enrolled courses' other combinations on one helper session (sign in once, then one swap read per course).
/// Reads only: a swap read opens the guide and looks at a course, it stages nothing.
fn read_held_on_helper(make: HelperMaker, share: &[(String, u64)]) -> Vec<(String, Result<HeldRead, String>)> {
    let mut be = match make() {
        Ok(b) => b,
        Err(e) => return share.iter().map(|(c, _)| (c.clone(), Err(e.clone()))).collect(),
    };
    share
        .iter()
        .map(|(code, row)| {
            let t = Instant::now();
            let r = be.tool("swap_options", &json!({ "candidate_row": row })).and_then(|v| {
                if v["sections"]["course"].as_str() == Some(code.as_str()) { Ok(v) } else { Err(format!("a different course is on swap row {row}")) }
            });
            let ms = t.elapsed().as_millis() as u64;
            log::event("parallel_held", json!({ "course": code, "ms": ms, "ok": r.is_ok(), "error": r.as_ref().err().map(|e| log::trunc(e, 200)) }));
            (code.clone(), r.map(|v| (v, ms)))
        })
        .collect()
}

/// The other combinations of the named enrolled courses (read through the swap pages). The main session reads one share and up to
/// `MAX_HELPERS` extra read-only sessions read the others at the same time; a course that fails on a helper is read again on the
/// main session, and parallel reads are switched off from then on. `workers` of 1 reads everything one after another.
pub fn fetch_held_options(st: &mut WriteState, creds: &Option<Creds>, demo: bool, codes: &[String], workers: usize, make: Option<HelperMaker>) -> Result<Value, ApiErr> {
    let t0 = Instant::now();
    let cands = step("held_options", "swap_candidates", || read_tool(st, creds, demo, "swap_candidates", &json!({})))?;
    let mut jobs: Vec<(String, u64)> = Vec::new();
    for code in codes {
        let hit = cands.as_array().and_then(|a| a.iter().find(|c| swap_code_of(c["course"].as_str().unwrap_or("")) == *code));
        if let Some(row) = hit.and_then(|c| c["row"].as_u64()) {
            jobs.push((code.clone(), row));
        }
    }
    let n = match make {
        Some(_) if workers >= 2 && jobs.len() >= 2 && !st.parallel_off => workers.min(MAX_HELPERS + 1).min(jobs.len()),
        _ => 1,
    };
    let mut shares: Vec<Vec<(String, u64)>> = vec![Vec::new(); n];
    for (i, j) in jobs.iter().enumerate() {
        shares[i % n].push(j.clone());
    }
    let relogins_before = st.relogins;
    let mut out: HashMap<String, (Result<HeldRead, String>, &'static str)> = HashMap::new();
    std::thread::scope(|sc| {
        let handles: Vec<_> = match make {
            Some(mk) if n > 1 => shares.iter().skip(1).map(|share| sc.spawn(move || read_held_on_helper(mk, share))).collect(),
            _ => Vec::new(),
        };
        for (code, row) in &shares[0] {
            let t = Instant::now();
            let r = step("held_options", &format!("main:{code}"), || read_tool(st, creds, demo, "swap_options", &json!({ "candidate_row": row }))).map_err(|e| e.1).and_then(|v| {
                if v["sections"]["course"].as_str() == Some(code.as_str()) { Ok(v) } else { Err(format!("a different course is on swap row {row}")) }
            });
            out.insert(code.clone(), (r.map(|v| (v, t.elapsed().as_millis() as u64)), "main"));
        }
        for h in handles {
            if let Ok(list) = h.join() {
                for (code, r) in list {
                    out.insert(code, (r, "parallel"));
                }
            }
        }
    });
    let helper_failed = out.values().any(|(r, via)| *via == "parallel" && r.is_err()) || (n > 1 && out.len() < jobs.len());
    if n > 1 && (helper_failed || st.relogins > relogins_before) {
        st.parallel_off = true;
        log::event("parallel_disabled", json!({ "helper_failed": helper_failed, "main_session_lost": st.relogins > relogins_before, "where": "held_options" }));
    }
    // Anything a helper could not read is read again, one after another, on the main session.
    for (code, row) in &jobs {
        if matches!(out.get(code), Some((Ok(_), _))) {
            continue;
        }
        let t = Instant::now();
        let r = step("held_options", &format!("retry:{code}"), || read_tool(st, creds, demo, "swap_options", &json!({ "candidate_row": row }))).map_err(|e| e.1).and_then(|v| {
            if v["sections"]["course"].as_str() == Some(code.as_str()) { Ok(v) } else { Err(format!("a different course is on swap row {row}")) }
        });
        out.insert(code.clone(), (r.map(|v| (v, t.elapsed().as_millis() as u64)), "retry"));
    }
    let mut courses = Vec::new();
    let mut failed = Vec::new();
    for code in codes {
        match out.remove(code) {
            Some((Ok((v, ms)), via)) => {
                let row = jobs.iter().find(|(c, _)| c == code).map(|(_, r)| *r);
                courses.push(json!({ "code": code, "row": row, "course_key": v["course_key"], "sections": v["sections"], "via": via, "ms": ms }));
            }
            Some((Err(e), _)) => failed.push(json!({ "code": code, "error": e })),
            None => failed.push(json!({ "code": code, "error": "not on the swap page" })),
        }
    }
    Ok(json!({ "courses": courses, "failed": failed, "stats": { "workers": n, "ms": t0.elapsed().as_millis() as u64, "parallel": n > 1 } }))
}

#[derive(Deserialize)]
pub struct HeldReq {
    codes: Vec<String>,
    /// false reads one after another (used to compare timings).
    #[serde(default)]
    parallel: Option<bool>,
}

pub async fn held_options(State(app): State<App>, Json(r): Json<HeldReq>) -> Result<Json<Value>, ApiErr> {
    if r.codes.is_empty() || r.codes.len() > 10 || r.codes.iter().any(|c| c.len() > 12 || !c.bytes().all(|b| b.is_ascii_alphanumeric() || b == b' ')) {
        return Err(err(StatusCode::BAD_REQUEST, "Pick between 1 and 10 course codes."));
    }
    let demo = app.demo;
    let creds = app.creds.lock().unwrap().clone();
    if !demo && creds.is_none() {
        return Err(err(StatusCode::UNAUTHORIZED, "Sign in first."));
    }
    let helpers = if r.parallel == Some(false) { 1 } else { app.parallel_reads + 1 };
    let codes = r.codes;
    let v = with_state(&app, "held_options", move |st| {
        let make = || -> Result<Box<dyn Backend>, String> {
            match &creds {
                Some(c) => build_live(c),
                None => Ok(build_demo()),
            }
        };
        let par: Option<HelperMaker> = if parallel_allowed(helpers, st.armed, st.parallel_off) { Some(&make) } else { None };
        fetch_held_options(st, &creds, demo, &codes, helpers, par)
    })
    .await?;
    Ok(Json(v))
}

pub async fn sections(app: &App, rows: Vec<u32>, known: HashMap<String, Value>) -> Result<Value, ApiErr> {
    let demo = app.demo;
    let creds = app.creds.lock().unwrap().clone();
    if !demo && creds.is_none() {
        return Err(err(StatusCode::UNAUTHORIZED, "Sign in first."));
    }
    let helpers = app.parallel_reads;
    with_state(app, "sections", move |st| {
        let make = || -> Result<Box<dyn Backend>, String> {
            match &creds {
                Some(c) => build_live(c),
                None => Ok(build_demo()),
            }
        };
        // Extra sessions are only for reading, and only while writes are off: opening a new sign-in is never done in the
        // middle of a write session.
        let par: Option<HelperMaker> = if parallel_allowed(helpers, st.armed, st.parallel_off) { Some(&make) } else { None };
        fetch_sections(st, &creds, demo, &rows, &known, helpers, par)
    })
    .await
}

// ------------------------------------------------------------------ swap several courses in one go

#[derive(Deserialize, Clone, Debug)]
pub struct SwapItem {
    /// The course code, as the swap page names it ("DACS 2202").
    pub course: String,
    #[serde(rename = "oldNbrs")]
    pub old_nbrs: Vec<String>,
    #[serde(rename = "newNbrs")]
    pub new_nbrs: Vec<String>,
    /// For the fresh seat read of the new combination (the same key and seats a swap preview needs).
    pub key: Value,
    #[serde(default)]
    pub seen: HashMap<String, Option<u32>>,
    /// Courses whose swap has to have worked first (this swap's new classes overlap what they hold today).
    #[serde(default, rename = "waitsFor")]
    pub waits_for: Vec<String>,
    /// What the page worked out about why this swap could be refused, shown next to PeopleSoft's own words.
    #[serde(default)]
    pub hint: String,
}

#[derive(Deserialize)]
pub struct SwapApplyReq {
    pub term: String,
    pub swaps: Vec<SwapItem>,
}

/// The course code at the start of a swap page's course text.
fn swap_code(text: &str) -> String {
    let mut it = text.split_whitespace();
    match (it.next(), it.next()) {
        (Some(a), Some(b)) => format!("{a} {b}"),
        _ => text.to_string(),
    }
}

/// Swap several enrolled courses to other combinations through ONE PeopleSoft swap: stage each course's new class on the swap's
/// select page, then press Continue once. PeopleSoft decides the whole set together. Afterwards every course is checked against
/// PeopleSoft's own enrolled list: the new classes must be there and the old ones gone.
/// The class numbers a swap page lists for a course ("Lecture - Class 3496 - Section 1 Laboratory - Class 3505 ...").
fn class_numbers_in(text: &str) -> Vec<String> {
    text.split("Class").skip(1).filter_map(|p| p.trim_start().split(|c: char| !c.is_ascii_digit()).next().filter(|d| !d.is_empty()).map(String::from)).collect()
}

fn swap_status(held: &BTreeSet<String>, sw: &SwapItem) -> (&'static str, &'static str) {
    let new_in = sw.new_nbrs.iter().all(|n| held.contains(n));
    let old_left = sw.old_nbrs.iter().filter(|n| !sw.new_nbrs.contains(n)).any(|n| held.contains(n));
    match (new_in, old_left) {
        (true, false) => ("swapped", ""),
        (false, false) => ("not_enrolled", "Neither the new nor the old class is on your enrolled list."),
        (false, true) => ("unchanged", "PeopleSoft kept your old combination."),
        (true, true) => ("both", "Both the old and the new classes are on your list."),
    }
}

/// How one round of swaps ends and what to check.
pub struct RoundOpts<'a> {
    /// Read PeopleSoft's enrolled list at the end (the last round). An earlier round skips that read: the next round's first request
    /// (the swap page, which lists what is held now) proves it for free.
    pub verify_list: bool,
    /// The previous round's swaps, to be proved from this round's first page.
    pub prior: &'a [SwapItem],
}

#[cfg(test)]
pub fn swap_apply(st: &mut WriteState, req: &SwapApplyReq, stop: &AtomicBool) -> Result<Value, ApiErr> {
    swap_round(st, req, stop, &RoundOpts { verify_list: true, prior: &[] })
}

/// Swap several enrolled courses through ONE PeopleSoft commit: stage each course's new class on the swap's select page, then press
/// Continue once. The caller decides which swaps share a commit (see `planSwapRounds` in the page): PeopleSoft judges each swap
/// against what is held at that moment, so swaps that overlap each other's current classes must not share one.
pub fn swap_round(st: &mut WriteState, req: &SwapApplyReq, stop: &AtomicBool, opts: &RoundOpts) -> Result<Value, ApiErr> {
    if req.swaps.is_empty() || req.swaps.len() > 6 {
        return Err(err(StatusCode::BAD_REQUEST, "Pick between 1 and 6 courses to swap."));
    }
    for sw in &req.swaps {
        if sw.old_nbrs.is_empty() || sw.new_nbrs.is_empty() || sw.old_nbrs.iter().chain(&sw.new_nbrs).any(|n| n.is_empty() || n.len() > 10 || !n.bytes().all(|b| b.is_ascii_digit())) {
            return Err(err(StatusCode::BAD_REQUEST, "Class numbers must be digits."));
        }
        if sorted(&sw.old_nbrs) == sorted(&sw.new_nbrs) {
            return Err(err(StatusCode::BAD_REQUEST, format!("{} would swap to the combination you already hold.", sw.course)));
        }
    }
    armed_backend(st)?;
    st.swap_checks = None;
    let t0 = Instant::now();
    let op = "swap_apply";
    let off = || err(StatusCode::CONFLICT, "Writes were switched off while this was running. Nothing further was sent.");
    // 1. Open the guide and read the candidates (a fresh guided session: anything staged before is forgotten).
    let cands = step(op, "open_guide", || armed_backend(st)?.tool("swap_candidates", &json!({})).map_err(|m| err(StatusCode::UNPROCESSABLE_ENTITY, m)))?;
    let cand_list = cands.as_array().cloned().unwrap_or_default();
    let cand_rows: Vec<(u32, String)> = cand_list.iter().filter_map(|c| Some((c["row"].as_u64()? as u32, swap_code(c["course"].as_str()?)))).collect();
    // The page that lists what is held now proves the previous round.
    let prior_results: Vec<Value> = opts
        .prior
        .iter()
        .map(|sw| {
            let held: BTreeSet<String> = cand_list.iter().filter(|c| c["course"].as_str().map(swap_code).as_deref() == Some(sw.course.as_str())).flat_map(|c| class_numbers_in(c["class_text"].as_str().unwrap_or(""))).collect();
            let (status, note) = if held.is_empty() { ("unknown", "PeopleSoft's swap page did not list this course afterwards.") } else { swap_status(&held, sw) };
            json!({ "course": sw.course, "status": status, "note": note })
        })
        .collect();
    // 2. Stage every course.
    let mut staged: Vec<String> = Vec::new();
    for sw in &req.swaps {
        if stop.load(Ordering::SeqCst) {
            return Err(off());
        }
        let row = cand_rows.iter().find(|(_, c)| *c == sw.course).map(|(r, _)| *r).ok_or_else(|| err(StatusCode::CONFLICT, format!("PeopleSoft does not offer {} for a swap right now. Nothing was sent.", sw.course)))?;
        let check = SeatCheck { course: sw.course.clone(), key: sw.key.clone(), class_nbrs: sw.new_nbrs.clone(), seen: sw.seen.clone(), waitlist: false };
        let args = json!({ "candidate_row": row, "new_class_nbrs": sw.new_nbrs });
        let prev = step(op, "preview_stage", || preview_checked(st, "preview_swap_stage", &args, Some(&sw.new_nbrs), Some(&check)))?;
        let token = prev["token"].as_str().ok_or_else(|| err(StatusCode::CONFLICT, "no preview was issued"))?.to_string();
        if stop.load(Ordering::SeqCst) {
            return Err(off());
        }
        let _ = step(op, "stage_confirm", || confirm_token(st, &token, None, false, stop))?;
        staged.push(sw.course.clone());
    }
    // 3. One commit for all of them.
    if stop.load(Ordering::SeqCst) {
        return Err(off());
    }
    let commit = step(op, "preview_commit", || preview_checked(st, "preview_swap_commit", &json!({}), None, None))?;
    let token = commit["token"].as_str().ok_or_else(|| err(StatusCode::CONFLICT, "no preview was issued"))?.to_string();
    let out = step(op, "commit_confirm", || confirm_token(st, &token, None, false, stop))?;
    // PeopleSoft's own per-course answer from the result page ("Enrollment not successful", ...), matched by the class given up.
    let rows: Vec<Value> = out["detail"]["results"].as_array().cloned().unwrap_or_default();
    let said_for = |sw: &SwapItem| -> String {
        rows.iter().find(|r| r["class_nbr"].as_str().is_some_and(|n| sw.old_nbrs.iter().any(|o| o == n))).and_then(|r| r["message"].as_str()).unwrap_or("").to_string()
    };
    let said: Vec<String> = rows.iter().filter_map(|r| r["message"].as_str().map(String::from)).collect();
    // 4. The proof: PeopleSoft's own enrolled list for the term, for the last round. An earlier round is proved by the next round's page.
    let held: Option<BTreeSet<String>> = if opts.verify_list {
        let be = armed_backend(st)?;
        let terms = be.tool("drop_terms", &json!({})).map_err(|m| err(StatusCode::UNPROCESSABLE_ENTITY, m))?;
        let term_row = terms.as_array().and_then(|a| a.iter().find(|t| t["term"].as_str().map(|x| x.eq_ignore_ascii_case(&req.term)).unwrap_or(false))).and_then(|t| t["row"].as_u64()).map(|r| r as u32);
        term_row.and_then(|r| be.tool("enrolled_classes", &json!({ "row": r })).ok()).and_then(|l| listed_numbers(&l))
    } else {
        None
    };
    let results: Vec<Value> = req
        .swaps
        .iter()
        .map(|sw| {
            let (status, note) = match (&held, opts.verify_list) {
                (_, false) => ("sent", "Sent. The next round's first page shows whether it took."),
                (None, true) => ("unknown", "PeopleSoft's enrolled list could not be read afterwards."),
                (Some(h), true) => swap_status(h, sw),
            };
            json!({ "course": sw.course, "status": status, "note": note, "said": said_for(sw) })
        })
        .collect();
    log::event("swap_apply_done", json!({ "ms": log::ms_since(t0), "courses": staged, "results": results.iter().map(|r| r["status"].clone()).collect::<Vec<_>>() }));
    Ok(json!({ "results": results, "prior": prior_results, "said": said, "ms": log::ms_since(t0) }))
}

// ------------------------------------------------------------------ apply a whole plan: drops, then swaps, then enrols

/// The latest job's progress, for the page to poll: `{id, phase, finished, error, items: [{id, kind, course, status, note}]}`.
/// An item's status is `pending`, `working`, `done` or `error`.
static JOB: std::sync::Mutex<Option<Value>> = std::sync::Mutex::new(None);

fn job_update(f: impl FnOnce(&mut Value)) {
    if let Ok(mut j) = JOB.lock() {
        if let Some(v) = j.as_mut() {
            f(v);
        }
    }
}

fn job_item(id: &str, status: &str, note: &str) {
    job_update(|j| {
        if let Some(a) = j["items"].as_array_mut() {
            if let Some(it) = a.iter_mut().find(|x| x["id"] == id) {
                it["status"] = json!(status);
                it["note"] = json!(note);
            }
        }
    });
}

/// The first of these courses whose swap or drop in this run did not work (a course is either swapped or dropped, never both).
fn failed_step(courses: &[String]) -> Option<(String, &'static str)> {
    courses.iter().find_map(|c| {
        if job_status(&format!("swap:{c}")) == "error" {
            Some((c.clone(), "swap"))
        } else if job_status(&format!("drop:{c}")) == "error" {
            Some((c.clone(), "drop"))
        } else {
            None
        }
    })
}

fn job_status(id: &str) -> String {
    JOB.lock().ok().and_then(|j| j.as_ref().and_then(|v| v["items"].as_array().and_then(|a| a.iter().find(|x| x["id"] == id).and_then(|x| x["status"].as_str().map(String::from))))).unwrap_or_default()
}

#[derive(Deserialize, Clone, Debug)]
pub struct ApplyDrop {
    pub course: String,
    #[serde(rename = "classNbr")]
    pub class_nbr: String,
}

#[derive(Deserialize, Clone, Debug)]
pub struct ApplyEnrol {
    pub pick: Pick,
    /// Courses whose swap must have worked first (the new classes overlap the combination that swap gives up).
    #[serde(default)]
    pub depends: Vec<String>,
    /// A course that was swapped away from in a loop: it is dropped first and enrolled again, so it is only staged once the drops are done.
    #[serde(default, rename = "afterDrop")]
    pub after_drop: bool,
    /// A course that is not planned in PeopleSoft yet: it is planned first (see `ApplyPlan`), and only staged once that worked.
    #[serde(default, rename = "afterPlan")]
    pub after_plan: bool,
}

/// One course to plan into the open term before it is enrolled (course planning only, the same write as the degree board's Save).
#[derive(Deserialize, Clone, Debug)]
pub struct ApplyPlan {
    pub course: String,
    pub year: u32,
    pub index: u32,
    #[serde(rename = "termCode")]
    pub term_code: String,
    /// The credit check of the destination term after this plan, as the page counted it.
    pub credits: CreditsReq,
}

/// The credit check the page made. PeopleSoft refuses at the Enrol click when the term's planned plus enrolled credits pass the limit, and
/// our enrol presses that click to open the class list, so an apply that enrols must not start over the limit.
#[derive(Deserialize, Clone, Debug)]
pub struct CreditsReq {
    pub term: String,
    pub limit: f64,
    pub after: f64,
    #[serde(default)]
    pub unknown: Vec<String>,
}

/// The most credits PeopleSoft allows in a term: 18 in Fall and Winter (confirmed by its refusal), 12 in Spring (believed, not yet seen).
pub fn credit_limit(term: &str) -> f64 {
    if term.to_ascii_lowercase().contains("spring") { 12.0 } else { 18.0 }
}

/// Refuse an apply that enrols without a credit check, with a limit the server does not agree with, with unknown credits, or over the limit.
pub fn check_credits(req: &ApplyReq) -> Result<(), String> {
    if req.enrols.is_empty() {
        return Ok(());
    }
    let Some(c) = &req.credits else {
        return Err("This page is out of date: it did not check the credit limit. Reload it and try again.".into());
    };
    let limit = credit_limit(&req.term);
    if c.term != req.term || (c.limit - limit).abs() > 1e-9 {
        return Err(format!("The page and the server disagree about the credit limit for {} ({} against {}). Reload the page.", req.term, c.limit, limit));
    }
    if !c.unknown.is_empty() {
        return Err(format!("The credits of {} are unknown, so the term's total cannot be trusted. Nothing was sent.", c.unknown.join(", ")));
    }
    if !(c.after.is_finite() && c.after >= 0.0) {
        return Err("The credit total is not a number. Nothing was sent.".into());
    }
    if c.after > limit + 1e-9 {
        return Err(format!("{} credits is over the {} credits PeopleSoft allows in {}. PeopleSoft would refuse the enrolment, so nothing was sent.", c.after, limit, req.term));
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct ApplyReq {
    pub term: String,
    /// The term's row on PeopleSoft's drop page. `null` (or absent) means the page does not know it, which is only possible when nothing is
    /// enrolled in the term; an apply that drops is then refused (before, a missing row silently became row 0, the first term).
    #[serde(default, rename = "termRow", deserialize_with = "optional_row")]
    pub term_row: Option<u32>,
    #[serde(default)]
    pub drops: Vec<ApplyDrop>,
    /// All swaps in one commit (kept for callers that do not plan rounds).
    #[serde(default)]
    pub swaps: Vec<SwapItem>,
    /// Swaps in rounds: each round is one PeopleSoft commit, in order.
    #[serde(default)]
    pub rounds: Vec<Vec<SwapItem>>,
    #[serde(default)]
    pub enrols: Vec<ApplyEnrol>,
    /// Courses planned into the term first, then enrolled (each has one enrolment with `afterPlan`).
    #[serde(default)]
    pub plans: Vec<ApplyPlan>,
    #[serde(default)]
    pub credits: Option<CreditsReq>,
}

/// Every step an enrolment or a swap waits for must be part of this very request. A page that unticked a drop used to leave the enrolments
/// that overlapped it in the request: `failed_step` only stops on a step that FAILED, and a step that was never sent has no status, so the
/// enrolment went ahead and could overlap a class the student still held.
pub fn check_dependencies(req: &ApplyReq) -> Result<(), String> {
    let mut known: BTreeSet<&str> = req.drops.iter().map(|d| d.course.as_str()).collect();
    let rounds = req.all_rounds();
    for sw in rounds.iter().flatten() {
        known.insert(sw.course.as_str());
    }
    for e in &req.enrols {
        if let Some(d) = e.depends.iter().find(|d| !known.contains(d.as_str())) {
            return Err(format!("{} waits for {d}, which is not part of this Apply, so it was not sent. Nothing was sent.", e.pick.course));
        }
    }
    for sw in rounds.iter().flatten() {
        if let Some(d) = sw.waits_for.iter().find(|d| !known.contains(d.as_str())) {
            return Err(format!("The swap of {} waits for {d}, which is not part of this Apply. Nothing was sent.", sw.course));
        }
    }
    Ok(())
}

/// The plans and the enrolments that wait for them must match one to one, and every plan must carry its own credit check.
pub fn check_plans(req: &ApplyReq) -> Result<(), String> {
    if req.plans.len() > 6 {
        return Err("Too many courses to plan in one go.".into());
    }
    let mut seen = BTreeSet::new();
    for p in &req.plans {
        if !seen.insert(p.course.clone()) {
            return Err(format!("{} is planned twice.", p.course));
        }
        if p.term_code.is_empty() || p.term_code.len() > 6 || !p.term_code.bytes().all(|b| b.is_ascii_digit()) {
            return Err("A term code must be digits.".into());
        }
        if !req.enrols.iter().any(|e| e.after_plan && e.pick.course == p.course) {
            return Err(format!("{} is planned but not enrolled; Apply plans a course only to enrol it.", p.course));
        }
        check_plan_request(&json!({"year": p.year, "index": p.index, "term_code": p.term_code}), Some(&p.credits))?;
        if p.credits.term != req.term {
            return Err(format!("{} would be planned into {}, not the term being applied ({}).", p.course, p.credits.term, req.term));
        }
    }
    if let Some(e) = req.enrols.iter().find(|e| e.after_plan && !req.plans.iter().any(|p| p.course == e.pick.course)) {
        return Err(format!("{} waits for a plan step that was not sent.", e.pick.course));
    }
    Ok(())
}

impl ApplyReq {
    fn all_rounds(&self) -> Vec<Vec<SwapItem>> {
        if !self.rounds.is_empty() {
            self.rounds.clone()
        } else if !self.swaps.is_empty() {
            vec![self.swaps.clone()]
        } else {
            vec![]
        }
    }
}

/// Drops, one session per course at once (the same path as the parallel drop), each checked against that session's own list.
fn par_drops(creds: &Creds, term_row: u32, nums: &[String], stop: &AtomicBool) -> Vec<Value> {
    std::thread::scope(|sc| {
        let hs: Vec<_> = nums
            .iter()
            .enumerate()
            .map(|(i, n)| {
                sc.spawn(move || {
                    let be = match build_live(creds) {
                        Ok(b) => b,
                        Err(m) => return json!({ "status": "failed", "note": log::trunc(&m, 200) }),
                    };
                    std::thread::sleep(std::time::Duration::from_millis(800 * i as u64));
                    let mut hst = lab_state(be);
                    let mut out = json!({});
                    for _ in 0..2 {
                        out = match drop_bulk(&mut hst, term_row, std::slice::from_ref(n), stop, false, None) {
                            Ok(v) => json!({ "status": v["results"][0]["status"], "note": v["results"][0]["note"] }),
                            Err(e) => json!({ "status": "failed", "note": e.1 }),
                        };
                        let retry = out["status"] == "failed" && out["note"].as_str().is_some_and(|x| x.contains("not this term's enrolled list"));
                        if !retry || stop.load(Ordering::SeqCst) {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(1500));
                    }
                    out
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap_or_else(|_| json!({ "status": "failed", "note": "panicked" }))).collect()
    })
}

fn apply_run(st: &mut WriteState, creds: &Creds, req: &ApplyReq, stop: &std::sync::Arc<AtomicBool>) -> Result<(), ApiErr> {
    armed_backend(st)?;
    let t0 = Instant::now();
    let halted = || stop.load(Ordering::SeqCst);
    let rounds = req.all_rounds();
    // The enrolments are staged in the background from the first second (staging does not depend on the swaps, only the submit does),
    // except a course that is dropped and enrolled again: it can only be staged once the drops are done.
    // When courses are planned first, NOTHING is staged in the background: the plan is saved from this session and must not meet
    // enrolment choices another session has just made on the same Plan & Enrol page. Everything waits for the plan steps.
    let planning = !req.plans.is_empty();
    let early: Vec<usize> = req.enrols.iter().enumerate().filter(|(_, e)| !e.after_drop && !e.after_plan && !planning).map(|(i, _)| i).collect();
    let late: Vec<usize> = req.enrols.iter().enumerate().filter(|(_, e)| e.after_drop || e.after_plan || planning).map(|(i, _)| i).collect();
    let spawn_stage = |idx: &[usize]| {
        if idx.is_empty() {
            return None;
        }
        for &i in idx {
            job_item(&format!("enrol:{}", req.enrols[i].pick.course), "working", "staging in the background");
        }
        let picks: Vec<Pick> = idx.iter().map(|&i| req.enrols[i].pick.clone()).collect();
        let (c, sp) = (creds.clone(), stop.clone());
        Some(std::thread::spawn(move || prestage_run(&c, &picks, &sp)))
    };
    let stage_early = spawn_stage(&early);
    for &i in &late {
        let why = if req.enrols[i].after_plan { "waits for its plan step" } else if planning { "waits for the plan steps" } else { "waits for its drop" };
        job_item(&format!("enrol:{}", req.enrols[i].pick.course), "pending", why);
    }

    // 1. Drops.
    if !req.drops.is_empty() {
        job_update(|j| j["phase"] = json!("dropping"));
        for d in &req.drops {
            job_item(&format!("drop:{}", d.course), "working", "");
        }
        let nums: Vec<String> = req.drops.iter().map(|d| d.class_nbr.clone()).collect();
        let res = par_drops(creds, req.term_row.unwrap_or(0), &nums, stop);
        for (d, r) in req.drops.iter().zip(res) {
            let ok = matches!(r["status"].as_str(), Some("dropped") | Some("gone"));
            job_item(&format!("drop:{}", d.course), if ok { "done" } else { "error" }, if ok { "Dropped. PeopleSoft's list no longer shows it." } else { r["note"].as_str().unwrap_or("not dropped") });
        }
    }
    // 1b. Plans: courses that PeopleSoft does not have planned yet are planned into the term (after the drops, so the freed credits count),
    // one at a time, each as a preview and a confirm with every credit check; then their enrolments can be staged.
    if planning {
        job_update(|j| j["phase"] = json!("planning"));
        for p in &req.plans {
            let id = format!("plan:{}", p.course);
            if halted() {
                job_item(&id, "error", "Stopped before it was sent.");
                continue;
            }
            job_item(&id, "working", "");
            let args = json!({ "year": p.year, "index": p.index, "term_code": p.term_code });
            let outcome = plan_preview(st, &args, &p.credits, &p.course).and_then(|v| {
                let (tok, phrase) = match (v["token"].as_str(), v["plan"]["required_phrase"].as_str()) {
                    (Some(t), Some(ph)) => (t.to_string(), ph.to_string()),
                    _ => return Err(err(StatusCode::CONFLICT, "no preview token was issued.")),
                };
                let be = armed_backend(st)?;
                be.confirm(&tok, &phrase).map_err(|f| err(StatusCode::CONFLICT, f.text()))
            });
            match outcome {
                Ok(_) => job_item(&id, "done", "Planned in PeopleSoft; its plan was read back and matches."),
                Err(e) => job_item(&id, "error", &format!("{} Nothing was enrolled for it.", e.1)),
            }
        }
    }
    // The drop-and-enrol-again courses and the newly planned ones are staged now, while the swaps run (a step that failed leaves its enrolment unsent).
    let late_ok: Vec<usize> = late
        .iter()
        .copied()
        .filter(|&i| {
            let c = &req.enrols[i].pick.course;
            (req.drops.iter().all(|d| d.course != *c) || job_status(&format!("drop:{c}")) == "done") && (!req.enrols[i].after_plan || job_status(&format!("plan:{c}")) == "done")
        })
        .collect();
    for &i in &late {
        if !late_ok.contains(&i) {
            let e = &req.enrols[i];
            let why = if e.after_plan && job_status(&format!("plan:{}", e.pick.course)) != "done" { "Not sent: its plan step did not work, so PeopleSoft does not have it planned." } else { "Not sent: its drop did not work, so it is still enrolled." };
            job_item(&format!("enrol:{}", e.pick.course), "error", why);
        }
    }
    let stage_late = spawn_stage(&late_ok);

    // 2. Swaps, round by round. A round that needs another swap to have worked is skipped when that one did not.
    let mut previous: Vec<SwapItem> = Vec::new();
    let last = rounds.len().saturating_sub(1);
    for (ri, round) in rounds.iter().enumerate() {
        job_update(|j| j["phase"] = json!(if rounds.len() > 1 { format!("swapping, round {} of {}", ri + 1, rounds.len()) } else { "swapping".to_string() }));
        let mut go: Vec<SwapItem> = Vec::new();
        for sw in round {
            let id = format!("swap:{}", sw.course);
            if let Some((w, what)) = failed_step(&sw.waits_for) {
                job_item(&id, "error", &format!("Not sent: it has to wait for the {what} of {w}, which did not work."));
            } else {
                job_item(&id, "working", &if rounds.len() > 1 { format!("round {}", ri + 1) } else { String::new() });
                go.push(sw.clone());
            }
        }
        if go.is_empty() {
            continue;
        }
        if halted() {
            for sw in &go {
                job_item(&format!("swap:{}", sw.course), "error", "Stopped before it was sent.");
            }
            continue;
        }
        let r = swap_round(st, &SwapApplyReq { term: req.term.clone(), swaps: go.clone() }, stop, &RoundOpts { verify_list: ri == last, prior: &previous });
        match r {
            Ok(v) => {
                // The previous round, proved by this round's first page.
                for pr in v["prior"].as_array().cloned().unwrap_or_default() {
                    let course = pr["course"].as_str().unwrap_or("");
                    let ok = pr["status"] == "swapped";
                    let hint = previous.iter().find(|x| x.course == course).map(|x| x.hint.clone()).unwrap_or_default();
                    job_item(&format!("swap:{course}"), if ok { "done" } else { "error" }, &if ok { "Swapped. PeopleSoft's list shows the new classes.".to_string() } else { format!("PeopleSoft kept your old combination. {hint}") });
                }
                for res in v["results"].as_array().cloned().unwrap_or_default() {
                    let course = res["course"].as_str().unwrap_or("");
                    match res["status"].as_str() {
                        Some("sent") => job_item(&format!("swap:{course}"), "working", "Sent; checked when the next step opens its page."),
                        Some("swapped") => job_item(&format!("swap:{course}"), "done", "Swapped. PeopleSoft's list shows the new classes."),
                        _ => {
                            let hint = go.iter().find(|x| x.course == course).map(|x| x.hint.clone()).unwrap_or_default();
                            let said = res["said"].as_str().unwrap_or("");
                            let mut note = String::from("PeopleSoft kept your old combination.");
                            if !said.is_empty() {
                                note = format!("PeopleSoft said: \u{201c}{said}\u{201d}. Your old combination is unchanged.");
                            }
                            if !hint.is_empty() {
                                note.push(' ');
                                note.push_str(&hint);
                            }
                            job_item(&format!("swap:{course}"), "error", &note);
                        }
                    }
                }
                previous = if ri == last { vec![] } else { go };
            }
            Err(e) => {
                for sw in &go {
                    job_item(&format!("swap:{}", sw.course), "error", &e.1);
                }
                previous = vec![];
            }
        }
    }

    // 3. The enrolments: whatever does not depend on a failed swap.
    let mut staged_all: Vec<(usize, Value, Option<Held>)> = Vec::new();
    for (handle, idx) in [(stage_early, &early), (stage_late, &late_ok)] {
        if let Some(h) = handle {
            for (i, (r, held)) in idx.iter().zip(h.join().unwrap_or_default()) {
                staged_all.push((*i, r, held));
            }
        }
    }
    if !staged_all.is_empty() {
        job_update(|j| j["phase"] = json!("enrolling"));
        let mut held: Vec<Held> = Vec::new();
        for (i, r, h) in staged_all {
            let e = &req.enrols[i];
            let id = format!("enrol:{}", e.pick.course);
            let failed_dep = failed_step(&e.depends);
            let dropped = req.drops.iter().any(|d| d.course == e.pick.course) && job_status(&format!("drop:{}", e.pick.course)) == "done";
            let after = if dropped { " Its drop already happened, so you are not enrolled in this course now." } else { "" };
            match (h, r["status"].as_str(), failed_dep) {
                (_, _, Some((d, what))) => job_item(&id, "error", &format!("Not sent: it needs the {what} of {d}, which did not work.{after}")),
                (Some(held_one), Some("staged"), None) => {
                    if halted() {
                        job_item(&id, "error", "Stopped before it was sent.");
                    } else {
                        held.push(held_one);
                    }
                }
                (_, Some("blocked"), None) => job_item(&id, "error", &format!("Not sent: other courses are already chosen in PeopleSoft's Plan & Enrol list.{after}")),
                _ => job_item(&id, "error", &format!("{}{after}", r["detail"].as_str().unwrap_or("could not be prepared"))),
            }
        }
        if !held.is_empty() {
            if let Ok(mut f) = FIRED.lock() {
                f.clear();
                for h in &held {
                    f.push(json!({ "course": h.0, "status": "sending", "verifying": true }));
                }
            }
            let names: Vec<String> = held.iter().map(|h| h.0.clone()).collect();
            fire_spawn(held, stop);
            // Follow every session until its class-list check is in (at most five minutes).
            let deadline = Instant::now() + Duration::from_secs(300);
            loop {
                let snap = FIRED.lock().map(|f| f.clone()).unwrap_or_default();
                let mut all_done = true;
                for (i, c) in names.iter().enumerate() {
                    let r = snap.get(i).cloned().unwrap_or(json!({}));
                    let id = format!("enrol:{c}");
                    if r["verifying"] == true {
                        all_done = false;
                        let note = match r["status"].as_str() {
                            Some("submitted") => "PeopleSoft says success; checking your class list",
                            Some("rejected") => "PeopleSoft refused it; checking your class list",
                            _ => "sent, waiting for PeopleSoft",
                        };
                        job_item(&id, "working", note);
                    } else if r["status"] == "enrolled" {
                        job_item(&id, "done", "Enrolled. PeopleSoft's own list shows it.");
                    } else if r["status"] == "waitlisted" {
                        job_item(&id, "done", "On the waitlist. You are NOT enrolled: PeopleSoft may enrol you later without asking.");
                    } else {
                        let again = if req.drops.iter().any(|d| d.course == *c) { " It was dropped first, so you are not enrolled in this course now: enrol it again." } else { "" };
                        let why = match r["status"].as_str() {
                            Some("rejected") | Some("not_enrolled") => format!("PeopleSoft did not enrol it (a clash or a rule).{again}"),
                            Some("unknown") => "Unclear: check PeopleSoft before retrying.".to_string(),
                            _ => format!("{}{again}", r["detail"].as_str().unwrap_or("failed")),
                        };
                        job_item(&id, "error", &why);
                    }
                }
                if all_done || Instant::now() > deadline {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        }
    }
    log::event("apply_done", json!({ "ms": log::ms_since(t0), "drops": req.drops.len(), "rounds": rounds.len(), "enrols": req.enrols.len() }));
    Ok(())
}

/// The progress record of a run that is about to start: one pending item per drop, swap and enrolment.
fn new_job(req: &ApplyReq) -> Value {
    let mut items: Vec<Value> = vec![];
    for d in &req.drops {
        items.push(json!({ "id": format!("drop:{}", d.course), "kind": "drop", "course": d.course, "status": "pending", "note": "" }));
    }
    for (ri, round) in req.all_rounds().iter().enumerate() {
        for sw in round {
            items.push(json!({ "id": format!("swap:{}", sw.course), "kind": "swap", "course": sw.course, "status": "pending", "note": "", "round": ri + 1 }));
        }
    }
    for p in &req.plans {
        items.push(json!({ "id": format!("plan:{}", p.course), "kind": "plan", "course": p.course, "status": "pending", "note": "" }));
    }
    for e in &req.enrols {
        items.push(json!({ "id": format!("enrol:{}", e.pick.course), "kind": "enrol", "course": e.pick.course, "status": "pending", "note": "" }));
    }
    json!({ "id": log::now_ms(), "phase": "starting", "finished": false, "error": null, "items": items })
}

/// The run is over: nothing may stay pending or working. After an error every unfinished item carries it; after a normal end an item
/// that never got its answer (for example an enrolment still waiting after the five minutes) says so instead of spinning for ever.
fn settle_job(r: &Result<(), ApiErr>) {
    job_update(|j| {
        j["finished"] = json!(true);
        j["phase"] = json!("finished");
        let why = match r {
            Err(e) => {
                j["error"] = json!(e.1);
                e.1.clone()
            }
            Ok(()) => "No answer arrived in time. Unclear: check PeopleSoft before retrying.".to_string(),
        };
        if let Some(a) = j["items"].as_array_mut() {
            for it in a.iter_mut().filter(|x| x["status"] == "pending" || x["status"] == "working") {
                it["status"] = json!("error");
                it["note"] = json!(why);
            }
        }
    });
}

pub async fn apply_start(State(app): State<App>, Json(req): Json<ApplyReq>) -> Result<Json<Value>, ApiErr> {
    let swap_count: usize = req.all_rounds().iter().map(|r| r.len()).sum();
    if req.drops.len() > 8 || swap_count > 6 || req.all_rounds().iter().any(|r| r.len() > 6) || req.enrols.len() > PARALLEL_MAX || req.drops.len() + swap_count + req.enrols.len() == 0 {
        return Err(err(StatusCode::BAD_REQUEST, "Nothing to do, or too much for one go."));
    }
    if let Err(m) = check_credits(&req) {
        return Err(err(StatusCode::UNPROCESSABLE_ENTITY, m));
    }
    if let Err(m) = check_plans(&req) {
        return Err(err(StatusCode::UNPROCESSABLE_ENTITY, m));
    }
    if let Err(m) = check_dependencies(&req) {
        return Err(err(StatusCode::UNPROCESSABLE_ENTITY, m));
    }
    if app.confirm_phrase {
        // Apply confirms every step itself; there is nowhere to type a phrase. Better refused than silently one-click.
        return Err(err(StatusCode::CONFLICT, "This server was started to ask for a typed phrase on every write, which Apply does not support. Start it without PLANNER_CONFIRM=phrase."));
    }
    if !req.drops.is_empty() && req.term_row.is_none() {
        return Err(err(StatusCode::UNPROCESSABLE_ENTITY, "The page did not say which term the drops are in. Nothing was sent. Reload the page."));
    }
    {
        // A course is dropped as a whole, by one class number: never class by class.
        let mut seen = BTreeSet::new();
        if !req.drops.iter().all(|d| seen.insert(d.course.clone())) {
            return Err(err(StatusCode::BAD_REQUEST, "Drop a course once, as a whole, never class by class."));
        }
    }
    if req.drops.iter().any(|d| d.class_nbr.is_empty() || d.class_nbr.len() > 10 || !d.class_nbr.bytes().all(|b| b.is_ascii_digit())) {
        return Err(err(StatusCode::BAD_REQUEST, "Class numbers must be digits."));
    }
    let creds = app.creds.lock().unwrap().clone().ok_or_else(|| err(StatusCode::UNAUTHORIZED, "Sign in first."))?;
    {
        let mut j = JOB.lock().map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "internal error"))?;
        if j.as_ref().is_some_and(|v| v["finished"] == false) {
            return Err(err(StatusCode::CONFLICT, "A plan is already running. Wait for it to finish."));
        }
        *j = Some(new_job(&req));
    }
    let stop = app.stop.clone();
    let a = app.clone();
    let demo = app.demo;
    tokio::spawn(async move {
        let r = with_state(&a, "apply", move |st| {
            // Right before anything is sent: PeopleSoft's own plan is read again and the credit total recounted (not in demo mode).
            if !demo {
                apply_recount(st, &req)?;
            }
            apply_run(st, &creds, &req, &stop)
        })
        .await;
        settle_job(&r);
        // The run is over, whatever happened: writes go off again here, not only when the page remembers to ask.
        disarm_now(&a);
    });
    Ok(Json(json!({ "started": true })))
}

pub async fn apply_progress() -> Json<Value> {
    Json(JOB.lock().ok().and_then(|j| j.clone()).unwrap_or(json!({ "finished": true, "items": [] })))
}

// ------------------------------------------------------------------ lab: what does PeopleSoft accept in parallel?

fn lab_state(be: Box<dyn Backend>) -> WriteState {
    let mut be = be;
    be.arm();
    WriteState { backend: Some(be), armed: true, last_used: Some(Instant::now()), ..Default::default() }
}

// ------------------------------------------------------------------ parallel enrol and drop (one session per course)

/// Measured on the real site on 2026-11-16 (see README): extra sign-ins of the same account work at the same time and leave the
/// main session alone; each session has its own Plan & Enrol list, so a course staged on one is not submitted by another;
/// concurrent drops and enrolments of different courses all took effect. The cap is deliberately low.
pub const PARALLEL_MAX: usize = 5;

fn par_enrol_status(r: &Value) -> (&'static str, Value) {
    let res = &r["result"];
    if r["login_ok"] == false {
        return ("failed", json!(r["error"]));
    }
    if let Some(b) = res.get("blocked") {
        return ("blocked", json!({ "alreadyChosen": b }));
    }
    if let Some(e) = res["error"].as_str().or(res["submit_error"].as_str()) {
        return ("failed", json!(e));
    }
    if res["unknown"] == true {
        return ("unknown", json!(res["verify"]));
    }
    match res["verify"]["rows"].as_array().and_then(|a| a.first()) {
        Some(row) if row["enrolled"] == true => ("enrolled", json!(row["status"])),
        Some(row) if row["waitlisted"] == true => ("waitlisted", json!(row["status"])),
        Some(row) => ("not_enrolled", json!(row["status"])),
        None => ("unknown", json!(res["verify"])),
    }
}

// ------------------------------------------------------------------ stage ahead, then fire

/// Sessions that already staged their course and hold a submit preview, ready to be sent at once. Cleared whenever writes
/// are switched off or the person signs out, and nothing older than `PRESTAGE_TTL` is sent.
const PRESTAGE_TTL: Duration = Duration::from_secs(10 * 60);
static PRESTAGED: std::sync::Mutex<Vec<(String, String, Instant, WriteState)>> = std::sync::Mutex::new(Vec::new());

pub fn prestaged_clear() {
    if let Ok(mut p) = PRESTAGED.lock() {
        p.clear();
    }
}

type Held = (String, String, Instant, WriteState);

/// One signed-in session per pick, each staged up to the submit preview. Returns the per-course answer and the held session.
fn prestage_run(creds: &Creds, picks: &[Pick], stop: &AtomicBool) -> Vec<(Value, Option<Held>)> {
    std::thread::scope(|sc| {
            let hs: Vec<_> = picks
                .iter()
                .enumerate()
                .map(|(i, pick)| {
                    let (creds, stop) = (&creds, &stop);
                    sc.spawn(move || {
                        let t = Instant::now();
                        let be = match build_live(creds) {
                            Ok(b) => b,
                            Err(m) => return (json!({ "course": pick.course, "status": "failed", "detail": log::trunc(&m, 200) }), None),
                        };
                        std::thread::sleep(std::time::Duration::from_millis(800 * i as u64));
                        let mut hst = lab_state(be);
                        match prepare_enrol_stop(&mut hst, std::slice::from_ref(pick), stop) {
                            Ok(v) if v["blocked"] == "leftovers" => (json!({ "course": pick.course, "status": "blocked", "detail": { "alreadyChosen": v["leftovers"] }, "ms": log::ms_since(t) }), None),
                            Ok(v) => {
                                let tok = v["submit"]["token"].as_str().unwrap_or("").to_string();
                                (json!({ "course": pick.course, "status": "staged", "ms": log::ms_since(t) }), Some((pick.course.clone(), tok, Instant::now(), hst)))
                            }
                            Err(e) => (json!({ "course": pick.course, "status": "failed", "detail": e.1, "ms": log::ms_since(t) }), None),
                        }
                    })
                })
                .collect();
            hs.into_iter().map(|h| h.join().unwrap_or_else(|_| (json!({ "status": "failed", "detail": "panicked" }), None))).collect()
        })
}

/// What the last fire reported, per course, and whether the class-list checks after it are still running.
static FIRED: std::sync::Mutex<Vec<Value>> = std::sync::Mutex::new(Vec::new());

/// PeopleSoft's own confirmation page after a submit lists the course with an icon: `success` or `error`.
fn confirmation_status(detail: &Value, course: &str) -> &'static str {
    let icons: Vec<&str> = detail["icons"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default();
    let lines: Vec<&str> = detail["lines"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default();
    let code = course.trim();
    match lines.iter().position(|l| l.starts_with(code)) {
        Some(i) => match icons.get(i) {
            Some(&"success") => "submitted",
            Some(_) => "rejected",
            None => "unknown",
        },
        None => "unknown",
    }
}

/// Start the submit of every held session, each on its own thread, and return at once. Answers land in `FIRED`.
fn fire_spawn(held: Vec<Held>, stop: &std::sync::Arc<AtomicBool>) {
        for (idx, (course, tok, at, mut hst)) in held.into_iter().enumerate() {
            let stop = stop.clone();
            if let Ok(mut f) = FIRED.lock() {
                f.push(json!({ "course": course, "status": "sending", "verifying": true }));
            }
            std::thread::spawn(move || {
                let set = |v: Value| {
                    if let Ok(mut f) = FIRED.lock() {
                        if let Some(slot) = f.get_mut(idx) {
                            *slot = v;
                        }
                    }
                };
                if at.elapsed() > PRESTAGE_TTL {
                    set(json!({ "course": course, "status": "failed", "detail": "Staged too long ago; stage again.", "verifying": false }));
                    return;
                }
                let t = Instant::now();
                hst.defer_verify = true;
                match confirm_token(&mut hst, &tok, None, false, &stop) {
                    Ok(o) => {
                        let status = confirmation_status(&o["detail"], &course);
                        set(json!({ "course": course, "status": status, "detail": "PeopleSoft's confirmation page", "ms": log::ms_since(t), "verifying": true }));
                        // Now the proof: this session's own class list.
                        let checks = hst.pending_verify.take().unwrap_or_default();
                        let verify = match hst.backend.as_mut() {
                            Some(be) => verify_enrolled(be.as_mut(), &checks),
                            None => json!({ "error": "session gone" }),
                        };
                        let r = json!({ "login_ok": true, "result": { "unknown": o["unknown"], "verify": verify } });
                        let (final_status, detail) = par_enrol_status(&r);
                        set(json!({ "course": course, "status": final_status, "detail": detail, "ms": log::ms_since(t), "confirmation": status, "verifying": false }));
                    }
                    Err(e) => set(json!({ "course": course, "status": "failed", "detail": e.1, "ms": log::ms_since(t), "verifying": false })),
                }
            });
        }
}

// ------------------------------------------------------------------ tests (on the connector's own mock site)

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fresh_total_counts_every_tree_course_of_the_term_plus_the_moved_one() {
        let tree = json!({"rows": [
            {"code": "A 1", "units": "4.00", "term": "Winter 2027", "term_choice": null},
            {"code": "B 2", "units": "3.00", "term": null, "term_choice": {"selected": "1271", "options": [{"code": "1271", "label": "Winter 2027"}]}},
            {"code": "C 3", "units": "3.00", "term": null, "term_choice": {"selected": "1275", "options": [{"code": "1275", "label": "Spring 2027"}]}},
            {"code": "SOFT 2301", "units": "3.00", "term": null, "term_choice": {"selected": "1275", "options": [{"code": "1275", "label": "Spring 2027"}]}},
            {"code": "", "label": "A Course List", "units": null, "term": null, "term_choice": null},
        ]});
        assert_eq!(fresh_tree_total(&tree, "Winter 2027", "SOFT2301", false).unwrap(), 10.0); // 4 + 3 + the moved 3
        assert_eq!(fresh_tree_total(&tree, "Spring 2027", "SOFT2301", false).unwrap(), 6.0); // C 3 + the moved one
        assert!(fresh_tree_total(&tree, "Winter 2027", "NOPE1", false).is_err());
        let unknown = json!({"rows": [{"code": "A 1", "units": null, "term": "Winter 2027", "term_choice": null}, {"code": "SOFT 2301", "units": "3.00", "term": null, "term_choice": null}]});
        assert!(fresh_tree_total(&unknown, "Winter 2027", "SOFT2301", false).unwrap_err().contains("unknown"));
        assert!(fresh_tree_total(&json!({}), "Winter 2027", "SOFT2301", false).is_err());
    }

    #[test]
    fn a_catalog_keyword_is_letters_digits_spaces_and_hyphens_only() {
        assert!(clean_args(&json!({"keyword": "INFS 2201"})).is_ok());
        assert!(clean_args(&json!({"keyword": "infs-2201"})).is_ok());
        for bad in ["", "   ", "a;b", "x&y=1", "a\r\nb", "../etc", &"x".repeat(41)] {
            assert!(clean_args(&json!({"keyword": bad})).is_err(), "{bad:?}");
        }
        // an argument name the tools do not read is refused, not passed along
        assert!(clean_args(&json!({"rows": 1})).is_err());
        assert!(clean_args(&json!({"row": 1, "anything": 2})).is_err());
        // other text arguments are still refused
        assert!(clean_args(&json!({"term_code": "abc"})).is_err());
        assert!(READ_TOOLS.contains(&"catalog_search"));
    }

    #[test]
    fn the_servers_staging_limits_and_the_connectors_token_lifetime_are_the_same_number() {
        assert_eq!(PREVIEW_KEEP, PRESTAGE_TTL, "one limit, not two: the connector is given PREVIEW_KEEP in build_live");
        assert!(PREVIEW_KEEP > Duration::from_secs(120), "longer than the connector's default, on purpose (H2)");
    }

    #[test]
    fn a_planned_course_that_belongs_to_another_term_is_set_aside_only_when_the_term_is_over() {
        let row = |code: &str, sem: u64, units: &str, term: Option<&str>, sel: Option<(&str, &str)>| {
            json!({ "code": code, "semester": sem, "units": units, "term": term,
                    "term_choice": sel.map(|(c, l)| json!({ "selected": c, "options": [{ "code": c, "label": l }] })) })
        };
        let mut rows = vec![row("T 1", 0, "3.00", Some("Fall 2025"), None)];
        // 16 credits taken or enrolled in Winter 2027 (semester 4 = Winter 2027), one planned course of semester 6 (Fall 2027) stuck there
        for (i, u) in ["4.00", "3.00", "3.00", "3.00"].iter().enumerate() {
            rows.push(row(&format!("E {i}"), 4, u, Some("Winter 2027"), None));
        }
        rows.push(row("STUCK 1", 6, "3.00", None, Some(("1271", "Winter 2027"))));
        rows.push(row("SOFT 2301", 5, "3.00", None, Some(("1275", "Spring 2027"))));
        let tree = json!({ "rows": rows });
        // 13 enrolled + the stuck course + the moved one = 19 > 18: the stuck one belongs to Fall 2027, so it is set aside; the moved one counts
        // a plan WRITE is strict (the rule was never verified against PeopleSoft's Enrol click); the enrol-only Apply recount uses the rule
        assert_eq!(fresh_tree_total(&tree, "Winter 2027", "SOFT2301", false).unwrap(), 19.0);
        assert_eq!(fresh_tree_total(&tree, "Winter 2027", "SOFT2301", true).unwrap(), 16.0);
        // not over: nothing is set aside
        let lighter = json!({ "rows": [row("T 1", 0, "3.00", Some("Fall 2025"), None), row("E 0", 4, "4.00", Some("Winter 2027"), None), row("STUCK 1", 6, "3.00", None, Some(("1271", "Winter 2027"))), row("SOFT 2301", 5, "3.00", None, Some(("1275", "Spring 2027")))] });
        assert_eq!(fresh_tree_total(&lighter, "Winter 2027", "SOFT2301", true).unwrap(), 10.0);
        // a course that is only enrolled (a plain term) is never set aside, whatever its semester says
        let enrolled_over = json!({ "rows": [row("T 1", 0, "3.00", Some("Fall 2025"), None), row("E 0", 6, "18.00", Some("Winter 2027"), None), row("SOFT 2301", 5, "3.00", None, Some(("1275", "Spring 2027")))] });
        assert_eq!(fresh_tree_total(&enrolled_over, "Winter 2027", "SOFT2301", true).unwrap(), 21.0);
        assert_eq!(curriculum_term(&rows, 6).as_deref(), Some("Fall 2027"));
        assert_eq!(curriculum_term(&rows, 5).as_deref(), Some("Spring 2027"));
    }

    // ---- the contract with the page: the samples the page's own tests produce (frontend/src/v2/fixtures/contract-*.json)

    const CONTRACT_APPLY: &str = include_str!("../../frontend/src/v2/fixtures/contract-apply-plan.json");
    const CONTRACT_PREVIEW: &str = include_str!("../../frontend/src/v2/fixtures/contract-preview-plan.json");
    const CONTRACT_CATALOG: &str = include_str!("../../frontend/src/v2/fixtures/contract-catalog-read.json");

    #[test]
    fn contract_the_pages_apply_request_with_a_plan_step_parses_and_passes_every_check() {
        let req: ApplyReq = serde_json::from_str(CONTRACT_APPLY).expect("the server can read what the page sends");
        assert_eq!(req.plans.len(), 1);
        assert_eq!((req.plans[0].year, req.plans[0].index, req.plans[0].term_code.as_str()), (1, 8, "1271"));
        assert!(req.enrols[0].after_plan);
        assert_eq!(req.term_row, None, "nothing is enrolled in the term, so the page does not know a drop row (null, never 0)");
        assert_eq!((req.enrols[0].pick.row, req.enrols[0].pick.option_row), (u32::MAX, u32::MAX), "no row yet: the server finds it after planning");
        assert!(check_credits(&req).is_ok(), "{:?}", check_credits(&req));
        assert!(check_plans(&req).is_ok(), "{:?}", check_plans(&req));
    }

    #[test]
    fn contract_the_pages_plan_preview_and_catalog_reads_are_accepted() {
        let t: ToolReq = serde_json::from_str(CONTRACT_PREVIEW).expect("preview request parses");
        assert!(PREVIEW_TOOLS.contains(&t.tool.as_str()));
        let args = clean_args(&t.args).expect("the arguments are in the allowed shape");
        assert!(check_plan_request(&args, t.credits.as_ref()).is_ok());
        assert_eq!(t.expect, Some(vec!["TEST1001".to_string()]), "the page names the course the preview is for");
        let c: ToolReq = serde_json::from_str(CONTRACT_CATALOG).expect("catalog request parses");
        assert!(READ_TOOLS.contains(&c.tool.as_str()));
        assert!(clean_args(&c.args).is_ok());
    }

    #[test]
    fn contract_a_deliberately_wrong_shape_is_refused() {
        // a number where the term code (a string) belongs, a missing credit check, a negative row, a plan with no enrolment
        let mut v: Value = serde_json::from_str(CONTRACT_APPLY).unwrap();
        v["plans"][0]["termCode"] = json!(1271);
        assert!(serde_json::from_value::<ApplyReq>(v).is_err());
        let mut v: Value = serde_json::from_str(CONTRACT_APPLY).unwrap();
        v["plans"][0].as_object_mut().unwrap().remove("credits");
        assert!(serde_json::from_value::<ApplyReq>(v).is_err());
        let mut v: Value = serde_json::from_str(CONTRACT_APPLY).unwrap();
        v["enrols"][0]["pick"]["row"] = json!(-1);
        assert!(serde_json::from_value::<ApplyReq>(v).is_err());
        let mut v: Value = serde_json::from_str(CONTRACT_APPLY).unwrap();
        v["enrols"][0]["afterPlan"] = json!(false);
        let req: ApplyReq = serde_json::from_value(v).unwrap();
        assert!(check_plans(&req).unwrap_err().contains("not enrolled"));
        let mut t: Value = serde_json::from_str(CONTRACT_PREVIEW).unwrap();
        t["args"]["term_code"] = json!("12;7");
        assert!(clean_args(&t["args"]).is_err());
        t["credits"]["after"] = json!(19.0);
        let tr: ToolReq = serde_json::from_value(t).unwrap();
        assert!(check_plan_request(&json!({"term_code": "1278"}), tr.credits.as_ref()).is_err());
    }

    fn req_with(json_text: &str) -> ApplyReq {
        serde_json::from_str(json_text).unwrap()
    }

    #[test]
    fn an_enrolment_or_swap_that_waits_for_a_step_that_is_not_in_the_request_is_refused() {
        // H1: the page unticked a drop but left the enrolment that overlaps that course's slot.
        let mut v: Value = serde_json::from_str(CONTRACT_APPLY).unwrap();
        v["enrols"][0]["depends"] = json!(["Course A"]);
        let req: ApplyReq = serde_json::from_value(v.clone()).unwrap();
        assert!(check_dependencies(&req).unwrap_err().contains("not part of this Apply"), "{:?}", check_dependencies(&req));
        // the same request with the drop present is fine
        v["drops"] = json!([{ "course": "Course A", "classNbr": "123" }]);
        assert!(check_dependencies(&serde_json::from_value::<ApplyReq>(v.clone()).unwrap()).is_ok());
        // a swap that waits for an absent step
        let mut w = v.clone();
        w["drops"] = json!([]);
        w["enrols"][0]["depends"] = json!([]);
        w["rounds"] = json!([[{ "course": "Course B", "oldNbrs": ["1"], "newNbrs": ["2"], "key": {}, "waitsFor": ["Course A"] }]]);
        assert!(check_dependencies(&req_with(&w.to_string())).unwrap_err().contains("swap of Course B"));
    }

    #[test]
    fn an_apply_that_drops_needs_to_know_the_term_row_and_a_phrase_server_refuses_apply() {
        let mut v: Value = serde_json::from_str(CONTRACT_APPLY).unwrap();
        v["termRow"] = Value::Null;
        assert_eq!(req_with(&v.to_string()).term_row, None, "null is not 'row 0'");
        v["termRow"] = json!(-1);
        assert_eq!(req_with(&v.to_string()).term_row, None, "negative is not 'row 0' either");
        v["termRow"] = json!(2);
        assert_eq!(req_with(&v.to_string()).term_row, Some(2));
        v["termRow"] = json!("x");
        assert!(serde_json::from_value::<ApplyReq>(v).is_err());
    }

    fn tree_rows() -> Value {
        let row = |code: &str, sem: u64, units: &str, term: Option<&str>, sel: Option<(&str, &str)>| {
            json!({ "code": code, "semester": sem, "units": units, "term": term,
                    "term_choice": sel.map(|(c, l)| json!({ "selected": c, "options": [{ "code": c, "label": l }] })) })
        };
        json!({ "rows": [
            row("T 1", 0, "3.00", Some("Fall 2025"), None),
            row("E 0", 4, "10.00", Some("Winter 2027"), None),
            row("E 1", 4, "6.00", Some("Winter 2027"), None),
            row("STUCK 1", 6, "3.00", None, Some(("1271", "Winter 2027"))),
        ] })
    }

    #[test]
    fn the_apply_recount_reads_the_tree_and_refuses_a_page_that_undercounts_or_a_term_over_the_limit() {
        let mut st = WriteState::default();
        st.armed = true;
        st.backend = Some(Box::new(TreeBackend(tree_rows())));
        let mk = |after: f64, drops: Value| -> ApplyReq {
            let mut v: Value = serde_json::from_str(CONTRACT_APPLY).unwrap();
            v["term"] = json!("Winter 2027");
            v["credits"] = json!({ "term": "Winter 2027", "limit": 18.0, "after": after, "unknown": [] });
            v["drops"] = drops;
            req_with(&v.to_string())
        };
        // raw 19, the stuck course belongs to Fall 2027: set aside, floor 16; the page said 16: fine
        assert!(apply_recount(&mut st, &mk(16.0, json!([]))).is_ok());
        // a stale page that says 10 when PeopleSoft has at least 16: refused
        let e = apply_recount(&mut st, &mk(10.0, json!([]))).unwrap_err();
        assert!(e.1.contains("out of date"), "{}", e.1);
        // dropping a 10-credit course in this Apply: floor 9 (6 + the planned course that stays)
        assert!(apply_recount(&mut st, &mk(9.0, json!([{ "course": "E 0", "classNbr": "1" }]))).is_ok(), "6 enrolled + the planned one that stays = 9");
        // over the limit with nothing to set aside
        let mut over = tree_rows();
        over["rows"][3]["term_choice"] = Value::Null;
        over["rows"][3]["term"] = json!("Winter 2027");
        st.backend = Some(Box::new(TreeBackend(over)));
        let e = apply_recount(&mut st, &mk(19.0, json!([]))).unwrap_err();
        assert!(e.1.contains("over the 18"), "{}", e.1);
        // an Apply with no enrolment needs no recount, and no tree read
        let mut v: Value = serde_json::from_str(CONTRACT_APPLY).unwrap();
        v["enrols"] = json!([]);
        v["plans"] = json!([]);
        v["drops"] = json!([{ "course": "X", "classNbr": "1" }]);
        assert!(apply_recount(&mut st, &req_with(&v.to_string())).is_ok());
    }

    struct TreeBackend(Value);
    impl Backend for TreeBackend {
        fn tool(&mut self, name: &str, _args: &Value) -> Result<Value, String> {
            if name == "degree_tree" { Ok(self.0.clone()) } else { Err(format!("unexpected {name}")) }
        }
        fn confirm(&mut self, _t: &str, _p: &str) -> Result<Value, ConfirmFail> {
            Err(ConfirmFail { message: "no".into(), may_have_sent: false })
        }
        fn arm(&mut self) {}
        fn disarm(&mut self) {}
    }

    #[test]
    fn a_plan_preview_for_a_different_course_than_the_one_chosen_is_refused() {
        // M4: the page's copy of the tree is out of date: position (1, 8) is another course in PeopleSoft's tree now.
        struct Preview;
        impl Backend for Preview {
            fn tool(&mut self, name: &str, _a: &Value) -> Result<Value, String> {
                match name {
                    "preview_plan_term" => Ok(json!({ "plan": { "term": "Fall 2027", "class_numbers": ["OTHER9999", "1278"], "required_phrase": "x", "summary": "" }, "token": "t", "ttl_secs": 120 })),
                    _ => Err("unexpected".into()),
                }
            }
            fn confirm(&mut self, _t: &str, _p: &str) -> Result<Value, ConfirmFail> {
                Err(ConfirmFail { message: "no".into(), may_have_sent: false })
            }
            fn arm(&mut self) {}
            fn disarm(&mut self) {}
        }
        let mut st = WriteState::default();
        st.armed = true;
        st.backend = Some(Box::new(Preview));
        let credits = CreditsReq { term: "Fall 2027".into(), limit: 18.0, after: 6.0, unknown: vec![] };
        let e = plan_preview(&mut st, &json!({"year": 1, "index": 8, "term_code": "1278"}), &credits, "SOFT2301").unwrap_err();
        assert!(e.1.contains("not the one you chose"), "{}", e.1);
    }

    fn plan_credits(term: &str, limit: f64, after: f64) -> CreditsReq {
        CreditsReq { term: term.into(), limit, after, unknown: vec![] }
    }

    #[test]
    fn a_plan_write_needs_a_credit_check_that_fits() {
        let args = json!({"year": 1, "index": 8, "term_code": "1278"});
        assert!(check_plan_request(&args, Some(&plan_credits("Fall 2027", 18.0, 18.0))).is_ok());
        assert!(check_plan_request(&args, Some(&plan_credits("Spring 2027", 12.0, 12.0))).is_ok());
        // no check, wrong limit, over the limit, unknown credits, not a number: nothing is sent
        assert!(check_plan_request(&args, None).unwrap_err().contains("out of date"));
        assert!(check_plan_request(&args, Some(&plan_credits("Spring 2027", 18.0, 3.0))).unwrap_err().contains("disagree"));
        assert!(check_plan_request(&args, Some(&plan_credits("Fall 2027", 18.0, 18.5))).unwrap_err().contains("over the 18"));
        assert!(check_plan_request(&args, Some(&plan_credits("Spring 2027", 12.0, 15.0))).unwrap_err().contains("over the 12"));
        let mut unknown = plan_credits("Fall 2027", 18.0, 3.0);
        unknown.unknown = vec!["DACS 3101".into()];
        assert!(check_plan_request(&args, Some(&unknown)).unwrap_err().contains("unknown"));
        assert!(check_plan_request(&args, Some(&plan_credits("Fall 2027", 18.0, f64::NAN))).unwrap_err().contains("not a number"));
    }

    #[test]
    fn a_plan_write_refuses_removing_a_course_and_a_mismatched_term() {
        let remove = json!({"year": 1, "index": 8, "term_code": ""});
        assert!(check_plan_request(&remove, Some(&plan_credits("Fall 2027", 18.0, 3.0))).unwrap_err().contains("not supported"));
        assert!(check_plan_request(&json!({"year": 1, "index": 8}), Some(&plan_credits("Fall 2027", 18.0, 3.0))).is_err());
        let out = json!({"plan": {"term": "Fall 2027"}});
        assert!(plan_term_matches(&out, &plan_credits("Fall 2027", 18.0, 3.0)));
        assert!(!plan_term_matches(&out, &plan_credits("Winter 2027", 18.0, 3.0)));
    }

    fn armed_demo() -> WriteState {
        let mut st = WriteState::default();
        let mut b = build_demo();
        b.arm();
        st.backend = Some(b);
        st.armed = true;
        st.touch();
        st
    }

    /// A pick as the page would build it: the chosen option, the course key, and the seats the person saw.
    fn pick_from(opened: &Value, row: u32, option: usize) -> Pick {
        let o = &opened["sections"]["options"][option];
        let comps = o["components"].as_array().unwrap();
        Pick {
            row,
            option_row: o["row"].as_u64().unwrap() as u32,
            course: opened["sections"]["course"].as_str().unwrap().to_string(),
            class_nbrs: comps.iter().map(|c| c["class_nbr"].as_str().unwrap().to_string()).collect(),
            key: opened["course_key"].clone(),
            seen: comps.iter().map(|c| (c["class_nbr"].as_str().unwrap().to_string(), c["seats"]["open"].as_u64().map(|x| x as u32))).collect(),
            waitlist: false,
        }
    }

    fn first_pick(st: &mut WriteState) -> Pick {
        let be = st.backend.as_mut().unwrap();
        let list = be.tool("plan_class_list", &json!({})).unwrap();
        let row = list["rows"][0]["row"].as_u64().unwrap() as u32;
        let opened = be.tool("plan_select_class", &json!({ "row": row })).unwrap();
        pick_from(&opened, row, 0)
    }

    #[test]
    fn every_statement_has_a_unique_id() {
        let ids: BTreeSet<_> = statements().iter().map(|s| s.0).collect();
        assert_eq!(ids.len(), statements().len());
        assert!(statements().len() >= 9);
    }

    #[test]
    fn enrol_stages_then_previews_then_confirms() {
        let mut st = armed_demo();
        let pick = first_pick(&mut st);
        let r = prepare_enrol(&mut st, std::slice::from_ref(&pick)).unwrap();
        assert_eq!(sorted(&nums_of(&r["staged"])), sorted(&pick.class_nbrs));
        let tok = r["submit"]["token"].as_str().unwrap().to_string();
        assert!(r["submit"].get("instruction").is_none(), "the CLI-only hint must not reach the page");
        let out = confirm_token(&mut st, &tok, None, false, &no_stop()).unwrap();
        assert_eq!(out["kind"], "ENROLL");
        // The token is single use.
        assert!(confirm_token(&mut st, &tok, None, false, &no_stop()).is_err());
    }

    #[test]
    fn wrong_class_numbers_abort_before_any_write() {
        let mut st = armed_demo();
        let mut pick = first_pick(&mut st);
        pick.class_nbrs = vec!["999999".into()];
        let e = prepare_enrol(&mut st, &[pick]).unwrap_err();
        assert!(e.1.contains("Nothing was added"), "{}", e.1);
    }

    #[test]
    fn wrong_course_on_the_row_aborts() {
        let mut st = armed_demo();
        let mut pick = first_pick(&mut st);
        pick.course = "NOPE 0000".into();
        let e = prepare_enrol(&mut st, &[pick]).unwrap_err();
        // The fresh seat read sees that the course on PeopleSoft's page is not the one that was chosen, before opening it.
        assert!(e.1.contains("NOPE 0000") && e.1.contains("Nothing was added"), "{}", e.1);
    }

    #[test]
    fn a_preview_naming_other_classes_is_refused_and_never_stashed() {
        let mut st = armed_demo();
        let prev = st.backend.as_mut().unwrap().tool("preview_swap_stage", &json!({ "candidate_row": 0, "option_row": 1 })).unwrap();
        let e = check_expected(&prev, Some(&["1".to_string()])).unwrap_err();
        assert_eq!(e.0, StatusCode::CONFLICT);
        assert!(check_expected(&prev, None).is_ok());
        let real = nums_of(&prev["plan"]["class_numbers"]);
        assert!(check_expected(&prev, Some(&real)).is_ok());
        assert!(st.phrases.is_empty(), "nothing is stashed unless the caller stashes it");
    }

    #[test]
    fn leftovers_on_the_enrol_list_block_the_submit() {
        // The plain mock's second option has a full class (9003), which the seat check would now stop first, so show
        // seats on it to reach the leftover guard.
        let k = Knobs::default();
        k.lift_zero.store(true, Ordering::SeqCst);
        let mut st = armed_with(&k);
        let a = first_pick(&mut st);
        let opened = st.backend.as_mut().unwrap().tool("plan_select_class", &json!({ "row": a.row })).unwrap();
        assert!(opened["sections"]["options"].as_array().unwrap().len() >= 2, "the mock page must have a second option");
        let b = pick_from(&opened, a.row, 1);
        prepare_enrol(&mut st, std::slice::from_ref(&a)).unwrap();
        let e = prepare_enrol(&mut st, std::slice::from_ref(&b)).unwrap_err();
        assert!(e.1.contains("would not enrol exactly what you chose"), "{}", e.1);
    }

    #[test]
    fn courses_already_chosen_in_peoplesoft_stop_the_enrol_before_a_single_write() {
        // Row 1 (TEST 2002) was chosen by an earlier run; this enrol is for TEST 1001 only.
        let k = Knobs::default();
        let mut st = armed_with_scenario(&k, Scenario { preselected_rows: vec![1], ..Default::default() });
        let a = first_pick(&mut st);
        assert_eq!(a.course, "TEST 1001");
        let out = prepare_enrol(&mut st, std::slice::from_ref(&a)).unwrap();
        assert_eq!(out["blocked"], "leftovers", "{out}");
        assert_eq!(out["leftovers"][0]["course"], "TEST 2002");
        assert_eq!(out["leftovers"].as_array().unwrap().len(), 1);
        assert!(out["message"].as_str().unwrap().contains("Nothing was sent"));
        assert!(k.writes().is_empty(), "no stage, no submit: {:?}", k.writes());
        assert!(st.phrases.is_empty(), "no preview token was issued");
        // Choosing it too is allowed: then it is not a leftover.
        let k2 = Knobs::default();
        let mut st2 = armed_with_scenario(&k2, Scenario { preselected_rows: vec![1], ..Default::default() });
        let first = first_pick(&mut st2);
        let two = Pick { course: "TEST 2002".into(), ..first.clone() };
        let left = live_leftovers(&json!({ "rows": [{ "course_label": "TEST 2002: Adv", "status": "Classes available", "schedule_summary": "Tue" }] }), &[first, two]);
        assert!(left.is_empty());
    }

    #[test]
    fn only_a_class_chosen_counts_as_a_leftover_never_no_classes_or_enrolled() {
        let rows = json!({ "rows": [
            { "course_label": "A 1: x", "status": "No classes available", "schedule_summary": "" },
            { "course_label": "B 2: y", "status": "Enrolled", "schedule_summary": "Mon" },
            { "course_label": "C 3: z", "status": "Classes available", "schedule_summary": "" },
            { "course_label": "D 4: w", "status": "Classes available", "schedule_summary": "Tue 8:00" },
        ] });
        let left = live_leftovers(&rows, &[]);
        assert_eq!(left.len(), 1);
        assert_eq!(left[0]["course"], "D 4");
    }

    #[test]
    fn nothing_works_while_writes_are_off() {
        let mut st = WriteState::default();
        st.backend = Some(build_demo()); // a session exists (demo reads) but writes were never enabled
        let pick = Pick { row: 0, option_row: 0, course: "X".into(), class_nbrs: vec!["1".into()], ..Default::default() };
        assert!(prepare_enrol(&mut st, &[pick]).is_err());
        assert!(confirm_token(&mut st, "anything", None, false, &no_stop()).is_err());
    }

    #[test]
    fn a_token_the_server_did_not_issue_is_refused() {
        let mut st = armed_demo();
        let e = confirm_token(&mut st, "not-a-token", None, false, &no_stop()).unwrap_err();
        assert_eq!(e.0, StatusCode::CONFLICT);
    }

    #[test]
    fn drop_previews_then_confirms() {
        let mut st = armed_demo();
        let be = st.backend.as_mut().unwrap();
        let terms = be.tool("drop_terms", &json!({})).unwrap();
        let term_row = terms[0]["row"].as_u64().unwrap();
        let list = be.tool("enrolled_classes", &json!({ "row": term_row })).unwrap();
        // The mock's drop-review page only knows class 9101; the connector refuses if the review does not show the
        // class that was selected, which is exactly the safety check we want to keep.
        let nbr = list["classes"].as_array().unwrap().iter().filter_map(|c| c["class_nbr"].as_str()).find(|n| *n == "9101").unwrap().to_string();
        let prev = be.tool("preview_drop", &json!({ "term_row": term_row, "class_nbr": nbr })).unwrap();
        let data = stash_preview(&mut st, prev);
        assert_eq!(data["plan"]["kind"], "drop");
        let out = confirm_token(&mut st, data["token"].as_str().unwrap(), None, false, &no_stop()).unwrap();
        assert_eq!(out["kind"], "DROP");
    }

    #[test]
    fn swap_stages_then_commits_each_through_its_own_preview() {
        let mut st = armed_demo();
        let prev = st.backend.as_mut().unwrap().tool("preview_swap_stage", &json!({ "candidate_row": 0, "option_row": 1 })).unwrap();
        let stage = stash_preview(&mut st, prev);
        assert_eq!(stage["plan"]["kind"], "swap_stage");
        confirm_token(&mut st, stage["token"].as_str().unwrap(), None, false, &no_stop()).unwrap();
        let prev = st.backend.as_mut().unwrap().tool("preview_swap_commit", &json!({})).unwrap();
        let commit = stash_preview(&mut st, prev);
        let out = confirm_token(&mut st, commit["token"].as_str().unwrap(), None, false, &no_stop()).unwrap();
        assert_eq!(out["kind"], "SWAP");
        assert!(out["messages"].as_array().unwrap().iter().any(|m| m.as_str().unwrap().contains("Swap completed")));
    }

    #[test]
    fn phrase_mode_needs_the_right_phrase() {
        let mut st = armed_demo();
        let pick = first_pick(&mut st);
        let r = prepare_enrol(&mut st, &[pick]).unwrap();
        let tok = r["submit"]["token"].as_str().unwrap().to_string();
        assert!(confirm_token(&mut st, &tok, None, true, &no_stop()).is_err());
        let e = confirm_token(&mut st, &tok, Some("CONFIRM ENROLL 1"), true, &no_stop()).unwrap_err();
        assert!(e.1.contains("did not match"), "{}", e.1);
        let phrase = r["submit"]["plan"]["required_phrase"].as_str().unwrap().to_string();
        assert!(confirm_token(&mut st, &tok, Some(&phrase), true, &no_stop()).is_ok());
    }

    #[test]
    fn idle_expiry_switches_writes_off_but_keeps_the_read_session() {
        let mut st = armed_demo();
        st.phrases.insert("t".into(), ("p".into(), Instant::now()));
        st.last_used = Some(Instant::now() - IDLE_LIMIT - Duration::from_secs(1));
        st.expire_idle();
        assert!(!st.armed && st.phrases.is_empty());
        assert!(st.backend.is_some(), "reads keep working after writes time out");
        assert!(prepare_enrol(&mut st, &[Pick { row: 0, option_row: 0, course: "X".into(), class_nbrs: vec!["1".into()], ..Default::default() }]).is_err());
    }

    #[test]
    fn sign_out_drops_the_session_entirely() {
        let mut st = armed_demo();
        st.list_cache = Some((Instant::now(), json!({})));
        st.drop_session();
        assert!(st.backend.is_none() && !st.armed && st.list_cache.is_none());
    }

    #[test]
    fn tool_arguments_are_restricted_to_digits() {
        assert!(clean_args(&json!({"row": 3, "class_nbr": "3496"})).is_ok());
        assert!(clean_args(&json!({"class_nbr": "3496; drop"})).is_err());
        assert!(clean_args(&json!({"a": {"b": 1}})).is_err());
        assert!(clean_args(&json!([1, 2])).is_err());
    }

    fn first_two_rows(st: &mut WriteState) -> Vec<(u32, String)> {
        let v = read_tool(st, &None, true, "plan_class_list", &json!({})).unwrap();
        v["rows"].as_array().unwrap().iter().map(|r| (r["row"].as_u64().unwrap() as u32, code_of(r["course_label"].as_str().unwrap()))).collect()
    }

    #[test]
    fn fetch_without_keys_opens_each_course_and_hands_back_keys() {
        let mut st = WriteState::default();
        let rows: Vec<u32> = first_two_rows(&mut st).iter().take(1).map(|r| r.0).collect();
        st.list_cache = None;
        let r = fetch_sections(&mut st, &None, true, &rows, &HashMap::new(), 0, None).unwrap();
        assert_eq!(r["stats"]["direct"], 0);
        assert_eq!(r["stats"]["opened"], 1);
        assert_eq!(r["stats"]["listVisited"], true);
        let c = &r["courses"][0];
        assert_eq!(c["via"], "open");
        assert!(safe_key(&c["key"]).is_some(), "the key comes back so the page can cache it");
        assert!(c["sections"]["options"].as_array().unwrap().len() >= 2);
    }

    #[test]
    fn a_wrong_or_unsafe_key_falls_back_to_opening_the_course() {
        let mut st = WriteState::default();
        let rows = first_two_rows(&mut st);
        let (row, code) = rows[0].clone();
        st.list_cache = None;
        let first = fetch_sections(&mut st, &None, true, &[row], &HashMap::new(), 0, None).unwrap();
        let key = first["courses"][0]["key"].clone();
        // 1. A key for the wrong course: the direct result names another course (or fails), so it must be opened.
        let mut bad = key.clone();
        bad["crse_id"] = json!("999999");
        let mut known = HashMap::new();
        known.insert(code.clone(), bad);
        let r = fetch_sections(&mut st, &None, true, &[row], &known, 0, None).unwrap();
        assert_eq!(r["courses"][0]["sections"]["course"], code.as_str());
        assert!(r["courses"][0].get("error").is_none(), "{}", r["courses"][0]);
        // 2. A key with characters that could shape a request is ignored outright.
        let mut evil = key.clone();
        evil["strm"] = json!("1;rm -rf");
        assert!(safe_key(&evil).is_none());
        known.insert(code, evil);
        let r = fetch_sections(&mut st, &None, true, &[row], &known, 0, None).unwrap();
        assert_eq!(r["courses"][0]["via"], "open");
    }

    #[test]
    fn a_row_that_is_not_on_the_list_is_reported_not_guessed() {
        let mut st = WriteState::default();
        let r = fetch_sections(&mut st, &None, true, &[77], &HashMap::new(), 0, None).unwrap();
        assert!(r["courses"][0]["error"].as_str().unwrap().contains("not on PeopleSoft"));
    }

    #[test]
    fn an_idle_sign_out_is_repaired_for_reads_but_never_while_writes_are_armed() {
        let expired = || -> Box<dyn Backend> {
            let mut cfg = PeopleSoftConfig::udst();
            cfg.min_interval_ms = 0;
            let sc = Scenario { session_expired: true, ..Default::default() };
            Box::new(Live(PeopleSoft::new(Session::new(MockPs::signed_in(sc), cfg))))
        };
        // reads: sign in again once and carry on
        let mut st = WriteState { backend: Some(expired()), ..Default::default() };
        assert!(read_tool(&mut st, &None, true, "plan_class_list", &json!({})).is_ok());
        // armed: the error reaches the person, the staged state is not silently rebuilt
        let mut st = WriteState { backend: Some(expired()), armed: true, ..Default::default() };
        let e = read_tool(&mut st, &None, true, "plan_class_list", &json!({})).unwrap_err();
        assert!(needs_login(&e.1), "{}", e.1);
    }

    // ---------------------------------------------------------------- fresh seat check, on a controllable mock

    use frankie_auth::{AuthFailure, HttpRequest, HttpResponse, Method};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    const WRITE_STAGES: [&str; 5] = ["stage_option", "enrol_submit", "drop_click", "drop_yes", "swap_commit"];

    /// Switches a test can flip mid-flow, and a log of every request that reached the (mock) site.
    #[derive(Clone, Default)]
    struct Knobs {
        /// Class 9002 shows no open seats, from now on.
        full: Arc<AtomicBool>,
        /// The same, but only once the stage step has been sent.
        full_after_stage: Arc<AtomicBool>,
        /// Class number 9002 disappears from every page.
        vanish: Arc<AtomicBool>,
        /// Class 9002 shows no open seats, but only on the page that opens the course (not on a fresh read before it).
        full_on_open: Arc<AtomicBool>,
        /// The second option's "Open Seats 0 of 20" shows 4 instead.
        lift_zero: Arc<AtomicBool>,
        /// PeopleSoft's class list shows "Classes available" where the mock says "Selected" (the stage did not take).
        unselect: Arc<AtomicBool>,
        /// ...or "Enrolled" (the enrolment did take).
        enrolled: Arc<AtomicBool>,
        /// Raised by the mock itself right after the stage step is sent: a stop request arriving mid-enrol.
        stop_after_stage: Arc<AtomicBool>,
        /// The flag a test passes to the enrol as its stop signal.
        stop_flag: Arc<AtomicBool>,
        /// The submit is processed by PeopleSoft but its answer never arrives (the client times out), as happened for real.
        timeout_on_submit: Arc<AtomicBool>,
        /// The same for the stage step.
        timeout_on_stage: Arc<AtomicBool>,
        /// The next attempt to open a course fails before reaching PeopleSoft (once).
        fail_next_open: Arc<AtomicBool>,
        log: Arc<Mutex<Vec<String>>>,
    }

    impl Knobs {
        fn log(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }
        /// Fresh reads of a course page (course_sections is a new GET each time; seat_status would answer from memory).
        fn reads(&self) -> usize {
            self.log().iter().filter(|l| *l == "GET course_sections").count()
        }
        /// Requests that change something at PeopleSoft: POSTs carrying a write stage (a GET sharing the label is a read).
        fn writes(&self) -> Vec<String> {
            self.log().into_iter().filter(|l| l.starts_with("POST ") && WRITE_STAGES.iter().any(|w| l.ends_with(w))).collect()
        }
        /// Position of the first `what` among the requests made after the first `from` requests.
        fn index_after(&self, from: usize, what: &str) -> Option<usize> {
            self.log().iter().skip(from).position(|l| l == what)
        }
    }

    struct SeatMock {
        inner: MockPs,
        k: Knobs,
        stage_sent: bool,
    }

    impl Http for SeatMock {
        fn send(&mut self, r: HttpRequest) -> Result<HttpResponse, AuthFailure> {
            let stage = r.stage;
            let method = if matches!(r.method, Method::Get) { "GET" } else { "POST" };
            self.k.log.lock().unwrap().push(format!("{method} {stage}"));
            if stage == "plan_select_class" && method == "POST" && self.k.fail_next_open.swap(false, Ordering::SeqCst) {
                return Err(AuthFailure::Network);
            }
            let mut res = self.inner.send(r)?;
            let lost = (stage == "enrol_submit" && self.k.timeout_on_submit.load(Ordering::SeqCst)) || (stage == "stage_option" && method == "POST" && self.k.timeout_on_stage.load(Ordering::SeqCst));
            if lost {
                return Err(AuthFailure::Timeout); // PeopleSoft handled it; only the answer was lost
            }
            if stage == "stage_option" {
                self.stage_sent = true;
                if self.k.stop_after_stage.load(Ordering::SeqCst) {
                    self.k.stop_flag.store(true, Ordering::SeqCst);
                }
            }
            let on = |a: &Arc<AtomicBool>| a.load(Ordering::SeqCst);
            if on(&self.k.full) || (on(&self.k.full_after_stage) && self.stage_sent) {
                res.body = res.body.replace("Open Seats 7 of 20", "Open Seats 0 of 20");
            }
            if stage == "plan_select_class" && on(&self.k.full_on_open) {
                res.body = res.body.replace("Open Seats 7 of 20", "Open Seats 0 of 20");
            }
            if on(&self.k.vanish) {
                res.body = res.body.replace("9002", "9992");
            }
            if on(&self.k.lift_zero) {
                res.body = res.body.replace("Open Seats 0 of 20", "Open Seats 4 of 20");
            }
            if on(&self.k.unselect) {
                res.body = res.body.replace("Monday 9:30AM to 11:00AM", ""); // nothing chosen: no schedule shown
            }
            if on(&self.k.enrolled) {
                res.body = res.body.replace(">Classes available<", ">Enrolled<");
            }
            Ok(res)
        }
    }

    fn armed_with(k: &Knobs) -> WriteState {
        armed_with_scenario(k, Scenario::default())
    }

    fn armed_with_scenario(k: &Knobs, sc: Scenario) -> WriteState {
        let mut cfg = PeopleSoftConfig::udst();
        cfg.min_interval_ms = 0;
        let http = SeatMock { inner: MockPs::signed_in(sc), k: k.clone(), stage_sent: false };
        let mut b: Box<dyn Backend> = Box::new(Live(PeopleSoft::new(Session::new(http, cfg))));
        b.arm();
        WriteState { backend: Some(b), armed: true, last_used: Some(Instant::now()), ..Default::default() }
    }

    #[test]
    fn seat_gone_between_fetch_and_stage_stops_before_any_write() {
        let k = Knobs::default();
        let mut st = armed_with(&k);
        let pick = first_pick(&mut st); // the person's fetch: 9002 had 7 seats
        k.full.store(true, Ordering::SeqCst); // ...then it fills up
        let e = prepare_enrol(&mut st, std::slice::from_ref(&pick)).unwrap_err();
        assert_eq!(e.0, StatusCode::CONFLICT);
        assert!(e.1.contains("Class 9002") && e.1.contains("Open Seats 0 of 20"), "names the class and the seat text: {}", e.1);
        assert!(e.1.contains("Nothing was added to your Plan & Enrol list"), "{}", e.1);
        assert!(k.writes().is_empty(), "no write may reach the site: {:?}", k.log());
        assert_eq!(k.reads(), 0, "the page that is opened is the fresh reading, so there is no separate read");
        assert!(st.phrases.is_empty() && st.checks.is_empty(), "no token was issued");
    }

    #[test]
    fn seat_gone_between_stage_and_submit_is_stopped_at_the_click_before_the_submit_is_sent() {
        let k = Knobs::default();
        let mut st = armed_with(&k);
        let pick = first_pick(&mut st);
        let r = prepare_enrol(&mut st, std::slice::from_ref(&pick)).unwrap(); // staged while seats were open
        k.full_after_stage.store(true, Ordering::SeqCst); // it fills up after the stage step went through
        let tok = r["submit"]["token"].as_str().unwrap().to_string();
        let e = confirm_token(&mut st, &tok, None, false, &no_stop()).unwrap_err();
        assert!(e.1.contains("Class 9002") && e.1.contains("Open Seats 0 of 20"), "{}", e.1);
        let w = k.writes();
        assert!(w.contains(&"POST stage_option".to_string()), "staging happened while seats were open: {w:?}");
        assert!(!w.iter().any(|x| x.ends_with("enrol_submit")), "the submit must never be sent: {w:?}");
        assert_eq!(k.reads(), 1, "the one fresh read, made at the click");
        assert!(st.phrases.is_empty() && st.checks.is_empty(), "the token is spent");
    }

    #[test]
    fn seat_gone_between_preview_and_click_stops_the_confirm_and_spends_the_token() {
        let k = Knobs::default();
        let mut st = armed_with(&k);
        let pick = first_pick(&mut st);
        let r = prepare_enrol(&mut st, std::slice::from_ref(&pick)).unwrap();
        let tok = r["submit"]["token"].as_str().unwrap().to_string();
        let reads_before_click = k.reads();
        k.full.store(true, Ordering::SeqCst); // the person takes their time; the class fills up
        let e = confirm_token(&mut st, &tok, None, false, &no_stop()).unwrap_err();
        assert_eq!(e.0, StatusCode::CONFLICT);
        assert!(e.1.contains("Class 9002") && e.1.contains("Open Seats 0 of 20"), "{}", e.1);
        assert_eq!(k.reads(), reads_before_click + 1, "exactly one fresh read at the click");
        assert!(!k.writes().iter().any(|x| x.ends_with("enrol_submit")), "{:?}", k.writes());
        assert!(st.phrases.is_empty() && st.checks.is_empty());
        assert!(confirm_token(&mut st, &tok, None, false, &no_stop()).is_err(), "the token cannot be pressed again");
    }

    #[test]
    fn a_seat_that_empties_in_the_moment_before_staging_is_caught_on_the_opened_page() {
        // The page that is opened next is the one the stage step is built from, and it is the freshest reading there is.
        let k = Knobs::default();
        let mut st = armed_with(&k);
        let pick = first_pick(&mut st);
        k.full_on_open.store(true, Ordering::SeqCst);
        let mark = k.log().len();
        let e = prepare_enrol(&mut st, std::slice::from_ref(&pick)).unwrap_err();
        assert!(e.1.contains("Class 9002") && e.1.contains("Open Seats 0 of 20"), "{}", e.1);
        assert_eq!(k.log().iter().skip(mark).filter(|l| *l == "GET course_sections").count(), 0, "no separate read was needed");
        assert!(k.writes().is_empty(), "{:?}", k.log());
    }

    #[test]
    fn a_class_number_that_vanishes_stops_before_any_write() {
        let k = Knobs::default();
        let mut st = armed_with(&k);
        let pick = first_pick(&mut st);
        k.vanish.store(true, Ordering::SeqCst);
        let e = prepare_enrol(&mut st, std::slice::from_ref(&pick)).unwrap_err();
        assert!(e.1.contains("no longer has the class numbers you chose"), "{}", e.1);
        assert!(k.writes().is_empty(), "{:?}", k.log());
    }

    #[test]
    fn seats_still_open_the_write_proceeds_and_the_only_fresh_read_is_made_at_the_click() {
        let k = Knobs::default();
        let mut st = armed_with(&k);
        let pick = first_pick(&mut st);
        let mark = k.log().len(); // everything from here on is the enrol itself, not the simulated fetch before it
        let r = prepare_enrol(&mut st, std::slice::from_ref(&pick)).unwrap();
        assert_eq!(k.reads(), 0, "preparing makes no separate seat read: the opened page carries the seats");

        // The preview shows the seats as read from the opened page, and when.
        let seats = &r["submit"]["seats"][0];
        assert_eq!(seats["course"], "TEST 1001");
        assert!(seats["readAtMs"].as_u64().unwrap() > 1_700_000_000_000, "a real timestamp");
        let rd = seats["readings"].as_array().unwrap();
        assert_eq!(rd.len(), 2);
        assert!(rd.iter().all(|x| x["ok"] == true && x["dropped"] == false));
        assert_eq!(rd[1]["open"], 7);
        assert_eq!(rd[1]["seatsText"], "Open Seats 7 of 20");

        let tok = r["submit"]["token"].as_str().unwrap().to_string();
        let out = confirm_token(&mut st, &tok, None, false, &no_stop()).unwrap();
        assert_eq!(out["kind"], "ENROLL");
        let w = k.writes();
        assert!(w.contains(&"POST stage_option".to_string()) && w.contains(&"POST enrol_submit".to_string()), "{w:?}");

        // Budget: one fresh GET per course, made when the person presses confirm. It is a real GET each time (seat_status
        // would answer from a 30 s memory), and it comes immediately before the submit is sent.
        assert_eq!(k.reads(), 1);
        let click_read = k.index_after(mark, "GET course_sections").unwrap();
        let open = k.index_after(mark, "POST plan_select_class").unwrap();
        let stage = k.index_after(mark, "POST stage_option").unwrap();
        let submit = k.index_after(mark, "POST enrol_submit").unwrap();
        assert!(open < stage && stage < click_read && click_read < submit, "open {open}, stage {stage}, click read {click_read}, submit {submit}");
    }

    #[test]
    fn a_drop_since_the_fetch_is_flagged_in_the_preview_but_does_not_block() {
        let k = Knobs::default();
        let mut st = armed_with(&k);
        let mut pick = first_pick(&mut st);
        pick.seen.insert("9002".into(), Some(19)); // the person saw 19; it is 7 now
        let r = prepare_enrol(&mut st, std::slice::from_ref(&pick)).unwrap();
        let rd = &r["submit"]["seats"][0]["readings"];
        assert_eq!(rd[1]["dropped"], true);
        assert_eq!(rd[1]["seenOpen"], 19);
        assert_eq!(rd[0]["dropped"], false);
    }

    #[test]
    fn a_pick_without_a_usable_key_is_refused_before_any_request() {
        let k = Knobs::default();
        let mut st = armed_with(&k);
        let mut pick = first_pick(&mut st);
        let before = k.log().len();
        pick.key = Value::Null;
        let e = prepare_enrol(&mut st, std::slice::from_ref(&pick)).unwrap_err();
        assert_eq!(e.0, StatusCode::BAD_REQUEST);
        assert!(e.1.contains("course key"), "{}", e.1);
        assert_eq!(k.log().len(), before, "nothing was sent");
    }

    #[test]
    fn in_phrase_mode_a_wrong_phrase_spends_no_read_and_still_reaches_the_gate() {
        let k = Knobs::default();
        let mut st = armed_with(&k);
        let pick = first_pick(&mut st);
        let r = prepare_enrol(&mut st, std::slice::from_ref(&pick)).unwrap();
        let tok = r["submit"]["token"].as_str().unwrap().to_string();
        let reads = k.reads();
        let e = confirm_token(&mut st, &tok, Some("CONFIRM ENROLL 1"), true, &no_stop()).unwrap_err();
        assert!(e.1.contains("did not match"), "{}", e.1);
        assert_eq!(k.reads(), reads, "a wrong phrase costs no seat read");
        let phrase = r["submit"]["plan"]["required_phrase"].as_str().unwrap().to_string();
        assert!(confirm_token(&mut st, &tok, Some(&phrase), true, &no_stop()).is_ok());
        assert_eq!(k.reads(), reads + 1, "the right phrase triggers the one fresh read");
    }

    #[test]
    fn enrol_stage_and_submit_cannot_be_previewed_around_the_checks() {
        assert!(!PREVIEW_TOOLS.contains(&"preview_stage_enrol"));
        assert!(!PREVIEW_TOOLS.contains(&"preview_submit_enrol"));
        assert_eq!(PREVIEW_TOOLS.len(), 4);
        // the fourth is course planning only (set a planned course's term); no enrol, drop or swap is reachable through it
        assert!(PREVIEW_TOOLS.contains(&"preview_plan_term"));
    }

    fn check_for(opened: &Value, option: usize) -> SeatCheck {
        pick_from(opened, 0, option).to_check()
    }

    #[test]
    fn a_swap_into_a_full_class_gets_no_token_and_sends_nothing() {
        let k = Knobs::default();
        let mut st = armed_with(&k);
        let opened = {
            let be = st.backend.as_mut().unwrap();
            be.tool("plan_class_list", &json!({})).unwrap();
            be.tool("plan_select_class", &json!({ "row": 0 })).unwrap()
        };
        // The mock's second option (9003, 9004) has a class showing "Open Seats 0 of 20".
        let check = check_for(&opened, 1);
        let e = preview_checked(&mut st, "preview_swap_stage", &json!({ "candidate_row": 0, "option_row": 1 }), None, Some(&check)).unwrap_err();
        assert_eq!(e.0, StatusCode::CONFLICT);
        assert!(e.1.contains("Class 9003") && e.1.contains("Open Seats 0 of 20"), "{}", e.1);
        assert!(k.writes().is_empty());
        assert!(st.phrases.is_empty());
        // A swap preview without the check is refused outright.
        let e = preview_checked(&mut st, "preview_swap_stage", &json!({ "candidate_row": 0, "option_row": 1 }), None, None).unwrap_err();
        assert_eq!(e.0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn a_swap_is_checked_at_its_stage_and_again_at_its_final_confirm() {
        let k = Knobs::default();
        let mut st = armed_with(&k);
        let opened = {
            let be = st.backend.as_mut().unwrap();
            be.tool("plan_class_list", &json!({})).unwrap();
            be.tool("plan_select_class", &json!({ "row": 0 })).unwrap()
        };
        let check = check_for(&opened, 0); // 9001 + 9002, both with seats
        let stage = preview_checked(&mut st, "preview_swap_stage", &json!({ "candidate_row": 0, "option_row": 0 }), None, Some(&check)).unwrap();
        assert_eq!(stage["seats"][0]["readings"][1]["open"], 7, "the stage preview shows the fresh seats");
        confirm_token(&mut st, stage["token"].as_str().unwrap(), None, false, &no_stop()).unwrap();
        assert!(st.swap_checks.is_some(), "the final confirm will repeat the check");

        let commit = preview_checked(&mut st, "preview_swap_commit", &json!({}), None, None).unwrap();
        k.full.store(true, Ordering::SeqCst); // it fills up before the person presses the final confirm
        let e = confirm_token(&mut st, commit["token"].as_str().unwrap(), None, false, &no_stop()).unwrap_err();
        assert!(e.1.contains("Class 9002") && e.1.contains("Open Seats 0 of 20"), "{}", e.1);
        assert!(!k.writes().iter().any(|x| x.ends_with("swap_commit")), "the swap must not be committed: {:?}", k.writes());
    }

    // ---------------------------------------------------------------- freeze fix, logging, verification, stop, bulk drop

    use crate::Inner;

    fn test_app() -> App {
        Arc::new(Inner {
            creds: Mutex::new(None),
            write: Arc::new(Mutex::new(WriteState::default())),
            demo: true,
            confirm_phrase: false,
            last_failed_login: Mutex::new(None),
            run_lock: tokio::sync::Mutex::new(()),
            snap: Mutex::new(Snap::default()),
            stop: Arc::new(AtomicBool::new(false)),
            inflight: std::sync::atomic::AtomicUsize::new(0),
            parallel_reads: 0,
            hosts: Default::default(),
            origins: Default::default(),
        })
    }

    #[test]
    fn status_answers_at_once_while_an_operation_holds_the_write_state() {
        // The freeze: a status check used to wait for the lock a long operation held, and the page's polling piled up.
        let app = test_app();
        let held = app.write.lock().unwrap(); // an operation is running and owns the state
        app.snap.lock().unwrap().busy = Some(("enrol_prepare".into(), Instant::now()));
        let (tx, rx) = std::sync::mpsc::channel();
        let a = app.clone();
        std::thread::spawn(move || tx.send(status_json(&a)).unwrap());
        let v = rx.recv_timeout(Duration::from_secs(2)).expect("status must not wait for the running operation");
        assert_eq!(v["busy"]["op"], "enrol_prepare");
        assert!(v["busy"]["seconds"].is_number());
        drop(held);
    }

    #[tokio::test]
    async fn a_request_dropped_mid_operation_still_clears_busy_and_updates_armed() {
        // M7: the client goes away while the blocking work runs. The bookkeeping used to run after the await and was skipped.
        let app = test_app();
        let a = app.clone();
        let task = tokio::spawn(async move {
            let _ = with_state(&a, "slow_read", |st| {
                std::thread::sleep(Duration::from_millis(400));
                st.armed = true;
                Ok(())
            })
            .await;
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(app.snap.lock().unwrap().busy.as_ref().map(|b| b.0.clone()), Some("slow_read".to_string()), "busy while it runs");
        task.abort(); // the HTTP handler is dropped
        tokio::time::sleep(Duration::from_millis(700)).await;
        let s = app.snap.lock().unwrap();
        assert!(s.busy.is_none(), "busy must be cleared even though nobody awaited the result");
        assert!(s.armed, "the armed flag the operation set must reach the status");
    }

    #[tokio::test]
    async fn disable_writes_shows_off_at_once_and_takes_effect_when_the_operation_lets_go() {
        let app = test_app();
        *app.write.lock().unwrap() = armed_demo();
        {
            let mut s = app.snap.lock().unwrap();
            s.armed = true;
            s.last_used = Some(Instant::now());
        }
        assert_eq!(status_json(&app)["armed"], true);
        let held = app.write.lock().unwrap(); // a long write is in progress
        disarm_now(&app); // must not wait for it
        assert_eq!(status_json(&app)["armed"], false, "the page is told at once");
        assert!(app.stop.load(Ordering::SeqCst), "the running operation is told to stop between its steps");
        drop(held);
        tokio::time::sleep(Duration::from_millis(400)).await;
        let g = app.write.lock().unwrap();
        assert!(!g.armed && g.backend.is_some(), "writes are off and the read session is kept");
    }

    #[tokio::test]
    async fn every_operation_is_logged_with_its_wait_and_its_run_time() {
        let app = test_app();
        assert_eq!(with_state(&app, "unit_ok", |_st| Ok(7)).await.unwrap(), 7);
        let e = with_state(&app, "unit_bad", |_st| -> Result<(), ApiErr> { Err(err(StatusCode::CONFLICT, "boom")) }).await.unwrap_err();
        assert_eq!(e.1, "boom");
        let evs = log::recent(500);
        let find = |op: &str| evs.iter().rev().find(|e| e["ev"] == "op_end" && e["op"] == op).cloned().expect("an op_end event");
        let ok = find("unit_ok");
        assert_eq!(ok["ok"], true);
        assert!(ok["wait_ms"].is_number() && ok["ms"].is_number());
        let bad = find("unit_bad");
        assert_eq!(bad["ok"], false);
        assert_eq!(bad["error"], "boom");
        assert!(app.snap.lock().unwrap().busy.is_none(), "nothing is shown as running afterwards");
    }

    #[test]
    fn every_peoplesoft_request_is_timed_in_the_log_without_any_secret() {
        let mut st = WriteState::default();
        st.backend = Some(build_demo());
        read_tool(&mut st, &None, true, "plan_class_list", &json!({})).unwrap();
        let evs: Vec<Value> = log::recent(2000).into_iter().filter(|e| e["ev"] == "ps_req" && e["stage"] == "plan_class_list").collect();
        let e = evs.last().expect("the request was logged");
        assert_eq!(e["method"], "GET");
        assert!(e["status"].is_number() && e["ms"].is_number() && e["bytes"].as_u64().unwrap() > 0);
        let allowed = ["ts", "ev", "stage", "method", "path", "status", "ms", "bytes", "gap_ms", "error"];
        for k in e.as_object().unwrap().keys() {
            assert!(allowed.contains(&k.as_str()), "unexpected field in a request log line: {k}");
        }
        assert!(!e["path"].as_str().unwrap().contains('?'), "no query string is logged");
    }

    #[test]
    fn selection_is_judged_from_what_peoplesoft_itself_shows() {
        let row = |label: &str, status: &str, summary: &str| json!({ "course_label": label, "status": status, "schedule_summary": summary });
        let list = json!({ "rows": [
            row("DACS 2202: Intro", "Selected", ""),
            row("DSAI 2201: Data", "Classes available", "Monday 9:30AM to 11:00AM"),
            row("GARC 2001: History", "Classes available", ""),
            row("INFS 2201: Databases", "Enrolled", "Sunday 8:00AM to 11:00AM"),
        ] });
        assert!(verify_selected(&list, "DACS 2202").is_ok(), "a changed status means it took");
        assert!(verify_selected(&list, "DSAI 2201").is_ok(), "a selected class on the row means it took");
        let e = verify_selected(&list, "GARC 2001").unwrap_err();
        assert!(e.contains("GARC 2001") && e.contains("Classes available") && e.contains("did not take effect"), "{e}");
        assert!(verify_selected(&list, "INFS 2201").is_ok());
        assert!(verify_selected(&list, "NOPE 1").is_err());
    }

    #[test]
    fn after_an_enrol_peoplesofts_own_list_is_read_and_reported() {
        let k = Knobs::default();
        let mut st = armed_with(&k);
        let pick = first_pick(&mut st);
        let r = prepare_enrol(&mut st, std::slice::from_ref(&pick)).unwrap();
        let tok = r["submit"]["token"].as_str().unwrap().to_string();
        k.enrolled.store(true, Ordering::SeqCst);
        let out = confirm_token(&mut st, &tok, None, false, &no_stop()).unwrap();
        let rows = out["verify"]["rows"].as_array().expect("PeopleSoft's list was read after the submit");
        assert_eq!(rows[0]["course"], "TEST 1001");
        assert_eq!(rows[0]["status"], "Enrolled");
        assert_eq!(rows[0]["enrolled"], true);
        let log = k.log();
        let submit = log.iter().position(|l| l.ends_with("enrol_submit")).unwrap();
        assert!(log.iter().skip(submit).any(|l| l == "GET plan_class_list"), "the verification read came after the submit: {log:?}");
    }

    #[test]
    fn an_enrol_that_did_not_take_is_reported_as_not_enrolled() {
        let k = Knobs::default();
        let mut st = armed_with(&k);
        let pick = first_pick(&mut st);
        let r = prepare_enrol(&mut st, std::slice::from_ref(&pick)).unwrap();
        let out = confirm_token(&mut st, r["submit"]["token"].as_str().unwrap(), None, false, &no_stop()).unwrap();
        assert_eq!(out["verify"]["rows"][0]["enrolled"], false, "the page's own answer is not taken as proof");
    }

    #[test]
    fn a_stop_request_ends_an_enrol_before_the_next_write_and_before_the_submit() {
        let k = Knobs::default();
        let mut st = armed_with(&k);
        let pick = first_pick(&mut st);
        // Already asked to stop: not a single request is made.
        k.stop_flag.store(true, Ordering::SeqCst);
        let mark = k.log().len();
        let e = prepare_enrol_stop(&mut st, std::slice::from_ref(&pick), &k.stop_flag).unwrap_err();
        assert!(e.1.contains("switched off"), "{}", e.1);
        assert_eq!(k.log().len(), mark, "nothing was sent");
        // Asked to stop right after the stage step: the submit is never prepared.
        k.stop_flag.store(false, Ordering::SeqCst);
        k.stop_after_stage.store(true, Ordering::SeqCst);
        let e = prepare_enrol_stop(&mut st, std::slice::from_ref(&pick), &k.stop_flag).unwrap_err();
        assert!(e.1.contains("switched off") && e.1.contains("Already on your Plan & Enrol list"), "{}", e.1);
        let w = k.writes();
        assert!(w.contains(&"POST stage_option".to_string()) && !w.iter().any(|x| x.ends_with("enrol_submit")), "{w:?}");
        assert!(st.phrases.is_empty(), "no submit token was issued");
    }

    // ---- bulk drop

    /// A backend that answers drops from a script, so each outcome the real site could give can be tried.
    #[derive(Default)]
    struct Scripted {
        /// What PeopleSoft's real enrolled list shows at each successive read of it (None = the read fails). The connector's own
        /// "after the drop" answer is always the useless empty page seen on the real site, to prove it is not trusted.
        reads: Vec<Option<Vec<&'static str>>>,
        read_i: usize,
        /// Sentences a PeopleSoft page "says" during each confirm.
        says: Vec<&'static str>,
        stopped_before_yes: Vec<bool>,
        calls: Arc<Mutex<Vec<String>>>,
        raise_stop: Option<Arc<AtomicBool>>,
        plan_names: Option<&'static str>,
        n: usize,
        /// Make the confirm at this index fail; `true` = the request may have been sent (a timeout), `false` = it was not.
        fail_confirm_at: Option<(usize, bool)>,
        /// What PeopleSoft's drop page lists as terms at each successive read (None = the read fails).
        terms: Vec<Option<Vec<(u32, &'static str)>>>,
        terms_i: usize,
        /// What each drop preview's own enrolled-list read showed, in order. Empty = the preview reports no list.
        listed: Vec<Vec<&'static str>>,
        previews: usize,
    }

    impl Backend for Scripted {
        fn tool(&mut self, name: &str, args: &Value) -> Result<Value, String> {
            let nbr = args["class_nbr"].as_str().unwrap_or("").to_string();
            self.calls.lock().unwrap().push(format!("{name} {nbr}"));
            match name {
                "enrolled_classes" => {
                    let i = self.read_i;
                    self.read_i += 1;
                    match self.reads.get(i) {
                        Some(Some(list)) => Ok(json!({ "classes": list.iter().map(|c| json!({ "class_nbr": c })).collect::<Vec<_>>() })),
                        _ => Err("could not read the enrolled list".into()),
                    }
                }
                "drop_terms" => {
                    let i = self.terms_i;
                    self.terms_i += 1;
                    match self.terms.get(i) {
                        Some(Some(t)) => Ok(json!(t.iter().map(|(r, n)| json!({ "row": r, "term": n })).collect::<Vec<_>>())),
                        _ => Err("could not read the terms".into()),
                    }
                }
                "preview_drop" => {
                    let named = self.plan_names.map_or(nbr.clone(), String::from);
                    let mut plan = json!({ "class_numbers": [named], "required_phrase": format!("CONFIRM DROP {nbr}"), "term": "Winter 2027" });
                    if let Some(l) = self.listed.get(self.previews) {
                        plan["listed_classes"] = json!(l);
                    }
                    self.previews += 1;
                    Ok(json!({ "plan": plan, "token": format!("tok-{nbr}") }))
                }
                _ => Err("unexpected tool".into()),
            }
        }
        fn confirm(&mut self, token: &str, _phrase: &str) -> Result<Value, ConfirmFail> {
            self.calls.lock().unwrap().push(format!("confirm {token}"));
            let i = self.n;
            self.n += 1;
            if let Some((at, sent)) = self.fail_confirm_at {
                if at == i {
                    return Err(ConfirmFail { message: "timeout".into(), may_have_sent: sent });
                }
            }
            if let Some(s) = &self.raise_stop {
                s.store(true, Ordering::SeqCst);
            }
            if !self.says.is_empty() {
                let said: Vec<String> = self.says.iter().map(|x| x.to_string()).collect();
                log::note_said("drop_yes", &said);
            }
            let detail = if self.stopped_before_yes.get(i).copied().unwrap_or(false) {
                json!({ "stopped_before_yes": true })
            } else {
                json!({ "classes": [], "term": "Winter 2027" }) // the real site's useless answer: an empty list from the wrong page
            };
            Ok(json!({ "kind": "DROP", "messages": [], "detail": detail }))
        }
        fn arm(&mut self) {}
        fn disarm(&mut self) {}
    }

    fn scripted(reads: Vec<Option<Vec<&'static str>>>) -> (WriteState, Arc<Mutex<Vec<String>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let b = Scripted { reads, calls: calls.clone(), ..Default::default() };
        (WriteState { backend: Some(Box::new(b)), armed: true, last_used: Some(Instant::now()), ..Default::default() }, calls)
    }

    fn nums(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn statuses(r: &Value) -> Vec<String> {
        r["results"].as_array().unwrap().iter().map(|x| x["status"].as_str().unwrap().to_string()).collect()
    }

    fn no_stop() -> AtomicBool {
        AtomicBool::new(false)
    }

    #[test]
    fn bulk_drop_drops_each_class_through_its_own_preview_and_proves_it_from_the_enrolled_list() {
        let (mut st, calls) = scripted(vec![Some(vec!["3160", "3169"]), Some(vec!["3169"]), Some(vec![])]);
        let r = drop_bulk(&mut st, 0, &nums(&["3159", "3160", "3169"]), &no_stop(), false, None).unwrap();
        assert_eq!(statuses(&r), ["dropped", "dropped", "dropped"]);
        assert_eq!(r["halted"], Value::Null);
        assert_eq!(r["remaining"], json!([]));
        let c = calls.lock().unwrap().clone();
        assert_eq!(c.iter().filter(|x| x.starts_with("preview_drop")).count(), 3, "one preview each: {c:?}");
        assert_eq!(c.iter().filter(|x| x.starts_with("confirm")).count(), 3);
    }

    #[test]
    fn a_class_that_leaves_with_its_linked_class_is_gone_not_a_failure_and_costs_no_request() {
        // Dropping the lecture 3159 also removes its lab 3160.
        let (mut st, calls) = scripted(vec![Some(vec!["3169"]), Some(vec![])]);
        let r = drop_bulk(&mut st, 0, &nums(&["3159", "3160", "3169"]), &no_stop(), false, None).unwrap();
        assert_eq!(statuses(&r), ["dropped", "gone", "dropped"]);
        let c = calls.lock().unwrap().clone();
        assert_eq!(c.iter().filter(|x| x.starts_with("confirm")).count(), 2, "no write for a class that is already gone: {c:?}");
    }

    #[test]
    fn a_class_still_listed_after_its_drop_halts_the_batch() {
        let (mut st, calls) = scripted(vec![Some(vec!["3159", "3160"])]);
        let r = drop_bulk(&mut st, 0, &nums(&["3159", "3160"]), &no_stop(), false, None).unwrap();
        assert_eq!(statuses(&r), ["still_listed", "skipped"]);
        assert!(r["halted"].as_str().unwrap().contains("3159"));
        let c = calls.lock().unwrap().clone();
        assert_eq!(c.iter().filter(|x| x.starts_with("confirm")).count(), 1, "the second drop is never confirmed: {c:?}");
    }

    #[test]
    fn a_drop_whose_enrolled_list_cannot_be_read_afterwards_is_unverified_and_halts() {
        let (mut st, _) = scripted(vec![None]);
        let r = drop_bulk(&mut st, 0, &nums(&["3159", "3160"]), &no_stop(), false, None).unwrap();
        assert_eq!(statuses(&r), ["dropped_unverified", "skipped"], "{r}");
        assert!(r["results"][0]["note"].as_str().unwrap().contains("not confirmed"));
    }

    #[test]
    fn an_empty_page_in_the_drop_answer_is_not_proof_the_real_list_decides() {
        // The real run: the connector's after-drop page parsed as an empty list. The old code took that as "everything is
        // gone" and reported ten classes dropped that were never touched. The real enrolled list still shows all of them.
        let all = vec!["3382", "3383", "3496", "3497"];
        let (mut st, calls) = scripted(vec![Some(all)]);
        let r = drop_bulk(&mut st, 0, &nums(&["3382", "3383", "3496", "3497"]), &no_stop(), false, None).unwrap();
        assert_eq!(statuses(&r), ["still_listed", "skipped", "skipped", "skipped"], "{r}");
        assert!(r["results"][0]["note"].as_str().unwrap().contains("did not take effect"));
        assert!(!statuses(&r).contains(&"gone".to_string()), "nothing may be called gone on the strength of an empty page");
        assert_eq!(calls.lock().unwrap().iter().filter(|c| c.starts_with("confirm")).count(), 1, "and nothing further is sent");
    }

    #[test]
    fn dropping_a_terms_last_class_is_proved_by_the_term_leaving_the_drop_page() {
        // Real site: Winter 2027 was row 1; after its last class went, only Fall 2026 (row 0) was listed, so the list read fails.
        let terms = vec![Some(vec![(0, "Fall 2026")])];
        let b = Scripted { reads: vec![None], terms, ..Default::default() };
        let mut st = WriteState { backend: Some(Box::new(b)), armed: true, last_used: Some(Instant::now()), ..Default::default() };
        let r = drop_bulk(&mut st, 1, &nums(&["3623"]), &no_stop(), false, None).unwrap();
        assert_eq!(statuses(&r), ["dropped"]);
        assert!(r["halted"].is_null());
    }

    #[test]
    fn an_unreadable_list_is_still_unverified_when_the_term_is_still_listed_or_the_page_cannot_be_read() {
        for after in [Some(vec![(0, "Fall 2026"), (1, "Winter 2027")]), Some(vec![]), None] {
            let terms = vec![after];
            let b = Scripted { reads: vec![None], terms, ..Default::default() };
            let mut st = WriteState { backend: Some(Box::new(b)), armed: true, last_used: Some(Instant::now()), ..Default::default() };
            let r = drop_bulk(&mut st, 1, &nums(&["3623"]), &no_stop(), false, None).unwrap();
            assert_eq!(statuses(&r), ["dropped_unverified"]);
        }
    }

    #[test]
    fn form_decode_handles_dollar_signs_and_plus() {
        assert_eq!(form_decode("P_SELECT%245"), "P_SELECT$5");
        assert_eq!(form_decode("a+b%2"), "a b%2");
    }

    #[test]
    fn what_peoplesofts_pages_said_travels_with_the_result() {
        let _g = APPLY_LOCK.lock().unwrap_or_else(|e| e.into_inner()); // the "said" buffer is process-wide: tests that fill it take turns
        let b = Scripted { reads: vec![Some(vec!["3382"])], says: vec!["Unable to drop: a linked class must be dropped with it"], ..Default::default() };
        let mut st = WriteState { backend: Some(Box::new(b)), armed: true, last_used: Some(Instant::now()), ..Default::default() };
        let r = drop_bulk(&mut st, 0, &nums(&["3382"]), &no_stop(), false, None).unwrap();
        assert_eq!(statuses(&r), ["still_listed"]);
        assert_eq!(r["results"][0]["said"][0], "Unable to drop: a linked class must be dropped with it");
    }

    #[test]
    fn a_problem_reported_before_the_final_yes_stops_the_batch_and_says_nothing_was_dropped() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let b = Scripted { stopped_before_yes: vec![true], calls: calls.clone(), ..Default::default() };
        let mut st = WriteState { backend: Some(Box::new(b)), armed: true, last_used: Some(Instant::now()), ..Default::default() };
        let r = drop_bulk(&mut st, 0, &nums(&["3159", "3160"]), &no_stop(), false, None).unwrap();
        assert_eq!(statuses(&r), ["stopped", "skipped"]);
        assert!(r["results"][0]["note"].as_str().unwrap().contains("nothing was dropped"));
    }

    #[test]
    fn if_peoplesofts_plan_names_another_class_nothing_is_confirmed() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let b = Scripted { plan_names: Some("9999"), calls: calls.clone(), ..Default::default() };
        let mut st = WriteState { backend: Some(Box::new(b)), armed: true, last_used: Some(Instant::now()), ..Default::default() };
        let r = drop_bulk(&mut st, 0, &nums(&["3159"]), &no_stop(), false, None).unwrap();
        assert_eq!(statuses(&r), ["failed"]);
        assert!(!calls.lock().unwrap().iter().any(|x| x.starts_with("confirm")), "no confirm was sent");
    }

    #[test]
    fn a_stop_request_during_a_bulk_drop_skips_the_rest() {
        let stop = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(Mutex::new(Vec::new()));
        let b = Scripted { reads: vec![Some(vec!["3160", "3169"]), Some(vec![])], raise_stop: Some(stop.clone()), calls: calls.clone(), ..Default::default() };
        let mut st = WriteState { backend: Some(Box::new(b)), armed: true, last_used: Some(Instant::now()), ..Default::default() };
        let r = drop_bulk(&mut st, 0, &nums(&["3159", "3160", "3169"]), &stop, false, None).unwrap();
        assert_eq!(statuses(&r), ["dropped", "skipped", "skipped"]);
        assert!(r["halted"].as_str().unwrap().contains("switched off"));
    }

    #[test]
    fn a_bulk_drop_needs_writes_on_and_sane_input_and_sends_nothing_otherwise() {
        let mut off = WriteState::default();
        assert_eq!(drop_bulk(&mut off, 0, &nums(&["3159"]), &no_stop(), false, None).unwrap_err().0, StatusCode::CONFLICT);
        let (mut st, calls) = scripted(vec![None; 20]);
        for bad in [nums(&[]), nums(&["3159", "3159"]), nums(&["31x9"]), nums(&["3159;drop"]), (0..13).map(|i| format!("{}", 3000 + i)).collect::<Vec<_>>()] {
            let e = drop_bulk(&mut st, 0, &bad, &no_stop(), false, None).unwrap_err();
            assert_eq!(e.0, StatusCode::BAD_REQUEST, "{bad:?}");
        }
        assert!(calls.lock().unwrap().is_empty(), "nothing reached PeopleSoft");
    }

    #[test]
    fn in_phrase_mode_one_phrase_naming_every_class_is_needed_before_anything_is_sent() {
        let (mut st, calls) = scripted(vec![Some(vec!["3169"]), Some(vec![])]);
        let want = bulk_drop_phrase(&nums(&["3159", "3169"]));
        assert_eq!(want, "CONFIRM DROP 3159 3169");
        assert!(drop_bulk(&mut st, 0, &nums(&["3159", "3169"]), &no_stop(), true, None).is_err());
        assert!(drop_bulk(&mut st, 0, &nums(&["3159", "3169"]), &no_stop(), true, Some("CONFIRM DROP 3159")).is_err());
        assert!(calls.lock().unwrap().is_empty(), "a wrong phrase sends nothing");
        let r = drop_bulk(&mut st, 0, &nums(&["3159", "3169"]), &no_stop(), true, Some(&want)).unwrap();
        assert_eq!(statuses(&r), ["dropped", "dropped"]);
    }

    #[test]
    fn on_the_connectors_mock_a_bulk_drop_sends_one_drop_and_halts_because_the_page_still_lists_it() {
        // The mock's "after the drop" page still lists class 9101 (a fixed fixture). The proof check treats that as a
        // failed drop, halts the batch, and sends nothing further.
        let k = Knobs::default();
        let mut st = armed_with(&k);
        let term_row = st.backend.as_mut().unwrap().tool("drop_terms", &json!({})).unwrap()[0]["row"].as_u64().unwrap() as u32;
        let r = drop_bulk(&mut st, term_row, &nums(&["9101", "9002"]), &no_stop(), false, None).unwrap();
        assert_eq!(statuses(&r), ["still_listed", "failed"], "{r}");
        let w = k.writes();
        assert_eq!(w.iter().filter(|x| x.ends_with("drop_click")).count(), 1, "{w:?}");
        assert_eq!(w.iter().filter(|x| x.ends_with("drop_yes")).count(), 1, "{w:?}");
        // The mock's review page only knows class 9101, so the connector refuses any other class number at its preview
        // (its own safety check). Put one first and nothing at all is dropped.
        let k2 = Knobs::default();
        let mut st2 = armed_with(&k2);
        let r2 = drop_bulk(&mut st2, term_row, &nums(&["9001", "9101"]), &no_stop(), false, None).unwrap();
        assert_eq!(statuses(&r2), ["failed", "skipped"], "{r2}");
        assert!(k2.writes().is_empty(), "{:?}", k2.writes());
    }

    // ---------------------------------------------------------------- unknown outcomes, skipped reload, parallel reads

    #[test]
    fn the_timeout_is_long_enough_for_a_slow_submit_and_the_gap_setting_is_bounded() {
        assert!(PS_TIMEOUT >= Duration::from_secs(120), "the real submit ran past 30 s");
        assert_eq!(gap_from(None), None);
        assert_eq!(gap_from(Some("nonsense")), None);
        assert_eq!(gap_from(Some("1")), Some(200), "never below 200 ms");
        assert_eq!(gap_from(Some("400")), Some(400));
        assert_eq!(gap_from(Some("999999")), Some(5000), "never above 5 s");
    }

    #[test]
    fn a_submit_that_times_out_is_reported_as_unknown_and_peoplesofts_own_list_is_read() {
        let k = Knobs::default();
        let mut st = armed_with(&k);
        let pick = first_pick(&mut st);
        let r = prepare_enrol(&mut st, std::slice::from_ref(&pick)).unwrap();
        let tok = r["submit"]["token"].as_str().unwrap().to_string();
        k.timeout_on_submit.store(true, Ordering::SeqCst);
        k.enrolled.store(true, Ordering::SeqCst); // PeopleSoft did enrol the student, as it did for real
        let out = confirm_token(&mut st, &tok, None, false, &no_stop()).expect("an unclear submit is an answer, not an error");
        assert_eq!(out["unknown"], true);
        assert_eq!(out["kind"], "ENROLL");
        assert!(out["error"].as_str().unwrap().contains("timeout"));
        assert_eq!(out["verify"]["rows"][0]["status"], "Enrolled");
        assert_eq!(out["verify"]["rows"][0]["enrolled"], true, "the list, not the timeout, says what happened");
        let log = k.log();
        let submit = log.iter().position(|l| l.ends_with("enrol_submit")).unwrap();
        assert!(log.iter().skip(submit + 1).any(|l| l == "GET plan_class_list"), "PeopleSoft's list was read after the lost answer: {log:?}");
        assert!(st.phrases.is_empty() && st.checks.is_empty() && st.kinds.is_empty(), "the token is spent");
        assert!(confirm_token(&mut st, &tok, None, false, &no_stop()).is_err(), "the same preview cannot be pressed again");
    }

    #[test]
    fn an_unclear_submit_that_did_not_enrol_says_so_instead_of_guessing() {
        let k = Knobs::default();
        let mut st = armed_with(&k);
        let pick = first_pick(&mut st);
        let r = prepare_enrol(&mut st, std::slice::from_ref(&pick)).unwrap();
        k.timeout_on_submit.store(true, Ordering::SeqCst);
        let out = confirm_token(&mut st, r["submit"]["token"].as_str().unwrap(), None, false, &no_stop()).unwrap();
        assert_eq!(out["unknown"], true);
        assert_eq!(out["verify"]["rows"][0]["enrolled"], false);
    }

    #[test]
    fn failures_that_happen_before_anything_is_sent_stay_plain_errors() {
        let k = Knobs::default();
        let mut st = armed_with(&k);
        // The connector's own gate refuses a token it never issued: nothing was sent, so it is certain, not "unknown".
        let f = st.backend.as_mut().unwrap().confirm("not-a-token", "x").unwrap_err();
        assert!(!f.may_have_sent, "{f:?}");
        assert_eq!(f.text(), f.message, "no 'may have gone through' warning when nothing was sent");
        let e = confirm_token(&mut st, "not-a-token", None, false, &no_stop()).unwrap_err();
        assert_eq!(e.0, StatusCode::CONFLICT);
        let may = ConfirmFail { message: "timeout".into(), may_have_sent: true };
        assert!(may.text().contains("may or may not have gone through"));
    }

    #[test]
    fn a_stage_that_times_out_stops_the_enrol_and_says_it_may_have_gone_through() {
        let k = Knobs::default();
        let mut st = armed_with(&k);
        let pick = first_pick(&mut st);
        k.timeout_on_stage.store(true, Ordering::SeqCst);
        let e = prepare_enrol(&mut st, std::slice::from_ref(&pick)).unwrap_err();
        assert!(e.1.contains("may or may not have gone through"), "{}", e.1);
        assert!(!k.writes().iter().any(|x| x.ends_with("enrol_submit")), "the submit is never prepared: {:?}", k.writes());
        assert!(st.phrases.is_empty());
    }

    // ---- bulk drop with an unclear answer

    #[test]
    fn a_drop_whose_answer_was_lost_is_settled_by_reading_the_enrolled_list() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        // 3159's confirm times out, but the list afterwards no longer shows it: it did drop. Then 3169 drops normally.
        // read 1 is the one made because the answer was lost; read 2 verifies the drop of 3169
        let b = Scripted { reads: vec![Some(vec!["3169"]), Some(vec![])], calls: calls.clone(), fail_confirm_at: Some((0, true)), ..Default::default() };
        let mut st = WriteState { backend: Some(Box::new(b)), armed: true, last_used: Some(Instant::now()), ..Default::default() };
        let r = drop_bulk(&mut st, 0, &nums(&["3159", "3169"]), &no_stop(), false, None).unwrap();
        assert_eq!(statuses(&r), ["dropped", "dropped"], "{r}");
        assert!(r["results"][0]["note"].as_str().unwrap().contains("cut off"));
        assert!(calls.lock().unwrap().iter().any(|c| c.starts_with("enrolled_classes")), "the list was read to find out");
    }

    fn scripted_state(b: Scripted) -> WriteState {
        WriteState { backend: Some(Box::new(b)), armed: true, last_used: Some(Instant::now()), ..Default::default() }
    }

    fn count(calls: &Arc<Mutex<Vec<String>>>, prefix: &str) -> usize {
        calls.lock().unwrap().iter().filter(|x| x.starts_with(prefix)).count()
    }

    #[test]
    fn a_drops_proof_comes_from_the_next_preview_so_only_the_last_course_costs_a_read() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let b = Scripted {
            listed: vec![vec!["3159", "3160", "3169"], vec!["3160", "3169"], vec!["3169"]],
            reads: vec![Some(vec![])],
            calls: calls.clone(),
            ..Default::default()
        };
        let mut st = scripted_state(b);
        let r = drop_bulk(&mut st, 0, &nums(&["3159", "3160", "3169"]), &no_stop(), false, None).unwrap();
        assert_eq!(statuses(&r), ["dropped", "dropped", "dropped"], "{r}");
        assert_eq!(count(&calls, "enrolled_classes"), 1, "one read, after the last drop: {:?}", calls.lock().unwrap());
        assert_eq!(count(&calls, "confirm"), 3);
    }

    #[test]
    fn a_drop_the_next_preview_still_lists_stops_before_the_next_confirm() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        // Preview 2 reads the list and 3159 is still on it.
        let b = Scripted { listed: vec![vec!["3159", "3160"], vec!["3159", "3160"]], calls: calls.clone(), ..Default::default() };
        let mut st = scripted_state(b);
        let r = drop_bulk(&mut st, 0, &nums(&["3159", "3160"]), &no_stop(), false, None).unwrap();
        assert_eq!(statuses(&r), ["still_listed", "skipped"], "{r}");
        assert_eq!(count(&calls, "confirm"), 1, "the second drop is never confirmed");
        assert_eq!(count(&calls, "enrolled_classes"), 0);
    }

    #[test]
    fn the_live_cart_must_show_exactly_the_chosen_courses_as_selected() {
        let pick = Pick { course: "TEST 1001".into(), ..Default::default() };
        let row = |l: &str, st: &str, sum: &str| json!([l, st, sum]);
        // Statuses as the real page words them.
        let ok = [
            row("TEST 1001: Intro", "Classes available", "Mon 9:30"),
            row("TEST 2002: Adv", "Classes available", ""),
            row("TEST 3003: Other", "No classes available", ""),
            row("TEST 4004: Done", "Enrolled", "Tue 8:00"),
        ];
        assert!(live_cart_mismatch(&ok, std::slice::from_ref(&pick)).is_none());
        let extra = [row("TEST 1001: Intro", "Classes available", "Mon 9:30"), row("TEST 2002: Adv", "Classes available", "Tue 9:30")];
        assert!(live_cart_mismatch(&extra, std::slice::from_ref(&pick)).unwrap().contains("extra: TEST 2002"));
        let missing = [row("TEST 1001: Intro", "Classes available", ""), row("TEST 2002: Adv", "Classes available", "")];
        assert!(live_cart_mismatch(&missing, std::slice::from_ref(&pick)).unwrap().contains("missing: TEST 1001"));
    }

    #[test]
    fn a_parallel_enrol_is_only_enrolled_when_that_sessions_own_list_says_so() {
        let ok = json!({ "login_ok": true, "result": { "verify": { "rows": [{ "course": "A 1", "enrolled": true, "status": "Enrolled" }] } } });
        assert_eq!(par_enrol_status(&ok).0, "enrolled");
        let not = json!({ "login_ok": true, "result": { "verify": { "rows": [{ "course": "A 1", "enrolled": false, "status": "Classes available" }] } } });
        assert_eq!(par_enrol_status(&not).0, "not_enrolled");
        let unknown = json!({ "login_ok": true, "result": { "unknown": true, "verify": { "error": "x" } } });
        assert_eq!(par_enrol_status(&unknown).0, "unknown");
        assert_eq!(par_enrol_status(&json!({ "login_ok": true, "result": { "verify": { "error": "x" } } })).0, "unknown", "no row means unproved");
        assert_eq!(par_enrol_status(&json!({ "login_ok": true, "result": { "blocked": [{ "course": "B 2" }] } })).0, "blocked");
        assert_eq!(par_enrol_status(&json!({ "login_ok": true, "result": { "error": "Stopped" } })).0, "failed");
        assert_eq!(par_enrol_status(&json!({ "login_ok": false, "error": "no" })).0, "failed");
    }

    #[test]
    fn a_waitlisted_row_is_reported_apart_from_an_enrolment() {
        let wl = json!({ "login_ok": true, "result": { "verify": { "rows": [{ "course": "A 1", "enrolled": false, "waitlisted": true, "status": "Waitlisted" }] } } });
        assert_eq!(par_enrol_status(&wl).0, "waitlisted");
        // verify_enrolled: a waitlist pick that shows Enrolled is not reported as a waitlist, and an ordinary pick that shows Waitlisted is not an enrolment
        struct Lst(&'static str);
        impl Backend for Lst {
            fn tool(&mut self, _: &str, _: &Value) -> Result<Value, String> {
                Ok(json!({ "rows": [{ "course_label": "A 1: x", "status": self.0 }] }))
            }
            fn confirm(&mut self, _: &str, _: &str) -> Result<Value, ConfirmFail> { unreachable!() }
            fn arm(&mut self) {}
            fn disarm(&mut self) {}
        }
        let join = SeatCheck { course: "A 1".into(), waitlist: true, ..Default::default() };
        let plain = SeatCheck { course: "A 1".into(), ..Default::default() };
        let v = verify_enrolled(&mut Lst("Waitlisted"), &[join.clone()]);
        assert_eq!((v["rows"][0]["waitlisted"].clone(), v["rows"][0]["enrolled"].clone()), (json!(true), json!(false)));
        let v = verify_enrolled(&mut Lst("Enrolled"), &[join]);
        assert_eq!((v["rows"][0]["waitlisted"].clone(), v["rows"][0]["enrolled"].clone()), (json!(false), json!(false)), "a waitlist join that shows Enrolled is not a clean result: checked by hand");
        let v = verify_enrolled(&mut Lst("Waitlisted"), &[plain]);
        assert_eq!((v["rows"][0]["waitlisted"].clone(), v["rows"][0]["enrolled"].clone()), (json!(false), json!(false)));
    }

    #[test]
    fn the_confirmation_pages_own_icon_per_course_gives_the_first_answer() {
        let d = json!({ "icons": ["error", "success"], "lines": ["INFS 3102: Object-Oriented Programming", "DSAI 2201: Intro"] });
        assert_eq!(confirmation_status(&d, "INFS 3102"), "rejected");
        assert_eq!(confirmation_status(&d, "DSAI 2201"), "submitted");
        assert_eq!(confirmation_status(&d, "GARC 2001"), "unknown");
        assert_eq!(confirmation_status(&json!({}), "DSAI 2201"), "unknown");
    }

    #[test]
    fn a_swap_of_one_course_runs_the_guided_chain_stages_by_class_numbers_and_commits_once() {
        let k = Knobs::default();
        let mut st = armed_with(&k);
        let opened = {
            let be = st.backend.as_mut().unwrap();
            be.tool("plan_class_list", &json!({})).unwrap();
            be.tool("plan_select_class", &json!({ "row": 0 })).unwrap()
        };
        // Option 1 of the mock is 9003 + 9004 (open seats on both after the knob).
        k.lift_zero.store(true, Ordering::SeqCst);
        let p = pick_from(&opened, 0, 1);
        assert_eq!(sorted(&p.class_nbrs), ["9003", "9004"]);
        let req = SwapApplyReq {
            term: "Winter 2099".into(),
            swaps: vec![SwapItem { course: "TEST 1001".into(), old_nbrs: vec!["9001".into(), "9002".into()], new_nbrs: p.class_nbrs.clone(), key: p.key.clone(), seen: p.seen.clone(), waits_for: vec![], hint: String::new() }],
        };
        let out = swap_apply(&mut st, &req, &no_stop()).unwrap();
        let w = k.writes();
        assert_eq!(w.iter().filter(|x| x.ends_with("swap_commit")).count(), 1, "{w:?}");
        assert_eq!(out["results"].as_array().unwrap().len(), 1, "{out}");
        // The same combination is refused before anything is sent.
        let same = SwapApplyReq { term: "Winter 2099".into(), swaps: vec![SwapItem { course: "TEST 1001".into(), old_nbrs: vec!["9001".into()], new_nbrs: vec!["9001".into()], key: p.key.clone(), seen: Default::default(), waits_for: vec![], hint: String::new() }] };
        assert!(swap_apply(&mut st, &same, &no_stop()).is_err());
        // A course PeopleSoft does not offer is refused with nothing staged.
        let k2 = Knobs::default();
        let mut st2 = armed_with(&k2);
        let none = SwapApplyReq { term: "Winter 2099".into(), swaps: vec![SwapItem { course: "ZZZZ 9999".into(), old_nbrs: vec!["1".into()], new_nbrs: vec!["2".into()], key: p.key.clone(), seen: Default::default(), waits_for: vec![], hint: String::new() }] };
        let e = swap_apply(&mut st2, &none, &no_stop()).unwrap_err();
        assert!(e.1.contains("does not offer"), "{}", e.1);
        assert!(k2.writes().is_empty());
    }

    #[test]
    fn a_course_named_only_by_code_and_class_numbers_is_found_on_the_plan_list_and_its_option_by_numbers() {
        let mut st = armed_demo();
        let mut pick = first_pick(&mut st);
        let nums = pick.class_nbrs.clone();
        pick.row = u32::MAX;
        pick.option_row = u32::MAX;
        let r = prepare_enrol(&mut st, &[pick]).unwrap();
        assert_eq!(sorted(&nums_of(&r["staged"])), sorted(&nums));
        // A combination the page does not offer stops before anything is staged.
        let mut st2 = armed_demo();
        let mut bad = first_pick(&mut st2);
        bad.row = u32::MAX;
        bad.option_row = u32::MAX;
        bad.class_nbrs = vec!["424242".into()];
        let e = prepare_enrol(&mut st2, &[bad]).unwrap_err();
        assert!(e.1.contains("no longer offers") || e.1.contains("Nothing was added"), "{}", e.1);
    }

    #[test]
    fn the_swap_pages_class_text_gives_the_class_numbers_held() {
        assert_eq!(class_numbers_in("Lecture - Class 3496 - Section 1 Laboratory - Class 3505 - Section 10"), ["3496", "3505"]);
        assert!(class_numbers_in("nothing here").is_empty());
        let sw = SwapItem { course: "A 1".into(), old_nbrs: vec!["1".into(), "2".into()], new_nbrs: vec!["3".into(), "4".into()], key: json!({}), seen: Default::default(), waits_for: vec![], hint: String::new() };
        let held = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<BTreeSet<_>>();
        assert_eq!(swap_status(&held(&["3", "4"]), &sw).0, "swapped");
        assert_eq!(swap_status(&held(&["1", "2"]), &sw).0, "unchanged");
        assert_eq!(swap_status(&held(&["1", "3", "4"]), &sw).0, "both");
    }

    #[test]
    fn an_answer_without_an_enrolled_list_is_unknown_never_empty() {
        assert!(listed_numbers(&json!({})).is_none());
        assert!(listed_numbers(&json!({ "classes": "x" })).is_none());
        assert_eq!(listed_numbers(&json!({ "classes": [] })).unwrap().len(), 0);
    }

    fn single_drop_state(b: Scripted) -> WriteState {
        let mut st = scripted_state(b);
        st.phrases.insert("tok-3159".into(), ("CONFIRM DROP 3159".into(), Instant::now()));
        st.kinds.insert("tok-3159".into(), "drop".into());
        st.drop_ctx.insert("tok-3159".into(), (0, "3159".into(), Some("Winter 2027".into())));
        st
    }

    #[test]
    fn a_single_drop_whose_answer_was_lost_is_reconciled_and_never_a_plain_error() {
        for (reads, want) in [(Some(vec![]), "absent"), (Some(vec!["3159"]), "still_present"), (None, "unknown")] {
            let b = Scripted { reads: vec![reads], fail_confirm_at: Some((0, true)), ..Default::default() };
            let mut st = single_drop_state(b);
            let out = confirm_token(&mut st, "tok-3159", None, false, &no_stop()).unwrap();
            assert_eq!(out["unknown"], true, "{out}");
            assert_eq!(out["verify"]["state"], want, "{out}");
            assert_eq!(out["kind"], "DROP");
        }
    }

    #[test]
    fn a_single_drop_refused_before_it_was_sent_stays_a_plain_error() {
        let b = Scripted { fail_confirm_at: Some((0, false)), ..Default::default() };
        let mut st = single_drop_state(b);
        assert!(confirm_token(&mut st, "tok-3159", None, false, &no_stop()).is_err());
    }

    #[test]
    fn a_stop_request_reaches_the_confirm_and_nothing_is_sent() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut st = single_drop_state(Scripted { calls: calls.clone(), ..Default::default() });
        let stop = AtomicBool::new(true);
        let e = confirm_token(&mut st, "tok-3159", None, false, &stop).unwrap_err();
        assert_eq!(e.0, StatusCode::CONFLICT);
        assert_eq!(count(&calls, "confirm"), 0);
        assert!(st.phrases.is_empty(), "the preview is spent");
    }

    #[test]
    fn a_swap_commit_without_this_arming_checks_gets_no_preview() {
        let k = Knobs::default();
        let mut st = armed_with(&k);
        assert!(st.swap_checks.is_none());
        let e = preview_checked(&mut st, "preview_swap_commit", &json!({}), None, None).unwrap_err();
        assert_eq!(e.0, StatusCode::CONFLICT);
        assert!(k.writes().is_empty());
    }

    #[test]
    fn a_drop_with_a_lost_answer_that_is_still_listed_halts_as_unknown() {
        let b = Scripted { reads: vec![Some(vec!["3159", "3160"])], fail_confirm_at: Some((0, true)), ..Default::default() };
        let mut st = WriteState { backend: Some(Box::new(b)), armed: true, last_used: Some(Instant::now()), ..Default::default() };
        let r = drop_bulk(&mut st, 0, &nums(&["3159", "3160"]), &no_stop(), false, None).unwrap();
        assert_eq!(statuses(&r), ["unknown", "skipped"], "{r}");
        assert!(r["results"][0]["note"].as_str().unwrap().contains("may or may not have gone through"));
    }

    #[test]
    fn a_drop_refused_before_it_was_sent_is_a_plain_failure_and_reads_nothing_more() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let b = Scripted { calls: calls.clone(), fail_confirm_at: Some((0, false)), ..Default::default() };
        let mut st = WriteState { backend: Some(Box::new(b)), armed: true, last_used: Some(Instant::now()), ..Default::default() };
        let r = drop_bulk(&mut st, 0, &nums(&["3159", "3160"]), &no_stop(), false, None).unwrap();
        assert_eq!(statuses(&r), ["failed", "skipped"]);
        assert!(!calls.lock().unwrap().iter().any(|c| c.starts_with("enrolled_classes")));
    }

    // ---- the redundant reload before the first course

    fn warm_list(st: &mut WriteState) -> Value {
        let list = read_tool(st, &None, true, "plan_class_list", &json!({})).unwrap();
        st.list_cache = Some((Instant::now(), list.clone()));
        st.list_page_valid = true;
        list
    }

    fn count_after(k: &Knobs, mark: usize, what: &str) -> usize {
        k.log().iter().skip(mark).filter(|l| *l == what).count()
    }

    #[test]
    fn the_first_course_is_opened_from_the_list_page_already_in_front_of_the_session() {
        let k = Knobs::default();
        let mut st = armed_with(&k);
        warm_list(&mut st);
        let mark = k.log().len();
        let r = fetch_sections(&mut st, &None, true, &[0], &HashMap::new(), 0, None).unwrap();
        assert_eq!(r["courses"][0]["via"], "open", "{r}");
        assert_eq!(count_after(&k, mark, "GET plan_class_list"), 0, "no reload was needed");
        assert_eq!(count_after(&k, mark, "POST plan_select_class"), 1);
        assert!(!st.list_page_valid, "opening a course moves the session off the list page");
    }

    #[test]
    fn if_opening_straight_from_the_list_page_fails_it_reloads_and_tries_once_more() {
        let k = Knobs::default();
        let mut st = armed_with(&k);
        warm_list(&mut st);
        let mark = k.log().len();
        k.fail_next_open.store(true, Ordering::SeqCst);
        let r = fetch_sections(&mut st, &None, true, &[0], &HashMap::new(), 0, None).unwrap();
        assert_eq!(r["courses"][0]["via"], "open", "the retry worked: {r}");
        assert!(r["courses"][0].get("error").is_none());
        assert_eq!(count_after(&k, mark, "POST plan_select_class"), 2, "the failed attempt and the retry");
        assert_eq!(count_after(&k, mark, "GET plan_class_list"), 1, "the list was reloaded in between");
    }

    #[test]
    fn any_other_read_or_write_in_between_makes_the_reload_necessary() {
        let k = Knobs::default();
        let mut st = armed_with(&k);
        warm_list(&mut st);
        read_tool(&mut st, &None, true, "drop_terms", &json!({})).unwrap();
        assert!(!st.list_page_valid, "another read moved the session off the list page");
        let mark = k.log().len();
        fetch_sections(&mut st, &None, true, &[0], &HashMap::new(), 0, None).unwrap();
        assert_eq!(count_after(&k, mark, "GET plan_class_list"), 1, "so the list is read before opening");
        warm_list(&mut st);
        let _ = armed_backend(&mut st).unwrap();
        assert!(!st.list_page_valid, "a write path clears it too");
    }

    // ---- parallel read sessions

    /// A pretend PeopleSoft with three openable courses. Opening a course on a helper waits until every helper is inside
    /// at once, so it only succeeds if they really ran at the same time.
    struct Fake {
        gate: Arc<(Mutex<usize>, std::sync::Condvar)>,
        want: usize,
        helper: bool,
        direct_needs_login_once: bool,
    }

    fn rows_json() -> Value {
        json!({ "rows": [
            { "row": 0, "course_label": "AAA 100: A", "term": "Winter", "status": "Classes available", "schedule_summary": "" },
            { "row": 1, "course_label": "BBB 200: B", "term": "Winter", "status": "Classes available", "schedule_summary": "" },
            { "row": 2, "course_label": "CCC 300: C", "term": "Winter", "status": "Classes available", "schedule_summary": "" },
        ] })
    }

    impl Backend for Fake {
        fn tool(&mut self, name: &str, args: &Value) -> Result<Value, String> {
            match name {
                "plan_class_list" => Ok(rows_json()),
                "course_sections" if self.direct_needs_login_once => {
                    self.direct_needs_login_once = false;
                    Err("sign-in required".into())
                }
                "plan_select_class" => {
                    let row = args["row"].as_u64().unwrap();
                    if self.helper {
                        let (m, cv) = &*self.gate;
                        let mut n = m.lock().unwrap();
                        *n += 1;
                        cv.notify_all();
                        let (guard, timeout) = cv.wait_timeout_while(n, Duration::from_secs(5), |n| *n < self.want).unwrap();
                        drop(guard);
                        if timeout.timed_out() {
                            return Err("the helpers did not run at the same time".into());
                        }
                    }
                    let code = ["AAA 100", "BBB 200", "CCC 300"][row as usize];
                    Ok(json!({ "course_key": { "strm": "1", "crse_id": format!("00000{row}"), "crse_offer_nbr": "1", "acad_career": "CRED", "institution": "I" }, "sections": { "course": code, "term": "Winter", "options": [] } }))
                }
                _ => Err("unexpected".into()),
            }
        }
        fn confirm(&mut self, _t: &str, _p: &str) -> Result<Value, ConfirmFail> {
            Err("no".to_string().into())
        }
        fn arm(&mut self) {}
        fn disarm(&mut self) {}
    }

    fn fake_state(want: usize, direct_needs_login_once: bool) -> (WriteState, Arc<(Mutex<usize>, std::sync::Condvar)>) {
        let gate = Arc::new((Mutex::new(0usize), std::sync::Condvar::new()));
        let main = Fake { gate: gate.clone(), want, helper: false, direct_needs_login_once };
        (WriteState { backend: Some(Box::new(main)), ..Default::default() }, gate)
    }

    #[test]
    fn a_cold_fetch_opens_courses_on_their_own_sessions_all_at_the_same_time() {
        let (mut st, gate) = fake_state(3, false);
        let made = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (g, m2) = (gate.clone(), made.clone());
        let make = move || -> Result<Box<dyn Backend>, String> {
            m2.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(Fake { gate: g.clone(), want: 3, helper: true, direct_needs_login_once: false }))
        };
        let r = fetch_sections(&mut st, &None, true, &[0, 1, 2], &HashMap::new(), 3, Some(&make)).unwrap();
        let via: Vec<&str> = r["courses"].as_array().unwrap().iter().map(|c| c["via"].as_str().unwrap_or("error")).collect();
        assert_eq!(via, ["parallel", "parallel", "parallel"], "{r}");
        assert_eq!(made.load(Ordering::SeqCst), 3, "one extra session per course");
        assert_eq!(r["stats"]["parallel"], 3);
        assert_eq!(r["stats"]["helpers"], 3);
        assert!(!st.parallel_off, "nothing went wrong");
        assert!(r["courses"][0]["key"]["crse_id"].is_string(), "the key comes back so the page can remember it");
    }

    #[test]
    fn if_a_helper_cannot_sign_in_that_course_falls_back_to_the_main_session_and_parallel_switches_off() {
        let (mut st, gate) = fake_state(2, false);
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (g, c2) = (gate.clone(), calls.clone());
        let make = move || -> Result<Box<dyn Backend>, String> {
            // the first helper to ask cannot sign in; the others work
            if c2.fetch_add(1, Ordering::SeqCst) == 0 {
                Err("sign-in refused".into())
            } else {
                Ok(Box::new(Fake { gate: g.clone(), want: 2, helper: true, direct_needs_login_once: false }))
            }
        };
        let r = fetch_sections(&mut st, &None, true, &[0, 1, 2], &HashMap::new(), 3, Some(&make)).unwrap();
        let courses = r["courses"].as_array().unwrap();
        assert!(courses.iter().all(|c| c.get("error").is_none()), "every course still came back: {r}");
        assert!(courses.iter().any(|c| c["via"] == "open"), "one was opened on the main session instead");
        assert!(st.parallel_off, "the breaker tripped");
        // And it stays off: a second fetch makes no helper sessions.
        let before = calls.load(Ordering::SeqCst);
        let _ = fetch_sections(&mut st, &None, true, &[0, 1, 2], &HashMap::new(), 3, Some(&make)).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), before, "no more helpers once switched off");
    }

    #[test]
    fn if_opening_extra_sessions_signs_the_main_session_out_parallel_switches_off() {
        // Some PeopleSoft installs allow one session per user. The main session's next read is then refused; the server signs
        // in again (a read) and stops using parallel sessions from then on.
        let (mut st, gate) = fake_state(2, true);
        let g = gate.clone();
        let make = move || -> Result<Box<dyn Backend>, String> { Ok(Box::new(Fake { gate: g.clone(), want: 2, helper: true, direct_needs_login_once: false })) };
        let mut known = HashMap::new();
        known.insert("CCC 300".to_string(), json!({ "strm": "1", "crse_id": "000002", "crse_offer_nbr": "1", "acad_career": "CRED", "institution": "I" }));
        let _ = fetch_sections(&mut st, &None, true, &[0, 1, 2], &known, 2, Some(&make)).unwrap();
        assert!(st.relogins >= 1, "the lost main session was repaired by signing in again");
        assert!(st.parallel_off, "and parallel reads are off from now on");
    }

    #[test]
    fn extra_sessions_are_never_opened_while_writes_are_on_or_after_a_problem() {
        assert!(parallel_allowed(3, false, false));
        assert!(!parallel_allowed(3, true, false), "writes are on: no new sign-ins");
        assert!(!parallel_allowed(3, false, true), "switched off after a problem");
        assert!(!parallel_allowed(1, false, false));
        assert!(!parallel_allowed(0, false, false));
    }

    #[test]
    fn parallel_needs_two_cold_courses_and_is_never_used_while_writes_are_on_or_when_not_offered() {
        let (mut st, gate) = fake_state(1, false);
        let made = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (g, m2) = (gate.clone(), made.clone());
        let make = move || -> Result<Box<dyn Backend>, String> {
            m2.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(Fake { gate: g.clone(), want: 1, helper: true, direct_needs_login_once: false }))
        };
        // one cold course: nothing to run alongside, so the normal path
        let _ = fetch_sections(&mut st, &None, true, &[0], &HashMap::new(), 3, Some(&make)).unwrap();
        assert_eq!(made.load(Ordering::SeqCst), 0);
        // helpers set to 1 or 0: off
        let _ = fetch_sections(&mut st, &None, true, &[0, 1], &HashMap::new(), 1, Some(&make)).unwrap();
        assert_eq!(made.load(Ordering::SeqCst), 0);
        // no maker offered (as when writes are on): off
        let _ = fetch_sections(&mut st, &None, true, &[0, 1], &HashMap::new(), 3, None).unwrap();
        assert_eq!(made.load(Ordering::SeqCst), 0);
    }

    /// A pretend swap page: three enrolled courses. A helper's read waits until every reader is inside at once.
    struct SwapFake {
        gate: Arc<(Mutex<usize>, std::sync::Condvar)>,
        want: usize,
        gated: bool,
        fail: bool,
    }

    impl Backend for SwapFake {
        fn tool(&mut self, name: &str, args: &Value) -> Result<Value, String> {
            match name {
                "swap_candidates" => Ok(json!([
                    { "row": 0, "course": "AAA 100 A" }, { "row": 1, "course": "BBB 200 B" }, { "row": 2, "course": "CCC 300 C" },
                ])),
                "swap_options" => {
                    if self.fail {
                        return Err("PeopleSoft said no".into());
                    }
                    let row = args["candidate_row"].as_u64().unwrap();
                    if self.gated {
                        let (m, cv) = &*self.gate;
                        let mut n = m.lock().unwrap();
                        *n += 1;
                        cv.notify_all();
                        let (guard, timeout) = cv.wait_timeout_while(n, Duration::from_secs(5), |n| *n < self.want).unwrap();
                        drop(guard);
                        if timeout.timed_out() {
                            return Err("the readers did not run at the same time".into());
                        }
                    }
                    let code = ["AAA 100", "BBB 200", "CCC 300"][row as usize];
                    Ok(json!({ "course_key": { "crse_id": format!("0000{row}") }, "sections": { "course": code, "options": [] } }))
                }
                _ => Err("unexpected".into()),
            }
        }
        fn confirm(&mut self, _t: &str, _p: &str) -> Result<Value, ConfirmFail> {
            Err("no".to_string().into())
        }
        fn arm(&mut self) {}
        fn disarm(&mut self) {}
    }

    fn codes3() -> Vec<String> {
        vec!["AAA 100".into(), "BBB 200".into(), "CCC 300".into()]
    }

    #[test]
    fn held_options_are_read_on_several_sessions_at_the_same_time() {
        let gate = Arc::new((Mutex::new(0usize), std::sync::Condvar::new()));
        let main = SwapFake { gate: gate.clone(), want: 3, gated: true, fail: false };
        let mut st = WriteState { backend: Some(Box::new(main)), ..Default::default() };
        let g = gate.clone();
        let make = move || -> Result<Box<dyn Backend>, String> { Ok(Box::new(SwapFake { gate: g.clone(), want: 3, gated: true, fail: false })) };
        let r = fetch_held_options(&mut st, &None, true, &codes3(), 3, Some(&make)).unwrap();
        assert_eq!(r["courses"].as_array().unwrap().len(), 3);
        assert_eq!(r["failed"].as_array().unwrap().len(), 0);
        assert_eq!(r["stats"]["workers"], 3);
        assert!(!st.parallel_off);
        let vias: Vec<&str> = r["courses"].as_array().unwrap().iter().map(|c| c["via"].as_str().unwrap()).collect();
        assert_eq!(vias.iter().filter(|v| **v == "parallel").count(), 2);
    }

    #[test]
    fn a_helper_that_fails_is_retried_on_the_main_session_and_parallel_switches_off() {
        let gate = Arc::new((Mutex::new(0usize), std::sync::Condvar::new()));
        let main = SwapFake { gate: gate.clone(), want: 1, gated: false, fail: false };
        let mut st = WriteState { backend: Some(Box::new(main)), ..Default::default() };
        let g = gate.clone();
        let make = move || -> Result<Box<dyn Backend>, String> { Ok(Box::new(SwapFake { gate: g.clone(), want: 1, gated: false, fail: true })) };
        let r = fetch_held_options(&mut st, &None, true, &codes3(), 3, Some(&make)).unwrap();
        assert_eq!(r["courses"].as_array().unwrap().len(), 3, "every course still arrives");
        assert!(st.parallel_off);
        assert!(r["courses"].as_array().unwrap().iter().any(|c| c["via"] == "retry"));
    }

    #[test]
    fn one_worker_reads_everything_on_the_main_session() {
        let gate = Arc::new((Mutex::new(0usize), std::sync::Condvar::new()));
        let main = SwapFake { gate, want: 1, gated: false, fail: false };
        let mut st = WriteState { backend: Some(Box::new(main)), ..Default::default() };
        let r = fetch_held_options(&mut st, &None, true, &codes3(), 1, None).unwrap();
        assert_eq!(r["stats"]["parallel"], false);
        assert!(r["courses"].as_array().unwrap().iter().all(|c| c["via"] == "main"));
    }

    struct RefreshFake {
        fail: bool,
    }

    impl Backend for RefreshFake {
        fn tool(&mut self, name: &str, args: &Value) -> Result<Value, String> {
            match name {
                "plan_class_list" => Ok(json!({ "rows": [{ "row": 0, "course_label": "AAA 100: A", "term": "Winter 2027", "status": "Enrolled", "schedule_summary": "" }] })),
                _ if self.fail => Err("PeopleSoft said no".into()),
                "drop_terms" => Ok(json!([{ "row": 0, "term": "Fall 2026" }, { "row": 1, "term": "Winter 2027" }])),
                "enrolled_classes" => Ok(json!({ "classes": [{ "class_nbr": format!("1{}", args["row"]) }] })),
                _ => Err("unexpected".into()),
            }
        }
        fn confirm(&mut self, _t: &str, _p: &str) -> Result<Value, ConfirmFail> {
            Err("no".to_string().into())
        }
        fn arm(&mut self) {}
        fn disarm(&mut self) {}
    }

    #[test]
    fn a_refresh_reads_the_class_list_and_the_enrolled_list_on_two_sessions() {
        let mut st = WriteState { backend: Some(Box::new(RefreshFake { fail: false })), ..Default::default() };
        let make = || -> Result<Box<dyn Backend>, String> { Ok(Box::new(RefreshFake { fail: false })) };
        let r = fetch_refresh(&mut st, &None, true, Some("Winter 2027"), 2, Some(&make)).unwrap();
        assert_eq!(r["stats"]["via"], "parallel");
        assert_eq!(r["enrolled"]["termRow"], 1);
        assert_eq!(r["enrolled"]["classes"][0]["class_nbr"], "11");
        assert_eq!(r["classes"]["rows"].as_array().unwrap().len(), 1);
        assert!(!st.parallel_off);
    }

    #[test]
    fn if_the_helper_fails_the_enrolled_list_is_read_on_the_main_session_and_parallel_switches_off() {
        let mut st = WriteState { backend: Some(Box::new(RefreshFake { fail: false })), ..Default::default() };
        let make = || -> Result<Box<dyn Backend>, String> { Ok(Box::new(RefreshFake { fail: true })) };
        let r = fetch_refresh(&mut st, &None, true, Some("Winter 2027"), 2, Some(&make)).unwrap();
        assert_eq!(r["stats"]["via"], "main");
        assert_eq!(r["enrolled"]["termRow"], 1);
        assert!(st.parallel_off);
    }

    #[test]
    fn without_a_term_only_the_class_list_is_read_and_an_unknown_term_has_no_enrolled_classes() {
        let mut st = WriteState { backend: Some(Box::new(RefreshFake { fail: false })), ..Default::default() };
        let r = fetch_refresh(&mut st, &None, true, None, 1, None).unwrap();
        assert!(r["enrolled"].is_null());
        let r = fetch_refresh(&mut st, &None, true, Some("Spring 2099"), 1, None).unwrap();
        assert_eq!(r["enrolled"]["termRow"], -1);
    }

    // ---- the whole apply run against the pretend PeopleSoft: what each kind of failure does to the steps that depend on it ----

    /// The apply run keeps its progress in process-wide statics, so tests that use it take turns.
    static APPLY_LOCK: Mutex<()> = Mutex::new(());

    fn demo_with(sc: Scenario) -> Box<dyn Backend> {
        let mut cfg = PeopleSoftConfig::udst();
        cfg.min_interval_ms = 0;
        Box::new(Live(PeopleSoft::new(Session::new(Timed::new(MockPs::signed_in(sc)), cfg))))
    }

    struct Ran {
        items: Vec<Value>,
    }

    impl Ran {
        fn status(&self, id: &str) -> String {
            self.items.iter().find(|x| x["id"] == id).and_then(|x| x["status"].as_str()).unwrap_or("missing").to_string()
        }
        fn note(&self, id: &str) -> String {
            self.items.iter().find(|x| x["id"] == id).and_then(|x| x["note"].as_str()).unwrap_or("").to_string()
        }
    }

    fn run_apply(req: &ApplyReq, main: WriteState, helper: impl Fn() -> Box<dyn Backend> + Send + Sync + 'static) -> Ran {
        let _g = APPLY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        *TEST_BACKENDS.lock().unwrap() = Some(Box::new(helper));
        *JOB.lock().unwrap() = Some(new_job(req));
        let mut st = main;
        let creds = Creds { user: "u".into(), pass: "p".into() };
        let stop = Arc::new(AtomicBool::new(false));
        let r = apply_run(&mut st, &creds, req, &stop);
        settle_job(&r);
        *TEST_BACKENDS.lock().unwrap() = None;
        let j = JOB.lock().unwrap().clone().unwrap();
        Ran { items: j["items"].as_array().cloned().unwrap_or_default() }
    }

    fn mock_pick() -> Pick {
        let mut st = armed_demo();
        first_pick(&mut st)
    }

    fn a_swap(course: &str, p: &Pick, waits_for: Vec<&str>) -> SwapItem {
        SwapItem { course: course.into(), old_nbrs: vec!["9001".into(), "9002".into()], new_nbrs: vec!["9003".into(), "9004".into()], key: p.key.clone(), seen: Default::default(), waits_for: waits_for.into_iter().map(String::from).collect(), hint: String::new() }
    }

    fn req_of(drops: Vec<(&str, &str)>, rounds: Vec<Vec<SwapItem>>, enrols: Vec<(Pick, Vec<&str>, bool)>) -> ApplyReq {
        ApplyReq {
            term: "Winter 2099".into(),
            term_row: Some(0),
            drops: drops.into_iter().map(|(c, n)| ApplyDrop { course: c.into(), class_nbr: n.into() }).collect(),
            swaps: vec![],
            rounds,
            enrols: enrols.into_iter().map(|(pick, depends, after_drop)| ApplyEnrol { pick, depends: depends.into_iter().map(String::from).collect(), after_drop, after_plan: false }).collect(),
            plans: vec![],
            credits: None,
        }
    }

    /// A pretend PeopleSoft for the plan steps of an apply: it records every call and answers the plan preview, the tree and the confirm.
    struct PlanFake {
        calls: Arc<Mutex<Vec<String>>>,
        tree_total_units: &'static str,
        confirm_fails: bool,
    }

    impl Backend for PlanFake {
        fn tool(&mut self, name: &str, args: &Value) -> Result<Value, String> {
            self.calls.lock().unwrap().push(format!("tool {name}"));
            match name {
                "preview_plan_term" => Ok(json!({
                    "plan": { "term": "Winter 2099", "class_numbers": ["TEST1001", args["term_code"]], "required_phrase": "CONFIRM PLANTERM TEST1001 x", "summary": "Plan TEST 1001" },
                    "token": "tok", "ttl_secs": 120
                })),
                "degree_tree" => Ok(json!({ "rows": [
                    { "code": "TEST 1001", "units": self.tree_total_units, "term": null, "term_choice": { "selected": "1275", "options": [{ "code": "1275", "label": "Spring 2099" }] } },
                    { "code": "OTHR 2000", "units": "3.00", "term": "Winter 2099", "term_choice": null },
                ] })),
                _ => Err(format!("unexpected {name}")),
            }
        }
        fn confirm(&mut self, token: &str, _phrase: &str) -> Result<Value, ConfirmFail> {
            self.calls.lock().unwrap().push(format!("confirm {token}"));
            if self.confirm_fails {
                Err(ConfirmFail { message: "PeopleSoft did not show the new term".into(), may_have_sent: true })
            } else {
                Ok(json!({ "kind": "PLANTERM", "classNumbers": ["TEST1001"], "messages": [], "detail": { "saved": true } }))
            }
        }
        fn arm(&mut self) {}
        fn disarm(&mut self) {}
    }

    fn plan_main(calls: &Arc<Mutex<Vec<String>>>, tree_total_units: &'static str, confirm_fails: bool) -> WriteState {
        WriteState { backend: Some(Box::new(PlanFake { calls: calls.clone(), tree_total_units, confirm_fails })), armed: true, last_used: Some(Instant::now()), ..Default::default() }
    }

    fn plan_req(after: f64) -> ApplyReq {
        let mut p = mock_pick();
        p.row = u32::MAX;
        p.option_row = u32::MAX;
        let mut req = req_of(vec![], vec![], vec![(p, vec![], false)]);
        req.enrols[0].after_plan = true;
        req.plans = vec![ApplyPlan { course: "TEST 1001".into(), year: 1, index: 8, term_code: "1271".into(), credits: CreditsReq { term: "Winter 2099".into(), limit: 18.0, after, unknown: vec![] } }];
        req.credits = Some(CreditsReq { term: "Winter 2099".into(), limit: 18.0, after, unknown: vec![] });
        req
    }

    #[test]
    fn a_course_is_planned_first_and_only_then_staged_for_enrolment() {
        let calls = Arc::new(Mutex::new(vec![]));
        let req = plan_req(6.0);
        assert!(check_plans(&req).is_ok());
        let ran = run_apply(&req, plan_main(&calls, "3.00", false), || demo_with(Scenario::default()));
        assert_eq!(ran.status("plan:TEST 1001"), "done", "{:?}", ran.items);
        assert!(!ran.note("enrol:TEST 1001").contains("plan step did not work"), "{}", ran.note("enrol:TEST 1001"));
        let c = calls.lock().unwrap().clone();
        assert_eq!(c, vec!["tool preview_plan_term", "tool degree_tree", "confirm tok"], "plan step order: {c:?}");
    }

    #[test]
    fn a_plan_step_that_fails_leaves_its_enrolment_unsent() {
        let calls = Arc::new(Mutex::new(vec![]));
        let k = Knobs::default();
        let ran = run_apply(&plan_req(6.0), plan_main(&calls, "3.00", true), || demo_with(Scenario::default()));
        assert_eq!(ran.status("plan:TEST 1001"), "error", "{:?}", ran.items);
        assert!(ran.note("plan:TEST 1001").contains("did not show the new term"), "{}", ran.note("plan:TEST 1001"));
        assert_eq!(ran.status("enrol:TEST 1001"), "error");
        assert!(ran.note("enrol:TEST 1001").contains("its plan step did not work"), "{}", ran.note("enrol:TEST 1001"));
        let _ = k;
    }

    #[test]
    fn a_plan_that_peoplesofts_own_tree_says_is_over_the_limit_is_not_confirmed() {
        let calls = Arc::new(Mutex::new(vec![]));
        // the course is 17 credits in the fresh tree (a stale page said 6), plus the 3 already there: 20 of 18
        let ran = run_apply(&plan_req(6.0), plan_main(&calls, "17.00", false), || demo_with(Scenario::default()));
        assert_eq!(ran.status("plan:TEST 1001"), "error", "{:?}", ran.items);
        assert!(calls.lock().unwrap().iter().all(|c| !c.starts_with("confirm")), "nothing may be confirmed: {:?}", calls.lock().unwrap());
        assert_eq!(ran.status("enrol:TEST 1001"), "error");
    }

    #[test]
    fn plans_and_their_enrolments_must_match_and_carry_a_fitting_credit_check() {
        assert!(check_plans(&plan_req(18.0)).is_ok());
        assert!(check_plans(&plan_req(18.5)).unwrap_err().contains("over the 18"));
        let mut no_enrol = plan_req(6.0);
        no_enrol.enrols[0].after_plan = false;
        assert!(check_plans(&no_enrol).unwrap_err().contains("not enrolled"));
        let mut no_plan = plan_req(6.0);
        no_plan.plans.clear();
        assert!(check_plans(&no_plan).unwrap_err().contains("waits for a plan step"));
        let mut twice = plan_req(6.0);
        twice.plans.push(twice.plans[0].clone());
        assert!(check_plans(&twice).unwrap_err().contains("twice"));
        let mut other_term = plan_req(6.0);
        other_term.plans[0].credits.term = "Fall 2099".into();
        assert!(check_plans(&other_term).is_err());
        let mut bad_code = plan_req(6.0);
        bad_code.plans[0].term_code = "12;7".into();
        assert!(check_plans(&bad_code).is_err());
    }

    #[test]
    fn an_enrolment_that_needs_a_swap_is_not_sent_when_that_swap_fails() {
        let k = Knobs::default();
        let main = armed_with_scenario(&k, Scenario { swap_fails: true, ..Default::default() });
        let p = mock_pick();
        let req = req_of(vec![], vec![vec![a_swap("TEST 1001", &p, vec![])]], vec![(p.clone(), vec!["TEST 1001"], false)]);
        let ran = run_apply(&req, main, || demo_with(Scenario::default()));
        assert_eq!(ran.status("swap:TEST 1001"), "error", "{:?}", ran.items);
        assert_eq!(ran.status("enrol:TEST 1001"), "error");
        assert!(ran.note("enrol:TEST 1001").contains("Not sent: it needs the swap of TEST 1001, which did not work."), "{}", ran.note("enrol:TEST 1001"));
    }

    #[test]
    fn an_enrolment_that_needs_a_drop_is_not_sent_when_that_drop_fails() {
        let p = mock_pick();
        let req = req_of(vec![("OLDC 1000", "123456")], vec![], vec![(p, vec!["OLDC 1000"], false)]);
        let ran = run_apply(&req, armed_demo(), || demo_with(Scenario::default()));
        assert_eq!(ran.status("drop:OLDC 1000"), "error", "{:?}", ran.items);
        assert!(ran.note("enrol:TEST 1001").contains("Not sent: it needs the drop of OLDC 1000, which did not work."), "{}", ran.note("enrol:TEST 1001"));
    }

    #[test]
    fn a_swap_that_waits_for_a_drop_is_not_sent_when_that_drop_fails() {
        let p = mock_pick();
        let k = Knobs::default();
        let req = req_of(vec![("OLDC 1000", "123456")], vec![vec![a_swap("TEST 1001", &p, vec!["OLDC 1000"])]], vec![]);
        let ran = run_apply(&req, armed_with(&k), || demo_with(Scenario::default()));
        assert_eq!(ran.status("swap:TEST 1001"), "error");
        assert!(ran.note("swap:TEST 1001").contains("has to wait for the drop of OLDC 1000"), "{}", ran.note("swap:TEST 1001"));
        assert!(k.writes().iter().all(|w| !w.ends_with("swap_commit")), "no swap may be committed: {:?}", k.writes());
    }

    #[test]
    fn a_course_dropped_to_be_enrolled_again_is_not_enrolled_when_its_drop_failed() {
        let p = mock_pick();
        let req = req_of(vec![("TEST 1001", "123456")], vec![], vec![(p, vec![], true)]);
        let ran = run_apply(&req, armed_demo(), || demo_with(Scenario::default()));
        assert_eq!(ran.status("drop:TEST 1001"), "error");
        assert_eq!(ran.status("enrol:TEST 1001"), "error");
        assert!(ran.note("enrol:TEST 1001").contains("Not sent: its drop did not work, so it is still enrolled."), "{}", ran.note("enrol:TEST 1001"));
    }

    #[test]
    fn an_enrolment_that_depends_on_nothing_is_not_held_back_by_an_unrelated_failed_swap() {
        let k = Knobs::default();
        let main = armed_with_scenario(&k, Scenario { swap_fails: true, ..Default::default() });
        let p = mock_pick();
        let req = req_of(vec![], vec![vec![a_swap("OTHER 2000", &p, vec![])]], vec![(p, vec![], false)]);
        let ran = run_apply(&req, main, || demo_with(Scenario::default()));
        assert_eq!(ran.status("swap:OTHER 2000"), "error");
        assert!(!ran.note("enrol:TEST 1001").contains("Not sent: it needs"), "{}", ran.note("enrol:TEST 1001"));
    }

    #[test]
    fn nothing_is_left_spinning_when_the_run_ends() {
        let _g = APPLY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        *JOB.lock().unwrap() = Some(json!({ "id": 1, "phase": "enrolling", "finished": false, "error": null, "items": [
            { "id": "enrol:A", "status": "working", "note": "sent, waiting for PeopleSoft" },
            { "id": "enrol:B", "status": "done", "note": "Enrolled." },
            { "id": "swap:C", "status": "pending", "note": "" },
        ] }));
        settle_job(&Ok(()));
        let j = JOB.lock().unwrap().clone().unwrap();
        assert_eq!(j["finished"], true);
        let st = |i: usize| j["items"][i]["status"].as_str().unwrap().to_string();
        assert_eq!((st(0), st(1), st(2)), ("error".into(), "done".into(), "error".into()));
        assert!(j["items"][0]["note"].as_str().unwrap().contains("check PeopleSoft"));
        // After a failure of the whole run every unfinished item carries its reason.
        *JOB.lock().unwrap() = Some(json!({ "id": 2, "finished": false, "error": null, "items": [{ "id": "drop:A", "status": "working", "note": "" }] }));
        settle_job(&Err(err(StatusCode::CONFLICT, "Writes are off.")));
        let j = JOB.lock().unwrap().clone().unwrap();
        assert_eq!(j["error"], "Writes are off.");
        assert_eq!(j["items"][0]["note"], "Writes are off.");
    }

    // ---- every kind of failure at every step of every flow (the fault matrix) ----

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Fault {
        /// The request never reaches PeopleSoft.
        Network,
        /// PeopleSoft handled the request; only the answer was lost (a client timeout).
        LostAnswer,
        Http(u16),
        /// 200 with nothing in it.
        Empty,
        /// 200 with a maintenance page.
        Maintenance,
    }

    const FAULTS: [Fault; 8] = [Fault::Network, Fault::LostAnswer, Fault::Http(500), Fault::Http(503), Fault::Http(429), Fault::Http(403), Fault::Empty, Fault::Maintenance];

    #[derive(Clone, Default)]
    struct Trace {
        /// Every request that was made: method and stage.
        reqs: Arc<Mutex<Vec<(&'static str, &'static str)>>>,
        /// Index in `reqs` of the request the fault hit.
        hit: Arc<Mutex<Option<usize>>>,
        on: Arc<AtomicBool>,
    }

    struct FaultMock {
        inner: MockPs,
        t: Trace,
        stage: &'static str,
        nth: usize,
        fault: Fault,
        seen: usize,
        /// The submit reached the (pretend) site, so its class list shows the course as enrolled from now on (as the real list would).
        submitted: bool,
    }

    impl Http for FaultMock {
        fn send(&mut self, r: HttpRequest) -> Result<HttpResponse, AuthFailure> {
            let stage = r.stage;
            let method = if matches!(r.method, Method::Get) { "GET" } else { "POST" };
            let idx = {
                let mut v = self.t.reqs.lock().unwrap();
                v.push((method, stage));
                v.len() - 1
            };
            if self.t.on.load(Ordering::SeqCst) && stage == self.stage {
                self.seen += 1;
                if self.seen == self.nth {
                    *self.t.hit.lock().unwrap() = Some(idx);
                    let page = |status: u16, body: &str| HttpResponse { status, location: None, content_type: Some("text/html".into()), body: body.into() };
                    match self.fault {
                        Fault::Network => return Err(AuthFailure::Network),
                        Fault::LostAnswer => {
                            if stage == "enrol_submit" {
                                self.submitted = true;
                            }
                            let _ = self.inner.send(r)?;
                            return Err(AuthFailure::Timeout);
                        }
                        Fault::Http(c) => return Ok(page(c, "Service error")),
                        Fault::Empty => return Ok(page(200, "")),
                        Fault::Maintenance => return Ok(page(200, "<html><body><h1>The system is down for maintenance. Please try again later.</h1></body></html>")),
                    }
                }
            }
            if stage == "enrol_submit" {
                self.submitted = true;
            }
            let mut res = self.inner.send(r)?;
            res.body = res.body.replace("Open Seats 0 of 20", "Open Seats 4 of 20");
            if self.submitted {
                res.body = res.body.replace(">Classes available<", ">Enrolled<");
            }
            Ok(res)
        }
    }

    fn faulty_state(t: &Trace, stage: &'static str, nth: usize, fault: Fault) -> WriteState {
        let mut cfg = PeopleSoftConfig::udst();
        cfg.min_interval_ms = 0;
        let http = FaultMock { inner: MockPs::signed_in(Scenario::default()), t: t.clone(), stage, nth, fault, seen: 0, submitted: false };
        let mut b: Box<dyn Backend> = Box::new(Live(PeopleSoft::new(Session::new(http, cfg))));
        b.arm();
        WriteState { backend: Some(b), armed: true, last_used: Some(Instant::now()), ..Default::default() }
    }

    #[derive(Clone, Copy, Debug)]
    enum Flow {
        Enrol,
        Drop,
        Swap,
        Read(&'static str),
    }

    /// One flow on a state: (did it report success, what it said).
    fn run_flow(flow: Flow, st: &mut WriteState, pick: &Pick, t: &Trace) -> (bool, String) {
        t.on.store(true, Ordering::SeqCst);
        match flow {
            Flow::Enrol => match prepare_enrol(st, std::slice::from_ref(pick)) {
                Err(e) => (false, e.1),
                Ok(r) => {
                    let tok = r["submit"]["token"].as_str().unwrap_or("").to_string();
                    match confirm_token(st, &tok, None, false, &no_stop()) {
                        // An answer that says "unknown" or carries an error is not a success.
                        Ok(v) => {
                            let proved = v["verify"]["rows"].as_array().is_none_or(|rows| rows.iter().all(|r| r["enrolled"] == true));
                            (v["unknown"] != true && v["error"].is_null() && proved, v.to_string())
                        }
                        Err(e) => (false, e.1),
                    }
                }
            },
            Flow::Drop => match drop_bulk(st, 0, &nums(&["9101"]), &no_stop(), false, None) {
                Err(e) => (false, e.1),
                Ok(v) => (v["results"][0]["status"] == "dropped", v.to_string()),
            },
            Flow::Swap => {
                let req = SwapApplyReq { term: "Winter 2099".into(), swaps: vec![SwapItem { course: "TEST 1001".into(), old_nbrs: vec!["9001".into(), "9002".into()], new_nbrs: vec!["9003".into(), "9004".into()], key: pick.key.clone(), seen: pick.seen.clone(), waits_for: vec![], hint: String::new() }] };
                match swap_apply(st, &req, &no_stop()) {
                    Err(e) => (false, e.1),
                    Ok(v) => (v["results"][0]["status"] == "swapped", v.to_string()),
                }
            }
            Flow::Read(tool) => match read_tool(st, &None, true, tool, &if tool == "enrolled_classes" { json!({ "row": 0 }) } else if tool == "swap_options" { json!({ "candidate_row": 0 }) } else { json!({}) }) {
                Ok(v) => (true, v.to_string()),
                Err(e) => (false, e.1),
            },
        }
    }

    /// Steps whose answers the connector does not rely on: the guide's loading requests (its ids are fixed), and a form-field step whose
    /// effect is checked afterwards by reading the cart back. A fault there changes nothing about the result.
    ///
    /// Also the steps whose answer is only a stepping stone and is checked by a later page before anything irreversible: `drop_term` (the
    /// drop page that follows must name the term), `swap_course_page` (the swap review must list exactly the staged classes before the
    /// commit is even offered) and `drop_click` (the final yes only ever answers the class that was already previewed and approved).
    const NOT_RELIED_ON: [&str; 9] = ["swap_guide_preload", "swap_guide", "drop_guide_preload", "drop_guide", "stage_setfield", "swap_start", "drop_term", "swap_course_page", "drop_click"];

    /// Read pages whose empty or maintenance answer the pinned connector still accepts as "nothing there" (the fix is in the frankie working
    /// tree, not yet in the pinned commit). The planner guards against it one level up: see `empty_list_is_not_trusted`.
    const PARSE_GAPS: [&str; 4] = ["plan_class_list", "plan_open_list", "drop_terms", "swap_candidates"];

    fn debug_leak(text: &str) -> Option<&'static str> {
        ["Unexpected(", "Err(", "Auth(", "HttpFailure", "PsError", "NeedsLogin", "WriteRefused", "NotFound(", "thread '", "panicked"].into_iter().find(|w| text.contains(w))
    }

    /// Runs the flow once without a fault to learn its steps, then once for every step and every fault.
    fn matrix(name: &str, flow: Flow) -> Vec<String> {
        let _g = APPLY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut rows = Vec::new();
        let pick = mock_pick();
        let t0 = Trace::default();
        let mut st0 = faulty_state(&t0, "-", 1, Fault::Network);
        let (ok0, said0) = run_flow(flow, &mut st0, &pick, &t0);
        let mut stages: Vec<&'static str> = Vec::new();
        for (_, s) in t0.reqs.lock().unwrap().iter() {
            if !stages.contains(s) {
                stages.push(*s);
            }
        }
        rows.push(format!("{name:<22} (no fault)         ok={ok0} {}", log::trunc(&said0, 90)));
        for stage in stages {
            for fault in FAULTS {
                let t = Trace::default();
                let mut st = faulty_state(&t, stage, 1, fault);
                let (ok, said) = run_flow(flow, &mut st, &pick, &t);
                let reqs = t.reqs.lock().unwrap().clone();
                let hit = *t.hit.lock().unwrap();
                let writes_after = hit.map(|h| reqs.iter().skip(h + 1).filter(|(m, s)| *m == "POST" && WRITE_STAGES.contains(s)).count()).unwrap_or(0);
                let wrote_at_or_before = hit.map(|h| reqs.iter().take(h + 1).any(|(m, s)| *m == "POST" && WRITE_STAGES.contains(s))).unwrap_or(false);
                rows.push(format!("{name:<22} {stage:<20} {fault:<16?} ok={ok:<5} writes_after_fault={writes_after} sent_before={wrote_at_or_before} :: {}", log::trunc(&said, 110)));
                assert!(!said.trim().is_empty(), "{name} {stage} {fault:?}: an empty message");
                assert!(debug_leak(&said).is_none(), "{name} {stage} {fault:?}: internal text reached the person: {said}");
                let gap = matches!(fault, Fault::Empty | Fault::Maintenance) && PARSE_GAPS.contains(&stage);
                if gap && ok {
                    rows.push(format!("  ^ known gap: an {fault:?} page was read as an empty list"));
                }
                if !NOT_RELIED_ON.contains(&stage) && !gap {
                    assert!(!ok, "{name} {stage} {fault:?}: success was reported although this step failed: {said}");
                    assert_eq!(writes_after, 0, "{name} {stage} {fault:?}: something was sent after the failure: {reqs:?}");
                }
            }
        }
        rows
    }

    #[test]
    fn fault_matrix_enrol() {
        for r in matrix("enrol", Flow::Enrol) {
            println!("{r}");
        }
    }

    #[test]
    fn fault_matrix_drop() {
        for r in matrix("drop", Flow::Drop) {
            println!("{r}");
        }
    }

    #[test]
    fn fault_matrix_swap() {
        for r in matrix("swap", Flow::Swap) {
            println!("{r}");
        }
    }

    #[test]
    fn fault_matrix_reads() {
        for tool in ["plan_class_list", "drop_terms", "enrolled_classes", "swap_candidates", "swap_options"] {
            for r in matrix(&format!("read:{tool}"), Flow::Read(tool)) {
                println!("{r}");
            }
        }
    }

    #[test]
    fn an_empty_class_list_after_a_good_one_is_not_trusted() {
        let good = json!({ "rows": [{ "row": 0 }] });
        let empty = json!({ "rows": [], "term_label": "" });
        assert!(empty_list_is_not_trusted(Some(&good), &empty).is_err());
        assert!(empty_list_is_not_trusted(Some(&good), &good).is_ok());
        assert!(empty_list_is_not_trusted(None, &empty).is_ok(), "nothing to compare with: the first read is believed");
        assert!(empty_list_is_not_trusted(Some(&empty), &empty).is_ok());
    }

    // ---- the requests the page sends must be readable (a bare 422 means the shape was wrong) ----

    #[test]
    fn the_apply_request_the_page_builds_is_readable_in_every_state() {
        // Nothing enrolled in the term: the page used to send termRow -1, which failed the whole request. Now both -1 and 0 read.
        for row in ["-1", "0", "3"] {
            let body = format!(
                r#"{{"term":"Winter 2027","termRow":{row},"drops":[],"rounds":[],"enrols":[{{"pick":{{"row":4,"optionRow":0,"course":"DACS 2202","key":{{"strm":"1271"}},"classNbrs":["3380","3381"],"seen":{{"3380":18,"3381":null}}}},"depends":[]}}]}}"#
            );
            let req: ApplyReq = serde_json::from_str(&body).unwrap_or_else(|e| panic!("termRow {row}: {e}"));
            assert!(req.term_row.unwrap_or(0) <= 3);
            assert_eq!(req.enrols[0].pick.class_nbrs.len(), 2);
        }
        // A course the page could not give a row for is sent as "not known" and read as such.
        let b = r#"{"term":"W","enrols":[{"pick":{"row":4294967295,"optionRow":4294967295,"course":"X 1","classNbrs":["1"],"seen":{}},"depends":["Y 2"],"afterDrop":true}],"drops":[{"course":"X 1","classNbr":"5"}],"rounds":[[{"course":"Y 2","oldNbrs":["1"],"newNbrs":["2"],"key":null,"seen":{"2":3},"waitsFor":[],"hint":""}]]}"#;
        let req: ApplyReq = serde_json::from_str(b).unwrap();
        assert_eq!(req.enrols[0].pick.row, u32::MAX);
        assert!(req.enrols[0].after_drop);
        assert_eq!(req.rounds[0][0].new_nbrs, ["2"]);
    }

    #[test]
    fn a_pick_without_a_row_is_still_refused_not_guessed() {
        let b = r#"{"term":"W","enrols":[{"pick":{"optionRow":1,"course":"X 1","classNbrs":["1"]},"depends":[]}]}"#;
        assert!(serde_json::from_str::<ApplyReq>(b).is_err());
    }

    // ---- the credit guard on the apply request ----

    fn enrolling(term: &str, credits: Option<CreditsReq>) -> ApplyReq {
        let p = mock_pick();
        let mut req = req_of(vec![], vec![], vec![(p, vec![], false)]);
        req.term = term.into();
        req.credits = credits;
        req
    }

    fn credits(term: &str, limit: f64, after: f64, unknown: &[&str]) -> Option<CreditsReq> {
        Some(CreditsReq { term: term.into(), limit, after, unknown: unknown.iter().map(|s| s.to_string()).collect() })
    }

    #[test]
    fn an_apply_that_enrols_is_refused_over_the_limit_without_a_check_with_unknown_credits_or_a_wrong_limit() {
        // The recorded case: 19 credits in a Winter term is refused by PeopleSoft, so it is refused here before anything is sent.
        let e = check_credits(&enrolling("Winter 2027", credits("Winter 2027", 18.0, 19.0, &[]))).unwrap_err();
        assert!(e.contains("19 credits is over the 18"), "{e}");
        assert!(check_credits(&enrolling("Winter 2027", credits("Winter 2027", 18.0, 18.0, &[]))).is_ok(), "exactly 18 is allowed");
        assert!(check_credits(&enrolling("Winter 2027", credits("Winter 2027", 18.0, 16.0, &[]))).is_ok());
        assert!(check_credits(&enrolling("Winter 2027", None)).unwrap_err().contains("out of date"));
        assert!(check_credits(&enrolling("Winter 2027", credits("Winter 2027", 18.0, 16.0, &["NEW 1000"]))).unwrap_err().contains("unknown"));
        assert!(check_credits(&enrolling("Winter 2027", credits("Winter 2027", 30.0, 25.0, &[]))).unwrap_err().contains("disagree"), "a page that sends a looser limit is refused");
        assert!(check_credits(&enrolling("Winter 2027", credits("Fall 2027", 18.0, 16.0, &[]))).unwrap_err().contains("disagree"));
        assert!(check_credits(&enrolling("Winter 2027", credits("Winter 2027", 18.0, f64::NAN, &[]))).is_err());
    }

    #[test]
    fn spring_uses_twelve_and_an_apply_without_enrolments_is_not_checked() {
        assert!(check_credits(&enrolling("Spring 2027", credits("Spring 2027", 12.0, 13.0, &[]))).unwrap_err().contains("over the 12"));
        assert!(check_credits(&enrolling("Spring 2027", credits("Spring 2027", 12.0, 12.0, &[]))).is_ok());
        // Drops and swaps never press Enrol, so they are not held back by the credit check.
        let drops_only = req_of(vec![("X 1", "123")], vec![], vec![]);
        assert!(check_credits(&drops_only).is_ok());
    }

    #[test]
    fn extract_said_captures_corequisite_and_error_icon_box_text() {
        let html_field = r#"<span id="VC_PE_MSG$0">You cannot enrol in AEMA 2233 because corequisite AEMA 2133 is not enrolled.</span>"#;
        let s1 = extract_said("enrol_submit", html_field);
        assert_eq!(s1, vec!["You cannot enrol in AEMA 2233 because corequisite AEMA 2133 is not enrolled."]);

        let html_div = r#"<img src="/cs/PS_CS_STATUS_ERROR_ICN.svg" alt="Error"/><div class="ps_box-group">Maximum term unit limit exceeded.</div>"#;
        let s2 = extract_said("enrol_submit", html_div);
        assert_eq!(s2, vec!["Maximum term unit limit exceeded."]);
    }
}
