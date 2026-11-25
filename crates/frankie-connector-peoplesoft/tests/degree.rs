use frankie_connector_peoplesoft::connector::nth_row_in_year;
use frankie_connector_peoplesoft::model::dedupe_latest;
use frankie_connector_peoplesoft::parse::{parse_catalog_results, parse_course_list, parse_degree, parse_requirement_text};

const COLLAPSED: &str = include_str!("fixtures/degree/overview_collapsed.html");
const TREE: &str = include_str!("fixtures/degree/tree_expanded.html");
const REQ: &str = include_str!("fixtures/degree/requirement_text.html");
const LIST_MATH: &str = include_str!("fixtures/degree/course_list_math_prereq.html");
const CATALOG_MATH: &str = include_str!("fixtures/degree/catalog_results_math1020.html");
const CATALOG_INFS: &str = include_str!("fixtures/degree/catalog_results_infs.html");
const LIST_SCIE: &str = include_str!("fixtures/degree/course_list_scie.html");

#[test]
fn the_collapsed_overview_has_four_years_that_all_say_expand() {
    let d = parse_degree(COLLAPSED).unwrap();
    assert_eq!(d.years.len(), 4);
    assert_eq!(d.years.iter().map(|y| y.name.as_str()).collect::<Vec<_>>(), ["910-901-R Year 1", "910-901-R Year 2", "910-901-R Year 3", "910-901-R Year 4"]);
    assert!(d.years.iter().all(|y| y.state == "Expand"), "{:?}", d.years);
    assert!(d.years[0].summary.contains("Total Courses Required: 10"), "{}", d.years[0].summary);
    assert!(d.rows.is_empty(), "a collapsed tree has no course rows");
}

#[test]
fn the_expanded_tree_has_all_39_rows() {
    let d = parse_degree(TREE).unwrap();
    assert_eq!(d.rows.len(), 39);
    assert_eq!(d.rows.iter().map(|r| r.row).collect::<Vec<_>>(), (0..39).collect::<Vec<_>>());
    // Each row sits under the right Year: the capture's Year 1 has rows 0 to 10, Year 2 starts at row 11.
    assert!(d.rows[..11].iter().all(|r| r.year == 0));
    assert_eq!(d.rows[11].year, 1);
    assert_eq!(d.rows[38].year, 3);
}

#[test]
fn a_course_row_reads_code_title_category_units_and_term() {
    let d = parse_degree(TREE).unwrap();
    let r = &d.rows[15];
    assert_eq!((r.code.as_str(), r.title.as_str()), ("DACS 2202", "Intro. to Computing Systems"));
    assert_eq!((r.category.as_str(), r.units.as_deref()), ("Required", Some("4.00")));
    assert_eq!(r.term.as_deref(), Some("Winter 2027"), "a course that is taken or in progress shows its term as text");
    assert!(r.term_choice.is_none());
    assert!(!r.joined_by_and, "the first row of a block has no 'and'");
    assert!(d.rows[16].joined_by_and, "the next row of the block is joined with 'and'");
    assert_eq!(d.rows[16].code, "DSAI 2201");
}

#[test]
fn a_course_that_can_still_be_planned_has_a_term_dropdown() {
    let d = parse_degree(TREE).unwrap();
    let r = &d.rows[19];
    assert_eq!(r.code, "SOFT 2301");
    let c = r.term_choice.as_ref().expect("a dropdown");
    assert!(r.term.is_none());
    assert_eq!(c.options.len(), 14, "Fall, Spring and Winter from 2027 to 2031 (no summer)");
    assert!(c.options.iter().any(|o| o.code == "1271" && o.label == "Winter 2027"));
    assert!(c.options.iter().any(|o| o.code == "1278" && o.label == "Fall 2027"));
    assert!(c.options.iter().all(|o| !o.code.is_empty()), "the blank option is not a term");
    assert!(c.options.iter().any(|o| o.code == c.selected), "the selected code is one of the options: {}", c.selected);
}

#[test]
fn a_course_list_row_is_a_group_with_a_select_button() {
    let d = parse_degree(TREE).unwrap();
    let g = &d.rows[2];
    assert!(g.label.contains("EFFL Course List"), "{}", g.label);
    assert!(g.code.is_empty() && g.units.is_none());
    assert!(g.has_select);
    assert!(g.joined_by_and);
    assert!(d.rows.iter().filter(|r| r.has_select).count() >= 7, "the capture has seven Select Courses buttons");
}

#[test]
fn the_semester_headers_are_read_and_each_row_knows_its_semester() {
    let d = parse_degree(TREE).unwrap();
    assert_eq!(d.years.len(), 4);
    assert_eq!(d.semesters.len(), 12);
    assert_eq!(d.semesters[0].name, "910-901-R Semester 1");
    assert_eq!(d.semesters[0].year, 0);
    assert!(d.semesters[0].summary.contains("Total Courses Required: 4"), "{}", d.semesters[0].summary);
    assert_eq!(d.semesters[11].name, "910-902-R Semester 12");
    // Rows 0 to 4 are Semester 1, rows 5 to 8 Semester 2, row 9 and 10 Semester 3 (the explicit headers agree with the "and" pattern).
    let sem = |r: usize| d.rows[r].semester;
    assert_eq!([sem(0), sem(4), sem(5), sem(8), sem(9), sem(10), sem(38)], [Some(0), Some(0), Some(1), Some(1), Some(2), Some(2), Some(11)]);
    let starts: Vec<u32> = d.rows.iter().filter(|r| !r.joined_by_and).map(|r| r.row).collect();
    assert_eq!(&starts[..3], &[0, 5, 9]);
}

#[test]
fn a_course_list_page_has_its_choices_joined_with_or() {
    let l = parse_course_list(LIST_MATH).unwrap();
    assert_eq!(l.name, "910-901-R - MATH Pre-Requisites");
    assert_eq!(l.choices.iter().map(|c| (c.code.as_str(), c.joined_by.as_str())).collect::<Vec<_>>(), [("MATH 1010", ""), ("MATH 1020", "or")]);
    assert_eq!(l.choices[0].units.as_deref(), Some("3.00"));
    assert_eq!(l.choices[0].term, "Fall 2025");
    let s = parse_course_list(LIST_SCIE).unwrap();
    assert_eq!(s.choices.len(), 2);
    assert_eq!((s.choices[0].term.as_str(), s.choices[1].term.as_str()), ("Spring 2026", ""), "the chosen one has a term, the alternative has none");
    assert_eq!(s.choices[1].category, "Elective");
}

#[test]
fn the_requirement_text_is_kept_whole() {
    let r = parse_requirement_text(REQ).unwrap();
    assert_eq!((r.course.as_str(), r.title.as_str()), ("MATH 1030", "Calculus I"));
    assert!(r.text.starts_with("MATH1020 OR AMPII Score of 85%"), "the title must not run into the text: {}", r.text);
    assert!(r.text.contains("pre-requisite of MATH1030"), "{}", r.text);
    assert!(!r.text.contains("Calculus"), "{}", r.text);
}

#[test]
fn an_empty_or_maintenance_page_is_never_a_degree() {
    for body in ["", "<html><body>The system is down for maintenance.</body></html>"] {
        assert!(parse_degree(body).is_err(), "{body:?}");
        assert!(parse_course_list(body).is_err());
        assert!(parse_requirement_text(body).is_err());
    }
}

#[test]
fn a_row_is_found_by_its_position_in_its_year_not_by_the_page_numbering() {
    let d = parse_degree(TREE).unwrap();
    // Year 2 (index 1) starts at page row 11 in the full tree; its first row is position 0, its 4th (the first with a Select button, DACS 2202's block) etc.
    assert_eq!(nth_row_in_year(&d, 1, 0, |_| true), Some(11));
    assert_eq!(nth_row_in_year(&d, 1, 9, |r| r.has_select), Some(20), "Year 2's Semester 6 Course List");
    assert_eq!(nth_row_in_year(&d, 1, 9, |r| r.has_requirement), None, "a group row has no requirement text");
    assert_eq!(nth_row_in_year(&d, 1, 99, |_| true), None);
}

/// Writes what the parser returns for the expanded tree as JSON (the shape the server sends the page); used to make the frontend's test fixture.
#[test]
#[ignore]
fn write_wire_fixture() {
    let d = parse_degree(TREE).unwrap();
    let out = std::env::var("DEGREE_WIRE_OUT").expect("DEGREE_WIRE_OUT");
    std::fs::write(out, serde_json::to_string_pretty(&d).unwrap()).unwrap();
    let l = parse_course_list(LIST_SCIE).unwrap();
    std::fs::write(std::env::var("LIST_WIRE_OUT").unwrap(), serde_json::to_string_pretty(&l).unwrap()).unwrap();
}

#[test]
fn every_parser_refuses_every_other_page_and_login_or_error_pages() {
    let login = "<html><body><form name='login'><input name='userid'><input name='pwd' type='password'></form></body></html>";
    let error = "<html><body><div id='errorPage'>Your session has expired. Please sign in again.</div></body></html>";
    for page in [COLLAPSED, TREE, REQ, LIST_MATH, LIST_SCIE, login, error] {
        let is_tree = page.contains("VC_PE_HDR_DVW_DESCR100");
        let is_list = page.contains("VC_PE_SS_CL_VW_DESCR50");
        let is_req = page.contains("Course Requirements for");
        assert_eq!(parse_degree(page).is_ok(), is_tree, "degree parser on a page that is not a tree");
        assert_eq!(parse_course_list(page).is_ok(), is_list, "list parser on a page that is not a list");
        assert_eq!(parse_requirement_text(page).is_ok(), is_req, "requirement parser on a page that is not one");
    }
}

/// KNOWN LIMIT, written down on purpose: the unwrapped AJAX content has no closing tag, so a tree cut off in the middle cannot be told
/// from a whole one by the parser. It is protected one layer down (the HTTP client errors on a short body). This test only proves the
/// cut tree never invents rows: what it returns is a prefix of the real rows.
#[test]
fn a_tree_cut_off_in_the_middle_only_ever_loses_rows_never_invents_them() {
    let full = parse_degree(TREE).unwrap();
    assert_eq!(full.rows.len(), 39);
    for cut in [TREE.len() / 2, TREE.len() * 9 / 10] {
        let mut end = cut;
        while !TREE.is_char_boundary(end) {
            end -= 1;
        }
        if let Ok(d) = parse_degree(&TREE[..end]) {
            assert!(d.rows.len() <= full.rows.len());
            for (a, b) in d.rows.iter().zip(full.rows.iter()) {
                assert_eq!((&a.code, &a.label), (&b.code, &b.label));
            }
        }
    }
}


#[test]
fn catalog_results_give_code_course_id_offer_effective_date_units_and_title() {
    let hits = parse_catalog_results(CATALOG_MATH).unwrap();
    let m = hits.iter().find(|h| h.code == "MATH 1020").expect("MATH 1020 is in its own search");
    assert_eq!((m.crse_id.as_str(), m.offer_nbr.as_str(), m.units.as_str(), m.title.as_str()), ("004432", "1", "3.00", "Pre-Calculus"));
    assert!(m.effdt.starts_with("20") && m.effdt.len() == 10 && m.effdt.as_bytes()[4] == b'-', "{}", m.effdt);
}

#[test]
fn a_course_with_several_effective_dates_is_listed_once_with_the_newest() {
    let all = parse_catalog_results(CATALOG_INFS).unwrap();
    assert_eq!(all.len(), 50, "the search shows 50 results");
    let ones: Vec<_> = all.iter().filter(|h| h.code == "INFS 1301").collect();
    assert!(ones.len() > 1, "the capture lists INFS 1301 once per effective date");
    let newest = ones.iter().map(|h| h.effdt.clone()).max().unwrap();
    let d = dedupe_latest(all);
    let one: Vec<_> = d.iter().filter(|h| h.code == "INFS 1301").collect();
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].effdt, newest);
    let mut codes: Vec<_> = d.iter().map(|h| (h.code.clone(), h.offer_nbr.clone())).collect();
    let n = codes.len();
    codes.sort();
    codes.dedup();
    assert_eq!(codes.len(), n, "every course appears once");
}

#[test]
fn only_the_results_page_is_a_catalog_answer() {
    for page in [COLLAPSED, TREE, REQ, LIST_MATH, "", "<html>maintenance</html>"] {
        assert!(parse_catalog_results(page).is_err());
    }
    assert!(parse_catalog_results(include_str!("fixtures/degree/catalog_start.html")).is_err(), "the search page is not a results page");
}
