use super::apply::*;
use super::error::*;
use super::planning::*;
use super::reads::*;
use super::state::*;
use crate::demo::DemoState;
use crate::seats::{safe_key, SeatCheck};
use crate::{log, ApiErr, App, Creds};
use axum::http::StatusCode;
use frankie_auth::{AuthFailure, Http, HttpRequest, HttpResponse, Method};
use frankie_connector_peoplesoft::testkit::{MockPs, Scenario};
use frankie_connector_peoplesoft::{PeopleSoft, PeopleSoftConfig, Session};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};


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

const CONTRACT_APPLY: &str = include_str!("../../../frontend/src/v2/fixtures/contract-apply-plan.json");
const CONTRACT_PREVIEW: &str = include_str!("../../../frontend/src/v2/fixtures/contract-preview-plan.json");
const CONTRACT_CATALOG: &str = include_str!("../../../frontend/src/v2/fixtures/contract-catalog-read.json");

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
    let credits = CreditsReq { term: "Fall 2027".into(), limit: 18.0, after: 6.0, unknown: vec![], override_limit: false };
    let e = plan_preview(&mut st, &json!({"year": 1, "index": 8, "term_code": "1278"}), &credits, "SOFT2301").unwrap_err();
    assert!(e.1.contains("not the one you chose"), "{}", e.1);
}

fn plan_credits(term: &str, limit: f64, after: f64) -> CreditsReq {
    CreditsReq { term: term.into(), limit, after, unknown: vec![], override_limit: false }
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
    assert!(statements().len() >= 3);
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
        run_lock: tokio::sync::Mutex::new(()),
        snap: Mutex::new(Snap::default()),
        stop: Arc::new(AtomicBool::new(false)),
        shutdown: Arc::new(AtomicBool::new(false)),
        parallel_reads: 0,
        hosts: Default::default(),
        origins: Default::default(),
        job: Arc::new(Mutex::new(None)),
        fired: Arc::new(Mutex::new(Vec::new())),
        prestaged: Mutex::new(Vec::new()),
        said: Arc::new(Mutex::new(Vec::new())),
        demo_state: Arc::new(Mutex::new(DemoState::new())),
        last_seen: Mutex::new(Instant::now()),
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
    let ctx = JobCtx::for_test();
    *ctx.job.lock().unwrap() = Some(new_job(req));
    let mut st = main;
    let creds = Some(Creds { user: "u".into(), pass: "p".into() });
    let stop = Arc::new(AtomicBool::new(false));
    let r = apply_run(&mut st, &ctx, &creds, req, &stop);
    settle_job(&ctx, &r);
    *TEST_BACKENDS.lock().unwrap() = None;
    let j = ctx.job.lock().unwrap().clone().unwrap();
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
    req.plans = vec![ApplyPlan { course: "TEST 1001".into(), year: 1, index: 8, term_code: "1271".into(), credits: CreditsReq { term: "Winter 2099".into(), limit: 18.0, after, unknown: vec![], override_limit: false } }];
    req.credits = Some(CreditsReq { term: "Winter 2099".into(), limit: 18.0, after, unknown: vec![], override_limit: false });
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
    let ctx = JobCtx::for_test();
    *ctx.job.lock().unwrap() = Some(json!({ "id": 1, "phase": "enrolling", "finished": false, "error": null, "items": [
        { "id": "enrol:A", "status": "working", "note": "sent, waiting for PeopleSoft" },
        { "id": "enrol:B", "status": "done", "note": "Enrolled." },
        { "id": "swap:C", "status": "pending", "note": "" },
    ] }));
    settle_job(&ctx, &Ok(()));
    let j = ctx.job.lock().unwrap().clone().unwrap();
    assert_eq!(j["finished"], true);
    let st = |i: usize| j["items"][i]["status"].as_str().unwrap().to_string();
    assert_eq!((st(0), st(1), st(2)), ("error".into(), "done".into(), "error".into()));
    assert!(j["items"][0]["note"].as_str().unwrap().contains("check PeopleSoft"));
    // After a failure of the whole run every unfinished item carries its reason.
    *ctx.job.lock().unwrap() = Some(json!({ "id": 2, "finished": false, "error": null, "items": [{ "id": "drop:A", "status": "working", "note": "" }] }));
    settle_job(&ctx, &Err(err(StatusCode::CONFLICT, "Writes are off.")));
    let j = ctx.job.lock().unwrap().clone().unwrap();
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
    Some(CreditsReq { term: term.into(), limit, after, unknown: unknown.iter().map(|s| s.to_string()).collect(), override_limit: false })
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


// ---- redesign tests: standalone plan moves, credit override, non-contiguous terms, parallel demo bootstrap/compatible ----

#[test]
fn standalone_plan_move_without_enrol_passes_check_plans() {
    let raw = json!({
        "term": "Winter 2027",
        "plans": [{
            "year": 1,
            "index": 4,
            "termCode": "1275",
            "course": "MATH 2030",
            "fromTerm": "Winter 2027",
            "credits": { "term": "Spring 2027", "limit": 12.0, "after": 9.0, "unknown": [] }
        }],
        "enrols": []
    });
    let req: ApplyReq = serde_json::from_value(raw).expect("standalone plan move parses");
    assert!(check_plans(&req).is_ok(), "{:?}", check_plans(&req));
}

#[test]
fn credit_limit_override_allows_over_limit_enrol_while_keeping_guard_for_non_override() {
    let mut c = credits("Winter 2027", 18.0, 21.0, &[]).unwrap();
    c.override_limit = true;
    let req = enrolling("Winter 2027", Some(c));
    assert!(check_credits(&req).is_ok());

    let mut wrong_limit = credits("Winter 2027", 25.0, 21.0, &[]).unwrap();
    wrong_limit.override_limit = true;
    assert!(check_credits(&enrolling("Winter 2027", Some(wrong_limit))).is_err());
}

#[test]
fn curriculum_term_handles_non_contiguous_semesters() {
    let row = |code: &str, sem: u64, term: Option<&str>| {
        json!({ "code": code, "semester": sem, "units": "3.00", "term": term, "term_choice": null })
    };
    let rows = vec![
        row("A 1", 0, Some("Fall 2025")),
        row("B 2", 4, Some("Winter 2027")),
    ];
    assert_eq!(curriculum_term(&rows, 5).as_deref(), Some("Spring 2027"));
    assert_eq!(curriculum_term(&rows, 6).as_deref(), Some("Fall 2027"));
}

#[test]
fn demo_backend_supports_compatible_catalog_sections_and_state_mutations() {
    let state = Arc::new(Mutex::new(crate::demo::DemoState::new()));
    let mut be = build_demo_with_state(Arc::clone(&state));
    let secs = be.tool("catalog_sections", &json!({"crse_id": "004601", "offer_nbr": "1", "strm": "1271"})).unwrap();
    assert!(!secs["options"].as_array().unwrap().is_empty());

    let empty_secs = be.tool("catalog_sections", &json!({"crse_id": "006231", "offer_nbr": "1", "strm": "1271"})).unwrap();
    assert_eq!(empty_secs["options"].as_array().unwrap().len(), 0);
}

#[test]
fn demo_evening_section_and_bootstrap_term_inference_work() {
    let state = Arc::new(Mutex::new(crate::demo::DemoState::new()));
    let mut st = WriteState::default();
    st.backend = Some(build_demo_with_state(Arc::clone(&state)));

    let boot = super::reads::fetch_bootstrap(&mut st, &None, true, None, 1, None).unwrap();
    assert_eq!(boot["inferredTerm"].as_str(), Some("Winter 2027"));
    assert!(!boot["enrolled"]["classes"].as_array().unwrap().is_empty());

    let secs = st.backend.as_mut().unwrap().tool("catalog_sections", &json!({"crse_id": "004614", "offer_nbr": "1", "strm": "1271"})).unwrap();
    let has_evening = secs["options"]
        .as_array()
        .unwrap()
        .iter()
        .any(|o| o["components"].as_array().unwrap().iter().any(|c| c["schedule_text"].as_str().unwrap_or("").contains("8:00PM")));
    assert!(has_evening, "expected INFS 3102 evening section 8:00PM-10:00PM");
}
