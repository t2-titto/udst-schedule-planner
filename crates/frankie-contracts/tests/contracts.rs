use frankie_contracts::*;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

fn fx(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(rel)
}
fn read_json(rel: &str) -> Value {
    serde_json::from_str(&fs::read_to_string(fx(rel)).unwrap_or_else(|e| panic!("{rel}: {e}")))
        .unwrap()
}

// ---------- error vocabulary ----------

#[test]
fn error_codes_round_trip_and_match_as_str() {
    assert_eq!(
        ErrorCode::ALL.len(),
        29,
        "19 schema codes + 10 added in F-001"
    );
    let mut seen = HashSet::new();
    for c in ErrorCode::ALL {
        let text = serde_json::to_value(c).unwrap();
        assert_eq!(text, json!(c.as_str()), "{c:?}");
        let back: ErrorCode = serde_json::from_value(text).unwrap();
        assert_eq!(back, *c);
        assert!(
            seen.insert(c.as_str()),
            "duplicate code string {}",
            c.as_str()
        );
        assert!(
            c.as_str()
                .chars()
                .all(|ch| ch.is_ascii_uppercase() || ch == '_'),
            "{}",
            c.as_str()
        );
    }
}

#[test]
fn contract_contains_every_code_of_the_existing_tool_error_schema() {
    let schema =
        read_json("extensions/edu.udst.student/modules/libcal/schemas/tool_error.schema.json");
    let codes: Vec<&str> = schema["properties"]["code"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(codes.len(), 19);
    let ours: HashSet<&str> = ErrorCode::ALL.iter().map(|c| c.as_str()).collect();
    for c in codes {
        assert!(ours.contains(c), "schema code {c} missing from ErrorCode");
    }
}

#[test]
fn retryable_is_exactly_the_four_codes_the_old_rule_used() {
    let expected: HashSet<&str> = [
        "NETWORK_ERROR",
        "TIMEOUT",
        "SESSION_EXPIRED",
        "RATE_LIMITED",
    ]
    .into();
    for c in ErrorCode::ALL {
        assert_eq!(c.retryable(), expected.contains(c.as_str()), "{c:?}");
    }
}

#[test]
fn status_follows_login_shaped_codes_only() {
    for c in ErrorCode::ALL {
        let want = match c {
            ErrorCode::NeedsLogin | ErrorCode::SessionExpired | ErrorCode::ChallengeRequired => {
                ToolStatus::NeedsLogin
            }
            ErrorCode::NotConfigured => ToolStatus::NotConfigured,
            _ => ToolStatus::Fail,
        };
        assert_eq!(c.status(), want, "{c:?}");
    }
}

#[test]
fn legacy_table_is_unique_and_exact() {
    let mut keys = HashSet::new();
    for (k, c) in LEGACY_CODES {
        assert!(keys.insert(*k), "duplicate legacy code {k}");
        assert_eq!(from_legacy_code(k), Some(*c));
    }
    assert_eq!(from_legacy_code("not_a_code"), None);
    assert_eq!(
        from_legacy_code("invalid_request"),
        None,
        "matching is exact and case-sensitive"
    );
    // Contract codes that no current crate emits. They exist for producers not in the repository yet
    // (the PowerShell/Node logins and PeopleSoft); if one becomes reachable, update this list on purpose.
    let reachable: HashSet<ErrorCode> = LEGACY_CODES.iter().map(|(_, c)| *c).collect();
    let mut unreachable: Vec<&str> = ErrorCode::ALL
        .iter()
        .filter(|c| !reachable.contains(c))
        .map(|c| c.as_str())
        .collect();
    unreachable.sort();
    assert_eq!(
        unreachable,
        [
            "ACCOUNT_LOCKED",
            "COUNT_MISMATCH",
            "ROUTE_NOT_FOUND",
            "SESSION_EXPIRED"
        ]
    );
}

#[test]
fn from_legacy_keeps_the_original_code_and_derives_retryable() {
    let e = ToolError::from_legacy("outlook_throttled", "slow down").unwrap();
    assert_eq!(e.code, ErrorCode::RateLimited);
    assert!(e.retryable && e.is_consistent());
    assert_eq!(e.source_code.as_deref(), Some("outlook_throttled"));
    assert!(ToolError::from_legacy("brand_new_code", "x").is_none());
}

#[test]
fn tool_error_rejects_unknown_fields_and_flags_inconsistent_retryable() {
    let bad = json!({"code":"TIMEOUT","message":"x","retryable":true,"extra":1});
    assert!(serde_json::from_value::<ToolError>(bad).is_err());
    let lie: ToolError =
        serde_json::from_value(json!({"code":"INVALID_INPUT","message":"x","retryable":true}))
            .unwrap();
    assert!(!lie.is_consistent());
    assert!(serde_json::from_value::<ToolError>(
        json!({"code":"NOPE","message":"x","retryable":false})
    )
    .is_err());
}

// ---------- result envelope ----------

#[test]
fn golden_results_round_trip_to_the_same_canonical_json() {
    for name in [
        "contracts/tool_result.pass.json",
        "contracts/tool_result.fail.json",
    ] {
        let v = read_json(name);
        let r: ToolResult = serde_json::from_value(v.clone()).unwrap();
        r.validate().unwrap_or_else(|e| panic!("{name}: {e}"));
        let back = serde_json::to_value(&r).unwrap();
        assert_eq!(canonical_json(&back), canonical_json(&v), "{name}");
    }
}

#[test]
fn camel_case_spellings_emitted_today_are_accepted() {
    let r: ToolResult =
        serde_json::from_value(read_json("contracts/tool_result.camelcase.json")).unwrap();
    assert_eq!(r.fetched_at.as_deref(), Some("2026-11-11T12:00:00Z"));
    assert_eq!(r.record_count, Some(1));
    // and it is written back in snake_case
    let out = serde_json::to_value(&r).unwrap();
    assert!(out.get("fetched_at").is_some() && out.get("fetchedAt").is_none());
}

#[test]
fn result_constructors_and_validation_rules() {
    let ok = ToolResult::pass("d2l", Origin::Live, json!([{"a":1},{"a":2}]))
        .with_fetched_at("2026-11-11T12:00:00Z");
    assert_eq!(ok.record_count, Some(2));
    ok.validate().unwrap();

    let fail = ToolResult::fail(
        "navigate",
        Origin::Live,
        ToolError::new(ErrorCode::NeedsLogin, "sign in"),
    );
    assert_eq!(fail.status, ToolStatus::NeedsLogin);
    fail.validate().unwrap();
    assert_eq!(
        ToolResult::fail("x", Origin::Live, ToolError::new(ErrorCode::Timeout, "t")).status,
        ToolStatus::Fail
    );
    assert_eq!(
        ToolResult::unsupported(
            "x",
            Origin::Live,
            ToolError::new(ErrorCode::NotImplemented, "n")
        )
        .status,
        ToolStatus::Unsupported
    );

    // negative cases
    let mut bad = ok.clone();
    bad.error = Some(ToolError::new(ErrorCode::Timeout, "t"));
    assert!(bad.validate().is_err(), "PASS must not carry an error");
    let mut bad = ok.clone();
    bad.data = None;
    assert!(bad.validate().is_err(), "PASS must carry data");
    let mut bad = fail.clone();
    bad.data = Some(json!(1));
    assert!(bad.validate().is_err(), "failure must not carry data");
    let mut bad = fail.clone();
    bad.status = ToolStatus::Fail;
    assert!(
        bad.validate().is_err(),
        "status must follow the login-shaped code"
    );
    let mut bad = ok.clone();
    bad.contract_version = "9.9.9".into();
    assert!(bad.validate().is_err());
    let mut bad = ok.clone();
    bad.source = "  ".into();
    assert!(bad.validate().is_err());
    let mut bad = fail.clone();
    bad.error.as_mut().unwrap().retryable = true;
    assert!(
        bad.validate().is_err(),
        "retryable must agree with the code"
    );
    assert!(serde_json::from_value::<ToolResult>(json!({"status":"MAYBE"})).is_err());
}

// ---------- descriptors ----------

#[test]
fn all_57_catalog_operations_become_descriptors() {
    let ds = descriptors_from_catalog(&read_json("catalog.manifests.json")).unwrap();
    assert_eq!(ds.len(), 57);
    let ids: HashSet<&str> = ds.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(ids.len(), 57, "ids are unique");
    assert!(ds
        .iter()
        .all(|d| !d.capability.is_empty() && !d.description.is_empty()));
    assert!(ds
        .iter()
        .all(|d| matches!(d.input_schema, SchemaRef::Ref(_))));
    assert!(
        ds.iter().all(|d| !d.mutates),
        "all 57 catalogued operations are read-only today"
    );
    // round trip
    for d in &ds {
        let back: ToolDescriptor =
            serde_json::from_value(serde_json::to_value(d).unwrap()).unwrap();
        assert_eq!(&back, d);
    }
}

#[test]
fn descriptor_needs_only_the_five_required_fields_and_accepts_mcp_spelling() {
    let d: ToolDescriptor = serde_json::from_value(json!({
        "id":"navigate.x","capability":"x","description":"d","inputSchema":{"type":"object"},"mutates":false
    }))
    .unwrap();
    assert!(d.when_to_use.is_none() && d.examples.is_empty());
    assert!(matches!(d.input_schema, SchemaRef::Inline(_)));
    for missing in ["id", "capability", "description", "inputSchema", "mutates"] {
        let mut v = json!({"id":"a","capability":"b","description":"c","inputSchema":"s.json","mutates":false});
        v.as_object_mut().unwrap().remove(missing);
        assert!(
            serde_json::from_value::<ToolDescriptor>(v).is_err(),
            "{missing} is required"
        );
    }
}

#[test]
fn catalog_with_a_broken_operation_is_an_error_not_a_silent_skip() {
    let mut cat = read_json("catalog.manifests.json");
    cat[0]["module"]["operations"][0]
        .as_object_mut()
        .unwrap()
        .remove("capability");
    assert!(matches!(
        descriptors_from_catalog(&cat),
        Err(ContractError::InvalidDescriptor(_))
    ));
    assert!(descriptors_from_catalog(&json!({})).is_err());
}

// ---------- modes ----------

#[test]
fn policy_invariants_hold_for_every_mode_and_class() {
    let default = PolicyOptions::default();
    for m in Mode::ALL {
        for c in DataClass::ALL {
            match policy(*m, *c, default) {
                Ok(p) => {
                    if c.is_secret() {
                        assert!(!p.may_sync, "{m:?}/{c:?}: secrets never sync");
                        if *m != Mode::Server {
                            assert_eq!(
                                p.location,
                                Location::Device,
                                "{m:?}/{c:?}: secrets stay on the device outside Server mode"
                            );
                        }
                    }
                    if *m == Mode::Device {
                        assert_eq!(
                            p.location,
                            Location::Device,
                            "{c:?}: Device mode has no server"
                        );
                    }
                }
                Err(e) => {
                    assert_eq!(
                        (*m, c.is_secret()),
                        (Mode::Server, true),
                        "only Server-mode secrets may be refused"
                    );
                    assert_eq!(e, PolicyError::ServerCustodyDisabled);
                }
            }
        }
    }
}

#[test]
fn server_custody_is_off_by_default_and_needs_an_explicit_opt_in() {
    assert_eq!(
        policy(
            Mode::Server,
            DataClass::Credential,
            PolicyOptions::default()
        ),
        Err(PolicyError::ServerCustodyDisabled)
    );
    assert_eq!(
        policy(
            Mode::Server,
            DataClass::SessionArtifact,
            PolicyOptions::default()
        ),
        Err(PolicyError::ServerCustodyDisabled)
    );
    let on = PolicyOptions {
        server_credential_custody: true,
    };
    let p = policy(Mode::Server, DataClass::Credential, on).unwrap();
    assert_eq!(
        (p.location, p.may_sync, p.requires_consent),
        (Location::Server, false, true)
    );
    // the opt-in changes nothing for Device or Hybrid
    for m in [Mode::Device, Mode::Hybrid] {
        assert_eq!(
            policy(m, DataClass::Credential, on).unwrap().location,
            Location::Device
        );
    }
}

#[test]
fn hybrid_puts_files_on_the_server_and_device_mode_never_does() {
    let d = PolicyOptions::default();
    assert_eq!(
        policy(Mode::Hybrid, DataClass::File, d).unwrap().location,
        Location::Server
    );
    assert_eq!(
        policy(Mode::Device, DataClass::File, d).unwrap().location,
        Location::Device
    );
    assert!(
        policy(Mode::Device, DataClass::File, d).unwrap().may_sync,
        "device-to-device sync of files is allowed"
    );
    assert!(
        policy(Mode::Hybrid, DataClass::File, d)
            .unwrap()
            .requires_consent
    );
}

#[test]
fn modes_serialise_as_snake_case() {
    assert_eq!(serde_json::to_value(Mode::Hybrid).unwrap(), json!("hybrid"));
    assert_eq!(
        serde_json::to_value(DataClass::SessionArtifact).unwrap(),
        json!("session_artifact")
    );
}

// ---------- presentation ----------

#[test]
fn courses_screen_round_trips() {
    let v = read_json("contracts/courses_screen.json");
    let s: CoursesScreen = serde_json::from_value(v.clone()).unwrap();
    assert_eq!(s.courses.len(), 1);
    assert_eq!(s.sources[1].state, SourceState::NeedsLogin);
    assert_eq!(s.sources[1].origin, Origin::Cached);
    assert_eq!(
        canonical_json(&serde_json::to_value(&s).unwrap()),
        canonical_json(&v)
    );
    assert!(serde_json::from_value::<CoursesScreen>(
        json!({"courses":[],"sources":[],"surprise":1})
    )
    .is_err());
}

// ---------- canonical json ----------

#[test]
fn canonical_json_ignores_object_order_and_keeps_array_order() {
    let a: Value = serde_json::from_str(r#"{"b":1,"a":{"y":[3,1,2],"x":null}}"#).unwrap();
    let b: Value = serde_json::from_str(r#"{"a":{"x":null,"y":[3,1,2]},"b":1}"#).unwrap();
    assert_eq!(canonical_json(&a), canonical_json(&b));
    assert_eq!(canonical_json(&a), r#"{"a":{"x":null,"y":[3,1,2]},"b":1}"#);
    let c: Value = serde_json::from_str(r#"{"a":{"x":null,"y":[1,2,3]},"b":1}"#).unwrap();
    assert_ne!(
        canonical_json(&a),
        canonical_json(&c),
        "array order is significant"
    );
}
