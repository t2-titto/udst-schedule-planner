use frankie_auth::{AuthFailure, Credentials};
use frankie_connector_peoplesoft::connector::ConfirmError;
use frankie_connector_peoplesoft::gate::GateError;
use frankie_connector_peoplesoft::login::login;
use frankie_connector_peoplesoft::model::CourseKey;
use frankie_connector_peoplesoft::testkit::*;
use frankie_connector_peoplesoft::{PeopleSoft, PeopleSoftConfig, PsError, Session};

fn cfg() -> PeopleSoftConfig {
    let mut c = PeopleSoftConfig::udst();
    c.min_interval_ms = 0;
    c
}
fn ps(sc: Scenario) -> PeopleSoft<MockPs> {
    PeopleSoft::new(Session::new(MockPs::signed_in(sc), cfg()))
}
fn log_of(p: &mut PeopleSoft<MockPs>) -> Vec<Seen> {
    p.session_mut().http_mut().log.clone()
}
fn writes(p: &mut PeopleSoft<MockPs>) -> Vec<String> {
    p.session_mut().http_mut().write_actions_seen()
}
fn key() -> CourseKey {
    CourseKey {
        strm: "9999".into(),
        crse_id: "000001".into(),
        crse_offer_nbr: "1".into(),
        acad_career: "CRED".into(),
        institution: "TESTI".into(),
    }
}

// ------------------------------------------------------------------ login

#[test]
fn login_succeeds_and_never_shows_the_password() {
    let mut s = Session::new(MockPs::new(Scenario::default()), cfg());
    let creds = Credentials::new(FAKE_USER, FAKE_PASSWORD).unwrap();
    login(&mut s, &creds).unwrap();
    assert!(!format!("{creds:?}").contains(FAKE_PASSWORD));
    let log = &s.http_mut().log;
    assert_eq!(log.iter().filter(|x| x.method == "POST").count(), 1);
}

#[test]
fn wrong_password_stops_after_one_post() {
    let mut s = Session::new(
        MockPs::new(Scenario {
            wrong_password: true,
            ..Default::default()
        }),
        cfg(),
    );
    let creds = Credentials::new(FAKE_USER, "wrong").unwrap();
    assert_eq!(
        login(&mut s, &creds),
        Err(PsError::Auth(AuthFailure::CredentialsRejected))
    );
    assert_eq!(
        s.http_mut()
            .log
            .iter()
            .filter(|x| x.method == "POST")
            .count(),
        1,
        "no retry after a rejection"
    );
}

#[test]
fn a_307_on_the_login_post_is_refused_not_replayed() {
    let mut s = Session::new(
        MockPs::new(Scenario {
            login_307: true,
            ..Default::default()
        }),
        cfg(),
    );
    let creds = Credentials::new(FAKE_USER, FAKE_PASSWORD).unwrap();
    assert_eq!(
        login(&mut s, &creds),
        Err(PsError::Auth(AuthFailure::RedirectRefused))
    );
    assert_eq!(
        s.http_mut()
            .log
            .iter()
            .filter(|x| x.method == "POST")
            .count(),
        1
    );
}

// ------------------------------------------------------------------ reads

#[test]
fn reads_work_and_send_no_write_action() {
    let mut p = ps(Scenario::default());
    assert_eq!(p.notification_count().unwrap(), Some(3));
    assert!(!p.landing_tiles().unwrap().is_empty());
    let s = p.course_sections(&key()).unwrap();
    assert_eq!(s.options.len(), 2);
    let o = p.plan_overview().unwrap();
    assert_eq!(o.requirements.len(), 2);
    p.plan_expand_requirement(1).unwrap();
    let l = p.plan_class_list().unwrap();
    assert!(l.can_submit);
    let (k, sec) = p.plan_select_class(0).unwrap();
    assert_eq!(k.strm, "9999");
    assert_eq!(sec.options.len(), 2);
    let (_, terms) = p.drop_terms().unwrap();
    assert_eq!(terms.len(), 2);
    assert_eq!(p.enrolled_classes(0).unwrap().classes.len(), 3);
    assert_eq!(p.swap_candidates().unwrap().len(), 1);
    assert!(
        writes(&mut p).is_empty(),
        "a read must never send a write action"
    );
}

#[test]
fn seat_status_is_cached_so_it_cannot_poll() {
    let mut p = ps(Scenario::default());
    let (c, cached) = p.seat_status(&key(), "9001").unwrap();
    assert!(!cached);
    assert_eq!(c.seats.unwrap().open, 20);
    let before = log_of(&mut p).len();
    let (c2, cached2) = p.seat_status(&key(), "9004").unwrap();
    assert!(cached2);
    assert_eq!(c2.seats.unwrap().open, 3);
    assert_eq!(
        log_of(&mut p).len(),
        before,
        "second call must not touch the network"
    );
    assert_eq!(
        p.seat_status(&key(), "0000").unwrap_err(),
        PsError::NotFound("class number not in this course")
    );
}

#[test]
fn bad_course_key_is_refused_before_any_request() {
    let mut p = ps(Scenario::default());
    let mut k = key();
    k.crse_id = "../etc".into();
    assert!(matches!(
        p.course_sections(&k),
        Err(PsError::InvalidInput(_))
    ));
    assert!(log_of(&mut p).is_empty());
}

#[test]
fn expired_session_is_reported_as_needs_login() {
    let mut p = ps(Scenario {
        session_expired: true,
        ..Default::default()
    });
    assert_eq!(p.course_sections(&key()).unwrap_err(), PsError::NeedsLogin);
}

#[test]
fn redirect_to_another_host_is_refused() {
    let mut p = ps(Scenario {
        redirect_to_foreign_host: true,
        ..Default::default()
    });
    assert!(matches!(p.course_sections(&key()), Err(PsError::Origin(_))));
}

#[test]
fn form_pointing_elsewhere_gives_a_page_without_a_form_and_blocks_posts() {
    let mut p = ps(Scenario {
        foreign_form_action: true,
        ..Default::default()
    });
    p.course_sections(&key()).unwrap();
    p.arm_writes();
    assert!(matches!(
        p.preview_stage_enrol(0),
        Err(PsError::Unexpected(_))
    ));
    assert!(writes(&mut p).is_empty());
}

#[test]
fn a_read_step_post_cannot_carry_a_write_action() {
    let mut p = ps(Scenario::default());
    p.plan_class_list().unwrap();
    let (page, _) = p.drop_terms().unwrap();
    for a in [
        "VC_PE_SS_WRK_VC_SUBMIT_ENROL_PB",
        "DERIVED_SSR_FL_SSR_DROPPED",
        "#ICYes",
        "CQ_SS_SWAP_WRK_CONTINUE_PB",
        "SSR_CLSRCH_F_WK_SSR_OPTION_DESCR$0",
    ] {
        assert!(
            matches!(
                p.session_mut().post(&page, a, &[], "t"),
                Err(PsError::WriteRefused(_))
            ),
            "{a}"
        );
    }
    assert!(writes(&mut p).is_empty());
}

// ------------------------------------------------------------------ writes are off by default

#[test]
fn previews_refuse_while_writes_are_off() {
    let mut p = ps(Scenario::default());
    assert!(!p.writes_armed());
    p.course_sections(&key()).unwrap();
    assert!(matches!(
        p.preview_stage_enrol(0),
        Err(PsError::WriteRefused(_))
    ));
    assert!(matches!(
        p.preview_submit_enrol(),
        Err(PsError::WriteRefused(_))
    ));
    assert!(matches!(
        p.preview_drop(0, "9101"),
        Err(PsError::WriteRefused(_))
    ));
    assert!(matches!(
        p.preview_swap_stage(0, 1),
        Err(PsError::WriteRefused(_))
    ));
    assert!(matches!(
        p.preview_swap_commit(),
        Err(PsError::WriteRefused(_))
    ));
    assert!(writes(&mut p).is_empty());
}

#[test]
fn confirm_without_a_preview_does_nothing() {
    let mut p = ps(Scenario::default());
    p.arm_writes();
    assert_eq!(
        p.confirm("nope", "CONFIRM DROP 9101").unwrap_err(),
        ConfirmError::Gate(GateError::UnknownToken)
    );
    assert!(writes(&mut p).is_empty());
}

// ------------------------------------------------------------------ drop
#[test]
fn drop_preview_uses_this_sessions_guide_ids_and_refuses_without_them() {
    // The review only shows the class when it is opened with the live guide id (as on the real site).
    let mut p = ps(Scenario::default());
    p.arm_writes();
    assert!(p.preview_drop(0, "9101").is_ok());
    // No grouplets listed: the ids cannot be read, so nothing is selected and nothing is sent that could be misapplied.
    let mut p = ps(Scenario { no_drop_guide: true, ..Default::default() });
    p.arm_writes();
    assert!(matches!(p.preview_drop(0, "9101"), Err(frankie_connector_peoplesoft::PsError::Unexpected(m)) if m.contains("guide ids")));
    assert!(!log_of(&mut p).iter().any(|s| s.action.starts_with("P_SELECT")), "no class was selected");
}


#[test]
fn a_page_that_is_not_the_enrolled_list_is_an_error_not_an_empty_list() {
    let mut p = ps(Scenario { wrong_select_page: true, ..Default::default() });
    let r = p.enrolled_classes(0);
    assert!(matches!(r, Err(frankie_connector_peoplesoft::PsError::Unexpected(m)) if m.contains("not this term's enrolled list")), "{r:?}");
    p.arm_writes();
    assert!(p.preview_drop(0, "9101").is_err());
}

#[test]
fn disarming_forgets_what_was_staged_so_a_later_arming_cannot_reuse_it() {
    let mut p = ps(Scenario::default());
    p.arm_writes();
    let (_, sections) = p.plan_select_class(0).unwrap();
    let _ = sections;
    p.preview_stage_enrol(0).and_then(|pv| p.confirm(&pv.token, &pv.plan.required_phrase).map_err(|_| frankie_connector_peoplesoft::PsError::Unexpected("x"))).unwrap();
    assert!(!p.staged_class_numbers().is_empty());
    p.disarm_writes();
    assert!(p.staged_class_numbers().is_empty());
    p.arm_writes();
    assert!(p.preview_submit_enrol().is_err(), "nothing staged by this arming");
}

#[test]
fn drop_needs_the_exact_phrase_then_sends_exactly_two_posts() {
    let mut p = ps(Scenario::default());
    p.arm_writes();
    let pv = p.preview_drop(0, "9101").unwrap();
    assert_eq!(pv.plan.required_phrase, "CONFIRM DROP 9101");
    assert!(
        writes(&mut p).is_empty(),
        "preview must not change anything"
    );
    assert_eq!(
        p.confirm(&pv.token, "CONFIRM DROP 9001").unwrap_err(),
        ConfirmError::Gate(GateError::PhraseMismatch)
    );
    assert!(writes(&mut p).is_empty(), "a wrong phrase sends nothing");
    let out = p.confirm(&pv.token, "CONFIRM DROP 9101").unwrap();
    assert_eq!(
        writes(&mut p),
        vec![
            "DERIVED_SSR_FL_SSR_DROPPED".to_string(),
            "#ICYes".to_string()
        ]
    );
    assert_eq!(out.detail["classes"][0]["status"], "Dropped");
    // the token is gone
    assert_eq!(
        p.confirm(&pv.token, "CONFIRM DROP 9101").unwrap_err(),
        ConfirmError::Gate(GateError::UnknownToken)
    );
    assert_eq!(writes(&mut p).len(), 2);
}

#[test]
fn drop_of_an_unknown_class_number_is_refused_at_preview() {
    let mut p = ps(Scenario::default());
    p.arm_writes();
    assert!(matches!(
        p.preview_drop(0, "4242"),
        Err(PsError::NotFound(_))
    ));
    assert!(matches!(
        p.preview_drop(0, "9101; DROP"),
        Err(PsError::InvalidInput(_))
    ));
    assert!(matches!(
        p.preview_drop(7, "9101"),
        Err(PsError::NotFound(_))
    ));
    assert!(writes(&mut p).is_empty());
}

#[test]
fn drop_stops_before_the_final_yes_if_the_page_reports_a_problem() {
    let mut p = ps(Scenario {
        drop_prompt_error: true,
        ..Default::default()
    });
    p.arm_writes();
    let pv = p.preview_drop(0, "9101").unwrap();
    let out = p.confirm(&pv.token, "CONFIRM DROP 9101").unwrap();
    assert_eq!(
        writes(&mut p),
        vec!["DERIVED_SSR_FL_SSR_DROPPED".to_string()],
        "no #ICYes after an error"
    );
    assert!(!out.messages.is_empty());
}

// ------------------------------------------------------------------ enrol

#[test]
fn enrol_submit_refuses_when_nothing_was_staged_here() {
    let mut p = ps(Scenario::default());
    p.arm_writes();
    assert!(matches!(
        p.preview_submit_enrol(),
        Err(PsError::WriteRefused(_))
    ));
}

#[test]
fn stage_then_submit_each_need_their_own_phrase() {
    let mut p = ps(Scenario::default());
    p.arm_writes();
    p.course_sections(&key()).unwrap();
    let st = p.preview_stage_enrol(0).unwrap();
    assert_eq!(st.plan.required_phrase, "CONFIRM STAGE 9001 9002");
    assert!(st.plan.warnings.iter().any(|w| w.contains("inferred")));
    p.confirm(&st.token, &st.plan.required_phrase).unwrap();
    assert_eq!(p.staged_class_numbers(), ["9001", "9002"]);

    let sb = p.preview_submit_enrol().unwrap();
    assert_eq!(sb.plan.required_phrase, "CONFIRM ENROLL 9001 9002");
    assert!(sb.plan.warnings.iter().any(|w| w.contains("fees")));
    // the stage phrase does not authorise the submit
    assert_eq!(
        p.confirm(&sb.token, "CONFIRM STAGE 9001 9002").unwrap_err(),
        ConfirmError::Gate(GateError::PhraseMismatch)
    );
    assert_eq!(
        writes(&mut p),
        vec!["SSR_CLSRCH_F_WK_SSR_OPTION_DESCR$0".to_string()]
    );
    let out = p.confirm(&sb.token, "CONFIRM ENROLL 9001 9002").unwrap();
    assert_eq!(out.detail["icons"][0], "success");
    assert!(p.staged_class_numbers().is_empty());
    assert_eq!(
        p.session_mut()
            .http_mut()
            .count("VC_PE_SS_WRK_VC_SUBMIT_ENROL_PB"),
        1
    );
}

#[test]
fn enrol_failure_text_is_returned_as_data() {
    let mut p = ps(Scenario {
        submit_fails: true,
        ..Default::default()
    });
    p.arm_writes();
    p.course_sections(&key()).unwrap();
    let st = p.preview_stage_enrol(1).unwrap();
    assert!(
        st.plan.warnings.iter().any(|w| w.contains("no open seats")),
        "{:?}",
        st.plan.warnings
    );
    p.confirm(&st.token, &st.plan.required_phrase).unwrap();
    let sb = p.preview_submit_enrol().unwrap();
    let out = p.confirm(&sb.token, &sb.plan.required_phrase).unwrap();
    assert!(out.messages.iter().any(|m| m.contains("not successful")));
}

#[test]
fn disarming_drops_pending_previews() {
    let mut p = ps(Scenario::default());
    p.arm_writes();
    let pv = p.preview_drop(0, "9101").unwrap();
    p.disarm_writes();
    p.arm_writes();
    assert_eq!(
        p.confirm(&pv.token, &pv.plan.required_phrase).unwrap_err(),
        ConfirmError::Gate(GateError::UnknownToken)
    );
    assert!(writes(&mut p).is_empty());
}

// ------------------------------------------------------------------ swap

#[test]
fn swap_commit_needs_a_staged_swap_in_this_session() {
    let mut p = ps(Scenario::default());
    p.arm_writes();
    assert!(matches!(
        p.preview_swap_commit(),
        Err(PsError::WriteRefused(_))
    ));
}

#[test]
fn swap_stage_then_commit_each_gated() {
    let mut p = ps(Scenario::default());
    p.arm_writes();
    let st = p.preview_swap_stage(0, 1).unwrap();
    assert_eq!(st.plan.required_phrase, "CONFIRM SWAPSTAGE 9003 9004");
    p.confirm(&st.token, &st.plan.required_phrase).unwrap();
    let cm = p.preview_swap_commit().unwrap();
    assert_eq!(cm.plan.required_phrase, "CONFIRM SWAP 9001 TO 9003 9004");
    let out = p.confirm(&cm.token, &cm.plan.required_phrase).unwrap();
    assert!(
        out.messages.iter().any(|m| m.contains("Swap completed")),
        "{:?}",
        out.messages
    );
    assert_eq!(
        p.session_mut()
            .http_mut()
            .count("CQ_SS_SWAP_WRK_CONTINUE_PB"),
        1
    );
}

#[test]
fn a_swap_review_that_shows_another_swap_gets_no_commit_preview() {
    let mut p = ps(Scenario { swap_review_other: true, ..Default::default() });
    p.arm_writes();
    let st = p.preview_swap_stage(0, 1).unwrap();
    p.confirm(&st.token, &st.plan.required_phrase).unwrap();
    let r = p.preview_swap_commit();
    assert!(matches!(r, Err(frankie_connector_peoplesoft::PsError::Unexpected(m)) if m.contains("does not show the swap")));
    assert_eq!(p.session_mut().http_mut().count("CQ_SS_SWAP_WRK_CONTINUE_PB"), 0);
}

#[test]
fn two_courses_swap_through_one_commit_and_the_review_must_list_both() {
    // Stage the same mock course twice is not possible (one candidate), so check the plan names the staged pair and phrase.
    let mut p = ps(Scenario::default());
    p.arm_writes();
    let st = p.preview_swap_stage_nums(0, &["9003".to_string(), "9004".to_string()]).unwrap();
    assert_eq!(st.plan.class_numbers, ["9003", "9004"]);
    p.confirm(&st.token, &st.plan.required_phrase).unwrap();
    let cm = p.preview_swap_commit().unwrap();
    assert_eq!(cm.plan.required_phrase, "CONFIRM SWAP 9001 TO 9003 9004");
    // The guide was loaded the way the browser does: start page, preload, grouplets, then the select step, before and after the stage.
    let h = p.session_mut().http_mut();
    let preloads = h.log.iter().filter(|x| x.path.contains("IScript_FluidAGPreLoad")).count();
    assert!(preloads >= 2, "the guide is loaded before the select step and again after the stage: {preloads}");
}

#[test]
fn naming_an_option_by_class_numbers_that_the_course_page_does_not_offer_is_refused() {
    let mut p = ps(Scenario::default());
    p.arm_writes();
    let r = p.preview_swap_stage_nums(0, &["1".to_string()]);
    assert!(matches!(r, Err(frankie_connector_peoplesoft::PsError::NotFound(_))));
    assert_eq!(p.session_mut().http_mut().count("SSR_CLSRCH_F_WK_SSR_OPTION_DESCR"), 0, "nothing was staged");
}

#[test]
fn the_swap_result_is_asked_for_after_the_navigation_grouplet_and_carries_the_per_course_answer() {
    let mut p = ps(Scenario { swap_fails: true, ..Default::default() });
    p.arm_writes();
    let st = p.preview_swap_stage_nums(0, &["9003".to_string(), "9004".to_string()]).unwrap();
    p.confirm(&st.token, &st.plan.required_phrase).unwrap();
    let cm = p.preview_swap_commit().unwrap();
    let out = p.confirm(&cm.token, &cm.plan.required_phrase).unwrap();
    let rows = out.detail["results"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["class_nbr"], "9001");
    assert_eq!(rows[0]["changed_to"], "9003");
    assert!(rows[0]["message"].as_str().unwrap().contains("not successful"));
    let seen = &p.session_mut().http_mut().log;
    let nav = seen.iter().position(|x| x.path.contains("PTGP_GROUPLETS_FL") && x.method == "GET").unwrap_or(0);
    let result = seen.iter().rposition(|x| x.path.contains("CQ_SS_SWAP_CLASS")).unwrap();
    assert!(nav < result, "the grouplet comes before the result page");
}

#[test]
fn swap_failure_text_is_reported() {
    let mut p = ps(Scenario {
        swap_fails: true,
        ..Default::default()
    });
    p.arm_writes();
    let st = p.preview_swap_stage(0, 1).unwrap();
    p.confirm(&st.token, &st.plan.required_phrase).unwrap();
    let cm = p.preview_swap_commit().unwrap();
    let out = p.confirm(&cm.token, &cm.plan.required_phrase).unwrap();
    assert!(out.messages.iter().any(|m| m.contains("not successful")));
}

// ------------------------------------------------------------------ tool layer

#[test]
fn tool_layer_returns_valid_results_and_never_confirms() {
    use frankie_connector_peoplesoft::tools::call;
    use serde_json::json;
    let mut p = ps(Scenario::default());
    let r = call(
        &mut p,
        "peoplesoft.course_sections",
        &json!({"strm":"9999","crse_id":"000001","crse_offer_nbr":"1","acad_career":"CRED","institution":"TESTI"}),
    );
    r.validate().unwrap();
    assert_eq!(r.status, frankie_contracts::ToolStatus::Pass);
    // a model asking for a confirm is refused, and nothing is sent
    p.arm_writes();
    let pv = call(
        &mut p,
        "preview_drop",
        &json!({"term_row":0,"class_nbr":"9101"}),
    );
    pv.validate().unwrap();
    let token = pv.data.unwrap()["token"].as_str().unwrap().to_string();
    let c = call(
        &mut p,
        "confirm_write",
        &json!({"token": token, "phrase": "CONFIRM DROP 9101"}),
    );
    c.validate().unwrap();
    assert_ne!(c.status, frankie_contracts::ToolStatus::Pass);
    assert!(writes(&mut p).is_empty());
    let bad = call(&mut p, "nonsense", &json!({}));
    assert_eq!(bad.status, frankie_contracts::ToolStatus::Unsupported);
    let inv = call(&mut p, "enrolled_classes", &json!({"row":"x"}));
    inv.validate().unwrap();
    assert_eq!(
        inv.error.unwrap().code,
        frankie_contracts::ErrorCode::InvalidInput
    );
}

#[test]
fn every_write_capable_descriptor_is_marked_and_confirm_is_the_only_mutating_one() {
    let ds = frankie_connector_peoplesoft::tools::descriptors();
    assert!(ds.len() >= 17);
    let m: Vec<_> = ds
        .iter()
        .filter(|d| d.mutates)
        .map(|d| d.id.as_str())
        .collect();
    assert_eq!(m, vec!["peoplesoft.confirm_write"]);
}

#[test]
fn an_enrolled_courses_swap_options_are_a_read_that_stages_nothing() {
    let mut p = ps(Scenario::default());
    let (key, sections) = p.swap_course_options(0).unwrap();
    assert!(!key.crse_id.is_empty());
    assert!(!sections.options.is_empty());
    assert!(writes(&mut p).is_empty(), "no write action was sent");
    p.arm_writes();
    assert!(p.preview_swap_commit().is_err(), "nothing was staged by reading the options");
}

// ------------------------------------------------------------------ pages that are not the page asked for

/// Wraps the pretend site: from `from` on, every answer to the given stage is an empty 200 or a maintenance page.
struct Blank {
    inner: MockPs,
    stage: &'static str,
    body: &'static str,
}

impl frankie_auth::Http for Blank {
    fn send(&mut self, r: frankie_auth::HttpRequest) -> Result<frankie_auth::HttpResponse, AuthFailure> {
        if r.stage == self.stage {
            return Ok(frankie_auth::HttpResponse { status: 200, location: None, content_type: Some("text/html".into()), body: self.body.into() });
        }
        self.inner.send(r)
    }
}

const MAINTENANCE: &str = "<html><body><h1>The system is down for maintenance.</h1></body></html>";

fn blank(stage: &'static str, body: &'static str) -> PeopleSoft<Blank> {
    PeopleSoft::new(Session::new(Blank { inner: MockPs::signed_in(Scenario::default()), stage, body }, cfg()))
}

#[test]
fn an_empty_or_maintenance_page_is_never_read_as_an_empty_plan_list() {
    for body in ["", MAINTENANCE] {
        let mut p = blank("plan_class_list", body);
        assert!(matches!(p.plan_class_list(), Err(PsError::Unexpected(_))), "body {body:?}");
    }
}

#[test]
fn an_empty_or_maintenance_page_is_never_read_as_no_enrolled_terms() {
    for body in ["", MAINTENANCE] {
        let mut p = blank("drop_terms", body);
        assert!(matches!(p.drop_terms(), Err(PsError::Unexpected(_))), "body {body:?}");
    }
}

#[test]
fn an_empty_or_maintenance_page_is_never_read_as_no_swappable_courses() {
    for body in ["", MAINTENANCE] {
        let mut p = blank("swap_candidates", body);
        assert!(matches!(p.swap_candidates(), Err(PsError::Unexpected(_))), "body {body:?}");
    }
}

#[test]
fn an_empty_or_maintenance_answer_to_opening_the_plan_list_is_never_read_as_an_empty_list() {
    for body in ["", MAINTENANCE] {
        let mut p = blank("plan_open_list", body);
        assert!(matches!(p.plan_class_list(), Err(PsError::Unexpected(_))), "body {body:?}");
    }
}
