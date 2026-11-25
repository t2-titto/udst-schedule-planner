//! A lab for measuring what PeopleSoft accepts when several signed-in sessions act at once. It exists for one purpose: to find
//! out, with the account holder's explicit go-ahead, whether the planner can be faster than the website by running one
//! session per class.
//!
//! It is off unless the server was STARTED with `PLANNER_LAB_UNTIL=<unix seconds>` in the future (so a web page cannot turn
//! it on), it needs sign-in and writes switched on through the normal consent screen, it only drops or enrols what the
//! caller names, it refuses anything that is not Winter 2027, and it runs one experiment at a time. Every write still goes
//! through the connector's own preview, one-time token and exact phrase.

use crate::log;
use crate::write::{build_live, confirm_token, Backend, Pick, WriteState};
use crate::{ApiErr, App};
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const TERM: &str = "Winter 2027";
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

#[derive(Deserialize)]
pub struct LabReq {
    /// `reads`, `drop`, `cart_isolation`, `enrol`.
    experiment: String,
    /// `serial` or `parallel` (drop and enrol).
    #[serde(default)]
    mode: String,
    #[serde(default, rename = "termRow")]
    term_row: u32,
    #[serde(default, rename = "classNbrs")]
    class_nbrs: Vec<String>,
    #[serde(default)]
    sessions: usize,
    #[serde(default)]
    picks: Vec<Pick>,
}

fn deny(code: StatusCode, m: &str) -> ApiErr {
    ApiErr(code, m.to_string())
}

fn lab_until() -> Option<u64> {
    std::env::var("PLANNER_LAB_UNTIL").ok().and_then(|v| v.trim().parse().ok())
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// One worker's record, with timings in ms since the experiment started.
fn rec(idx: usize, what: &str, t0: Instant, extra: Value) -> Value {
    let mut v = json!({ "worker": idx, "what": what, "at_ms": (t0.elapsed().as_secs_f64() * 1000.0).round() });
    if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
        for (k, x) in e {
            o.insert(k.clone(), x.clone());
        }
    }
    v
}

fn ms(t: Instant) -> f64 {
    (t.elapsed().as_secs_f64() * 1000.0).round()
}

fn sign_in(app: &App) -> Result<(Box<dyn Backend>, f64), String> {
    let creds = crate::current_creds(app).map_err(|e| e.1)?;
    let t = Instant::now();
    let be = build_live(&creds)?;
    Ok((be, ms(t)))
}

/// Read only: N sessions sign in and read the enrolled list at once.
fn run_reads(app: &App, n: usize, term_row: u32) -> Vec<Value> {
    let t0 = Instant::now();
    std::thread::scope(|s| {
        let hs: Vec<_> = (0..n)
            .map(|i| {
                s.spawn(move || {
                    let (mut be, login_ms) = match sign_in(app) {
                        Ok(x) => x,
                        Err(m) => return json!({ "worker": i, "ok": false, "error": log::trunc(&m, 200), "at_ms": ms(t0) }),
                    };
                    let t = Instant::now();
                    let r = be.tool("enrolled_classes", &json!({ "row": term_row }));
                    json!({ "worker": i, "ok": r.is_ok(), "login_ms": login_ms, "read_ms": ms(t), "classes": r.as_ref().ok().and_then(|v| v["classes"].as_array().map(|a| a.len())), "error": r.err().map(|m| log::trunc(&m, 200)), "done_at_ms": ms(t0) })
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap_or_else(|_| json!({ "ok": false, "error": "worker panicked" }))).collect()
    })
}

/// Drop one class on its own session: sign in, preview (checked to be Winter 2027 and exactly that class), confirm.
fn drop_one(app: &App, idx: usize, term_row: u32, nbr: &str, t0: Instant, shared: Option<&mut Box<dyn Backend>>) -> Value {
    let mut own;
    let (be, login_ms): (&mut Box<dyn Backend>, f64) = match shared {
        Some(b) => (b, 0.0),
        None => match sign_in(app) {
            Ok((b, l)) => {
                own = b;
                (&mut own, l)
            }
            Err(m) => return rec(idx, "login_failed", t0, json!({ "class": nbr, "error": log::trunc(&m, 200) })),
        },
    };
    be.arm();
    let tp = Instant::now();
    let pv = match be.tool("preview_drop", &json!({ "term_row": term_row, "class_nbr": nbr })) {
        Ok(p) => p,
        Err(m) => return rec(idx, "preview_failed", t0, json!({ "class": nbr, "login_ms": login_ms, "error": log::trunc(&m, 300) })),
    };
    let preview_ms = ms(tp);
    let plan = &pv["plan"];
    let term_ok = plan["term"].as_str() == Some(TERM);
    let nums_ok = plan["class_numbers"].as_array().map(|a| a.len() == 1 && a[0].as_str() == Some(nbr)).unwrap_or(false);
    let (Some(token), Some(phrase)) = (pv["token"].as_str(), plan["required_phrase"].as_str()) else {
        return rec(idx, "no_token", t0, json!({ "class": nbr }));
    };
    if !term_ok || !nums_ok {
        return rec(idx, "refused_by_lab", t0, json!({ "class": nbr, "term_ok": term_ok, "plan_names_only_this_class": nums_ok }));
    }
    let tc = Instant::now();
    let c = be.confirm(token, phrase);
    let confirm_ms = ms(tc);
    be.disarm();
    match c {
        Ok(o) => rec(idx, "confirmed", t0, json!({ "class": nbr, "login_ms": login_ms, "preview_ms": preview_ms, "confirm_ms": confirm_ms, "stopped_before_yes": o["detail"]["stopped_before_yes"], "messages": o["messages"], "done_at_ms": ms(t0) })),
        Err(f) => rec(idx, "confirm_error", t0, json!({ "class": nbr, "login_ms": login_ms, "preview_ms": preview_ms, "confirm_ms": confirm_ms, "may_have_sent": f.may_have_sent, "error": log::trunc(&f.message, 300) })),
    }
}

fn run_drop(app: &App, parallel: bool, term_row: u32, nbrs: &[String]) -> Vec<Value> {
    let t0 = Instant::now();
    if parallel {
        std::thread::scope(|s| {
            let hs: Vec<_> = nbrs.iter().enumerate().map(|(i, n)| s.spawn(move || drop_one(app, i, term_row, n, t0, None))).collect();
            hs.into_iter().map(|h| h.join().unwrap_or_else(|_| json!({ "ok": false, "error": "worker panicked" }))).collect()
        })
    } else {
        // One extra session used one after another (signed in once), so serial and parallel are compared like with like.
        let mut shared = match sign_in(app) {
            Ok((b, _)) => b,
            Err(m) => return vec![json!({ "error": log::trunc(&m, 200) })],
        };
        nbrs.iter().enumerate().map(|(i, n)| drop_one(app, i, term_row, n, t0, Some(&mut shared))).collect()
    }
}

/// Enrol one pick on its own session: the normal prepare (seat checks, leftover check, stage, preview) and the normal confirm.
fn enrol_one(app: &App, idx: usize, pick: &Pick, t0: Instant, hold_submit: bool, skip_leftovers: bool) -> (Value, Option<(WriteState, String)>) {
    let (mut be, login_ms) = match sign_in(app) {
        Ok(x) => x,
        Err(m) => return (rec(idx, "login_failed", t0, json!({ "course": pick.course, "error": log::trunc(&m, 200) })), None),
    };
    be.arm();
    let mut st = WriteState { backend: Some(be), armed: true, last_used: Some(Instant::now()), ..Default::default() };
    let stop = AtomicBool::new(false);
    let tp = Instant::now();
    let prep = crate::write::prepare_enrol_lab(&mut st, std::slice::from_ref(pick), &stop, skip_leftovers);
    let prepare_ms = ms(tp);
    let prep = match prep {
        Ok(v) => v,
        Err(e) => return (rec(idx, "prepare_error", t0, json!({ "course": pick.course, "login_ms": login_ms, "prepare_ms": prepare_ms, "error": log::trunc(&e.1, 400) })), None),
    };
    if prep["blocked"] == "leftovers" {
        return (rec(idx, "blocked_leftovers", t0, json!({ "course": pick.course, "login_ms": login_ms, "prepare_ms": prepare_ms, "leftovers": prep["leftovers"] })), None);
    }
    let Some(token) = prep["submit"]["token"].as_str().map(String::from) else {
        return (rec(idx, "no_submit_token", t0, json!({ "course": pick.course })), None);
    };
    if hold_submit {
        return (rec(idx, "staged", t0, json!({ "course": pick.course, "login_ms": login_ms, "prepare_ms": prepare_ms, "staged": prep["staged"] })), Some((st, token)));
    }
    let tc = Instant::now();
    let out = confirm_token(&mut st, &token, None, false, &stop);
    let r = match out {
        Ok(v) => rec(idx, "submitted", t0, json!({ "course": pick.course, "login_ms": login_ms, "prepare_ms": prepare_ms, "submit_ms": ms(tc), "unknown": v["unknown"], "verify": v["verify"], "messages": v["messages"], "done_at_ms": ms(t0) })),
        Err(e) => rec(idx, "submit_error", t0, json!({ "course": pick.course, "login_ms": login_ms, "prepare_ms": prepare_ms, "submit_ms": ms(tc), "error": log::trunc(&e.1, 400) })),
    };
    (r, None)
}

fn run_enrol(app: &App, parallel: bool, picks: &[Pick]) -> Vec<Value> {
    let t0 = Instant::now();
    if parallel {
        // The leftover check is skipped here on purpose: if the cart is shared between sessions, the other sessions' staged
        // courses WILL show up, and what we want to learn is what happens then, not to be stopped by it. The final-step
        // check still runs per session and its answer is reported.
        std::thread::scope(|s| {
            let hs: Vec<_> = picks.iter().enumerate().map(|(i, p)| s.spawn(move || enrol_one(app, i, p, t0, false, true).0)).collect();
            hs.into_iter().map(|h| h.join().unwrap_or_else(|_| json!({ "ok": false, "error": "worker panicked" }))).collect()
        })
    } else {
        picks.iter().enumerate().map(|(i, p)| enrol_one(app, i, p, t0, false, false).0).collect()
    }
}

/// Does a second session see what a first session staged? Session A stages and previews (nothing submitted yet); session B
/// reads the Plan & Enrol list; then A submits.
fn run_cart_isolation(app: &App, pick: &Pick) -> Vec<Value> {
    let t0 = Instant::now();
    let (a, held) = enrol_one(app, 0, pick, t0, true, false);
    let mut out = vec![a];
    let Some((mut st_a, token)) = held else { return out };
    match sign_in(app) {
        Ok((mut b, _)) => {
            let r = b.tool("plan_class_list", &json!({}));
            let chosen: Vec<Value> = r
                .as_ref()
                .ok()
                .and_then(|v| v["rows"].as_array().cloned())
                .unwrap_or_default()
                .into_iter()
                .filter(|row| row["status"].as_str().unwrap_or("").eq_ignore_ascii_case("classes available") && !row["schedule_summary"].as_str().unwrap_or("").trim().is_empty())
                .map(|row| json!(row["course_label"]))
                .collect();
            out.push(rec(1, "second_session_sees_chosen", t0, json!({ "chosen_courses_seen_by_B": chosen, "staged_by_A": pick.course, "error": r.err() })));
        }
        Err(m) => out.push(rec(1, "login_failed", t0, json!({ "error": log::trunc(&m, 200) }))),
    }
    let stop = AtomicBool::new(false);
    let tc = Instant::now();
    out.push(match confirm_token(&mut st_a, &token, None, false, &stop) {
        Ok(v) => rec(0, "submitted", t0, json!({ "submit_ms": ms(tc), "unknown": v["unknown"], "verify": v["verify"], "messages": v["messages"] })),
        Err(e) => rec(0, "submit_error", t0, json!({ "error": log::trunc(&e.1, 400) })),
    });
    out
}

pub async fn run(State(app): State<App>, Json(req): Json<LabReq>) -> Result<Json<Value>, ApiErr> {
    let Some(until) = lab_until() else { return Err(deny(StatusCode::NOT_FOUND, "The lab is not enabled for this server run.")) };
    if now_secs() >= until {
        return Err(deny(StatusCode::FORBIDDEN, "The lab window has ended."));
    }
    if app.demo {
        return Err(deny(StatusCode::CONFLICT, "The lab measures the real site; it does not run in demo mode."));
    }
    crate::current_creds(&app)?;
    if !app.snap.lock().map(|s| s.armed).unwrap_or(false) {
        return Err(deny(StatusCode::CONFLICT, "Switch writes on through the consent screen first."));
    }
    let n = req.sessions.clamp(0, 4);
    if req.experiment != "reads" && req.class_nbrs.len() + req.picks.len() > 6 {
        return Err(deny(StatusCode::BAD_REQUEST, "At most 6 items per experiment."));
    }
    let a = app.clone();
    let remaining = until.saturating_sub(now_secs());
    let v = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let _one = ONE_AT_A_TIME.lock().map_err(|_| "lab lock poisoned".to_string())?;
        let t = Instant::now();
        let parallel = req.mode == "parallel";
        let rows = match req.experiment.as_str() {
            "reads" => run_reads(&a, n.max(1), req.term_row),
            "drop" => run_drop(&a, parallel, req.term_row, &req.class_nbrs),
            "cart_isolation" => match req.picks.first() {
                Some(p) => run_cart_isolation(&a, p),
                None => return Err("cart_isolation needs one pick".into()),
            },
            "enrol" => run_enrol(&a, parallel, &req.picks),
            other => return Err(format!("unknown experiment {other}")),
        };
        log::event("lab_run", json!({ "experiment": req.experiment, "mode": req.mode, "ms": ms(t), "workers": rows.len() }));
        Ok(json!({ "experiment": req.experiment, "mode": req.mode, "wall_ms": ms(t), "seconds_left_in_window": remaining, "rows": rows }))
    })
    .await
    .map_err(|_| deny(StatusCode::INTERNAL_SERVER_ERROR, "lab task failed"))?
    .map_err(|m| ApiErr(StatusCode::BAD_REQUEST, m))?;
    Ok(Json(v))
}
