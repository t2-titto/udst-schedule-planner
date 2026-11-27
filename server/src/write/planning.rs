use super::error::err;
use super::state::{armed_backend, WriteState};
use crate::seats::{self, Report, SeatCheck};
use crate::{log, ApiErr};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::time::Instant;

pub const ARG_NAMES: [&str; 14] = [
    "row",
    "term_row",
    "class_nbr",
    "candidate_row",
    "option_row",
    "new_class_nbrs",
    "year",
    "index",
    "term_code",
    "keyword",
    "crse_id",
    "offer_nbr",
    "strm",
    "effdt",
];

#[derive(Deserialize)]
pub struct ToolReq {
    pub tool: String,
    #[serde(default)]
    pub args: Value,
    #[serde(default)]
    pub expect: Option<Vec<String>>,
    #[serde(default)]
    pub check: Option<SeatCheck>,
    #[serde(default)]
    pub credits: Option<CreditsReq>,
}

pub fn clean_args(v: &Value) -> Result<Value, ApiErr> {
    match v {
        Value::Null => Ok(json!({})),
        Value::Object(m)
            if m.len() <= 4
                && m.keys().all(|k| ARG_NAMES.contains(&k.as_str()))
                && m.iter().all(|(k, x)| {
                    if k == "keyword" {
                        x.as_str().is_some_and(|s| {
                            !s.trim().is_empty() && s.len() <= 40 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '-')
                        })
                    } else if k == "effdt" {
                        x.as_str().is_some_and(|s| {
                            s.len() == 10
                                && s.as_bytes()[4] == b'-'
                                && s.as_bytes()[7] == b'-'
                                && s.bytes().enumerate().all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
                        })
                    } else {
                        x.is_u64()
                            || x.as_str().is_some_and(|s| s.len() <= 12 && s.bytes().all(|b| b.is_ascii_digit()))
                            || x.as_array().is_some_and(|a| {
                                !a.is_empty()
                                    && a.len() <= 6
                                    && a.iter().all(|e| e.as_str().is_some_and(|s| !s.is_empty() && s.len() <= 10 && s.bytes().all(|b| b.is_ascii_digit())))
                            })
                    }
                }) =>
        {
            Ok(v.clone())
        }
        _ => Err(err(StatusCode::BAD_REQUEST, "Unexpected arguments.")),
    }
}

pub fn sorted(v: &[String]) -> Vec<String> {
    let mut s = v.to_vec();
    s.sort();
    s
}

pub fn nums_of(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
}

pub fn compact_code(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

pub fn optional_row<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<u32>, D::Error> {
    let n = Option::<i64>::deserialize(d)?;
    Ok(n.filter(|n| *n >= 0).map(|n| u32::try_from(n).unwrap_or(u32::MAX)))
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct Pick {
    pub row: u32,
    #[serde(rename = "optionRow")]
    pub option_row: u32,
    pub course: String,
    #[serde(rename = "classNbrs")]
    pub class_nbrs: Vec<String>,
    #[serde(default)]
    pub key: Value,
    #[serde(default)]
    pub seen: HashMap<String, Option<u32>>,
    #[serde(default)]
    pub waitlist: bool,
}

impl Pick {
    pub fn to_check(&self) -> SeatCheck {
        SeatCheck {
            course: self.course.clone(),
            key: self.key.clone(),
            class_nbrs: self.class_nbrs.clone(),
            seen: self.seen.clone(),
            waitlist: self.waitlist,
        }
    }
}

#[derive(Deserialize, Clone, Debug)]
pub struct SwapItem {
    pub course: String,
    #[serde(rename = "oldNbrs")]
    pub old_nbrs: Vec<String>,
    #[serde(rename = "newNbrs")]
    pub new_nbrs: Vec<String>,
    pub key: Value,
    #[serde(default)]
    pub seen: HashMap<String, Option<u32>>,
    #[serde(default, rename = "waitsFor")]
    pub waits_for: Vec<String>,
    #[serde(default)]
    pub hint: String,
}

#[derive(Deserialize)]
pub struct SwapApplyReq {
    pub term: String,
    pub swaps: Vec<SwapItem>,
}

pub struct RoundOpts<'a> {
    pub verify_list: bool,
    pub prior: &'a [SwapItem],
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
    #[serde(default)]
    pub depends: Vec<String>,
    #[serde(default, rename = "afterDrop")]
    pub after_drop: bool,
    #[serde(default, rename = "afterPlan")]
    pub after_plan: bool,
}

#[derive(Deserialize, Clone, Debug)]
pub struct ApplyPlan {
    pub course: String,
    pub year: u32,
    pub index: u32,
    #[serde(rename = "termCode")]
    pub term_code: String,
    pub credits: CreditsReq,
}

#[derive(Deserialize, Clone, Debug)]
pub struct CreditsReq {
    pub term: String,
    pub limit: f64,
    pub after: f64,
    #[serde(default)]
    pub unknown: Vec<String>,
    #[serde(default, rename = "overrideLimit")]
    pub override_limit: bool,
}

#[derive(Deserialize)]
pub struct ApplyReq {
    pub term: String,
    #[serde(default, rename = "termRow", deserialize_with = "optional_row")]
    pub term_row: Option<u32>,
    #[serde(default)]
    pub drops: Vec<ApplyDrop>,
    #[serde(default)]
    pub swaps: Vec<SwapItem>,
    #[serde(default)]
    pub rounds: Vec<Vec<SwapItem>>,
    #[serde(default)]
    pub enrols: Vec<ApplyEnrol>,
    #[serde(default)]
    pub plans: Vec<ApplyPlan>,
    #[serde(default)]
    pub credits: Option<CreditsReq>,
}

impl ApplyReq {
    pub fn all_rounds(&self) -> Vec<Vec<SwapItem>> {
        if !self.rounds.is_empty() {
            self.rounds.clone()
        } else if !self.swaps.is_empty() {
            vec![self.swaps.clone()]
        } else {
            vec![]
        }
    }
}

pub fn credit_limit(term: &str) -> f64 {
    if term.to_ascii_lowercase().contains("spring") {
        12.0
    } else {
        18.0
    }
}

fn check_credit_cap(c: &CreditsReq, context_msg: &str) -> Result<(), String> {
    let default_limit = credit_limit(&c.term);
    if (c.limit - default_limit).abs() > 1e-9 {
        return Err(format!(
            "The page and the server disagree about the credit limit for {} ({} against {}). Reload the page.",
            c.term, c.limit, default_limit
        ));
    }
    if !c.unknown.is_empty() {
        return Err(format!("The credits of {} are unknown, so the term's total cannot be trusted. Nothing was sent.", c.unknown.join(", ")));
    }
    if !(c.after.is_finite() && c.after >= 0.0) {
        return Err("The credit total is not a number. Nothing was sent.".into());
    }
    let effective_limit = if c.override_limit { 24.0 } else { c.limit };
    if c.after > effective_limit + 1e-9 {
        return Err(format!(
            "{} credits is over the {} credits PeopleSoft allows in {}. {context_msg}",
            c.after, effective_limit, c.term
        ));
    }
    Ok(())
}

pub fn check_plan_request(args: &Value, credits: Option<&CreditsReq>) -> Result<(), String> {
    match args["term_code"].as_str() {
        Some(t) if !t.is_empty() => {}
        _ => return Err("Removing a course from the plan is not supported yet: PeopleSoft did not accept it when tried.".into()),
    }
    let Some(c) = credits else {
        return Err("This page is out of date: it did not check the credit limit. Reload it and try again.".into());
    };
    check_credit_cap(c, "PeopleSoft would save it but refuse to enrol later, so nothing was sent.")
}

pub fn check_credits(req: &ApplyReq) -> Result<(), String> {
    if req.enrols.is_empty() {
        return Ok(());
    }
    let Some(c) = &req.credits else {
        return Err("This page is out of date: it did not check the credit limit. Reload it and try again.".into());
    };
    if c.term != req.term {
        return Err(format!("The credit check is for {}, which disagrees with {}. Reload the page.", c.term, req.term));
    }
    check_credit_cap(c, "PeopleSoft would refuse the enrolment, so nothing was sent.")
}

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
        let is_move_out = p.credits.term != req.term;
        let has_enrol = req.enrols.iter().any(|e| e.after_plan && e.pick.course == p.course);
        if !is_move_out && !has_enrol {
            return Err(format!("{} is planned but not enrolled; Apply plans a course only to enrol it.", p.course));
        }
        if is_move_out && has_enrol {
            return Err(format!("{} cannot both move out to {} and enrol in {}.", p.course, p.credits.term, req.term));
        }
        check_plan_request(&json!({"year": p.year, "index": p.index, "term_code": p.term_code}), Some(&p.credits))?;
    }
    if let Some(e) = req.enrols.iter().find(|e| e.after_plan && !req.plans.iter().any(|p| p.course == e.pick.course)) {
        return Err(format!("{} waits for a plan step that was not sent.", e.pick.course));
    }
    Ok(())
}

pub fn term_order(label: &str) -> Option<i64> {
    let (season, year) = label.split_once(' ')?;
    let y: i64 = year.parse().ok()?;
    match season {
        "Fall" => Some(y * 3),
        "Winter" => Some((y - 1) * 3 + 1),
        "Spring" => Some((y - 1) * 3 + 2),
        _ => None,
    }
}

pub fn term_from_order(n: i64) -> String {
    match n % 3 {
        0 => format!("Fall {}", n / 3),
        1 => format!("Winter {}", (n - 1) / 3 + 1),
        _ => format!("Spring {}", (n - 2) / 3 + 1),
    }
}

/// Safe curriculum term inference: checks concrete terms in the same semester first, then uses observed
/// semester-to-term anchors to verify cadence before extrapolating, avoiding misclassifications on skipped/gap terms.
pub fn curriculum_term(rows: &[Value], semester: u64) -> Option<String> {
    let mut anchors: Vec<(u64, i64)> = Vec::new();
    for r in rows {
        let Some(s) = r["semester"].as_u64() else { continue };
        let Some(t) = r["term"].as_str().filter(|t| !t.is_empty()).and_then(term_order) else { continue };
        if !anchors.iter().any(|(sem, _)| *sem == s) {
            anchors.push((s, t));
        }
    }
    if anchors.is_empty() {
        return None;
    }
    anchors.sort_by_key(|(s, _)| *s);
    if let Some((_, ord)) = anchors.iter().find(|(s, _)| *s == semester) {
        return Some(term_from_order(*ord));
    }
    // Verify consecutive anchors follow standard semester-step progression (no skipped-term distortion across anchors).
    let nearest = anchors.iter().min_by_key(|(s, _)| s.abs_diff(semester)).copied()?;
    let contiguous = anchors.windows(2).all(|w| (w[1].1 - w[0].1) == (w[1].0 as i64 - w[0].0 as i64));
    if !contiguous && anchors.len() >= 2 {
        // If there is a gap between anchors, only extrapolate forward from the latest anchor (or backward before first if diff is 1).
        let last = *anchors.last()?;
        if semester > last.0 {
            return Some(term_from_order(last.1 + (semester as i64 - last.0 as i64)));
        }
        return None;
    }
    Some(term_from_order(nearest.1 + (semester as i64 - nearest.0 as i64)))
}

fn row_term(r: &Value) -> Option<String> {
    if let Some(t) = r["term"].as_str().filter(|t| !t.is_empty()) {
        return Some(t.to_string());
    }
    let c = r["term_choice"].as_object()?;
    let sel = c.get("selected")?.as_str()?;
    c.get("options")?
        .as_array()?
        .iter()
        .find(|o| o["code"].as_str() == Some(sel))
        .and_then(|o| o["label"].as_str())
        .map(String::from)
}

pub fn term_floor(rows: &[Value], term: &str, skip: &BTreeSet<String>, set_aside: bool, extra: f64) -> Result<f64, String> {
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

pub fn fresh_tree_total(tree: &Value, dest: &str, moved_compact: &str, set_aside: bool) -> Result<f64, String> {
    let rows = tree["rows"].as_array().ok_or("The degree tree could not be read again.")?;
    let moved = rows
        .iter()
        .find(|r| r["code"].as_str().is_some_and(|c| compact_code(c) == moved_compact))
        .ok_or("The course being planned was not found in a fresh read of the tree.")?;
    let moved_units = moved["units"]
        .as_str()
        .and_then(|u| u.trim().parse::<f64>().ok())
        .ok_or("The credits of the course being planned are unknown.")?;
    let skip: BTreeSet<String> = [moved_compact.to_string()].into_iter().collect();
    term_floor(rows, dest, &skip, set_aside, moved_units)
}

pub fn apply_recount(st: &mut WriteState, req: &ApplyReq) -> Result<(), ApiErr> {
    if req.enrols.is_empty() {
        return Ok(());
    }
    let c = req
        .credits
        .as_ref()
        .ok_or_else(|| err(StatusCode::UNPROCESSABLE_ENTITY, "This page did not check the credit limit. Nothing was sent."))?;
    let tree = armed_backend(st)?
        .tool("degree_tree", &json!({}))
        .map_err(|m| err(StatusCode::UNPROCESSABLE_ENTITY, format!("Could not read PeopleSoft's plan to re-check the credits: {m} Nothing was sent.")))?;
    let rows = tree["rows"]
        .as_array()
        .ok_or_else(|| err(StatusCode::UNPROCESSABLE_ENTITY, "PeopleSoft's plan could not be read again. Nothing was sent."))?;
    let mut skip: BTreeSet<String> = req.drops.iter().map(|d| compact_code(&d.course)).collect();
    for p in &req.plans {
        if p.credits.term != req.term {
            skip.insert(compact_code(&p.course));
        }
    }
    let floor = term_floor(rows, &req.term, &skip, true, 0.0).map_err(|m| err(StatusCode::UNPROCESSABLE_ENTITY, format!("{m} Nothing was sent.")))?;
    let limit = if c.override_limit { 24.0 } else { credit_limit(&req.term) };
    if floor > limit + 1e-9 {
        return Err(err(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("PeopleSoft's own plan has {floor} credits in {} after your drops, over the {limit} allowed. Nothing was sent.", req.term),
        ));
    }
    if floor > c.after + 1e-9 {
        return Err(err(
            StatusCode::CONFLICT,
            format!(
                "The page counted {} credits for {} but PeopleSoft's plan has at least {floor}. The page is out of date: reload it and try again. Nothing was sent.",
                c.after, req.term
            ),
        ));
    }
    Ok(())
}

pub fn plan_term_matches(out: &Value, credits: &CreditsReq) -> bool {
    out["plan"]["term"].as_str() == Some(credits.term.as_str())
}

pub fn check_expected(data: &Value, expect: Option<&[String]>) -> Result<(), ApiErr> {
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

pub fn stash_preview(st: &mut WriteState, mut data: Value) -> Value {
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

pub fn stash_checked(st: &mut WriteState, data: Value, checks: Vec<SeatCheck>, reports: &[Report]) -> Value {
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

pub fn seat_block(m: String) -> ApiErr {
    err(StatusCode::CONFLICT, m)
}

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
            let term = out["plan"]["term"].as_str().filter(|t| !t.trim().is_empty()).map(String::from);
            st.drop_ctx.insert(tok.to_string(), (row as u32, nbr.to_string(), term));
        }
    }
    Ok(out)
}

pub fn plan_preview(st: &mut WriteState, args: &Value, credits: &CreditsReq, expect_course: &str) -> Result<Value, ApiErr> {
    check_plan_request(args, Some(credits)).map_err(|m| err(StatusCode::UNPROCESSABLE_ENTITY, m))?;
    let v = preview_checked(st, "preview_plan_term", args, None, None)?;
    if v["plan"]["class_numbers"][0].as_str() != Some(compact_code(expect_course).as_str()) {
        return Err(err(
            StatusCode::CONFLICT,
            "The course at that place in PeopleSoft's degree is not the one you chose, so the page's copy is out of date. Nothing was changed. Reload and try again.",
        ));
    }
    if !plan_term_matches(&v, credits) {
        return Err(err(StatusCode::CONFLICT, "The term PeopleSoft would plan into is not the one the page checked. Reload and try again."));
    }
    let moved = v["plan"]["class_numbers"][0].as_str().unwrap_or("").to_string();
    let tree = st
        .backend
        .as_mut()
        .ok_or_else(|| err(StatusCode::CONFLICT, "Writes are off. Enable them first."))?
        .tool("degree_tree", &json!({}))
        .map_err(|m| err(StatusCode::UNPROCESSABLE_ENTITY, m))?;
    let fresh = fresh_tree_total(&tree, &credits.term, &moved, false).map_err(|m| err(StatusCode::UNPROCESSABLE_ENTITY, m))?;
    let limit = if credits.override_limit { 24.0 } else { credit_limit(&credits.term) };
    if fresh > limit + 1e-9 {
        return Err(err(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("PeopleSoft's own plan now has {fresh} credits in {} with this course, over the {limit} allowed. Nothing was sent.", credits.term),
        ));
    }
    if fresh > credits.after + 1e-9 {
        return Err(err(
            StatusCode::CONFLICT,
            format!("The page counted {} credits for {} but PeopleSoft's plan now has at least {fresh}. The page is out of date: reload it and try again.", credits.after, credits.term),
        ));
    }
    if credits.override_limit {
        log::event("credit_override", json!({ "term": credits.term, "limit": credits.limit, "after": credits.after, "course": moved }));
    }
    Ok(v)
}
