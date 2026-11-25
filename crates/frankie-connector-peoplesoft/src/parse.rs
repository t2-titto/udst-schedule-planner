//! Page parsers: pure functions from HTML text to typed values. Tested with invented fixtures only.
use crate::model::*;
use crate::pia::{field_text, fields, outcome_hints, rows, visible_text, Field};
use regex::Regex;
use std::sync::OnceLock;

fn re_class() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?i)^(.*?)\s*-\s*Class\s+(\d+)\s*-\s*Section\s+(\S+)").unwrap())
}
fn re_seats() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?i)open seats\s+(\d+)\s+of\s+(\d+)").unwrap())
}
fn re_meet() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"(?i)(Sunday|Monday|Tuesday|Wednesday|Thursday|Friday|Saturday)\s+(\d{1,2}:\d{2}\s?[AP]M)\s+to\s+(\d{1,2}:\d{2}\s?[AP]M)").unwrap()
    })
}
fn re_class_in_text() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?i)Class\s+(\d+)").unwrap())
}

/// `A B A B` -> `A B`: PeopleSoft repeats a value once per meeting pattern.
pub fn collapse_repeat(s: &str) -> String {
    let t: Vec<&str> = s.split_whitespace().collect();
    if t.len() >= 2 && t.len().is_multiple_of(2) {
        let (a, b) = t.split_at(t.len() / 2);
        if a == b {
            return a.join(" ");
        }
    }
    t.join(" ")
}

pub fn parse_meetings(s: &str) -> Vec<Meeting> {
    re_meet()
        .captures_iter(s)
        .map(|c| Meeting {
            day: c[1].to_string(),
            start: c[2].replace(' ', ""),
            end: c[3].replace(' ', ""),
        })
        .collect()
}

pub fn parse_seats(s: &str) -> Option<Seats> {
    let c = re_seats().captures(s)?;
    Some(Seats {
        open: c[1].parse().ok()?,
        total: c[2].parse().ok()?,
    })
}

/// Course information page (`SSR_CRSE_INFO_FL`): header facts and every class option with its components.
pub fn parse_course_sections(html: &str) -> CourseSections {
    let fs = fields(html);
    let t = |n: &str| field_text(&fs, n).unwrap_or("").to_string();
    let option_rows = rows(
        &fs,
        &[
            "SSR_CLSRCH_F_WK_SSR_OPTION_DESCR",
            "SSR_DER_CS_GRP_",
            "SSR_CLSRCH_F_WK_SSR_CMPNT_DESCR_",
            "SSR_CLSRCH_F_WK_SSR_MTG_DT_LONG_",
            "SSR_CLSRCH_F_WK_SSR_MTG_SCHED_L_",
            "SSR_CLSRCH_F_WK_SSR_MTG_LOC_LONG_",
            "SSR_CLSRCH_F_WK_SSR_INSTR_LONG_",
            "SSR_CLSRCH_F_WK_SSR_DESCR50_",
        ],
    );
    let mut options = Vec::new();
    for (row, m) in option_rows {
        let get = |p: &str| m.get(p).cloned().unwrap_or_default();
        let mut components = Vec::new();
        for n in 1..=4u8 {
            let desc = get(&format!("SSR_CLSRCH_F_WK_SSR_CMPNT_DESCR_{n}"));
            let Some(c) = re_class().captures(&desc) else {
                continue;
            };
            let sched = get(&format!("SSR_CLSRCH_F_WK_SSR_MTG_SCHED_L_{n}"));
            let seats_text = get(&format!("SSR_CLSRCH_F_WK_SSR_DESCR50_{n}"));
            components.push(ClassComponent {
                kind: c[1].trim().to_string(),
                class_nbr: c[2].to_string(),
                section: c[3].to_string(),
                dates: get(&format!("SSR_CLSRCH_F_WK_SSR_MTG_DT_LONG_{n}")),
                meetings: parse_meetings(&sched),
                schedule_text: sched,
                room: collapse_repeat(&get(&format!("SSR_CLSRCH_F_WK_SSR_MTG_LOC_LONG_{n}"))),
                instructor: collapse_repeat(&get(&format!("SSR_CLSRCH_F_WK_SSR_INSTR_LONG_{n}"))),
                seats: parse_seats(&seats_text),
                seats_text,
            });
        }
        options.push(ClassOption {
            row,
            option_no: get("SSR_CLSRCH_F_WK_SSR_OPTION_DESCR"),
            status: get("SSR_DER_CS_GRP_SSR_OPTION_STAT"),
            session: get("SSR_DER_CS_GRP_SESSION_CODE"),
            components,
        });
    }
    CourseSections {
        term: t("TERM_VAL_TBL_DESCR"),
        course: t("SSR_CRSE_INFO_V_SSS_SUBJ_CATLG"),
        title: t("SSR_CRSE_INFO_V_COURSE_TITLE_LONG"),
        description: t("SSR_CRSE_INFO_V_DESCRLONG"),
        units: t("SSR_CLSRCH_F_WK_UNITS_RANGE"),
        grading: t("SSR_CLSRCH_F_WK_SSR_GRAD_BASIS_LNG"),
        components_offered: t("SSR_CLSRCH_F_WK_SSR_COMPONENT_LONG"),
        career: t("SSR_CLSRCH_F_WK_SSR_ACADCAR_LONG"),
        options_text: t("SSR_CLSRCH_F_WK_SSR_MSG_TEXT"),
        options,
    }
}

fn first(m: &std::collections::BTreeMap<String, String>, k: &str) -> String {
    m.get(k).cloned().unwrap_or_default()
}

/// Plan & Enrol home (`VC_PE_SS`): requirement groups in document order, each with its course rows.
pub fn parse_plan_overview(html: &str) -> PlanOverview {
    let fs = fields(html);
    let mut reqs: Vec<Requirement> = Vec::new();
    let mut pending: std::collections::BTreeMap<u32, PlanCourse> = Default::default();
    for f in &fs {
        match f.name.as_str() {
            "VC_PE_HDR_DVW_DESCR100" => reqs.push(Requirement {
                name: f.text.clone(),
                detail: String::new(),
                status: String::new(),
                courses: vec![],
            }),
            "VC_PE_HDR_DVW_DESCR254" => {
                if let Some(r) = reqs.last_mut() {
                    r.detail = f.text.clone();
                }
            }
            "VC_PE_SS_WRK_DESCR35" => {
                if let Some(r) = reqs.last_mut() {
                    r.status = f.text.clone();
                }
            }
            "VC_PE_SS_L3_VW_DESCR50" => {
                if let Some(row) = f.row {
                    pending.insert(
                        row,
                        PlanCourse {
                            row,
                            title: f.text.clone(),
                            category: String::new(),
                            units: String::new(),
                            code: String::new(),
                        },
                    );
                    if reqs.is_empty() {
                        reqs.push(Requirement {
                            name: String::new(),
                            detail: String::new(),
                            status: String::new(),
                            courses: vec![],
                        });
                    }
                    // attach in document order; fields below fill the details by row
                    let idx = reqs.len() - 1;
                    reqs[idx].courses.push(pending[&row].clone());
                }
            }
            "SSR_ENRL_CAT" | "UNITS_MINIMUM" | "DESCR10" => {
                if let Some(row) = f.row {
                    for r in reqs.iter_mut() {
                        if let Some(c) = r.courses.iter_mut().find(|c| c.row == row) {
                            match f.name.as_str() {
                                "SSR_ENRL_CAT" => c.category = f.text.clone(),
                                "UNITS_MINIMUM" => c.units = f.text.clone(),
                                _ => c.code = f.text.clone(),
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    PlanOverview { requirements: reqs }
}

/// Plan & Enrol class list (`VC_PE_SS_ENROL`).
pub fn parse_plan_class_list(html: &str) -> PlanClassList {
    let fs = fields(html);
    let m = rows(
        &fs,
        &[
            "VC_PE_ENROL_VW_DESCR",
            "VC_PE_SS_WRK_CLASS_DESCR",
            "SSR_MUL_MTG_LNK",
        ],
    );
    let rows_v = m
        .into_iter()
        .map(|(row, r)| PlanClassRow {
            row,
            course_label: first(&r, "VC_PE_ENROL_VW_DESCR50"),
            term: first(&r, "VC_PE_ENROL_VW_DESCR"),
            status: first(&r, "VC_PE_SS_WRK_CLASS_DESCR"),
            schedule_summary: first(&r, "SSR_MUL_MTG_LNK"),
        })
        .filter(|r| !r.course_label.is_empty() || !r.term.is_empty())
        .collect();
    PlanClassList {
        term_label: field_text(&fs, "VC_PE_SS_WRK_STRM")
            .unwrap_or("")
            .to_string(),
        rows: rows_v,
        can_submit: html.contains("VC_PE_SS_WRK_VC_SUBMIT_ENROL_PB"),
    }
}

fn dedup(v: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for x in v {
        if !out.contains(&x) {
            out.push(x);
        }
    }
    out
}

/// Drop-class pages (`SSR_DROP_SELECT_FL`, `SSR_DROP_REVIEW_FL`): the student's enrolled classes.
pub fn parse_enrolled(html: &str) -> EnrolledList {
    let fs = fields(html);
    let m = rows(
        &fs,
        &[
            "P_CLASS_NAME",
            "DERIVED_SSR_FL_SSR_DESCR80",
            "SSR_MTG_PAT_WRK_",
            "DERIVED_SSR_FL_SSR_UNITS_LBL",
            "DERIVED_REGFRM1_SSR_STATUS_LONG",
        ],
    );
    let mut classes: Vec<EnrolledClass> = m
        .into_iter()
        .map(|(row, r)| {
            let name = first(&r, "P_CLASS_NAME");
            let (component, class_nbr, section) = match re_class().captures(&name) {
                Some(c) => (c[1].trim().to_string(), c[2].to_string(), c[3].to_string()),
                None => (
                    name.clone(),
                    re_class_in_text()
                        .captures(&name)
                        .map(|c| c[1].to_string())
                        .unwrap_or_default(),
                    String::new(),
                ),
            };
            let nonempty = |a: String, b: String| {
                [a, b]
                    .into_iter()
                    .filter(|x| !x.is_empty())
                    .collect::<Vec<_>>()
            };
            EnrolledClass {
                row,
                class_nbr,
                component,
                section,
                course: first(&r, "DERIVED_SSR_FL_SSR_DESCR80"),
                schedule: nonempty(
                    first(&r, "SSR_MTG_PAT_WRK_SSR_MTG_SCHED1"),
                    first(&r, "SSR_MTG_PAT_WRK_SSR_MTG_SCHED2"),
                ),
                rooms: dedup(nonempty(
                    first(&r, "SSR_MTG_PAT_WRK_SSR_ROOM1"),
                    first(&r, "SSR_MTG_PAT_WRK_SSR_ROOM2"),
                )),
                instructors: dedup(nonempty(
                    first(&r, "SSR_MTG_PAT_WRK_SSR_INSTRUCTOR1"),
                    first(&r, "SSR_MTG_PAT_WRK_SSR_INSTRUCTOR2"),
                )),
                units: first(&r, "DERIVED_SSR_FL_SSR_UNITS_LBL"),
                status: first(&r, "DERIVED_REGFRM1_SSR_STATUS_LONG"),
            }
        })
        .filter(|c| !c.class_nbr.is_empty() || !c.course.is_empty())
        .collect();
    // A course's lab or second component carries no course name or status of its own on the page: inherit the
    // course and status of the row above it.
    let (mut course, mut status) = (String::new(), String::new());
    for c in classes.iter_mut() {
        if c.course.is_empty() {
            c.course = course.clone();
        } else {
            course = c.course.clone();
        }
        if c.status.is_empty() {
            c.status = status.clone();
        } else {
            status = c.status.clone();
        }
    }
    EnrolledList {
        term: field_text(&fs, "TERM_VAL_TBL_DESCR")
            .unwrap_or("")
            .to_string(),
        classes,
    }
}

/// True only for the enrolled-list page of the term that was asked for. The page that follows a drop, and any error or
/// login page, carry the term's name too but not the list's own markers, and parse as an empty list.
pub fn is_drop_select_page(html: &str, list: &EnrolledList, expected_term: &str) -> bool {
    let norm = |t: &str| t.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    if list.term.is_empty() || norm(&list.term) != norm(expected_term) {
        return false;
    }
    // The select page offers a checkbox per class and carries the guide's ids; the page after a drop does neither.
    html.contains("P_SELECT") || html.contains("PTAI_LIST_ID")
}

/// Drop-class term chooser (`SSR_DROP_CLASS_FL`): `TERM_LINK$n` rows.
pub fn parse_terms(html: &str) -> Vec<(u32, String)> {
    let fs = fields(html);
    fs.iter()
        .filter(|f| f.name == "TERM_LINK" && f.row.is_some() && !f.text.is_empty())
        .map(|f| (f.row.unwrap_or(0), f.text.clone()))
        .collect()
}

/// Swap, first step (`CQ_SS_SWAP_FL`): enrolled classes that may be swapped.
pub fn parse_swap_candidates(html: &str) -> Vec<SwapCandidate> {
    let fs = fields(html);
    rows(
        &fs,
        &[
            "CQ_SS_SWAP_DVW_",
            "LINK1",
            "LINK2",
            "CQ_SS_SWAP_WRK_SELECT_PB",
        ],
    )
    .into_iter()
    .map(|(row, r)| SwapCandidate {
        row,
        term: first(&r, "CQ_SS_SWAP_DVW_TERM_DESCR30"),
        course: first(&r, "CQ_SS_SWAP_DVW_DESCR100"),
        class_text: first(&r, "LINK1"),
        other_text: first(&r, "LINK2"),
    })
    .filter(|c| !c.course.is_empty())
    .collect()
}

/// Swap review (`CQ_SS_SWAP_REV_FL`), or the same page after a failed attempt.
pub fn parse_swap_review(html: &str) -> SwapReview {
    let fs = fields(html);
    let r = |n: &str| {
        field_text(&fs, n).map(str::to_string).or_else(|| {
            fs.iter()
                .find(|f| f.name == n && f.row == Some(0))
                .map(|f| f.text.clone())
        })
    };
    let mut messages: Vec<String> = fs
        .iter()
        .filter(|f| f.name == "CQ_SS_SWAP_TBL_DESCRLONG" && !f.text.is_empty())
        .map(|f| f.text.clone())
        .collect();
    for h in outcome_hints(&fs) {
        if !messages.contains(&h) {
            messages.push(h);
        }
    }
    SwapReview {
        term: r("CQ_SS_SWAP_TBL_TERM_DESCR30").unwrap_or_default(),
        course: r("CQ_SS_SWAP_TBL_DESCR100").unwrap_or_default(),
        current_class: r("CQ_SS_SWAP_TBL_DESCR254").unwrap_or_default(),
        new_class: r("CQ_SS_SWAP_TBL_DESCR254A").unwrap_or_default(),
        messages,
    }
}

/// Every course of the swap review page, in page order.
pub fn parse_swap_review_rows(html: &str) -> Vec<SwapReviewRow> {
    let fs = fields(html);
    rows(&fs, &["CQ_SS_SWAP_TBL_TERM_DESCR30", "CQ_SS_SWAP_TBL_DESCR100", "CQ_SS_SWAP_TBL_DESCR254", "CQ_SS_SWAP_TBL_DESCR254A"])
        .into_iter()
        .map(|(_, r)| SwapReviewRow {
            term: first(&r, "CQ_SS_SWAP_TBL_TERM_DESCR30"),
            course: first(&r, "CQ_SS_SWAP_TBL_DESCR100"),
            current_text: first(&r, "CQ_SS_SWAP_TBL_DESCR254"),
            new_text: first(&r, "CQ_SS_SWAP_TBL_DESCR254A"),
        })
        .filter(|r| !r.course.is_empty() || !r.current_text.is_empty())
        .collect()
}

/// Every course of the page after a swap, with PeopleSoft's own answer for it ("Enrollment not successful", ...).
pub fn parse_swap_result_rows(html: &str) -> Vec<SwapResultRow> {
    let fs = fields(html);
    rows(&fs, &["CQ_SS_SWAP_TBL_DESCR100", "CQ_SS_SWAP_TBL_CLASS_NBR", "CQ_SS_SWAP_TBL_CLASS_NBR_CHG_TO", "CQ_SS_SWAP_TBL_DESCRLONG"])
        .into_iter()
        .map(|(_, r)| SwapResultRow {
            course: first(&r, "CQ_SS_SWAP_TBL_DESCR100"),
            class_nbr: first(&r, "CQ_SS_SWAP_TBL_CLASS_NBR"),
            changed_to: first(&r, "CQ_SS_SWAP_TBL_CLASS_NBR_CHG_TO"),
            message: first(&r, "CQ_SS_SWAP_TBL_DESCRLONG"),
        })
        .filter(|r| !r.course.is_empty() || !r.class_nbr.is_empty())
        .collect()
}

/// Enrolment confirmation (`VC_PE_SS_ENR_CONF`).
pub fn parse_enrol_confirmation(html: &str) -> EnrolConfirmation {
    let fs = fields(html);
    let icons = {
        let re = Regex::new(r"PS_CS_STATUS_([A-Z]+)_ICN").unwrap();
        re.captures_iter(html)
            .map(|c| c[1].to_ascii_lowercase())
            .collect()
    };
    EnrolConfirmation {
        total_amount: field_text(&fs, "VC_PE_SS_WRK_AMOUNT_TOTAL")
            .unwrap_or("")
            .to_string(),
        payment_detail: field_text(&fs, "VC_PE_SS_WRK_PAYMENT_DTL")
            .unwrap_or("")
            .to_string(),
        lines: fs
            .iter()
            .filter(|f| f.name == "VC_PE_STD_ENRL_DESCR50" && !f.text.is_empty())
            .map(|f| f.text.clone())
            .collect(),
        icons,
        messages: outcome_hints(&fs),
    }
}

/// Notification count iScript: a bare number.
pub fn parse_count(body: &str) -> Option<u32> {
    body.trim().parse().ok()
}

/// Generic dump for pages whose layout is not mapped: field name and text only.
pub fn generic_fields(html: &str, limit: usize) -> Vec<Field> {
    fields(html)
        .into_iter()
        .filter(|f| !f.text.is_empty())
        .take(limit)
        .collect()
}

// ---------------------------------------------------------------- the degree tree

fn split_course_label(label: &str) -> (String, String) {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"^([A-Z]{2,6} ?\d{3,4}[A-Z]?):\s*(.*)$").unwrap());
    match re.captures(label) {
        Some(c) => (c[1].to_string(), c[2].trim().to_string()),
        None => (String::new(), label.to_string()),
    }
}

/// The leading "and" / "or" that joins a row to the one before it, and the label without it.
fn strip_join(raw: &str) -> (String, String) {
    let t = raw.trim();
    for j in ["and", "or"] {
        if let Some(rest) = t.strip_prefix(j) {
            if rest.starts_with(char::is_whitespace) {
                return (j.to_string(), rest.trim().to_string());
            }
        }
    }
    (String::new(), t.to_string())
}

fn select_choice(html: &str, id: &str) -> Option<TermChoice> {
    let re = regex::Regex::new(&format!(r"(?s)<select id='{}'[^>]*>(.*?)</select>", regex::escape(id))).ok()?;
    let body = re.captures(html)?.get(1)?.as_str().to_string();
    let opt = regex::Regex::new(r#"(?s)<option value="([^"]*)"( selected='selected')?[^>]*>(.*?)</option>"#).ok()?;
    let mut selected = String::new();
    let mut options = Vec::new();
    for c in opt.captures_iter(&body) {
        let label = decode_entities(&re_strip(&c[3])).replace('\u{a0}', " ").trim().to_string();
        if c.get(2).is_some() {
            selected = c[1].to_string();
        }
        if !c[1].is_empty() {
            options.push(TermOption { code: c[1].to_string(), label });
        }
    }
    Some(TermChoice { selected, options })
}

fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&").replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">").replace("&#39;", "'").replace("&quot;", "\"")
}

fn re_strip(s: &str) -> String {
    regex::Regex::new(r"<[^>]+>").unwrap().replace_all(s, " ").to_string()
}

/// The degree tree of Plan & Enrol's landing page. Fail-closed: a page that has no Year header is not the tree (an empty answer or a
/// maintenance page must never read as "a degree with nothing in it").
pub fn parse_degree(html: &str) -> Result<Degree, &'static str> {
    if !html.contains("VC_PE_HDR_DVW_DESCR100") {
        return Err("the degree tree page was not recognised");
    }
    let fs = fields(html);
    let mut years: Vec<DegreeYear> = Vec::new();
    let mut semesters: Vec<DegreeSemester> = Vec::new();
    let mut rows: Vec<DegreeRow> = Vec::new();
    let mut current: u32 = 0;
    let mut current_sem: Option<u32> = None;
    let by_row = rows_by_row(&fs);
    for f in &fs {
        match (f.name.as_str(), f.row) {
            // A Year header has no grid level; the Semester headers nested in it carry one (`...DESCR100$62$$N`).
            ("VC_PE_HDR_DVW_DESCR100", Some(r)) if f.level == 0 => {
                current = r;
                current_sem = None;
                years.push(DegreeYear { index: r, name: f.text.clone(), summary: String::new(), state: String::new() });
            }
            ("VC_PE_HDR_DVW_DESCR100", Some(r)) => {
                current_sem = Some(r);
                semesters.push(DegreeSemester { index: r, year: current, name: f.text.clone(), summary: String::new() });
            }
            ("VC_PE_HDR_DVW_DESCR254", Some(r)) if f.level == 0 => {
                if let Some(y) = years.iter_mut().find(|y| y.index == r) {
                    y.summary = f.text.clone();
                }
            }
            ("VC_PE_HDR_DVW_DESCR254", Some(r)) => {
                if let Some(x) = semesters.iter_mut().find(|x| x.index == r) {
                    x.summary = f.text.clone();
                }
            }
            ("VC_PE_SS_WRK_DESCR35", Some(r)) => {
                if let Some(y) = years.iter_mut().find(|y| y.index == r) {
                    y.state = f.text.clone();
                }
            }
            ("VC_PE_SS_L3_VW_DESCR50", Some(r)) => {
                let (join, label) = strip_join(&f.text);
                let (code, title) = split_course_label(&label);
                let d = by_row.get(&r);
                let get = |n: &str| d.and_then(|m| m.get(n)).cloned().unwrap_or_default();
                let choice = select_choice(html, &format!("DESCR10${r}"));
                let units = get("UNITS_MINIMUM");
                let term_text = get("DESCR10");
                rows.push(DegreeRow {
                    row: r,
                    year: current,
                    semester: current_sem,
                    label,
                    joined_by_and: join == "and",
                    code,
                    title,
                    category: get("SSR_ENRL_CAT"),
                    units: if units.is_empty() { None } else { Some(units) },
                    term: if choice.is_some() || term_text.is_empty() { None } else { Some(term_text) },
                    term_choice: choice,
                    has_select: d.is_some_and(|m| m.contains_key("VC_PE_SS_WRK_SELECT_PB")),
                    has_requirement: d.is_some_and(|m| m.contains_key("RQRMNT_NAME_LBL")),
                });
            }
            _ => {}
        }
    }
    if years.is_empty() {
        return Err("the degree tree page was not recognised");
    }
    Ok(Degree { years, semesters, rows })
}

fn rows_by_row(fs: &[Field]) -> std::collections::BTreeMap<u32, std::collections::BTreeMap<String, String>> {
    rows(fs, &["SSR_ENRL_CAT", "UNITS_MINIMUM", "DESCR10", "VC_PE_SS_WRK_SELECT_PB", "RQRMNT_NAME_LBL"])
}

/// A "Select Courses (Course List)" page: the list's name and its choices, each joined to the one before with "or" or "and".
pub fn parse_course_list(html: &str) -> Result<CourseList, &'static str> {
    if !html.contains("VC_PE_SS_CL_VW_DESCR50") {
        return Err("the course list page was not recognised");
    }
    let fs = fields(html);
    let name = field_text(&fs, "VC_PE_SS_WRK_DESCR50").unwrap_or("").to_string();
    let m = rows(&fs, &["VC_PE_SS_CL_VW_DESCR50", "VC_PE_SS_CL_VW_SSR_ENRL_CAT", "VC_PE_SS_CL_VW_UNITS_MINIMUM", "VC_PE_SS_CL_VW_DESCR10"]);
    let mut choices = Vec::new();
    for (row, r) in m {
        let (join, label) = strip_join(r.get("VC_PE_SS_CL_VW_DESCR50").map(String::as_str).unwrap_or(""));
        if label.is_empty() {
            continue;
        }
        let (code, title) = split_course_label(&label);
        let units = r.get("VC_PE_SS_CL_VW_UNITS_MINIMUM").cloned().unwrap_or_default();
        choices.push(ListChoice {
            row,
            joined_by: join,
            code,
            title,
            category: r.get("VC_PE_SS_CL_VW_SSR_ENRL_CAT").cloned().unwrap_or_default(),
            units: if units.is_empty() { None } else { Some(units) },
            term: r.get("VC_PE_SS_CL_VW_DESCR10").cloned().unwrap_or_default(),
        });
    }
    if choices.is_empty() {
        return Err("the course list page has no choices");
    }
    Ok(CourseList { name, choices })
}

/// "Course Requirements for X" and its free text. The page puts a bold heading ("Course Requirements for MATH 1030: Calculus I") and then the
/// requirement itself in one HTML area; the text is only that part, so the title does not run into it.
pub fn parse_requirement_text(html: &str) -> Result<RequirementText, &'static str> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"(?s)<b>\s*Course Requirements for ([^<]*)</b>(.*?)(?:<!--\s*End HTML Area|</div>)").unwrap());
    let Some(c) = re.captures(html) else {
        return Err("the course requirements page was not recognised");
    };
    let heading = decode_entities(c[1].trim());
    let (code, title) = split_course_label(&heading);
    let body = decode_entities(&re_strip(&c[2])).replace('\u{a0}', " ");
    let text = body.split_whitespace().collect::<Vec<_>>().join(" ");
    Ok(RequirementText { course: if code.is_empty() { heading } else { code }, title, text })
}

/// The results page of the Browse Course Catalog search. Each result carries a hidden line "CODE CRSE_ID YYYYMMDD units" and a link with the
/// offer number and the title. A page that is not that results page is an error (an empty result list is a valid, empty answer).
pub fn parse_catalog_results(html: &str) -> Result<Vec<CatalogHit>, &'static str> {
    if !html.contains("SSR_BRWSCAT_ES_FL") || !html.contains("PTS_LIST_TITLE") {
        return Err("the catalog results page was not recognised");
    }
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?s)<p hidden>\s*([A-Z]{2,6} \d{3,4}[A-Z]?)\s+(\d{6})\s+(\d{4})(\d{2})(\d{2})\s+([\d.]+) units\s*</p>.*?CRSE_OFFER_NBR=(\d+).*?>\s*(.*?)</a>").unwrap()
    });
    let mut out = Vec::new();
    for c in re.captures_iter(html) {
        let label = decode_entities(&re_strip(&c[8]));
        let label = label.split_whitespace().collect::<Vec<_>>().join(" ");
        let title = label.split_once(" - ").map(|(_, t)| t.trim().to_string()).unwrap_or_default();
        out.push(CatalogHit {
            code: c[1].to_string(),
            title,
            crse_id: c[2].to_string(),
            offer_nbr: c[7].to_string(),
            effdt: format!("{}-{}-{}", &c[3], &c[4], &c[5]),
            units: c[6].to_string(),
        });
    }
    Ok(out)
}

/// Course detail page from Browse Course Catalog (`SSR_CRSE_DTL_FL`), including `DERIVED_CRSECAT_DESCR254A$0` (Enrollment Requirement).
pub fn parse_catalog_detail(html: &str) -> Result<CatalogDetail, &'static str> {
    if !html.contains("SSR_CRSE_DTL_WK_SSS_SUBJ_CATLG") && !html.contains("DERIVED_CRSECAT_DESCR200") {
        return Err("the catalog detail page was not recognised");
    }
    let fs = fields(html);
    let get = |name: &str| {
        field_text(&fs, name)
            .map(str::to_string)
            .or_else(|| fs.iter().find(|f| f.name == name && f.row == Some(0)).map(|f| f.text.clone()))
            .unwrap_or_default()
    };
    let components = fs
        .iter()
        .filter(|f| f.name == "DERIVED_CRSECAT_DESCR40" && !f.text.is_empty())
        .map(|f| f.text.clone())
        .collect();
    Ok(CatalogDetail {
        code: get("SSR_CRSE_DTL_WK_SSS_SUBJ_CATLG"),
        title: get("DERIVED_CRSECAT_DESCR200"),
        units: get("DERIVED_CRSECAT_UNITS_RANGE"),
        career: get("SSR_CRSE_OFF_VW_ACAD_CAREER"),
        grading: get("SSR_CRSE_OFF_VW_GRADING_BASIS"),
        components,
        campus: get("CAMPUS_TBL_DESCR"),
        academic_group: get("ACAD_GROUP_TBL_DESCR"),
        academic_org: get("ACAD_ORG_TBL_DESCR"),
        requirement: get("DERIVED_CRSECAT_DESCR254A"),
        attribute: get("DERIVED_CRSECAT_SSR_CRSE_ATTR_LONG"),
        description: get("SSR_CRSE_OFF_VW_DESCRLONG"),
    })
}
