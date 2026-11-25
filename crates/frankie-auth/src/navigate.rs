//! Navigate360 sign-in on the Microsoft session that D2L's sign-in already created. Port of
//! `navigate360_fresh.ps1 -ReuseMsSso`. There is NO password step here: the function takes a verified
//! `D2lArtifact` as proof that the shared session exists, and if Microsoft asks for anything (a sign-in form, MFA,
//! consent) the flow stops. It never falls back to sending a password.
use crate::artifact::D2lArtifact;
use crate::challenge::{classify_challenge, Challenge};
use crate::credentials::Credentials;
use crate::d2l::{follow_chain, form_body, send, FORM, MS_HOST};
use crate::failure::AuthFailure;
use crate::html::{exactly_one, find_saml_forms, AcsRule};
use crate::http::{Http, HttpRequest};
use crate::redirect::{follow_redirect, is_allowed_uri, RedirectDecision};
use serde_json::Value;
use std::time::Instant;
use url::Url;
use zeroize::Zeroizing;

const NAV_HOST: &str = "udst.navigate.eab.com";
const BOUNCER_HOST: &str = "bouncer.eab.com";
const CAMPUS_HOST: &str = "udst.campus.eab.com";
const MAX_CAMPUS_BODY: usize = 512 * 1024;

pub struct NavigateConfig {
    pub root: Url,
    pub campus_current_user: Url,
}

impl NavigateConfig {
    pub fn udst() -> Self {
        Self {
            root: Url::parse("https://udst.navigate.eab.com").expect("static url"),
            campus_current_user: Url::parse(
                "https://udst.campus.eab.com/api/v2/scheduling/current_user",
            )
            .expect("static url"),
        }
    }
}

/// A verified Navigate360 session: the student key, the Navigate cookie header and the campus bearer.
pub struct NavigateArtifact {
    student_nk: String,
    cookie_header: Zeroizing<String>,
    campus_bearer: Zeroizing<String>,
    pub elapsed_ms: u64,
}

impl NavigateArtifact {
    pub fn student_nk(&self) -> &str {
        &self.student_nk
    }
    /// For the private connector only. Never log it.
    pub fn campus_bearer(&self) -> &str {
        &self.campus_bearer
    }
    /// The shape `navigate360_fresh.ps1` prints. Contains the cookie header and the bearer: private pipe only.
    pub fn to_navigate_json(&self) -> Zeroizing<String> {
        Zeroizing::new(
            serde_json::json!({
                "ok": true,
                "provider": "navigate360",
                "credential_type": "composite",
                "verified_subject": self.student_nk,
                "navigate_cookie_header": self.cookie_header.as_str(),
                "campus_bearer": self.campus_bearer.as_str(),
                "identity_verified": true,
                "campus_api_verified": true,
                "session_verified": true,
                "http_status": 200,
                "elapsed_ms": self.elapsed_ms,
            })
            .to_string(),
        )
    }
}

impl std::fmt::Debug for NavigateArtifact {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NavigateArtifact([REDACTED])")
    }
}

fn json_body(resp: &crate::http::HttpResponse) -> Result<Value, AuthFailure> {
    if resp.status != 200 || !resp.is_json() {
        return Err(AuthFailure::UnexpectedPage);
    }
    serde_json::from_str(&resp.body).map_err(|_| AuthFailure::UnexpectedPage)
}

fn text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn valid_student_nk(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Sign in to Navigate360 using the Microsoft session already in `http`'s cookie jar.
/// `_session` proves a D2L sign-in just verified that session; `creds` supplies only the expected account name
/// (the password is never read here).
pub fn authenticate_navigate(
    http: &mut dyn Http,
    creds: &Credentials,
    _session: &D2lArtifact,
    cfg: &NavigateConfig,
) -> Result<NavigateArtifact, AuthFailure> {
    let started = Instant::now();
    let nav = [NAV_HOST];
    let bouncer = [BOUNCER_HOST];
    let ms = [MS_HOST];
    let eab = [BOUNCER_HOST, NAV_HOST];
    let campus = [CAMPUS_HOST];

    // The campus API is the last host we call: open its connection while the earlier steps run.
    let campus_root = Url::parse("https://udst.campus.eab.com/").expect("static url");
    if is_allowed_uri(&campus_root, &campus) {
        http.prewarm(&campus_root);
    }

    // 1. Navigate starts SAML and redirects to the EAB bouncer.
    let start = cfg
        .root
        .join("/api/v1/auth/login/")
        .map_err(|_| AuthFailure::InvalidInput)?;
    let r = send(
        http,
        HttpRequest::get(start.clone()).stage("nav_initiate"),
        &nav,
    )?;
    let bouncer_url = match follow_redirect(r.status, r.location.as_deref(), &start, &bouncer, 0)? {
        RedirectDecision::Follow(u) if u.path().starts_with("/sso/login/") => u,
        _ => return Err(AuthFailure::UnexpectedPage),
    };

    // 2. The bouncer redirects to Microsoft's SAML endpoint.
    let b = send(
        http,
        HttpRequest::get(bouncer_url.clone()).stage("nav_eab_sso"),
        &bouncer,
    )?;
    let ms_url = match follow_redirect(b.status, b.location.as_deref(), &bouncer_url, &ms, 0)? {
        RedirectDecision::Follow(u) if u.path().ends_with("/saml2") => u,
        _ => return Err(AuthFailure::UnexpectedPage),
    };

    // 3. Microsoft must hand back the SAML form from the existing session. Anything else stops the flow.
    let page = send(
        http,
        HttpRequest::get(ms_url.clone()).stage("nav_ms_session"),
        &ms,
    )?;
    if page.status != 200 {
        return Err(AuthFailure::UnexpectedPage);
    }
    match classify_challenge(&page.body) {
        Challenge::None => {}
        kind => return Err(AuthFailure::ChallengeRequired(kind)),
    }
    let form = exactly_one(find_saml_forms(&page.body, &ms_url, &AcsRule::navigate()))?;

    // 4. The assertion goes to the exact EAB ACS and only there; then the EAB hand-off redirects (GET only).
    let pairs: Vec<(&str, &str)> = form
        .fields
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let acs = send(
        http,
        HttpRequest::post(form.target.clone(), FORM, form_body(&pairs)).stage("nav_saml_acs"),
        &bouncer,
    )?;
    let (done, _) = follow_chain(http, acs, form.target.clone(), &eab, "nav_handoff")?;
    if !(200..300).contains(&done.status) {
        return Err(AuthFailure::UnexpectedPage);
    }

    // 5. Bind the Navigate session to the account that signed in (not to whichever account was in the jar).
    let account_url = cfg
        .root
        .join("/api/v1/core/account/")
        .map_err(|_| AuthFailure::InvalidInput)?;
    let acct = json_body(&send(
        http,
        HttpRequest::get(account_url).stage("nav_account"),
        &nav,
    )?)?;
    let user = acct.get("userdata").ok_or(AuthFailure::UnexpectedPage)?;
    let (id, nk, email) = (
        text(user.get("id")),
        text(user.get("student_nk")),
        text(user.get("email")),
    );
    let expected = creds.username().split('@').next().unwrap_or("");
    let actual_ok = email
        .strip_suffix("@udst.edu.qa")
        .is_some_and(|l| !l.is_empty() && !l.contains('@') && !l.chars().any(char::is_whitespace));
    let actual = email.split('@').next().unwrap_or("");
    if id.is_empty()
        || !valid_student_nk(&nk)
        || !actual_ok
        || expected.is_empty()
        || !actual.eq_ignore_ascii_case(expected)
    {
        return Err(AuthFailure::IdentityMismatch);
    }
    let cookie_header = http
        .cookie_header(&cfg.root)
        .filter(|c| !c.trim().is_empty() && c.len() <= 16_000)
        .ok_or(AuthFailure::UnexpectedPage)?;

    // 6. Campus bearer.
    let token_url = cfg
        .root
        .join("/api/v1/ssccampus/token/?scheduling_api=None")
        .map_err(|_| AuthFailure::InvalidInput)?;
    let tok = json_body(&send(
        http,
        HttpRequest::get(token_url).stage("nav_campus_token"),
        &nav,
    )?)?;
    let bearer = tok
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|t| t.len() >= 24 && t.len() <= 32_768 && !t.chars().any(char::is_whitespace))
        .ok_or(AuthFailure::UnexpectedPage)?
        .to_string();

    // 7. Prove the bearer works on its own: no cookies, campus host only.
    let verify = HttpRequest::get(cfg.campus_current_user.clone())
        .stage("nav_campus_verify")
        .without_cookies()
        .with_header("Authorization", format!("Bearer {bearer}"))
        .with_header(
            "Accept",
            "application/json, application/vnd.eab+json".into(),
        );
    let v = send(http, verify, &campus)?;
    if v.status != 200 || !v.is_json() || v.body.len() > MAX_CAMPUS_BODY {
        return Err(AuthFailure::UnexpectedPage);
    }
    let who: Value = serde_json::from_str(&v.body).map_err(|_| AuthFailure::UnexpectedPage)?;
    if !who.as_object().is_some_and(|o| !o.is_empty()) {
        return Err(AuthFailure::IdentityMismatch);
    }

    Ok(NavigateArtifact {
        student_nk: nk,
        cookie_header,
        campus_bearer: Zeroizing::new(bearer),
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}
