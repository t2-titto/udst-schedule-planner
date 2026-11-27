use super::error::err;
use super::planning::{
    apply_recount, check_credits, check_dependencies, check_plan_request, check_plans, clean_args, nums_of, plan_preview,
    preview_checked, sorted, stash_checked, ApplyReq, Pick, RoundOpts, SwapApplyReq, SwapItem, ToolReq,
};
use super::reads::{code_of, PREVIEW_TOOLS};
use super::state::{
    armed_backend, build_demo_with_state, build_live_with_said, step, with_state, Backend, SaidBuf, WriteState,
    PRESTAGE_TTL,
};
use crate::demo::DemoState;
use crate::seats::{self, SeatCheck};
use crate::{log, ApiErr, App, Creds};
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const PARALLEL_MAX: usize = 5;

#[derive(Clone)]
pub struct JobCtx {
    pub job: Arc<Mutex<Option<Value>>>,
    pub fired: Arc<Mutex<Vec<Value>>>,
    pub said: Option<SaidBuf>,
    pub demo_state: Option<Arc<Mutex<DemoState>>>,
    pub demo: bool,
}

impl JobCtx {
    pub fn from_app(app: &App) -> Self {
        Self {
            job: Arc::clone(&app.job),
            fired: Arc::clone(&app.fired),
            said: Some(Arc::clone(&app.said)),
            demo_state: Some(Arc::clone(&app.demo_state)),
            demo: app.demo,
        }
    }

    #[cfg(test)]
    pub fn for_test() -> Self {
        Self {
            job: Arc::new(Mutex::new(None)),
            fired: Arc::new(Mutex::new(Vec::new())),
            said: None,
            demo_state: None,
            demo: false,
        }
    }

    fn update(&self, f: impl FnOnce(&mut Value)) {
        if let Ok(mut j) = self.job.lock() {
            if let Some(v) = j.as_mut() {
                f(v);
            }
        }
    }

    fn item(&self, id: &str, status: &str, note: &str) {
        self.update(|j| {
            if let Some(a) = j["items"].as_array_mut() {
                if let Some(it) = a.iter_mut().find(|x| x["id"] == id) {
                    it["status"] = json!(status);
                    it["note"] = json!(note);
                }
            }
        });
    }

    fn status_of(&self, id: &str) -> String {
        self.job
            .lock()
            .ok()
            .and_then(|j| {
                j.as_ref().and_then(|v| {
                    v["items"]
                        .as_array()
                        .and_then(|a| a.iter().find(|x| x["id"] == id).and_then(|x| x["status"].as_str().map(String::from)))
                })
            })
            .unwrap_or_default()
    }

    fn failed_step(&self, courses: &[String]) -> Option<(String, &'static str)> {
        courses.iter().find_map(|c| {
            if self.status_of(&format!("swap:{c}")) == "error" {
                Some((c.clone(), "swap"))
            } else if self.status_of(&format!("drop:{c}")) == "error" {
                Some((c.clone(), "drop"))
            } else {
                None
            }
        })
    }

    fn helper_backend(&self, creds: &Option<Creds>) -> Result<Box<dyn Backend>, String> {
        if self.demo {
            let ds = self.demo_state.clone().ok_or_else(|| "missing demo state".to_string())?;
            Ok(build_demo_with_state(ds))
        } else {
            let c = creds.as_ref().ok_or_else(|| "Sign in first.".to_string())?;
            build_live_with_said(c, self.said.clone())
        }
    }
}

pub async fn preview(State(app): State<App>, Json(req): Json<ToolReq>) -> Result<Json<Value>, ApiErr> {
    if !PREVIEW_TOOLS.contains(&req.tool.as_str()) {
        return Err(err(StatusCode::BAD_REQUEST, "Unknown preview."));
    }
    let args = clean_args(&req.args)?;
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
    })
    .await?;
    Ok(Json(v))
}

#[derive(Deserialize)]
pub struct ConfirmReq {
    pub token: String,
    #[serde(default)]
    pub phrase: Option<String>,
}

fn list_row<'a>(list: &'a Value, course: &str) -> Option<&'a Value> {
    list["rows"].as_array()?.iter().find(|r| code_of(r["course_label"].as_str().unwrap_or("")) == course)
}

pub(super) fn verify_enrolled(be: &mut dyn Backend, checks: &[SeatCheck]) -> Value {
    let t = Instant::now();
    match be.tool("plan_class_list", &json!({})) {
        Ok(list) => {
            let rows: Vec<Value> = checks
                .iter()
                .map(|c| {
                    let status = list_row(&list, &c.course).and_then(|r| r["status"].as_str()).unwrap_or("not on the list");
                    let waitlisted = c.waitlist && seats::is_wait_status(status);
                    json!({
                        "course": c.course,
                        "status": status,
                        "enrolled": !c.waitlist && status.eq_ignore_ascii_case("enrolled"),
                        "waitlisted": waitlisted
                    })
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

#[derive(Debug)]
enum Proof {
    Absent { remaining: BTreeSet<String>, term_left: bool },
    Present { remaining: BTreeSet<String> },
    Unreadable(String),
}

pub(super) fn listed_numbers(l: &Value) -> Option<BTreeSet<String>> {
    let a = l["classes"].as_array()?;
    Some(a.iter().filter_map(|c| c["class_nbr"].as_str().map(String::from)).collect())
}

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
            "result": match &proof {
                Proof::Absent { term_left: true, .. } => "absent_term_left",
                Proof::Absent { .. } => "absent",
                Proof::Present { .. } => "present",
                Proof::Unreadable(_) => "unreadable"
            },
        }),
    );
    proof
}

fn proof_json(class_nbr: &str, p: &Proof) -> Value {
    match p {
        Proof::Absent { remaining, term_left } => {
            json!({ "readAtMs": log::now_ms(), "classNbr": class_nbr, "state": "absent", "termLeft": term_left, "stillListed": false, "remaining": remaining })
        }
        Proof::Present { remaining } => {
            json!({ "readAtMs": log::now_ms(), "classNbr": class_nbr, "state": "still_present", "stillListed": true, "remaining": remaining })
        }
        Proof::Unreadable(m) => json!({ "classNbr": class_nbr, "state": "unknown", "error": log::trunc(m, 200) }),
    }
}

pub fn confirm_token(st: &mut WriteState, token: &str, typed: Option<&str>, phrase_mode: bool, stop: &AtomicBool) -> Result<Value, ApiErr> {
    let off = || err(StatusCode::CONFLICT, "Writes were switched off before this was sent. Nothing was sent.");
    if stop.load(Ordering::SeqCst) {
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
    let will_pass = !phrase_mode || typed == Some(required.as_str());
    let phrase = if phrase_mode {
        typed.ok_or_else(|| err(StatusCode::BAD_REQUEST, "Type the phrase shown."))?.to_string()
    } else {
        required
    };
    st.touch();

    if will_pass {
        if let Some(checks) = st.checks.get(token).cloned().filter(|c| !c.is_empty()) {
            let t = Instant::now();
            let verdict = match st.backend.as_mut() {
                Some(be) => seats::check_all(be.as_mut(), &checks),
                None => return Err(err(StatusCode::CONFLICT, "Writes are off. Enable them first.")),
            };
            log::event(
                "confirm_seat_check",
                json!({ "ms": log::ms_since(t), "courses": checks.iter().map(|c| c.course.clone()).collect::<Vec<_>>(), "blocked": verdict.is_err() }),
            );
            if let Err(msg) = verdict {
                st.phrases.remove(token);
                st.checks.remove(token);
                return Err(super::planning::seat_block(msg));
            }
        }
    }

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
            v["said"] = json!(st.take_said(&["drop_click", "drop_yes", "drop_result", "enrol_submit", "stage_option", "swap_commit"]));
            if v["kind"].as_str() == Some("ENROLL") {
                if st.defer_verify {
                    st.pending_verify = spent.clone();
                    v["verify_deferred"] = json!(true);
                } else if let (Some(be), Some(checks)) = (st.backend.as_mut(), spent.as_ref()) {
                    v["verify"] = verify_enrolled(be.as_mut(), checks);
                }
            }
            match v["kind"].as_str() {
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
            log::event(
                "confirm",
                json!({ "ms": confirm_ms, "ok": false, "error": log::trunc(&fail.message, 300), "may_have_sent": fail.may_have_sent }),
            );
            let keep = fail.message.contains("did not match");
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
                let verify = match (st.backend.as_mut(), spent.as_ref()) {
                    (Some(be), Some(checks)) => verify_enrolled(be.as_mut(), checks),
                    _ => json!({ "error": "PeopleSoft's list could not be read afterwards." }),
                };
                log::event("enrol_unknown", json!({ "error": log::trunc(&fail.message, 300), "rows": verify["rows"] }));
                let said = st.take_said(&["enrol_submit"]);
                return Ok(json!({ "kind": "ENROLL", "unknown": true, "error": fail.message, "classNumbers": [], "messages": [], "detail": null, "verify": verify, "said": said }));
            }
            if fail.may_have_sent && drop_ctx.is_some() {
                let (term_row, nbr, term) = drop_ctx.clone().unwrap_or_default();
                let proof = match st.backend.as_mut() {
                    Some(be) => read_proof(be.as_mut(), "drop", "enrolled_after_unclear_answer", term_row, &nbr, &term),
                    None => Proof::Unreadable("writes are off".into()),
                };
                let said = st.take_said(&["drop_click", "drop_yes", "drop_result"]);
                log::event("drop_unknown", json!({ "error": log::trunc(&fail.message, 300), "state": proof_json(&nbr, &proof)["state"] }));
                return Ok(json!({ "kind": "DROP", "unknown": true, "error": fail.message, "classNumbers": [nbr], "messages": [], "detail": null, "verify": proof_json(&nbr, &proof), "said": said }));
            }
            let said = st.take_said(&["drop_click", "drop_yes", "drop_result", "enrol_submit", "stage_option", "swap_commit"]);
            if said.is_empty() {
                Err(err(StatusCode::UNPROCESSABLE_ENTITY, fail.text()))
            } else {
                Err(err(StatusCode::UNPROCESSABLE_ENTITY, format!("{} [PeopleSoft said: {}]", fail.text(), said.join(" | "))))
            }
        }
    }
}

pub async fn confirm(State(app): State<App>, Json(req): Json<ConfirmReq>) -> Result<Json<Value>, ApiErr> {
    let phrase_mode = app.confirm_phrase;
    let stop = app.stop.clone();
    let v = with_state(&app, "confirm", move |st| confirm_token(st, &req.token, req.phrase.as_deref(), phrase_mode, &stop)).await?;
    Ok(Json(v))
}

pub(super) fn verify_selected(list: &Value, course: &str) -> Result<(), String> {
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
        Err(format!(
            "PeopleSoft does not show {course} as selected (it still says \"{status}\" with no selected class), so the stage step did not take effect. Nothing was submitted."
        ))
    }
}

fn is_chosen(status: &str, summary: &str) -> bool {
    status.trim().eq_ignore_ascii_case("classes available") && !summary.trim().is_empty()
}

pub(super) fn live_leftovers(list: &Value, picks: &[Pick]) -> Vec<Value> {
    let want: BTreeSet<String> = picks.iter().map(|p| code_of(&p.course)).collect();
    list["rows"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter(|r| is_chosen(r["status"].as_str().unwrap_or(""), r["schedule_summary"].as_str().unwrap_or("")))
                .filter_map(|r| {
                    let label = r["course_label"].as_str().unwrap_or("");
                    let code = code_of(label);
                    (!want.contains(&code)).then(|| {
                        json!({
                            "course": code,
                            "title": label.split_once(':').map(|x| x.1.trim()).unwrap_or(""),
                            "schedule": log::trunc(r["schedule_summary"].as_str().unwrap_or(""), 120)
                        })
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn live_cart_mismatch(live: &[Value], picks: &[Pick]) -> Option<String> {
    let live_selected: BTreeSet<String> = live
        .iter()
        .filter(|r| is_chosen(r[1].as_str().unwrap_or(""), r[2].as_str().unwrap_or("")))
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

#[cfg(test)]
pub fn prepare_enrol(st: &mut WriteState, picks: &[Pick]) -> Result<Value, ApiErr> {
    prepare_enrol_stop(st, picks, &AtomicBool::new(false))
}

pub fn prepare_enrol_stop(st: &mut WriteState, picks: &[Pick], stop: &AtomicBool) -> Result<Value, ApiErr> {
    if picks.is_empty() || picks.len() > 10 {
        return Err(err(StatusCode::BAD_REQUEST, "Pick between 1 and 10 courses."));
    }
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
    let mut stage_reports = Vec::new();
    for (p, check) in picks.iter().zip(&checks) {
        let at = p.course.clone();
        let op = format!("enrol:{at}");
        if stop.load(Ordering::SeqCst) {
            return Err(stopped(&staged, &at, off));
        }
        let be = armed_backend(st)?;
        let list = step(&op, "class_list", || be.tool("plan_class_list", &json!({}))).map_err(|m| stopped(&staged, &at, &m))?;
        if let Some(prev) = &previous {
            verify_selected(&list, prev).map_err(|m| stopped(&staged, prev, &m))?;
        } else {
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
        let row = if p.row == u32::MAX {
            list_row(&list, &p.course)
                .and_then(|r| r["row"].as_u64())
                .map(|x| x as u32)
                .ok_or_else(|| stopped(&staged, &at, "the course is not on PeopleSoft's Plan & Enrol list."))?
        } else {
            p.row
        };
        let opened = step(&op, "open_course", || be.tool("plan_select_class", &json!({ "row": row }))).map_err(|m| stopped(&staged, &at, &m))?;
        if opened["sections"]["course"].as_str() != Some(p.course.as_str()) {
            return Err(stopped(&staged, &at, "PeopleSoft's class list has changed since you fetched it (a different course is on that row). Fetch again."));
        }
        let option_row = if p.option_row == u32::MAX {
            opened["sections"]["options"]
                .as_array()
                .and_then(|o| {
                    o.iter().find(|x| {
                        sorted(&nums_of(
                            &x["components"]
                                .as_array()
                                .map(|c| c.iter().map(|y| y["class_nbr"].clone()).collect::<Vec<_>>())
                                .map(Value::Array)
                                .unwrap_or(Value::Null),
                        )) == sorted(&p.class_nbrs)
                    })
                })
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
                o["components"]
                    .as_array()
                    .map(|c| c.iter().filter_map(|x| x["class_nbr"].as_str().map(String::from)).collect::<Vec<_>>())
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        if sorted(&live_nums) != sorted(&p.class_nbrs) {
            return Err(stopped(&staged, &at, "the option on PeopleSoft no longer has the class numbers you chose. Fetch again."));
        }
        let on_page = seats::evaluate(check, &opened["sections"], seats::now_ms());
        log::event(
            "seat_on_open_page",
            json!({ "course": at, "readings": on_page.readings.iter().map(|r| json!({ "class": r.class_nbr, "open": r.open, "status": r.option_status, "blocked": !r.ok() })).collect::<Vec<_>>() }),
        );
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
                "course": at, "class_numbers": p.class_nbrs,
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
    let reports = stage_reports;
    let sub = step("enrol", "preview_submit", || be.tool("preview_submit_enrol", &json!({}))).map_err(|m| stopped(&staged, "the final step", &m))?;
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
    let live = sub["plan"]["live_rows"].as_array().ok_or_else(|| {
        stopped(&staged, "the final step", "PeopleSoft's class list could not be read, so the final list cannot be proved. Nothing was submitted.")
    })?;
    if let Some(why) = live_cart_mismatch(live, picks) {
        return Err(stopped(&staged, "the final step", &why));
    }
    let courses: Vec<&str> = picks.iter().map(|p| p.course.as_str()).collect();
    let sub = stash_checked(st, sub, checks, &reports);
    log::event("enrol_prepared", json!({ "ms": log::ms_since(t_all), "courses": courses, "staged": staged }));
    Ok(json!({ "staged": staged, "submit": sub }))
}

pub fn bulk_drop_phrase(nums: &[String]) -> String {
    format!("CONFIRM DROP {}", nums.join(" "))
}

struct PendingDrop {
    idx: usize,
    nbr: String,
}

fn settle(results: &mut [Value], p: &PendingDrop, proof: Proof, halted: &mut Option<String>, remaining: &mut Option<BTreeSet<String>>) -> bool {
    let n = &p.nbr;
    let row = &mut results[p.idx];
    match proof {
        Proof::Absent { remaining: r, term_left } => {
            row["status"] = json!("dropped");
            row["note"] = if term_left {
                json!("PeopleSoft no longer lists this term on its drop page: no enrolled classes are left in it.")
            } else {
                Value::Null
            };
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
    armed_backend(st)?;
    let t_all = Instant::now();
    let mut results: Vec<Value> = Vec::new();
    let mut remaining: Option<BTreeSet<String>> = None;
    let mut halted: Option<String> = None;
    let mut term_name: Option<String> = None;
    let mut pending: Option<PendingDrop> = None;

    for n in nums {
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
        let _ = st.take_said(&["drop_click", "drop_yes", "drop_result", "drop_review"]);
        let t = Instant::now();
        let be = armed_backend(st)?;
        let preview = match step(&op, "preview", || be.tool("preview_drop", &json!({ "term_row": term_row, "class_nbr": n }))) {
            Ok(p) => p,
            Err(m) => {
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
            term_name = preview["plan"]["term"].as_str().filter(|t| !t.trim().is_empty()).map(String::from);
        }
        if let Some(p) = pending.take() {
            let proof = match preview["plan"]["listed_classes"].as_array() {
                Some(a) => {
                    let set: BTreeSet<String> = a.iter().filter_map(|x| x.as_str().map(String::from)).collect();
                    log::event("verify_dropped", json!({ "class_nbr": p.nbr, "result": if set.contains(&p.nbr) { "present" } else { "absent" }, "via": "next_preview" }));
                    if set.contains(&p.nbr) {
                        Proof::Present { remaining: set }
                    } else {
                        Proof::Absent { remaining: set, term_left: false }
                    }
                }
                None => read_proof(&mut **be, &format!("drop:{}", p.nbr), "verify_enrolled_list", term_row, &p.nbr, &term_name),
            };
            if !settle(&mut results, &p, proof, &mut halted, &mut remaining) {
                results.push(json!({ "classNbr": n, "status": "skipped", "note": halted }));
                continue;
            }
            if remaining.as_ref().is_some_and(|r| !r.contains(n)) {
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
            Err(f) if f.may_have_sent => match read_proof(&mut **be, &op, "enrolled_after_unclear_answer", term_row, n, &term_name) {
                Proof::Absent { remaining: r, term_left } => {
                    let note = if term_left {
                        "PeopleSoft's answer was cut off, but its drop page no longer lists this term: no enrolled classes are left in it."
                    } else {
                        "PeopleSoft's answer was cut off, but its enrolled list no longer shows this class, so it was dropped."
                    };
                    remaining = Some(r);
                    results.push(json!({ "classNbr": n, "status": "dropped", "note": note, "unclear": true, "ms": log::ms_since(t) }));
                }
                Proof::Present { remaining: r } => {
                    remaining = Some(r);
                    halted = Some(format!("stopped: {f}; PeopleSoft's enrolled list still shows class {n}."));
                    results.push(json!({ "classNbr": n, "status": "unknown", "note": format!("{} PeopleSoft's enrolled list still shows this class.", f.text()), "unclear": true, "ms": log::ms_since(t) }));
                }
                Proof::Unreadable(m) => {
                    halted = Some(format!("stopped: {f}; could not read the enrolled list afterwards."));
                    results.push(json!({ "classNbr": n, "status": "unknown", "note": format!("{} Could not read PeopleSoft's enrolled list afterwards ({}). Check PeopleSoft before retrying.", f.text(), log::trunc(&m, 200)), "ms": log::ms_since(t) }));
                }
            },
            Err(f) => {
                halted = Some(format!("stopped at class {n}: {}", f.message));
                results.push(json!({ "classNbr": n, "status": "failed", "note": f.message, "ms": log::ms_since(t) }));
            }
            Ok(out) => {
                if out["detail"]["stopped_before_yes"] == true {
                    let said = st.take_said(&["drop_click", "drop_yes", "drop_result", "drop_review"]);
                    halted = Some(format!("stopped at class {n}: nothing was dropped."));
                    results.push(json!({ "classNbr": n, "status": "stopped", "note": "PeopleSoft reported a problem before the final confirmation; nothing was dropped.", "messages": out["messages"], "said": said, "ms": log::ms_since(t) }));
                } else {
                    let said = st.take_said(&["drop_click", "drop_yes", "drop_result", "drop_review"]);
                    let idx = results.len();
                    results.push(json!({ "classNbr": n, "status": "pending_verify", "messages": out["messages"], "said": said, "ms": log::ms_since(t) }));
                    pending = Some(PendingDrop { idx, nbr: n.clone() });
                }
            }
        }
    }

    if let Some(p) = pending.take() {
        let be = armed_backend(st)?;
        let proof = read_proof(&mut **be, &format!("drop:{}", p.nbr), "verify_enrolled_list", term_row, &p.nbr, &term_name);
        settle(&mut results, &p, proof, &mut halted, &mut remaining);
    }
    log::event(
        "bulk_drop_done",
        json!({ "ms": log::ms_since(t_all), "count": nums.len(), "halted": halted.is_some(), "statuses": results.iter().map(|r| r["status"].clone()).collect::<Vec<_>>() }),
    );
    Ok(json!({ "results": results, "halted": halted, "remaining": remaining, "ms": log::ms_since(t_all) }))
}

fn swap_code(text: &str) -> String {
    let mut it = text.split_whitespace();
    match (it.next(), it.next()) {
        (Some(a), Some(b)) => format!("{a} {b}"),
        _ => text.to_string(),
    }
}

pub(super) fn class_numbers_in(text: &str) -> Vec<String> {
    text.split("Class")
        .skip(1)
        .filter_map(|p| p.trim_start().split(|c: char| !c.is_ascii_digit()).next().filter(|d| !d.is_empty()).map(String::from))
        .collect()
}

pub(super) fn swap_status(held: &BTreeSet<String>, sw: &SwapItem) -> (&'static str, &'static str) {
    let new_in = sw.new_nbrs.iter().all(|n| held.contains(n));
    let old_left = sw.old_nbrs.iter().filter(|n| !sw.new_nbrs.contains(n)).any(|n| held.contains(n));
    match (new_in, old_left) {
        (true, false) => ("swapped", ""),
        (false, false) => ("not_enrolled", "Neither the new nor the old class is on your enrolled list."),
        (false, true) => ("unchanged", "PeopleSoft kept your old combination."),
        (true, true) => ("both", "Both the old and the new classes are on your list."),
    }
}

#[cfg(test)]
pub fn swap_apply(st: &mut WriteState, req: &SwapApplyReq, stop: &AtomicBool) -> Result<Value, ApiErr> {
    swap_round(st, req, stop, &RoundOpts { verify_list: true, prior: &[] })
}

pub fn swap_round(st: &mut WriteState, req: &SwapApplyReq, stop: &AtomicBool, opts: &RoundOpts) -> Result<Value, ApiErr> {
    if req.swaps.is_empty() || req.swaps.len() > 6 {
        return Err(err(StatusCode::BAD_REQUEST, "Pick between 1 and 6 courses to swap."));
    }
    for sw in &req.swaps {
        if sw.old_nbrs.is_empty()
            || sw.new_nbrs.is_empty()
            || sw.old_nbrs.iter().chain(&sw.new_nbrs).any(|n| n.is_empty() || n.len() > 10 || !n.bytes().all(|b| b.is_ascii_digit()))
        {
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
    let cands = step(op, "open_guide", || armed_backend(st)?.tool("swap_candidates", &json!({})).map_err(|m| err(StatusCode::UNPROCESSABLE_ENTITY, m)))?;
    let cand_list = cands.as_array().cloned().unwrap_or_default();
    let cand_rows: Vec<(u32, String)> = cand_list
        .iter()
        .filter_map(|c| Some((c["row"].as_u64()? as u32, swap_code(c["course"].as_str()?))))
        .collect();
    let prior_results: Vec<Value> = opts
        .prior
        .iter()
        .map(|sw| {
            let held: BTreeSet<String> = cand_list
                .iter()
                .filter(|c| c["course"].as_str().map(swap_code).as_deref() == Some(sw.course.as_str()))
                .flat_map(|c| class_numbers_in(c["class_text"].as_str().unwrap_or("")))
                .collect();
            let (status, note) = if held.is_empty() {
                ("unknown", "PeopleSoft's swap page did not list this course afterwards.")
            } else {
                swap_status(&held, sw)
            };
            json!({ "course": sw.course, "status": status, "note": note })
        })
        .collect();
    let mut staged: Vec<String> = Vec::new();
    for sw in &req.swaps {
        if stop.load(Ordering::SeqCst) {
            return Err(off());
        }
        let row = cand_rows
            .iter()
            .find(|(_, c)| *c == sw.course)
            .map(|(r, _)| *r)
            .ok_or_else(|| err(StatusCode::CONFLICT, format!("PeopleSoft does not offer {} for a swap right now. Nothing was sent.", sw.course)))?;
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
    if stop.load(Ordering::SeqCst) {
        return Err(off());
    }
    let commit = step(op, "preview_commit", || preview_checked(st, "preview_swap_commit", &json!({}), None, None))?;
    let token = commit["token"].as_str().ok_or_else(|| err(StatusCode::CONFLICT, "no preview was issued"))?.to_string();
    let out = step(op, "commit_confirm", || confirm_token(st, &token, None, false, stop))?;
    let rows: Vec<Value> = out["detail"]["results"].as_array().cloned().unwrap_or_default();
    let said_for = |sw: &SwapItem| -> String {
        rows.iter()
            .find(|r| r["class_nbr"].as_str().is_some_and(|n| sw.old_nbrs.iter().any(|o| o == n)))
            .and_then(|r| r["message"].as_str())
            .unwrap_or("")
            .to_string()
    };
    let said: Vec<String> = rows.iter().filter_map(|r| r["message"].as_str().map(String::from)).collect();
    let held: Option<BTreeSet<String>> = if opts.verify_list {
        let be = armed_backend(st)?;
        let terms = be.tool("drop_terms", &json!({})).map_err(|m| err(StatusCode::UNPROCESSABLE_ENTITY, m))?;
        let term_row = terms
            .as_array()
            .and_then(|a| a.iter().find(|t| t["term"].as_str().map(|x| x.eq_ignore_ascii_case(&req.term)).unwrap_or(false)))
            .and_then(|t| t["row"].as_u64())
            .map(|r| r as u32);
        term_row
            .and_then(|r| be.tool("enrolled_classes", &json!({ "row": r })).ok())
            .and_then(|l| listed_numbers(&l))
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
    log::event(
        "swap_apply_done",
        json!({ "ms": log::ms_since(t0), "courses": staged, "results": results.iter().map(|r| r["status"].clone()).collect::<Vec<_>>() }),
    );
    Ok(json!({ "results": results, "prior": prior_results, "said": said, "ms": log::ms_since(t0) }))
}

fn lab_state(be: Box<dyn Backend>, said: Option<SaidBuf>) -> WriteState {
    let mut be = be;
    be.arm();
    WriteState {
        backend: Some(be),
        armed: true,
        last_used: Some(Instant::now()),
        said_buf: said,
        ..Default::default()
    }
}

fn par_drops(ctx: &JobCtx, creds: &Option<Creds>, term_row: u32, nums: &[String], stop: &AtomicBool) -> Vec<Value> {
    std::thread::scope(|sc| {
        let hs: Vec<_> = nums
            .iter()
            .enumerate()
            .map(|(i, n)| {
                sc.spawn(move || {
                    let be = match ctx.helper_backend(creds) {
                        Ok(b) => b,
                        Err(m) => return json!({ "status": "failed", "note": log::trunc(&m, 200) }),
                    };
                    if !ctx.demo {
                        std::thread::sleep(Duration::from_millis(800 * i as u64));
                    }
                    let worker_said = Arc::new(Mutex::new(Vec::new()));
                    let mut hst = lab_state(be, Some(worker_said));
                    let mut out = json!({});
                    for _ in 0..2 {
                        out = match drop_bulk(&mut hst, term_row, std::slice::from_ref(n), stop, false, None) {
                            Ok(v) => json!({ "status": v["results"][0]["status"], "note": v["results"][0]["note"], "said": v["results"][0]["said"] }),
                            Err(e) => json!({ "status": "failed", "note": e.1 }),
                        };
                        let retry = out["status"] == "failed" && out["note"].as_str().is_some_and(|x| x.contains("not this term's enrolled list"));
                        if !retry || stop.load(Ordering::SeqCst) {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(1500));
                    }
                    out
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap_or_else(|_| json!({ "status": "failed", "note": "panicked" }))).collect()
    })
}

type Held = (String, String, Instant, WriteState);

fn prestage_run(ctx: &JobCtx, creds: &Option<Creds>, picks: &[Pick], stop: &AtomicBool) -> Vec<(Value, Option<Held>)> {
    std::thread::scope(|sc| {
        let hs: Vec<_> = picks
            .iter()
            .enumerate()
            .map(|(i, pick)| {
                sc.spawn(move || {
                    let t = Instant::now();
                    let be = match ctx.helper_backend(creds) {
                        Ok(b) => b,
                        Err(m) => return (json!({ "course": pick.course, "status": "failed", "detail": log::trunc(&m, 200) }), None),
                    };
                    if !ctx.demo {
                        std::thread::sleep(Duration::from_millis(800 * i as u64));
                    }
                    let worker_said = Arc::new(Mutex::new(Vec::new()));
                    let mut hst = lab_state(be, Some(worker_said));
                    match prepare_enrol_stop(&mut hst, std::slice::from_ref(pick), stop) {
                        Ok(v) if v["blocked"] == "leftovers" => (
                            json!({ "course": pick.course, "status": "blocked", "detail": { "alreadyChosen": v["leftovers"] }, "ms": log::ms_since(t) }),
                            None,
                        ),
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

pub(super) fn par_enrol_status(r: &Value) -> (&'static str, Value) {
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

pub(super) fn confirmation_status(detail: &Value, course: &str) -> &'static str {
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

fn fire_spawn(ctx: &JobCtx, held: Vec<Held>, stop: &Arc<AtomicBool>) -> Vec<std::thread::JoinHandle<()>> {
    let mut handles = Vec::with_capacity(held.len());
    for (idx, (course, tok, at, mut hst)) in held.into_iter().enumerate() {
        let stop = stop.clone();
        let fired = Arc::clone(&ctx.fired);
        if let Ok(mut f) = fired.lock() {
            f.push(json!({ "course": course, "status": "sending", "verifying": true }));
        }
        handles.push(std::thread::spawn(move || {
            let set = |v: Value| {
                if let Ok(mut f) = fired.lock() {
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
                    let checks = hst.pending_verify.take().unwrap_or_default();
                    let verify = match hst.backend.as_mut() {
                        Some(be) => verify_enrolled(be.as_mut(), &checks),
                        None => json!({ "error": "session gone" }),
                    };
                    let r = json!({ "login_ok": true, "result": { "unknown": o["unknown"], "verify": verify } });
                    let (final_status, detail) = par_enrol_status(&r);
                    set(json!({ "course": course, "status": final_status, "detail": detail, "said": o["said"], "ms": log::ms_since(t), "confirmation": status, "verifying": false }));
                }
                Err(e) => set(json!({ "course": course, "status": "failed", "detail": e.1, "verifying": false })),
            }
        }));
    }
    handles
}

pub fn apply_run(st: &mut WriteState, ctx: &JobCtx, creds: &Option<Creds>, req: &ApplyReq, stop: &Arc<AtomicBool>) -> Result<(), ApiErr> {
    armed_backend(st)?;
    let t0 = Instant::now();
    let halted = || stop.load(Ordering::SeqCst);
    let rounds = req.all_rounds();
    let planning = !req.plans.is_empty();
    let early: Vec<usize> = req.enrols.iter().enumerate().filter(|(_, e)| !e.after_drop && !e.after_plan && !planning).map(|(i, _)| i).collect();
    let late: Vec<usize> = req.enrols.iter().enumerate().filter(|(_, e)| e.after_drop || e.after_plan || planning).map(|(i, _)| i).collect();
    let spawn_stage = |idx: &[usize]| {
        if idx.is_empty() {
            return None;
        }
        for &i in idx {
            ctx.item(&format!("enrol:{}", req.enrols[i].pick.course), "working", "staging in the background");
        }
        let picks: Vec<Pick> = idx.iter().map(|&i| req.enrols[i].pick.clone()).collect();
        let (ctx2, c2, sp) = (ctx.clone(), creds.clone(), stop.clone());
        Some(std::thread::spawn(move || prestage_run(&ctx2, &c2, &picks, &sp)))
    };
    let stage_early = spawn_stage(&early);
    for &i in &late {
        let why = if req.enrols[i].after_plan {
            "waits for its plan step"
        } else if planning {
            "waits for the plan steps"
        } else {
            "waits for its drop"
        };
        ctx.item(&format!("enrol:{}", req.enrols[i].pick.course), "pending", why);
    }

    // 1. Drops.
    if !req.drops.is_empty() {
        ctx.update(|j| j["phase"] = json!("dropping"));
        for d in &req.drops {
            ctx.item(&format!("drop:{}", d.course), "working", "");
        }
        let nums: Vec<String> = req.drops.iter().map(|d| d.class_nbr.clone()).collect();
        let res = par_drops(ctx, creds, req.term_row.unwrap_or(0), &nums, stop);
        for (d, r) in req.drops.iter().zip(res) {
            let ok = matches!(r["status"].as_str(), Some("dropped") | Some("gone"));
            ctx.item(
                &format!("drop:{}", d.course),
                if ok { "done" } else { "error" },
                if ok {
                    "Dropped. PeopleSoft's list no longer shows it."
                } else {
                    r["note"].as_str().unwrap_or("not dropped")
                },
            );
        }
    }

    // 1b. Plans (both standalone move-outs to free credits AND move-ins before afterPlan enrols).
    if planning {
        ctx.update(|j| j["phase"] = json!("planning"));
        for p in &req.plans {
            let id = format!("plan:{}", p.course);
            if halted() {
                ctx.item(&id, "error", "Stopped before it was sent.");
                continue;
            }
            ctx.item(&id, "working", "");
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
                Ok(_) => ctx.item(&id, "done", &format!("Planned into {} in PeopleSoft.", p.credits.term)),
                Err(e) => ctx.item(&id, "error", &format!("{} Nothing was enrolled for it.", e.1)),
            }
        }
    }

    let late_ok: Vec<usize> = late
        .iter()
        .copied()
        .filter(|&i| {
            let c = &req.enrols[i].pick.course;
            (req.drops.iter().all(|d| d.course != *c) || ctx.status_of(&format!("drop:{c}")) == "done")
                && (!req.enrols[i].after_plan || ctx.status_of(&format!("plan:{c}")) == "done")
        })
        .collect();
    for &i in &late {
        if !late_ok.contains(&i) {
            let e = &req.enrols[i];
            let why = if e.after_plan && ctx.status_of(&format!("plan:{}", e.pick.course)) != "done" {
                "Not sent: its plan step did not work, so PeopleSoft does not have it planned."
            } else {
                "Not sent: its drop did not work, so it is still enrolled."
            };
            ctx.item(&format!("enrol:{}", e.pick.course), "error", why);
        }
    }
    let stage_late = spawn_stage(&late_ok);

    // 2. Swaps, round by round.
    let mut previous: Vec<SwapItem> = Vec::new();
    let last = rounds.len().saturating_sub(1);
    for (ri, round) in rounds.iter().enumerate() {
        ctx.update(|j| j["phase"] = json!(if rounds.len() > 1 { format!("swapping, round {} of {}", ri + 1, rounds.len()) } else { "swapping".to_string() }));
        let mut go: Vec<SwapItem> = Vec::new();
        for sw in round {
            let id = format!("swap:{}", sw.course);
            if let Some((w, what)) = ctx.failed_step(&sw.waits_for) {
                ctx.item(&id, "error", &format!("Not sent: it has to wait for the {what} of {w}, which did not work."));
            } else {
                ctx.item(&id, "working", &if rounds.len() > 1 { format!("round {}", ri + 1) } else { String::new() });
                go.push(sw.clone());
            }
        }
        if go.is_empty() {
            continue;
        }
        if halted() {
            for sw in &go {
                ctx.item(&format!("swap:{}", sw.course), "error", "Stopped before it was sent.");
            }
            continue;
        }
        let r = swap_round(st, &SwapApplyReq { term: req.term.clone(), swaps: go.clone() }, stop, &RoundOpts { verify_list: ri == last, prior: &previous });
        match r {
            Ok(v) => {
                for pr in v["prior"].as_array().cloned().unwrap_or_default() {
                    let course = pr["course"].as_str().unwrap_or("");
                    let ok = pr["status"] == "swapped";
                    let hint = previous.iter().find(|x| x.course == course).map(|x| x.hint.clone()).unwrap_or_default();
                    ctx.item(
                        &format!("swap:{course}"),
                        if ok { "done" } else { "error" },
                        &if ok { "Swapped. PeopleSoft's list shows the new classes.".to_string() } else { format!("PeopleSoft kept your old combination. {hint}") },
                    );
                }
                for res in v["results"].as_array().cloned().unwrap_or_default() {
                    let course = res["course"].as_str().unwrap_or("");
                    match res["status"].as_str() {
                        Some("sent") => ctx.item(&format!("swap:{course}"), "working", "Sent; checked when the next step opens its page."),
                        Some("swapped") => ctx.item(&format!("swap:{course}"), "done", "Swapped. PeopleSoft's list shows the new classes."),
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
                            ctx.item(&format!("swap:{course}"), "error", &note);
                        }
                    }
                }
                previous = if ri == last { vec![] } else { go };
            }
            Err(e) => {
                for sw in &go {
                    ctx.item(&format!("swap:{}", sw.course), "error", &e.1);
                }
                previous = vec![];
            }
        }
    }

    // 3. Enrolments.
    let mut staged_all: Vec<(usize, Value, Option<Held>)> = Vec::new();
    for (handle, idx) in [(stage_early, &early), (stage_late, &late_ok)] {
        if let Some(h) = handle {
            for (i, (r, held)) in idx.iter().zip(h.join().unwrap_or_default()) {
                staged_all.push((*i, r, held));
            }
        }
    }
    if !staged_all.is_empty() {
        ctx.update(|j| j["phase"] = json!("enrolling"));
        let mut held: Vec<Held> = Vec::new();
        for (i, r, h) in staged_all {
            let e = &req.enrols[i];
            let id = format!("enrol:{}", e.pick.course);
            let failed_dep = ctx.failed_step(&e.depends);
            let dropped = req.drops.iter().any(|d| d.course == e.pick.course) && ctx.status_of(&format!("drop:{}", e.pick.course)) == "done";
            let after = if dropped { " Its drop already happened, so you are not enrolled in this course now." } else { "" };
            match (h, r["status"].as_str(), failed_dep) {
                (_, _, Some((d, what))) => ctx.item(&id, "error", &format!("Not sent: it needs the {what} of {d}, which did not work.{after}")),
                (Some(held_one), Some("staged"), None) => {
                    if halted() {
                        ctx.item(&id, "error", "Stopped before it was sent.");
                    } else {
                        held.push(held_one);
                    }
                }
                (_, Some("blocked"), None) => ctx.item(&id, "error", &format!("Not sent: other courses are already chosen in PeopleSoft's Plan & Enrol list.{after}")),
                _ => ctx.item(&id, "error", &format!("{}{after}", r["detail"].as_str().unwrap_or("could not be prepared"))),
            }
        }
        if !held.is_empty() {
            if let Ok(mut f) = ctx.fired.lock() {
                f.clear();
                for h in &held {
                    f.push(json!({ "course": h.0, "status": "sending", "verifying": true }));
                }
            }
            let names: Vec<String> = held.iter().map(|h| h.0.clone()).collect();
            let handles = fire_spawn(ctx, held, stop);
            let deadline = Instant::now() + Duration::from_secs(300);
            loop {
                let snap = ctx.fired.lock().map(|f| f.clone()).unwrap_or_default();
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
                        ctx.item(&id, "working", note);
                    } else if r["status"] == "enrolled" {
                        ctx.item(&id, "done", "Enrolled. PeopleSoft's own list shows it.");
                    } else if r["status"] == "waitlisted" {
                        ctx.item(&id, "done", "On the waitlist. You are NOT enrolled: PeopleSoft may enrol you later without asking.");
                    } else {
                        let again = if req.drops.iter().any(|d| d.course == *c) {
                            " It was dropped first, so you are not enrolled in this course now: enrol it again."
                        } else {
                            ""
                        };
                        let said_list: Vec<&str> = r["said"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default();
                        let ps_tail = if said_list.is_empty() { String::new() } else { format!(" [PeopleSoft said: {}]", said_list.join(" | ")) };
                        let why = match r["status"].as_str() {
                            Some("rejected") | Some("not_enrolled") => format!("PeopleSoft did not enrol it (a clash or a rule).{again}{ps_tail}"),
                            Some("unknown") => format!("Unclear: check PeopleSoft before retrying.{ps_tail}"),
                            _ => format!("{}{again}{ps_tail}", r["detail"].as_str().unwrap_or("failed")),
                        };
                        ctx.item(&id, "error", &why);
                    }
                }
                if all_done {
                    for h in handles {
                        let _ = h.join();
                    }
                    break;
                }
                if Instant::now() > deadline {
                    stop.store(true, Ordering::SeqCst);
                    break;
                }
                std::thread::sleep(Duration::from_millis(if ctx.demo { 20 } else { 500 }));
            }
        }
    }
    log::event("apply_done", json!({ "ms": log::ms_since(t0), "drops": req.drops.len(), "rounds": rounds.len(), "plans": req.plans.len(), "enrols": req.enrols.len() }));
    Ok(())
}

pub(super) fn new_job(req: &ApplyReq) -> Value {
    let mut items: Vec<Value> = vec![];
    for d in &req.drops {
        items.push(json!({ "id": format!("drop:{}", d.course), "kind": "drop", "course": d.course, "status": "pending", "note": "" }));
    }
    for p in &req.plans {
        items.push(json!({ "id": format!("plan:{}", p.course), "kind": "plan", "course": p.course, "status": "pending", "note": "" }));
    }
    for (ri, round) in req.all_rounds().iter().enumerate() {
        for sw in round {
            items.push(json!({ "id": format!("swap:{}", sw.course), "kind": "swap", "course": sw.course, "status": "pending", "note": "", "round": ri + 1 }));
        }
    }
    for e in &req.enrols {
        items.push(json!({ "id": format!("enrol:{}", e.pick.course), "kind": "enrol", "course": e.pick.course, "status": "pending", "note": "" }));
    }
    json!({ "id": log::now_ms(), "phase": "starting", "finished": false, "error": null, "items": items })
}

pub(super) fn settle_job(ctx: &JobCtx, r: &Result<(), ApiErr>) {
    ctx.update(|j| {
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
    if req.drops.len() > 8
        || swap_count > 6
        || req.all_rounds().iter().any(|r| r.len() > 6)
        || req.enrols.len() > PARALLEL_MAX
        || req.drops.len() + swap_count + req.enrols.len() + req.plans.len() == 0
    {
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
        return Err(err(
            StatusCode::CONFLICT,
            "This server was started to ask for a typed phrase on every write, which Apply does not support. Start it without PLANNER_CONFIRM=phrase.",
        ));
    }
    if !req.drops.is_empty() && req.term_row.is_none() {
        return Err(err(StatusCode::UNPROCESSABLE_ENTITY, "The page did not say which term the drops are in. Nothing was sent. Reload the page."));
    }
    {
        let mut seen = BTreeSet::new();
        if !req.drops.iter().all(|d| seen.insert(d.course.clone())) {
            return Err(err(StatusCode::BAD_REQUEST, "Drop a course once, as a whole, never class by class."));
        }
    }
    if req
        .drops
        .iter()
        .any(|d| d.class_nbr.is_empty() || d.class_nbr.len() > 10 || !d.class_nbr.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(err(StatusCode::BAD_REQUEST, "Class numbers must be digits."));
    }
    let creds = app.creds.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if !app.demo && creds.is_none() {
        return Err(err(StatusCode::UNAUTHORIZED, "Sign in first."));
    }
    {
        let mut j = app.job.lock().map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "internal error"))?;
        if j.as_ref().is_some_and(|v| v["finished"] == false) {
            return Err(err(StatusCode::CONFLICT, "A plan is already running. Wait for it to finish."));
        }
        *j = Some(new_job(&req));
    }
    let stop = app.stop.clone();
    let ctx = JobCtx::from_app(&app);
    let a = app.clone();
    let demo = app.demo;
    tokio::spawn(async move {
        let ctx_inner = ctx.clone();
        let r = with_state(&a, "apply", move |st| {
            if !demo {
                apply_recount(st, &req)?;
            }
            let res = apply_run(st, &ctx_inner, &creds, &req, &stop);
            st.disarm();
            res
        })
        .await;
        settle_job(&ctx, &r);
        a.stop.store(true, Ordering::SeqCst);
        if let Ok(mut s) = a.snap.lock() {
            s.armed = false;
        }
        if let Ok(mut p) = a.prestaged.lock() {
            p.clear();
        }
    });
    Ok(Json(json!({ "started": true })))
}

pub async fn apply_progress(State(app): State<App>) -> Result<Json<Value>, ApiErr> {
    if !app.demo && app.creds.lock().unwrap_or_else(|e| e.into_inner()).is_none() {
        return Err(err(StatusCode::UNAUTHORIZED, "Sign in first."));
    }
    let val = app
        .job
        .lock()
        .ok()
        .and_then(|j| j.clone())
        .unwrap_or_else(|| json!({ "finished": true, "items": [] }));
    Ok(Json(val))
}
