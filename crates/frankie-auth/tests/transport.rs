#![cfg(feature = "http-client")]
//! The real HTTP transport against a throwaway server on 127.0.0.1 (plain http, loopback only, invented data).
use frankie_auth::*;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use url::Url;

type Heads = Arc<Mutex<Vec<String>>>;

fn read_request(s: &mut TcpStream) -> Option<(String, Vec<u8>)> {
    s.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    let head_end = loop {
        let n = s.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let len = head
        .lines()
        .find_map(|l| {
            l.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .and_then(|v| v.trim().parse::<usize>().ok())
        })
        .unwrap_or(0);
    while buf.len() < head_end + len {
        let n = s.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    Some((head, buf[head_end..].to_vec()))
}

/// Serve `responses.len()` connections; each gets the next raw response. Returns the base URL and the recorded request heads.
fn serve(responses: Vec<Vec<u8>>) -> (Url, Heads) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let heads: Heads = Arc::new(Mutex::new(vec![]));
    let h = heads.clone();
    thread::spawn(move || {
        for raw in responses {
            let Ok((mut s, _)) = listener.accept() else {
                return;
            };
            if let Some((head, body)) = read_request(&mut s) {
                h.lock()
                    .unwrap()
                    .push(format!("{head}{}", String::from_utf8_lossy(&body)));
            }
            let _ = s.write_all(&raw);
            let _ = s.flush();
        }
    });
    (url, heads)
}

fn resp(status: &str, headers: &[&str], body: &str) -> Vec<u8> {
    let mut r = format!(
        "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for h in headers {
        r.push_str(h);
        r.push_str("\r\n");
    }
    r.push_str("\r\n");
    r.push_str(body);
    r.into_bytes()
}

fn client() -> ReqwestHttp {
    ReqwestHttp::new(Duration::from_secs(5)).unwrap()
}

#[test]
fn redirects_are_returned_not_followed() {
    let (url, heads) = serve(vec![
        resp("302 Found", &["Location: /elsewhere"], ""),
        resp("200 OK", &[], "should never be fetched"),
    ]);
    let r = client().send(HttpRequest::get(url)).unwrap();
    assert_eq!((r.status, r.location.as_deref()), (302, Some("/elsewhere")));
    thread::sleep(Duration::from_millis(200));
    assert_eq!(
        heads.lock().unwrap().len(),
        1,
        "exactly one request was made"
    );
}

#[test]
fn a_307_on_a_post_is_returned_and_the_body_is_not_replayed() {
    let (url, heads) = serve(vec![
        resp("307 Temporary Redirect", &["Location: /again"], ""),
        resp("200 OK", &[], "x"),
    ]);
    let r = client()
        .send(HttpRequest::post(
            url,
            "application/x-www-form-urlencoded",
            "passwd=SECRET-VALUE".into(),
        ))
        .unwrap();
    assert_eq!(r.status, 307);
    thread::sleep(Duration::from_millis(200));
    let h = heads.lock().unwrap();
    assert_eq!(h.len(), 1, "the POST was sent once and not replayed");
    assert!(h[0].contains("passwd=SECRET-VALUE"));
}

#[test]
fn post_body_content_type_and_headers_are_sent() {
    let (url, heads) = serve(vec![resp(
        "200 OK",
        &["Content-Type: application/json"],
        "{}",
    )]);
    let req = HttpRequest::post(
        url,
        "application/x-www-form-urlencoded",
        "scope=*:*:*".into(),
    )
    .with_header("X-Csrf-Token", "CSRF-VALUE".into());
    let r = client().send(req).unwrap();
    assert!(r.is_json());
    let h = heads.lock().unwrap()[0].to_ascii_lowercase();
    assert!(h.starts_with("post / http/1.1"), "{h}");
    assert!(
        h.contains("content-type: application/x-www-form-urlencoded")
            && h.contains("x-csrf-token: csrf-value")
            && h.ends_with("scope=*:*:*"),
        "{h}"
    );
}

#[test]
fn cookie_jar_is_used_in_jar_mode_and_never_in_none_mode() {
    let (url, heads) = serve(vec![
        resp("200 OK", &["Set-Cookie: sid=abc123; Path=/"], "first"),
        resp("200 OK", &[], "second"),
        resp("200 OK", &["Set-Cookie: leaked=1; Path=/"], "third"),
        resp("200 OK", &[], "fourth"),
    ]);
    let mut c = client();
    c.send(HttpRequest::get(url.clone())).unwrap(); // sets sid
    c.send(HttpRequest::get(url.clone())).unwrap(); // jar: must send sid
    c.send(HttpRequest::get(url.clone()).without_cookies())
        .unwrap(); // none: must not send sid, must not store `leaked`
    c.send(HttpRequest::get(url)).unwrap(); // jar again: sid yes, leaked no
    thread::sleep(Duration::from_millis(200));
    let h: Vec<String> = heads
        .lock()
        .unwrap()
        .iter()
        .map(|s| s.to_ascii_lowercase())
        .collect();
    assert_eq!(h.len(), 4);
    assert!(!h[0].contains("cookie:"));
    assert!(h[1].contains("cookie: sid=abc123"), "{}", h[1]);
    assert!(
        !h[2].contains("cookie:"),
        "none-mode must send no cookies: {}",
        h[2]
    );
    assert!(
        h[3].contains("sid=abc123") && !h[3].contains("leaked"),
        "{}",
        h[3]
    );
}

#[test]
fn oversized_bodies_are_refused() {
    let big = "x".repeat(MAX_BODY_BYTES as usize + 10);
    let (url, _) = serve(vec![resp("200 OK", &[], &big)]);
    assert_eq!(
        client().send(HttpRequest::get(url)).err(),
        Some(AuthFailure::UnexpectedPage)
    );
}

#[test]
fn a_body_just_under_the_cap_is_accepted() {
    let ok = "y".repeat(MAX_BODY_BYTES as usize);
    let (url, _) = serve(vec![resp("200 OK", &[], &ok)]);
    assert_eq!(
        client().send(HttpRequest::get(url)).unwrap().body.len(),
        MAX_BODY_BYTES as usize
    );
}

#[test]
fn a_silent_server_times_out_and_a_closed_port_is_a_network_error() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let hold = thread::spawn(move || {
        let (_s, _) = listener.accept().unwrap();
        thread::sleep(Duration::from_secs(2)); // never answers
    });
    let mut quick = ReqwestHttp::new(Duration::from_millis(300)).unwrap();
    assert_eq!(
        quick.send(HttpRequest::get(url)).err(),
        Some(AuthFailure::Timeout)
    );
    hold.join().unwrap();

    let dead = TcpListener::bind("127.0.0.1:0").unwrap();
    let dead_url = Url::parse(&format!("http://{}/", dead.local_addr().unwrap())).unwrap();
    drop(dead);
    assert_eq!(
        client().send(HttpRequest::get(dead_url)).err(),
        Some(AuthFailure::Network)
    );
}

#[test]
fn http_request_and_response_debug_hide_secrets() {
    let req = HttpRequest::post(
        Url::parse("https://d2l.udst.edu.qa/x").unwrap(),
        "application/x-www-form-urlencoded",
        "passwd=SECRET-VALUE".into(),
    )
    .with_header("Authorization", "Bearer SECRET-BEARER".into());
    let shown = format!("{req:?}");
    assert!(!shown.contains("SECRET"), "{shown}");
    let r = HttpResponse {
        status: 200,
        location: None,
        content_type: None,
        body: "SECRET-PAGE".into(),
    };
    assert!(!format!("{r:?}").contains("SECRET"));
}

#[cfg(feature = "dev-session-export")]
#[test]
fn dev_export_returns_only_jar_cookies_for_the_url() {
    let (url, _heads) = serve(vec![resp(
        "200 OK",
        &[
            "Set-Cookie: sid=abc123; Path=/",
            "Set-Cookie: other=2; Path=/x",
        ],
        "first",
    )]);
    let mut c = client();
    c.send(HttpRequest::get(url.clone())).unwrap();
    let got = c.dev_export_cookies(&url);
    assert_eq!(got, vec![("sid".to_string(), "abc123".to_string())]);
}
