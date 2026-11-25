use regex::Regex;
use std::collections::BTreeMap;
use std::sync::OnceLock;
use url::Url;

/// The exact service endpoint a SAML response may be posted to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcsRule {
    pub host: String,
    pub path: String,
}

impl AcsRule {
    /// An ACS given by the caller (the proto engine takes it from a signed grant). Matching is as strict as for
    /// the fixed rules below: exact https host, default port and path; no query, fragment or userinfo.
    pub fn new(host: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            path: path.into(),
        }
    }
    /// D2L: `d2l.udst.edu.qa` + `/d2l/lp/auth/login/samlLogin.d2l`.
    pub fn d2l() -> Self {
        Self {
            host: "d2l.udst.edu.qa".into(),
            path: "/d2l/lp/auth/login/samlLogin.d2l".into(),
        }
    }
    /// Navigate360 (EAB): `bouncer.eab.com` + `/sso/saml2/acs`.
    pub fn navigate() -> Self {
        Self {
            host: "bouncer.eab.com".into(),
            path: "/sso/saml2/acs".into(),
        }
    }
    fn accepts(&self, u: &Url) -> bool {
        u.scheme() == "https"
            && u.host_str() == Some(self.host.as_str())
            && u.port().is_none()
            && u.path() == self.path
            && u.username().is_empty()
            && u.password().is_none()
            && u.query().is_none()
            && u.fragment().is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SamlForm {
    pub target: Url,
    /// Field name to value. Values are page content (the SAML assertion), so `Debug` of a `SamlForm` shows names only.
    pub fields: BTreeMap<String, String>,
}

/// Port of `Get-Attribute`: first `name=value` (double-quoted, single-quoted or bare), entity-decoded; empty if absent.
pub fn get_attribute(tag: &str, name: &str) -> String {
    let pattern = format!(
        r#"(?is)\b{}\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#,
        regex::escape(name)
    );
    let Ok(re) = Regex::new(&pattern) else {
        return String::new();
    };
    match re.captures(tag) {
        Some(c) => (1..=3)
            .find_map(|i| c.get(i))
            .map(|m| html_escape::decode_html_entities(m.as_str()).into_owned())
            .unwrap_or_default(),
        None => String::new(),
    }
}

/// Port of `Find-D2LSamlForm`: every `<form method=post>` whose action resolves (against `base`) to exactly the ACS,
/// with the fields of the inputs inside it plus inputs elsewhere linked by `form="<id>"`, kept only if `SAMLResponse`
/// is present and non-empty. An empty result means "no acceptable form": the caller must not post anything.
pub fn find_saml_forms(page: &str, base: &Url, acs: &AcsRule) -> Vec<SamlForm> {
    static FORM: OnceLock<Regex> = OnceLock::new();
    static INPUT: OnceLock<Regex> = OnceLock::new();
    let form_re = FORM.get_or_init(|| {
        Regex::new(r"(?is)<form\b(?P<attrs>[^>]*)>(?P<body>.*?)</form\s*>").expect("static pattern")
    });
    let input_re =
        INPUT.get_or_init(|| Regex::new(r"(?is)<input\b[^>]*>").expect("static pattern"));

    let mut found = Vec::new();
    for form in form_re.captures_iter(page) {
        let attrs = &form["attrs"];
        if !get_attribute(attrs, "method").eq_ignore_ascii_case("post") {
            continue;
        }
        let action = get_attribute(attrs, "action");
        if action.trim().is_empty() {
            continue;
        }
        let Ok(target) = base.join(&action) else {
            continue;
        };
        if !acs.accepts(&target) {
            continue;
        }
        let form_id = get_attribute(attrs, "id");
        let body = form.name("body").expect("group exists");
        let mut fields = BTreeMap::new();
        for input in input_re.find_iter(page) {
            let inside = input.start() >= body.start() && input.start() < body.end();
            let linked = !form_id.is_empty() && get_attribute(input.as_str(), "form") == form_id;
            if !inside && !linked {
                continue;
            }
            let name = get_attribute(input.as_str(), "name");
            if name.is_empty() {
                continue;
            }
            fields.insert(name, get_attribute(input.as_str(), "value"));
        }
        if fields.get("SAMLResponse").is_some_and(|v| !v.is_empty()) {
            found.push(SamlForm { target, fields });
        }
    }
    found
}

/// Exactly one acceptable form, or a refusal. Two or more is treated as an unexpected page, not "pick the first".
pub fn exactly_one(forms: Vec<SamlForm>) -> Result<SamlForm, crate::failure::AuthFailure> {
    let mut it = forms.into_iter();
    match (it.next(), it.next()) {
        (Some(f), None) => Ok(f),
        (None, _) => Err(crate::failure::AuthFailure::AcsMismatch),
        _ => Err(crate::failure::AuthFailure::UnexpectedPage),
    }
}
