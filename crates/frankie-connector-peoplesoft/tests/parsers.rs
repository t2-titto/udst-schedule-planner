use frankie_connector_peoplesoft::config::PeopleSoftConfig;
use frankie_connector_peoplesoft::parse::*;
use frankie_connector_peoplesoft::pia::*;
use frankie_connector_peoplesoft::testkit::*;
use url::Url;

#[test]
fn split_id_handles_every_observed_shape() {
    assert_eq!(split_id("FIELD$3"), ("FIELD".into(), 0, Some(3)));
    assert_eq!(split_id("FIELD$span$3"), ("FIELD".into(), 0, Some(3)));
    assert_eq!(
        split_id("FIELD$294$$span$0"),
        ("FIELD".into(), 294, Some(0))
    );
    assert_eq!(split_id("FIELD$62$$2"), ("FIELD".into(), 62, Some(2)));
    assert_eq!(split_id("ICSID"), ("ICSID".into(), 0, None));
}

#[test]
fn fields_skip_session_ids_and_dedupe_links() {
    let html = "<span id='ICSCRIPTSID'>ptnbsid=SECRET</span><span id='X$span$0'><a id='X$0'>Hello  there</a></span>";
    let fs = fields(html);
    assert_eq!(fs.len(), 1, "{fs:?}");
    assert_eq!(fs[0].text, "Hello there");
    assert!(!format!("{fs:?}").contains("SECRET"));
}

#[test]
fn course_sections_parse_components_seats_meetings() {
    let s = parse_course_sections(&course_info_page());
    assert_eq!(s.term, "Winter 2099");
    assert_eq!(s.course, "TEST 1001");
    assert_eq!(s.options.len(), 2);
    let o = &s.options[0];
    assert_eq!(
        (o.row, o.option_no.as_str(), o.status.as_str()),
        (0, "1", "Open")
    );
    assert_eq!(o.components.len(), 2);
    let l = &o.components[0];
    assert_eq!(
        (l.kind.as_str(), l.class_nbr.as_str(), l.section.as_str()),
        ("Lecture", "9001", "1")
    );
    assert_eq!(l.meetings.len(), 2);
    assert_eq!(l.meetings[0].day, "Monday");
    assert_eq!(l.meetings[0].start, "9:30AM");
    assert_eq!(l.room, "Room T1", "repeated room collapsed");
    assert_eq!(l.instructor, "Dr. Test One");
    assert_eq!(l.seats.as_ref().unwrap().open, 20);
    assert_eq!(s.options[1].components[0].seats.as_ref().unwrap().open, 0);
}

#[test]
fn collapse_repeat_only_collapses_exact_halves() {
    assert_eq!(collapse_repeat("A B A B"), "A B");
    assert_eq!(collapse_repeat("A B C D"), "A B C D");
    assert_eq!(collapse_repeat(""), "");
}

#[test]
fn plan_overview_groups_courses_under_their_requirement() {
    let o = parse_plan_overview(&plan_overview_page());
    assert_eq!(o.requirements.len(), 2);
    assert_eq!(o.requirements[0].name, "REQ-1 Invented Core");
    assert_eq!(o.requirements[0].courses[0].code, "TEST 1001");
    assert_eq!(o.requirements[0].courses[0].category, "Required");
    assert_eq!(o.requirements[1].name, "REQ-2 Invented Elective");
    assert_eq!(o.requirements[1].courses[0].row, 1);
}

#[test]
fn plan_class_list_and_submit_button() {
    let l = parse_plan_class_list(&plan_list_page(true));
    assert_eq!(l.rows.len(), 2);
    assert!(l.can_submit);
    assert_eq!(l.rows[0].status, "Selected");
    assert!(!parse_plan_class_list(&plan_list_page(false)).can_submit);
}

#[test]
fn enrolled_classes_extract_class_numbers() {
    let e = parse_enrolled(&drop_select_page());
    assert_eq!(e.term, "Winter 2099");
    assert_eq!(e.classes.len(), 3);
    assert_eq!(e.classes[2].class_nbr, "9101");
    assert_eq!(e.classes[2].component, "Lecture");
    assert_eq!(
        parse_terms(&drop_terms_page()),
        vec![(0, "Winter 2099".to_string()), (1, "Fall 2098".to_string())]
    );
}

#[test]
fn swap_pages_and_failure_text() {
    let c = parse_swap_candidates(&swap_select_page());
    assert_eq!(c.len(), 1);
    assert!(c[0].class_text.contains("9001"));
    let r = parse_swap_review(&swap_review_page());
    assert!(r.new_class.contains("Class 9003") && r.new_class.contains("Class 9004"), "{}", r.new_class);
    let bad = parse_swap_review(&swap_result_page(false));
    assert!(bad.messages.iter().any(|m| m.contains("not successful")));
    assert!(parse_swap_review(&swap_result_page(true))
        .messages
        .iter()
        .any(|m| m == "Swap completed"));
}

#[test]
fn enrol_confirmation_reads_amount_and_icons() {
    let ok = parse_enrol_confirmation(&enrol_conf_html(true));
    assert_eq!(ok.total_amount, "0.00");
    assert_eq!(ok.icons, vec!["success"]);
    let bad = parse_enrol_confirmation(&enrol_conf_html(false));
    assert_eq!(bad.icons, vec!["error"]);
    assert!(!bad.messages.is_empty());
}

#[test]
fn ajax_unwrap_finds_redirect_and_state() {
    let body = xml(
        "<input type='hidden' name='ICStateNum' value='9'/>",
        "oDoc.win9.ICStateNum.value=11; document.location='https://campus.udst.edu.qa/psc/x';",
    );
    let a = unwrap_ajax(&body);
    assert!(a.is_xml);
    assert_eq!(
        a.redirect.as_deref(),
        Some("https://campus.udst.edu.qa/psc/x")
    );
    assert!(a
        .set_values
        .contains(&("ICStateNum".to_string(), "11".to_string())));
    let plain = unwrap_ajax("<html>hi</html>");
    assert!(!plain.is_xml);
}

#[test]
fn form_must_target_the_allowed_host_and_hides_values() {
    let cfg = PeopleSoftConfig::udst();
    let page = Url::parse("https://campus.udst.edu.qa/psc/csqaprd9/EMPLOYEE/SA/c/X.GBL").unwrap();
    let f = FormState::parse(&course_info_page(), &page, &cfg).unwrap();
    assert_eq!(f.get("ICSID"), Some(FAKE_SESSION));
    assert!(
        !format!("{f:?}").contains(FAKE_SESSION),
        "Debug must not show hidden values"
    );
    let evil =
        course_info_page().replace("https://campus.udst.edu.qa/psc", "https://evil.example/psc");
    assert_eq!(
        FormState::parse(&evil, &page, &cfg),
        Err(FormError::BadTarget)
    );
    let wrong_path = course_info_page().replace("/psc/csqaprd9_9/", "/other/");
    assert_eq!(
        FormState::parse(&wrong_path, &page, &cfg),
        Err(FormError::BadTarget)
    );
}

#[test]
fn build_post_forces_action_and_rejects_bad_names() {
    let cfg = PeopleSoftConfig::udst();
    let page = Url::parse("https://campus.udst.edu.qa/psc/csqaprd9/EMPLOYEE/SA/c/X.GBL").unwrap();
    let f = FormState::parse(&course_info_page(), &page, &cfg).unwrap();
    let b = build_post(&f, "P_SELECT$0", &[("P_SELECT$0", "Y")]).unwrap();
    assert!(b.contains("ICAction=P_SELECT%240"));
    assert!(b.contains("ICAJAX=1") && b.contains("ICResubmit=0") && b.contains("ICType=Panel"));
    assert!(b.contains("ICStateNum=1"));
    assert_eq!(b.matches("ICAction=").count(), 1);
    assert!(build_post(&f, "bad action;rm", &[]).is_err());
    assert!(build_post(&f, "", &[]).is_err());
    assert!(!valid_action("a b"));
    assert!(valid_action("#ICYes") && valid_action("#ICSetFieldSSR_CRSE_INFO_FL.SSR_CNTRLFIELD.A"));
}

#[test]
fn classify_pages() {
    assert_eq!(classify(&login_page(false)), PageKind::LoginForm);
    assert_eq!(
        classify("<p>Your session has timed out.</p>"),
        PageKind::SessionExpired
    );
    assert_eq!(
        classify("<p>Authorization Error</p>"),
        PageKind::NotAuthorized
    );
    assert_eq!(classify(&course_info_page()), PageKind::Content);
}

#[test]
fn enrolled_lab_row_inherits_course_and_status_and_lists_are_deduped() {
    let e = parse_enrolled(&drop_select_page());
    // invented page: row 1 (the lab) has its own text for course/status, so build one without them
    let html = drop_select_page().replace("TEST 1001 Introduction to Testing", "");
    let e2 = parse_enrolled(&html);
    assert_eq!(e.classes[1].course, "TEST 1001 Introduction to Testing");
    assert!(e2.classes.iter().all(|c| !c.status.is_empty()));
    let l = parse_plan_class_list(&plan_list_page(true));
    assert_eq!(l.term_label, "Winter 2099");
    assert_eq!(l.rows[0].term, "Winter 2099");
}

#[test]
fn parses_catalog_detail_with_requirement_and_components() {
    let html = r#"
        <span id="SSR_CRSE_DTL_WK_SSS_SUBJ_CATLG">TEST 2010</span>
        <span id="DERIVED_CRSECAT_DESCR200">Applied Testing</span>
        <span id="DERIVED_CRSECAT_UNITS_RANGE$0">3.00</span>
        <span id="SSR_CRSE_OFF_VW_ACAD_CAREER$0">Credit</span>
        <span id="SSR_CRSE_OFF_VW_GRADING_BASIS$0">60 Percent and Letter Grade</span>
        <span id="DERIVED_CRSECAT_DESCR40$0">Lecture</span>
        <span id="DERIVED_CRSECAT_DESCR40$1">Laboratory</span>
        <span id="CAMPUS_TBL_DESCR$0">Doha Campus</span>
        <span id="ACAD_GROUP_TBL_DESCR$0">College of Engineering</span>
        <span id="ACAD_ORG_TBL_DESCR$0">University of Doha</span>
        <span id="DERIVED_CRSECAT_DESCR254A$0">TEST1010 is the pre-requisite for TEST2010</span>
        <span id="DERIVED_CRSECAT_SSR_CRSE_ATTR_LONG$0">Exempt from Supplementary</span>
        <span id="SSR_CRSE_OFF_VW_DESCRLONG$0">Course description here.</span>
    "#;
    let d = parse_catalog_detail(html).unwrap();
    assert_eq!(d.code, "TEST 2010");
    assert_eq!(d.title, "Applied Testing");
    assert_eq!(d.units, "3.00");
    assert_eq!(d.components, vec!["Lecture", "Laboratory"]);
    assert_eq!(d.requirement, "TEST1010 is the pre-requisite for TEST2010");
    assert_eq!(d.description, "Course description here.");
}
