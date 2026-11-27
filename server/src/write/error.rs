use crate::log;
use crate::ApiErr;
use axum::http::StatusCode;
use frankie_connector_peoplesoft::GateError;

pub const SAID_STAGES: [&str; 7] = [
    "drop_click",
    "drop_yes",
    "drop_result",
    "drop_review",
    "enrol_submit",
    "stage_option",
    "swap_commit",
];

pub fn err(code: StatusCode, m: impl Into<String>) -> ApiErr {
    ApiErr(code, m.into())
}

pub fn needs_login(m: &str) -> bool {
    m.contains("sign-in required")
}

pub fn gate_msg(g: GateError) -> String {
    match g {
        GateError::Disarmed => "Writes are off.".into(),
        GateError::UnknownToken => "That preview is no longer valid. Preview again.".into(),
        GateError::Expired => "The preview expired. Preview again.".into(),
        GateError::PhraseMismatch => "The phrase did not match.".into(),
        GateError::TooManyAttempts => "Too many wrong phrases. Preview again.".into(),
    }
}

/// Why a confirm failed, and whether anything may have reached PeopleSoft before it did.
#[derive(Debug, Clone)]
pub struct ConfirmFail {
    pub message: String,
    pub may_have_sent: bool,
}

impl std::fmt::Display for ConfirmFail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl From<String> for ConfirmFail {
    fn from(message: String) -> Self {
        Self { message, may_have_sent: false }
    }
}

impl ConfirmFail {
    pub fn text(&self) -> String {
        if self.may_have_sent {
            format!(
                "{} This step may or may not have gone through on PeopleSoft's side. Check PeopleSoft before doing anything again.",
                self.message
            )
        } else {
            self.message.clone()
        }
    }
}

/// Pulls PeopleSoft's own outcome/error sentences from a write-stage response without logging raw HTML or personal data.
/// Starts with the connector's `outcome_hints`, then falls back to message/error fields and status-icon error text when
/// `outcome_hints` missed a refusal (e.g. "corequisite", "waitlist", "limit", or a message inside a `<div>` or `<p>`).
pub fn extract_said(_stage: &str, body: &str) -> Vec<String> {
    let fs = frankie_connector_peoplesoft::pia::fields(body);
    let mut out: Vec<String> = frankie_connector_peoplesoft::pia::outcome_hints(&fs)
        .into_iter()
        .map(|h| log::trunc(&h, 200))
        .collect();
    let push_unique = |dst: &mut Vec<String>, s: &str| {
        let t = log::trunc(s, 200);
        if !t.is_empty() && !dst.contains(&t) {
            dst.push(t);
        }
    };
    const EXTRA_WORDS: [&str; 13] = [
        "corequisite",
        "co-requisite",
        "requisite",
        "waitlist",
        "wait list",
        "closed",
        "full",
        "limit",
        "maximum",
        "exceed",
        "credit",
        "reserved",
        "permission",
    ];
    let has_error_icon = body.contains("PS_CS_STATUS_ERROR_ICN");
    for f in &fs {
        if out.len() >= 6 {
            break;
        }
        if f.text.is_empty() || f.text.len() >= 400 {
            continue;
        }
        let name_upper = f.name.to_ascii_uppercase();
        if name_upper.contains("AMOUNT") || name_upper.contains("PAYMENT") || name_upper.contains("DESCR50") {
            continue;
        }
        let low = f.text.to_ascii_lowercase();
        let is_msg_field =
            name_upper.contains("MSG") || name_upper.contains("ERR") || name_upper.contains("DESCRLONG") || name_upper.contains("ALERT");
        if EXTRA_WORDS.iter().any(|w| low.contains(w)) || (is_msg_field && (has_error_icon || _stage == "enrol_submit")) {
            push_unique(&mut out, &f.text);
        }
    }
    if out.is_empty() && has_error_icon {
        let mut rest = body;
        while let Some(pos) = rest.find("PS_CS_STATUS_ERROR_ICN") {
            let after_icon = rest[pos..].find('>').map(|i| pos + i + 1).unwrap_or(pos + "PS_CS_STATUS_ERROR_ICN".len());
            let chunk = &rest[after_icon..rest.len().min(after_icon + 900)];
            let mut plain = String::with_capacity(200);
            let mut in_tag = false;
            for ch in chunk.chars() {
                match ch {
                    '<' => {
                        in_tag = true;
                        plain.push(' ');
                    }
                    '>' => in_tag = false,
                    _ if !in_tag => plain.push(ch),
                    _ => {}
                }
            }
            let cleaned = plain
                .replace("&nbsp;", " ")
                .replace("&amp;", "&")
                .replace("&quot;", "\"")
                .replace("&#39;", "'");
            let trimmed: String = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
            if !trimmed.is_empty() && trimmed.len() >= 8 {
                push_unique(&mut out, &trimmed);
                if out.len() >= 6 {
                    break;
                }
            }
            rest = &rest[pos + "PS_CS_STATUS_ERROR_ICN".len()..];
        }
    }
    out.truncate(6);
    out
}
