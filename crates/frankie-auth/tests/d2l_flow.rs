mod mock;
use frankie_auth::*;
use mock::*;

fn creds() -> Credentials {
    Credentials::new(USER, PASSWORD).unwrap()
}
fn run(s: Scenario) -> (Result<D2lArtifact, AuthFailure>, MockIdp) {
    let mut idp = MockIdp::new(s);
    let r = authenticate_d2l(&mut idp, &creds(), &D2lConfig::udst());
    (r, idp)
}
fn fails(s: Scenario) -> (AuthFailure, MockIdp) {
    let (r, idp) = run(s);
    (r.expect_err("flow should have stopped"), idp)
}

// ---------- the happy path ----------

#[test]
fn full_flow_succeeds_with_one_password_post_and_a_verified_bearer() {
    let (r, idp) = run(Scenario::default());
    let a = r.expect("flow should succeed");
    assert_eq!(a.subject(), SUBJECT);
    assert_eq!(a.bearer(), BEARER);
    assert_eq!(idp.password_posts, 1, "exactly one password POST");
    assert_eq!(idp.acs_posts, 1);
}

#[test]
fn request_sequence_is_the_one_in_the_powershell() {
    let (_, idp) = run(Scenario::default());
    let seq: Vec<String> = idp
        .log
        .iter()
        .map(|s| format!("{:?} {}", s.method, s.url))
        .collect();
    assert_eq!(
        seq,
        [
            "Get d2l.udst.edu.qa/d2l/lp/auth/saml/initiate-login",
            "Get login.microsoftonline.com/TENANT/saml2",
            "Post login.microsoftonline.com/gct",
            "Post login.microsoftonline.com/TENANT/login",
            "Get login.microsoftonline.com/next",
            "Post d2l.udst.edu.qa/d2l/lp/auth/login/samlLogin.d2l",
            "Get d2l.udst.edu.qa/d2l/api/lp/1.49/users/whoami",
            "Get d2l.udst.edu.qa/d2l/lp/auth/xsrf-tokens",
            "Post d2l.udst.edu.qa/d2l/lp/auth/oauth2/token",
            "Get d2l.udst.edu.qa/d2l/api/lp/1.49/users/whoami",
        ]
    );
}

#[test]
fn bearer_is_verified_without_cookies_and_with_the_authorization_header() {
    let (_, idp) = run(Scenario::default());
    let last = idp.log.last().unwrap();
    assert_eq!(last.cookies, CookieMode::None);
    assert!(last.has_authorization);
    // and every other request used the jar and carried no Authorization header
    for s in &idp.log[..idp.log.len() - 1] {
        assert_eq!(s.cookies, CookieMode::Jar, "{}", s.url);
        assert!(!s.has_authorization, "{}", s.url);
    }
}

#[test]
fn kmsi_step_is_answered_once_and_the_flow_then_succeeds() {
    let (r, idp) = run(Scenario {
        kmsi: true,
        ..Default::default()
    });
    assert!(r.is_ok());
    assert_eq!(
        idp.password_posts, 1,
        "KMSI must not cause a second password POST"
    );
    assert!(idp
        .log
        .iter()
        .any(|s| s.url == "login.microsoftonline.com/kmsi"));
}

// ---------- failures that must stop the flow ----------

#[test]
fn wrong_password_page_stops_after_exactly_one_attempt() {
    let (f, idp) = fails(Scenario {
        wrong_password_page: true,
        ..Default::default()
    });
    assert_eq!(f, AuthFailure::CredentialsRejected);
    assert_eq!(idp.password_posts, 1, "never a second attempt");
    assert_eq!(idp.acs_posts, 0);
    assert!(!f.may_retry_automatically());
}

#[test]
fn every_challenge_kind_stops_the_flow_before_any_assertion_is_posted() {
    for (text, kind) in [
        (
            "Enter the verification code from your authenticator",
            Challenge::MfaOrIdentityVerification,
        ),
        ("Access blocked by policy", Challenge::PolicyBlocked),
        ("Please register your device", Challenge::DeviceRequirement),
        ("Admin approval required", Challenge::ConsentOrAdminApproval),
    ] {
        let (f, idp) = fails(Scenario {
            challenge_text: Some(text),
            ..Default::default()
        });
        assert_eq!(f, AuthFailure::ChallengeRequired(kind), "{text}");
        assert_eq!(idp.password_posts, 1);
        assert_eq!(idp.acs_posts, 0);
        assert!(!f.may_retry_automatically());
    }
}

#[test]
fn a_challenge_at_the_kmsi_state_stops_the_flow() {
    let (f, idp) = fails(Scenario {
        kmsi: true,
        kmsi_challenge_text: Some("More information required"),
        ..Default::default()
    });
    assert_eq!(
        f,
        AuthFailure::ChallengeRequired(Challenge::MfaOrIdentityVerification)
    );
    assert_eq!(idp.acs_posts, 0);
}

#[test]
fn a_second_kmsi_prompt_is_not_looped() {
    let (f, idp) = fails(Scenario {
        kmsi: true,
        kmsi_again: true,
        ..Default::default()
    });
    assert_eq!(f, AuthFailure::UnexpectedPage);
    assert_eq!(
        idp.log.iter().filter(|s| s.url.ends_with("/kmsi")).count(),
        1
    );
}

#[test]
fn a_307_redirect_after_the_password_is_refused_not_replayed() {
    for status in [307u16, 308] {
        let (f, idp) = fails(Scenario {
            password_redirect_status: status,
            ..Default::default()
        });
        assert_eq!(f, AuthFailure::RedirectRefused, "{status}");
        assert_eq!(idp.password_posts, 1, "the password POST was not replayed");
    }
}

#[test]
fn a_redirect_to_an_unapproved_host_is_refused() {
    let (f, idp) = fails(Scenario {
        redirect_host: Some("evil.example"),
        ..Default::default()
    });
    assert_eq!(f, AuthFailure::HostNotAllowed);
    assert!(
        !idp.log.iter().any(|s| s.url.starts_with("evil.example")),
        "nothing was requested from the unapproved host"
    );
}

#[test]
fn a_redirect_loop_hits_the_hop_limit() {
    let (f, idp) = fails(Scenario {
        redirect_loop: true,
        ..Default::default()
    });
    assert_eq!(f, AuthFailure::RedirectLimit);
    // initiate, saml2, gct, password POST, then at most 10 followed hops
    assert!(idp.log.len() <= 4 + 10, "bounded: {}", idp.log.len());
    assert_eq!(idp.password_posts, 1);
}

#[test]
fn saml_form_for_the_wrong_acs_is_never_posted() {
    for acs in [
        "https://example.invalid/d2l/lp/auth/login/samlLogin.d2l",
        "http://d2l.udst.edu.qa/d2l/lp/auth/login/samlLogin.d2l",
        "https://d2l.udst.edu.qa/d2l/lp/auth/login/samlLogin.d2l?x=1",
    ] {
        let (f, idp) = fails(Scenario {
            acs_url: acs,
            ..Default::default()
        });
        assert_eq!(f, AuthFailure::AcsMismatch, "{acs}");
        assert_eq!(
            idp.acs_posts, 0,
            "the assertion must not be posted to {acs}"
        );
    }
}

#[test]
fn two_saml_forms_are_ambiguous_and_refused() {
    let (f, idp) = fails(Scenario {
        two_forms: true,
        ..Default::default()
    });
    assert_eq!(f, AuthFailure::UnexpectedPage);
    assert_eq!(idp.acs_posts, 0);
}

#[test]
fn account_not_recognised_or_no_password_method_never_posts_a_password() {
    let (f, idp) = fails(Scenario {
        if_exists: 1,
        ..Default::default()
    });
    assert_eq!(f, AuthFailure::CredentialsRejected);
    assert_eq!(idp.password_posts, 0);
    let (f, idp) = fails(Scenario {
        has_password: false,
        ..Default::default()
    });
    assert_eq!(f, AuthFailure::UnexpectedPage);
    assert_eq!(idp.password_posts, 0);
}

#[test]
fn bearer_for_a_different_user_is_an_identity_mismatch() {
    let (f, _) = fails(Scenario {
        bearer_subject: "SUBJ-2",
        ..Default::default()
    });
    assert_eq!(f, AuthFailure::IdentityMismatch);
}

#[test]
fn missing_token_non_json_whoami_and_bad_acs_status_are_refused() {
    assert_eq!(
        fails(Scenario {
            no_token: true,
            ..Default::default()
        })
        .0,
        AuthFailure::UnexpectedPage
    );
    assert_eq!(
        fails(Scenario {
            whoami_not_json: true,
            ..Default::default()
        })
        .0,
        AuthFailure::UnexpectedPage
    );
    assert_eq!(
        fails(Scenario {
            acs_status: 500,
            ..Default::default()
        })
        .0,
        AuthFailure::UnexpectedPage
    );
}

// ---------- secrets ----------

#[test]
fn password_and_bearer_never_appear_in_urls_headers_or_failure_output() {
    let (r, idp) = run(Scenario::default());
    assert!(
        !idp.exposed_text.contains(PASSWORD),
        "password leaked into a URL or header"
    );
    // the bearer is only ever sent in the Authorization header of the one verification request
    assert_eq!(idp.exposed_text.matches(BEARER).count(), 1);
    let a = r.unwrap();
    assert!(!format!("{a:?}").contains(BEARER));
    for s in [
        Scenario {
            wrong_password_page: true,
            ..Default::default()
        },
        Scenario {
            password_redirect_status: 307,
            ..Default::default()
        },
    ] {
        let (f, idp) = fails(s);
        for text in [
            format!("{f} {f:?} {}", f.to_contract().message),
            failure_json(f),
            idp.exposed_text.clone(),
        ] {
            assert!(!text.contains(PASSWORD));
        }
    }
}

// ---------- vault compatibility ----------

#[test]
fn success_json_is_accepted_by_the_vault_parser() {
    let (r, _) = run(Scenario::default());
    let a = r.unwrap();
    let line = a.to_vault_json();
    let ok = frankie_vault::auth_contract::parse_private_worker_reply(
        line.as_bytes(),
        true,
        Some(SUBJECT),
    )
    .expect("vault accepts our output");
    assert_eq!(ok.subject(), SUBJECT);
    assert_eq!(ok.bearer_for_private_connector(), BEARER);
    // a different expected subject is rejected by the vault
    assert!(frankie_vault::auth_contract::parse_private_worker_reply(
        line.as_bytes(),
        true,
        Some("someone-else")
    )
    .is_err());
}

#[test]
fn failure_json_maps_to_the_vault_failures_and_carries_no_token() {
    use frankie_vault::auth_contract::{parse_private_worker_reply, AuthFailure as V};
    let cases = [
        (AuthFailure::CredentialsRejected, V::AuthenticationRejected),
        (
            AuthFailure::ChallengeRequired(Challenge::PolicyBlocked),
            V::ChallengeRequired,
        ),
        (AuthFailure::Network, V::UpstreamUnavailable),
        (AuthFailure::Timeout, V::UpstreamUnavailable),
        (AuthFailure::IdentityMismatch, V::AuthenticationUnverified),
        (AuthFailure::AcsMismatch, V::AuthenticationUnverified),
    ];
    for (ours, theirs) in cases {
        let line = failure_json(ours);
        assert!(!line.contains("token") || line.contains("\"ok\":false"));
        let got = parse_private_worker_reply(line.as_bytes(), false, None)
            .expect_err("a failure line is an error");
        assert_eq!(got, theirs, "{ours:?}");
    }
}

// ---------- structure ----------

#[test]
fn only_transport_rs_may_touch_the_network_and_nothing_spawns_processes_or_reads_files() {
    let all = [
        ("lib.rs", include_str!("../src/lib.rs")),
        ("artifact.rs", include_str!("../src/artifact.rs")),
        ("challenge.rs", include_str!("../src/challenge.rs")),
        ("config.rs", include_str!("../src/config.rs")),
        ("credentials.rs", include_str!("../src/credentials.rs")),
        ("d2l.rs", include_str!("../src/d2l.rs")),
        ("failure.rs", include_str!("../src/failure.rs")),
        ("html.rs", include_str!("../src/html.rs")),
        ("http.rs", include_str!("../src/http.rs")),
        ("redirect.rs", include_str!("../src/redirect.rs")),
        ("transport.rs", include_str!("../src/transport.rs")),
        (
            "frankie-auth-cli.rs",
            include_str!("../src/bin/frankie-auth-cli.rs"),
        ),
    ];
    for (name, src) in all {
        for needle in [
            "std::process::Command",
            "process::Command",
            "tokio::process",
            "Command::new",
            "std::fs",
            "unsafe ",
            "danger_accept_invalid",
        ] {
            assert!(!src.contains(needle), "{name} contains forbidden {needle}");
        }
        if name != "transport.rs" {
            for needle in ["reqwest::", "std::net", "TcpStream", "TcpListener"] {
                assert!(
                    !src.contains(needle),
                    "{name} touches the network ({needle}); only transport.rs may"
                );
            }
        }
    }
}

#[test]
fn every_request_carries_its_timing_stage_label_in_order() {
    let (_, idp) = run(Scenario::default());
    let stages: Vec<&str> = idp.log.iter().map(|s| s.stage).collect();
    assert_eq!(
        stages,
        [
            "d2l_initiate",
            "ms_signin_page",
            "ms_account_discovery",
            "ms_password",
            "ms_password_redirect",
            "d2l_saml_acs",
            "d2l_whoami",
            "d2l_xsrf",
            "d2l_token_mint",
            "d2l_bearer_verify"
        ]
    );
}

#[test]
fn a_script_rendered_wrong_password_page_is_a_rejection_and_the_password_is_sent_once() {
    let (e, idp) = fails(Scenario {
        wrong_password_script_page: true,
        ..Scenario::default()
    });
    assert_eq!(e, AuthFailure::CredentialsRejected);
    assert_eq!(idp.password_posts, 1);
    assert_eq!(idp.acs_posts, 0);
}
