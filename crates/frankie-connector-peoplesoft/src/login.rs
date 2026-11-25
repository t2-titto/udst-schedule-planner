//! PeopleSoft sign-in (the user-id/password form seen in the capture). Safety rules follow `frankie-auth`:
//! the form must post to the exact sign-in URL on the allow-listed host, a POST is never replayed through a
//! redirect, a rejection is final (no retry), and a sign-in page that is not the plain form (MFA, policy page,
//! unknown layout) stops the flow. Mock-tested only; NOT run against the real site by this code's author.
use crate::pia::{self, PageKind};
use crate::session::{PsError, Session};
use frankie_auth::{AuthFailure, Credentials, Http, HttpRequest};
use regex::Regex;
use url::Url;

/// The PeopleSoft user id is the bare student number (the capture's login form carried 8 digits, no domain).
fn userid_of(creds: &Credentials) -> &str {
    creds
        .username()
        .strip_suffix("@udst.edu.qa")
        .unwrap_or_else(|| creds.username())
}

struct LoginForm {
    hidden: Vec<(String, String)>,
    /// The form's own `action`, resolved against the page address.
    action: Url,
}

/// Find the sign-in form: a `<form>` holding both a `userid` and a `pwd` input (any quote style, any attribute order).
fn parse_login_form(html: &str, page: &Url) -> Option<LoginForm> {
    let re_form = Regex::new(r"(?is)<form\b[^>]*>(.*?)</form>").ok()?;
    let re_tag = Regex::new(r"(?is)<form\b[^>]*>").ok()?;
    let re_input = Regex::new(r"(?is)<input\b[^>]*>").ok()?;
    for f in re_form.captures_iter(html) {
        let whole = f.get(0)?.as_str();
        let body = f.get(1)?.as_str();
        let (mut has_user, mut has_pwd) = (false, false);
        let mut hidden: Vec<(String, String)> = vec![];
        for i in re_input.find_iter(body) {
            let a = pia::attrs(i.as_str());
            match a.get("name").map(String::as_str) {
                Some("userid") => has_user = true,
                Some("pwd") => has_pwd = true,
                Some(n)
                    if a.get("type")
                        .is_some_and(|t| t.eq_ignore_ascii_case("hidden")) =>
                {
                    hidden.push((n.to_string(), a.get("value").cloned().unwrap_or_default()))
                }
                _ => {}
            }
        }
        if has_user && has_pwd {
            let tag = re_tag.find(whole)?.as_str();
            let action = pia::attrs(tag).get("action").cloned().unwrap_or_default();
            return Some(LoginForm {
                hidden,
                action: page.join(&action).ok()?,
            });
        }
    }
    None
}

fn bad_credentials_text(html: &str) -> bool {
    let re = Regex::new(
        r"(?i)user id and/or password (are|is) invalid|invalid user id|incorrect (user|password)",
    )
    .unwrap();
    re.is_match(&pia::visible_text(html))
}

/// GET the sign-in page. The site answers the first request with a 302 to the same address (it sets a cookie),
/// so up to three 301/302/303 hops are followed, all on the allow-listed host.
fn fetch_login_page<H: Http>(sess: &mut Session<H>) -> Result<(Url, String), PsError> {
    let mut cur = sess.cfg.login_url();
    for _ in 0..4 {
        let r = sess.send(HttpRequest::get(cur.clone()).stage("ps_login_form"))?;
        match r.status {
            200 => return Ok((cur, r.body)),
            301..=303 => {
                let loc = r
                    .location
                    .as_deref()
                    .ok_or(PsError::Unexpected("sign-in redirect without location"))?;
                let next = cur
                    .join(loc)
                    .map_err(|_| PsError::Unexpected("bad redirect"))?;
                if !sess.cfg.host_allowed(&next) {
                    return Err(PsError::Origin("sign-in redirect not allowed"));
                }
                cur = next;
            }
            _ => return Err(PsError::Unexpected("sign-in page status not 200")),
        }
    }
    Err(PsError::Unexpected("sign-in page redirect limit"))
}

/// Sign in. On success the session cookie lives only in the HTTP client's jar.
pub fn login<H: Http>(sess: &mut Session<H>, creds: &Credentials) -> Result<(), PsError> {
    let (page_url, body) = fetch_login_page(sess)?;
    let form = parse_login_form(&body, &page_url)
        .ok_or(PsError::Unexpected("sign-in form not found on a 200 page"))?;
    let url = form.action.clone();
    if !sess.cfg.host_allowed(&url) || !url.path().starts_with(&format!("/psp/{}", sess.cfg.site)) {
        return Err(PsError::Origin("sign-in target not allowed"));
    }
    let mut ser = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in &form.hidden {
        if k != "userid" && k != "pwd" {
            ser.append_pair(k, v);
        }
    }
    if !form.hidden.iter().any(|(k, _)| k == "ptlangsel") {
        ser.append_pair("ptlangsel", "ENG");
    }
    ser.append_pair("userid", userid_of(creds));
    ser.append_pair("pwd", creds.expose_password());
    let post = HttpRequest::post(
        url.clone(),
        "application/x-www-form-urlencoded",
        ser.finish(),
    )
    .stage("ps_login_post");
    let resp = sess.send(post)?;
    match resp.status {
        302 | 303 => {
            let loc = resp
                .location
                .as_deref()
                .ok_or(PsError::Unexpected("sign-in redirect without location"))?;
            let next = url
                .join(loc)
                .map_err(|_| PsError::Unexpected("bad redirect"))?;
            if !sess.cfg.host_allowed(&next) {
                return Err(PsError::Origin("sign-in redirect not allowed"));
            }
            if next.query().is_some_and(|q| q.contains("cmd=login")) {
                return Err(PsError::Auth(AuthFailure::CredentialsRejected));
            }
            // GET only from here on; a 307/308 would have been refused above (not 302/303).
            sess.get(next, "ps_login_landing").map(|_| ())
        }
        200 => {
            if bad_credentials_text(&resp.body) {
                return Err(PsError::Auth(AuthFailure::CredentialsRejected));
            }
            match pia::classify(&resp.body) {
                PageKind::LoginForm => Err(PsError::Auth(AuthFailure::CredentialsRejected)),
                _ => Err(PsError::Auth(AuthFailure::UnexpectedPage)),
            }
        }
        307 | 308 => Err(PsError::Auth(AuthFailure::RedirectRefused)),
        _ => Err(PsError::Auth(AuthFailure::UnexpectedPage)),
    }
}
