use frankie_connector_peoplesoft::gate::*;
use std::time::{Duration, Instant};

fn plan(kind: WriteKind, nums: &[&str]) -> WritePlan {
    WritePlan::new(
        kind,
        "T",
        nums.iter().map(|s| s.to_string()).collect(),
        "s".into(),
        vec![],
        vec![],
    )
}

#[test]
fn phrases_name_action_and_class_numbers() {
    assert_eq!(
        plan(WriteKind::Drop, &["9101"]).required_phrase,
        "CONFIRM DROP 9101"
    );
    assert_eq!(
        plan(WriteKind::SubmitEnrol, &["9001", "9002"]).required_phrase,
        "CONFIRM ENROLL 9001 9002"
    );
    assert_eq!(
        plan(WriteKind::SwapCommit, &["9001", "9003", "9004"]).required_phrase,
        "CONFIRM SWAP 9001 TO 9003 9004"
    );
    assert_eq!(
        plan(WriteKind::StageEnrol, &["9001"]).required_phrase,
        "CONFIRM STAGE 9001"
    );
}

#[test]
fn disarmed_gate_refuses_everything() {
    let mut g: Gate<u8> = Gate::new(Duration::from_secs(60));
    let now = Instant::now();
    assert_eq!(
        g.register(plan(WriteKind::Drop, &["1"]), 0, now).err(),
        Some(GateError::Disarmed)
    );
    assert_eq!(g.redeem("x", "y", now).err(), Some(GateError::Disarmed));
}

#[test]
fn token_is_single_use_and_phrase_exact() {
    let mut g: Gate<u8> = Gate::new(Duration::from_secs(60));
    g.arm();
    let now = Instant::now();
    let p = g
        .register(plan(WriteKind::Drop, &["9101"]), 7, now)
        .unwrap();
    assert_eq!(p.token.len(), 64);
    assert_eq!(
        g.redeem(&p.token, "confirm drop 9101", now).err(),
        Some(GateError::PhraseMismatch)
    );
    assert_eq!(
        g.redeem(&p.token, "CONFIRM DROP 9101 ", now).err(),
        Some(GateError::PhraseMismatch)
    );
    let (proof, pl, payload) = g.redeem(&p.token, "CONFIRM DROP 9101", now).unwrap();
    assert_eq!(payload, 7);
    assert_eq!(pl.kind, WriteKind::Drop);
    assert!(proof.allows("DERIVED_SSR_FL_SSR_DROPPED") && proof.allows("#ICYes"));
    assert!(
        !proof.allows("VC_PE_SS_WRK_VC_SUBMIT_ENROL_PB"),
        "a drop proof must not allow an enrol"
    );
    assert_eq!(
        g.redeem(&p.token, "CONFIRM DROP 9101", now).err(),
        Some(GateError::UnknownToken)
    );
}

#[test]
fn three_wrong_phrases_burn_the_token() {
    let mut g: Gate<u8> = Gate::new(Duration::from_secs(60));
    g.arm();
    let now = Instant::now();
    let p = g.register(plan(WriteKind::Drop, &["1"]), 0, now).unwrap();
    assert_eq!(
        g.redeem(&p.token, "a", now).err(),
        Some(GateError::PhraseMismatch)
    );
    assert_eq!(
        g.redeem(&p.token, "b", now).err(),
        Some(GateError::PhraseMismatch)
    );
    assert_eq!(
        g.redeem(&p.token, "c", now).err(),
        Some(GateError::TooManyAttempts)
    );
    assert_eq!(
        g.redeem(&p.token, "CONFIRM DROP 1", now).err(),
        Some(GateError::UnknownToken)
    );
}

#[test]
fn expired_token_is_refused_and_disarm_clears_pending() {
    let mut g: Gate<u8> = Gate::new(Duration::from_secs(10));
    g.arm();
    let t0 = Instant::now();
    let p = g.register(plan(WriteKind::Drop, &["1"]), 0, t0).unwrap();
    assert_eq!(
        g.redeem(&p.token, "CONFIRM DROP 1", t0 + Duration::from_secs(11))
            .err(),
        Some(GateError::Expired)
    );
    let p2 = g.register(plan(WriteKind::Drop, &["1"]), 0, t0).unwrap();
    g.disarm();
    g.arm();
    assert_eq!(
        g.redeem(&p2.token, "CONFIRM DROP 1", t0).err(),
        Some(GateError::UnknownToken)
    );
}

#[test]
fn kinds_only_allow_their_own_actions() {
    let mut g: Gate<()> = Gate::new(Duration::from_secs(60));
    g.arm();
    let now = Instant::now();
    for (k, ok, bad) in [
        (
            WriteKind::SubmitEnrol,
            "VC_PE_SS_WRK_VC_SUBMIT_ENROL_PB",
            "#ICYes",
        ),
        (
            WriteKind::SwapCommit,
            "CQ_SS_SWAP_WRK_CONTINUE_PB",
            "DERIVED_SSR_FL_SSR_DROPPED",
        ),
        (
            WriteKind::StageEnrol,
            "SSR_CLSRCH_F_WK_SSR_OPTION_DESCR$1",
            "VC_PE_SS_WRK_VC_SUBMIT_ENROL_PB",
        ),
        (WriteKind::PlanTerm, "DESCR10$19", "VC_PE_SS_WRK_VC_ENROL_PB"),
        (WriteKind::PlanTerm, "VC_PE_SS_WRK_VC_SAVE_PLAN_PB", "VC_PE_SS_WRK_VC_SUBMIT_ENROL_PB"),
        (WriteKind::PlanTerm, "DESCR10$0", "VC_PE_HDR_DVW_DESCR100$0"),
        (WriteKind::PlanTerm, "DESCR10$3", "DERIVED_SSR_FL_SSR_DROPPED"),
        (WriteKind::SubmitEnrol, "VC_PE_SS_WRK_VC_SUBMIT_ENROL_PB", "VC_PE_SS_WRK_VC_SAVE_PLAN_PB"),
        (WriteKind::Drop, "#ICYes", "DESCR10$19"),
    ] {
        let pl = plan(k, &["5"]);
        let ph = pl.required_phrase.clone();
        let p = g.register(pl, (), now).unwrap();
        let (proof, _, _) = g.redeem(&p.token, &ph, now).unwrap();
        assert!(proof.allows(ok), "{k:?} should allow {ok}");
        assert!(!proof.allows(bad), "{k:?} must not allow {bad}");
    }
}


#[test]
fn the_preview_lifetime_is_configurable_and_a_token_past_it_is_refused() {
    // Default 120 s: a token redeemed after 5 minutes is expired. Set to 10 minutes: the same gap is accepted.
    for (ttl, gap, ok) in [(120u64, 300u64, false), (600, 300, true), (600, 601, false), (120, 119, true)] {
        let mut g: Gate<()> = Gate::new(Duration::from_secs(120));
        g.set_ttl(Duration::from_secs(ttl));
        g.arm();
        let now = Instant::now();
        let pl = plan(WriteKind::SubmitEnrol, &["5"]);
        let ph = pl.required_phrase.clone();
        let p = g.register(pl, (), now).unwrap();
        let later = now + Duration::from_secs(gap);
        let r = g.redeem(&p.token, &ph, later);
        assert_eq!(r.is_ok(), ok, "ttl {ttl}s, redeemed after {gap}s: {:?}", r.err());
    }
}
