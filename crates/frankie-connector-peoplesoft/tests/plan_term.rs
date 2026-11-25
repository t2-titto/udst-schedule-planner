//! The plan write (set a planned course's term, Save Planning) against a scripted PeopleSoft built from the real captured pages.
use frankie_auth::{AuthFailure, Http, HttpRequest, HttpResponse, Method};
use frankie_connector_peoplesoft::connector::ConfirmError;
use frankie_connector_peoplesoft::gate::GateError;
use frankie_connector_peoplesoft::{PeopleSoft, PeopleSoftConfig, PsError, Session};
use std::collections::BTreeMap;

const COLLAPSED: &str = include_str!("fixtures/degree/overview_collapsed.html");
const TREE: &str = include_str!("fixtures/degree/tree_expanded.html");

/// Year 1, position 8 is SOFT 2301 (row 19 of the full tree), planned in Spring 2027 (1275).
const SOFT: (u32, u32, u32) = (1, 8, 19);

#[derive(Default, Clone)]
struct Quirks {
    /// The answer to the term change still shows the old value.
    ignore_change: bool,
    /// Save answers fine but the plan is not changed.
    save_noop: bool,
    /// Save also changes another course of the same Year (row 22; the target of that test is row 21, Year 2).
    save_hits_other: bool,
    /// The plan changes behind our back between preview and confirm.
    drift_after_first_read: bool,
}

struct Scripted {
    terms: BTreeMap<u32, String>,
    pending: BTreeMap<u32, String>,
    q: Quirks,
    reads: u32,
    log: Vec<(&'static str, String)>,
}

impl Scripted {
    fn new(q: Quirks) -> Self {
        Self { terms: BTreeMap::new(), pending: BTreeMap::new(), q, reads: 0, log: vec![] }
    }
    fn tree_html(&self) -> String {
        let mut h = TREE.to_string();
        for (row, code) in &self.terms {
            h = set_selected(&h, *row, code);
        }
        h
    }
    fn posts(&self) -> Vec<String> {
        self.log.iter().filter(|(m, _)| *m == "POST").map(|(_, a)| a.clone()).collect()
    }
}

fn set_selected(html: &str, row: u32, code: &str) -> String {
    let id = format!("<select id='DESCR10${row}'");
    let Some(start) = html.find(&id) else { return html.to_string() };
    let end = start + html[start..].find("</select>").unwrap();
    let block = &html[start..end];
    let block = block.replace(" selected='selected'", "");
    let from = format!("<option value=\"{code}\"");
    let block = block.replacen(&from, &format!("{from} selected='selected'"), 1);
    format!("{}{}{}", &html[..start], block, &html[end..])
}

fn ok(body: String) -> HttpResponse {
    HttpResponse { status: 200, location: None, content_type: Some("text/html".into()), body }
}

fn field(name: &str, value: &str) -> Option<String> {
    let _ = (name, value);
    None
}

impl Http for Scripted {
    fn send(&mut self, req: HttpRequest) -> Result<HttpResponse, AuthFailure> {
        let body = req.body.as_ref().map(|b| b.to_string()).unwrap_or_default();
        let pairs: Vec<(String, String)> = url::form_urlencoded::parse(body.as_bytes()).map(|(a, b)| (a.into_owned(), b.into_owned())).collect();
        let get = |k: &str| pairs.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone()).unwrap_or_default();
        let _ = field("", "");
        if req.method == Method::Get {
            self.log.push(("GET", String::new()));
            self.reads += 1;
            if self.q.drift_after_first_read && self.reads > 1 {
                self.terms.insert(SOFT.2, "1288".into());
            }
            return Ok(ok(COLLAPSED.to_string()));
        }
        let action = get("ICAction");
        self.log.push(("POST", action.clone()));
        if action.starts_with("EDIT_PB$") {
            return Ok(ok(self.tree_html()));
        }
        if let Some(row) = action.strip_prefix("DESCR10$").and_then(|r| r.parse::<u32>().ok()) {
            let new = get(&action);
            self.pending.insert(row, new.clone());
            let shown = if self.q.ignore_change { self.terms.get(&row).cloned().unwrap_or("1275".into()) } else { new };
            let opt = |c: &str| format!("<option value=\"{c}\"{}>x</option>", if c == shown { " selected='selected'" } else { "" });
            return Ok(ok(format!(
                "<?xml version=\"1.0\"?><PAGE id='x'><FIELD id='win9divDESCR10${row}'><![CDATA[<select id='DESCR10${row}'>{}{}{}{}</select>]]></FIELD></PAGE>",
                opt(""), opt("1275"), opt("1278"), opt("1281")
            )));
        }
        if action == "VC_PE_SS_WRK_VC_SAVE_PLAN_PB" {
            if !self.q.save_noop {
                for (r, c) in std::mem::take(&mut self.pending) {
                    self.terms.insert(r, c);
                }
            }
            if self.q.save_hits_other {
                self.terms.insert(22, "1288".into());
            }
            return Ok(ok(self.tree_html()));
        }
        Ok(ok(COLLAPSED.to_string()))
    }
}

fn ps(q: Quirks) -> PeopleSoft<Scripted> {
    let mut c = PeopleSoftConfig::udst();
    c.min_interval_ms = 0;
    PeopleSoft::new(Session::new(Scripted::new(q), c))
}
fn posts(p: &mut PeopleSoft<Scripted>) -> Vec<String> {
    p.session_mut().http_mut().posts()
}
fn write_posts(p: &mut PeopleSoft<Scripted>) -> Vec<String> {
    posts(p).into_iter().filter(|a| a.starts_with("DESCR10$") || a == "VC_PE_SS_WRK_VC_SAVE_PLAN_PB").collect()
}
fn armed(q: Quirks) -> PeopleSoft<Scripted> {
    let mut p = ps(q);
    p.arm_writes();
    p
}
fn phrase(p: &frankie_connector_peoplesoft::gate::Preview) -> String {
    p.plan.required_phrase.clone()
}

#[test]
fn a_plan_change_sets_the_term_then_saves_and_proves_it_by_reading_back() {
    let mut p = armed(Quirks::default());
    let pre = p.preview_plan_term(SOFT.0, SOFT.1, "1278").unwrap();
    assert_eq!(pre.plan.term, "Fall 2027");
    assert!(pre.plan.summary.contains("SOFT 2301"), "{}", pre.plan.summary);
    assert_eq!(phrase(&pre), "CONFIRM PLANTERM SOFT2301 1278");
    assert!(write_posts(&mut p).is_empty(), "a preview sends nothing that changes data");
    let out = p.confirm(&pre.token, &phrase(&pre)).unwrap();
    assert_eq!(out.detail["saved"], true);
    assert_eq!(out.detail["others_unchanged"], true);
    assert_eq!(write_posts(&mut p), vec!["DESCR10$19".to_string(), "VC_PE_SS_WRK_VC_SAVE_PLAN_PB".to_string()]);
}

#[test]
fn the_enrol_click_and_every_enrol_drop_swap_action_are_never_sent() {
    let mut p = armed(Quirks::default());
    let pre = p.preview_plan_term(SOFT.0, SOFT.1, "1278").unwrap();
    p.confirm(&pre.token, &phrase(&pre)).unwrap();
    for a in posts(&mut p) {
        for banned in ["ENROL", "DROP", "SWAP", "#ICYes", "SSR_OPTION"] {
            assert!(!a.contains(banned), "plan write sent {a}");
        }
    }
}

#[test]
fn nothing_is_sent_while_writes_are_off() {
    let mut p = ps(Quirks::default());
    assert!(matches!(p.preview_plan_term(SOFT.0, SOFT.1, "1278"), Err(PsError::WriteRefused("writes are off"))));
    assert!(p.session_mut().http_mut().log.is_empty());
}

#[test]
fn a_wrong_phrase_sends_nothing_and_a_token_works_once() {
    let mut p = armed(Quirks::default());
    let pre = p.preview_plan_term(SOFT.0, SOFT.1, "1278").unwrap();
    assert_eq!(p.confirm(&pre.token, "CONFIRM PLANTERM SOFT2301 1288").unwrap_err(), ConfirmError::Gate(GateError::PhraseMismatch));
    assert!(write_posts(&mut p).is_empty());
    p.confirm(&pre.token, &phrase(&pre)).unwrap();
    assert_eq!(p.confirm(&pre.token, &phrase(&pre)).unwrap_err(), ConfirmError::Gate(GateError::UnknownToken));
}

#[test]
fn rows_that_cannot_be_planned_and_terms_not_offered_are_refused_at_preview() {
    let mut p = armed(Quirks::default());
    // year 1, position 0 is INFS 2101, already taken: no dropdown
    assert!(matches!(p.preview_plan_term(1, 0, "1278"), Err(PsError::NotFound(_))));
    // a course list row
    assert!(matches!(p.preview_plan_term(1, 9, "1278"), Err(PsError::NotFound(_))));
    // a term the dropdown does not have
    assert!(matches!(p.preview_plan_term(SOFT.0, SOFT.1, "9999"), Err(PsError::NotFound(_))));
    // already in that term
    assert!(matches!(p.preview_plan_term(SOFT.0, SOFT.1, "1275"), Err(PsError::InvalidInput(_))));
    // junk
    assert!(matches!(p.preview_plan_term(SOFT.0, SOFT.1, "12x8"), Err(PsError::InvalidInput(_))));
    assert!(matches!(p.preview_plan_term(99, 0, "1278"), Err(PsError::InvalidInput(_))));
    assert!(write_posts(&mut p).is_empty());
}

#[test]
fn a_plan_that_changed_since_the_preview_is_not_overwritten() {
    let mut p = armed(Quirks { drift_after_first_read: true, ..Quirks::default() });
    let pre = p.preview_plan_term(SOFT.0, SOFT.1, "1278").unwrap();
    let err = p.confirm(&pre.token, &phrase(&pre)).unwrap_err();
    assert!(matches!(err, ConfirmError::Ps(PsError::Unexpected(m)) if m.contains("changed since the preview")), "{err:?}");
    assert!(write_posts(&mut p).is_empty(), "nothing is sent when the plan is not as previewed");
}

#[test]
fn if_peoplesoft_does_not_show_the_new_term_nothing_is_saved() {
    let mut p = armed(Quirks { ignore_change: true, ..Quirks::default() });
    let pre = p.preview_plan_term(SOFT.0, SOFT.1, "1278").unwrap();
    assert!(p.confirm(&pre.token, &phrase(&pre)).is_err());
    assert_eq!(write_posts(&mut p), vec!["DESCR10$19".to_string()], "the Save click must not follow a change PeopleSoft did not show");
}

#[test]
fn a_save_that_did_nothing_or_touched_another_course_is_reported_not_trusted() {
    // Year 2, position 0 is DACS 3101 (row 21), planned in Fall 2027 (1278); the test moves it to Winter 2028 (1281).
    for q in [Quirks { save_noop: true, ..Quirks::default() }, Quirks { save_hits_other: true, ..Quirks::default() }] {
        let mut p = armed(q);
        let pre = p.preview_plan_term(2, 0, "1281").unwrap();
        let err = p.confirm(&pre.token, &phrase(&pre)).unwrap_err();
        assert!(matches!(err, ConfirmError::Ps(PsError::Unexpected(m)) if m.contains("does not match")), "{err:?}");
    }
}

// ------------------------------------------------------------------ the failure matrix of the plan write

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

/// Counts every request and breaks the `at`-th one (0-based) in the way `fault` says.
struct Faulty {
    inner: Scripted,
    at: usize,
    fault: Fault,
    n: usize,
}

impl Http for Faulty {
    fn send(&mut self, req: HttpRequest) -> Result<HttpResponse, AuthFailure> {
        let i = self.n;
        self.n += 1;
        if i == self.at {
            let page = |status: u16, body: &str| HttpResponse { status, location: None, content_type: Some("text/html".into()), body: body.into() };
            match self.fault {
                Fault::Network => return Err(AuthFailure::Network),
                Fault::LostAnswer => {
                    let _ = self.inner.send(req)?;
                    return Err(AuthFailure::Timeout);
                }
                Fault::Http(c) => return Ok(page(c, "Service error")),
                Fault::Empty => return Ok(page(200, "")),
                Fault::Maintenance => return Ok(page(200, "<html><body><h1>The system is down for maintenance. Please try again later.</h1></body></html>")),
            }
        }
        self.inner.send(req)
    }
}

fn faulty_ps(at: usize, fault: Fault) -> PeopleSoft<Faulty> {
    let mut c = PeopleSoftConfig::udst();
    c.min_interval_ms = 0;
    let mut p = PeopleSoft::new(Session::new(Faulty { inner: Scripted::new(Quirks::default()), at, fault, n: 0 }, c));
    p.arm_writes();
    p
}

/// Preview then confirm on one session. Returns what happened and the requests the scripted PeopleSoft saw.
fn run_plan_flow(p: &mut PeopleSoft<Faulty>) -> Result<serde_json::Value, String> {
    let pre = p.preview_plan_term(SOFT.0, SOFT.1, "1278").map_err(|e| format!("preview: {e:?}"))?;
    let ph = pre.plan.required_phrase.clone();
    p.confirm(&pre.token, &ph).map(|o| o.detail).map_err(|e| format!("confirm: {e:?}"))
}

#[test]
fn every_request_of_the_plan_write_under_every_fault_fails_safely() {
    // A clean run first, to learn how many requests the flow makes.
    let mut clean = faulty_ps(usize::MAX, Fault::Network);
    let ok = run_plan_flow(&mut clean).expect("the clean run works");
    assert_eq!(ok["saved"], true);
    let total = clean.session_mut().http_mut().n;
    assert!(total >= 8, "the flow makes several requests: {total}");
    let mut cases = 0;
    let mut sent_after_failure = vec![];
    let mut claimed_success_wrongly = vec![];
    let mut empty_messages = vec![];
    for at in 0..total {
        for fault in FAULTS {
            let mut p = faulty_ps(at, fault);
            let res = run_plan_flow(&mut p);
            cases += 1;
            let h = p.session_mut().http_mut();
            // Which requests came after the broken one? A write (the term change or Save) must not.
            let writes_after: Vec<_> = h.inner.log.iter().skip(at + 1).filter(|(m, a)| *m == "POST" && (a.starts_with("DESCR10$") || a == "VC_PE_SS_WRK_VC_SAVE_PLAN_PB")).cloned().collect();
            let broken_is_write = h.inner.log.get(at).is_some_and(|(m, a)| *m == "POST" && (a.starts_with("DESCR10$") || a == "VC_PE_SS_WRK_VC_SAVE_PLAN_PB"));
            let _ = broken_is_write;
            match &res {
                Ok(d) => {
                    // Success is claimed only when the plan really changed.
                    let changed = h.inner.terms.get(&SOFT.2).map(String::as_str) == Some("1278");
                    if !(d["saved"] == true && changed) {
                        claimed_success_wrongly.push((at, fault));
                    }
                }
                Err(m) => {
                    if m.trim().is_empty() || m.contains("Unexpected(\"\")") {
                        empty_messages.push((at, fault));
                    }
                }
            }
            // After a failed step the only writes that may follow are none at all (the flow stops at the first failure).
            if res.is_err() && !writes_after.is_empty() {
                sent_after_failure.push((at, fault, writes_after));
            }
        }
    }
    assert!(cases >= 8 * total, "{cases} cases");
    assert!(sent_after_failure.is_empty(), "writes were sent after a failure: {sent_after_failure:?}");
    assert!(claimed_success_wrongly.is_empty(), "success claimed without a saved change: {claimed_success_wrongly:?}");
    assert!(empty_messages.is_empty(), "empty failure messages: {empty_messages:?}");
}

#[test]
fn a_lost_answer_on_the_save_is_an_error_even_though_the_plan_may_have_changed() {
    // Find the Save request, lose its answer, and check the flow reports an error (never "saved").
    let mut clean = faulty_ps(usize::MAX, Fault::Network);
    run_plan_flow(&mut clean).unwrap();
    let save_at = clean.session_mut().http_mut().inner.log.iter().position(|(m, a)| *m == "POST" && a == "VC_PE_SS_WRK_VC_SAVE_PLAN_PB").unwrap();
    let mut p = faulty_ps(save_at, Fault::LostAnswer);
    let res = run_plan_flow(&mut p);
    assert!(res.is_err(), "a lost Save answer must not read as success: {res:?}");
    assert_eq!(p.session_mut().http_mut().inner.terms.get(&SOFT.2).map(String::as_str), Some("1278"), "the scripted PeopleSoft did apply it: this is the unclear case the message must warn about");
}
