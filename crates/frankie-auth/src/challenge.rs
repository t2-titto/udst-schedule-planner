use regex::Regex;
use std::sync::OnceLock;

/// What kind of interactive step the identity provider asked for. The flow stops on all of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Challenge {
    MfaOrIdentityVerification,
    PolicyBlocked,
    DeviceRequirement,
    ConsentOrAdminApproval,
    CredentialRejected,
    None,
}

fn re(cell: &'static OnceLock<Regex>, pat: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pat).expect("static pattern"))
}

/// Port of `Get-Challenge` (d2l_auth.ps1). Looks at AADSTS codes anywhere in the page, and at the
/// *visible* text (script and style content removed, tags removed, entities decoded, lower-cased).
pub fn classify_challenge(page: &str) -> Challenge {
    static CODES: OnceLock<Regex> = OnceLock::new();
    static SCRIPT: OnceLock<Regex> = OnceLock::new();
    static STYLE: OnceLock<Regex> = OnceLock::new();
    static TAGS: OnceLock<Regex> = OnceLock::new();
    static MFA: OnceLock<Regex> = OnceLock::new();
    static POLICY: OnceLock<Regex> = OnceLock::new();
    static DEVICE: OnceLock<Regex> = OnceLock::new();
    static CONSENT: OnceLock<Regex> = OnceLock::new();
    static REJECTED: OnceLock<Regex> = OnceLock::new();

    let codes: Vec<String> = re(&CODES, r"(?i)AADSTS(\d+)")
        .captures_iter(page)
        .map(|c| c[1].to_string())
        .collect();
    // The real wrong-password page carries only `"sErrorCode":"50126"` in its script config (no AADSTS text; seen
    // live 2026-11-11), so that number counts as a code too. Informational codes such as 50058 (no session yet)
    // are not in any list below and are ignored.
    static SERR: OnceLock<Regex> = OnceLock::new();
    let mut codes = codes;
    codes.extend(
        re(&SERR, r#""sErrorCode"\s*:\s*"?(\d+)"#)
            .captures_iter(page)
            .map(|c| c[1].to_string()),
    );
    let has = |wanted: &[&str]| codes.iter().any(|c| wanted.contains(&c.as_str()));

    let no_script = re(&SCRIPT, r"(?is)<script\b[^>]*>.*?</script\s*>").replace_all(page, " ");
    let no_style = re(&STYLE, r"(?is)<style\b[^>]*>.*?</style\s*>").replace_all(&no_script, " ");
    let no_tags = re(&TAGS, r"<[^>]*>").replace_all(&no_style, " ");
    let visible = html_escape::decode_html_entities(&no_tags).to_lowercase();

    if has(&["50074", "50076", "50079"])
        || re(&MFA, r"multi.factor|verification code|approve sign.in|verify your identity|more information required|authenticator").is_match(&visible)
    {
        return Challenge::MfaOrIdentityVerification;
    }
    if has(&["53000", "53003"])
        || re(
            &POLICY,
            r"conditional access|device must be compliant|access blocked by policy",
        )
        .is_match(&visible)
    {
        return Challenge::PolicyBlocked;
    }
    if has(&["50097"])
        || re(
            &DEVICE,
            r"device authentication|required device|register your device",
        )
        .is_match(&visible)
    {
        return Challenge::DeviceRequirement;
    }
    if has(&["65001"]) || re(&CONSENT, r"consent required|admin approval").is_match(&visible) {
        return Challenge::ConsentOrAdminApproval;
    }
    // Microsoft's own error codes for a bad sign-in. The sign-in page is rendered by script, so the wording is
    // often NOT in the visible text; the code is in the page's `$Config` (`sErrorCode` and the AADSTS message).
    // 50126 wrong password, 50034 unknown user -> rejected. 50053 locked out, 50055 password expired,
    // 50057 account disabled -> stop and tell a human (never retry).
    if has(&["50053", "50055", "50057"]) {
        return Challenge::PolicyBlocked;
    }
    if has(&["50126", "50034"]) {
        return Challenge::CredentialRejected;
    }
    if re(&REJECTED, r"incorrect password|account or password is incorrect|invalid username or password|sign.in failed").is_match(&visible) {
        return Challenge::CredentialRejected;
    }
    Challenge::None
}
