//! A fresh seat check, right before a write step.
//!
//! Seats change minute to minute. The page the person chose from may be minutes old, so before staging an option,
//! before the final enrol submit, and when the person presses confirm, the server reads the course again with the
//! connector's `course_sections` (a fresh GET; NOT `seat_status`, which answers from a 30 s memory) and requires, for
//! every chosen class number, an option status of Open and more than zero open seats. Anything else stops the write
//! BEFORE it is sent, with a message that names the class number and the seat text. Nothing retries by itself.
//!
//! This module only reads and judges. It never sends a write, and it never touches the connector's preview, token or
//! phrase gate.
//!
//! Cost: one extra read per course per step. A read is a single GET and was measured at about 2.2 s against the real
//! site (the connector still spaces requests by at least 1 s).

use crate::write::Backend;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::time::{SystemTime, UNIX_EPOCH};

/// What the person chose for one course, and what they saw when they fetched it.
#[derive(Deserialize, Clone, Debug, Default)]
pub struct SeatCheck {
    pub course: String,
    /// PeopleSoft's course and term key. Validated by [`SeatCheck::valid`] before use.
    pub key: Value,
    #[serde(rename = "classNbrs")]
    pub class_nbrs: Vec<String>,
    /// Open seats per class number when the person fetched the course (null when the page showed none).
    #[serde(default)]
    pub seen: HashMap<String, Option<u32>>,
    /// The person is joining a waitlist, not enrolling (UNVERIFIED against a real enrolment: only the catalog wording is known).
    /// Then the option must say Wait List and show waitlist places instead of open seats.
    #[serde(default)]
    pub waitlist: bool,
}

/// A course key from the page, accepted only if every field is short and alphanumeric.
pub fn safe_key(v: &Value) -> Option<Value> {
    let o = v.as_object()?;
    let mut out = serde_json::Map::new();
    for f in ["strm", "crse_id", "crse_offer_nbr", "acad_career", "institution"] {
        let s = o.get(f)?.as_str()?;
        if s.is_empty() || s.len() > 12 || !s.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return None;
        }
        out.insert(f.into(), Value::String(s.into()));
    }
    Some(Value::Object(out))
}

impl SeatCheck {
    /// Refuse anything the page could not have produced, before a single request is made.
    pub fn valid(&self) -> Result<(), String> {
        let safe_course = !self.course.is_empty()
            && self.course.len() <= 24
            && self
                .course
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b' ' | b'-' | b'_' | b'/' | b':'));
        if !safe_course || safe_key(&self.key).is_none() {
            return Err("Selected course has no usable course key. Fetch it from PeopleSoft again.".into());
        }
        if self.class_nbrs.is_empty() || self.class_nbrs.len() > 6 || self.class_nbrs.iter().any(|n| n.is_empty() || n.len() > 10 || !n.bytes().all(|b| b.is_ascii_digit())) {
            return Err(format!("{} has no usable class numbers.", self.course));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Reading {
    pub class_nbr: String,
    pub kind: String,
    pub open: Option<u32>,
    pub total: Option<u32>,
    pub seats_text: String,
    pub option_status: String,
    pub seen_open: Option<u32>,
    pub problem: Option<String>,
}

impl Reading {
    pub fn ok(&self) -> bool {
        self.problem.is_none()
    }
    /// Fewer seats open now than when the person fetched the course.
    pub fn dropped(&self) -> bool {
        matches!((self.seen_open, self.open), (Some(was), Some(now)) if now < was)
    }
}

#[derive(Clone, Debug)]
pub struct Report {
    pub course: String,
    pub term: String,
    pub read_at_ms: u64,
    pub readings: Vec<Reading>,
}

impl Report {
    /// One sentence per class that cannot be enrolled right now, each naming the class number and the seat text.
    pub fn blockers(&self) -> Vec<String> {
        self.readings
            .iter()
            .filter_map(|r| {
                let why = r.problem.as_ref()?;
                let text = if r.seats_text.is_empty() { "no seat text shown".to_string() } else { format!("\"{}\"", r.seats_text) };
                Some(format!("Class {} ({} of {}): {}. Seat text: {}.", r.class_nbr, if r.kind.is_empty() { "class" } else { &r.kind }, self.course, why, text))
            })
            .collect()
    }

    pub fn to_json(&self) -> Value {
        json!({
            "course": self.course,
            "term": self.term,
            "readAtMs": self.read_at_ms,
            "readings": self.readings.iter().map(|r| json!({
                "classNbr": r.class_nbr,
                "kind": r.kind,
                "open": r.open,
                "total": r.total,
                "seatsText": r.seats_text,
                "optionStatus": r.option_status,
                "seenOpen": r.seen_open,
                "dropped": r.dropped(),
                "ok": r.ok(),
            })).collect::<Vec<_>>(),
        })
    }
}

/// PeopleSoft's status for a class option that has a waitlist (seen in the catalog: "Wait List").
pub fn is_wait_status(s: &str) -> bool {
    let t: String = s.chars().filter(|c| c.is_ascii_alphabetic()).collect::<String>().to_ascii_lowercase();
    t == "waitlist" || t == "waitlisted"
}

/// The places left on a waitlist from the seat text "Waitlist Available Places 3 of 10" (seen in the catalog): (available, total).
pub fn wait_places(text: &str) -> Option<(u32, u32)> {
    let l = text.trim().to_ascii_lowercase();
    let rest = l.strip_prefix("waitlist available places")?.trim();
    let (a, b) = rest.split_once(" of ")?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn class_nbrs_of(option: &Value) -> BTreeSet<String> {
    option["components"].as_array().map(|c| c.iter().filter_map(|x| x["class_nbr"].as_str().map(String::from)).collect()).unwrap_or_default()
}

/// Judge one course's freshly read sections against what the person chose. Pure: no I/O.
///
/// Every chosen class number must be found in ONE option that holds all of them (the way they were chosen), that
/// option's status must be Open, and each chosen component must show more than zero open seats. A missing seat figure
/// counts as a failure, not as "probably fine".
pub fn evaluate(check: &SeatCheck, sections: &Value, read_at_ms: u64) -> Report {
    let term = sections["term"].as_str().unwrap_or("").to_string();
    let mut report = Report { course: check.course.clone(), term, read_at_ms, readings: Vec::new() };
    let blank = |n: &str, problem: String| Reading {
        class_nbr: n.to_string(),
        kind: String::new(),
        open: None,
        total: None,
        seats_text: String::new(),
        option_status: String::new(),
        seen_open: check.seen.get(n).copied().flatten(),
        problem: Some(problem),
    };

    if sections["course"].as_str() != Some(check.course.as_str()) {
        let got = sections["course"].as_str().unwrap_or("no course");
        for n in &check.class_nbrs {
            report.readings.push(blank(n, format!("PeopleSoft's page is for {got}, not {}", check.course)));
        }
        return report;
    }

    let options = sections["options"].as_array().cloned().unwrap_or_default();
    let want: BTreeSet<String> = check.class_nbrs.iter().cloned().collect();
    let chosen = options.iter().find(|o| want.is_subset(&class_nbrs_of(o)));

    for n in &check.class_nbrs {
        let Some(option) = chosen else {
            let anywhere = options.iter().any(|o| class_nbrs_of(o).contains(n));
            let why = if anywhere {
                "it is no longer offered together with the other class you chose".to_string()
            } else {
                format!("it can no longer be found in {}", check.course)
            };
            report.readings.push(blank(n, why));
            continue;
        };
        let comp = option["components"].as_array().and_then(|c| c.iter().find(|x| x["class_nbr"].as_str() == Some(n.as_str())));
        let Some(c) = comp else {
            report.readings.push(blank(n, format!("it can no longer be found in {}", check.course)));
            continue;
        };
        let status = option["status"].as_str().unwrap_or("").to_string();
        let open = c["seats"]["open"].as_u64().map(|x| x as u32);
        let total = c["seats"]["total"].as_u64().map(|x| x as u32);
        let seats_text = c["seats_text"].as_str().unwrap_or("");
        let places = wait_places(seats_text);
        let problem = if check.waitlist {
            // Joining a waitlist: the option must say Wait List now, and every component must show a place (or a seat) left.
            if !is_wait_status(&status) {
                Some(format!("it is not on a waitlist now (status \"{}\")", if status.is_empty() { "blank" } else { &status }))
            } else {
                match (places, open) {
                    (Some((0, _)), _) => Some("the waitlist has no places left".to_string()),
                    (Some(_), _) | (None, Some(1..)) => None,
                    _ => Some("PeopleSoft shows no waitlist places figure".to_string()),
                }
            }
        } else if !status.eq_ignore_ascii_case("open") {
            Some(format!("the option's status is \"{}\", not Open", if status.is_empty() { "blank" } else { &status }))
        } else {
            match open {
                None => Some("PeopleSoft shows no seat figure".to_string()),
                Some(0) => Some("it has no open seats".to_string()),
                Some(_) => None,
            }
        };
        let (open, total) = match (check.waitlist, places) { (true, Some((a, t))) => (Some(a), Some(t)), _ => (open, total) };
        report.readings.push(Reading {
            class_nbr: n.clone(),
            kind: c["kind"].as_str().unwrap_or("").to_string(),
            open,
            total,
            seats_text: c["seats_text"].as_str().unwrap_or("").to_string(),
            option_status: status,
            seen_open: check.seen.get(n).copied().flatten(),
            problem,
        });
    }
    report
}

/// Read one course fresh and judge it. A read that fails blocks the write: no answer is not "open".
pub fn read_and_judge(be: &mut dyn Backend, check: &SeatCheck) -> Result<Report, String> {
    check.valid()?;
    let started = std::time::Instant::now();
    let sections = be.tool("course_sections", &check.key).map_err(|m| {
        crate::log::event("seat_read", json!({ "course": check.course, "ms": crate::log::ms_since(started), "ok": false, "error": crate::log::trunc(&m, 200) }));
        format!("Could not read {} to check its seats ({m}). Nothing was sent.", check.course)
    })?;
    let report = evaluate(check, &sections, now_ms());
    crate::log::event(
        "seat_read",
        json!({
            "course": check.course, "ms": crate::log::ms_since(started), "ok": true,
            "readings": report.readings.iter().map(|r| json!({ "class": r.class_nbr, "open": r.open, "status": r.option_status, "blocked": !r.ok() })).collect::<Vec<_>>(),
        }),
    );
    Ok(report)
}

/// Read every chosen course fresh. `Ok` only if every chosen class is open with seats; otherwise the sentences that
/// say which class number is blocked and why, ready to show. One read per course, no retries.
pub fn check_all(be: &mut dyn Backend, checks: &[SeatCheck]) -> Result<Vec<Report>, String> {
    let mut reports = Vec::new();
    let mut blockers: Vec<String> = Vec::new();
    for c in checks {
        let r = read_and_judge(be, c)?;
        blockers.extend(r.blockers());
        reports.push(r);
    }
    if blockers.is_empty() {
        Ok(reports)
    } else {
        Err(format!("Not sent. {} Nothing was changed by this step, and it will not retry by itself.", blockers.join(" ")))
    }
}

pub fn reports_json(reports: &[Report]) -> Value {
    Value::Array(reports.iter().map(Report::to_json).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sections(status: &str, a_seats: Option<u64>, b_seats: Option<u64>) -> Value {
        let seat = |n: Option<u64>| match n {
            Some(x) => json!({ "open": x, "total": 20 }),
            None => Value::Null,
        };
        json!({
            "course": "TEST 1001", "term": "Winter 2099",
            "options": [
                { "status": status, "components": [
                    { "class_nbr": "9001", "kind": "Lecture", "seats": seat(a_seats), "seats_text": format!("Open Seats {} of 20", a_seats.map_or("?".to_string(), |x| x.to_string())) },
                    { "class_nbr": "9002", "kind": "Laboratory", "seats": seat(b_seats), "seats_text": format!("Open Seats {} of 20", b_seats.map_or("?".to_string(), |x| x.to_string())) },
                ] },
                { "status": "Open", "components": [
                    { "class_nbr": "9003", "kind": "Lecture", "seats": seat(Some(5)), "seats_text": "Open Seats 5 of 20" },
                    { "class_nbr": "9004", "kind": "Laboratory", "seats": seat(Some(5)), "seats_text": "Open Seats 5 of 20" },
                ] },
            ],
        })
    }

    fn check(nbrs: &[&str]) -> SeatCheck {
        SeatCheck {
            course: "TEST 1001".into(),
            key: json!({ "strm": "9999", "crse_id": "000001", "crse_offer_nbr": "1", "acad_career": "CRED", "institution": "TESTI" }),
            class_nbrs: nbrs.iter().map(|s| s.to_string()).collect(),
            seen: HashMap::from([("9001".to_string(), Some(20)), ("9002".to_string(), Some(7))]),
            waitlist: false,
        }
    }

    fn wait_sections(status: &str, text: &str) -> Value {
        json!({ "course": "TEST 1001", "term": "Winter 2099", "options": [
            { "status": status, "components": [ { "class_nbr": "9001", "kind": "Lecture", "seats": null, "seats_text": text } ] } ] })
    }

    #[test]
    fn waitlist_wording_is_parsed_exactly_as_seen_in_the_catalog() {
        assert_eq!(wait_places("Waitlist Available Places 3 of 10"), Some((3, 10)));
        assert_eq!(wait_places("Open Seats 3 of 10"), None);
        assert_eq!(wait_places("Closed"), None);
        assert!(is_wait_status("Wait List") && is_wait_status("Waitlisted") && !is_wait_status("Open") && !is_wait_status("Closed"));
    }

    #[test]
    fn joining_a_waitlist_needs_the_wait_list_status_and_a_place() {
        let w = SeatCheck { waitlist: true, ..check(&["9001"]) };
        assert!(evaluate(&w, &wait_sections("Wait List", "Waitlist Available Places 3 of 10"), 0).blockers().is_empty());
        let none = evaluate(&w, &wait_sections("Wait List", "Waitlist Available Places 0 of 10"), 0);
        assert!(none.blockers()[0].contains("no places"), "{:?}", none.blockers());
        let open = evaluate(&w, &wait_sections("Open", "Open Seats 4 of 20"), 0);
        assert!(open.blockers()[0].contains("not on a waitlist"), "{:?}", open.blockers());
        assert!(evaluate(&w, &wait_sections("Closed", "Closed"), 0).blockers()[0].contains("not on a waitlist"));
        assert_eq!(evaluate(&w, &wait_sections("Wait List", "Waitlist Available Places 3 of 10"), 0).readings[0].open, Some(3));
    }

    #[test]
    fn the_real_catalog_capture_of_a_waitlist_class_can_be_joined_and_not_enrolled() {
        // The same capture the page tests read: status "Wait List", both components "Waitlist Available Places 2 of 4", no seat figure.
        let sample: Value = serde_json::from_str(include_str!("../../frontend/src/v2/fixtures/contract-waitlist.json")).unwrap();
        let nbrs: Vec<String> = sample["options"][0]["components"].as_array().unwrap().iter().map(|c| c["class_nbr"].as_str().unwrap().to_string()).collect();
        let base = SeatCheck { course: sample["course"].as_str().unwrap().into(), class_nbrs: nbrs, ..Default::default() };
        let join = SeatCheck { waitlist: true, ..base.clone() };
        assert!(evaluate(&join, &sample, 0).blockers().is_empty());
        assert!(!evaluate(&base, &sample, 0).blockers().is_empty(), "an ordinary enrolment is still refused");
    }

    #[test]
    fn an_ordinary_enrolment_never_goes_to_a_waitlist_class() {
        let r = evaluate(&check(&["9001"]), &wait_sections("Wait List", "Waitlist Available Places 3 of 10"), 0);
        assert!(r.blockers()[0].contains("not Open"), "{:?}", r.blockers());
    }

    #[test]
    fn open_option_with_seats_passes_and_reports_the_numbers() {
        let r = evaluate(&check(&["9001", "9002"]), &sections("Open", Some(20), Some(7)), 5);
        assert!(r.blockers().is_empty());
        assert_eq!(r.read_at_ms, 5);
        assert_eq!(r.readings[1].open, Some(7));
        assert!(!r.readings[1].dropped());
    }

    #[test]
    fn zero_seats_blocks_and_names_the_class_and_the_seat_text() {
        let r = evaluate(&check(&["9001", "9002"]), &sections("Open", Some(20), Some(0)), 0);
        let b = r.blockers();
        assert_eq!(b.len(), 1);
        assert!(b[0].contains("Class 9002") && b[0].contains("Open Seats 0 of 20") && b[0].contains("no open seats"), "{}", b[0]);
    }

    #[test]
    fn a_status_other_than_open_blocks_even_with_seats_showing() {
        for s in ["Closed", "Wait List", ""] {
            let r = evaluate(&check(&["9001", "9002"]), &sections(s, Some(3), Some(3)), 0);
            assert_eq!(r.blockers().len(), 2, "status {s:?}");
            assert!(r.blockers()[0].contains("not Open"));
        }
    }

    #[test]
    fn a_missing_seat_figure_fails_closed() {
        let r = evaluate(&check(&["9001", "9002"]), &sections("Open", Some(3), None), 0);
        let b = r.blockers();
        assert_eq!(b.len(), 1);
        assert!(b[0].contains("Class 9002") && b[0].contains("no seat figure"), "{}", b[0]);
    }

    #[test]
    fn a_vanished_class_number_blocks() {
        let r = evaluate(&check(&["9001", "9999"]), &sections("Open", Some(3), Some(3)), 0);
        let b = r.blockers();
        assert!(b.iter().any(|x| x.contains("Class 9999") && x.contains("can no longer be found")), "{b:?}");
    }

    #[test]
    fn classes_no_longer_offered_together_block() {
        let r = evaluate(&check(&["9001", "9004"]), &sections("Open", Some(3), Some(3)), 0);
        assert!(r.blockers().iter().all(|x| x.contains("no longer offered together")), "{:?}", r.blockers());
        assert_eq!(r.blockers().len(), 2);
    }

    #[test]
    fn a_page_for_another_course_blocks_every_class() {
        let mut s = sections("Open", Some(3), Some(3));
        s["course"] = json!("OTHER 2002");
        assert_eq!(evaluate(&check(&["9001", "9002"]), &s, 0).blockers().len(), 2);
    }

    #[test]
    fn a_drop_since_the_fetch_is_flagged_but_does_not_block() {
        let r = evaluate(&check(&["9001", "9002"]), &sections("Open", Some(20), Some(2)), 0);
        assert!(r.blockers().is_empty());
        assert!(r.readings[1].dropped() && !r.readings[0].dropped());
        assert_eq!(r.to_json()["readings"][1]["dropped"], true);
    }

    #[test]
    fn keys_and_class_numbers_are_validated_before_any_request() {
        assert!(check(&["9001"]).valid().is_ok());
        let mut c = check(&["9001"]);
        c.key["strm"] = json!("1;x");
        assert!(c.valid().is_err());
        assert!(check(&[]).valid().is_err());
        assert!(check(&["90;01"]).valid().is_err());
    }
}
