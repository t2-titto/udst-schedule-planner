use super::error::err;
use super::planning::{clean_args, ToolReq};
use super::state::{build_demo_with_state, build_live_with_said, live, read_tool, step, with_state, Backend, WriteState};
use crate::seats::safe_key;
use crate::{log, ApiErr, App, Creds};
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const READ_TOOLS: [&str; 14] = [
    "catalog_search",
    "catalog_detail",
    "catalog_sections",
    "degree_tree",
    "degree_requirement",
    "degree_course_list",
    "plan_overview",
    "plan_expand_requirement",
    "plan_class_list",
    "plan_select_class",
    "drop_terms",
    "enrolled_classes",
    "swap_candidates",
    "swap_options",
];

pub const PREVIEW_TOOLS: [&str; 4] = [
    "preview_drop",
    "preview_swap_stage",
    "preview_swap_commit",
    "preview_plan_term",
];

pub const LIST_FRESH: Duration = Duration::from_secs(10 * 60);
const MAX_HELPERS: usize = 4;

pub type HelperMaker<'a> = &'a (dyn Fn() -> Result<Box<dyn Backend>, String> + Sync);

pub fn parallel_allowed(helpers: usize, writes_armed: bool, switched_off: bool) -> bool {
    helpers >= 2 && !writes_armed && !switched_off
}

pub(super) fn empty_list_is_not_trusted(prev: Option<&Value>, now: &Value) -> Result<(), ApiErr> {
    let n = |v: &Value| v["rows"].as_array().map_or(0, |a| a.len());
    if n(now) == 0 && prev.is_some_and(|p| n(p) > 0) {
        return Err(err(
            StatusCode::BAD_GATEWAY,
            "PeopleSoft returned an empty class list. It was not trusted, so nothing changed here. Try again in a moment.",
        ));
    }
    Ok(())
}

pub fn code_of(label: &str) -> String {
    label.split(':').next().unwrap_or("").trim().to_string()
}

fn course_row(list: &Value, row: u32) -> Option<&Value> {
    list["rows"].as_array().and_then(|a| a.iter().find(|r| r["row"].as_u64() == Some(u64::from(row))))
}

fn helper_factory(app: &App, creds: Option<Creds>) -> Box<dyn Fn() -> Result<Box<dyn Backend>, String> + Send + Sync> {
    let demo = app.demo;
    let ds = app.demo_state.clone();
    let said = app.said.clone();
    Box::new(move || {
        if demo {
            Ok(build_demo_with_state(Arc::clone(&ds)))
        } else {
            match &creds {
                Some(c) => build_live_with_said(c, Some(Arc::clone(&said))),
                None => Err("Sign in first.".into()),
            }
        }
    })
}

pub async fn read(State(app): State<App>, Json(req): Json<ToolReq>) -> Result<Json<Value>, ApiErr> {
    if !READ_TOOLS.contains(&req.tool.as_str()) {
        return Err(err(StatusCode::BAD_REQUEST, "Unknown read."));
    }
    let args = clean_args(&req.args)?;
    let demo = app.demo;
    let creds = app.creds.lock().unwrap_or_else(|e| e.into_inner()).clone();
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

pub async fn classes(app: &App) -> Result<Value, ApiErr> {
    let demo = app.demo;
    let creds = app.creds.lock().unwrap_or_else(|e| e.into_inner()).clone();
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
    log::event(
        "parallel_open",
        json!({ "course": code, "ms": log::ms_since(t), "ok": r.is_ok(), "error": r.as_ref().err().map(|e| log::trunc(e, 200)) }),
    );
    r
}

pub fn fetch_sections(
    st: &mut WriteState,
    creds: &Option<Creds>,
    demo: bool,
    rows: &[u32],
    known: &HashMap<String, Value>,
    helpers: usize,
    make: Option<HelperMaker>,
) -> Result<Value, ApiErr> {
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
                let same = d["course"].as_str() == Some(code.as_str())
                    && (term.is_empty() || d["term"].as_str() == Some(term.as_str()))
                    && d["options"].is_array();
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
            Err(_) => helper_failed = true,
        }
    }
    if !jobs.is_empty() && (helper_failed || st.relogins > relogins_before) {
        st.parallel_off = true;
        log::event("parallel_disabled", json!({ "helper_failed": helper_failed, "main_session_lost": st.relogins > relogins_before }));
    }

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

fn read_enrolled_on(be: &mut dyn Backend, term: &str) -> Result<Value, String> {
    let terms = be.tool("drop_terms", &json!({}))?;
    if terms.as_array().is_some_and(|a| a.is_empty()) {
        return Err("PeopleSoft's drop page listed no terms. It was not trusted.".into());
    }
    let row = terms
        .as_array()
        .and_then(|a| a.iter().find(|t| t["term"].as_str().is_some_and(|x| x.trim().eq_ignore_ascii_case(term.trim()))))
        .and_then(|t| t["row"].as_u64());
    match row {
        None => Ok(json!({ "termRow": -1, "classes": [] })),
        Some(r) => {
            let l = be.tool("enrolled_classes", &json!({ "row": r }))?;
            Ok(json!({ "termRow": r, "classes": l["classes"] }))
        }
    }
}

pub fn fetch_refresh(
    st: &mut WriteState,
    creds: &Option<Creds>,
    demo: bool,
    term: Option<&str>,
    helpers: usize,
    make: Option<HelperMaker>,
) -> Result<Value, ApiErr> {
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
            if matches!(other, Some(Err(_))) || st.relogins > relogins_before {
                st.parallel_off = true;
                log::event(
                    "parallel_disabled",
                    json!({ "helper_failed": matches!(other, Some(Err(_))), "main_session_lost": st.relogins > relogins_before, "where": "refresh" }),
                );
            }
            via = "main";
            let t = t.to_string();
            let terms = step("refresh", "drop_terms", || read_tool(st, creds, demo, "drop_terms", &json!({})))?;
            let row = terms
                .as_array()
                .and_then(|a| a.iter().find(|x| x["term"].as_str().is_some_and(|y| y.trim().eq_ignore_ascii_case(t.trim()))))
                .and_then(|x| x["row"].as_u64());
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
    pub term: Option<String>,
}

pub async fn refresh(State(app): State<App>, Json(r): Json<RefreshReq>) -> Result<Json<Value>, ApiErr> {
    if r.term.as_ref().is_some_and(|t| t.len() > 40 || !t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b' ')) {
        return Err(err(StatusCode::BAD_REQUEST, "Unexpected term."));
    }
    let demo = app.demo;
    let creds = app.creds.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if !demo && creds.is_none() {
        return Err(err(StatusCode::UNAUTHORIZED, "Sign in first."));
    }
    let helpers = app.parallel_reads + 1;
    let term = r.term;
    let make = helper_factory(&app, creds.clone());
    let v = with_state(&app, "refresh", move |st| {
        let par: Option<HelperMaker> = if parallel_allowed(helpers, st.armed, st.parallel_off) { Some(&*make) } else { None };
        fetch_refresh(st, &creds, demo, term.as_deref(), helpers, par)
    })
    .await?;
    Ok(Json(v))
}

fn season_rank(term: &str) -> (u32, u32) {
    let mut parts = term.split_whitespace();
    let s = parts.next().unwrap_or("");
    let y: u32 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let r = match s.to_ascii_lowercase().as_str() {
        "winter" => 1,
        "spring" | "summer" => 2,
        "fall" => 3,
        _ => 0,
    };
    (y, r)
}

pub(crate) fn infer_open_term_from_list(list: &Value) -> Option<String> {
    let best_row_term = list["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| matches!(r["status"].as_str(), Some("Classes available" | "Enrolled" | "Waitlisted")))
        .filter_map(|r| r["term"].as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from))
        .max_by_key(|t| season_rank(t));
    best_row_term.or_else(|| list["term_label"].as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from))
}

/// Parallel initial bootstrap: fetches class list, degree tree, and (if term hinted or inferred) enrolled list.
pub fn fetch_bootstrap(
    st: &mut WriteState,
    creds: &Option<Creds>,
    demo: bool,
    term: Option<&str>,
    helpers: usize,
    make: Option<HelperMaker>,
) -> Result<Value, ApiErr> {
    let t0 = Instant::now();
    let relogins_before = st.relogins;
    let use_helpers = helpers >= 2 && !st.parallel_off && make.is_some();

    let mut degree_res: Option<Result<Value, String>> = None;
    let mut enrolled_res: Option<Result<Value, String>> = None;
    let list = std::thread::scope(|sc| {
        let h_deg = if use_helpers {
            make.map(|mk| sc.spawn(move || mk().and_then(|mut be| be.tool("degree_tree", &json!({})))))
        } else {
            None
        };
        let h_enr = match (use_helpers, make, term) {
            (true, Some(mk), Some(t)) if helpers >= 3 => Some(sc.spawn(move || mk().and_then(|mut be| read_enrolled_on(&mut *be, t)))),
            _ => None,
        };
        let list = step("bootstrap", "class_list", || read_tool(st, creds, demo, "plan_class_list", &json!({})));
        if let Some(h) = h_deg {
            degree_res = Some(h.join().unwrap_or_else(|_| Err("degree helper stopped".into())));
        }
        if let Some(h) = h_enr {
            enrolled_res = Some(h.join().unwrap_or_else(|_| Err("enrolled helper stopped".into())));
        }
        list
    })?;
    empty_list_is_not_trusted(st.list_cache.as_ref().map(|(_, p)| p), &list)?;
    st.list_cache = Some((Instant::now(), list.clone()));
    st.list_page_valid = true;

    let degree = match degree_res {
        Some(Ok(v)) => v,
        other => {
            if matches!(other, Some(Err(_))) || st.relogins > relogins_before {
                st.parallel_off = true;
            }
            step("bootstrap", "degree_tree", || read_tool(st, creds, demo, "degree_tree", &json!({})))?
        }
    };
    let effective_term = term.map(String::from).or_else(|| infer_open_term_from_list(&list));
    let enrolled = match (effective_term.as_deref(), enrolled_res) {
        (None, _) => Value::Null,
        (Some(_), Some(Ok(v))) => v,
        (Some(t), other) => {
            if matches!(other, Some(Err(_))) {
                st.parallel_off = true;
            }
            let terms = step("bootstrap", "drop_terms", || read_tool(st, creds, demo, "drop_terms", &json!({})))?;
            let row = terms
                .as_array()
                .and_then(|a| a.iter().find(|x| x["term"].as_str().is_some_and(|y| y.trim().eq_ignore_ascii_case(t.trim()))))
                .and_then(|x| x["row"].as_u64());
            match row {
                None => json!({ "termRow": -1, "classes": [] }),
                Some(r) => {
                    let l = step("bootstrap", "enrolled_classes", || read_tool(st, creds, demo, "enrolled_classes", &json!({ "row": r })))?;
                    json!({ "termRow": r, "classes": l["classes"] })
                }
            }
        }
    };
    Ok(json!({
        "classes": list,
        "degree": degree,
        "enrolled": enrolled,
        "inferredTerm": effective_term,
        "stats": { "ms": t0.elapsed().as_millis() as u64, "parallel": use_helpers && !st.parallel_off },
    }))
}

pub async fn bootstrap(State(app): State<App>, Json(r): Json<RefreshReq>) -> Result<Json<Value>, ApiErr> {
    if r.term.as_ref().is_some_and(|t| t.len() > 40 || !t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b' ')) {
        return Err(err(StatusCode::BAD_REQUEST, "Unexpected term."));
    }
    let demo = app.demo;
    let creds = app.creds.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if !demo && creds.is_none() {
        return Err(err(StatusCode::UNAUTHORIZED, "Sign in first."));
    }
    let helpers = app.parallel_reads + 1;
    let term = r.term;
    let make = helper_factory(&app, creds.clone());
    let v = with_state(&app, "bootstrap", move |st| {
        let par: Option<HelperMaker> = if parallel_allowed(helpers, st.armed, st.parallel_off) { Some(&*make) } else { None };
        fetch_bootstrap(st, &creds, demo, term.as_deref(), helpers, par)
    })
    .await?;
    Ok(Json(v))
}

#[derive(Deserialize, Clone, Debug)]
pub struct CompatibleCourseReq {
    pub code: String,
    #[serde(default)]
    pub crse_id: Option<String>,
    #[serde(default)]
    pub offer_nbr: Option<String>,
    #[serde(default)]
    pub effdt: Option<String>,
    #[serde(default)]
    pub need_detail: bool,
}

#[derive(Deserialize)]
pub struct CompatibleSectionsReq {
    pub strm: String,
    pub courses: Vec<CompatibleCourseReq>,
    #[serde(default)]
    pub acad_career: Option<String>,
    #[serde(default)]
    pub institution: Option<String>,
}

fn valid_short_alnum(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

fn read_compatible_one(
    be: &mut dyn Backend,
    strm: &str,
    career: &str,
    inst: &str,
    req: &CompatibleCourseReq,
) -> Result<Value, String> {
    let norm = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_uppercase();
    let (crse_id, offer_nbr, effdt) = match (&req.crse_id, &req.offer_nbr) {
        (Some(id), Some(off)) => (id.clone(), off.clone(), req.effdt.clone()),
        _ => {
            let hits = be.tool("catalog_search", &json!({ "keyword": req.code }))?;
            let want = norm(&req.code);
            let hit = hits
                .as_array()
                .and_then(|a| a.iter().find(|h| h["code"].as_str().is_some_and(|c| norm(c) == want)))
                .ok_or_else(|| "not found in the course catalog".to_string())?;
            (
                hit["crse_id"].as_str().ok_or("missing crse_id")?.to_string(),
                hit["offer_nbr"].as_str().unwrap_or("1").to_string(),
                hit["effdt"].as_str().map(String::from),
            )
        }
    };
    let detail = if req.need_detail {
        if let Some(ref e) = effdt {
            be.tool("catalog_detail", &json!({ "crse_id": crse_id, "offer_nbr": offer_nbr, "effdt": e })).ok()
        } else {
            None
        }
    } else {
        None
    };
    let effective_career = detail
        .as_ref()
        .and_then(|d| d["career"].as_str())
        .and_then(|c| if c.eq_ignore_ascii_case("Credit") { Some("CRED") } else { None })
        .unwrap_or(career);
    let key = json!({
        "strm": strm,
        "crse_id": crse_id,
        "crse_offer_nbr": offer_nbr,
        "acad_career": effective_career,
        "institution": inst,
    });
    let sections = be.tool("course_sections", &key);
    match sections {
        Ok(s) => Ok(json!({ "code": req.code, "key": key, "sections": s, "detail": detail })),
        Err(e) => {
            // An error on course_sections often means "no classes this term" -> return empty options cleanly when course exists.
            let no_classes = e.to_ascii_lowercase().contains("no classes") || e.to_ascii_lowercase().contains("not offered");
            if no_classes {
                Ok(json!({ "code": req.code, "key": key, "sections": { "course": req.code, "term": "", "options": [] }, "detail": detail }))
            } else {
                Err(e)
            }
        }
    }
}

/// Fast parallel batch read of compatible courses' open/waitlist/closed sections (and optional catalog_detail fallback).
pub fn fetch_compatible_sections(
    st: &mut WriteState,
    creds: &Option<Creds>,
    demo: bool,
    strm: &str,
    career: &str,
    inst: &str,
    courses: &[CompatibleCourseReq],
    workers: usize,
    make: Option<HelperMaker>,
) -> Result<Value, ApiErr> {
    let t0 = Instant::now();
    // Ensure main session is warm if needed (or helpers warm via catalog_search / direct course_sections).
    let n = match make {
        Some(_) if workers >= 2 && courses.len() >= 2 && !st.parallel_off => workers.min(MAX_HELPERS + 1).min(courses.len()),
        _ => 1,
    };
    let mut shares: Vec<Vec<CompatibleCourseReq>> = vec![Vec::new(); n];
    for (i, c) in courses.iter().enumerate() {
        shares[i % n].push(c.clone());
    }
    let mut out: HashMap<String, Result<Value, String>> = HashMap::new();
    std::thread::scope(|sc| {
        let handles: Vec<_> = match make {
            Some(mk) if n > 1 => shares
                .iter()
                .skip(1)
                .map(|share| {
                    let share = share.clone();
                    sc.spawn(move || {
                        let mut be = match mk() {
                            Ok(b) => b,
                            Err(e) => return share.into_iter().map(|c| (c.code, Err(e.clone()))).collect::<Vec<_>>(),
                        };
                        share
                            .into_iter()
                            .map(|c| {
                                let res = read_compatible_one(&mut *be, strm, career, inst, &c);
                                (c.code, res)
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect(),
            _ => Vec::new(),
        };
        for c in &shares[0] {
            let res = live(st, creds, demo)
                .map_err(|e| e.1)
                .and_then(|be| read_compatible_one(&mut **be, strm, career, inst, c));
            out.insert(c.code.clone(), res);
        }
        for h in handles {
            if let Ok(list) = h.join() {
                for (code, r) in list {
                    out.insert(code, r);
                }
            }
        }
    });
    let results: Vec<Value> = courses
        .iter()
        .map(|c| match out.remove(&c.code) {
            Some(Ok(v)) => v,
            Some(Err(e)) => json!({ "code": c.code, "error": e }),
            None => json!({ "code": c.code, "error": "read did not complete" }),
        })
        .collect();
    Ok(json!({
        "results": results,
        "stats": { "workers": n, "ms": t0.elapsed().as_millis() as u64, "parallel": n > 1 },
    }))
}

pub async fn compatible_sections(State(app): State<App>, Json(req): Json<CompatibleSectionsReq>) -> Result<Json<Value>, ApiErr> {
    if !valid_short_alnum(&req.strm, 8) || !req.strm.bytes().all(|b| b.is_ascii_digit()) {
        return Err(err(StatusCode::BAD_REQUEST, "Invalid term code."));
    }
    if req.courses.is_empty() || req.courses.len() > 30 {
        return Err(err(StatusCode::BAD_REQUEST, "Request between 1 and 30 courses."));
    }
    for c in &req.courses {
        if c.code.trim().is_empty() || c.code.len() > 24 || !c.code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b' ') {
            return Err(err(StatusCode::BAD_REQUEST, "Invalid course code."));
        }
        if c.crse_id.as_ref().is_some_and(|x| !valid_short_alnum(x, 12)) {
            return Err(err(StatusCode::BAD_REQUEST, "Invalid crse_id."));
        }
        if c.offer_nbr.as_ref().is_some_and(|x| !valid_short_alnum(x, 12)) {
            return Err(err(StatusCode::BAD_REQUEST, "Invalid offer_nbr."));
        }
        if c.effdt.as_ref().is_some_and(|s| {
            !(s.len() == 10 && s.as_bytes()[4] == b'-' && s.as_bytes()[7] == b'-' && s.bytes().enumerate().all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit()))
        }) {
            return Err(err(StatusCode::BAD_REQUEST, "Invalid effdt."));
        }
    }
    let career = req.acad_career.as_deref().unwrap_or("CRED");
    let inst = req.institution.as_deref().unwrap_or("CNAQA");
    if !valid_short_alnum(career, 12) || !valid_short_alnum(inst, 12) {
        return Err(err(StatusCode::BAD_REQUEST, "Invalid academic career or institution."));
    }
    let demo = app.demo;
    let creds = app.creds.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if !demo && creds.is_none() {
        return Err(err(StatusCode::UNAUTHORIZED, "Sign in first."));
    }
    let helpers = app.parallel_reads + 1;
    let strm = req.strm;
    let career_s = career.to_string();
    let inst_s = inst.to_string();
    let courses = req.courses;
    let make = helper_factory(&app, creds.clone());
    let v = with_state(&app, "compatible_sections", move |st| {
        let par: Option<HelperMaker> = if parallel_allowed(helpers, st.armed, st.parallel_off) { Some(&*make) } else { None };
        fetch_compatible_sections(st, &creds, demo, &strm, &career_s, &inst_s, &courses, helpers, par)
    })
    .await?;
    Ok(Json(v))
}

fn swap_code_of(label: &str) -> String {
    label.split_whitespace().take(2).collect::<Vec<_>>().join(" ")
}

type HeldRead = (Value, u64);

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
                if v["sections"]["course"].as_str() == Some(code.as_str()) {
                    Ok(v)
                } else {
                    Err(format!("a different course is on swap row {row}"))
                }
            });
            let ms = t.elapsed().as_millis() as u64;
            log::event("parallel_held", json!({ "course": code, "ms": ms, "ok": r.is_ok(), "error": r.as_ref().err().map(|e| log::trunc(e, 200)) }));
            (code.clone(), r.map(|v| (v, ms)))
        })
        .collect()
}

pub fn fetch_held_options(
    st: &mut WriteState,
    creds: &Option<Creds>,
    demo: bool,
    codes: &[String],
    workers: usize,
    make: Option<HelperMaker>,
) -> Result<Value, ApiErr> {
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
            let r = step("held_options", &format!("main:{code}"), || read_tool(st, creds, demo, "swap_options", &json!({ "candidate_row": row })))
                .map_err(|e| e.1)
                .and_then(|v| {
                    if v["sections"]["course"].as_str() == Some(code.as_str()) {
                        Ok(v)
                    } else {
                        Err(format!("a different course is on swap row {row}"))
                    }
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
        log::event(
            "parallel_disabled",
            json!({ "helper_failed": helper_failed, "main_session_lost": st.relogins > relogins_before, "where": "held_options" }),
        );
    }
    for (code, row) in &jobs {
        if matches!(out.get(code), Some((Ok(_), _))) {
            continue;
        }
        let t = Instant::now();
        let r = step("held_options", &format!("retry:{code}"), || read_tool(st, creds, demo, "swap_options", &json!({ "candidate_row": row })))
            .map_err(|e| e.1)
            .and_then(|v| {
                if v["sections"]["course"].as_str() == Some(code.as_str()) {
                    Ok(v)
                } else {
                    Err(format!("a different course is on swap row {row}"))
                }
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
    pub codes: Vec<String>,
    #[serde(default)]
    pub parallel: Option<bool>,
}

pub async fn held_options(State(app): State<App>, Json(r): Json<HeldReq>) -> Result<Json<Value>, ApiErr> {
    if r.codes.is_empty() || r.codes.len() > 10 || r.codes.iter().any(|c| c.len() > 12 || !c.bytes().all(|b| b.is_ascii_alphanumeric() || b == b' ')) {
        return Err(err(StatusCode::BAD_REQUEST, "Pick between 1 and 10 course codes."));
    }
    let demo = app.demo;
    let creds = app.creds.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if !demo && creds.is_none() {
        return Err(err(StatusCode::UNAUTHORIZED, "Sign in first."));
    }
    let helpers = if r.parallel == Some(false) { 1 } else { app.parallel_reads + 1 };
    let codes = r.codes;
    let make = helper_factory(&app, creds.clone());
    let v = with_state(&app, "held_options", move |st| {
        let par: Option<HelperMaker> = if parallel_allowed(helpers, st.armed, st.parallel_off) { Some(&*make) } else { None };
        fetch_held_options(st, &creds, demo, &codes, helpers, par)
    })
    .await?;
    Ok(Json(v))
}

pub async fn sections(app: &App, rows: Vec<u32>, known: HashMap<String, Value>) -> Result<Value, ApiErr> {
    let demo = app.demo;
    let creds = app.creds.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if !demo && creds.is_none() {
        return Err(err(StatusCode::UNAUTHORIZED, "Sign in first."));
    }
    let helpers = app.parallel_reads;
    let make = helper_factory(app, creds.clone());
    with_state(app, "sections", move |st| {
        let par: Option<HelperMaker> = if parallel_allowed(helpers, st.armed, st.parallel_off) { Some(&*make) } else { None };
        fetch_sections(st, &creds, demo, &rows, &known, helpers, par)
    })
    .await
}
