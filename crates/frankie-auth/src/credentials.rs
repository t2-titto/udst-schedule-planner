use zeroize::Zeroizing;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CredentialsError {
    #[error("invalid credentials input")]
    Invalid,
    #[error("input too large")]
    TooLarge,
}

/// Username and password. Zeroized on drop; `Debug` never shows either value.
pub struct Credentials {
    username: String,
    password: Zeroizing<String>,
}

const MAX_FIELD: usize = 8192;
const DOMAIN: &str = "@udst.edu.qa";

impl Credentials {
    /// Same rules as the PowerShell: a bare 8-digit student number gets the domain appended; the result
    /// must be `<local part>` followed by the UDST domain, with no whitespace, and the password must be non-empty.
    pub fn new(username: &str, password: &str) -> Result<Self, CredentialsError> {
        if username.len() > MAX_FIELD || password.len() > MAX_FIELD {
            return Err(CredentialsError::TooLarge);
        }
        // Accept what a student actually types: a bare 8-digit ID, or `<id>@udst.edu.qa` in any letter case, with
        // stray spaces around it. Anything else (another domain, spaces inside, two `@`) is refused here, before
        // a single request is made. The result is always `<local part>@udst.edu.qa` with the domain in lower case.
        let typed = username.trim();
        let username = if typed.len() == 8 && typed.bytes().all(|b| b.is_ascii_digit()) {
            format!("{typed}{DOMAIN}")
        } else if let Some((local, domain)) = typed.rsplit_once('@') {
            if domain.eq_ignore_ascii_case(&DOMAIN[1..]) {
                format!("{local}{DOMAIN}")
            } else {
                typed.to_string() // wrong domain: rejected just below
            }
        } else {
            typed.to_string()
        };
        let local_ok = username.strip_suffix(DOMAIN).is_some_and(|l| {
            !l.is_empty() && !l.contains('@') && !l.chars().any(char::is_whitespace)
        });
        if !local_ok || password.is_empty() {
            return Err(CredentialsError::Invalid);
        }
        Ok(Self {
            username,
            password: Zeroizing::new(password.to_string()),
        })
    }

    /// Parse the stdin JSON the PowerShell scripts accept: exactly `{"username","password"}`, at most 8 KiB.
    pub fn from_stdin_json(input: &[u8]) -> Result<Self, CredentialsError> {
        if input.len() > MAX_FIELD {
            return Err(CredentialsError::TooLarge);
        }
        let v: serde_json::Value =
            serde_json::from_slice(input).map_err(|_| CredentialsError::Invalid)?;
        let obj = v.as_object().ok_or(CredentialsError::Invalid)?;
        if obj.len() != 2 || !obj.contains_key("username") || !obj.contains_key("password") {
            return Err(CredentialsError::Invalid);
        }
        let s = |k: &str| obj[k].as_str().ok_or(CredentialsError::Invalid);
        Self::new(s("username")?, s("password")?)
    }

    pub fn username(&self) -> &str {
        &self.username
    }
    /// The only way to read the password. Call it at the point of use, never store the result.
    pub fn expose_password(&self) -> &str {
        &self.password
    }
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Credentials([REDACTED])")
    }
}
