#![cfg(feature = "http-client")]
//! The CLI is never run against the real network here. These cases fail on input validation before any request.
use std::io::Write;
use std::process::{Command, Stdio};

fn run(input: &[u8]) -> (i32, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_frankie-auth-cli"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let out = child.wait_with_output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

#[test]
fn invalid_input_prints_one_failure_line_exits_1_and_echoes_nothing_secret() {
    let secret = "CLI-PLANTED-PASSWORD";
    let cases: Vec<Vec<u8>> = vec![
        b"".to_vec(),
        b"not json".to_vec(),
        format!(r#"{{"username":"not-a-udst-user","password":"{secret}"}}"#).into_bytes(),
        format!(r#"{{"username":"12345678","password":"{secret}","extra":1}}"#).into_bytes(),
        vec![b' '; 9000],
    ];
    for input in cases {
        let (code, out, err) = run(&input);
        assert_eq!(code, 1);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 1, "exactly one stdout line: {out:?}");
        let v: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(
            (v["ok"].as_bool(), v["provider"].as_str()),
            (Some(false), Some("d2l"))
        );
        assert!(v.get("access_token").is_none());
        assert!(!out.contains(secret) && !err.contains(secret));
    }
}
