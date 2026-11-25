//! High-level operations. Reads are plain methods. Every write is a pair: `preview_*` (sends nothing that changes
//! anything, returns the plan and a one-time token) and `confirm` (needs the token and the exact typed phrase).
use crate::gate::{Gate, GateError, PlannedStep, Preview, WriteKind, WritePlan, WriteProof};
use crate::guided;
use crate::model::*;
use crate::parse;
use crate::pia::{self, Field};
use crate::session::{Page, PsError, Session};
use frankie_auth::Http;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use url::Url;

/// How long a preview token lives.
pub const PREVIEW_TTL: Duration = Duration::from_secs(120);
/// Seat numbers are not refreshed more often than this (no tight polling).
pub const SEAT_REFRESH_MIN: Duration = Duration::from_secs(30);

const STAGE_ACTION_PREFIX: &str = "SSR_CLSRCH_F_WK_SSR_OPTION_DESCR";
const SETFIELD_ACTION: &str = "#ICSetFieldSSR_CRSE_INFO_FL.SSR_CNTRLFIELD.A";
const PANEL_STYLE: &str = " pst_side1-fixed pst_panel-mode ";

/// How a swap names the new option on the swap's own course page.
enum OptionRef {
    Row(u32),
    Nums(Vec<String>),
}

enum Payload {
    Stage {
        page: Page,
        row: u32,
        nums: Vec<String>,
        swap: bool,
        /// For a swap: the candidate row the stage is for.
        swap_row: u32,
    },
    SubmitEnrol {
        page: Page,
    },
    Drop {
        page: Page,
    },
    SwapCommit {
        page: Page,
    },
    /// Set one planned course's term and save the plan. The page is read again at confirm time; only the target is kept here.
    PlanTerm {
        year: u32,
        index: u32,
        code: String,
        from: String,
        to: String,
    },
}

/// What a confirmed write did.
#[derive(Debug)]
pub struct WriteOutcome {
    pub kind: WriteKind,
    pub plan: WritePlan,
    /// Sentences the page itself showed after the write (success or failure text). Page text is data.
    pub messages: Vec<String>,
    pub detail: serde_json::Value,
}

#[derive(Debug, PartialEq)]
pub enum ConfirmError {
    Gate(GateError),
    Ps(PsError),
}

impl From<PsError> for ConfirmError {
    fn from(e: PsError) -> Self {
        ConfirmError::Ps(e)
    }
}

pub struct PeopleSoft<H: Http> {
    pub(crate) sess: Session<H>,
    gate: Gate<Payload>,
    /// Class numbers staged by THIS process for Plan & Enrol. A submit may only name these.
    staged: Vec<String>,
    /// The class numbers of the class a swap preview was made for, until its stage is confirmed.
    swap_pending_old: Option<Vec<String>>,
    /// Swaps staged in this guided session: (class numbers given up, class numbers taken). One commit sends them all.
    swap_pairs: Vec<(Vec<String>, Vec<String>)>,
    /// The swap guide's select page as it stands now (after the last stage), for choosing the next course's new class.
    swap_select: Option<Page>,
    /// The grouplet URLs and the page that listed them, from the last time the swap guide was loaded.
    swap_chain: Option<(Url, Vec<String>)>,
    sections: Option<(Page, CourseKey)>,
    plan_page: Option<Page>,
    seat_cache: HashMap<String, (Instant, CourseSections)>,
    /// The drop guide's ids for this session, read from PeopleSoft before a drop preview (see `guided::DropGuide`).
    drop_guide: Option<guided::DropGuide>,
}

fn key_from_url(u: &Url) -> Option<CourseKey> {
    Some(CourseKey {
        strm: pia::query_value(u, "STRM")?,
        crse_id: pia::query_value(u, "CRSE_ID")?,
        crse_offer_nbr: pia::query_value(u, "CRSE_OFFER_NBR")?,
        acad_career: pia::query_value(u, "ACAD_CAREER")?,
        institution: pia::query_value(u, "INSTITUTION")?,
    })
}

fn safe_token(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 32
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn class_numbers_in(text: &str) -> Vec<String> {
    let re = regex::Regex::new(r"(?i)Class\s+(\d+)").expect("static regex");
    re.captures_iter(text).map(|c| c[1].to_string()).collect()
}

/// The page's own row number of the `index`-th row (0-based) of a Year, if that row passes `ok`.
pub fn nth_row_in_year(deg: &Degree, year: u32, index: u32, ok: impl Fn(&DegreeRow) -> bool) -> Option<u32> {
    deg.rows.iter().filter(|r| r.year == year).nth(index as usize).filter(|r| ok(r)).map(|r| r.row)
}

impl<H: Http> PeopleSoft<H> {
    pub fn new(sess: Session<H>) -> Self {
        Self {
            sess,
            gate: Gate::new(PREVIEW_TTL),
            staged: vec![],
            swap_pending_old: None,
            swap_pairs: vec![],
            swap_select: None,
            swap_chain: None,
            sections: None,
            plan_page: None,
            seat_cache: HashMap::new(),
            drop_guide: None,
        }
    }

    /// How long a preview token lives (default [`PREVIEW_TTL`], 120 s). See [`Gate::set_ttl`].
    pub fn set_preview_ttl(&mut self, ttl: Duration) {
        self.gate.set_ttl(ttl);
    }

    pub fn preview_ttl(&self) -> Duration {
        self.gate.ttl()
    }

    pub fn session_mut(&mut self) -> &mut Session<H> {
        &mut self.sess
    }
    pub fn writes_armed(&self) -> bool {
        self.gate.is_armed()
    }
    /// Turn writes on. Off by default. The CLI asks for a typed phrase before calling this.
    pub fn arm_writes(&mut self) {
        self.gate.arm();
    }
    pub fn disarm_writes(&mut self) {
        self.gate.disarm();
        // Anything staged under an earlier arming is forgotten: a later arming must stage again and read fresh.
        self.staged.clear();
        self.swap_pending_old = None;
        self.swap_pairs.clear();
        self.swap_select = None;
        self.swap_chain = None;
        self.sections = None;
        self.plan_page = None;
        self.drop_guide = None;
    }
    pub fn staged_class_numbers(&self) -> &[String] {
        &self.staged
    }

    // ---------------------------------------------------------------- reads

    pub fn landing_tiles(&mut self) -> Result<Vec<Field>, PsError> {
        let u = self
            .sess
            .cfg
            .component_url("NUI_FRAMEWORK.PT_LANDINGPAGE", &[]);
        let p = self.sess.get(u, "landing")?;
        Ok(parse::generic_fields(&p.html, 200))
    }

    pub fn notification_count(&mut self) -> Result<Option<u32>, PsError> {
        let u = self.sess.cfg.iscript_url(
            "WEBLIB_PTPN.PTPN_ISCRIPT.FieldFormula.IScript_Notification_Count",
            &[("name", "")],
        );
        let p = self.sess.get(u, "notification_count")?;
        Ok(parse::parse_count(&pia::visible_text(&p.html)))
    }

    pub fn manage_classes_menu(&mut self) -> Result<Vec<Field>, PsError> {
        let u = self.sess.cfg.component_url(
            "SSR_STUDENT_FL.SSR_START_PAGE_FL",
            &[
                ("Page", "SSR_START_PAGE_FL"),
                ("Action", "U"),
                ("scname", "CS_SSR_MANAGE_CLASSES_NAV"),
                ("MD", "Y"),
            ],
        );
        let p = self.sess.get(u, "manage_menu")?;
        Ok(parse::generic_fields(&p.html, 100))
    }

    fn plan_start_url(&self) -> Url {
        let c = &self.sess.cfg;
        c.component_url(
            "VC_PE_MENU.VC_PE_SS",
            &[
                ("pslnkid", &c.plan_enrol_pslnkid),
                ("ICAJAX", "1"),
                ("ICMDTarget", "start"),
                ("ICPanelControlStyle", PANEL_STYLE),
            ],
        )
    }

    pub fn plan_overview(&mut self) -> Result<PlanOverview, PsError> {
        let p = self.sess.get(self.plan_start_url(), "plan_overview")?;
        let o = parse::parse_plan_overview(&p.html);
        self.plan_page = Some(p);
        Ok(o)
    }

    /// Expand a requirement row of the overview (a read: it only changes what the page shows).
    pub fn plan_expand_requirement(&mut self, row: u32) -> Result<PlanOverview, PsError> {
        if row > 500 {
            return Err(PsError::InvalidInput("row out of range"));
        }
        if self.plan_page.is_none() {
            self.plan_overview()?;
        }
        let prev = self
            .plan_page
            .take()
            .ok_or(PsError::Unexpected("no plan page"))?;
        if !prev.html.contains(&format!("RQRMNT_NAME_LBL${row}")) {
            self.plan_page = Some(prev);
            return Err(PsError::NotFound("requirement row not on page"));
        }
        let next = self
            .sess
            .post(&prev, &format!("RQRMNT_NAME_LBL${row}"), &[], "plan_expand")?;
        let o = parse::parse_plan_overview(&next.html);
        self.plan_page = Some(next);
        Ok(o)
    }

    // ------------------------------------------------------------ the degree tree (reads only)

    /// Open the landing page, expand the Years that say "Expand" (one `EDIT_PB$N` each; the toggle is not idempotent, so a Year is only
    /// acted on after reading its label), and return the page, the tree and the Years this call expanded.
    fn open_degree(&mut self, only_year: Option<u32>) -> Result<(Page, Degree, Vec<u32>), PsError> {
        let mut page = self.sess.get(self.plan_start_url(), "degree_overview")?;
        let mut deg = parse::parse_degree(&page.html).map_err(PsError::Unexpected)?;
        let mut expanded: Vec<u32> = Vec::new();
        for y in deg.years.clone() {
            if only_year.is_some_and(|o| o != y.index) {
                continue;
            }
            let now = deg.years.iter().find(|x| x.index == y.index).map(|x| x.state.clone()).unwrap_or_default();
            if now != "Expand" {
                continue;
            }
            let next = self.sess.post(&page, &format!("EDIT_PB${}", y.index), &[], "degree_expand")?;
            let d2 = parse::parse_degree(&next.html).map_err(PsError::Unexpected)?;
            if d2.years.iter().find(|x| x.index == y.index).map(|x| x.state.as_str()) != Some("Collapse") {
                return Err(PsError::Unexpected("a Year did not expand"));
            }
            expanded.push(y.index);
            page = next;
            deg = d2;
        }
        Ok((page, deg, expanded))
    }

    /// Put the Years this call expanded back as they were (PeopleSoft remembers a Year's state between visits). Best effort: a failure here
    /// never fails the read.
    fn close_degree(&mut self, mut page: Page, expanded: &[u32]) {
        for y in expanded.iter().rev() {
            match self.sess.post(&page, &format!("EDIT_PB${y}"), &[], "degree_collapse") {
                Ok(next) => page = next,
                Err(_) => return,
            }
        }
    }

    /// The whole degree tree of Plan & Enrol's landing page. Reads only: it expands the collapsed Years to read them and collapses them again.
    pub fn degree_tree(&mut self) -> Result<Degree, PsError> {
        let (page, deg, expanded) = self.open_degree(None)?;
        self.close_degree(page, &expanded);
        Ok(deg)
    }

    /// The free-text requirement ("MATH1020 OR AMPII Score of 85% is the pre-requisite of ...") of a course row. `year` is the Year of
    /// `degree_tree`, `index` the row's position among that Year's rows (0-based). Row numbers are not used: PeopleSoft numbers the rows of
    /// whatever is expanded, so they differ between a full tree and a single Year.
    pub fn degree_requirement(&mut self, year: u32, index: u32) -> Result<RequirementText, PsError> {
        if year > 20 || index > 500 {
            return Err(PsError::InvalidInput("row out of range"));
        }
        let (page, deg, expanded) = self.open_degree(Some(year))?;
        let Some(row) = nth_row_in_year(&deg, year, index, |r| r.has_requirement) else {
            self.close_degree(page, &expanded);
            return Err(PsError::NotFound("no requirement text for that row"));
        };
        let detail = self.sess.post(&page, &format!("RQRMNT_NAME_LBL${row}"), &[], "degree_requirement")?;
        let text = parse::parse_requirement_text(&detail.html).map_err(PsError::Unexpected);
        // Leave the page the way a person does (Cancel), then put the Year back.
        let back = self.sess.post(&detail, "#ICCancel", &[], "degree_requirement_back");
        if let Ok(b) = back {
            self.close_degree(b, &expanded);
        }
        text
    }

    /// The choices of a Course List row ("Select Courses"), with the "or" / "and" between them. Reads only: leaves with Cancel.
    /// `year` and `index` as in `degree_requirement`.
    pub fn degree_course_list(&mut self, year: u32, index: u32) -> Result<CourseList, PsError> {
        if year > 20 || index > 500 {
            return Err(PsError::InvalidInput("row out of range"));
        }
        let (page, deg, expanded) = self.open_degree(Some(year))?;
        let Some(row) = nth_row_in_year(&deg, year, index, |r| r.has_select) else {
            self.close_degree(page, &expanded);
            return Err(PsError::NotFound("that row has no course list"));
        };
        let list_page = self.sess.post(&page, &format!("VC_PE_SS_WRK_SELECT_PB${row}"), &[], "degree_course_list")?;
        let list = parse::parse_course_list(&list_page.html).map_err(PsError::Unexpected);
        let back = self.sess.post(&list_page, "VC_PE_SS_WRK_CANCEL_PB", &[], "degree_course_list_back");
        if let Ok(b) = back {
            self.close_degree(b, &expanded);
        }
        list
    }

    /// The class list of Plan & Enrol. If the start page is the overview, the "enrol" button is pressed (a read).
    pub fn plan_class_list(&mut self) -> Result<PlanClassList, PsError> {
        let p = self.sess.get(self.plan_start_url(), "plan_class_list")?;
        let direct = parse::parse_plan_class_list(&p.html);
        if !direct.rows.is_empty() {
            self.plan_page = Some(p);
            return Ok(direct);
        }
        // An empty page, a maintenance notice or anything else that is not the plan page must not read as "no courses planned":
        // every real plan page carries the component's own VC_PE_ fields.
        if !p.html.contains("VC_PE_") {
            return Err(PsError::Unexpected("the plan class list page was not recognised"));
        }
        if !p.html.contains("VC_PE_SS_WRK_VC_ENROL_PB") {
            self.plan_page = Some(p);
            return Ok(direct);
        }
        let next = self
            .sess
            .post(&p, "VC_PE_SS_WRK_VC_ENROL_PB", &[], "plan_open_list")?;
        let l = parse::parse_plan_class_list(&next.html);
        if l.rows.is_empty() && !next.html.contains("VC_PE_") {
            return Err(PsError::Unexpected("the plan class list page was not recognised"));
        }
        self.plan_page = Some(next);
        Ok(l)
    }

    /// Open the sections of the course on row `row` of the plan class list.
    pub fn plan_select_class(&mut self, row: u32) -> Result<(CourseKey, CourseSections), PsError> {
        if row > 500 {
            return Err(PsError::InvalidInput("row out of range"));
        }
        if self.plan_page.is_none() {
            self.plan_class_list()?;
        }
        let prev = self
            .plan_page
            .take()
            .ok_or(PsError::Unexpected("no plan page"))?;
        if !prev
            .html
            .contains(&format!("VC_PE_SS_WRK_VC_SELECT_CLASS_PB${row}"))
        {
            self.plan_page = Some(prev);
            return Err(PsError::NotFound("class row not on page"));
        }
        let page = self.sess.post(
            &prev,
            &format!("VC_PE_SS_WRK_VC_SELECT_CLASS_PB${row}"),
            &[],
            "plan_select_class",
        )?;
        self.plan_page = Some(prev);
        let key = key_from_url(&page.url)
            .ok_or(PsError::Unexpected("course page address missing keys"))?;
        let s = parse::parse_course_sections(&page.html);
        self.sections = Some((page, key.clone()));
        Ok((key, s))
    }

    /// Sections of one course in one term (class number, section, times, room, instructor, seats, status).
    pub fn course_sections(&mut self, key: &CourseKey) -> Result<CourseSections, PsError> {
        for v in [
            &key.strm,
            &key.crse_id,
            &key.crse_offer_nbr,
            &key.acad_career,
            &key.institution,
        ] {
            if !safe_token(v) {
                return Err(PsError::InvalidInput(
                    "course key has unexpected characters",
                ));
            }
        }
        let u = self.sess.cfg.component_url(
            "SSR_STUDENT_FL.SSR_CRSE_INFO_FL",
            &[
                ("Page", "SSR_CRSE_INFO_FL"),
                ("Action", "U"),
                ("ACAD_CAREER", &key.acad_career),
                ("CRSE_ID", &key.crse_id),
                ("CRSE_OFFER_NBR", &key.crse_offer_nbr),
                ("INSTITUTION", &key.institution),
                ("STRM", &key.strm),
            ],
        );
        let page = self.sess.get(u, "course_sections")?;
        let s = parse::parse_course_sections(&page.html);
        if s.options.is_empty() && s.course.is_empty() {
            return Err(PsError::Unexpected("course page has no course data"));
        }
        self.seat_cache
            .insert(format!("{key:?}"), (Instant::now(), s.clone()));
        self.sections = Some((page, key.clone()));
        Ok(s)
    }

    /// Seats of one class number. Served from a recent answer when asked again within [`SEAT_REFRESH_MIN`].
    /// Returns `(component, from_cache)`.
    pub fn seat_status(
        &mut self,
        key: &CourseKey,
        class_nbr: &str,
    ) -> Result<(ClassComponent, bool), PsError> {
        let ck = format!("{key:?}");
        let (sections, cached) = match self.seat_cache.get(&ck) {
            Some((t, s)) if t.elapsed() < SEAT_REFRESH_MIN => (s.clone(), true),
            _ => (self.course_sections(key)?, false),
        };
        sections
            .options
            .iter()
            .flat_map(|o| o.components.iter())
            .find(|c| c.class_nbr == class_nbr)
            .cloned()
            .map(|c| (c, cached))
            .ok_or(PsError::NotFound("class number not in this course"))
    }

    pub fn drop_terms(&mut self) -> Result<(Page, Vec<(u32, String)>), PsError> {
        let c = &self.sess.cfg;
        let u = c.component_url(
            "SSR_STUDENT_FL.SSR_DROP_CLASS_FL",
            &[
                ("Page", "SSR_DROP_TERM_FL"),
                ("pslnkid", &c.drop_pslnkid),
                ("ICAJAX", "1"),
                ("ICMDTarget", "start"),
                ("ICPanelControlStyle", PANEL_STYLE),
            ],
        );
        let p = self.sess.get(u, "drop_terms")?;
        let t = parse::parse_terms(&p.html);
        // No terms on a page that is not the drop pages at all is a failed read, not "nothing enrolled".
        if t.is_empty() && !p.html.contains("SSR_DROP") && !p.html.contains("TERM_LINK") {
            return Err(PsError::Unexpected("the drop terms page was not recognised"));
        }
        Ok((p, t))
    }

    fn open_drop_select(&mut self, term_row: u32, with_guide: bool) -> Result<(Page, EnrolledList), PsError> {
        let (terms_page, terms) = self.drop_terms()?;
        if !terms.iter().any(|(r, _)| *r == term_row) {
            return Err(PsError::NotFound("term row not on page"));
        }
        // Chooses the term on the server (a navigation step); the answer is the guide's start page.
        let start = self.sess.post(
            &terms_page,
            &format!("TERM_LINK${term_row}"),
            &[],
            "drop_term",
        )?;
        // For a write, load the guide's grouplets first, as the browser does: that is where this session's guide ids come from.
        // A read keeps the fixed ids (they open the same list) and costs no extra requests.
        self.drop_guide = None;
        if with_guide {
            self.drop_guide = self.load_drop_guide(&start);
        }
        let u = match &self.drop_guide {
            Some(g) => guided::DROP_SELECT.url_with(&self.sess.cfg, &g.list_id, &g.select_item),
            None => guided::DROP_SELECT.url(&self.sess.cfg),
        };
        let page = self.sess.get(u, "drop_select")?;
        let list = parse::parse_enrolled(&page.html);
        // A page that is not the term's enrolled list parses as an empty list. Say so instead of returning it: an empty list
        // must only ever mean "this term's list, with nothing on it".
        let expected = terms.iter().find(|(r, _)| *r == term_row).map(|(_, t)| t.as_str()).unwrap_or("");
        if !parse::is_drop_select_page(&page.html, &list, expected) {
            return Err(PsError::Unexpected(
                "the page returned is not this term's enrolled list; nothing can be concluded from it",
            ));
        }
        Ok((page, list))
    }

    /// Load the start page's grouplets and read the live guide ids from the navigation one. `None` if anything is missing.
    fn load_drop_guide(&mut self, start: &Page) -> Option<guided::DropGuide> {
        let mut found = None;
        // First the preload iScript, as the browser does: it creates this session's guide instance.
        if let Some(raw) = guided::preload_url(&start.html, "SR_DROP_FL") {
            let u = Url::parse(&raw).ok()?;
            if !self.sess.cfg.host_allowed(&u) || !u.path().contains("/EMPLOYEE/SA/s/WEBLIB_PTAI.") {
                return None;
            }
            // Its answer is a one-line script, not a page; only that the request went through matters.
            let _ = self.sess.get_from(u, &start.url, "drop_guide_preload");
        }
        for raw in guided::grouplet_urls(&start.html, "SR_DROP_FL") {
            let u = Url::parse(&raw).ok()?;
            if !self.sess.cfg.host_allowed(&u) || !u.path().contains("/EMPLOYEE/SA/c/PTGP_MENU.PTGP_GROUPLETS_FL.GBL") {
                return None;
            }
            let p = self.sess.get_from(u, &start.url, "drop_guide").ok()?;
            if let Some(g) = guided::parse_drop_guide(&p.html) {
                found = Some(g);
            }
        }
        found
    }

    /// Classes the student is enrolled in for a term (the drop pages' list). `term_row` comes from [`Self::drop_terms`].
    pub fn enrolled_classes(&mut self, term_row: u32) -> Result<EnrolledList, PsError> {
        Ok(self.open_drop_select(term_row, false)?.1)
    }

    /// Open the swap guide the way the browser does and return its select page. The browser does not just GET the select page:
    /// it opens the swap start page, which sends it to the guide's start page, loads the guide (preload, grouplets) and only then
    /// asks for the select step. Starting over also forgets anything staged before (it is a new guided session).
    fn swap_open(&mut self) -> Result<Page, PsError> {
        let u = {
            let c = &self.sess.cfg;
            c.component_url_node(
                "SA",
                "CNAQ_MENU_FL.CQ_SS_SWAP_CLASS",
                &[
                    ("pslnkid", &c.swap_pslnkid),
                    ("ICAJAX", "1"),
                    ("ICMDTarget", "start"),
                    ("ICPanelControlStyle", PANEL_STYLE),
                ],
            )
        };
        let start = self.sess.get(u, "swap_start")?;
        self.swap_pairs.clear();
        self.swap_pending_old = None;
        self.guide_chain(&start, "CQ_SS_SWAP");
        let sel = self.sess.get(guided::SWAP_SELECT.url(&self.sess.cfg), "swap_candidates")?;
        Ok(sel)
    }

    /// The options PeopleSoft's swap page offers for an enrolled course (a read: it opens the swap guide, selects the course and reads
    /// the course page; nothing is chosen or sent). Also returns the course's key, which lets later reads go straight to the course.
    pub fn swap_course_options(&mut self, candidate_row: u32) -> Result<(CourseKey, CourseSections), PsError> {
        let sel_page = self.swap_open()?;
        let cands = parse::parse_swap_candidates(&sel_page.html);
        if !cands.iter().any(|c| c.row == candidate_row) {
            return Err(PsError::NotFound("swap row not on page"));
        }
        let course_page = self.sess.post(&sel_page, &format!("CQ_SS_SWAP_WRK_SELECT_PB${candidate_row}"), &[], "swap_select")?;
        let key = key_from_url(&course_page.url).ok_or(PsError::Unexpected("the swap course page carries no course key"))?;
        let sections = parse::parse_course_sections(&course_page.html);
        // The guide is left as it was: a fresh open starts a new guided session, so nothing here can be staged by accident.
        self.swap_select = None;
        Ok((key, sections))
    }

    /// The guide's loading requests (preload, then the header and navigation grouplets) listed on its start page. Answers are not
    /// used (the swap guide's ids are fixed); the requests are what makes PeopleSoft hold the guided step.
    fn guide_chain(&mut self, start: &Page, template: &str) {
        self.swap_chain = Some((start.url.clone(), guided::grouplet_urls(&start.html, template)));
        let mut urls: Vec<(String, &'static str)> = vec![];
        if let Some(raw) = guided::preload_url(&start.html, template) {
            urls.push((raw, "swap_guide_preload"));
        }
        for raw in guided::grouplet_urls(&start.html, template) {
            urls.push((raw, "swap_guide"));
        }
        for (raw, stage) in urls {
            let Ok(u) = Url::parse(&raw) else { continue };
            let ok_path = u.path().contains("/s/WEBLIB_PTAI.") || u.path().contains("/c/PTGP_MENU.PTGP_GROUPLETS_FL.GBL");
            if !self.sess.cfg.host_allowed(&u) || !ok_path {
                continue;
            }
            let _ = self.sess.get_from(u, &start.url, stage);
        }
    }

    pub fn swap_candidates(&mut self) -> Result<Vec<SwapCandidate>, PsError> {
        let p = self.swap_open()?;
        let c = parse::parse_swap_candidates(&p.html);
        if c.is_empty() && !p.html.contains("CQ_SS_SWAP") {
            return Err(PsError::Unexpected("the swap page was not recognised"));
        }
        self.swap_select = Some(p);
        Ok(c)
    }

    /// Where a guided step stands (the iScript behind the step indicator). Raw text is not returned, only whether
    /// the answer was non-empty.
    pub fn guided_status(&mut self, list_id: &str, item_id: &str) -> Result<bool, PsError> {
        if !safe_token(list_id) || !safe_token(item_id) {
            return Err(PsError::InvalidInput("bad guided id"));
        }
        let u = self.sess.cfg.iscript_url(
            "WEBLIB_PTAI.ISCRIPT1.FieldFormula.IScript_GetItemStatus",
            &[("PTAI_ACTION", "GetItemStatus"), ("PTAI_ITEM_ID", item_id)],
        );
        let p = self.sess.get(u, "guided_status")?;
        let _ = list_id;
        Ok(!p.html.trim().is_empty())
    }

    // ---------------------------------------------------------------- write previews

    fn register(&mut self, plan: WritePlan, payload: Payload) -> Result<Preview, PsError> {
        self.gate
            .register(plan, payload, Instant::now())
            .map_err(|e| match e {
                GateError::Disarmed => PsError::WriteRefused("writes are off"),
                _ => PsError::WriteRefused("could not create preview"),
            })
    }

    fn require_armed(&self) -> Result<(), PsError> {
        if self.gate.is_armed() {
            Ok(())
        } else {
            Err(PsError::WriteRefused("writes are off"))
        }
    }

    /// Preview: pick class option `option_row` of the last opened course page as the pending enrolment choice.
    pub fn preview_stage_enrol(&mut self, option_row: u32) -> Result<Preview, PsError> {
        self.require_armed()?;
        let (page, _key) = self
            .sections
            .take()
            .ok_or(PsError::InvalidInput("open a course's sections first"))?;
        if page.form.is_none() {
            return Err(PsError::Unexpected("course page has no usable form"));
        }
        let sections = parse::parse_course_sections(&page.html);
        let opt = sections
            .options
            .iter()
            .find(|o| o.row == option_row)
            .ok_or(PsError::NotFound("option row not on page"))?;
        let nums: Vec<String> = opt.components.iter().map(|c| c.class_nbr.clone()).collect();
        if nums.is_empty() {
            return Err(PsError::Unexpected("option has no class numbers"));
        }
        let mut warnings = vec![
            "Effect inferred from the capture, not confirmed: the server appears to remember this option as your pending choice; nothing is enrolled until the separate ENROLL step.".to_string(),
        ];
        for c in &opt.components {
            if let Some(s) = &c.seats {
                if s.open == 0 {
                    warnings.push(format!(
                        "Class {} shows no open seats ({}).",
                        c.class_nbr, c.seats_text
                    ));
                }
            }
        }
        if opt.status != "Open" {
            warnings.push(format!("Option status is '{}', not Open.", opt.status));
        }
        let plan = WritePlan::new(
            WriteKind::StageEnrol,
            &sections.term,
            nums.clone(),
            format!(
                "Choose option {} of {} ({})",
                opt.option_no, sections.course, sections.title
            ),
            vec![
                PlannedStep {
                    component: "SSR_STUDENT_FL.SSR_CRSE_INFO_FL".into(),
                    icaction: SETFIELD_ACTION.into(),
                },
                PlannedStep {
                    component: "SSR_STUDENT_FL.SSR_CRSE_INFO_FL".into(),
                    icaction: format!("{STAGE_ACTION_PREFIX}${option_row}"),
                },
            ],
            warnings,
        );
        self.register(
            plan,
            Payload::Stage {
                page,
                row: option_row,
                nums,
                swap: false,
                swap_row: 0,
            },
        )
    }

    /// Preview: submit the pending Plan & Enrol choices. Only class numbers staged by this process can be named.
    pub fn preview_submit_enrol(&mut self) -> Result<Preview, PsError> {
        self.require_armed()?;
        if self.staged.is_empty() {
            return Err(PsError::WriteRefused(
                "nothing was staged by this session; stage a class option first",
            ));
        }
        let list = self.plan_class_list()?;
        let page = self
            .plan_page
            .take()
            .ok_or(PsError::Unexpected("no plan page"))?;
        if !list.can_submit {
            return Err(PsError::Unexpected("submit button not on the page"));
        }
        let mut warnings = vec![
            "Submitting enrols you for real. The confirmation page shows an amount and payment details, so fees may apply; that amount is only known after the submit.".to_string(),
            "Whether this can be undone depends on drop deadlines and seat availability, which this preview did not read.".to_string(),
        ];
        for r in &list.rows {
            warnings.push(format!("On page: {} [{}]", r.course_label, r.status));
        }
        let plan = WritePlan::new(
            WriteKind::SubmitEnrol,
            &list.term_label,
            self.staged.clone(),
            format!(
                "Submit Plan & Enrol for staged classes {}",
                self.staged.join(", ")
            ),
            vec![PlannedStep {
                component: "VC_PE_MENU.VC_PE_SS".into(),
                icaction: "VC_PE_SS_WRK_VC_SUBMIT_ENROL_PB".into(),
            }],
            warnings,
        )
        .with_live_rows(list.rows.iter().map(|r| (r.course_label.clone(), r.status.clone(), r.schedule_summary.clone())).collect());
        self.register(plan, Payload::SubmitEnrol { page })
    }

    /// Preview: drop one enrolled class. Navigates and selects the class (no change yet), reads the review page.
    pub fn preview_drop(&mut self, term_row: u32, class_nbr: &str) -> Result<Preview, PsError> {
        self.require_armed()?;
        if !safe_token(class_nbr) || !class_nbr.bytes().all(|b| b.is_ascii_digit()) {
            return Err(PsError::InvalidInput("class number must be digits"));
        }
        let (select_page, list) = self.open_drop_select(term_row, true)?;
        // Without this session's own guide ids a drop is accepted by the site and silently not applied, so refuse.
        let guide = self
            .drop_guide
            .clone()
            .ok_or(PsError::Unexpected("could not read this session's drop guide ids; nothing was selected"))?;
        let cls = list
            .classes
            .iter()
            .find(|c| c.class_nbr == class_nbr)
            .ok_or(PsError::NotFound("class number not in your enrolled list"))?
            .clone();
        let r = cls.row.to_string();
        let chk = format!("P_SELECT$chk${r}");
        let sel = format!("P_SELECT${r}");
        let _ = self.sess.post(
            &select_page,
            &sel,
            &[(&chk, "Y"), (&sel, "Y")],
            "drop_select",
        )?;
        let review = self
            .sess
            .get(
                guided::DROP_REVIEW.url_with(&self.sess.cfg, &guide.list_id, &guide.review_item),
                "drop_review",
            )?;
        let shown = parse::parse_enrolled(&review.html);
        if !shown.classes.iter().any(|c| c.class_nbr == class_nbr) {
            return Err(PsError::Unexpected(
                "review page does not show the selected class",
            ));
        }
        let mut warnings = vec![
            "A dropped seat can be taken by another student; getting it back is not guaranteed."
                .to_string(),
            "Drop deadlines, penalties and fees were not read from the pages and are unknown here."
                .to_string(),
        ];
        if shown.classes.len() > 1 {
            warnings.push(format!(
                "The review page lists {} classes (a lecture and its lab may be linked): {}",
                shown.classes.len(),
                shown
                    .classes
                    .iter()
                    .map(|c| c.class_nbr.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        let plan = WritePlan::new(
            WriteKind::Drop,
            &list.term,
            vec![class_nbr.to_string()],
            format!("Drop {} {} {}", cls.course, cls.component, cls.section),
            vec![
                PlannedStep {
                    component: "SSR_STUDENT_FL.SSR_DROP_REVIEW_FL".into(),
                    icaction: "DERIVED_SSR_FL_SSR_DROPPED".into(),
                },
                PlannedStep {
                    component: "SSR_STUDENT_FL.SSR_DROP_REVIEW_FL".into(),
                    icaction: "#ICYes".into(),
                },
            ],
            warnings,
        )
        .with_listed_classes(list.classes.iter().map(|c| c.class_nbr.clone()).collect());
        self.register(plan, Payload::Drop { page: review })
    }

    /// Preview of swap, step one: choose the class to swap out (`candidate_row`) and the new option of that course, by the option's
    /// row on the swap's own course page.
    pub fn preview_swap_stage(&mut self, candidate_row: u32, option_row: u32) -> Result<Preview, PsError> {
        self.swap_stage_preview(candidate_row, OptionRef::Row(option_row))
    }

    /// The same, naming the new option by its class numbers (what PeopleSoft's own course page for the swap lists), which is the
    /// reliable way: option rows differ between the plan's course page and the swap's.
    pub fn preview_swap_stage_nums(&mut self, candidate_row: u32, new_nums: &[String]) -> Result<Preview, PsError> {
        self.swap_stage_preview(candidate_row, OptionRef::Nums(new_nums.to_vec()))
    }

    fn swap_stage_preview(&mut self, candidate_row: u32, which: OptionRef) -> Result<Preview, PsError> {
        self.require_armed()?;
        let sel_page = match self.swap_select.take() {
            Some(p) => p,
            None => self.swap_open()?,
        };
        let cands = parse::parse_swap_candidates(&sel_page.html);
        let cand = cands
            .iter()
            .find(|c| c.row == candidate_row)
            .ok_or(PsError::NotFound("swap row not on page"))?
            .clone();
        let old_nums = class_numbers_in(&cand.class_text);
        let course_page = self.sess.post(
            &sel_page,
            &format!("CQ_SS_SWAP_WRK_SELECT_PB${candidate_row}"),
            &[],
            "swap_select",
        )?;
        let sections = parse::parse_course_sections(&course_page.html);
        let opt = match &which {
            OptionRef::Row(r) => sections.options.iter().find(|o| o.row == *r),
            OptionRef::Nums(n) => sections.options.iter().find(|o| {
                let have: Vec<&String> = o.components.iter().map(|c| &c.class_nbr).collect();
                have.len() == n.len() && n.iter().all(|x| have.contains(&x))
            }),
        }
        .ok_or(PsError::NotFound("that option is not offered on the swap's course page"))?;
        let option_row = opt.row;
        let after_setfield = self.sess.post(&course_page, SETFIELD_ACTION, &[], "swap_course_page")?;
        let new_nums: Vec<String> = opt.components.iter().map(|c| c.class_nbr.clone()).collect();
        if new_nums.is_empty() {
            return Err(PsError::Unexpected("option has no class numbers"));
        }
        let mut warnings = vec![
            "PeopleSoft swaps in one step: it takes the new combination and releases the old one. If it refuses (a clash, a full class), nothing changes.".to_string(),
        ];
        if new_nums.iter().any(|n| old_nums.contains(n)) {
            warnings.push("This option shares a class number with your current class.".to_string());
        }
        let plan = WritePlan::new(
            WriteKind::SwapStage,
            &sections.term,
            new_nums.clone(),
            format!("Swap {}: pick new option {} ({})", cand.course, opt.option_no, new_nums.join(", ")),
            vec![PlannedStep {
                component: "SSR_STUDENT_FL.SSR_CRSE_INFO_FL".into(),
                icaction: format!("{STAGE_ACTION_PREFIX}${option_row}"),
            }],
            warnings,
        );
        self.swap_pending_old = Some(old_nums);
        self.register(
            plan,
            Payload::Stage {
                page: after_setfield,
                row: option_row,
                nums: new_nums,
                swap: true,
                swap_row: candidate_row,
            },
        )
    }

    /// Preview of swap, step two: the review page, which lists every swap staged in this guided session. One confirm sends them all.
    pub fn preview_swap_commit(&mut self) -> Result<Preview, PsError> {
        self.require_armed()?;
        if self.swap_pairs.is_empty() || self.swap_pairs.iter().any(|(o, n)| o.is_empty() || n.is_empty()) {
            return Err(PsError::WriteRefused("no swap was staged by this session"));
        }
        // The browser loads the guide's grouplets again between the select step and the review step.
        self.guide_chain_light();
        let review = self.sess.get(guided::SWAP_REVIEW.url(&self.sess.cfg), "swap_review")?;
        let rows = parse::parse_swap_review_rows(&review.html);
        // The review must show exactly the swaps that were staged, no more and no fewer. A review opened without the guide being
        // in the right step shows something else or nothing; confirming it would act on a page the person did not approve.
        let nbrs = |t: &str| class_numbers_in(t);
        let mut shown: Vec<(Vec<String>, Vec<String>)> = rows.iter().map(|r| (nbrs(&r.current_text), nbrs(&r.new_text))).collect();
        let mut want = self.swap_pairs.clone();
        for (o, n) in shown.iter_mut().chain(want.iter_mut()) {
            o.sort();
            n.sort();
        }
        shown.sort();
        want.sort();
        if shown != want {
            return Err(PsError::Unexpected(
                "the swap review page does not show the swaps that were staged; nothing was confirmed",
            ));
        }
        let term = rows.first().map(|r| r.term.clone()).unwrap_or_default();
        let mut nums: Vec<String> = self.swap_pairs.iter().map(|(o, _)| o.first().cloned().unwrap_or_default()).collect();
        nums.push("TO".into());
        for (_, n) in &self.swap_pairs {
            nums.extend(n.iter().cloned());
        }
        let summary = rows.iter().map(|r| format!("{}: {} -> {}", r.course, r.current_text.replace('\n', " / "), r.new_text.replace('\n', " / "))).collect::<Vec<_>>().join("; ");
        let plan = WritePlan::new(
            WriteKind::SwapCommit,
            &term,
            nums,
            format!("Confirm {} swap(s): {summary}", rows.len()),
            vec![PlannedStep { component: "CNAQ_MENU_FL.CQ_SS_SWAP_FL".into(), icaction: "CQ_SS_SWAP_WRK_CONTINUE_PB".into() }],
            vec![
                "PeopleSoft swaps in one step per course: it takes the new combination and releases the old one. It tells you afterwards, per course, whether it worked.".to_string(),
            ],
        );
        self.register(plan, Payload::SwapCommit { page: review })
    }

    /// The grouplets again (no preload), as the browser does between the select and review steps.
    fn guide_chain_light(&mut self) {
        let Some((referer, urls)) = self.swap_chain.clone() else { return };
        for raw in urls {
            let Ok(u) = Url::parse(&raw) else { continue };
            if self.sess.cfg.host_allowed(&u) && u.path().contains("/c/PTGP_MENU.PTGP_GROUPLETS_FL.GBL") {
                let _ = self.sess.get_from(u, &referer, "swap_guide");
            }
        }
    }


    // ------------------------------------------------------------ the course catalog (reads only)

    /// Search the Browse Course Catalog by keyword (a code such as "INFS 2201" or a subject such as "INFS"). Returns each course once, with the
    /// newest effective date. Reads only: it types into the search box and reads the results.
    pub fn catalog_search(&mut self, keyword: &str) -> Result<Vec<CatalogHit>, PsError> {
        let k = keyword.trim();
        if k.is_empty() || k.len() > 40 || !k.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '-') {
            return Err(PsError::InvalidInput("keyword must be 1 to 40 letters, digits, spaces or hyphens"));
        }
        let c = &self.sess.cfg;
        let url = c.component_url(
            "SSR_STUDENT_FL.SSR_BROWSE_CTLG_FL",
            &[("Page", "SSR_BROWSE_CTLG_FL"), ("pslnkid", "CS_SSR_BROWSE_CTLG_FL_LINK"), ("ICAJAX", "1"), ("ICMDTarget", "start"), ("ICPanelControlStyle", PANEL_STYLE)],
        );
        let start = self.sess.get(url, "catalog_start")?;
        if !start.html.contains("PTS_KEYWORDS3") {
            return Err(PsError::Unexpected("the catalog search page was not recognised"));
        }
        let results = self.sess.post(&start, "#KEY\r", &[("PTS_KEYWORDS3", k)], "catalog_search")?;
        let hits = parse::parse_catalog_results(&results.html).map_err(PsError::Unexpected)?;
        Ok(dedupe_latest(hits))
    }

    /// Course detail from Browse Course Catalog (`SSR_CRSE_DTL_FL`): units, components, description, and the Enrollment Requirement text.
    /// Reads only: opens the catalog detail page for `(crse_id, offer_nbr, effdt)` from `catalog_search`.
    pub fn catalog_detail(&mut self, crse_id: &str, offer_nbr: &str, effdt: &str) -> Result<CatalogDetail, PsError> {
        let valid_digits = |s: &str, max: usize| !s.is_empty() && s.len() <= max && s.bytes().all(|b| b.is_ascii_digit());
        if !valid_digits(crse_id, 10) || !valid_digits(offer_nbr, 4) {
            return Err(PsError::InvalidInput("crse_id and offer_nbr must be digits"));
        }
        let d = effdt.as_bytes();
        if d.len() != 10 || d[4] != b'-' || d[7] != b'-' || !effdt.bytes().enumerate().all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit()) {
            return Err(PsError::InvalidInput("effdt must be YYYY-MM-DD"));
        }
        let url = self.sess.cfg.component_url_node(
            "PSFT_CS",
            "SSR_STUDENT_FL.SSR_CRSE_DTL_FL",
            &[
                ("Page", "SSR_CRSE_DTL_FL"),
                ("Action", "U"),
                ("CRSE_ID", crse_id),
                ("CRSE_OFFER_NBR", offer_nbr),
                ("EFFDT", effdt),
                ("ICAJAX", "1"),
                ("ICMDTarget", "start"),
                ("ICPanelControlStyle", PANEL_STYLE),
            ],
        );
        let page = self.sess.get(url, "catalog_detail")?;
        parse::parse_catalog_detail(&page.html).map_err(PsError::Unexpected)
    }

    // ------------------------------------------------------------ course planning writes (never enrols)

    /// The term dropdown of the `index`-th row of a Year, read from a degree tree. Errors when the row has no dropdown (taken, in progress,
    /// or a group row).
    fn plan_row(deg: &Degree, year: u32, index: u32) -> Result<DegreeRow, PsError> {
        deg.rows
            .iter()
            .filter(|r| r.year == year)
            .nth(index as usize)
            .filter(|r| r.term_choice.is_some())
            .cloned()
            .ok_or(PsError::NotFound("that row has no term dropdown (taken, in progress, or not a course)"))
    }

    /// Preview: plan the `index`-th row of `year` into the term with code `to` (a code from that row's own dropdown) and save the plan.
    /// Reads the tree (a read) and sends nothing that changes data.
    pub fn preview_plan_term(&mut self, year: u32, index: u32, to: &str) -> Result<Preview, PsError> {
        self.require_armed()?;
        // An empty term code means "not planned" (un-plan): the blank entry of the dropdown.
        if year > 20 || index > 500 || to.len() > 6 || !to.bytes().all(|b| b.is_ascii_digit()) {
            return Err(PsError::InvalidInput("year, index or term code out of range"));
        }
        let (page, deg, expanded) = self.open_degree(Some(year))?;
        self.close_degree(page, &expanded);
        let row = Self::plan_row(&deg, year, index)?;
        let choice = row.term_choice.clone().ok_or(PsError::Unexpected("no term dropdown"))?;
        let label_of = |code: &str| choice.options.iter().find(|o| o.code == code).map(|o| o.label.clone());
        let to_label = if to.is_empty() { "not planned".to_string() } else { label_of(to).ok_or(PsError::NotFound("that term is not offered for this course"))? };
        if choice.selected == to {
            return Err(PsError::InvalidInput("the course is already planned in that term"));
        }
        let from_label = label_of(&choice.selected).unwrap_or_else(|| "not planned".to_string());
        let compact: String = row.code.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        let plan = WritePlan::new(
            WriteKind::PlanTerm,
            &to_label,
            vec![compact, if to.is_empty() { "NONE".to_string() } else { to.to_string() }],
            if to.is_empty() { format!("Remove {} from your plan (now: {}) and save the plan", row.code, from_label) } else { format!("Plan {} in {} (now: {}) and save the plan", row.code, to_label, from_label) },
            vec![
                PlannedStep { component: "VC_PE_MENU.VC_PE_SS".into(), icaction: format!("DESCR10${}", row.row) },
                PlannedStep { component: "VC_PE_MENU.VC_PE_SS".into(), icaction: "VC_PE_SS_WRK_VC_SAVE_PLAN_PB".into() },
            ],
            vec![
                "This changes your course planning only. It does not enrol you in anything.".to_string(),
                "PeopleSoft counts every planned course of a term towards its credit limit when you enrol.".to_string(),
            ],
        );
        self.register(plan, Payload::PlanTerm { year, index, code: row.code.clone(), from: choice.selected.clone(), to: to.to_string() })
    }

    fn run_plan_term(&mut self, proof: &WriteProof, plan: WritePlan, year: u32, index: u32, code: String, from: String, to: String) -> Result<WriteOutcome, PsError> {
        // Read again right now: the plan may have changed since the preview. Nothing is sent unless it is exactly as previewed.
        let (page, deg, expanded) = self.open_degree(Some(year))?;
        let row = match Self::plan_row(&deg, year, index) {
            Ok(r) if r.code == code && r.term_choice.as_ref().is_some_and(|t| t.selected == from && (to.is_empty() || t.options.iter().any(|o| o.code == to))) => r,
            Ok(_) | Err(_) => {
                self.close_degree(page, &expanded);
                return Err(PsError::Unexpected("the plan changed since the preview; nothing was sent"));
            }
        };
        let before_year: Vec<(u32, Option<String>)> = deg.rows.iter().filter(|r| r.year == year).map(|r| (r.row, r.term_choice.as_ref().map(|t| t.selected.clone()))).collect();
        let name = format!("DESCR10${}", row.row);
        let hidden = format!("VC_PE_SS_L3_VW_DESCR10${}", row.row);
        let set = self.sess.post_write(proof, &page, &name, &[(&name, &to), (&hidden, &from)], "plan_set_term")?;
        // The answer re-renders this one dropdown; it must show the new term before anything is saved.
        let shown = selected_in(&set.html, &name);
        if shown.as_deref() != Some(to.as_str()) {
            self.close_degree(set, &expanded);
            return Err(PsError::Unexpected("PeopleSoft did not show the new term after the change; nothing was saved"));
        }
        let saved = self.sess.post_write(proof, &set, "VC_PE_SS_WRK_VC_SAVE_PLAN_PB", &[(&hidden, &to)], "plan_save")?;
        let messages = pia::outcome_hints(&pia::fields(&saved.html));
        self.close_degree(saved, &expanded);
        // The proof: read the plan again from PeopleSoft and compare every course of this Year.
        let (after_page, after, after_expanded) = self.open_degree(Some(year))?;
        self.close_degree(after_page, &after_expanded);
        let after_year: Vec<(u32, Option<String>)> = after.rows.iter().filter(|r| r.year == year).map(|r| (r.row, r.term_choice.as_ref().map(|t| t.selected.clone()))).collect();
        let target_now = Self::plan_row(&after, year, index).ok().and_then(|r| r.term_choice).map(|t| t.selected);
        let mut changed: Vec<u32> = Vec::new();
        for (b, a) in before_year.iter().zip(after_year.iter()) {
            if b != a {
                changed.push(b.0);
            }
        }
        let others_unchanged = before_year.len() == after_year.len() && changed.iter().all(|r| *r == row.row);
        let saved_ok = target_now.as_deref() == Some(to.as_str());
        let detail = serde_json::json!({
            "course": code, "from": from, "to": to,
            "saved": saved_ok, "others_unchanged": others_unchanged, "changed_rows": changed,
            "shown_after_change": shown,
        });
        if !saved_ok || !others_unchanged {
            return Err(PsError::Unexpected("PeopleSoft's plan read back does not match what was sent; check Plan and Enrol by hand"));
        }
        Ok(WriteOutcome { kind: plan.kind, messages, detail, plan })
    }

    // ---------------------------------------------------------------- confirm

    /// Execute a previewed write. Needs the token from the preview and the exact phrase the human typed.
    pub fn confirm(
        &mut self,
        token: &str,
        typed_phrase: &str,
    ) -> Result<WriteOutcome, ConfirmError> {
        let (proof, plan, payload) = self
            .gate
            .redeem(token, typed_phrase, Instant::now())
            .map_err(ConfirmError::Gate)?;
        let kind = plan.kind;
        let out = match payload {
            Payload::Stage {
                page,
                row,
                nums,
                swap,
                swap_row,
            } => self.run_stage(&proof, plan, page, row, nums, swap, swap_row)?,
            Payload::SubmitEnrol { page } => self.run_submit_enrol(&proof, plan, page)?,
            Payload::Drop { page } => self.run_drop(&proof, plan, page)?,
            Payload::SwapCommit { page } => self.run_swap_commit(&proof, plan, page)?,
            Payload::PlanTerm { year, index, code, from, to } => self.run_plan_term(&proof, plan, year, index, code, from, to)?,
        };
        debug_assert_eq!(out.kind, kind);
        Ok(out)
    }

    fn run_stage(
        &mut self,
        proof: &WriteProof,
        plan: WritePlan,
        page: Page,
        row: u32,
        nums: Vec<String>,
        swap: bool,
        swap_row: u32,
    ) -> Result<WriteOutcome, PsError> {
        // The set-field step (a read) comes first for a plain enrol stage; the swap path already did it.
        let base = if swap {
            page
        } else {
            self.sess
                .post(&page, SETFIELD_ACTION, &[], "stage_setfield")?
        };
        let result = self.sess.post_write(
            proof,
            &base,
            &format!("{STAGE_ACTION_PREFIX}${row}"),
            &[],
            "stage_option",
        )?;
        let messages = pia::outcome_hints(&pia::fields(&result.html));
        if swap {
            // The stage answered with a redirect into the guide's start page. Follow the browser from there: load the guide, then ask
            // for the select step again, which now shows the chosen new class next to the current one. That page is the proof the
            // stage took effect, and the page the next course's new class is chosen on.
            let old = self.swap_pending_old.take().unwrap_or_default();
            self.guide_chain(&result, "CQ_SS_SWAP");
            let sel = self.sess.get(guided::SWAP_SELECT.url(&self.sess.cfg), "swap_select_after")?;
            let shown = parse::parse_swap_candidates(&sel.html)
                .into_iter()
                .find(|c| c.row == swap_row)
                .map(|c| class_numbers_in(&c.other_text))
                .unwrap_or_default();
            let mut a = shown.clone();
            let mut b = nums.clone();
            a.sort();
            b.sort();
            self.swap_select = Some(sel);
            if a != b {
                // Class numbers only (never page text): what the page shows against what was staged. Rare, so the leak is bounded.
                let why: &'static str = Box::leak(format!("PeopleSoft's select page does not show the new class that was staged (it shows {a:?}, expected {b:?})").into_boxed_str());
                return Err(PsError::Unexpected(why));
            }
            self.swap_pairs.push((old, nums.clone()));
        } else {
            for n in &nums {
                if !self.staged.contains(n) {
                    self.staged.push(n.clone());
                }
            }
        }
        Ok(WriteOutcome {
            kind: plan.kind,
            messages,
            detail: serde_json::json!({"staged_class_numbers": nums}),
            plan,
        })
    }

    fn run_submit_enrol(
        &mut self,
        proof: &WriteProof,
        plan: WritePlan,
        page: Page,
    ) -> Result<WriteOutcome, PsError> {
        let result = self.sess.post_write(
            proof,
            &page,
            "VC_PE_SS_WRK_VC_SUBMIT_ENROL_PB",
            &[],
            "enrol_submit",
        )?;
        let conf = parse::parse_enrol_confirmation(&result.html);
        self.staged.clear();
        Ok(WriteOutcome {
            kind: plan.kind,
            messages: conf.messages.clone(),
            detail: serde_json::to_value(&conf).unwrap_or_default(),
            plan,
        })
    }

    fn run_drop(
        &mut self,
        proof: &WriteProof,
        plan: WritePlan,
        page: Page,
    ) -> Result<WriteOutcome, PsError> {
        let prompt = self.sess.post_write(
            proof,
            &page,
            "DERIVED_SSR_FL_SSR_DROPPED",
            &[],
            "drop_click",
        )?;
        let hints = pia::outcome_hints(&pia::fields(&prompt.html));
        if !hints.is_empty() {
            // The page reported a problem before the final yes: stop here, send nothing more.
            return Ok(WriteOutcome {
                kind: plan.kind,
                messages: hints,
                detail: serde_json::json!({"stopped_before_yes": true}),
                plan,
            });
        }
        let _ = self
            .sess
            .post_write(proof, &prompt, "#ICYes", &[], "drop_yes")?;
        let c = &self.sess.cfg;
        let u = c.component_url(
            "SSR_STUDENT_FL.SSR_DROP_CLASS_FL",
            &[
                ("Page", "SSR_DROP_CONFRM_FL"),
                ("Action", "U"),
                ("ICAJAX", "1"),
                ("ICMDTarget", "start"),
                ("ICPanelControlStyle", PANEL_STYLE),
            ],
        );
        let done = self.sess.get(u, "drop_result")?;
        let after = parse::parse_enrolled(&done.html);
        Ok(WriteOutcome {
            kind: plan.kind,
            messages: pia::outcome_hints(&pia::fields(&done.html)),
            detail: serde_json::to_value(&after).unwrap_or_default(),
            plan,
        })
    }

    fn run_swap_commit(
        &mut self,
        proof: &WriteProof,
        plan: WritePlan,
        page: Page,
    ) -> Result<WriteOutcome, PsError> {
        let after = self.sess.post_write(
            proof,
            &page,
            "CQ_SS_SWAP_WRK_CONTINUE_PB",
            &[],
            "swap_commit",
        )?;
        // The browser does not ask for the result page right away: the commit sends it to the Manage Classes navigation start page, it
        // loads that navigation's grouplet, and only then asks for the result page (qa3 HAR, entries 393 to 501). Asked for at once, the
        // answer is the start page again (10 KB, no result rows), which is why PeopleSoft's per-course message used to be missing.
        let nav = {
            let c = &self.sess.cfg;
            c.component_url_node(
                "PSFT_CS",
                "PTGP_MENU.PTGP_GROUPLETS_FL",
                &[
                    ("PTGP_TYPE", "NavigationCollectionDataSource"),
                    ("PAGE", "PTGP_GPLT_NV_FL"),
                    ("AGComp", "N"),
                    ("TEMPLATE_ID", "PTPPNAVCOL"),
                    ("scname", "CS_SSR_MANAGE_CLASSES_NAV"),
                    ("ptgpid", &c.swap_pslnkid),
                    ("ICDoModal", "1"),
                    ("ICGrouplet", "1"),
                    ("ICAction", "GROUPLET_CONTENTS$hmodal"),
                    ("ICLoc", "1"),
                    ("nWidth", "286"),
                    ("nHeight", "57"),
                ],
            )
        };
        if self.sess.cfg.host_allowed(&nav) {
            let _ = self.sess.get_from(nav, &after.url, "swap_result_nav");
        }
        let u = self.sess.cfg.component_url_node(
            "SA",
            "CNAQ_MENU_FL.CQ_SS_SWAP_CLASS",
            &[
                ("NavColl", "true"),
                ("ICAJAX", "1"),
                ("ICAGTarget", "start"),
                ("ICPanelControlStyle", "pst_side1-fixed pst_panel-mode "),
            ],
        );
        let done = self.sess.get(u, "swap_result")?;
        let rows = parse::parse_swap_result_rows(&done.html);
        let r = parse::parse_swap_review(&done.html);
        self.swap_pairs.clear();
        self.swap_pending_old = None;
        self.swap_select = None;
        let mut messages = r.messages.clone();
        for row in &rows {
            if !row.message.is_empty() && !messages.contains(&row.message) {
                messages.push(row.message.clone());
            }
        }
        Ok(WriteOutcome {
            kind: plan.kind,
            messages,
            detail: serde_json::json!({ "results": rows }),
            plan,
        })
    }
}

/// The value a `<select id="...">` shows as selected in a piece of page html (an AJAX answer carries the re-rendered dropdown).
fn selected_in(html: &str, id: &str) -> Option<String> {
    let start = html.find(&format!("<select id='{id}'")).or_else(|| html.find(&format!("<select id=\"{id}\"")))?;
    let tail = &html[start..];
    let sel = &tail[..tail.find("</select>")?];
    let re = regex::Regex::new(r#"(?is)<option\s+value=['"]([^'"]*)['"]\s+selected"#).ok()?;
    re.captures(sel).map(|c| c[1].to_string())
}

#[cfg(test)]
mod selected_tests {
    use super::selected_in;
    #[test]
    fn reads_the_selected_option_of_one_dropdown() {
        let html = "<select id='DESCR10$19' class='x'>\n<option value=\"\">&nbsp; </option>\n<option value=\"1278\">Fall 2027</option>\n<option value=\"1271\" selected='selected'>Winter 2027</option>\n</select><select id='DESCR10$21'><option value=\"1275\" selected='selected'>S</option></select>";
        assert_eq!(selected_in(html, "DESCR10$19").as_deref(), Some("1271"));
        assert_eq!(selected_in(html, "DESCR10$21").as_deref(), Some("1275"));
        assert_eq!(selected_in(html, "DESCR10$5"), None);
        assert_eq!(selected_in("<select id='DESCR10$19'><option value=\"\">x</option></select>", "DESCR10$19"), None);
    }
}
