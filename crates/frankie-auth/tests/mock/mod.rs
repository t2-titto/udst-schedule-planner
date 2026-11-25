//! A synthetic Microsoft + D2L, in process. It serves only invented pages and values; there is no real service,
//! account or recorded page behind it. It exists so the whole flow runs offline with https URLs and the real
//! allow-list rules in force.
#![allow(dead_code)]
use frankie_auth::{AuthFailure, CookieMode, Http, HttpRequest, HttpResponse, Method};
use std::collections::HashMap;
use url::Url;

pub const USER: &str = "test-user@udst.edu.qa"; // gitleaks:allow (invented)
pub const PASSWORD: &str = "MOCK-PASSWORD-VALUE";
pub const SUBJECT: &str = "SUBJ-1";
pub const BEARER: &str = "mock-bearer-token-0123456789abcdef";
pub const NAV_STUDENT_NK: &str = "NK-1";
pub const NAV_BEARER: &str = "nav-campus-bearer-0123456789abcdef";
pub const NAV_ACS: &str = "https://bouncer.eab.com/sso/saml2/acs";
pub const ACS: &str = "https://d2l.udst.edu.qa/d2l/lp/auth/login/samlLogin.d2l";

#[derive(Clone)]
pub struct Scenario {
    pub kmsi: bool,
    pub kmsi_again: bool,
    pub wrong_password_page: bool,
    pub wrong_password_script_page: bool,
    pub challenge_text: Option<&'static str>,
    pub kmsi_challenge_text: Option<&'static str>,
    pub password_redirect_status: u16,
    pub redirect_host: Option<&'static str>,
    pub two_forms: bool,
    pub acs_url: &'static str,
    pub if_exists: i64,
    pub has_password: bool,
    pub bearer_subject: &'static str,
    pub no_token: bool,
    pub whoami_not_json: bool,
    pub acs_status: u16,
    pub redirect_loop: bool,
    // Navigate (shared Microsoft session)
    pub nav_no_session: bool,
    pub nav_challenge_text: Option<&'static str>,
    pub nav_acs_url: &'static str,
    pub nav_email: &'static str,
    pub nav_campus_rejects: bool,
    pub nav_bouncer_redirect: &'static str,
}

impl Default for Scenario {
    fn default() -> Self {
        Self {
            kmsi: false,
            kmsi_again: false,
            wrong_password_page: false,
            wrong_password_script_page: false,
            challenge_text: None,
            kmsi_challenge_text: None,
            password_redirect_status: 302,
            redirect_host: None,
            two_forms: false,
            acs_url: ACS,
            if_exists: 0,
            has_password: true,
            bearer_subject: SUBJECT,
            no_token: false,
            whoami_not_json: false,
            acs_status: 302,
            redirect_loop: false,
            nav_no_session: false,
            nav_challenge_text: None,
            nav_acs_url: NAV_ACS,
            nav_email: USER,
            nav_campus_rejects: false,
            nav_bouncer_redirect: "https://login.microsoftonline.com/TENANT2/saml2?SAMLRequest=nav",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Seen {
    pub method: Method,
    pub url: String,
    pub cookies: CookieMode,
    pub has_authorization: bool,
    pub stage: &'static str,
}

pub struct MockIdp {
    pub s: Scenario,
    pub password_posts: usize,
    pub acs_posts: usize,
    pub acs_done: bool,
    pub nav_acs_posts: usize,
    pub nav_session: bool,
    pub log: Vec<Seen>,
    /// Everything that would be visible outside the body: URLs and header values.
    pub exposed_text: String,
}

impl MockIdp {
    pub fn new(s: Scenario) -> Self {
        Self {
            s,
            password_posts: 0,
            acs_posts: 0,
            acs_done: false,
            nav_acs_posts: 0,
            nav_session: false,
            log: vec![],
            exposed_text: String::new(),
        }
    }

    fn page(status: u16, body: &str) -> HttpResponse {
        HttpResponse {
            status,
            location: None,
            content_type: Some("text/html".into()),
            body: body.into(),
        }
    }
    fn redirect(status: u16, to: &str) -> HttpResponse {
        HttpResponse {
            status,
            location: Some(to.into()),
            content_type: None,
            body: String::new(),
        }
    }
    fn json(body: serde_json::Value) -> HttpResponse {
        HttpResponse {
            status: 200,
            location: None,
            content_type: Some("application/json; charset=utf-8".into()),
            body: body.to_string(),
        }
    }
    fn config_page(extra: &str) -> HttpResponse {
        Self::page(
            200,
            &format!(
                "<html><script>var x=1; $Config={{{extra}}};</script><body>Sign in</body></html>"
            ),
        )
    }
    fn saml_page(&self) -> HttpResponse {
        let one = format!(
            r#"<form method="post" action="{}"><input type="hidden" name="SAMLResponse" value="ASSERTION-1"><input type="hidden" name="RelayState" value="rs"></form>"#,
            self.s.acs_url
        );
        let body = if self.s.two_forms {
            format!("{one}{one}")
        } else {
            one
        };
        Self::page(200, &format!("<html><body>{body}</body></html>"))
    }
    fn form(req: &HttpRequest) -> HashMap<String, String> {
        url::form_urlencoded::parse(req.body.as_ref().map(|b| b.as_bytes()).unwrap_or_default())
            .into_owned()
            .collect()
    }
}

impl Http for MockIdp {
    fn cookie_header(&self, url: &Url) -> Option<zeroize::Zeroizing<String>> {
        (self.nav_session && url.host_str() == Some("udst.navigate.eab.com"))
            .then(|| zeroize::Zeroizing::new("nav_session=MOCK-COOKIE".to_string()))
    }

    fn send(&mut self, req: HttpRequest) -> Result<HttpResponse, AuthFailure> {
        let url: Url = req.url.clone();
        let host = url.host_str().unwrap_or("").to_string();
        let path = url.path().to_string();
        self.exposed_text.push_str(url.as_str());
        for (k, v) in &req.headers {
            self.exposed_text.push_str(&format!(" {k}={}", v.as_str()));
        }
        self.log.push(Seen {
            method: req.method,
            url: format!("{host}{path}"),
            cookies: req.cookies,
            has_authorization: req
                .headers
                .iter()
                .any(|(k, _)| k.eq_ignore_ascii_case("authorization")),
            stage: req.stage,
        });
        let m = req.method;
        let s = self.s.clone();
        let resp = match (m, host.as_str(), path.as_str()) {
            (Method::Get, "d2l.udst.edu.qa", "/d2l/lp/auth/saml/initiate-login") => Self::redirect(
                302,
                "https://login.microsoftonline.com/TENANT/saml2?SAMLRequest=abc",
            ),
            (Method::Get, "login.microsoftonline.com", "/TENANT/saml2") => Self::config_page(
                r#""sFT":"FT0","sCtx":"CTX0","urlGetCredentialType":"https://login.microsoftonline.com/gct","urlPost":"/TENANT/login","canary":"CAN0","hisRegion":"r1","hisScaleUnit":"s1","sessionId":"SID0""#,
            ),
            (Method::Post, "login.microsoftonline.com", "/gct") => {
                let body: serde_json::Value =
                    serde_json::from_str(req.body.as_ref().unwrap()).unwrap();
                assert_eq!(body["username"], USER);
                Self::json(
                    serde_json::json!({"IfExistsResult": s.if_exists, "Credentials": {"HasPassword": s.has_password}, "FlowToken": "FT1"}),
                )
            }
            (Method::Post, "login.microsoftonline.com", "/TENANT/login") => {
                self.password_posts += 1;
                let f = Self::form(&req);
                if s.wrong_password_script_page {
                    Self::page(
                        200,
                        r#"<html><script>$Config={"sErrorCode":"50126","sFT":"x"};</script><body><div id="app"></div></body></html>"#,
                    )
                } else if f.get("passwd").map(String::as_str) != Some(PASSWORD)
                    || s.wrong_password_page
                {
                    Self::page(200, "<html><body><span>Your account or password is incorrect.</span></body></html>")
                } else if let Some(t) = s.challenge_text {
                    Self::page(200, &format!("<html><body><p>{t}</p></body></html>"))
                } else if s.kmsi {
                    Self::config_page(
                        r#""stsErrorCode":50140,"sFT":"FT2","sCtx":"CTX2","canary":"CAN2","correlationId":"CORR-1""#,
                    )
                } else if s.redirect_loop {
                    Self::redirect(302, "https://login.microsoftonline.com/TENANT/login2")
                } else {
                    let to = match s.redirect_host {
                        Some(h) => format!("https://{h}/next"),
                        None => "https://login.microsoftonline.com/next".to_string(),
                    };
                    Self::redirect(s.password_redirect_status, &to)
                }
            }
            (Method::Get, "login.microsoftonline.com", "/TENANT/login2") => {
                Self::redirect(302, "https://login.microsoftonline.com/TENANT/login2")
            }
            (Method::Post, "login.microsoftonline.com", "/kmsi") => {
                let f = Self::form(&req);
                assert_eq!(
                    (
                        f["LoginOptions"].as_str(),
                        f["type"].as_str(),
                        f["flowToken"].as_str(),
                        f["canary"].as_str()
                    ),
                    ("0", "28", "FT2", "CAN2")
                );
                if let Some(t) = s.kmsi_challenge_text {
                    Self::page(200, &format!("<html><body><p>{t}</p></body></html>"))
                } else if s.kmsi_again {
                    Self::config_page(
                        r#""stsErrorCode":50140,"sFT":"FT3","sCtx":"CTX3","canary":"CAN3""#,
                    )
                } else {
                    Self::redirect(302, "https://login.microsoftonline.com/next")
                }
            }
            (Method::Get, "login.microsoftonline.com", "/next") => self.saml_page(),
            (Method::Post, "d2l.udst.edu.qa", "/d2l/lp/auth/login/samlLogin.d2l") => {
                self.acs_posts += 1;
                assert_eq!(
                    Self::form(&req).get("SAMLResponse").map(String::as_str),
                    Some("ASSERTION-1")
                );
                self.acs_done = true;
                if s.acs_status == 302 {
                    Self::redirect(302, "/d2l/home")
                } else {
                    Self::page(s.acs_status, "x")
                }
            }
            (Method::Get, "d2l.udst.edu.qa", "/d2l/api/lp/1.49/users/whoami") => {
                if req.cookies == CookieMode::Jar {
                    if self.acs_done {
                        self.whoami(SUBJECT, s.whoami_not_json)
                    } else {
                        Self::page(401, "no session")
                    }
                } else {
                    let auth = req
                        .headers
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
                        .map(|(_, v)| v.as_str().to_string());
                    if auth.as_deref() == Some(&format!("Bearer {BEARER}")) {
                        self.whoami(s.bearer_subject, false)
                    } else {
                        Self::page(401, "no bearer")
                    }
                }
            }
            (Method::Get, "d2l.udst.edu.qa", "/d2l/lp/auth/xsrf-tokens") => {
                if self.acs_done {
                    Self::json(serde_json::json!({"referrerToken": "CSRF-1"}))
                } else {
                    Self::page(401, "no session")
                }
            }
            (Method::Post, "d2l.udst.edu.qa", "/d2l/lp/auth/oauth2/token") => {
                let csrf = req
                    .headers
                    .iter()
                    .find(|(k, _)| *k == "X-Csrf-Token")
                    .map(|(_, v)| v.as_str().to_string());
                assert_eq!(req.body.as_ref().map(|b| b.as_str()), Some("scope=*:*:*"));
                assert_eq!(
                    req.cookies,
                    CookieMode::Jar,
                    "minting needs the session cookies"
                );
                if csrf.as_deref() != Some("CSRF-1") || !self.acs_done {
                    Self::page(403, "no")
                } else if s.no_token {
                    Self::json(serde_json::json!({"token_type": "Bearer"}))
                } else {
                    Self::json(serde_json::json!({"access_token": BEARER, "token_type": "Bearer"}))
                }
            }
            // ---- Navigate360 on the shared Microsoft session ----
            (Method::Get, "udst.navigate.eab.com", "/api/v1/auth/login/") => {
                Self::redirect(302, "https://bouncer.eab.com/sso/login/xyz")
            }
            (Method::Get, "bouncer.eab.com", "/sso/login/xyz") => {
                Self::redirect(302, s.nav_bouncer_redirect)
            }
            (Method::Get, "login.microsoftonline.com", "/TENANT2/saml2") => {
                if let Some(t) = s.nav_challenge_text {
                    Self::page(200, &format!("<html><body><p>{t}</p></body></html>"))
                } else if self.acs_done && !s.nav_no_session {
                    Self::page(
                        200,
                        &format!(
                            r#"<html><body><form method="post" action="{}"><input type="hidden" name="SAMLResponse" value="NAV-ASSERTION-1"></form></body></html>"#,
                            s.nav_acs_url
                        ),
                    )
                } else {
                    Self::config_page(
                        r#""sFT":"FT0","sCtx":"CTX0","urlGetCredentialType":"https://login.microsoftonline.com/gct","urlPost":"/TENANT/login""#,
                    )
                }
            }
            (Method::Post, "bouncer.eab.com", "/sso/saml2/acs") => {
                self.nav_acs_posts += 1;
                assert_eq!(
                    Self::form(&req).get("SAMLResponse").map(String::as_str),
                    Some("NAV-ASSERTION-1")
                );
                self.nav_session = true;
                Self::redirect(302, "https://udst.navigate.eab.com/home")
            }
            (Method::Get, "udst.navigate.eab.com", "/home") => Self::page(200, "<html>home</html>"),
            (Method::Get, "udst.navigate.eab.com", "/api/v1/core/account/") => {
                if self.nav_session {
                    Self::json(
                        serde_json::json!({"userdata": {"id": 77, "student_nk": NAV_STUDENT_NK, "email": s.nav_email}}),
                    )
                } else {
                    Self::page(401, "no session")
                }
            }
            (Method::Get, "udst.navigate.eab.com", "/api/v1/ssccampus/token/") => {
                if self.nav_session {
                    Self::json(serde_json::json!({"access_token": NAV_BEARER}))
                } else {
                    Self::page(401, "no session")
                }
            }
            (Method::Get, "udst.campus.eab.com", "/api/v2/scheduling/current_user") => {
                let ok = req.cookies == CookieMode::None
                    && req.headers.iter().any(|(k, v)| {
                        k.eq_ignore_ascii_case("authorization")
                            && v.as_str() == format!("Bearer {NAV_BEARER}")
                    });
                if ok && !s.nav_campus_rejects {
                    Self::json(serde_json::json!({"id": 1, "role": "student"}))
                } else {
                    Self::page(401, "no bearer")
                }
            }
            _ => Self::page(404, "not found"),
        };
        Ok(resp)
    }
}

impl MockIdp {
    fn whoami(&self, id: &str, not_json: bool) -> HttpResponse {
        if not_json {
            Self::page(200, "<html>not json</html>")
        } else {
            Self::json(serde_json::json!({"Identifier": id, "FirstName": "Test"}))
        }
    }
}
