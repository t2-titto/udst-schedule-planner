//! Typed results. Everything here is plain data (serde) and is derived only from page text.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meeting {
    pub day: String,
    pub start: String,
    pub end: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seats {
    pub open: u32,
    pub total: u32,
}

/// One meeting component of a class option (lecture, laboratory, ...), with its own class number.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassComponent {
    pub kind: String,
    pub class_nbr: String,
    pub section: String,
    pub dates: String,
    pub meetings: Vec<Meeting>,
    pub schedule_text: String,
    pub room: String,
    pub instructor: String,
    pub seats: Option<Seats>,
    pub seats_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassOption {
    /// Row index on the page (what the option click uses), zero based.
    pub row: u32,
    /// Option number as the page shows it.
    pub option_no: String,
    pub status: String,
    pub session: String,
    pub components: Vec<ClassComponent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CourseKey {
    pub strm: String,
    pub crse_id: String,
    pub crse_offer_nbr: String,
    pub acad_career: String,
    pub institution: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CourseSections {
    pub term: String,
    pub course: String,
    pub title: String,
    pub description: String,
    pub units: String,
    pub grading: String,
    pub components_offered: String,
    pub career: String,
    pub options_text: String,
    pub options: Vec<ClassOption>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanCourse {
    pub row: u32,
    pub title: String,
    pub category: String,
    pub units: String,
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Requirement {
    pub name: String,
    pub detail: String,
    pub status: String,
    pub courses: Vec<PlanCourse>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanOverview {
    pub requirements: Vec<Requirement>,
}

/// A row of the Plan & Enrol class list (`VC_PE_SS_ENROL`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanClassRow {
    pub row: u32,
    pub course_label: String,
    /// The term this row belongs to (the live page shows the term here, e.g. "Fall 2026").
    pub term: String,
    pub status: String,
    pub schedule_summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanClassList {
    pub term_label: String,
    pub rows: Vec<PlanClassRow>,
    /// Whether the submit button is on the page.
    pub can_submit: bool,
}

/// A class the student is enrolled in, as listed on the drop pages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrolledClass {
    pub row: u32,
    pub class_nbr: String,
    pub component: String,
    pub section: String,
    pub course: String,
    pub schedule: Vec<String>,
    pub rooms: Vec<String>,
    pub instructors: Vec<String>,
    pub units: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrolledList {
    pub term: String,
    pub classes: Vec<EnrolledClass>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwapCandidate {
    pub row: u32,
    pub term: String,
    pub course: String,
    pub class_text: String,
    pub other_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwapReview {
    pub term: String,
    pub course: String,
    pub current_class: String,
    pub new_class: String,
    pub messages: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrolConfirmation {
    pub total_amount: String,
    pub payment_detail: String,
    pub lines: Vec<String>,
    pub icons: Vec<String>,
    pub messages: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MenuEntry {
    pub id: String,
    pub text: String,
}

/// One course of the swap review page (`CQ_SS_SWAP_REV_FL`): what is held now and what it becomes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwapReviewRow {
    pub term: String,
    pub course: String,
    pub current_text: String,
    pub new_text: String,
}

/// One course of the page that follows a swap: PeopleSoft's own per-course answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwapResultRow {
    pub course: String,
    pub class_nbr: String,
    pub changed_to: String,
    pub message: String,
}

// ---------------------------------------------------------------- the degree tree (Plan & Enrol landing page)

/// A Year of the degree tree ("910-901-R Year 1"). `state` is what the page says its toggle does: "Expand" (collapsed now) or "Collapse".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DegreeYear {
    pub index: u32,
    pub name: String,
    /// "Total Courses Required: 10 Completed: 100%", as the page words it.
    pub summary: String,
    pub state: String,
}

/// A Semester header inside a Year ("910-901-R Semester 1"), with its own summary line ("Total Courses Required: 4 Completed: 100%").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DegreeSemester {
    pub index: u32,
    pub year: u32,
    pub name: String,
    pub summary: String,
}

/// One term the planner offers for a course: PeopleSoft's term code ("1271") and its name ("Winter 2027").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermOption {
    pub code: String,
    pub label: String,
}

/// The term dropdown of a course that is not taken yet: the options and the code that is selected ("" = not planned).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermChoice {
    pub selected: String,
    pub options: Vec<TermOption>,
}

/// One row of the tree: a course, or a requirement group such as a Course List ("Semester 6 - GARC Course List").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DegreeRow {
    /// The row number the page uses in its field names (`VC_PE_SS_L3_VW_DESCR50$N`).
    pub row: u32,
    /// The Year this row sits under.
    pub year: u32,
    /// The Semester header this row follows (its `index`), when the page shows one.
    pub semester: Option<u32>,
    /// The label as shown, without the leading "and".
    pub label: String,
    /// The row was joined to the one before with "and" (inside a semester block); a row without it starts a new block.
    pub joined_by_and: bool,
    /// "DACS 2202" when the label is "DACS 2202: Intro. to Computing Systems"; empty for a group row.
    pub code: String,
    pub title: String,
    pub category: String,
    /// Units as the page writes them ("3.00"); None for a group row.
    pub units: Option<String>,
    /// The term as text, for a course that is taken or in progress ("Fall 2025"); None when the row has a dropdown or no term.
    pub term: Option<String>,
    /// The term dropdown, for a course that can still be planned.
    pub term_choice: Option<TermChoice>,
    /// The row has a "Select Courses" button (a Course List).
    pub has_select: bool,
    /// The course label opens a "Course Requirements" page (the prerequisite text).
    pub has_requirement: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Degree {
    pub years: Vec<DegreeYear>,
    pub semesters: Vec<DegreeSemester>,
    pub rows: Vec<DegreeRow>,
}

/// One choice inside a Course List page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListChoice {
    pub row: u32,
    /// "or" when the row is an alternative to the one before it, "and" when it is required with it, "" for the first.
    pub joined_by: String,
    pub code: String,
    pub title: String,
    pub category: String,
    pub units: Option<String>,
    /// The term it was taken or planned in, as text ("Fall 2025"), or empty.
    pub term: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CourseList {
    pub name: String,
    pub choices: Vec<ListChoice>,
}

/// "Course Requirements for MATH 1030: Calculus I" and its free text ("MATH1020 OR AMPII Score of 85% is the pre-requisite of ...").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequirementText {
    pub course: String,
    pub title: String,
    pub text: String,
}

/// One result of the Browse Course Catalog search. A course appears once per effective date; [`dedupe_latest`] keeps the newest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogHit {
    /// "INFS 2201"
    pub code: String,
    pub title: String,
    pub crse_id: String,
    pub offer_nbr: String,
    /// "2025-05-09"
    pub effdt: String,
    pub units: String,
}

/// Course detail from Browse Course Catalog (`SSR_CRSE_DTL_FL`), including the Enrollment Requirement text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogDetail {
    pub code: String,
    pub title: String,
    pub units: String,
    pub career: String,
    pub grading: String,
    pub components: Vec<String>,
    pub campus: String,
    pub academic_group: String,
    pub academic_org: String,
    pub requirement: String,
    pub attribute: String,
    pub description: String,
}

/// Keep, for each course code and offer number, only the hit with the latest effective date (order of first appearance is kept).
pub fn dedupe_latest(hits: Vec<CatalogHit>) -> Vec<CatalogHit> {
    let mut out: Vec<CatalogHit> = Vec::new();
    for h in hits {
        match out.iter_mut().find(|o| o.code == h.code && o.offer_nbr == h.offer_nbr) {
            Some(o) => {
                if h.effdt > o.effdt {
                    *o = h;
                }
            }
            None => out.push(h),
        }
    }
    out
}
