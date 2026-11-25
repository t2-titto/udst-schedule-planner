//! The only code that sends requests. Everything it sends passes the allow-list; reads and writes are separate
//! entry points, and a write needs a [`crate::gate::WriteProof`] that only the confirmation gate can create.
use crate::config::PeopleSoftConfig;
use crate::gate::WriteProof;
use crate::pia::{self, Ajax, FormState, PageKind};
use frankie_auth::{AuthFailure, Http, HttpRequest, HttpResponse, Method};
use frankie_contracts::{ErrorCode, ToolError};
use std::time::{Duration, Instant};
use url::Url;

#[derive(Debug, Clone, PartialEq)]
pub enum PsError {
    Auth(AuthFailure),
    /// The session is gone (login form, timeout banner, or a redirect to the sign-in page).
    NeedsLogin,
    NotAuthorized,
    /// A page or answer did not have the expected shape.
    Unexpected(&'static str),
    Origin(&'static str),
    /// A write was refused before anything was sent.
    WriteRefused(&'static str),
    /// A requested row, class number or option does not exist on the live page.
    NotFound(&'static str),
    InvalidInput(&'static str),
    TooLarge,
}

impl PsError {
    pub fn to_contract(&self, stage: &str) -> ToolError {
        let (code, msg): (ErrorCode, String) = match self {
            PsError::Auth(f) => return f.to_contract(),
            PsError::NeedsLogin => (ErrorCode::NeedsLogin, "sign-in required".into()),
            PsError::NotAuthorized => (
                ErrorCode::AccessDenied,
                "not authorized for this page".into(),
            ),
            PsError::Unexpected(m) => (ErrorCode::UnexpectedPage, (*m).into()),
            PsError::Origin(m) => (ErrorCode::OriginRejected, (*m).into()),
            PsError::WriteRefused(m) => (ErrorCode::ConfirmationRequired, (*m).into()),
            PsError::NotFound(m) => (ErrorCode::NotFound, (*m).into()),
            PsError::InvalidInput(m) => (ErrorCode::InvalidInput, (*m).into()),
            PsError::TooLarge => (ErrorCode::InvalidResponse, "response too large".into()),
        };
        ToolError::new(code, msg).with_stage(stage)
    }
}

impl From<AuthFailure> for PsError {
    fn from(f: AuthFailure) -> Self {
        PsError::Auth(f)
    }
}

/// A fetched page. `html` is page HTML or the FIELD html of an XML answer; never logged.
pub struct Page {
    pub url: Url,
    pub html: String,
    pub form: Option<FormState>,
    pub kind: PageKind,
}

impl std::fmt::Debug for Page {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Page({}{}, {} bytes, {:?})",
            self.url.host_str().unwrap_or("?"),
            self.url.path(),
            self.html.len(),
            self.kind
        )
    }
}

/// Which `ICAction` values change something on the server. Reads must never send these.
pub fn is_write_action(a: &str) -> bool {
    a == "VC_PE_SS_WRK_VC_SUBMIT_ENROL_PB"
        || a == "DERIVED_SSR_FL_SSR_DROPPED"
        || a == "#ICYes"
        || a == "CQ_SS_SWAP_WRK_CONTINUE_PB"
        || a == "VC_PE_SS_WRK_VC_SAVE_PLAN_PB"
        || a.starts_with("DESCR10$")
        || a.starts_with("SSR_CLSRCH_F_WK_SSR_OPTION_DESCR")
}

/// Sent on every request.
pub const USER_AGENT: &str = "FrankieConnector/0.1 (own-account student tool)";

pub struct Session<H: Http> {
    http: H,
    pub cfg: PeopleSoftConfig,
    last_at: Option<Instant>,
    pub requests_sent: u32,
}

impl<H: Http> Session<H> {
    pub fn new(http: H, cfg: PeopleSoftConfig) -> Self {
        Self {
            http,
            cfg,
            last_at: None,
            requests_sent: 0,
        }
    }

    pub fn http_mut(&mut self) -> &mut H {
        &mut self.http
    }

    fn pace(&mut self) {
        let gap = Duration::from_millis(self.cfg.min_interval_ms);
        if let Some(t) = self.last_at {
            let el = t.elapsed();
            if el < gap {
                std::thread::sleep(gap - el);
            }
        }
        self.last_at = Some(Instant::now());
    }

    pub(crate) fn send(&mut self, req: HttpRequest) -> Result<HttpResponse, PsError> {
        if !self.cfg.host_allowed(&req.url) {
            return Err(PsError::Origin("request target not allowed"));
        }
        self.pace();
        self.requests_sent += 1;
        // The site's browser check answers "Failed processing Browscap file" to a request with no User-Agent, so
        // every request names this tool honestly (it does not pretend to be a browser).
        let req = if req
            .headers
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case("user-agent"))
        {
            req
        } else {
            req.with_header("User-Agent", USER_AGENT.to_string())
        };
        let r = self.http.send(req)?;
        if r.body.len() > self.cfg.max_body_bytes {
            return Err(PsError::TooLarge);
        }
        Ok(r)
    }

    /// GET with at most a few redirects (301/302/303 only, same allow-list). A redirect to the sign-in page means
    /// the session is gone.
    pub fn get(&mut self, url: Url, stage: &'static str) -> Result<Page, PsError> {
        self.get_inner(url, None, stage)
    }

    /// Like [`Self::get`], naming the page the request comes from, as a browser does for the requests a page makes itself
    /// (the guide's grouplets). The referer must be on the allow-list.
    pub fn get_from(&mut self, url: Url, referer: &Url, stage: &'static str) -> Result<Page, PsError> {
        if !self.cfg.host_allowed(referer) {
            return Err(PsError::Origin("referer not allowed"));
        }
        self.get_inner(url, Some(referer.to_string()), stage)
    }

    fn get_inner(&mut self, url: Url, referer: Option<String>, stage: &'static str) -> Result<Page, PsError> {
        let mut cur = url;
        for hop in 0..=5usize {
            let mut rq = HttpRequest::get(cur.clone()).stage(stage);
            if let Some(r) = &referer {
                rq = rq.with_header("Referer", r.clone());
            }
            let r = self.send(rq)?;
            if (300..400).contains(&r.status) {
                if !matches!(r.status, 301..=303) {
                    return Err(PsError::Unexpected("redirect status not followed"));
                }
                let loc = r
                    .location
                    .as_deref()
                    .ok_or(PsError::Unexpected("redirect without location"))?;
                let next = cur
                    .join(loc)
                    .map_err(|_| PsError::Unexpected("bad redirect"))?;
                if !self.cfg.host_allowed(&next) {
                    return Err(PsError::Origin("redirect target not allowed"));
                }
                if next.query().is_some_and(|q| q.contains("cmd=login"))
                    || next.path().ends_with("/psp/")
                {
                    return Err(PsError::NeedsLogin);
                }
                if hop == 5 {
                    return Err(PsError::Unexpected("redirect limit"));
                }
                cur = next;
                continue;
            }
            let (page, script_redirect) = self.page_from(cur.clone(), r)?;
            match script_redirect {
                Some(next) if hop < 5 => {
                    if next.query().is_some_and(|q| q.contains("cmd=login")) {
                        return Err(PsError::NeedsLogin);
                    }
                    cur = next;
                }
                Some(_) => return Err(PsError::Unexpected("redirect limit")),
                None => return Ok(page),
            }
        }
        Err(PsError::Unexpected("redirect limit"))
    }

    /// Turn a 200 answer into a page. An XML answer (menu links asked with `ICAJAX=1`) has no `<form>` tag, so the
    /// form target is the component URL itself (same host, no query) and its hidden inputs come from the FIELD html.
    fn page_from(&self, url: Url, r: HttpResponse) -> Result<(Page, Option<Url>), PsError> {
        if r.status == 401 || r.status == 403 {
            return Err(PsError::NotAuthorized);
        }
        if r.status != 200 {
            return Err(PsError::Unexpected("unexpected http status"));
        }
        let ajax = pia::unwrap_ajax(&r.body);
        match pia::classify(&ajax.html) {
            PageKind::LoginForm | PageKind::SessionExpired => return Err(PsError::NeedsLogin),
            PageKind::NotAuthorized => return Err(PsError::NotAuthorized),
            PageKind::Content => {}
        }
        let redirect = match &ajax.redirect {
            Some(t) => Some(
                pia::vet_redirect(t, &url, &self.cfg)
                    .ok_or(PsError::Origin("script redirect target not allowed"))?,
            ),
            None => None,
        };
        let form = if ajax.is_xml {
            let mut action = url.clone();
            action.set_query(None);
            let mut f = FormState {
                action,
                fields: vec![],
            };
            f.absorb_hidden(&ajax.html);
            for (k, v) in &ajax.set_values {
                if k != "ICSID" && k != "ICAction" && k != "ICResubmit" {
                    f.set(k, v.clone());
                }
            }
            (self.cfg.host_allowed(&f.action) && f.get("ICSID").is_some()).then_some(f)
        } else {
            FormState::parse(&ajax.html, &url, &self.cfg).ok()
        };
        Ok((
            Page {
                url,
                html: ajax.html,
                form,
                kind: PageKind::Content,
            },
            redirect,
        ))
    }

    /// A read-step form POST (page navigation inside the same component). Refuses any write action.
    pub fn post(
        &mut self,
        prev: &Page,
        action: &str,
        extra: &[(&str, &str)],
        stage: &'static str,
    ) -> Result<Page, PsError> {
        if is_write_action(action) {
            return Err(PsError::WriteRefused(
                "write action needs a confirmed write",
            ));
        }
        self.post_inner(prev, action, extra, stage)
    }

    /// A write POST. The proof is consumed by the caller's gate and names exactly the action it allows.
    pub fn post_write(
        &mut self,
        proof: &WriteProof,
        prev: &Page,
        action: &str,
        extra: &[(&str, &str)],
        stage: &'static str,
    ) -> Result<Page, PsError> {
        if !is_write_action(action) {
            return Err(PsError::WriteRefused("not a write action"));
        }
        if !proof.allows(action) {
            return Err(PsError::WriteRefused(
                "confirmation does not cover this action",
            ));
        }
        self.post_inner(prev, action, extra, stage)
    }

    fn post_inner(
        &mut self,
        prev: &Page,
        action: &str,
        extra: &[(&str, &str)],
        stage: &'static str,
    ) -> Result<Page, PsError> {
        let form = prev
            .form
            .as_ref()
            .ok_or(PsError::Unexpected("page has no form"))?;
        let body = pia::build_post(form, action, extra)
            .map_err(|_| PsError::InvalidInput("bad action name"))?;
        let req = HttpRequest::post(
            form.action.clone(),
            "application/x-www-form-urlencoded",
            body,
        )
        .stage(stage);
        debug_assert!(req.method == Method::Post);
        let r = self.send(req)?;
        // A POST is never followed through a redirect (it would replay form data).
        if (300..400).contains(&r.status) {
            return Err(PsError::Unexpected("post answered with a redirect"));
        }
        if r.status != 200 {
            return Err(PsError::Unexpected("unexpected http status"));
        }
        let ajax: Ajax = pia::unwrap_ajax(&r.body);
        match pia::classify(&ajax.html) {
            PageKind::LoginForm | PageKind::SessionExpired => return Err(PsError::NeedsLogin),
            PageKind::NotAuthorized => return Err(PsError::NotAuthorized),
            PageKind::Content => {}
        }
        let mut next_form = form.clone();
        next_form.apply(&ajax);
        if let Some(target) = &ajax.redirect {
            let u = pia::vet_redirect(target, &form.action, &self.cfg)
                .ok_or(PsError::Origin("script redirect target not allowed"))?;
            if u.query().is_some_and(|q| q.contains("cmd=login")) {
                return Err(PsError::NeedsLogin);
            }
            return self.get(u, stage);
        }
        Ok(Page {
            url: form.action.clone(),
            html: ajax.html,
            form: Some(next_form),
            kind: PageKind::Content,
        })
    }
}
