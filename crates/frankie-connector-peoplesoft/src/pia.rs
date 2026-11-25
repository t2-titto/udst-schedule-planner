//! Pure parsing of PeopleSoft PIA (Fluid) pages. No network, no clock, no secrets in output.
//!
//! Shapes below were read from a real capture (structure only) and are reproduced here with invented data:
//! * a full page holds `<form ... action=URL>` with hidden `<input name=ICSID|ICStateNum|...>` fields;
//! * every visible value is `<span class='ps_box-value' id='FIELD$ROW'>text</span>` (or an `<a id=...>` link);
//! * an action answers XML: `<PAGE id=..><FIELD id=..><![CDATA[html]]></FIELD><GENSCRIPT ..><![CDATA[js]]>`, where the
//!   script may redirect (`document.location='URL'`) or set form values (`oDoc.win9.ICStateNum.value=7`).
//!
//! Session identifiers (`ICSID`, `ICSCRIPTSID`, `ptnbsid`) are never returned by any function here.
use crate::config::PeopleSoftConfig;
use regex::Regex;
use std::collections::BTreeMap;
use std::sync::OnceLock;
use url::Url;

macro_rules! re {
    ($name:ident, $pat:expr) => {
        fn $name() -> &'static Regex {
            static R: OnceLock<Regex> = OnceLock::new();
            R.get_or_init(|| Regex::new($pat).expect("static regex"))
        }
    };
}
re!(
    re_field,
    r#"(?is)<(?:span|a|label)\b[^>]*?\bid=['"]([A-Za-z][A-Za-z0-9_$]*)['"][^>]*>(.*?)</(?:span|a|label)>"#
);
re!(
    re_id,
    r"^([A-Za-z][A-Za-z0-9_]*?)(?:\$(?:span\$)?(\d+)|\$(\d+)\$\$(?:span\$)?(\d+))?$"
);
re!(re_tag, r"(?s)<[^>]*>");
re!(re_ws, r"\s+");
re!(re_input, r"(?is)<input\b[^>]*>");
re!(
    re_attr,
    r#"(?is)\b([A-Za-z_-]+)\s*=\s*(?:'([^']*)'|"([^"]*)")"#
);
re!(re_form, r"(?is)<form\b[^>]*>");
re!(
    re_cdata,
    r"(?s)<(FIELD|GENSCRIPT)\b[^>]*>\s*<!\[CDATA\[(.*?)\]\]>\s*</(?:FIELD|GENSCRIPT)>"
);
re!(re_loc, r#"document\.location\s*=\s*['"]([^'"]+)['"]"#);
re!(
    re_setval,
    r#"win\d+\.([A-Za-z_][A-Za-z0-9_$]*)\.value\s*=\s*(?:'([^']*)'|(\d+))"#
);
re!(re_script, r"(?is)<script\b.*?</script>|<style\b.*?</style>");
re!(
    re_hint,
    r"(?i)not successful|unable to|cannot|can not|not allowed|time conflict|prerequisite|error|failed|no authori"
);

/// Fields never exposed to callers or logs.
const SECRET_FIELDS: [&str; 3] = ["ICSID", "ICSCRIPTSID", "ptnbsid"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    /// The grid/level number some ids carry (`FIELD$62$$ROW`); 0 when absent.
    pub level: u32,
    pub row: Option<u32>,
    pub text: String,
}

fn clean_text(raw: &str) -> String {
    let s = re_tag().replace_all(raw, " ");
    let s = html_escape::decode_html_entities(&s).replace('\u{a0}', " ");
    re_ws().replace_all(&s, " ").trim().to_string()
}

/// Split a PeopleSoft element id into `(field name, row)`: `FIELD$ROW`, `FIELD$span$ROW`, `FIELD$294$$span$ROW`.
pub fn split_id(id: &str) -> (String, u32, Option<u32>) {
    match re_id().captures(id) {
        Some(c) => {
            let num = |i: usize| c.get(i).and_then(|m| m.as_str().parse::<u32>().ok());
            (
                c[1].to_string(),
                num(3).unwrap_or(0),
                num(2).or_else(|| num(4)),
            )
        }
        None => (id.to_string(), 0, None),
    }
}

/// All visible fields of a page, in document order. A field that appears twice (`<span>` wrapper and `<a>` link)
/// is returned once, keeping the first non-empty text. Session identifiers are skipped.
pub fn fields(html: &str) -> Vec<Field> {
    let mut out: Vec<Field> = Vec::new();
    let mut idx: BTreeMap<(String, u32, Option<u32>), usize> = BTreeMap::new();
    for c in re_field().captures_iter(html) {
        let (name, level, row) = split_id(&c[1]);
        if SECRET_FIELDS.contains(&name.as_str())
            || name.starts_with("win")
            || name.starts_with("PT_")
            || name.contains("hdrdiv")
        {
            continue;
        }
        let text = clean_text(&c[2]);
        match idx.get(&(name.clone(), level, row)) {
            Some(&i) => {
                if out[i].text.is_empty() && !text.is_empty() {
                    out[i].text = text;
                }
            }
            None => {
                idx.insert((name.clone(), level, row), out.len());
                out.push(Field {
                    name,
                    level,
                    row,
                    text,
                });
            }
        }
    }
    out
}

/// First value of a non-row field.
pub fn field_text<'a>(fs: &'a [Field], name: &str) -> Option<&'a str> {
    fs.iter()
        .find(|f| f.name == name && f.row.is_none())
        .map(|f| f.text.as_str())
        .filter(|t| !t.is_empty())
}

/// Row-grouped values: `row -> (field name -> text)`, only for names that start with one of `prefixes`.
pub fn rows(fs: &[Field], prefixes: &[&str]) -> BTreeMap<u32, BTreeMap<String, String>> {
    let mut m: BTreeMap<u32, BTreeMap<String, String>> = BTreeMap::new();
    for f in fs {
        if let Some(r) = f.row {
            if prefixes.iter().any(|p| f.name.starts_with(p)) {
                m.entry(r)
                    .or_default()
                    .insert(f.name.clone(), f.text.clone());
            }
        }
    }
    m
}

/// Visible text of a whole page (scripts and styles removed). Used only for classification and outcome hints.
pub fn visible_text(html: &str) -> String {
    clean_text(&re_script().replace_all(html, " "))
}

/// Result-like sentences ("Enrollment not successful", "unable to add ...") from the page's own fields, never raw HTML.
pub fn outcome_hints(fs: &[Field]) -> Vec<String> {
    fs.iter()
        .filter(|f| !f.text.is_empty() && f.text.len() < 400 && re_hint().is_match(&f.text))
        .map(|f| f.text.clone())
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageKind {
    Content,
    /// The sign-in form (`userid` + `pwd`): the session is gone.
    LoginForm,
    /// PeopleSoft's own timeout or authorization banner.
    SessionExpired,
    NotAuthorized,
}

pub fn classify(html: &str) -> PageKind {
    let l = html.to_ascii_lowercase();
    let has = |n: &str| l.contains(&format!("name='{n}'")) || l.contains(&format!("name=\"{n}\""));
    if has("userid") && has("pwd") {
        return PageKind::LoginForm;
    }
    let t = visible_text(html).to_ascii_lowercase();
    if t.contains("your session has timed out")
        || t.contains("session has expired")
        || t.contains("you have been signed out")
    {
        return PageKind::SessionExpired;
    }
    if t.contains("authorization error")
        || t.contains("not authorized to")
        || t.contains("no authorization")
    {
        return PageKind::NotAuthorized;
    }
    PageKind::Content
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormError {
    NoForm,
    /// More than one `<form>` with the same role or a target outside the allow-list.
    BadTarget,
}

/// The state a POST must echo: the form's action URL plus its hidden inputs (page order kept).
#[derive(Clone, PartialEq, Eq)]
pub struct FormState {
    pub action: Url,
    pub fields: Vec<(String, String)>,
}

impl std::fmt::Debug for FormState {
    // Hidden values include the session id, so only names are shown.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "FormState({}{}, {} fields [values hidden])",
            self.action.host_str().unwrap_or("?"),
            self.action.path(),
            self.fields.len()
        )
    }
}

pub(crate) fn attrs(tag: &str) -> BTreeMap<String, String> {
    re_attr()
        .captures_iter(tag)
        .map(|c| {
            let v = c.get(2).or_else(|| c.get(3)).map_or("", |m| m.as_str());
            (
                c[1].to_ascii_lowercase(),
                html_escape::decode_html_entities(v).into_owned(),
            )
        })
        .collect()
}

impl FormState {
    /// Parse the single PIA form of a full page. The action must resolve (against `page_url`) to an allow-listed
    /// https URL on `/psc/` or `/psp/`: a form that points anywhere else is refused (exact-target rule).
    pub fn parse(html: &str, page_url: &Url, cfg: &PeopleSoftConfig) -> Result<Self, FormError> {
        let forms: Vec<_> = re_form().find_iter(html).collect();
        // The PIA page has one data form; Fluid headers may add a search form with another name. Take forms that
        // carry the ICSID hidden input.
        let mut chosen: Vec<(Url, usize)> = Vec::new();
        for (i, m) in forms.iter().enumerate() {
            let a = attrs(m.as_str());
            let end = forms.get(i + 1).map_or(html.len(), |n| n.start());
            let body = &html[m.end()..end];
            if !body.contains("ICSID") {
                continue;
            }
            let action = a.get("action").ok_or(FormError::BadTarget)?;
            let url = page_url.join(action).map_err(|_| FormError::BadTarget)?;
            let p = url.path();
            let prefix_ok = p.starts_with(&format!("/psc/{}", cfg.site))
                || p.starts_with(&format!("/psp/{}", cfg.site));
            if !cfg.host_allowed(&url) || !prefix_ok {
                return Err(FormError::BadTarget);
            }
            chosen.push((url, m.end()));
        }
        if chosen.len() > 1 {
            return Err(FormError::BadTarget);
        }
        let (action, start) = chosen.pop().ok_or(FormError::NoForm)?;
        let mut st = Self {
            action,
            fields: vec![],
        };
        st.absorb_hidden(&html[start..]);
        if !st.fields.iter().any(|(k, _)| k == "ICSID") {
            return Err(FormError::NoForm);
        }
        Ok(st)
    }

    /// Update or add hidden inputs found in `html` (page or AJAX fragment).
    pub fn absorb_hidden(&mut self, html: &str) {
        for m in re_input().find_iter(html) {
            let a = attrs(m.as_str());
            if a.get("type").map(|t| t.eq_ignore_ascii_case("hidden")) != Some(true) {
                continue;
            }
            if let Some(name) = a.get("name") {
                self.set(name, a.get("value").cloned().unwrap_or_default());
            }
        }
    }

    pub fn set(&mut self, name: &str, value: String) {
        match self.fields.iter_mut().find(|(k, _)| k == name) {
            Some(slot) => slot.1 = value,
            None => self.fields.push((name.to_string(), value)),
        }
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn state_num(&self) -> Option<u32> {
        self.get("ICStateNum").and_then(|v| v.parse().ok())
    }

    /// Apply what an XML answer says about the form (`ICStateNum.value=7`, hidden inputs inside FIELD html).
    pub fn apply(&mut self, ajax: &Ajax) {
        self.absorb_hidden(&ajax.html);
        for (k, v) in &ajax.set_values {
            if k != "ICSID" && k != "ICAction" && k != "ICResubmit" {
                self.set(k, v.clone());
            }
        }
    }
}

/// Defaults taken from a captured POST; a value already on the page wins, except the four forced ones.
const POST_DEFAULTS: [(&str, &str); 19] = [
    ("ICNAVTYPEDROPDOWN", "0"),
    ("ICElementNum", "0"),
    ("ICModelCancel", "0"),
    ("ICXPos", "0"),
    ("ICYPos", "0"),
    ("ResponsetoDiffFrame", "-1"),
    ("TargetFrameName", "None"),
    ("FacetPath", "None"),
    ("ICFocus", ""),
    ("ICSaveWarningFilter", "0"),
    ("ICChanged", "0"),
    ("ICSkipPending", "0"),
    ("ICAutoSave", "0"),
    ("ICActionPrompt", "false"),
    ("ICTypeAheadID", ""),
    ("ICDNDSrc", ""),
    ("ICPanelName", ""),
    ("ICFind", ""),
    ("ICAddCount", ""),
];

/// A valid `ICAction` token: letters, digits and `_ $ # .` only (`#ICYes`, `P_SELECT$0`, `#ICSetField...`).
pub fn valid_action(a: &str) -> bool {
    // The Enter key in the catalog's search box is posted by the browser as "#KEY" followed by a carriage return.
    a == "#KEY\r"
        || !a.is_empty()
        && a.len() <= 120
        && a.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'$' | b'#' | b'.'))
}

/// Build the url-encoded POST body for one action. `extra` replaces or adds fields (for example `P_SELECT$0=Y`).
pub fn build_post(
    form: &FormState,
    action: &str,
    extra: &[(&str, &str)],
) -> Result<String, FormError> {
    if !valid_action(action) {
        return Err(FormError::BadTarget);
    }
    let mut f = form.clone();
    for (k, v) in POST_DEFAULTS {
        if f.get(k).is_none() {
            f.set(k, v.to_string());
        }
    }
    f.set("ICAJAX", "1".into());
    f.set("ICType", "Panel".into());
    f.set("ICResubmit", "0".into());
    f.set("ICAction", action.to_string());
    for (k, v) in extra {
        f.set(k, (*v).to_string());
    }
    let mut s = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in &f.fields {
        s.append_pair(k, v);
    }
    Ok(s.finish())
}

/// What an XML (AJAX) answer carries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ajax {
    /// Concatenated FIELD html.
    pub html: String,
    /// `document.location=` target, if the script redirects.
    pub redirect: Option<String>,
    /// `winN.NAME.value=` assignments.
    pub set_values: Vec<(String, String)>,
    pub is_xml: bool,
}

pub fn unwrap_ajax(body: &str) -> Ajax {
    if !body.trim_start().starts_with("<?xml") && !body.contains("<PAGE") {
        return Ajax {
            html: body.to_string(),
            ..Default::default()
        };
    }
    let mut a = Ajax {
        is_xml: true,
        ..Default::default()
    };
    for c in re_cdata().captures_iter(body) {
        if c[1].eq_ignore_ascii_case("FIELD") {
            a.html.push_str(&c[2]);
            a.html.push('\n');
        } else {
            let js = &c[2];
            if a.redirect.is_none() {
                a.redirect = re_loc().captures(js).map(|m| m[1].to_string());
            }
            for m in re_setval().captures_iter(js) {
                let v = m.get(2).or_else(|| m.get(3)).map_or("", |x| x.as_str());
                a.set_values.push((m[1].to_string(), v.to_string()));
            }
        }
    }
    a
}

/// Resolve and vet a script redirect: allow-listed https only.
pub fn vet_redirect(target: &str, base: &Url, cfg: &PeopleSoftConfig) -> Option<Url> {
    let u = base.join(target).ok()?;
    cfg.host_allowed(&u).then_some(u)
}

/// Query value by name.
pub fn query_value(u: &Url, key: &str) -> Option<String> {
    u.query_pairs()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}
