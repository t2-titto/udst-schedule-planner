//! Credentials, held only in memory.
//!
//! Reads used to spawn the connector binary for every request, which meant a fresh sign-in each time and a credentials
//! file on disk for the length of the run. The connector is now linked and one signed-in session is kept in memory
//! (see `write.rs`), so no credentials file is ever written.

use zeroize::Zeroize;

#[derive(Clone)]
pub struct Creds {
    pub user: String,
    pub pass: String,
}

impl Drop for Creds {
    fn drop(&mut self) {
        self.user.zeroize();
        self.pass.zeroize();
    }
}

impl Creds {
    /// Reject anything that could break the two-line credentials file format.
    pub fn new(user: &str, pass: &str) -> Result<Self, &'static str> {
        let user = user.trim();
        if user.is_empty() || pass.is_empty() {
            return Err("enter both the student number and the password");
        }
        if user.len() > 128 || pass.len() > 256 {
            return Err("input too long");
        }
        let bad = |s: &str| s.chars().any(|c| matches!(c, '\n' | '\r' | '\0'));
        if bad(user) || bad(pass) {
            return Err("input contains a character that is not allowed");
        }
        Ok(Self {
            user: user.to_string(),
            pass: pass.to_string(),
        })
    }

    /// A hint that is safe to show in the UI: first two characters and a mask.
    pub fn hint(&self) -> String {
        let head: String = self.user.chars().take(2).collect();
        format!("{head}\u{2022}\u{2022}\u{2022}\u{2022}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creds_reject_newlines_and_empty() {
        assert!(Creds::new("", "x").is_err());
        assert!(Creds::new("a", "").is_err());
        assert!(Creds::new("a", "b\nc").is_err());
        assert!(Creds::new("a\r", "b").is_ok()); // trailing CR on the user is trimmed
        assert!(Creds::new("a", "p\r").is_err());
    }

    #[test]
    fn hint_hides_most_of_the_number() {
        assert_eq!(Creds::new("60123456", "x").unwrap().hint(), "60\u{2022}\u{2022}\u{2022}\u{2022}");
    }
}
