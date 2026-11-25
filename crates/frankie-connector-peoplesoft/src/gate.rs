//! The write gate. A write is possible only through: writes armed, then `preview` (nothing sent that changes
//! anything), then a one-time token, then the exact phrase typed by the human. The proof that comes out names the
//! single action it allows. Nothing a page says can create a proof.
use frankie_contracts::canonical_json;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteKind {
    /// Pick a class option; the server remembers it as the pending choice.
    StageEnrol,
    /// Submit the pending Plan & Enrol choices (may involve fees).
    SubmitEnrol,
    /// Drop one enrolled class.
    Drop,
    /// Pick the class to swap in.
    SwapStage,
    /// Confirm the swap on the review page.
    SwapCommit,
    /// Set one planned course's term and Save Planning (course planning only; never enrols).
    PlanTerm,
}

impl WriteKind {
    pub const fn word(self) -> &'static str {
        match self {
            WriteKind::StageEnrol => "STAGE",
            WriteKind::SubmitEnrol => "ENROLL",
            WriteKind::Drop => "DROP",
            WriteKind::SwapStage => "SWAPSTAGE",
            WriteKind::SwapCommit => "SWAP",
            WriteKind::PlanTerm => "PLANTERM",
        }
    }
    /// The only `ICAction` values this kind may send.
    pub const fn actions(self) -> &'static [&'static str] {
        match self {
            WriteKind::StageEnrol | WriteKind::SwapStage => &["SSR_CLSRCH_F_WK_SSR_OPTION_DESCR"],
            WriteKind::SubmitEnrol => &["VC_PE_SS_WRK_VC_SUBMIT_ENROL_PB"],
            WriteKind::Drop => &["DERIVED_SSR_FL_SSR_DROPPED", "#ICYes"],
            WriteKind::SwapCommit => &["CQ_SS_SWAP_WRK_CONTINUE_PB"],
            // `DESCR10$` is a prefix (one term dropdown per row). The Enrol click is not here and never will be.
            WriteKind::PlanTerm => &["DESCR10$", "VC_PE_SS_WRK_VC_SAVE_PLAN_PB"],
        }
    }
}

/// What a confirmed write may send. Only [`Gate::redeem`] creates one.
#[derive(Debug)]
pub struct WriteProof {
    kind: WriteKind,
}

impl WriteProof {
    pub fn allows(&self, action: &str) -> bool {
        self.kind
            .actions()
            .iter()
            .any(|a| action == *a || ((a.ends_with("DESCR") || a.ends_with('$')) && action.starts_with(a)))
    }
    pub fn kind(&self) -> WriteKind {
        self.kind
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlannedStep {
    pub component: String,
    pub icaction: String,
}

/// Exactly what a confirmation would do. Shown to the human before anything is sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WritePlan {
    pub kind: WriteKind,
    pub term: String,
    pub class_numbers: Vec<String>,
    pub summary: String,
    pub steps: Vec<PlannedStep>,
    pub warnings: Vec<String>,
    /// The exact text the human must type.
    pub required_phrase: String,
    /// For a submit: what the live Plan & Enrol page itself lists, as (course label, status, selected-class summary). This is read from the page,
    /// not remembered, so a caller can compare it with what it meant to submit.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub live_rows: Vec<(String, String, String)>,
    /// For a drop: the class numbers the term's enrolled list showed when this preview read it (a page checked to be that
    /// term's list), before anything was selected. Lets a caller prove an earlier drop from the read it already made.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listed_classes: Option<Vec<String>>,
}

impl WritePlan {
    pub fn new(
        kind: WriteKind,
        term: &str,
        class_numbers: Vec<String>,
        summary: String,
        steps: Vec<PlannedStep>,
        warnings: Vec<String>,
    ) -> Self {
        let required_phrase = phrase_for(kind, &class_numbers);
        Self {
            kind,
            term: term.to_string(),
            class_numbers,
            summary,
            steps,
            warnings,
            required_phrase,
            live_rows: Vec::new(),
            listed_classes: None,
        }
    }

    pub fn with_listed_classes(mut self, nums: Vec<String>) -> Self {
        self.listed_classes = Some(nums);
        self
    }

    pub fn with_live_rows(mut self, rows: Vec<(String, String, String)>) -> Self {
        self.live_rows = rows;
        self
    }
}

/// `CONFIRM DROP 3496`, `CONFIRM ENROLL 3378 3379`, `CONFIRM SWAP 3496 TO 3497 3498`.
/// For a swap commit the first number is the class given up, the rest the new components.
pub fn phrase_for(kind: WriteKind, nums: &[String]) -> String {
    match (kind, nums.split_first()) {
        // The commit lists every class given up, then `TO`, then every class taken: `CONFIRM SWAP 3382 3496 TO 3378 3379 3500 3501`.
        (WriteKind::SwapCommit, Some(_)) if nums.iter().any(|n| n == "TO") => {
            let i = nums.iter().position(|n| n == "TO").unwrap_or(0);
            format!("CONFIRM SWAP {} TO {}", nums[..i].join(" "), nums[i + 1..].join(" "))
        }
        (WriteKind::SwapCommit, Some((old, new))) => {
            format!("CONFIRM SWAP {old} TO {}", new.join(" "))
        }
        _ => format!("CONFIRM {} {}", kind.word(), nums.join(" ")),
    }
}

pub struct Preview {
    pub plan: WritePlan,
    /// One-time token bound to this exact plan. Not secret, but useless without the phrase.
    pub token: String,
    pub ttl: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateError {
    Disarmed,
    UnknownToken,
    Expired,
    PhraseMismatch,
    TooManyAttempts,
}

struct Pending<P> {
    plan: WritePlan,
    payload: P,
    created: Instant,
    failures: u8,
}

pub struct Gate<P> {
    armed: bool,
    ttl: Duration,
    pending: HashMap<String, Pending<P>>,
}

const MAX_PHRASE_FAILURES: u8 = 3;

impl<P> Gate<P> {
    pub fn new(ttl: Duration) -> Self {
        Self {
            armed: false,
            ttl,
            pending: HashMap::new(),
        }
    }
    /// How long a preview lives before it must be made again. A host that stages work and submits it later (Apply stages enrolments first and
    /// submits after the drops, plans and swaps) sets this to the longest gap it will accept; the default of the connector is 120 s.
    /// Tokens already issued keep the lifetime they were issued with only if it is shorter: the new value applies to them too.
    pub fn set_ttl(&mut self, ttl: Duration) {
        self.ttl = ttl;
    }
    pub fn ttl(&self) -> Duration {
        self.ttl
    }
    pub fn is_armed(&self) -> bool {
        self.armed
    }
    pub fn arm(&mut self) {
        self.armed = true;
    }
    /// Disarming also drops every pending preview.
    pub fn disarm(&mut self) {
        self.armed = false;
        self.pending.clear();
    }

    /// Store a plan and hand back its token. Refuses while disarmed (so a preview cannot even be requested).
    pub fn register(
        &mut self,
        plan: WritePlan,
        payload: P,
        now: Instant,
    ) -> Result<Preview, GateError> {
        if !self.armed {
            return Err(GateError::Disarmed);
        }
        self.pending
            .retain(|_, p| now.duration_since(p.created) < self.ttl);
        let mut nonce = [0u8; 16];
        getrandom::getrandom(&mut nonce).map_err(|_| GateError::Disarmed)?;
        let mut h = Sha256::new();
        h.update(nonce);
        h.update(canonical_json(&serde_json::to_value(&plan).expect("plan serialises")).as_bytes());
        let token: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        self.pending.insert(
            token.clone(),
            Pending {
                plan: plan.clone(),
                payload,
                created: now,
                failures: 0,
            },
        );
        Ok(Preview {
            plan,
            token,
            ttl: self.ttl,
        })
    }

    /// Redeem a token with the typed phrase. On success the token is burned and the proof, plan and payload are
    /// returned. A wrong phrase does not burn the token until three failures; an expired token is burned.
    pub fn redeem(
        &mut self,
        token: &str,
        typed: &str,
        now: Instant,
    ) -> Result<(WriteProof, WritePlan, P), GateError> {
        if !self.armed {
            return Err(GateError::Disarmed);
        }
        let p = self.pending.get_mut(token).ok_or(GateError::UnknownToken)?;
        if now.duration_since(p.created) >= self.ttl {
            self.pending.remove(token);
            return Err(GateError::Expired);
        }
        if typed != p.plan.required_phrase {
            p.failures += 1;
            if p.failures >= MAX_PHRASE_FAILURES {
                self.pending.remove(token);
                return Err(GateError::TooManyAttempts);
            }
            return Err(GateError::PhraseMismatch);
        }
        let p = self.pending.remove(token).expect("present");
        Ok((WriteProof { kind: p.plan.kind }, p.plan, p.payload))
    }
}
