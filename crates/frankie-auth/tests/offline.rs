use frankie_auth::*;
use frankie_contracts::ErrorCode;
use url::Url;

fn base() -> Url {
    Url::parse("https://d2l.udst.edu.qa/d2l/login").unwrap()
}
const OK: &str = "https://d2l.udst.edu.qa/d2l/lp/auth/login/samlLogin.d2l";

// ---------- the 10 D2L SAML fixtures ported from d2l_auth.ps1 (names kept) ----------

fn fixtures() -> Vec<(&'static str, Option<&'static str>, String, bool)> {
    vec![
        ("exact_absolute_post", None, format!(r#"<form method="post" action="{OK}"><input type="hidden" name="SAMLResponse" value="SYNTHETIC"></form>"#), true),
        ("external_form_associated_input", None, format!(r#"<form id="target" method="post" action="{OK}"></form><input form="target" name="SAMLResponse" value="SYNTHETIC">"#), true),
        ("external_unassociated_input_rejected", None, format!(r#"<form id="target" method="post" action="{OK}"></form><input name="SAMLResponse" value="SYNTHETIC">"#), false),
        ("exact_relative_post", Some("https://d2l.udst.edu.qa/d2l/login"), r#"<form method="post" action="/d2l/lp/auth/login/samlLogin.d2l"><input name="SAMLResponse" value="SYNTHETIC"></form>"#.into(), true),
        ("old_nonmatching_saml_path", None, r#"<form method="post" action="https://d2l.udst.edu.qa/d2l/lp/auth/saml/samlLogin.d2l"><input name="SAMLResponse" value="SYNTHETIC"></form>"#.into(), false),
        ("wrong_host", None, r#"<form method="post" action="https://example.invalid/d2l/lp/auth/login/samlLogin.d2l"><input name="SAMLResponse" value="SYNTHETIC"></form>"#.into(), false),
        ("insecure_scheme", None, r#"<form method="post" action="http://d2l.udst.edu.qa/d2l/lp/auth/login/samlLogin.d2l"><input name="SAMLResponse" value="SYNTHETIC"></form>"#.into(), false),
        ("query_not_exact_acs", None, format!(r#"<form method="post" action="{OK}?x=1"><input name="SAMLResponse" value="SYNTHETIC"></form>"#), false),
        ("get_method", None, format!(r#"<form method="get" action="{OK}"><input name="SAMLResponse" value="SYNTHETIC"></form>"#), false),
        ("missing_saml_response", None, format!(r#"<form method="post" action="{OK}"><input name="RelayState" value="SYNTHETIC"></form>"#), false),
    ]
}

#[test]
fn the_ten_ported_saml_fixtures_behave_as_in_powershell() {
    let all = fixtures();
    assert_eq!(all.len(), 10);
    for (name, b, html, expected) in all {
        let b = b.map(|s| Url::parse(s).unwrap()).unwrap_or_else(base);
        let got = !find_saml_forms(&html, &b, &AcsRule::d2l()).is_empty();
        assert_eq!(got, expected, "fixture {name}");
    }
}

#[test]
fn accepted_form_carries_the_target_and_all_linked_fields() {
    let html = format!(
        r#"<form id="f" method="POST" action="{OK}"><input name="RelayState" value="r&amp;s"></form><input form="f" name="SAMLResponse" value="SYNTHETIC">"#
    );
    let forms = find_saml_forms(&html, &base(), &AcsRule::d2l());
    assert_eq!(forms.len(), 1);
    assert_eq!(forms[0].target.as_str(), OK);
    assert_eq!(forms[0].fields["RelayState"], "r&s", "entities are decoded");
    assert_eq!(forms[0].fields["SAMLResponse"], "SYNTHETIC");
}

// ---------- ACS strictness beyond the original fixtures ----------

#[test]
fn acs_rejects_userinfo_fragment_port_case_and_lookalike_hosts() {
    let bad = [
        "https://user@d2l.udst.edu.qa/d2l/lp/auth/login/samlLogin.d2l",
        "https://d2l.udst.edu.qa/d2l/lp/auth/login/samlLogin.d2l#frag",
        "https://d2l.udst.edu.qa:8443/d2l/lp/auth/login/samlLogin.d2l",
        "https://d2l.udst.edu.qa/D2L/lp/auth/login/samlLogin.d2l",
        "https://d2l.udst.edu.qa.evil.example/d2l/lp/auth/login/samlLogin.d2l",
        "https://evil.example/@d2l.udst.edu.qa/d2l/lp/auth/login/samlLogin.d2l",
        "https://d2l.udst.edu.qa/d2l/lp/auth/login/samlLogin.d2l/extra",
    ];
    for action in bad {
        let html = format!(
            r#"<form method="post" action="{action}"><input name="SAMLResponse" value="SYNTHETIC"></form>"#
        );
        assert!(
            find_saml_forms(&html, &base(), &AcsRule::d2l()).is_empty(),
            "must reject {action}"
        );
    }
    // explicit default port is the same endpoint (as in .NET IsDefaultPort)
    let html = r#"<form method="post" action="https://d2l.udst.edu.qa:443/d2l/lp/auth/login/samlLogin.d2l"><input name="SAMLResponse" value="SYNTHETIC"></form>"#;
    assert_eq!(find_saml_forms(html, &base(), &AcsRule::d2l()).len(), 1);
}

#[test]
fn empty_saml_response_and_missing_action_are_rejected() {
    let empty =
        format!(r#"<form method="post" action="{OK}"><input name="SAMLResponse" value=""></form>"#);
    assert!(find_saml_forms(&empty, &base(), &AcsRule::d2l()).is_empty());
    let no_action = r#"<form method="post"><input name="SAMLResponse" value="SYNTHETIC"></form>"#;
    assert!(find_saml_forms(no_action, &base(), &AcsRule::d2l()).is_empty());
}

#[test]
fn exactly_one_refuses_zero_and_two_forms() {
    let one = format!(
        r#"<form method="post" action="{OK}"><input name="SAMLResponse" value="A"></form>"#
    );
    assert!(exactly_one(find_saml_forms(&one, &base(), &AcsRule::d2l())).is_ok());
    assert_eq!(exactly_one(vec![]).unwrap_err(), AuthFailure::AcsMismatch);
    let two = format!("{one}{one}");
    assert_eq!(
        exactly_one(find_saml_forms(&two, &base(), &AcsRule::d2l())).unwrap_err(),
        AuthFailure::UnexpectedPage
    );
}

#[test]
fn get_attribute_handles_quote_styles_case_and_missing() {
    assert_eq!(
        get_attribute(r#"<input NAME="a" value='b' form=c>"#, "name"),
        "a"
    );
    assert_eq!(
        get_attribute(r#"<input NAME="a" value='b' form=c>"#, "value"),
        "b"
    );
    assert_eq!(
        get_attribute(r#"<input NAME="a" value='b' form=c>"#, "form"),
        "c"
    );
    assert_eq!(get_attribute(r#"<input name="a">"#, "action"), "");
    assert_eq!(
        get_attribute(r#"<input value="x&#58;y&lt;">"#, "value"),
        "x:y<"
    );
}

// ---------- redirects ----------

const HOSTS: [&str; 2] = ["login.microsoftonline.com", "d2l.udst.edu.qa"];

#[test]
fn only_301_302_303_are_followed_and_only_to_allowed_https_hosts() {
    let b = Url::parse("https://login.microsoftonline.com/x/y").unwrap();
    for ok in [301u16, 302, 303] {
        match follow_redirect(ok, Some("/next"), &b, &HOSTS, 0).unwrap() {
            RedirectDecision::Follow(u) => {
                assert_eq!(u.as_str(), "https://login.microsoftonline.com/next")
            }
            other => panic!("{other:?}"),
        }
    }
    // 307/308 would replay the password POST
    for refused in [300u16, 304, 305, 307, 308] {
        assert_eq!(
            follow_redirect(refused, Some("/next"), &b, &HOSTS, 0).unwrap_err(),
            AuthFailure::RedirectRefused,
            "{refused}"
        );
    }
    // not a redirect: stop
    for done in [200u16, 204, 400, 401, 500] {
        assert_eq!(
            follow_redirect(done, None, &b, &HOSTS, 0).unwrap(),
            RedirectDecision::Done
        );
    }
    assert_eq!(
        follow_redirect(302, None, &b, &HOSTS, 0).unwrap_err(),
        AuthFailure::UnexpectedPage
    );
}

#[test]
fn redirects_to_unapproved_targets_are_refused() {
    let b = Url::parse("https://login.microsoftonline.com/x").unwrap();
    for loc in [
        "http://login.microsoftonline.com/x",
        "https://evil.example/x",
        "https://login.microsoftonline.com.evil.example/x",
        "https://user@login.microsoftonline.com/x",
        "https://login.microsoftonline.com:8443/x",
        "//evil.example/x",
        "javascript:alert(1)",
    ] {
        let r = follow_redirect(302, Some(loc), &b, &HOSTS, 0);
        assert!(
            matches!(
                r,
                Err(AuthFailure::HostNotAllowed) | Err(AuthFailure::UnexpectedPage)
            ),
            "{loc}: {r:?}"
        );
    }
}

#[test]
fn redirect_limit_is_enforced() {
    let b = Url::parse("https://login.microsoftonline.com/x").unwrap();
    assert!(follow_redirect(302, Some("/n"), &b, &HOSTS, MAX_REDIRECTS - 1).is_ok());
    assert_eq!(
        follow_redirect(302, Some("/n"), &b, &HOSTS, MAX_REDIRECTS).unwrap_err(),
        AuthFailure::RedirectLimit
    );
}

#[test]
fn allowed_uri_is_case_insensitive_on_host_only() {
    assert!(is_allowed_uri(
        &Url::parse("https://LOGIN.MicrosoftOnline.com/p").unwrap(),
        &HOSTS
    ));
    assert!(!is_allowed_uri(
        &Url::parse("https://login.microsoftonline.com:444/p").unwrap(),
        &HOSTS
    ));
}

// ---------- $Config ----------

fn page_with(config: &str) -> String {
    format!("<html><script>//x\nvar a=1; $Config={config};\nvar b=2;</script></html>")
}

#[test]
fn config_is_extracted_and_never_printed() {
    let page = page_with(
        r#"{"sFT":"FLOW-SECRET","sCtx":"CTX-SECRET","urlGetCredentialType":"https://login.microsoftonline.com/gct","urlPost":"https://login.microsoftonline.com/post","other":[1,2]}"#,
    );
    let c = extract_signin_config(&page).unwrap();
    assert_eq!(c.flow_token(), "FLOW-SECRET");
    assert_eq!(c.context(), "CTX-SECRET");
    assert_eq!(c.url_post, "https://login.microsoftonline.com/post");
    let shown = format!("{c:?}");
    assert!(
        !shown.contains("SECRET") && !shown.contains("microsoftonline"),
        "{shown}"
    );
}

#[test]
fn config_failures_are_typed() {
    assert_eq!(
        extract_signin_config("<html>no config</html>").err(),
        Some(AuthFailure::UnexpectedPage)
    );
    assert_eq!(
        extract_signin_config(&page_with("{not json}")).err(),
        Some(AuthFailure::UnexpectedPage)
    );
    for missing in ["sFT", "sCtx", "urlGetCredentialType", "urlPost"] {
        let mut v: serde_json::Value =
            serde_json::json!({"sFT":"a","sCtx":"b","urlGetCredentialType":"c","urlPost":"d"});
        v.as_object_mut().unwrap().remove(missing);
        assert_eq!(
            extract_signin_config(&page_with(&v.to_string())).err(),
            Some(AuthFailure::ConfigIncomplete),
            "{missing}"
        );
    }
    assert_eq!(
        extract_signin_config(&page_with(
            r#"{"sFT":"","sCtx":"b","urlGetCredentialType":"c","urlPost":"d"}"#
        ))
        .err(),
        Some(AuthFailure::ConfigIncomplete),
        "empty value counts as missing"
    );
}

// ---------- challenge classification (every kind stops the flow) ----------

#[test]
fn challenges_are_classified_by_code_and_by_visible_text() {
    use Challenge::*;
    let cases = [
        (
            "<html>error AADSTS50076 occurred</html>",
            MfaOrIdentityVerification,
        ),
        ("<html>AADSTS50074</html>", MfaOrIdentityVerification),
        (
            "<html><div>Enter the verification code from your Authenticator</div></html>",
            MfaOrIdentityVerification,
        ),
        ("<html>AADSTS53003</html>", PolicyBlocked),
        (
            "<html><p>Access blocked by policy</p></html>",
            PolicyBlocked,
        ),
        ("<html>AADSTS50097</html>", DeviceRequirement),
        (
            "<html><p>Please register your device</p></html>",
            DeviceRequirement,
        ),
        ("<html>AADSTS65001</html>", ConsentOrAdminApproval),
        (
            "<html><p>Admin approval required</p></html>",
            ConsentOrAdminApproval,
        ),
        (
            "<html><span>Your account or password is incorrect.</span></html>",
            CredentialRejected,
        ),
        ("<html><p>Welcome back</p></html>", None),
    ];
    for (page, want) in cases {
        assert_eq!(classify_challenge(page), want, "{page}");
    }
}

#[test]
fn challenge_words_inside_script_or_style_are_ignored_but_codes_are_not() {
    assert_eq!(
        classify_challenge("<script>var s='multi factor authenticator';</script><p>hello</p>"),
        Challenge::None
    );
    assert_eq!(
        classify_challenge("<style>.x{content:'verification code'}</style><p>hello</p>"),
        Challenge::None
    );
    assert_eq!(
        classify_challenge("<script>var e='AADSTS50076';</script>"),
        Challenge::MfaOrIdentityVerification,
        "codes are read from the raw page"
    );
    assert_eq!(
        classify_challenge("<p>Multi&#45;factor</p>"),
        Challenge::MfaOrIdentityVerification,
        "entities decoded"
    );
}

#[test]
fn mfa_takes_priority_over_a_credential_rejected_message() {
    assert_eq!(
        classify_challenge("<p>sign-in failed</p> AADSTS50076"),
        Challenge::MfaOrIdentityVerification
    );
}

// ---------- credentials ----------

#[test]
fn credentials_follow_the_powershell_rules() {
    let c = Credentials::new("12345678", "pw").unwrap();
    assert_eq!(
        c.username(),
        "12345678@udst.edu.qa", // gitleaks:allow (invented test username)
        "bare 8-digit number gets the domain"
    );
    assert!(Credentials::new("a.b@udst.edu.qa", "pw").is_ok()); // gitleaks:allow (invented test username)
    for bad in [
        "",
        "a@other.edu",
        "@udst.edu.qa",
        "a b@udst.edu.qa", // gitleaks:allow (invented test username)
        "a@b@udst.edu.qa", // gitleaks:allow (invented test username)
        "1234567",
        "123456789",
    ] {
        assert_eq!(
            Credentials::new(bad, "pw").err(),
            Some(CredentialsError::Invalid),
            "{bad:?}"
        );
    }
    assert_eq!(
        Credentials::new("a@udst.edu.qa", "").err(), // gitleaks:allow (invented test username)
        Some(CredentialsError::Invalid)
    );
    assert_eq!(
        Credentials::new("a@udst.edu.qa", &"x".repeat(8193)).err(), // gitleaks:allow (invented test username)
        Some(CredentialsError::TooLarge)
    );
}

#[test]
fn stdin_json_must_be_exactly_username_and_password() {
    assert!(
        Credentials::from_stdin_json(br#"{"username":"a@udst.edu.qa","password":"p"}"#).is_ok() // gitleaks:allow (invented test username)
    );
    for bad in [
        br#"{"username":"a@udst.edu.qa"}"#.as_slice(), // gitleaks:allow (invented test username)
        br#"{"username":"a@udst.edu.qa","password":"p","extra":1}"#.as_slice(), // gitleaks:allow (invented test username)
        br#"["a","b"]"#.as_slice(),
        br#"{"username":1,"password":"p"}"#.as_slice(),
        b"not json".as_slice(),
    ] {
        assert_eq!(
            Credentials::from_stdin_json(bad).err(),
            Some(CredentialsError::Invalid)
        );
    }
    assert_eq!(
        Credentials::from_stdin_json(&vec![b' '; 8193]).err(),
        Some(CredentialsError::TooLarge)
    );
}

#[test]
fn secrets_never_appear_in_debug_or_error_text() {
    const PW: &str = "PLANTED-PASSWORD-VALUE";
    let c = Credentials::new("a@udst.edu.qa", PW).unwrap(); // gitleaks:allow (invented test username)
    let shown = format!("{c:?}");
    assert!(!shown.contains(PW) && !shown.contains("udst"), "{shown}");
    assert_eq!(c.expose_password(), PW);
    let err = Credentials::new("not-a-valid-user", PW).err().unwrap();
    assert!(!format!("{err} {err:?}").contains(PW));
    for f in [
        AuthFailure::CredentialsRejected,
        AuthFailure::AcsMismatch,
        AuthFailure::ChallengeRequired(Challenge::PolicyBlocked),
    ] {
        let t = format!("{f} {f:?} {}", f.to_contract().message);
        assert!(!t.contains(PW));
    }
}

// ---------- failures and the contract ----------

#[test]
fn failures_map_to_contract_codes_and_never_retry_after_rejection_or_challenge() {
    use AuthFailure::*;
    let all = [
        ChallengeRequired(Challenge::MfaOrIdentityVerification),
        CredentialsRejected,
        UnexpectedPage,
        HostNotAllowed,
        RedirectRefused,
        RedirectLimit,
        AcsMismatch,
        ConfigIncomplete,
        Network,
        Timeout,
        IdentityMismatch,
        InvalidInput,
    ];
    let mut codes = std::collections::HashSet::new();
    for f in all {
        assert!(
            codes.insert(f.safe_code()),
            "duplicate safe_code {}",
            f.safe_code()
        );
        let c = f.to_contract();
        assert!(c.is_consistent());
        assert_eq!(c.stage.as_deref(), Some("sign_in"));
        assert_eq!(c.source_code.as_deref(), Some(f.safe_code()));
        assert_eq!(
            f.may_retry_automatically(),
            matches!(f, Network | Timeout),
            "{f:?}"
        );
    }
    assert_eq!(
        CredentialsRejected.contract_code(),
        ErrorCode::InvalidCredentials
    );
    assert_eq!(
        ChallengeRequired(Challenge::PolicyBlocked).contract_code(),
        ErrorCode::ChallengeRequired
    );
    assert!(
        !CredentialsRejected.may_retry_automatically()
            && !ChallengeRequired(Challenge::None).may_retry_automatically()
    );
}

#[test]
fn the_domain_form_is_accepted_in_any_case_and_normalised() {
    for typed in [
        "91234567",
        " 91234567 ",
        "91234567@udst.edu.qa",
        "91234567@UDST.EDU.QA",
        "  91234567@Udst.Edu.Qa  ",
    ] {
        assert_eq!(
            Credentials::new(typed, "pw").unwrap().username(),
            "91234567@udst.edu.qa",
            "{typed:?}"
        );
    }
    for bad in [
        "91234567@gmail.com",
        "91234567@udst.edu.qa.evil.example",
        "91234567@@udst.edu.qa",
    ] {
        assert_eq!(
            Credentials::new(bad, "pw").err(),
            Some(CredentialsError::Invalid),
            "{bad:?}"
        );
    }
}

#[test]
fn microsoft_error_codes_in_the_script_config_are_recognised_without_visible_text() {
    let page = |code: &str| {
        format!(
            r#"<html><script>$Config={{"sErrorCode":"{code}","strServiceExceptionMessage":"AADSTS{code}: synthetic"}};</script><body><div id="app"></div></body></html>"#
        )
    };
    assert_eq!(
        classify_challenge(&page("50126")),
        Challenge::CredentialRejected
    );
    assert_eq!(
        classify_challenge(&page("50034")),
        Challenge::CredentialRejected
    );
    for locked in ["50053", "50055", "50057"] {
        assert_eq!(
            classify_challenge(&page(locked)),
            Challenge::PolicyBlocked,
            "{locked}"
        );
    }
    assert_eq!(
        classify_challenge(&page("50076")),
        Challenge::MfaOrIdentityVerification
    );
}

#[test]
fn a_bare_serrorcode_without_any_aadsts_text_is_enough() {
    let page = |code: &str| {
        format!(
            r#"<html><script>$Config={{"sErrorCode":"{code}"}};</script><body><div id="app"></div></body></html>"#
        )
    };
    assert_eq!(
        classify_challenge(&page("50126")),
        Challenge::CredentialRejected
    );
    assert_eq!(classify_challenge(&page("50053")), Challenge::PolicyBlocked);
    // 50058 is the normal "no session yet" state of the first sign-in page: not an error.
    assert_eq!(classify_challenge(&page("50058")), Challenge::None);
    assert_eq!(classify_challenge(&page("0")), Challenge::None);
}
