//! A timing log, one JSON object per line, in `<data>/logs/planner.log`.
//!
//! What is written: when something happened and how long it took. Every HTTP request this server answers, every
//! operation (how long it waited for its turn, how long it ran, whether it worked), every step inside the long ones
//! (class list, seat read, open course, stage, submit, verify), and every single request the connector sends to
//! PeopleSoft (its fixed step label, method, path without query, status, milliseconds, bytes, and the idle gap since the
//! previous request, which is the connector's 1 s pacing plus parsing).
//!
//! What is never written: passwords, cookies, headers, request or response bodies (no page excerpts either), query strings.
//! Callers pass only
//! short facts (counts, class numbers, step names, page messages truncated to a sentence).

use serde_json::{json, Map, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const ROTATE_AT: u64 = 20 * 1024 * 1024;

struct Sink {
    file: Option<std::fs::File>,
    path: PathBuf,
    bytes: u64,
}

static SINK: OnceLock<Mutex<Sink>> = OnceLock::new();
/// The most recent events, kept in memory too (tests read them, and they are handy for a quick look).
static RING: OnceLock<Mutex<std::collections::VecDeque<String>>> = OnceLock::new();
const RING_KEEP: usize = 2000;

/// What PeopleSoft's own pages said (result-like sentences only, never HTML), per stage, since last taken.
static SAID: OnceLock<Mutex<Vec<(String, String)>>> = OnceLock::new();

/// Remember sentences a page said, so the operation that caused the page can show them to the person.
pub fn note_said(stage: &str, sentences: &[String]) {
    if let Ok(mut v) = SAID.get_or_init(|| Mutex::new(Vec::new())).lock() {
        for s in sentences {
            if !v.iter().any(|(st, x)| st == stage && x == s) {
                v.push((stage.to_string(), s.clone()));
            }
        }
        let extra = v.len().saturating_sub(60);
        v.drain(..extra);
    }
}

/// Take (and forget) what the given stages' pages said. One operation runs at a time, so these belong to it.
pub fn take_said(stages: &[&str]) -> Vec<String> {
    let Some(m) = SAID.get() else { return Vec::new() };
    let Ok(mut v) = m.lock() else { return Vec::new() };
    let mut out = Vec::new();
    v.retain(|(st, x)| {
        if stages.contains(&st.as_str()) {
            if !out.contains(x) {
                out.push(x.clone());
            }
            false
        } else {
            true
        }
    });
    out
}

/// The last `n` events as JSON, oldest first.
#[cfg_attr(not(test), allow(dead_code))]
pub fn recent(n: usize) -> Vec<Value> {
    let Some(r) = RING.get() else { return Vec::new() };
    let Ok(g) = r.lock() else { return Vec::new() };
    let skip = g.len().saturating_sub(n);
    g.iter().skip(skip).filter_map(|l| serde_json::from_str(l).ok()).collect()
}

/// Open (or create) the log. Returns its path. Safe to call once at start-up; later calls are ignored.
pub fn init(dir: &Path) -> std::io::Result<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(dir)?;
    }
    let path = dir.join("planner.log");
    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    #[cfg(unix)]
    let file = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new().create(true).append(true).mode(0o600).open(&path)?
    };
    #[cfg(not(unix))]
    let file = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
    let _ = SINK.set(Mutex::new(Sink { file: Some(file), path: path.clone(), bytes }));
    Ok(path)
}

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// `2026-11-16T10:00:00.123Z` from epoch milliseconds (no time-zone database needed: UTC only).
pub fn iso_utc(ms: u64) -> String {
    let secs = (ms / 1000) as i64;
    let milli = ms % 1000;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{milli:03}Z", rem / 3600, (rem % 3600) / 60, rem % 60)
}

/// Cut a message to `n` characters so one odd page cannot bloat the log.
pub fn trunc(s: &str, n: usize) -> String {
    let one_line: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= n {
        one_line
    } else {
        let mut t: String = one_line.chars().take(n).collect();
        t.push('\u{2026}');
        t
    }
}

pub fn ms_since(t: std::time::Instant) -> f64 {
    (t.elapsed().as_secs_f64() * 10_000.0).round() / 10.0
}

/// Write one event. Never panics and never blocks on anything but the file write.
pub fn event(ev: &str, fields: Value) {
    let mut obj = Map::new();
    obj.insert("ts".into(), json!(iso_utc(now_ms())));
    obj.insert("ev".into(), json!(ev));
    if let Value::Object(m) = fields {
        obj.extend(m);
    }
    let line = match serde_json::to_string(&Value::Object(obj)) {
        Ok(l) => l,
        Err(_) => return,
    };
    if let Ok(mut r) = RING.get_or_init(|| Mutex::new(std::collections::VecDeque::new())).lock() {
        if r.len() >= RING_KEEP {
            r.pop_front();
        }
        r.push_back(line.clone());
    }
    let Some(sink) = SINK.get() else { return };
    let Ok(mut s) = sink.lock() else { return };
    if s.bytes > ROTATE_AT {
        let old = s.path.with_extension("log.1");
        let _ = std::fs::rename(&s.path, old);
        if let Ok(f) = std::fs::OpenOptions::new().create(true).append(true).open(&s.path) {
            s.file = Some(f);
            s.bytes = 0;
        }
    }
    if let Some(f) = s.file.as_mut() {
        if writeln!(f, "{line}").is_ok() {
            s.bytes += line.len() as u64 + 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_time_is_right_across_known_dates() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_utc(1_000_000_000_123), "2001-09-09T01:46:40.123Z");
        assert_eq!(iso_utc(1_794_000_000_000), "2026-11-06T21:20:00.000Z");
        assert_eq!(iso_utc(1_709_164_800_000), "2024-02-29T00:00:00.000Z"); // a leap day
        assert_eq!(iso_utc(4_102_444_799_999), "2099-12-31T23:59:59.999Z");
    }

    #[test]
    fn what_a_page_said_is_kept_per_stage_taken_once_and_deduplicated() {
        note_said("unit_stage_a", &["Unable to drop".to_string(), "Unable to drop".to_string()]);
        note_said("unit_stage_b", &["Something else".to_string()]);
        assert_eq!(take_said(&["unit_stage_a"]), vec!["Unable to drop".to_string()]);
        assert!(take_said(&["unit_stage_a"]).is_empty(), "taking forgets it");
        assert_eq!(take_said(&["unit_stage_b"]), vec!["Something else".to_string()], "other stages are untouched");
    }

    #[test]
    fn long_or_multiline_messages_are_cut_to_one_short_line() {
        assert_eq!(trunc("a  b\n c", 50), "a b c");
        let t = trunc(&"x".repeat(500), 20);
        assert_eq!(t.chars().count(), 21);
        assert!(t.ends_with('\u{2026}'));
    }

    #[test]
    fn events_are_json_lines_with_time_and_name_and_no_free_text_fields_added() {
        let dir = std::env::temp_dir().join(format!("planner-log-test-{}", now_ms()));
        let path = init(&dir).unwrap();
        event("unit", json!({ "ms": 1.5, "stage": "log-test-marker" }));
        // Other tests log to the same file at the same time, so find this test's own line instead of taking the last one.
        let text = std::fs::read_to_string(&path).unwrap();
        let mine: Value = text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).find(|e| e["stage"] == "log-test-marker").expect("the event was written as one JSON line");
        assert_eq!(mine["ev"], "unit");
        assert_eq!(mine["ms"], 1.5);
        assert!(mine["ts"].as_str().unwrap().ends_with('Z'));
        #[cfg(unix)]
        {
            let mode = std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&path).unwrap().permissions()) & 0o777;
            assert_eq!(mode, 0o600, "the log is private to the user");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
