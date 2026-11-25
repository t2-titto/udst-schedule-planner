//! Tool layer: descriptors and one dispatch function that returns the shared `ToolResult`.
//!
//! `confirm_write` is deliberately NOT dispatchable here. The typed phrase must come from a human at a keyboard
//! (the CLI/TUI reads it); if a model could pass it as an argument, a prompt injection in page text could confirm
//! its own write. Hosts call [`crate::PeopleSoft::confirm`] only with text a person typed.
use crate::connector::PeopleSoft;
use crate::model::CourseKey;
use crate::session::PsError;
use frankie_auth::Http;
use frankie_contracts::{ErrorCode, Origin, SchemaRef, ToolDescriptor, ToolError, ToolResult};
use serde_json::{json, Value};

pub const SOURCE: &str = "peoplesoft";

fn d(
    name: &str,
    desc: &str,
    schema: Value,
    mutates: bool,
    side: Option<&str>,
    limit: Option<&str>,
) -> ToolDescriptor {
    ToolDescriptor {
        id: format!("peoplesoft.{name}"),
        capability: "peoplesoft".into(),
        description: desc.into(),
        input_schema: SchemaRef::Inline(schema),
        mutates,
        when_to_use: None,
        returns: None,
        side_effects: side.map(str::to_string),
        limitations: limit.map(str::to_string),
        examples: vec![],
        output_schema: None,
        error_schema: None,
        contract_version: Some(frankie_contracts::CONTRACT_VERSION.to_string()),
    }
}

fn obj(props: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":props,"required":required,"additionalProperties":false})
}

fn key_schema() -> Value {
    obj(
        json!({"strm":{"type":"string"},"crse_id":{"type":"string"},"crse_offer_nbr":{"type":"string"},"acad_career":{"type":"string"},"institution":{"type":"string"}}),
        &[
            "strm",
            "crse_id",
            "crse_offer_nbr",
            "acad_career",
            "institution",
        ],
    )
}

pub fn descriptors() -> Vec<ToolDescriptor> {
    let none = obj(json!({}), &[]);
    let row = obj(json!({"row":{"type":"integer","minimum":0}}), &["row"]);
    let year_row = obj(json!({"year":{"type":"integer","minimum":0},"index":{"type":"integer","minimum":0}}), &["year", "index"]);
    let nav = Some("Navigates the portal session (page state on the server); changes no data.");
    vec![
        d("landing_tiles", "Home page tiles (generic field dump; layout not mapped).", none.clone(), false, None, Some("Layout not mapped; returns names and text only.")),
        d("notification_count", "Unread notification count.", none.clone(), false, None, None),
        d("manage_classes_menu", "Manage Classes menu entries.", none.clone(), false, None, None),
        d("plan_overview", "Plan & Enrol: degree requirements and the courses available this term.", none.clone(), false, None, None),
        d("plan_expand_requirement", "Expand one requirement row of the overview.", row.clone(), false, nav, None),
        d("degree_tree", "Plan & Enrol degree tree: Years, Semesters and every course row with units and planned or taken term. Reads only (expands the collapsed Years to read them and collapses them again).", none.clone(), false, nav, None),
        d("degree_requirement", "The free-text requirement (prerequisite) of one course row of the degree tree (year from degree_tree and the row's position among that year's rows). Read only.", year_row.clone(), false, nav, None),
        d("degree_course_list", "The choices of one Course List row of the degree tree, with or/and between them (year from degree_tree and the row's position among that year's rows). Read only.", year_row.clone(), false, nav, None),
        d("catalog_search", "Search the Browse Course Catalog by keyword (a course code like INFS 2201, or a subject like INFS): code, title, course id, offer number, effective date and units. Read only.", obj(json!({"keyword":{"type":"string"}}), &["keyword"]), false, nav, None),
        d("catalog_detail", "Course detail from Browse Course Catalog (crse_id, offer_nbr and effdt from catalog_search): units, components, description and Enrollment Requirement text. Read only.", obj(json!({"crse_id":{"type":"string"},"offer_nbr":{"type":"string"},"effdt":{"type":"string"}}), &["crse_id", "offer_nbr", "effdt"]), false, None, None),
        d("catalog_sections", "The classes of a catalog course in one term (course id and offer number from catalog_search, term code such as 1271). Read only. An error means the course has no classes that term.", obj(json!({"crse_id":{"type":"string"},"offer_nbr":{"type":"string"},"strm":{"type":"string"}}), &["crse_id", "offer_nbr", "strm"]), false, None, Some("Seats are as of the request.")),
        d("plan_class_list", "Plan & Enrol class list for the term, with each row's status.", none.clone(), false, nav, None),
        d("plan_select_class", "Open the sections of a class-list row.", row.clone(), false, nav, None),
        d("course_sections", "Sections of a course: class numbers, times, rooms, instructors, seats, status.", key_schema(), false, None, Some("Seats are as of the request; repeated calls inside 30 s are served from memory.")),
        d("seat_status", "Seats of one class number (rate limited by a 30 s memory).", obj(json!({"course":key_schema(),"class_nbr":{"type":"string"}}), &["course", "class_nbr"]), false, None, None),
        d("drop_terms", "Terms offered by the drop pages.", none.clone(), false, None, None),
        d("enrolled_classes", "Classes the student is enrolled in for a term (term row from drop_terms).", row.clone(), false, nav, None),
        d("swap_candidates", "Enrolled classes eligible to swap.", none.clone(), false, None, None),
        d("swap_options", "The options the swap page offers for one enrolled course (candidate row from swap_candidates), with the course key. Read only.", obj(json!({"candidate_row":{"type":"integer","minimum":0}}), &["candidate_row"]), false, None, None),
        d("preview_stage_enrol", "Preview choosing a class option of the last opened course as the pending enrolment choice. Sends nothing that changes data.", obj(json!({"option_row":{"type":"integer","minimum":0}}), &["option_row"]), false, nav, Some("Writes must be armed by the human first.")),
        d("preview_submit_enrol", "Preview submitting the pending Plan & Enrol choices staged in this session.", none.clone(), false, nav, Some("May involve fees; amount unknown until after submit.")),
        d("preview_drop", "Preview dropping one enrolled class by class number.", obj(json!({"term_row":{"type":"integer","minimum":0},"class_nbr":{"type":"string"}}), &["term_row", "class_nbr"]), false, nav, None),
        d("preview_swap_stage", "Preview step one of a swap: class to swap out and the new option.", obj(json!({"candidate_row":{"type":"integer","minimum":0},"option_row":{"type":"integer","minimum":0},"new_class_nbrs":{"type":"array","items":{"type":"string"}}}), &["candidate_row"]), false, nav, None),
        d("preview_swap_commit", "Preview step two of a swap (confirm on the review page).", none.clone(), false, nav, None),
        d("preview_plan_term", "Preview planning one course of the degree tree into a term and saving the plan (year and the row's position among that year's rows, and the term code from the row's own dropdown). Course planning only; never enrols.", obj(json!({"year":{"type":"integer","minimum":0},"index":{"type":"integer","minimum":0},"term_code":{"type":"string"}}), &["year", "index", "term_code"]), false, nav, Some("Writes must be armed by the human first.")),
        d("confirm_write", "Execute a previewed write. HUMAN INPUT ONLY: needs the preview token and the exact phrase typed by the user.", obj(json!({"token":{"type":"string"},"phrase":{"type":"string"}}), &["token", "phrase"]), true, Some("Changes enrolment data on the live system."), Some("Not dispatchable through call(); the host must read the phrase from a person.")),
    ]
}

fn map_err(e: PsError, stage: &str) -> ToolResult {
    let te: ToolError = e.to_contract(stage);
    ToolResult::fail(SOURCE, Origin::Live, te)
}

fn u32_arg(a: &Value, k: &str) -> Result<u32, PsError> {
    a.get(k)
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .ok_or(PsError::InvalidInput("missing or invalid integer argument"))
}
fn str_arg<'a>(a: &'a Value, k: &str) -> Result<&'a str, PsError> {
    a.get(k)
        .and_then(Value::as_str)
        .ok_or(PsError::InvalidInput("missing or invalid string argument"))
}
fn key_arg(a: &Value) -> Result<CourseKey, PsError> {
    Ok(CourseKey {
        strm: str_arg(a, "strm")?.into(),
        crse_id: str_arg(a, "crse_id")?.into(),
        crse_offer_nbr: str_arg(a, "crse_offer_nbr")?.into(),
        acad_career: str_arg(a, "acad_career")?.into(),
        institution: str_arg(a, "institution")?.into(),
    })
}

fn to_value<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

fn preview_value(p: crate::gate::Preview) -> Value {
    json!({
        "plan": to_value(&p.plan),
        "token": p.token,
        "ttl_secs": p.ttl.as_secs(),
        "instruction": format!("A person must type exactly: {}", p.plan.required_phrase),
    })
}

/// Run one tool. `args` is data from a caller that may be a model: it can request reads and previews, never a confirm.
pub fn call<H: Http>(ps: &mut PeopleSoft<H>, tool: &str, args: &Value) -> ToolResult {
    let name = tool.strip_prefix("peoplesoft.").unwrap_or(tool);
    let mut run = || -> Result<Value, PsError> {
        Ok(match name {
            "landing_tiles" => to_value(
                &ps.landing_tiles()?
                    .iter()
                    .map(|f| json!({"name":f.name,"text":f.text}))
                    .collect::<Vec<_>>(),
            ),
            "notification_count" => json!({"count": ps.notification_count()?}),
            "manage_classes_menu" => to_value(
                &ps.manage_classes_menu()?
                    .iter()
                    .map(|f| json!({"name":f.name,"text":f.text}))
                    .collect::<Vec<_>>(),
            ),
            "plan_overview" => to_value(&ps.plan_overview()?),
            "plan_expand_requirement" => {
                to_value(&ps.plan_expand_requirement(u32_arg(args, "row")?)?)
            }
            "degree_tree" => to_value(&ps.degree_tree()?),
            "catalog_sections" => {
                let key = CourseKey {
                    strm: str_arg(args, "strm")?.into(),
                    crse_id: str_arg(args, "crse_id")?.into(),
                    crse_offer_nbr: str_arg(args, "offer_nbr")?.into(),
                    // the student's career and institution as the captured course pages use them
                    acad_career: "CRED".into(),
                    institution: "CNAQA".into(),
                };
                to_value(&ps.course_sections(&key)?)
            }
            "catalog_search" => to_value(&ps.catalog_search(str_arg(args, "keyword")?)?),
            "catalog_detail" => to_value(&ps.catalog_detail(str_arg(args, "crse_id")?, str_arg(args, "offer_nbr")?, str_arg(args, "effdt")?)?),
            "degree_requirement" => to_value(&ps.degree_requirement(u32_arg(args, "year")?, u32_arg(args, "index")?)?),
            "degree_course_list" => to_value(&ps.degree_course_list(u32_arg(args, "year")?, u32_arg(args, "index")?)?),
            "plan_class_list" => to_value(&ps.plan_class_list()?),
            "plan_select_class" => {
                let (k, s) = ps.plan_select_class(u32_arg(args, "row")?)?;
                json!({"course_key": to_value(&k), "sections": to_value(&s)})
            }
            "course_sections" => to_value(&ps.course_sections(&key_arg(args)?)?),
            "seat_status" => {
                let k = key_arg(
                    args.get("course")
                        .ok_or(PsError::InvalidInput("missing course"))?,
                )?;
                let (c, cached) = ps.seat_status(&k, str_arg(args, "class_nbr")?)?;
                json!({"component": to_value(&c), "from_cache": cached})
            }
            "drop_terms" => to_value(
                &ps.drop_terms()?
                    .1
                    .iter()
                    .map(|(r, t)| json!({"row":r,"term":t}))
                    .collect::<Vec<_>>(),
            ),
            "enrolled_classes" => to_value(&ps.enrolled_classes(u32_arg(args, "row")?)?),
            "swap_candidates" => to_value(&ps.swap_candidates()?),
            "swap_options" => {
                let (k, s) = ps.swap_course_options(u32_arg(args, "candidate_row")?)?;
                json!({"course_key": to_value(&k), "sections": to_value(&s)})
            }
            "preview_stage_enrol" => {
                preview_value(ps.preview_stage_enrol(u32_arg(args, "option_row")?)?)
            }
            "preview_submit_enrol" => preview_value(ps.preview_submit_enrol()?),
            "preview_drop" => preview_value(
                ps.preview_drop(u32_arg(args, "term_row")?, str_arg(args, "class_nbr")?)?,
            ),
            "preview_swap_stage" => {
                let row = u32_arg(args, "candidate_row")?;
                match args.get("new_class_nbrs").and_then(|v| v.as_array()) {
                    Some(a) => {
                        let nums: Vec<String> = a.iter().filter_map(|x| x.as_str().map(String::from)).collect();
                        if nums.is_empty() || nums.len() > 6 || nums.iter().any(|n| n.is_empty() || n.len() > 10 || !n.bytes().all(|b| b.is_ascii_digit())) {
                            return Err(PsError::InvalidInput("new_class_nbrs must be 1 to 6 digit strings"));
                        }
                        preview_value(ps.preview_swap_stage_nums(row, &nums)?)
                    }
                    None => preview_value(ps.preview_swap_stage(row, u32_arg(args, "option_row")?)?),
                }
            }
            "preview_swap_commit" => preview_value(ps.preview_swap_commit()?),
            "preview_plan_term" => preview_value(ps.preview_plan_term(u32_arg(args, "year")?, u32_arg(args, "index")?, str_arg(args, "term_code")?)?),
            "confirm_write" => {
                return Err(PsError::WriteRefused(
                    "confirm needs a person at the keyboard; it cannot be called as a tool",
                ))
            }
            _ => return Err(PsError::NotFound("unknown tool")),
        })
    };
    match run() {
        Ok(v) => ToolResult::pass(SOURCE, Origin::Live, v),
        Err(PsError::NotFound("unknown tool")) => ToolResult::unsupported(
            SOURCE,
            Origin::Live,
            ToolError::new(ErrorCode::UnknownOrDisabledTool, "unknown tool"),
        ),
        Err(e) => map_err(e, name),
    }
}
