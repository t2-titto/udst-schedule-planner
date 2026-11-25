//! INVENTED pages and a mock server, for tests only. Every name, number, room and instructor below is made up;
//! nothing here was copied from a real student's pages. The shapes (ids, hidden inputs, XML wrapper) follow what
//! the real site does.
#![allow(dead_code)]
use frankie_auth::{AuthFailure, Http, HttpRequest, HttpResponse, Method};
use url::Url;

pub const FAKE_USER: &str = "10000001"; // invented
pub const FAKE_PASSWORD: &str = "MOCK-PASSWORD-VALUE"; // invented
pub const FAKE_SESSION: &str = "FAKE-SESSION-ID-0000";
const HOST: &str = "https://campus.udst.edu.qa";

fn span(id: &str, text: &str) -> String {
    format!("<DIV id='win9div{id}'><span class='ps_box-value'   id='{id}' >{text}</span></DIV>\n")
}
/// `id` is `BASE$ROW`; the real site wraps the anchor in `<span id='BASE$span$ROW'>`.
fn link(id: &str, text: &str) -> String {
    let (base, row) = id.rsplit_once('$').expect("BASE$ROW");
    format!("<span id='{base}$span${row}'  class='ps-link-wrapper'><a id='{id}'  class='ps-link' href=\"javascript:submitAction_win9(document.win9,'{id}');\" >{text}</a></span>\n")
}

pub fn hidden(state: u32) -> String {
    format!(
        "<input type='hidden' name='ICElementNum' id='ICElementNum' value='0' />\n\
         <input type='hidden' name='ICStateNum' id='ICStateNum' value='{state}' />\n\
         <input type='hidden' name='ICAction' id='ICAction' value='None' />\n\
         <input type='hidden' name='ICSID' id='ICSID' value='{FAKE_SESSION}' />\n\
         <input type='hidden' name='ICPanelControlStyle' id='ICPanelControlStyle' value=' pst_side1-fixed' />\n"
    )
}

/// A full page: one PIA form whose action is on the allow-listed host.
pub fn full_page(component: &str, body: &str) -> String {
    format!(
        "<html><body><form id='F' name='win9' method='post' action=\"{HOST}/psc/csqaprd9_9/EMPLOYEE/SA/c/{component}.GBL\" autocomplete='off'>\n{}{body}</form></body></html>",
        hidden(1)
    )
}

/// An XML answer: FIELD html plus an optional script.
pub fn xml(field_html: &str, script: &str) -> String {
    format!(
        "<?xml version='1.0' encoding='utf-8' ?>\n<PAGE id='X'><FIELD id='divPAGECONTAINER'><![CDATA[{field_html}]]></FIELD><GENSCRIPT id='script'><![CDATA[{script}]]></GENSCRIPT></PAGE>"
    )
}

pub fn redirect_script(to: &str) -> String {
    format!("document.location='{to}';")
}

pub fn course_info_page() -> String {
    let mut b = String::new();
    b += &span("TERM_VAL_TBL_DESCR", "Winter 2099");
    b += &span("SSR_CRSE_INFO_V_SSS_SUBJ_CATLG", "TEST 1001");
    b += &span(
        "SSR_CRSE_INFO_V_COURSE_TITLE_LONG",
        "Introduction to Testing",
    );
    b += &span(
        "SSR_CRSE_INFO_V_DESCRLONG",
        "An invented course used only by the test suite.",
    );
    b += &span("SSR_CLSRCH_F_WK_UNITS_RANGE", "3.00");
    b += &span("SSR_CLSRCH_F_WK_SSR_GRAD_BASIS_LNG", "Letter Grade");
    b += &span("SSR_CLSRCH_F_WK_SSR_COMPONENT_LONG", "Lecture, Laboratory");
    b += &span("SSR_CLSRCH_F_WK_SSR_ACADCAR_LONG", "Credit");
    b += &span("SSR_CLSRCH_F_WK_SSR_MSG_TEXT", "2 options");
    let opts = [
        (
            "1",
            "Open",
            "9001",
            "9002",
            "Open Seats 20 of 20",
            "Open Seats 7 of 20",
        ),
        (
            "2",
            "Open",
            "9003",
            "9004",
            "Open Seats 0 of 20",
            "Open Seats 3 of 20",
        ),
    ];
    for (i, (no, st, l1, l2, s1, s2)) in opts.iter().enumerate() {
        b += &span(
            &format!("SSR_CLSRCH_F_WK_SSR_OPTION_DESCR$306$$span${i}"),
            no,
        );
        b += &span(&format!("SSR_DER_CS_GRP_SSR_OPTION_STAT${i}"), st);
        b += &span(
            &format!("SSR_DER_CS_GRP_SESSION_CODE$215$${i}"),
            "Regular Academic Session",
        );
        b += &span(
            &format!("SSR_CLSRCH_F_WK_SSR_CMPNT_DESCR_1$294$$span${i}"),
            &format!("Lecture - Class {l1} -Section {}", i * 2 + 1),
        );
        b += &span(
            &format!("SSR_CLSRCH_F_WK_SSR_CMPNT_DESCR_2$295$$span${i}"),
            &format!("Laboratory - Class {l2} -Section {}", i * 2 + 2),
        );
        b += &span(
            &format!("SSR_CLSRCH_F_WK_SSR_MTG_DT_LONG_1$88$${i}"),
            "05/01/2099 - 14/04/2099",
        );
        b += &span(
            &format!("SSR_CLSRCH_F_WK_SSR_MTG_SCHED_L_1$134$${i}"),
            "Monday 9:30AM to 11:00AM Wednesday 9:30AM to 11:00AM",
        );
        b += &span(
            &format!("SSR_CLSRCH_F_WK_SSR_MTG_SCHED_L_2$135$${i}"),
            "Sunday 8:00AM to 11:00AM",
        );
        b += &span(
            &format!("SSR_CLSRCH_F_WK_SSR_MTG_LOC_LONG_1${i}"),
            "Room T1\nRoom T1",
        );
        b += &span(&format!("SSR_CLSRCH_F_WK_SSR_MTG_LOC_LONG_2${i}"), "Lab T2");
        b += &span(
            &format!("SSR_CLSRCH_F_WK_SSR_INSTR_LONG_1$86$${i}"),
            "Dr. Test One\nDr. Test One",
        );
        b += &span(
            &format!("SSR_CLSRCH_F_WK_SSR_INSTR_LONG_2$161$${i}"),
            "Dr. Test Two",
        );
        b += &span(&format!("SSR_CLSRCH_F_WK_SSR_DESCR50_1${i}"), s1);
        b += &span(&format!("SSR_CLSRCH_F_WK_SSR_DESCR50_2${i}"), s2);
    }
    full_page("SSR_STUDENT_FL.SSR_CRSE_INFO_FL", &b)
}

pub fn plan_overview_page() -> String {
    let mut b = String::new();
    b += &span("VC_PE_HDR_DVW_DESCR100$0", "REQ-1 Invented Core");
    b += &span("VC_PE_HDR_DVW_DESCR254$0", "Units needed: 6 Satisfied: 3");
    b += &span("VC_PE_SS_WRK_DESCR35$0", "Not Satisfied");
    b += &span(
        "VC_PE_SS_L3_VW_DESCR50$0",
        "TEST 1001: Introduction to Testing",
    );
    b += &span("SSR_ENRL_CAT$0", "Required");
    b += &span("UNITS_MINIMUM$0", "3.00");
    b += &span("DESCR10$0", "TEST 1001");
    b += &span("VC_PE_HDR_DVW_DESCR100$62$$1", "REQ-2 Invented Elective");
    b += &span("VC_PE_SS_L3_VW_DESCR50$1", "TEST 2002: Advanced Testing");
    b += &span("SSR_ENRL_CAT$1", "Elective");
    b += &span("UNITS_MINIMUM$1", "3.00");
    b += &span("DESCR10$1", "TEST 2002");
    b += &link("RQRMNT_NAME_LBL$0", "");
    b += &link("RQRMNT_NAME_LBL$1", "");
    b += "<a id='VC_PE_SS_WRK_VC_ENROL_PB' href=\"javascript:submitAction_win9(document.win9,'VC_PE_SS_WRK_VC_ENROL_PB');\">Enrol</a>\n";
    full_page("VC_PE_MENU.VC_PE_SS", &b)
}

pub fn plan_list_page(can_submit: bool) -> String {
    let mut b = String::new();
    b += "<label id='VC_PE_SS_WRK_STRM_LBL'>Term</label>";
    b += &span("VC_PE_SS_WRK_STRM", "Winter 2099");
    for (i, (label, code, status)) in [
        (
            "TEST 1001: Introduction to Testing",
            "TEST 1001",
            "Selected",
        ),
        ("TEST 2002: Advanced Testing", "TEST 2002", "Not enrolled"),
    ]
    .iter()
    .enumerate()
    {
        b += &span(&format!("VC_PE_ENROL_VW_DESCR50${i}"), label);
        let _ = code;
        // the live page shows the TERM in this field, not a course code
        b += &span(&format!("VC_PE_ENROL_VW_DESCR${i}"), "Winter 2099");
        b += &span(&format!("VC_PE_SS_WRK_CLASS_DESCR${i}"), status);
        b += &link(&format!("SSR_MUL_MTG_LNK${i}"), "Monday 9:30AM to 11:00AM");
        b += &format!("<a id='VC_PE_SS_WRK_VC_SELECT_CLASS_PB${i}' href='#'>Select</a>\n");
    }
    if can_submit {
        b += "<a id='VC_PE_SS_WRK_VC_SUBMIT_ENROL_PB' href=\"javascript:submitAction_win9(document.win9,'VC_PE_SS_WRK_VC_SUBMIT_ENROL_PB');\">Submit</a>\n";
    }
    full_page("VC_PE_MENU.VC_PE_SS", &b)
}

/// The list as the live page shows it once only the rows in `selected` have a chosen class: the others read
/// "Classes available" and show no schedule.
pub fn plan_list_page_selected(can_submit: bool, selected: &std::collections::BTreeSet<u32>) -> String {
    let mut b = String::new();
    b += "<label id='VC_PE_SS_WRK_STRM_LBL'>Term</label>";
    b += &span("VC_PE_SS_WRK_STRM", "Winter 2099");
    for (i, (label, code, status0)) in [
        (
            "TEST 1001: Introduction to Testing",
            "TEST 1001",
            "Selected",
        ),
        ("TEST 2002: Advanced Testing", "TEST 2002", "Not enrolled"),
    ]
    .iter()
    .enumerate()
    {
        b += &span(&format!("VC_PE_ENROL_VW_DESCR50${i}"), label);
        let _ = code;
        // the live page shows the TERM in this field, not a course code
        b += &span(&format!("VC_PE_ENROL_VW_DESCR${i}"), "Winter 2099");
        let chosen = selected.contains(&(i as u32));
        // The live site keeps saying "Classes available" for a course with a chosen class; the chosen class's schedule is what differs.
        let status = "Classes available";
        let _ = status0;
        b += &span(&format!("VC_PE_SS_WRK_CLASS_DESCR${i}"), status);
        b += &link(&format!("SSR_MUL_MTG_LNK${i}"), if chosen { "Monday 9:30AM to 11:00AM" } else { "" });
        b += &format!("<a id='VC_PE_SS_WRK_VC_SELECT_CLASS_PB${i}' href='#'>Select</a>\n");
    }
    if can_submit {
        b += "<a id='VC_PE_SS_WRK_VC_SUBMIT_ENROL_PB' href=\"javascript:submitAction_win9(document.win9,'VC_PE_SS_WRK_VC_SUBMIT_ENROL_PB');\">Submit</a>\n";
    }
    full_page("VC_PE_MENU.VC_PE_SS", &b)
}

pub fn enrol_conf_html(ok: bool) -> String {
    let mut b = String::new();
    b += &span("VC_PE_SS_WRK_AMOUNT_TOTAL", "0.00");
    b += &span("VC_PE_SS_WRK_PAYMENT_DTL", "No payment due");
    b += &span(
        "VC_PE_STD_ENRL_DESCR50$0",
        "TEST 1001: Introduction to Testing",
    );
    b += if ok {
        "<img src='/cs/csqaprd9/cache/PS_CS_STATUS_SUCCESS_ICN_1.gif'>"
    } else {
        "<img src='/cs/csqaprd9/cache/PS_CS_STATUS_ERROR_ICN_1.gif'>"
    };
    if !ok {
        b += &span(
            "MSG1",
            "Enrollment not successful. Unable to add this class.",
        );
    }
    b
}

pub fn drop_terms_page() -> String {
    full_page(
        "SSR_STUDENT_FL.SSR_DROP_CLASS_FL",
        &(link("TERM_LINK$0", "Winter 2099") + &link("TERM_LINK$1", "Fall 2098")),
    )
}

fn enrolled_rows() -> String {
    let mut b = span("TERM_VAL_TBL_DESCR", "Winter 2099");
    for (i, (nm, course, st)) in [
        (
            "Lecture - Class 9001 - Section 1",
            "TEST 1001 Introduction to Testing",
            "Enrolled",
        ),
        (
            "Laboratory - Class 9002 - Section 2",
            "TEST 1001 Introduction to Testing",
            "Enrolled",
        ),
        (
            "Lecture - Class 9101 - Section 1",
            "TEST 3003 Other Course",
            "Enrolled",
        ),
    ]
    .iter()
    .enumerate()
    {
        b += &link(&format!("P_CLASS_NAME${i}"), nm);
        b += &span(&format!("DERIVED_SSR_FL_SSR_DESCR80${i}"), course);
        b += &span(
            &format!("SSR_MTG_PAT_WRK_SSR_MTG_SCHED1${i}"),
            "Monday 9:30AM to 11:00AM",
        );
        b += &span(&format!("SSR_MTG_PAT_WRK_SSR_ROOM1${i}"), "Room T1");
        b += &span(
            &format!("SSR_MTG_PAT_WRK_SSR_INSTRUCTOR1${i}"),
            "Dr. Test One",
        );
        b += &span(&format!("DERIVED_SSR_FL_SSR_UNITS_LBL${i}"), "3.00");
        b += &span(&format!("DERIVED_REGFRM1_SSR_STATUS_LONG${i}"), st);
    }
    b
}
/// This mock session's live drop-guide instance. Deliberately not the constant in `guided.rs`.
pub const LIVE_GUIDE: &str = "SR_DROP_FL999111";

pub fn drop_select_page() -> String {
    full_page("SSR_STUDENT_FL.SSR_DROP_SELECT_FL", &enrolled_rows())
}
pub fn drop_review_page() -> String {
    // the review lists only the chosen class
    let mut b = span("TERM_VAL_TBL_DESCR", "Winter 2099");
    b += &link("P_CLASS_NAME$0", "Lecture - Class 9101 - Section 1");
    b += &span("DERIVED_SSR_FL_SSR_DESCR80$0", "TEST 3003 Other Course");
    b += &span("DERIVED_REGFRM1_SSR_STATUS_LONG$0", "Enrolled");
    full_page("SSR_STUDENT_FL.SSR_DROP_REVIEW_FL", &b)
}
pub fn drop_result_page() -> String {
    let mut b = span("TERM_VAL_TBL_DESCR", "Winter 2099");
    b += &link("P_CLASS_NAME$0", "Lecture - Class 9101 - Section 1");
    b += &span("DERIVED_REGFRM1_SSR_STATUS_LONG$0", "Dropped");
    full_page("SSR_STUDENT_FL.SSR_DROP_CLASS_FL", &b)
}

pub fn swap_select_page() -> String {
    swap_select_page_staged(None)
}
/// The select page; once a new combination was chosen for the course, it shows next to the current one (as `LINK2`).
pub fn swap_select_page_staged(new_text: Option<&str>) -> String {
    let mut b = String::new();
    b += &span("CQ_SS_SWAP_DVW_TERM_DESCR30$0", "Winter 2099");
    b += &span(
        "CQ_SS_SWAP_DVW_DESCR100$0",
        "TEST 1001 Introduction to Testing",
    );
    b += &link("LINK1$0", "Lecture - Class 9001 - Section 1 Laboratory - Class 9002 - Section 2");
    if let Some(t) = new_text {
        b += &link("LINK2$0", t);
    }
    b += "<a id='CQ_SS_SWAP_WRK_SELECT_PB$0' href='#'>Select this class</a>\n";
    full_page("CNAQ_MENU_FL.CQ_SS_SWAP_FL", &b)
}
pub fn swap_review_page() -> String {
    swap_review_page_for("9003")
}
/// The swap review for a swap into the option whose lecture is `new_nbr` (the lab is the next number; the old pair is 9001, 9002).
pub fn swap_review_page_for(new_nbr: &str) -> String {
    let lab: u32 = new_nbr.parse::<u32>().map(|n| n + 1).unwrap_or(0);
    let mut b = String::new();
    b += &span("CQ_SS_SWAP_TBL_TERM_DESCR30$0", "Winter 2099");
    b += &span("CQ_SS_SWAP_TBL_DESCR100$0", "TEST 1001 Introduction to Testing");
    b += &span("CQ_SS_SWAP_TBL_DESCR254$0", "Lecture - Class 9001 - Section 1 Laboratory - Class 9002 - Section 2");
    b += &span("CQ_SS_SWAP_TBL_DESCR254A$0", &format!("Lecture - Class {new_nbr} - Section 3 Laboratory - Class {lab} - Section 4"));
    full_page("CNAQ_MENU_FL.CQ_SS_SWAP_FL", &b)
}
pub fn swap_result_page(ok: bool) -> String {
    let mut b = String::new();
    b += &span("CQ_SS_SWAP_TBL_DESCR100$0", "TEST 1001 Introduction to Testing");
    b += &span("CQ_SS_SWAP_TBL_CLASS_NBR$0", "9001");
    b += &span("CQ_SS_SWAP_TBL_CLASS_NBR_CHG_TO$0", "9003");
    b += &span(
        "CQ_SS_SWAP_TBL_DESCRLONG$0",
        if ok {
            "Swap completed"
        } else {
            "Enrollment not successful"
        },
    );
    full_page("CNAQ_MENU_FL.CQ_SS_SWAP_CLASS", &b)
}

pub fn login_page(error: bool) -> String {
    // Same shape as the real page: double quotes, multi-line tags, a relative action, two copies of each field.
    format!(
        "<html><body><form id=\"login\" name=\"login\" method=\"post\" action=\"?&cmd=login&languageCd=ENG\" autocomplete=\"off\">\n\
         <input type=\"hidden\" name=\"usertype\" id=\"usertype\" value=\"\">\n\
         <input type=\"hidden\" id=\"lcsrf_token\" name=\"lcsrf_token\" value=\"MOCK-CSRF\">\n\
         <input type=\"hidden\" name=\"timezoneOffset\" value=\"-180\">\n\
         <input type=\"text\" id=\"userid\" name=\"userid\"\n      value=\"\" title=\"User ID\">\n\
         <input type=\"password\" id=\"pwd\" name=\"pwd\" title=\"Password\">\n{}\
         </form></body></html>",
        if error { "<span>Your User ID and/or Password are invalid.</span>" } else { "" }
    )
}

/// One recorded request: method, path, and the `ICAction` of a POST. Never bodies or cookies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    pub method: &'static str,
    pub path: String,
    pub action: String,
}

#[derive(Clone, Default)]
pub struct Scenario {
    pub session_expired: bool,
    pub wrong_password: bool,
    pub login_307: bool,
    pub submit_fails: bool,
    pub swap_fails: bool,
    pub foreign_form_action: bool,
    pub redirect_to_foreign_host: bool,
    /// The guide's start page lists no grouplets, so no live drop-guide ids can be read.
    pub no_drop_guide: bool,
    /// Answer the drop select request with a page that is not the enrolled list (it carries the term's name but no list).
    pub wrong_select_page: bool,
    /// The swap review shows a different new class than the one that was staged.
    pub swap_review_other: bool,
    /// Rows of the Plan & Enrol list that already have a class chosen when the session starts (left over from an earlier run).
    pub preselected_rows: Vec<u32>,
    pub drop_prompt_error: bool,
}

pub struct MockPs {
    pub sc: Scenario,
    pub log: Vec<Seen>,
    signed_in: bool,
    drop_selected: bool,
    guide_preloaded: bool,
    drop_done: bool,
    staged: bool,
    /// Rows of the plan list the student picked a class for, and the row whose course page was opened last.
    chosen_rows: std::collections::BTreeSet<u32>,
    last_row: u32,
    /// The option row staged last (option 0 is 9001 + 9002, option 1 is 9003 + 9004).
    last_option_row: u32,
    /// A swap is in progress (a candidate was selected), and whether a new combination was staged for it.
    swap_mode: bool,
    swap_staged: bool,
}

impl MockPs {
    pub fn new(sc: Scenario) -> Self {
        let chosen_rows = sc.preselected_rows.iter().copied().collect();
        Self {
            sc,
            log: vec![],
            signed_in: false,
            drop_selected: false,
            guide_preloaded: false,
            drop_done: false,
            staged: false,
            chosen_rows,
            last_row: 0,
            last_option_row: 0,
            swap_mode: false,
            swap_staged: false,
        }
    }
    /// A mock whose user is already signed in.
    pub fn signed_in(sc: Scenario) -> Self {
        let mut m = Self::new(sc);
        m.signed_in = true;
        m
    }
    pub fn write_actions_seen(&self) -> Vec<String> {
        self.log
            .iter()
            .filter(|s| s.method == "POST" && crate::session::is_write_action(&s.action))
            .map(|s| s.action.clone())
            .collect()
    }
    pub fn count(&self, action: &str) -> usize {
        self.log.iter().filter(|s| s.action == action).count()
    }
}

fn resp(status: u16, body: String) -> HttpResponse {
    HttpResponse {
        status,
        location: None,
        content_type: Some("text/html".into()),
        body,
    }
}
fn redirect(status: u16, to: &str) -> HttpResponse {
    HttpResponse {
        status,
        location: Some(to.into()),
        content_type: None,
        body: String::new(),
    }
}

impl Http for MockPs {
    fn send(&mut self, req: HttpRequest) -> Result<HttpResponse, AuthFailure> {
        let url: &Url = &req.url;
        assert_eq!(url.scheme(), "https");
        let path = url.path().to_string();
        let post_body: Vec<(String, String)> = req
            .body
            .as_ref()
            .map(|b| {
                url::form_urlencoded::parse(b.as_bytes())
                    .into_owned()
                    .collect()
            })
            .unwrap_or_default();
        let get = |k: &str| {
            post_body
                .iter()
                .find(|(a, _)| a == k)
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        };
        // like the real site: no User-Agent -> its browser check fails with a 200 error page
        if !req
            .headers
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case("user-agent"))
        {
            return Ok(resp(
                200,
                "Failed processing Browscap file. as it could be missing.\r\n".into(),
            ));
        }
        let action = get("ICAction");
        let method = if req.method == Method::Get {
            "GET"
        } else {
            "POST"
        };
        self.log.push(Seen {
            method,
            path: path.clone(),
            action: action.clone(),
        });

        // sign-in
        if path == "/psp/csqaprd9/" {
            if req.method == Method::Get {
                if !url.query().unwrap_or("").ends_with('&') {
                    let to = format!("{}?{}&", path, url.query().unwrap_or(""));
                    return Ok(redirect(302, &to));
                }
                return Ok(resp(200, login_page(false)));
            }
            if self.sc.login_307 {
                return Ok(redirect(307, "/psc/csqaprd9/EMPLOYEE/SA/s/WEBLIB_PTBR.ISCRIPT1.FieldFormula.IScript_StartPage"));
            }
            if self.sc.wrong_password
                || get("userid") != FAKE_USER
                || get("pwd") != FAKE_PASSWORD
                || get("lcsrf_token") != "MOCK-CSRF"
            {
                return Ok(resp(200, login_page(true)));
            }
            self.signed_in = true;
            return Ok(redirect(
                302,
                "/psc/csqaprd9/EMPLOYEE/SA/s/WEBLIB_PTBR.ISCRIPT1.FieldFormula.IScript_StartPage",
            ));
        }
        if !self.signed_in || self.sc.session_expired {
            return Ok(resp(200, login_page(false)));
        }
        if path.ends_with("IScript_StartPage") {
            return Ok(redirect(
                302,
                "/psc/csqaprd9/EMPLOYEE/SA/c/NUI_FRAMEWORK.PT_LANDINGPAGE.GBL",
            ));
        }
        let page = |component: &str, html: String| -> HttpResponse {
            let _ = component;
            resp(200, html)
        };
        let tweak = |html: String, sc: &Scenario| -> String {
            if sc.foreign_form_action {
                html.replace("https://campus.udst.edu.qa/psc", "https://evil.example/psc")
            } else {
                html
            }
        };
        if path.contains("NUI_FRAMEWORK.PT_LANDINGPAGE") {
            return Ok(page(
                "landing",
                full_page(
                    "NUI_FRAMEWORK.PT_LANDINGPAGE",
                    &span("TILE1", "Invented Tile"),
                ),
            ));
        }
        if path.contains("IScript_Notification_Count") {
            return Ok(resp(200, "3".into()));
        }
        if self.sc.redirect_to_foreign_host
            && path.contains("SSR_CRSE_INFO_FL")
            && req.method == Method::Get
        {
            return Ok(redirect(302, "https://evil.example/steal"));
        }
        let c = |name: &str| path.contains(name);
        let out = if c("SSR_CRSE_INFO_FL") {
            if req.method == Method::Get {
                resp(200, tweak(course_info_page(), &self.sc))
            } else if action.starts_with("SSR_CLSRCH_F_WK_SSR_OPTION_DESCR") {
                self.staged = true;
                self.last_option_row = action.rsplit('$').next().and_then(|n| n.parse().ok()).unwrap_or(0);
                if self.swap_mode {
                    self.swap_staged = true;
                    resp(200, xml("", &redirect_script("https://campus.udst.edu.qa/psp/csqaprd9_11/EMPLOYEE/PSFT_CS/c/NUI_FRAMEWORK.PT_AGSTARTPAGE_NUI.GBL?CONTEXTIDPARAMS=TEMPLATE_ID:CQ_SS_SWAP&CONTEXTIDPARAMS=INSTANCE_ID:CQ_SS_SWAP247106")))
                } else {
                    self.chosen_rows.insert(self.last_row);
                    resp(200, xml("", &redirect_script("https://campus.udst.edu.qa/psc/csqaprd9_11/EMPLOYEE/SA/c/SSR_STUDENT_FL.SSR_START_PAGE_FL.GBL")))
                }
            } else {
                resp(
                    200,
                    xml(
                        &hidden(2),
                        "var oDoc=window.document; oDoc.win9.ICStateNum.value=2;",
                    ),
                )
            }
        } else if c("SSR_START_PAGE_FL") {
            resp(
                200,
                full_page(
                    "SSR_STUDENT_FL.SSR_START_PAGE_FL",
                    &span("MENU1", "Manage Classes"),
                ),
            )
        } else if c("VC_PE_MENU.VC_PE_SS") {
            if req.method == Method::Get {
                // like the real site: the list (with the submit button) once a choice is pending, else the overview
                resp(
                    200,
                    if self.staged {
                        plan_list_page_selected(true, &self.chosen_rows)
                    } else {
                        plan_overview_page()
                    },
                )
            } else if action == "VC_PE_SS_WRK_VC_SUBMIT_ENROL_PB" {
                resp(200, xml(&enrol_conf_html(!self.sc.submit_fails), ""))
            } else if action == "VC_PE_SS_WRK_VC_ENROL_PB" {
                resp(200, xml(&plan_list_page_selected(true, &self.chosen_rows), ""))
            } else if action.starts_with("VC_PE_SS_WRK_VC_SELECT_CLASS_PB") {
                self.last_row = action.rsplit('$').next().and_then(|n| n.parse().ok()).unwrap_or(0);
                resp(200, xml("", &redirect_script("https://campus.udst.edu.qa/psc/csqaprd9_9/EMPLOYEE/SA/c/SSR_STUDENT_FL.SSR_CRSE_INFO_FL.GBL?Page=SSR_CRSE_INFO_FL&Action=U&ACAD_CAREER=CRED&CRSE_ID=000001&CRSE_OFFER_NBR=1&INSTITUTION=TESTI&STRM=9999")))
            } else {
                resp(200, xml(&hidden(2), ""))
            }
        } else if c("PT_AGSTARTPAGE_NUI") {
            // The guide's start page lists the grouplet URLs, as the real one does inside a script array.
            let body = if url.query().unwrap_or("").contains("CQ_SS_SWAP") {
                span(
                    "GROUPLETS",
                    &format!("x=[['p','5','{HOST}/psc/csqaprd9_newwin/EMPLOYEE/SA/s/WEBLIB_PTAI.ISCRIPT3.FieldFormula.IScript_FluidAGPreLoad?TEMPLATE_ID=CQ_SS_SWAP&INSTANCE_ID=CQ_SS_SWAP247106&ICLoc=1','0'],['h','5','{HOST}/psc/csqaprd9_newwin/EMPLOYEE/SA/c/PTGP_MENU.PTGP_GROUPLETS_FL.GBL?PTGP_TYPE=ActivityGuideDataSource&PAGE=PTGP_GPLT_HV_FL&AGComp=H&TEMPLATE_ID=CQ_SS_SWAP&INSTANCE_ID=CQ_SS_SWAP247106&ICLoc=0','0'],['n','5','{HOST}/psc/csqaprd9_newwin/EMPLOYEE/SA/c/PTGP_MENU.PTGP_GROUPLETS_FL.GBL?PTGP_TYPE=ActivityGuideDataSource&PAGE=PTGP_GPLT_NV_FL&AGComp=N&TEMPLATE_ID=CQ_SS_SWAP&INSTANCE_ID=CQ_SS_SWAP247106&ICLoc=1','0']]"),
                )
            } else if self.sc.no_drop_guide {
                String::new()
            } else {
                span(
                    "GROUPLETS",
                    &format!("x=[['p','5','{HOST}/psc/csqaprd9_newwin/EMPLOYEE/SA/s/WEBLIB_PTAI.ISCRIPT3.FieldFormula.IScript_FluidAGPreLoad?TEMPLATE_ID=SR_DROP_FL&OPRID=1&ICLoc=1','0'],['h','5','{HOST}/psc/csqaprd9_newwin/EMPLOYEE/SA/c/PTGP_MENU.PTGP_GROUPLETS_FL.GBL?PTGP_TYPE=ActivityGuideDataSource&PAGE=PTGP_GPLT_HV_FL&AGComp=H&TEMPLATE_ID=SR_DROP_FL&OPRID=1&ICLoc=0','0'],['n','5','{HOST}/psc/csqaprd9_newwin/EMPLOYEE/SA/c/PTGP_MENU.PTGP_GROUPLETS_FL.GBL?PTGP_TYPE=ActivityGuideDataSource&PAGE=PTGP_GPLT_NV_FL&AGComp=N&TEMPLATE_ID=SR_DROP_FL&OPRID=1&ICLoc=1','0']]"),
                )
            };
            resp(200, full_page("NUI_FRAMEWORK.PT_AGSTARTPAGE_NUI", &body))
        } else if c("IScript_FluidAGPreLoad") {
            self.guide_preloaded = true;
            resp(200, "<script>ptaiObj.InstanceExists=true;</script>".into())
        } else if c("PTGP_GROUPLETS_FL") {
            if url.query().unwrap_or("").contains("PTGP_GPLT_NV_FL") && !self.guide_preloaded {
                // Like the real site: no instance yet, so no ids.
                resp(200, "<script>ptgpPage.showMessage('Message', 'A new Activity Guide Instance cannot be created from the context data.', '');</script>".into())
            } else if url.query().unwrap_or("").contains("PTGP_GPLT_NV_FL") {
                resp(
                    200,
                    format!("<div><a href=\"{HOST}/psc/csqaprd9_newwin/EMPLOYEE/SA/c/SSR_STUDENT_FL.SSR_DROP_SELECT_FL.GBL?PAGE=SSR_DROP_SELECT_FL&amp;PTAI=T&amp;PTAI_LIST_ID={LIVE_GUIDE}&amp;PTAI_ITEM_ID=SR_DROP_FL1_{LIVE_GUIDE}\">1</a><a href=\"{HOST}/psc/csqaprd9_newwin/EMPLOYEE/SA/c/SSR_STUDENT_FL.SSR_DROP_REVIEW_FL.GBL?PAGE=SSR_DROP_REVIEW_FL&amp;PTAI=T&amp;PTAI_LIST_ID={LIVE_GUIDE}&amp;PTAI_ITEM_ID=SR_DROP_FL2_{LIVE_GUIDE}\">2</a></div>"),
                )
            } else {
                resp(200, "<div>header</div>".into())
            }
        } else if c("SSR_DROP_CLASS_FL") {
            if req.method == Method::Get {
                if url.query().unwrap_or("").contains("CONFRM") {
                    resp(200, drop_result_page())
                } else {
                    resp(200, drop_terms_page())
                }
            } else if action.starts_with("TERM_LINK") {
                resp(200, xml("", &redirect_script("https://campus.udst.edu.qa/psc/csqaprd9_18/EMPLOYEE/SA/c/NUI_FRAMEWORK.PT_AGSTARTPAGE_NUI.GBL?Page=PT_AGSTARTPAGE_NUI&Action=U")))
            } else {
                resp(200, xml(&hidden(2), ""))
            }
        } else if c("SSR_DROP_SELECT_FL") {
            if self.sc.wrong_select_page && req.method == Method::Get {
                resp(200, drop_result_page())
            } else {
                resp(200, drop_select_page())
            }
        } else if c("SSR_DROP_REVIEW_FL") {
            if req.method == Method::Get {
                // Like the real site: a review opened with another session's guide id is answered, but shows nothing selected.
                if url.query().unwrap_or("").contains(&format!("PTAI_LIST_ID={LIVE_GUIDE}")) {
                    self.drop_selected = true;
                    resp(200, drop_review_page())
                } else {
                    resp(200, full_page("SSR_STUDENT_FL.SSR_DROP_REVIEW_FL", &span("TERM_VAL_TBL_DESCR", "Winter 2099")))
                }
            } else if action == "DERIVED_SSR_FL_SSR_DROPPED" {
                resp(
                    200,
                    xml(
                        &(hidden(3)
                            + &if self.sc.drop_prompt_error {
                                span("MSG1", "Unable to drop: error")
                            } else {
                                String::new()
                            }),
                        "",
                    ),
                )
            } else {
                self.drop_done = true;
                resp(200, xml("", &redirect_script("https://campus.udst.edu.qa/psc/csqaprd9_36/EMPLOYEE/SA/c/SSR_STUDENT_FL.SSR_START_PAGE_FL.GBL")))
            }
        } else if c("CQ_SS_SWAP_CLASS") {
            if url.query().unwrap_or("").contains("pslnkid") {
                // The swap start page: the browser is sent on to the guide's start page.
                self.swap_mode = false;
                self.swap_staged = false;
                resp(200, xml("", &redirect_script("https://campus.udst.edu.qa/psp/csqaprd9_9/EMPLOYEE/SA/c/NUI_FRAMEWORK.PT_AGSTARTPAGE_NUI.GBL?CONTEXTIDPARAMS=TEMPLATE_ID:CQ_SS_SWAP&CONTEXTIDPARAMS=INSTANCE_ID:CQ_SS_SWAP247106")))
            } else {
                resp(200, swap_result_page(!self.sc.swap_fails))
            }
        } else if c("CQ_SS_SWAP_FL") {
            let new_lec = if self.sc.swap_review_other { "9777" } else if self.last_option_row == 0 { "9001" } else { "9003" };
            if req.method == Method::Get {
                if url.query().unwrap_or("").contains("CQ_SS_SWAP_REV_FL") {
                    resp(200, swap_review_page_for(new_lec))
                } else if self.swap_staged {
                    // The select page shows what was really staged, whatever the review later claims.
                    let staged = if self.last_option_row == 0 { "9001" } else { "9003" };
                    let lab = staged.parse::<u32>().map(|n| n + 1).unwrap_or(0);
                    resp(200, swap_select_page_staged(Some(&format!("Lecture - Class {staged} - Section 3 Laboratory - Class {lab} - Section 4"))))
                } else {
                    resp(200, swap_select_page())
                }
            } else if action.starts_with("CQ_SS_SWAP_WRK_SELECT_PB") {
                self.swap_mode = true;
                resp(200, xml("", &redirect_script("https://campus.udst.edu.qa/psc/csqaprd9_13/EMPLOYEE/PSFT_CS/c/SSR_STUDENT_FL.SSR_CRSE_INFO_FL.GBL?Page=SSR_CRSE_INFO_FL&Action=U&ACAD_CAREER=CRED&CRSE_ID=000001&CRSE_OFFER_NBR=1&INSTITUTION=TESTI&STRM=9999")))
            } else {
                resp(200, xml("", &redirect_script("https://campus.udst.edu.qa/psp/csqaprd9_13/EMPLOYEE/PSFT_CS/c/NUI_FRAMEWORK.PT_AGSTARTPAGE_NUI.GBL?CONTEXTIDPARAMS=TEMPLATE_ID:PTPPNAVCOL")))
            }
        } else {
            resp(404, String::new())
        };
        Ok(out)
    }
}
