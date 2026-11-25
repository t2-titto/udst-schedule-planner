//! Navigate360 sign-in on the shared Microsoft session, against the in-process synthetic mock (invented data).
mod mock;
use frankie_auth::*;
use mock::*;

fn creds() -> Credentials {
    Credentials::new(USER, PASSWORD).unwrap()
}

/// D2L first (creates the shared session), then Navigate on the same `Http`.
fn run(s: Scenario) -> (Result<NavigateArtifact, AuthFailure>, MockIdp) {
    let mut idp = MockIdp::new(s);
    let d2l =
        authenticate_d2l(&mut idp, &creds(), &D2lConfig::udst()).expect("D2L must succeed first");
    let r = authenticate_navigate(&mut idp, &creds(), &d2l, &NavigateConfig::udst());
    (r, idp)
}
fn fails(s: Scenario) -> (AuthFailure, MockIdp) {
    let (r, idp) = run(s);
    (r.expect_err("flow should have stopped"), idp)
}
fn stages(idp: &MockIdp) -> Vec<&'static str> {
    idp.log.iter().map(|s| s.stage).collect()
}

#[test]
fn navigate_reuses_the_microsoft_session_and_sends_no_second_password() {
    let (r, idp) = run(Scenario::default());
    let a = r.expect("navigate should succeed");
    assert_eq!(a.student_nk(), NAV_STUDENT_NK);
    assert_eq!(a.campus_bearer(), NAV_BEARER);
    assert_eq!(
        idp.password_posts, 1,
        "the password was sent once, for D2L only"
    );
    assert_eq!(idp.nav_acs_posts, 1);
    let nav: Vec<&str> = stages(&idp)
        .into_iter()
        .filter(|s| s.starts_with("nav_"))
        .collect();
    assert_eq!(
        nav,
        [
            "nav_initiate",
            "nav_eab_sso",
            "nav_ms_session",
            "nav_saml_acs",
            "nav_handoff",
            "nav_account",
            "nav_campus_token",
            "nav_campus_verify"
        ]
    );
}

#[test]
fn the_campus_check_carries_no_cookies_and_only_the_campus_host_sees_the_bearer() {
    let (_, idp) = run(Scenario::default());
    let last = idp.log.last().unwrap();
    assert_eq!(
        last.url,
        "udst.campus.eab.com/api/v2/scheduling/current_user"
    );
    assert_eq!(last.cookies, CookieMode::None);
    assert!(last.has_authorization);
    for s in &idp.log {
        if s.url.starts_with("udst.navigate.eab.com") || s.url.starts_with("bouncer.eab.com") {
            assert!(!s.has_authorization, "{}", s.url);
        }
    }
}

#[test]
fn json_output_has_the_powershell_shape_and_debug_hides_secrets() {
    let (r, _) = run(Scenario::default());
    let a = r.unwrap();
    let v: serde_json::Value = serde_json::from_str(&a.to_navigate_json()).unwrap();
    for k in [
        "ok",
        "provider",
        "credential_type",
        "verified_subject",
        "navigate_cookie_header",
        "campus_bearer",
        "identity_verified",
        "campus_api_verified",
        "session_verified",
        "http_status",
        "elapsed_ms",
    ] {
        assert!(v.get(k).is_some(), "missing {k}");
    }
    assert_eq!(v["provider"], "navigate360");
    assert_eq!(v["credential_type"], "composite");
    assert!(!format!("{a:?}").contains(NAV_BEARER));
}

#[test]
fn without_a_microsoft_session_it_stops_and_never_sends_a_password() {
    let (e, idp) = fails(Scenario {
        nav_no_session: true,
        ..Scenario::default()
    });
    assert_eq!(e, AuthFailure::AcsMismatch);
    assert_eq!(idp.password_posts, 1, "no second password POST");
    assert_eq!(idp.nav_acs_posts, 0);
}

#[test]
fn a_microsoft_challenge_stops_the_flow_before_anything_is_posted() {
    let (e, idp) = fails(Scenario {
        nav_challenge_text: Some("Approve sign in request. Open your Authenticator app."),
        ..Scenario::default()
    });
    assert!(matches!(e, AuthFailure::ChallengeRequired(_)), "{e:?}");
    assert_eq!(idp.nav_acs_posts, 0);
    assert_eq!(*stages(&idp).last().unwrap(), "nav_ms_session");
}

#[test]
fn a_saml_form_for_the_wrong_acs_is_never_posted() {
    let (e, idp) = fails(Scenario {
        nav_acs_url: "https://bouncer.eab.com/sso/saml2/acs?x=1",
        ..Scenario::default()
    });
    assert_eq!(e, AuthFailure::AcsMismatch);
    assert_eq!(idp.nav_acs_posts, 0);
    let (e2, _) = fails(Scenario {
        nav_acs_url: "https://evil.example/sso/saml2/acs",
        ..Scenario::default()
    });
    assert_eq!(e2, AuthFailure::AcsMismatch);
}

#[test]
fn a_session_for_a_different_account_is_rejected_before_a_token_is_requested() {
    let (e, idp) = fails(Scenario {
        nav_email: "test-other-user@udst.edu.qa",
        ..Scenario::default()
    });
    assert_eq!(e, AuthFailure::IdentityMismatch);
    assert!(!stages(&idp).contains(&"nav_campus_token"));
}

#[test]
fn a_bearer_the_campus_api_rejects_fails_the_flow() {
    let (e, _) = fails(Scenario {
        nav_campus_rejects: true,
        ..Scenario::default()
    });
    assert_eq!(e, AuthFailure::UnexpectedPage);
}

#[test]
fn a_bouncer_redirect_to_an_unapproved_host_is_refused() {
    let (e, idp) = fails(Scenario {
        nav_bouncer_redirect: "https://evil.example/TENANT2/saml2",
        ..Scenario::default()
    });
    assert_eq!(e, AuthFailure::HostNotAllowed);
    assert!(!idp.log.iter().any(|s| s.url.starts_with("evil.example")));
}
