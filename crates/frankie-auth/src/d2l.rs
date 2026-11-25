use crate::artifact::D2lArtifact;
use crate::challenge::{classify_challenge, Challenge};
use crate::config::{extract_signin_config, page_config_json};
use crate::credentials::Credentials;
use crate::failure::AuthFailure;
use crate::html::{exactly_one, find_saml_forms, AcsRule};
use crate::http::{Http, HttpRequest, HttpResponse};
use crate::redirect::{follow_redirect, is_allowed_uri, RedirectDecision};
use serde_json::{json, Value};
use std::time::Instant;
use url::{form_urlencoded, Url};

pub(crate) const MS_HOST: &str = "login.microsoftonline.com";
const D2L_HOST: &str = "d2l.udst.edu.qa";
pub(crate) const FORM: &str = "application/x-www-form-urlencoded";

/// Where D2L lives and which Microsoft tenant signs it in. Values are the ones in `d2l_auth.ps1`.
pub struct D2lConfig {
    pub root: Url,
    pub tenant: String,
}

impl D2lConfig {
    pub fn udst() -> Self {
        Self {
            root: Url::parse("https://d2l.udst.edu.qa").expect("static url"),
            tenant: "b30f4b44-46c6-4070-9997-f87b38d4771c".into(),
        }
    }
}

pub(crate) fn form_body(fields: &[(&str, &str)]) -> String {
    let mut s = form_urlencoded::Serializer::new(String::new());
    for (k, v) in fields {
        s.append_pair(k, v);
    }
    s.finish()
}

/// Every request goes through here: https, default port, no userinfo, and a host on the list for that step.
pub(crate) fn send(
    http: &mut dyn Http,
    req: HttpRequest,
    hosts: &[&str],
) -> Result<HttpResponse, AuthFailure> {
    if !is_allowed_uri(&req.url, hosts) {
        return Err(AuthFailure::HostNotAllowed);
    }
    http.send(req)
}

/// Follow 301/302/303 with GET only until a non-redirect, from `response` received at `base`.
pub(crate) fn follow_chain(
    http: &mut dyn Http,
    mut response: HttpResponse,
    mut base: Url,
    hosts: &[&str],
    stage: &'static str,
) -> Result<(HttpResponse, Url), AuthFailure> {
    let mut hops = 0;
    loop {
        match follow_redirect(
            response.status,
            response.location.as_deref(),
            &base,
            hosts,
            hops,
        )? {
            RedirectDecision::Done => return Ok((response, base)),
            RedirectDecision::Follow(next) => {
                hops += 1;
                response = send(http, HttpRequest::get(next.clone()).stage(stage), hosts)?;
                base = next;
            }
        }
    }
}

fn identifier(resp: &HttpResponse) -> Option<String> {
    if resp.status != 200 || !resp.is_json() {
        return None;
    }
    let v: Value = serde_json::from_str(&resp.body).ok()?;
    let id = match v.get("Identifier")? {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    (!id.is_empty() && id.len() <= 64 && !id.chars().any(char::is_control)).then_some(id)
}

pub(crate) fn json_field(resp: &HttpResponse, key: &str) -> Option<String> {
    let v: Value = serde_json::from_str(&resp.body).ok()?;
    v.get(key)?
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub(crate) fn stop_on_challenge(page: &str) -> Result<(), AuthFailure> {
    match classify_challenge(page) {
        Challenge::None => Ok(()),
        Challenge::CredentialRejected => Err(AuthFailure::CredentialsRejected),
        kind => Err(AuthFailure::ChallengeRequired(kind)),
    }
}

/// Sign in to D2L through Microsoft and return a verified bearer. Port of `d2l_auth.ps1`.
///
/// One attempt only: this function never retries a password, and any challenge stops it.
pub fn authenticate_d2l(
    http: &mut dyn Http,
    creds: &Credentials,
    cfg: &D2lConfig,
) -> Result<D2lArtifact, AuthFailure> {
    let started = Instant::now();
    let d2l = [D2L_HOST];
    let ms = [MS_HOST];
    let both = [MS_HOST, D2L_HOST];

    // Open the Microsoft connection in the background while D2L answers request 1 (no data is sent).
    let ms_root = Url::parse("https://login.microsoftonline.com/").expect("static url");
    if is_allowed_uri(&ms_root, &ms) {
        http.prewarm(&ms_root);
    }

    // 1. D2L starts the SAML flow and redirects to Microsoft.
    let start = cfg
        .root
        .join(&format!(
            "/d2l/lp/auth/saml/initiate-login?entityId=https%3a%2f%2fsts.windows.net%2f{}%2f",
            cfg.tenant
        ))
        .map_err(|_| AuthFailure::InvalidInput)?;
    let r = send(
        http,
        HttpRequest::get(start.clone()).stage("d2l_initiate"),
        &d2l,
    )?;
    let ms_url = match follow_redirect(r.status, r.location.as_deref(), &start, &ms, 0)? {
        RedirectDecision::Follow(u) if u.path().ends_with("/saml2") => u,
        _ => return Err(AuthFailure::UnexpectedPage),
    };

    // 2. Microsoft sign-in page and its embedded configuration.
    let page = send(
        http,
        HttpRequest::get(ms_url.clone()).stage("ms_signin_page"),
        &ms,
    )?;
    if page.status != 200 {
        return Err(AuthFailure::UnexpectedPage);
    }
    let config = extract_signin_config(&page.body)?;

    // 3. Account discovery.
    let gct =
        Url::parse(&config.url_get_credential_type).map_err(|_| AuthFailure::UnexpectedPage)?;
    let discovery = json!({
        "username": creds.username(), "isOtherIdpSupported": true, "checkPhones": true, "isRemoteNGCSupported": true,
        "isCookieBannerShown": false, "isFidoSupported": true, "originalRequest": config.context(), "country": "QA",
        "forceotclogin": false, "isExternalFederationDisallowed": false, "isRemoteConnectSupported": false, "federationFlags": 0,
        "isSignup": false, "flowToken": config.flow_token(), "isAccessPassSupported": true, "isQrCodePinSupported": true
    });
    let d = send(
        http,
        HttpRequest::post(gct, "application/json", discovery.to_string())
            .stage("ms_account_discovery"),
        &ms,
    )?;
    if d.status != 200 {
        return Err(AuthFailure::UnexpectedPage);
    }
    let reply: Value = serde_json::from_str(&d.body).map_err(|_| AuthFailure::UnexpectedPage)?;
    if reply.get("IfExistsResult").and_then(Value::as_i64) != Some(0) {
        return Err(AuthFailure::CredentialsRejected); // account not recognised: stop, do not try a password
    }
    if reply
        .pointer("/Credentials/HasPassword")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return Err(AuthFailure::UnexpectedPage); // password sign-in not offered
    }
    let fresh_flow = reply
        .get("FlowToken")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or(AuthFailure::UnexpectedPage)?
        .to_string();

    // 4. Password: exactly one POST. It is never repeated and never replayed by a redirect.
    let post_url = ms_url
        .join(&config.url_post)
        .map_err(|_| AuthFailure::UnexpectedPage)?;
    let mut fields: Vec<(&str, &str)> = vec![
        ("i13", "0"),
        ("login", creds.username()),
        ("loginfmt", creds.username()),
        ("type", "11"),
        ("LoginOptions", "3"),
        ("lrt", ""),
        ("lrtPartition", ""),
        ("hisRegion", &config.his_region),
        ("hisScaleUnit", &config.his_scale_unit),
        ("passwd", creds.expose_password()),
        ("ps", "2"),
        ("psRNGCDefaultType", ""),
        ("psRNGCEntropy", ""),
        ("psRNGCSLK", ""),
        ("ctx", config.context()),
        ("flowToken", &fresh_flow),
    ];
    if !config.canary().is_empty() {
        fields.push(("canary", config.canary()));
    }
    if !config.request_id.is_empty() {
        fields.push(("hpgrequestid", &config.request_id));
    }
    let pw = send(
        http,
        HttpRequest::post(post_url.clone(), FORM, form_body(&fields)).stage("ms_password"),
        &ms,
    )?;
    drop(fields);
    let (mut resp, mut base) = follow_chain(http, pw, post_url, &both, "ms_password_redirect")?;

    // 5. Challenge or "stay signed in" step. Challenges stop the flow; the KMSI step is answered once.
    stop_on_challenge(&resp.body)?;
    if let Some(cfg_json) = page_config_json(&resp.body) {
        let sts = cfg_json.get("stsErrorCode");
        let is_kmsi = sts.and_then(Value::as_i64) == Some(50140)
            || sts.and_then(Value::as_str) == Some("50140");
        if is_kmsi {
            if base.host_str() != Some(MS_HOST) || resp.status != 200 {
                return Err(AuthFailure::UnexpectedPage);
            }
            let get = |k: &str| {
                cfg_json
                    .get(k)
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty())
            };
            let (Some(sft), Some(sctx), Some(canary)) = (get("sFT"), get("sCtx"), get("canary"))
            else {
                return Err(AuthFailure::ConfigIncomplete);
            };
            let mut kmsi: Vec<(&str, &str)> = vec![
                ("LoginOptions", "0"),
                ("type", "28"),
                ("ctx", sctx),
                ("flowToken", sft),
                ("canary", canary),
            ];
            let correlation = cfg_json
                .get("correlationId")
                .and_then(Value::as_str)
                .unwrap_or("");
            if correlation.len() > 256 || correlation.chars().any(char::is_control) {
                return Err(AuthFailure::UnexpectedPage);
            }
            if !correlation.is_empty() {
                kmsi.push(("hpgrequestid", correlation));
            }
            let kmsi_url =
                Url::parse("https://login.microsoftonline.com/kmsi").expect("static url");
            let k = send(
                http,
                HttpRequest::post(kmsi_url.clone(), FORM, form_body(&kmsi)).stage("ms_kmsi"),
                &ms,
            )?;
            (resp, base) = follow_chain(http, k, kmsi_url, &both, "ms_kmsi_redirect")?;
            stop_on_challenge(&resp.body)?;
            if page_config_json(&resp.body)
                .and_then(|c| c.get("stsErrorCode").cloned())
                .is_some_and(|s| s.as_i64() == Some(50140) || s.as_str() == Some("50140"))
            {
                return Err(AuthFailure::UnexpectedPage); // asked again: do not loop
            }
        }
    }

    // 6. The SAML assertion goes to the exact D2L ACS, and only there.
    let form = exactly_one(find_saml_forms(&resp.body, &base, &AcsRule::d2l()))?;
    let pairs: Vec<(&str, &str)> = form
        .fields
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let acs = send(
        http,
        HttpRequest::post(form.target.clone(), FORM, form_body(&pairs)).stage("d2l_saml_acs"),
        &d2l,
    )?;
    match acs.status {
        301..=303 => {
            let loc = acs.location.as_deref().ok_or(AuthFailure::UnexpectedPage)?;
            let next = form
                .target
                .join(loc)
                .map_err(|_| AuthFailure::UnexpectedPage)?;
            if !is_allowed_uri(&next, &d2l) {
                return Err(AuthFailure::HostNotAllowed);
            }
        }
        200..=299 => {}
        _ => return Err(AuthFailure::UnexpectedPage),
    }

    // 7. Prove the session belongs to a user, then mint a bearer and prove the bearer belongs to the same user.
    let whoami = cfg
        .root
        .join("/d2l/api/lp/1.49/users/whoami")
        .map_err(|_| AuthFailure::InvalidInput)?;
    let session_subject = identifier(&send(
        http,
        HttpRequest::get(whoami.clone()).stage("d2l_whoami"),
        &d2l,
    )?)
    .ok_or(AuthFailure::UnexpectedPage)?;

    let xsrf = send(
        http,
        HttpRequest::get(
            cfg.root
                .join("/d2l/lp/auth/xsrf-tokens")
                .map_err(|_| AuthFailure::InvalidInput)?,
        )
        .stage("d2l_xsrf"),
        &d2l,
    )?;
    if xsrf.status != 200 {
        return Err(AuthFailure::UnexpectedPage);
    }
    let csrf = json_field(&xsrf, "referrerToken").ok_or(AuthFailure::UnexpectedPage)?;

    let token_url = cfg
        .root
        .join("/d2l/lp/auth/oauth2/token")
        .map_err(|_| AuthFailure::InvalidInput)?;
    let tok = send(
        http,
        HttpRequest::post(token_url, FORM, "scope=*:*:*".into())
            .stage("d2l_token_mint")
            .with_header("X-Csrf-Token", csrf),
        &d2l,
    )?;
    if tok.status != 200 {
        return Err(AuthFailure::UnexpectedPage);
    }
    let bearer = json_field(&tok, "access_token").ok_or(AuthFailure::UnexpectedPage)?;

    let verify = HttpRequest::get(whoami)
        .stage("d2l_bearer_verify")
        .without_cookies()
        .with_header("Authorization", format!("Bearer {bearer}"));
    let bearer_subject =
        identifier(&send(http, verify, &d2l)?).ok_or(AuthFailure::IdentityMismatch)?;
    if bearer_subject != session_subject {
        return Err(AuthFailure::IdentityMismatch);
    }
    Ok(D2lArtifact::new(
        bearer,
        session_subject,
        started.elapsed().as_millis() as u64,
    ))
}
