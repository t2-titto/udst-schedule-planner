//! Rich, stateful in-memory mock PeopleSoft backend for `--demo` mode and UI simulations.
//! Every student identifier, room, instructor, and schedule time is invented; course codes
//! align with public catalog structures so prerequisite and unlock graphs can be exercised live.

use crate::write::{Backend, ConfirmFail};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Debug)]
struct DemoOption {
    row: u32,
    option_no: String,
    status: String,
    components: Vec<DemoComp>,
}

#[derive(Clone, Debug)]
struct DemoComp {
    kind: String,
    class_nbr: String,
    section: String,
    dates: String,
    meetings: Vec<(u32, u32, u32)>, // (day 0=Sun..4=Thu, start_min, end_min)
    schedule_text: String,
    room: String,
    instructor: String,
    open: Option<u32>,
    total: Option<u32>,
    seats_text: String,
}

#[derive(Clone, Debug)]
struct DemoCourse {
    code: String,
    title: String,
    units: String,
    crse_id: String,
    offer_nbr: String,
    effdt: String,
    requirement: String,
    options: Vec<DemoOption>,
}

#[derive(Clone, Debug)]
struct DemoDegreeRow {
    row: u32,
    year: u32,
    semester: Option<u32>,
    label: String,
    code: String,
    title: String,
    category: String,
    units: Option<String>,
    term: Option<String>,
    selected_code: Option<String>,
    has_select: bool,
    has_requirement: bool,
}

#[derive(Clone, Debug)]
enum DemoToken {
    Drop { class_nbr: String },
    SwapStage { course: String, new_nbrs: Vec<String> },
    SwapCommit,
    PlanTerm { year: u32, index: u32, term_code: String, code: String },
    StageEnrol { option_row: u32 },
    SubmitEnrol,
}

pub struct DemoState {
    pub armed: bool,
    pub delay_ms: u64,
    courses: BTreeMap<String, DemoCourse>,
    /// Enrolled in Winter 2027: code -> (option_no, waitlisted)
    enrolled: BTreeMap<String, (String, bool)>,
    /// Degree rows
    degree: Vec<DemoDegreeRow>,
    /// Issued preview tokens
    tokens: BTreeMap<String, (DemoToken, String)>,
    next_tok: u64,
}

fn day_name(d: u32) -> &'static str {
    match d {
        0 => "Sunday",
        1 => "Monday",
        2 => "Tuesday",
        3 => "Wednesday",
        4 => "Thursday",
        5 => "Friday",
        _ => "Saturday",
    }
}

fn fmt_ps_time(m: u32) -> String {
    let h24 = m / 60;
    let min = m % 60;
    let ap = if h24 >= 12 { "PM" } else { "AM" };
    let h12 = if h24 % 12 == 0 { 12 } else { h24 % 12 };
    format!("{h12}:{min:02}{ap}")
}

fn sched_text(meetings: &[(u32, u32, u32)]) -> String {
    meetings
        .iter()
        .map(|&(d, s, e)| format!("{} {} to {}", day_name(d), fmt_ps_time(s), fmt_ps_time(e)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn comp(
    kind: &str,
    class_nbr: &str,
    section: &str,
    meetings: Vec<(u32, u32, u32)>,
    room: &str,
    instructor: &str,
    open: Option<u32>,
    total: Option<u32>,
    seats_text: &str,
) -> DemoComp {
    DemoComp {
        kind: kind.into(),
        class_nbr: class_nbr.into(),
        section: section.into(),
        dates: "05/01/2027 - 14/04/2027".into(),
        schedule_text: sched_text(&meetings),
        meetings,
        room: room.into(),
        instructor: instructor.into(),
        open,
        total,
        seats_text: seats_text.into(),
    }
}

fn open_comp(kind: &str, class_nbr: &str, section: &str, meetings: Vec<(u32, u32, u32)>, room: &str, instructor: &str, open: u32, total: u32) -> DemoComp {
    comp(kind, class_nbr, section, meetings, room, instructor, Some(open), Some(total), &format!("Open Seats {open} of {total}"))
}

fn wait_comp(kind: &str, class_nbr: &str, section: &str, meetings: Vec<(u32, u32, u32)>, room: &str, instructor: &str, left: u32, total: u32) -> DemoComp {
    comp(kind, class_nbr, section, meetings, room, instructor, None, None, &format!("Waitlist Available Places {left} of {total}"))
}

fn closed_comp(kind: &str, class_nbr: &str, section: &str, meetings: Vec<(u32, u32, u32)>, room: &str, instructor: &str) -> DemoComp {
    comp(kind, class_nbr, section, meetings, room, instructor, Some(0), Some(25), "Closed")
}

fn term_options() -> Vec<(&'static str, &'static str)> {
    vec![
        ("1271", "Winter 2027"),
        ("1275", "Spring 2027"),
        ("1278", "Fall 2027"),
        ("1281", "Winter 2028"),
        ("1285", "Spring 2028"),
    ]
}

fn term_label_for(code: &str) -> Option<&'static str> {
    term_options().into_iter().find(|(c, _)| *c == code).map(|(_, l)| l)
}

impl Default for DemoState {
    fn default() -> Self {
        let mut courses = BTreeMap::new();
        let mut add = |c: DemoCourse| {
            courses.insert(c.code.clone(), c);
        };

        // Enrolled courses
        add(DemoCourse {
            code: "DACS 2202".into(),
            title: "Computer Networks".into(),
            units: "3.00".into(),
            crse_id: "005947".into(),
            offer_nbr: "1".into(),
            effdt: "2024-09-01".into(),
            requirement: "Prerequisite(s): DACS 2101 and INFS 1201.".into(),
            options: vec![
                DemoOption {
                    row: 0,
                    option_no: "1".into(),
                    status: "Open".into(),
                    components: vec![
                        open_comp("Lecture", "3101", "Sec 01", vec![(1, 570, 660), (3, 570, 660)], "B1-204", "Prof. D. Kim", 5, 25),
                        open_comp("Laboratory", "3102", "Sec 01L", vec![(2, 570, 660)], "Lab 102", "S. Ali", 5, 25),
                    ],
                },
                DemoOption {
                    row: 1,
                    option_no: "2".into(),
                    status: "Open".into(),
                    components: vec![
                        // Same lecture, different lab -> tests lab-only swap protection!
                        open_comp("Lecture", "3101", "Sec 01", vec![(1, 570, 660), (3, 570, 660)], "B1-204", "Prof. D. Kim", 5, 25),
                        open_comp("Laboratory", "3103", "Sec 02L", vec![(0, 600, 690)], "Lab 104", "S. Ali", 9, 25),
                    ],
                },
                DemoOption {
                    row: 2,
                    option_no: "3".into(),
                    status: "Open".into(),
                    components: vec![
                        open_comp("Lecture", "3104", "Sec 02", vec![(0, 840, 930), (2, 840, 930)], "B1-208", "Dr. T. Vance", 11, 25),
                        open_comp("Laboratory", "3105", "Sec 03L", vec![(1, 840, 930)], "Lab 105", "R. Noor", 11, 25),
                    ],
                },
            ],
        });

        add(DemoCourse {
            code: "DSAI 2201".into(),
            title: "Introduction to Data Science".into(),
            units: "3.00".into(),
            crse_id: "004709".into(),
            offer_nbr: "1".into(),
            effdt: "2024-09-01".into(),
            requirement: "Prerequisite(s): INFS 1201 and MATH 1040.".into(),
            options: vec![
                DemoOption {
                    row: 0,
                    option_no: "1".into(),
                    status: "Open".into(),
                    components: vec![
                        open_comp("Lecture", "3201", "Sec 01", vec![(1, 840, 930), (3, 840, 930)], "B2-110", "Prof. A. Rao", 4, 25),
                    ],
                },
                DemoOption {
                    row: 1,
                    option_no: "2".into(),
                    status: "Open".into(),
                    components: vec![
                        open_comp("Lecture", "3202", "Sec 02", vec![(0, 570, 660), (2, 570, 660)], "B2-112", "Dr. H. Lin", 7, 25),
                    ],
                },
            ],
        });

        add(DemoCourse {
            code: "GARC 2001".into(),
            title: "Research & Ethics".into(),
            units: "3.00".into(),
            crse_id: "004910".into(),
            offer_nbr: "1".into(),
            effdt: "2024-09-01".into(),
            requirement: "".into(),
            options: vec![
                DemoOption {
                    row: 0,
                    option_no: "1".into(),
                    status: "Open".into(),
                    components: vec![
                        open_comp("Lecture", "3301", "Sec 01", vec![(0, 690, 780), (2, 690, 780)], "B3-301", "Dr. S. Lee", 6, 30),
                    ],
                },
                DemoOption {
                    row: 1,
                    option_no: "2".into(),
                    status: "Open".into(),
                    components: vec![
                        open_comp("Lecture", "3302", "Sec 02", vec![(1, 690, 780), (3, 690, 780)], "B3-302", "Dr. S. Lee", 14, 30),
                    ],
                },
            ],
        });

        // Planned in Winter 2027 (Classes available)
        add(DemoCourse {
            code: "INFS 2201".into(),
            title: "Database Management Systems".into(),
            units: "3.00".into(),
            crse_id: "004601".into(),
            offer_nbr: "1".into(),
            effdt: "2024-09-01".into(),
            requirement: "Prerequisite(s): INFS 1201.".into(),
            options: vec![
                DemoOption {
                    row: 0,
                    option_no: "1".into(),
                    status: "Open".into(),
                    components: vec![
                        open_comp("Lecture", "3401", "Sec 01", vec![(1, 690, 780), (3, 690, 780)], "B1-101", "Prof. L. Chen", 10, 25),
                        open_comp("Laboratory", "3402", "Sec 01L", vec![(2, 840, 930)], "Lab 201", "S. Ali", 10, 25),
                    ],
                },
                DemoOption {
                    row: 1,
                    option_no: "2".into(),
                    status: "Open".into(),
                    components: vec![
                        open_comp("Lecture", "3403", "Sec 02", vec![(2, 600, 690)], "B1-105", "Dr. A. Haddad", 8, 25),
                        open_comp("Laboratory", "3404", "Sec 02L", vec![(3, 690, 780)], "Lab 202", "S. Ali", 8, 25),
                    ],
                },
                // Same exact Sun 13:00-14:30 time slot, two different teachers -> tests same-time multi-teacher picker!
                DemoOption {
                    row: 2,
                    option_no: "3".into(),
                    status: "Open".into(),
                    components: vec![
                        open_comp("Lecture", "3405", "Sec 04", vec![(0, 780, 870)], "B1-109", "Dr. M. Rossi", 3, 25),
                        open_comp("Laboratory", "3406", "Sec 04L", vec![(2, 600, 690)], "Lab 203", "S. Ali", 5, 25),
                    ],
                },
                DemoOption {
                    row: 3,
                    option_no: "4".into(),
                    status: "Open".into(),
                    components: vec![
                        open_comp("Lecture", "3407", "Sec 07", vec![(0, 780, 870)], "B1-112", "Dr. L. Karim", 5, 25),
                        open_comp("Laboratory", "3408", "Sec 07L", vec![(2, 600, 690)], "Lab 203", "R. Noor", 6, 25),
                    ],
                },
                DemoOption {
                    row: 4,
                    option_no: "5".into(),
                    status: "Wait List".into(),
                    components: vec![
                        wait_comp("Lecture", "3409", "Sec 06", vec![(4, 780, 870)], "B1-115", "Dr. S. Karim", 2, 5),
                        wait_comp("Laboratory", "3410", "Sec 06L", vec![(4, 900, 990)], "Lab 204", "S. Ali", 2, 5),
                    ],
                },
                DemoOption {
                    row: 5,
                    option_no: "6".into(),
                    status: "Closed".into(),
                    components: vec![
                        closed_comp("Lecture", "3411", "Sec 05", vec![(1, 570, 660)], "B1-120", "Dr. K. Mansour"),
                        closed_comp("Laboratory", "3412", "Sec 05L", vec![(3, 570, 660)], "Lab 205", "S. Ali"),
                    ],
                },
            ],
        });

        add(DemoCourse {
            code: "INFS 3102".into(),
            title: "Enterprise Systems".into(),
            units: "3.00".into(),
            crse_id: "004614".into(),
            offer_nbr: "1".into(),
            effdt: "2024-09-01".into(),
            requirement: "Prerequisite(s): INFS 2101.".into(),
            options: vec![
                DemoOption {
                    row: 0,
                    option_no: "1".into(),
                    status: "Open".into(),
                    components: vec![open_comp("Lecture", "3501", "Sec 01", vec![(1, 960, 1050), (3, 960, 1050)], "B2-204", "Prof. E. Miller", 9, 25)],
                },
                DemoOption {
                    row: 1,
                    option_no: "2".into(),
                    status: "Open".into(),
                    components: vec![open_comp("Lecture", "3502", "Sec 02", vec![(0, 600, 690), (2, 600, 690)], "Online", "Dr. R. Malik", 18, 40)],
                },
                DemoOption {
                    row: 2,
                    option_no: "3".into(),
                    status: "Wait List".into(),
                    components: vec![wait_comp("Lecture", "3503", "Sec 03", vec![(4, 600, 690)], "B2-205", "Dr. N. Haddad", 3, 10)],
                },
                DemoOption {
                    row: 3,
                    option_no: "4".into(),
                    status: "Open".into(),
                    components: vec![open_comp("Lecture", "3504", "Sec 04", vec![(0, 1200, 1320), (2, 1200, 1320)], "B2-209", "Prof. H. Al-Mansoor", 11, 25)],
                },
            ],
        });

        // Ready to Add (compatible degree courses not yet in Winter 2027)
        add(DemoCourse {
            code: "SOFT 2301".into(),
            title: "Software Engineering".into(),
            units: "3.00".into(),
            crse_id: "005567".into(),
            offer_nbr: "1".into(),
            effdt: "2024-09-01".into(),
            requirement: "Prerequisite(s): INFS 1201.".into(),
            options: vec![
                DemoOption {
                    row: 0,
                    option_no: "1".into(),
                    status: "Open".into(),
                    components: vec![open_comp("Lecture", "3601", "Sec 01", vec![(0, 510, 600), (2, 510, 600)], "B1-301", "Dr. N. Salem", 6, 25)],
                },
                DemoOption {
                    row: 1,
                    option_no: "2".into(),
                    status: "Open".into(),
                    components: vec![open_comp("Lecture", "3602", "Sec 02", vec![(1, 600, 690), (3, 600, 690)], "B1-302", "Prof. O. Zayd", 12, 25)],
                },
                DemoOption {
                    row: 2,
                    option_no: "3".into(),
                    status: "Open".into(),
                    components: vec![open_comp("Lecture", "3603", "Sec 03", vec![(4, 510, 600)], "B1-303", "Dr. F. Tariq", 4, 25)],
                },
            ],
        });

        add(DemoCourse {
            code: "BIOL 1110".into(),
            title: "Human Anatomy & Physiology".into(),
            units: "3.00".into(),
            crse_id: "005184".into(),
            offer_nbr: "1".into(),
            effdt: "2024-09-01".into(),
            requirement: "Co-requisite(s): BIOL 1111.".into(),
            options: vec![
                DemoOption {
                    row: 0,
                    option_no: "1".into(),
                    status: "Open".into(),
                    components: vec![open_comp("Lecture", "3701", "Sec 01", vec![(0, 480, 570), (4, 480, 570)], "H1-101", "Dr. M. Farah", 11, 30)],
                },
                DemoOption {
                    row: 1,
                    option_no: "2".into(),
                    status: "Open".into(),
                    components: vec![open_comp("Lecture", "3702", "Sec 02", vec![(1, 780, 870), (3, 780, 870)], "H1-102", "Dr. M. Farah", 5, 30)],
                },
            ],
        });

        add(DemoCourse {
            code: "BIOL 1111".into(),
            title: "Anatomy Lab".into(),
            units: "1.00".into(),
            crse_id: "010208".into(),
            offer_nbr: "1".into(),
            effdt: "2024-09-01".into(),
            requirement: "Co-requisite(s): BIOL 1110.".into(),
            options: vec![
                DemoOption {
                    row: 0,
                    option_no: "1".into(),
                    status: "Open".into(),
                    components: vec![open_comp("Laboratory", "3711", "Sec 01L", vec![(3, 480, 600)], "H1-Lab1", "Dr. M. Farah", 9, 20)],
                },
            ],
        });

        add(DemoCourse {
            code: "STAT 2100".into(),
            title: "Applied Statistics".into(),
            units: "3.00".into(),
            crse_id: "010209".into(),
            offer_nbr: "1".into(),
            effdt: "2024-09-01".into(),
            requirement: "Prerequisite(s): MATH 1010.".into(),
            options: vec![
                DemoOption {
                    row: 0,
                    option_no: "1".into(),
                    status: "Open".into(),
                    components: vec![open_comp("Lecture", "3801", "Sec 01", vec![(0, 900, 990), (2, 900, 990)], "B3-101", "Prof. Y. Nasser", 8, 25)],
                },
                DemoOption {
                    row: 1,
                    option_no: "2".into(),
                    status: "Closed".into(),
                    components: vec![closed_comp("Lecture", "3802", "Sec 02", vec![(1, 690, 780)], "B3-102", "Prof. Y. Nasser")],
                },
            ],
        });

        add(DemoCourse {
            code: "MATH 2040".into(),
            title: "Discrete Mathematics".into(),
            units: "3.00".into(),
            crse_id: "010210".into(),
            offer_nbr: "1".into(),
            effdt: "2024-09-01".into(),
            requirement: "Prerequisite(s): MATH 1020.".into(),
            options: vec![
                // 0 open -> tests "No open classes right now" toast!
                DemoOption {
                    row: 0,
                    option_no: "1".into(),
                    status: "Closed".into(),
                    components: vec![closed_comp("Lecture", "3901", "Sec 01", vec![(1, 600, 690), (3, 600, 690)], "B4-101", "Dr. Z. Qureshi")],
                },
            ],
        });

        add(DemoCourse {
            code: "DACS 3101".into(),
            title: "Applied Cryptography".into(),
            units: "3.00".into(),
            crse_id: "004716".into(),
            offer_nbr: "1".into(),
            effdt: "2024-09-01".into(),
            requirement: "Prerequisite(s): DACS 2101 and MATH 1050.".into(),
            options: vec![
                DemoOption {
                    row: 0,
                    option_no: "1".into(),
                    status: "Open".into(),
                    components: vec![open_comp("Lecture", "4001", "Sec 01", vec![(2, 720, 810), (4, 720, 810)], "B1-401", "Dr. A. Mansoor", 7, 25)],
                },
            ],
        });

        add(DemoCourse {
            code: "COMM 3010".into(),
            title: "Technical Communication".into(),
            units: "3.00".into(),
            crse_id: "004447".into(),
            offer_nbr: "1".into(),
            effdt: "2024-09-01".into(),
            requirement: "Prerequisite(s): COMM 1020. Min 75 credits.".into(),
            options: vec![
                DemoOption {
                    row: 0,
                    option_no: "1".into(),
                    status: "Open".into(),
                    components: vec![open_comp("Lecture", "4101", "Sec 01", vec![(0, 600, 690), (2, 600, 690)], "B2-301", "Prof. C. Wright", 14, 30)],
                },
            ],
        });

        add(DemoCourse {
            code: "GARC 2004".into(),
            title: "Global Sustainability".into(),
            units: "3.00".into(),
            crse_id: "006231".into(),
            offer_nbr: "1".into(),
            effdt: "2024-09-01".into(),
            requirement: "Prerequisite(s): COMM 1020.".into(),
            options: vec![], // Not offered this semester
        });

        add(DemoCourse {
            code: "DACS 4105".into(),
            title: "Offensive Security".into(),
            units: "3.00".into(),
            crse_id: "004753".into(),
            offer_nbr: "1".into(),
            effdt: "2024-09-01".into(),
            requirement: "Prerequisite(s): DACS 3201.".into(),
            options: vec![
                DemoOption {
                    row: 0,
                    option_no: "1".into(),
                    status: "Open".into(),
                    components: vec![open_comp("Lecture", "4201", "Sec 01", vec![(1, 780, 870)], "B1-501", "Dr. V. Reed", 10, 20)],
                },
            ],
        });

        add(DemoCourse {
            code: "DACS 3201".into(),
            title: "Systems Security".into(),
            units: "3.00".into(),
            crse_id: "004718".into(),
            offer_nbr: "1".into(),
            effdt: "2024-09-01".into(),
            requirement: "Prerequisite(s): INFT 3101 and DACS 2201.".into(),
            options: vec![
                DemoOption {
                    row: 0,
                    option_no: "1".into(),
                    status: "Open".into(),
                    components: vec![open_comp("Lecture", "4301", "Sec 01", vec![(1, 1080, 1170), (3, 1080, 1170)], "B1-410", "Dr. S. Khan", 9, 25)],
                },
            ],
        });

        add(DemoCourse {
            code: "GARC 2003".into(),
            title: "Data Analytics Foundations".into(),
            units: "3.00".into(),
            crse_id: "006229".into(),
            offer_nbr: "1".into(),
            effdt: "2024-09-01".into(),
            requirement: "Prerequisite(s): COMM 1020.".into(),
            options: vec![
                DemoOption {
                    row: 0,
                    option_no: "1".into(),
                    status: "Open".into(),
                    components: vec![open_comp("Lecture", "4401", "Sec 01", vec![(2, 900, 990), (4, 900, 990)], "B3-201", "Dr. R. Farah", 13, 30)],
                },
            ],
        });

        add(DemoCourse {
            code: "COMM 2010".into(),
            title: "Intercultural Communication".into(),
            units: "3.00".into(),
            crse_id: "004419".into(),
            offer_nbr: "1".into(),
            effdt: "2024-09-01".into(),
            requirement: "Prerequisite(s): COMM 1010.".into(),
            options: vec![
                DemoOption {
                    row: 0,
                    option_no: "1".into(),
                    status: "Open".into(),
                    components: vec![open_comp("Lecture", "4501", "Sec 01", vec![(4, 510, 600)], "B2-104", "Prof. M. Zayed", 16, 30)],
                },
            ],
        });

        let enrolled = BTreeMap::from([
            ("DACS 2202".into(), ("1".into(), false)),
            ("DSAI 2201".into(), ("1".into(), false)),
            ("GARC 2001".into(), ("1".into(), false)),
        ]);

        // Degree tree across 4 academic years
        let mut degree = Vec::new();
        let mut r_idx = 0u32;
        let completed_y1_f = ["INFS 1101", "MATH 1010", "COMM 1010", "CHEM 1010"];
        for code in completed_y1_f {
            degree.push(DemoDegreeRow {
                row: r_idx, year: 0, semester: Some(0), label: code.into(), code: code.into(),
                title: format!("{code} Foundations"), category: "Required".into(), units: Some("3.00".into()),
                term: Some("Fall 2025".into()), selected_code: None, has_select: false, has_requirement: false,
            });
            r_idx += 1;
        }
        let completed_y1_w = ["INFS 1201", "MATH 1020", "COMM 1020", "BIOL 1030"];
        for code in completed_y1_w {
            degree.push(DemoDegreeRow {
                row: r_idx, year: 0, semester: Some(1), label: code.into(), code: code.into(),
                title: format!("{code} Core"), category: "Required".into(), units: Some("3.00".into()),
                term: Some("Winter 2026".into()), selected_code: None, has_select: false, has_requirement: false,
            });
            r_idx += 1;
        }
        let completed_y2_f = ["DACS 2101", "DACS 2201", "INFT 2101", "INFT 3101", "INFS 2101", "MATH 1040", "MATH 1050"];
        for code in completed_y2_f {
            degree.push(DemoDegreeRow {
                row: r_idx, year: 1, semester: Some(3), label: code.into(), code: code.into(),
                title: format!("{code} Intermediate"), category: "Required".into(), units: Some("3.00".into()),
                term: Some("Fall 2026".into()), selected_code: None, has_select: false, has_requirement: false,
            });
            r_idx += 1;
        }

        // Year 1, Sem 4 (Winter 2027 - open term): 3 enrolled + 2 planned (15 cr)
        for (code, title, planned) in [
            ("DACS 2202", "Computer Networks", false),
            ("DSAI 2201", "Introduction to Data Science", false),
            ("GARC 2001", "Research & Ethics", false),
            ("INFS 2201", "Database Management Systems", true),
            ("INFS 3102", "Enterprise Systems", true),
        ] {
            degree.push(DemoDegreeRow {
                row: r_idx, year: 1, semester: Some(4), label: format!("{code}: {title}"), code: code.into(),
                title: title.into(), category: "Required".into(), units: Some("3.00".into()),
                term: if planned { None } else { Some("Winter 2027".into()) },
                selected_code: if planned { Some("1271".into()) } else { None },
                has_select: false, has_requirement: true,
            });
            r_idx += 1;
        }

        // Year 1, Sem 5 (Spring 2027): 2 courses (6 / 12 cr)
        for (code, title) in [("DACS 3101", "Applied Cryptography"), ("COMM 3010", "Technical Communication")] {
            degree.push(DemoDegreeRow {
                row: r_idx, year: 1, semester: Some(5), label: format!("{code}: {title}"), code: code.into(),
                title: title.into(), category: "Required".into(), units: Some("3.00".into()),
                term: None, selected_code: Some("1275".into()), has_select: false, has_requirement: true,
            });
            r_idx += 1;
        }

        // Year 2, Sem 6 (Fall 2027): ready-to-add courses!
        for (code, title, units) in [
            ("SOFT 2301", "Software Engineering", "3.00"),
            ("BIOL 1110", "Human Anatomy & Physiology", "3.00"),
            ("BIOL 1111", "Anatomy Lab", "1.00"),
            ("STAT 2100", "Applied Statistics", "3.00"),
            ("MATH 2040", "Discrete Mathematics", "3.00"),
            ("DACS 3201", "Systems Security", "3.00"),
            ("GARC 2004", "Global Sustainability", "3.00"),
        ] {
            degree.push(DemoDegreeRow {
                row: r_idx, year: 2, semester: Some(6), label: format!("{code}: {title}"), code: code.into(),
                title: title.into(), category: "Required".into(), units: Some(units.into()),
                term: None, selected_code: Some("1278".into()), has_select: false, has_requirement: true,
            });
            r_idx += 1;
        }
        // Course List group row in Year 2
        degree.push(DemoDegreeRow {
            row: r_idx, year: 2, semester: Some(6), label: "Semester 6 - Open Elective List".into(),
            code: "".into(), title: "".into(), category: "".into(), units: None, term: None,
            selected_code: None, has_select: true, has_requirement: false,
        });
        r_idx += 1;

        // Year 3, Sem 7 (Winter 2028)
        degree.push(DemoDegreeRow {
            row: r_idx, year: 3, semester: Some(7), label: "DACS 4105: Offensive Security".into(), code: "DACS 4105".into(),
            title: "Offensive Security".into(), category: "Required".into(), units: Some("3.00".into()),
            term: None, selected_code: Some("1281".into()), has_select: false, has_requirement: true,
        });

        let delay_ms = std::env::var("PLANNER_DEMO_DELAY_MS").ok().and_then(|s| s.parse().ok()).unwrap_or(0);

        Self {
            armed: false,
            delay_ms,
            courses,
            enrolled,
            degree,
            tokens: BTreeMap::new(),
            next_tok: 1,
        }
    }
}

impl DemoState {
    pub fn new() -> Self {
        Self::default()
    }

    fn issue(&mut self, kind: &str, term: &str, class_numbers: Vec<String>, summary: &str, tok: DemoToken) -> Value {
        let id = format!("demo-tok-{}", self.next_tok);
        self.next_tok += 1;
        let phrase = format!("CONFIRM {} {}", kind.to_uppercase(), class_numbers.join(" "));
        self.tokens.insert(id.clone(), (tok, phrase.clone()));
        json!({
            "token": id,
            "ttl_secs": 600,
            "plan": {
                "kind": kind,
                "term": term,
                "class_numbers": class_numbers,
                "summary": summary,
                "steps": [],
                "warnings": [],
                "required_phrase": phrase,
            }
        })
    }

    fn sections_json(&self, code: &str) -> Result<Value, String> {
        let c = self.courses.get(code).ok_or_else(|| format!("{code} not found in demo catalog."))?;
        let opts: Vec<Value> = c
            .options
            .iter()
            .map(|o| {
                let comps: Vec<Value> = o
                    .components
                    .iter()
                    .map(|cm| {
                        let seats = match (cm.open, cm.total) {
                            (Some(open), Some(total)) => json!({ "open": open, "total": total }),
                            _ => Value::Null,
                        };
                        let meetings: Vec<Value> = cm
                            .meetings
                            .iter()
                            .map(|&(day, start, end)| json!({ "day": day, "start": start, "end": end }))
                            .collect();
                        json!({
                            "kind": cm.kind,
                            "class_nbr": cm.class_nbr,
                            "section": cm.section,
                            "dates": cm.dates,
                            "meetings": meetings,
                            "schedule_text": cm.schedule_text,
                            "room": cm.room,
                            "instructor": cm.instructor,
                            "seats": seats,
                            "seats_text": cm.seats_text,
                        })
                    })
                    .collect();
                json!({
                    "row": o.row,
                    "option_no": o.option_no,
                    "status": o.status,
                    "session": "Regular Academic Session",
                    "components": comps,
                })
            })
            .collect();
        Ok(json!({
            "term": "Winter 2027",
            "course": c.code,
            "title": c.title,
            "description": format!("{} ({})", c.title, c.code),
            "units": c.units,
            "grading": "60 Percent and Letter Grade",
            "components_offered": ["Lecture", "Laboratory"],
            "career": "Credit",
            "options_text": format!("{} options", opts.len()),
            "options": opts,
        }))
    }

    fn course_key(&self, code: &str) -> Value {
        let c = self.courses.get(code);
        json!({
            "strm": "1271",
            "crse_id": c.map(|x| x.crse_id.as_str()).unwrap_or("000000"),
            "crse_offer_nbr": c.map(|x| x.offer_nbr.as_str()).unwrap_or("1"),
            "acad_career": "CRED",
            "institution": "CNAQA",
        })
    }

    fn class_list_json(&self, staged_enrols: &BTreeMap<String, String>) -> Value {
        let mut rows = Vec::new();
        let mut row_num = 0u32;
        for dr in &self.degree {
            if dr.code.is_empty() {
                continue;
            }
            let term = dr
                .selected_code
                .as_deref()
                .and_then(term_label_for)
                .map(str::to_string)
                .or_else(|| dr.term.clone())
                .unwrap_or_default();
            if term != "Winter 2027" && !self.enrolled.contains_key(&dr.code) {
                continue;
            }
            let (status, summary) = if let Some((opt_no, wl)) = self.enrolled.get(&dr.code) {
                let text = self
                    .courses
                    .get(&dr.code)
                    .and_then(|c| c.options.iter().find(|o| &o.option_no == opt_no))
                    .map(|o| {
                        o.components
                            .iter()
                            .map(|cm| format!("{} - Class {} - {} {}", cm.kind, cm.class_nbr, cm.section, cm.schedule_text))
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default();
                (if *wl { "Waitlisted" } else { "Enrolled" }, text)
            } else if let Some(staged_no) = staged_enrols.get(&dr.code) {
                let text = self
                    .courses
                    .get(&dr.code)
                    .and_then(|c| c.options.iter().find(|o| &o.option_no == staged_no))
                    .map(|o| o.components.iter().map(|cm| cm.schedule_text.clone()).collect::<Vec<_>>().join(" "))
                    .unwrap_or_default();
                ("Classes available", text)
            } else {
                let has_opts = self.courses.get(&dr.code).is_some_and(|c| !c.options.is_empty());
                (if has_opts { "Classes available" } else { "No classes available" }, String::new())
            };
            rows.push(json!({
                "row": row_num,
                "course_label": format!("{}: {}", dr.code, dr.title),
                "term": "Winter 2027",
                "status": status,
                "schedule_summary": summary,
            }));
            row_num += 1;
        }
        json!({
            "term_label": "Winter 2027",
            "rows": rows,
            "can_submit": !staged_enrols.is_empty(),
        })
    }

    fn enrolled_classes_json(&self) -> Value {
        let mut classes = Vec::new();
        let mut row = 0u32;
        for (code, (opt_no, wl)) in &self.enrolled {
            let Some(c) = self.courses.get(code) else { continue };
            let Some(opt) = c.options.iter().find(|o| &o.option_no == opt_no) else { continue };
            for (ci, cm) in opt.components.iter().enumerate() {
                classes.push(json!({
                    "row": row,
                    "class_nbr": cm.class_nbr,
                    "component": cm.kind,
                    "section": cm.section,
                    "course": format!("{} {}", c.code, c.title),
                    "schedule": [cm.schedule_text],
                    "rooms": [cm.room],
                    "instructors": [cm.instructor],
                    "units": if ci == 0 { Value::String(c.units.clone()) } else { Value::Null },
                    "status": if *wl { "Waitlisted" } else { "Enrolled" },
                }));
                row += 1;
            }
        }
        json!({ "classes": classes })
    }

    fn degree_tree_json(&self) -> Value {
        let years = json!([
            { "index": 0, "name": "Year 1", "summary": "Completed", "state": "collapsed" },
            { "index": 1, "name": "Year 2", "summary": "In progress", "state": "collapsed" },
            { "index": 2, "name": "Year 3", "summary": "Planned", "state": "collapsed" },
            { "index": 3, "name": "Year 4", "summary": "Planned", "state": "collapsed" },
        ]);
        let semesters = json!([
            { "index": 0, "year": 0, "name": "Semester 1", "summary": "12 credits" },
            { "index": 1, "year": 0, "name": "Semester 2", "summary": "12 credits" },
            { "index": 3, "year": 1, "name": "Semester 3", "summary": "12 credits" },
            { "index": 4, "year": 1, "name": "Semester 4", "summary": "15 credits" },
            { "index": 5, "year": 1, "name": "Semester 5", "summary": "6 credits" },
            { "index": 6, "year": 2, "name": "Semester 6", "summary": "16 credits" },
            { "index": 7, "year": 3, "name": "Semester 7", "summary": "3 credits" },
        ]);
        let opts_json: Vec<Value> = term_options().into_iter().map(|(c, l)| json!({ "code": c, "label": l })).collect();
        let rows: Vec<Value> = self
            .degree
            .iter()
            .map(|r| {
                let term_choice = r.selected_code.as_ref().map(|sel| {
                    json!({
                        "selected": sel,
                        "options": opts_json,
                    })
                });
                json!({
                    "row": r.row,
                    "year": r.year,
                    "semester": r.semester,
                    "label": r.label,
                    "joined_by_and": true,
                    "code": r.code,
                    "title": r.title,
                    "category": r.category,
                    "units": r.units,
                    "term": r.term,
                    "term_choice": term_choice,
                    "has_select": r.has_select,
                    "has_requirement": r.has_requirement,
                })
            })
            .collect();
        json!({ "years": years, "semesters": semesters, "rows": rows })
    }

    fn swap_candidates_json(&self) -> Value {
        let mut out = Vec::new();
        for (i, (code, (opt_no, _wl))) in self.enrolled.iter().enumerate() {
            let Some(c) = self.courses.get(code) else { continue };
            let class_text = c
                .options
                .iter()
                .find(|o| &o.option_no == opt_no)
                .map(|o| {
                    o.components
                        .iter()
                        .map(|cm| format!("{} - Class {} - {}", cm.kind, cm.class_nbr, cm.section))
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            out.push(json!({
                "row": i,
                "course": format!("{} {}", c.code, c.title),
                "class_text": class_text,
            }));
        }
        Value::Array(out)
    }
}

pub struct DemoBackend {
    state: Arc<Mutex<DemoState>>,
    last_plan_course: Option<String>,
    staged_enrols: BTreeMap<String, String>,
    staged_swaps: BTreeMap<String, Vec<String>>,
}

impl DemoBackend {
    pub fn new(state: Arc<Mutex<DemoState>>) -> Self {
        Self {
            state,
            last_plan_course: None,
            staged_enrols: BTreeMap::new(),
            staged_swaps: BTreeMap::new(),
        }
    }

    fn pause(&self) {
        let ms = self.state.lock().map(|s| s.delay_ms).unwrap_or(0);
        if ms > 0 {
            std::thread::sleep(Duration::from_millis(ms));
        }
    }
}

impl Backend for DemoBackend {
    fn tool(&mut self, name: &str, args: &Value) -> Result<Value, String> {
        self.pause();
        let mut st = self.state.lock().map_err(|_| "demo state lock poisoned".to_string())?;
        match name {
            "plan_class_list" => Ok(st.class_list_json(&self.staged_enrols)),
            "plan_select_class" => {
                let row = args["row"].as_u64().ok_or("missing row")? as usize;
                let list = st.class_list_json(&self.staged_enrols);
                let r = list["rows"].as_array().and_then(|a| a.get(row)).ok_or("row not on list")?;
                let code = r["course_label"].as_str().unwrap_or("").split(':').next().unwrap_or("").trim().to_string();
                self.last_plan_course = Some(code.clone());
                let sections = st.sections_json(&code)?;
                Ok(json!({ "course_key": st.course_key(&code), "sections": sections }))
            }
            "course_sections" => {
                let crse_id = args["crse_id"].as_str().ok_or("missing crse_id")?;
                let code = st.courses.values().find(|c| c.crse_id == crse_id).map(|c| c.code.clone()).ok_or("course not found")?;
                st.sections_json(&code)
            }
            "catalog_sections" => {
                let crse_id = args["crse_id"].as_str().ok_or("missing crse_id")?;
                let code = st.courses.values().find(|c| c.crse_id == crse_id).map(|c| c.code.clone()).ok_or("course not found")?;
                st.sections_json(&code)
            }
            "catalog_search" => {
                let kw = args["keyword"].as_str().unwrap_or("").trim().to_ascii_lowercase();
                let kw_compact: String = kw.chars().filter(|c| !c.is_whitespace()).collect();
                let hits: Vec<Value> = st
                    .courses
                    .values()
                    .filter(|c| {
                        let c_compact: String = c.code.to_ascii_lowercase().chars().filter(|ch| !ch.is_whitespace()).collect();
                        c_compact.contains(&kw_compact) || c.title.to_ascii_lowercase().contains(&kw)
                    })
                    .map(|c| json!({ "code": c.code, "title": c.title, "crse_id": c.crse_id, "offer_nbr": c.offer_nbr, "effdt": c.effdt, "units": c.units }))
                    .collect();
                Ok(Value::Array(hits))
            }
            "catalog_detail" => {
                let crse_id = args["crse_id"].as_str().ok_or("missing crse_id")?;
                let c = st.courses.values().find(|c| c.crse_id == crse_id).ok_or("course not found")?;
                Ok(json!({
                    "code": c.code,
                    "title": c.title,
                    "units": c.units,
                    "career": "Credit",
                    "grading": "60 Percent and Letter Grade",
                    "components": ["Lecture"],
                    "campus": "Doha Campus",
                    "academic_group": "College of Computing & IT",
                    "academic_org": "University of Doha",
                    "requirement": c.requirement,
                    "attribute": "",
                    "description": format!("{} ({})", c.title, c.code),
                }))
            }
            "drop_terms" => {
                if st.enrolled.is_empty() {
                    Ok(json!([{ "row": 0, "term": "Fall 2026" }]))
                } else {
                    Ok(json!([{ "row": 0, "term": "Winter 2027" }, { "row": 1, "term": "Fall 2026" }]))
                }
            }
            "enrolled_classes" => Ok(st.enrolled_classes_json()),
            "swap_candidates" => Ok(st.swap_candidates_json()),
            "swap_options" => {
                let row = args["candidate_row"].as_u64().ok_or("missing candidate_row")? as usize;
                let cands = st.swap_candidates_json();
                let cand = cands.as_array().and_then(|a| a.get(row)).ok_or("swap row not on page")?;
                let code = cand["course"].as_str().unwrap_or("").split_whitespace().take(2).collect::<Vec<_>>().join(" ");
                let sections = st.sections_json(&code)?;
                Ok(json!({ "course_key": st.course_key(&code), "sections": sections }))
            }
            "degree_tree" => Ok(st.degree_tree_json()),
            "degree_requirement" => {
                let year = args["year"].as_u64().ok_or("missing year")? as u32;
                let index = args["index"].as_u64().ok_or("missing index")? as usize;
                let r = st.degree.iter().filter(|x| x.year == year).nth(index).ok_or("row not found")?;
                let req = st.courses.get(&r.code).map(|c| c.requirement.clone()).unwrap_or_default();
                Ok(json!({ "course": r.code, "text": req }))
            }
            "degree_course_list" => Ok(json!({
                "name": "Semester 6 - Open Elective List",
                "choices": [
                    { "row": 0, "joined_by": "", "code": "GARC 2003", "title": "Data Analytics Foundations", "category": "Elective", "units": "3.00", "term": "" },
                    { "row": 1, "joined_by": "or", "code": "COMM 2010", "title": "Intercultural Communication", "category": "Elective", "units": "3.00", "term": "" },
                ]
            })),
            "preview_drop" => {
                if !st.armed {
                    return Err("Writes are off.".into());
                }
                let nbr = args["class_nbr"].as_str().ok_or("missing class_nbr")?.to_string();
                let listed: Vec<String> = st
                    .enrolled_classes_json()["classes"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|x| x["class_nbr"].as_str().map(String::from)).collect())
                    .unwrap_or_default();
                if !listed.contains(&nbr) {
                    return Err("class number not in your enrolled list".into());
                }
                let mut v = st.issue("drop", "Winter 2027", vec![nbr.clone()], &format!("Drop {nbr}"), DemoToken::Drop { class_nbr: nbr });
                v["plan"]["listed_classes"] = json!(listed);
                Ok(v)
            }
            "preview_swap_stage" => {
                if !st.armed {
                    return Err("Writes are off.".into());
                }
                let row = args["candidate_row"].as_u64().ok_or("missing candidate_row")? as usize;
                let cands = st.swap_candidates_json();
                let cand = cands.as_array().and_then(|a| a.get(row)).ok_or("swap row not on page")?;
                let code = cand["course"].as_str().unwrap_or("").split_whitespace().take(2).collect::<Vec<_>>().join(" ");
                let nbrs: Vec<String> = args["new_class_nbrs"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                    .unwrap_or_default();
                Ok(st.issue("swap_stage", "Winter 2027", nbrs.clone(), &format!("Stage swap for {code}"), DemoToken::SwapStage { course: code, new_nbrs: nbrs }))
            }
            "preview_swap_commit" => {
                if !st.armed {
                    return Err("Writes are off.".into());
                }
                let all: Vec<String> = self.staged_swaps.values().flatten().cloned().collect();
                Ok(st.issue("swap", "Winter 2027", all, "Commit swaps", DemoToken::SwapCommit))
            }
            "preview_plan_term" => {
                if !st.armed {
                    return Err("Writes are off.".into());
                }
                let year = args["year"].as_u64().ok_or("missing year")? as u32;
                let index = args["index"].as_u64().ok_or("missing index")? as u32;
                let term_code = args["term_code"].as_str().ok_or("missing term_code")?.to_string();
                let term_label = term_label_for(&term_code).ok_or("unknown term code")?.to_string();
                let code = st.degree.iter().filter(|x| x.year == year).nth(index as usize).map(|x| x.code.clone()).ok_or("row not found")?;
                let compact: String = code.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
                Ok(st.issue("plan_term", &term_label, vec![compact], &format!("Move {code} to {term_label}"), DemoToken::PlanTerm { year, index, term_code, code }))
            }
            "preview_stage_enrol" => {
                if !st.armed {
                    return Err("Writes are off.".into());
                }
                let option_row = args["option_row"].as_u64().ok_or("missing option_row")? as u32;
                let code = self.last_plan_course.clone().ok_or("open a course first")?;
                let nbrs: Vec<String> = st
                    .courses
                    .get(&code)
                    .and_then(|c| c.options.iter().find(|o| o.row == option_row))
                    .map(|o| o.components.iter().map(|cm| cm.class_nbr.clone()).collect())
                    .ok_or("option not found")?;
                Ok(st.issue("stage_enrol", "Winter 2027", nbrs, &format!("Stage {code}"), DemoToken::StageEnrol { option_row }))
            }
            "preview_submit_enrol" => {
                if !st.armed {
                    return Err("Writes are off.".into());
                }
                let mut nbrs = Vec::new();
                let mut live_rows = Vec::new();
                for (code, opt_no) in &self.staged_enrols {
                    if let Some(c) = st.courses.get(code) {
                        if let Some(o) = c.options.iter().find(|x| &x.option_no == opt_no) {
                            nbrs.extend(o.components.iter().map(|cm| cm.class_nbr.clone()));
                        }
                        live_rows.push(json!([format!("{}: {}", c.code, c.title), "Classes available", "Selected"]));
                    }
                }
                let mut v = st.issue("submit_enrol", "Winter 2027", nbrs, "Submit enrolment", DemoToken::SubmitEnrol);
                v["plan"]["live_rows"] = Value::Array(live_rows);
                Ok(v)
            }
            _ => Err(format!("demo tool {name} not supported")),
        }
    }

    fn confirm(&mut self, token: &str, phrase: &str) -> Result<Value, ConfirmFail> {
        self.pause();
        let mut st = self.state.lock().map_err(|_| ConfirmFail::from("demo lock error".to_string()))?;
        if !st.armed {
            return Err(ConfirmFail::from("Writes are off.".to_string()));
        }
        let Some((tok, req_phrase)) = st.tokens.remove(token) else {
            return Err(ConfirmFail::from("That preview is no longer valid. Preview again.".to_string()));
        };
        if phrase != req_phrase {
            return Err(ConfirmFail::from("The phrase did not match.".to_string()));
        }
        match tok {
            DemoToken::Drop { class_nbr } => {
                let mut dropped_code = None;
                for (code, (opt_no, _)) in &st.enrolled {
                    if let Some(c) = st.courses.get(code) {
                        if let Some(o) = c.options.iter().find(|x| &x.option_no == opt_no) {
                            if o.components.iter().any(|cm| cm.class_nbr == class_nbr) {
                                dropped_code = Some(code.clone());
                                break;
                            }
                        }
                    }
                }
                if let Some(code) = dropped_code {
                    st.enrolled.remove(&code);
                    for dr in &mut st.degree {
                        if dr.code == code && dr.term.as_deref() == Some("Winter 2027") {
                            dr.term = None;
                            dr.selected_code = Some("1271".into());
                        }
                    }
                }
                Ok(json!({ "kind": "DROP", "classNumbers": [class_nbr], "messages": ["Dropped."], "detail": {} }))
            }
            DemoToken::SwapStage { course, new_nbrs } => {
                self.staged_swaps.insert(course, new_nbrs.clone());
                Ok(json!({ "kind": "SWAPSTAGE", "classNumbers": new_nbrs, "messages": ["Staged."], "detail": {} }))
            }
            DemoToken::SwapCommit => {
                let staged = std::mem::take(&mut self.staged_swaps);
                let mut all_nbrs = Vec::new();
                for (code, new_nbrs) in staged {
                    all_nbrs.extend(new_nbrs.clone());
                    let want: BTreeSet<String> = new_nbrs.into_iter().collect();
                    let matched = st.courses.get(&code).and_then(|c| {
                        c.options
                            .iter()
                            .find(|o| {
                                let have: BTreeSet<String> = o.components.iter().map(|cm| cm.class_nbr.clone()).collect();
                                have == want
                            })
                            .map(|opt| (opt.option_no.clone(), opt.status.eq_ignore_ascii_case("wait list")))
                    });
                    if let Some((opt_no, wl)) = matched {
                        st.enrolled.insert(code, (opt_no, wl));
                    }
                }
                Ok(json!({ "kind": "SWAP", "classNumbers": all_nbrs, "messages": ["Swapped."], "detail": { "results": [] } }))
            }
            DemoToken::PlanTerm { year, index, term_code, code } => {
                if let Some(r) = st.degree.iter_mut().filter(|x| x.year == year).nth(index as usize) {
                    r.selected_code = Some(term_code);
                    r.term = None;
                }
                let compact: String = code.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
                Ok(json!({ "kind": "PLAN", "classNumbers": [compact], "messages": ["Saved."], "detail": {} }))
            }
            DemoToken::StageEnrol { option_row } => {
                let code = self.last_plan_course.clone().ok_or_else(|| ConfirmFail::from("no course".to_string()))?;
                let opt = st.courses.get(&code).and_then(|c| c.options.iter().find(|o| o.row == option_row)).cloned().ok_or_else(|| ConfirmFail::from("no option".to_string()))?;
                let nbrs: Vec<String> = opt.components.iter().map(|cm| cm.class_nbr.clone()).collect();
                self.staged_enrols.insert(code, opt.option_no);
                Ok(json!({ "kind": "STAGE", "classNumbers": nbrs, "messages": ["Staged."], "detail": {} }))
            }
            DemoToken::SubmitEnrol => {
                let staged = std::mem::take(&mut self.staged_enrols);
                let mut icons = Vec::new();
                let mut lines = Vec::new();
                let mut all_nbrs = Vec::new();
                for (code, opt_no) in staged {
                    let matched = st.courses.get(&code).and_then(|c| {
                        c.options
                            .iter()
                            .find(|o| o.option_no == opt_no)
                            .map(|opt| {
                                let nbrs: Vec<String> = opt.components.iter().map(|cm| cm.class_nbr.clone()).collect();
                                let wl = opt.status.eq_ignore_ascii_case("wait list");
                                (nbrs, wl, c.code.clone(), c.title.clone())
                            })
                    });
                    if let Some((nbrs, wl, c_code, c_title)) = matched {
                        all_nbrs.extend(nbrs);
                        st.enrolled.insert(code.clone(), (opt_no, wl));
                        if !wl {
                            for dr in &mut st.degree {
                                if dr.code == code {
                                    dr.selected_code = None;
                                    dr.term = Some("Winter 2027".into());
                                }
                            }
                        }
                        icons.push("success");
                        lines.push(format!("{c_code}: {c_title}"));
                    }
                }
                Ok(json!({ "kind": "ENROLL", "classNumbers": all_nbrs, "messages": ["Enrolled."], "detail": { "icons": icons, "lines": lines } }))
            }
        }
    }

    fn arm(&mut self) {
        if let Ok(mut st) = self.state.lock() {
            st.armed = true;
        }
    }

    fn disarm(&mut self) {
        self.staged_enrols.clear();
        self.staged_swaps.clear();
        self.last_plan_course = None;
        if let Ok(mut st) = self.state.lock() {
            st.armed = false;
            st.tokens.clear();
        }
    }
}
