use crate::failure::AuthFailure;
use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;
use zeroize::Zeroize;

/// The four values the Microsoft sign-in page embeds for the next steps. `flow_token` and `context`
/// are single-use secrets for that page: they are zeroized on drop and never printed.
pub struct SigninConfig {
    flow_token: String,
    context: String,
    canary: String,
    pub url_get_credential_type: String,
    pub url_post: String,
    pub his_region: String,
    pub his_scale_unit: String,
    /// `hpgrequestid`, or `sessionId` when that is all the page has (as in the PowerShell).
    pub request_id: String,
}

impl SigninConfig {
    pub fn flow_token(&self) -> &str {
        &self.flow_token
    }
    pub fn context(&self) -> &str {
        &self.context
    }
    /// Empty when the page has none (the PowerShell only sends `canary` when present).
    pub fn canary(&self) -> &str {
        &self.canary
    }
}
impl Drop for SigninConfig {
    fn drop(&mut self) {
        self.flow_token.zeroize();
        self.context.zeroize();
        self.canary.zeroize();
    }
}
impl std::fmt::Debug for SigninConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SigninConfig([REDACTED])")
    }
}

/// The first `$Config = {...};` block of a page, parsed as JSON. `None` if absent or not valid JSON.
pub fn page_config_json(page: &str) -> Option<Value> {
    static CONFIG: OnceLock<Regex> = OnceLock::new();
    let re = CONFIG
        .get_or_init(|| Regex::new(r"(?is)\$Config\s*=\s*(\{.+?\});").expect("static pattern"));
    serde_json::from_str(&re.captures(page)?[1]).ok()
}

/// Port of the `$Config` extraction (d2l_auth.ps1 ~line 514): the first `$Config = {...};` block, parsed as JSON,
/// requiring `sFT`, `sCtx`, `urlGetCredentialType` and `urlPost` to be non-empty strings.
pub fn extract_signin_config(page: &str) -> Result<SigninConfig, AuthFailure> {
    let json = page_config_json(page).ok_or(AuthFailure::UnexpectedPage)?;
    let get = |k: &str| {
        json.get(k)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let opt = |k: &str| {
        json.get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    match (
        get("sFT"),
        get("sCtx"),
        get("urlGetCredentialType"),
        get("urlPost"),
    ) {
        (Some(flow_token), Some(context), Some(url_get_credential_type), Some(url_post)) => {
            let request_id = get("hpgrequestid")
                .or_else(|| get("sessionId"))
                .unwrap_or_default();
            Ok(SigninConfig {
                flow_token,
                context,
                canary: opt("canary"),
                url_get_credential_type,
                url_post,
                his_region: opt("hisRegion"),
                his_scale_unit: opt("hisScaleUnit"),
                request_id,
            })
        }
        _ => Err(AuthFailure::ConfigIncomplete),
    }
}
