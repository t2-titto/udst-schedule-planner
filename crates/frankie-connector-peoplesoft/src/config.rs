use url::Url;

/// Where the connector may talk. Everything is checked against this before a request leaves.
#[derive(Debug, Clone)]
pub struct PeopleSoftConfig {
    /// `https://campus.udst.edu.qa` (https, default port, no userinfo).
    pub origin: Url,
    /// PeopleSoft site name seen in every path of the capture (`/psc/<site>/EMPLOYEE/<node>/c/...`).
    pub site: String,
    /// Portal node used for student pages (`SA` in the capture; some guided pages used `PSFT_CS`).
    pub node: String,
    /// Hosts a redirect or a form may point to (lower case).
    pub allowed_hosts: Vec<String>,
    /// Minimum gap between two requests. Seat status must never be polled in a tight loop.
    pub min_interval_ms: u64,
    /// Menu link id of the Plan & Enrol entry, observed in the capture (tenant-wide, not personal).
    pub plan_enrol_pslnkid: String,
    /// Menu link ids for the drop and swap entries, observed in the capture (tenant-wide, not personal).
    pub drop_pslnkid: String,
    pub swap_pslnkid: String,
    /// Responses larger than this are refused.
    pub max_body_bytes: usize,
}

impl PeopleSoftConfig {
    pub fn udst() -> Self {
        Self {
            origin: Url::parse("https://campus.udst.edu.qa").expect("static url"),
            site: "csqaprd9".into(),
            node: "SA".into(),
            allowed_hosts: vec!["campus.udst.edu.qa".into()],
            min_interval_ms: 1000,
            plan_enrol_pslnkid: "ADMN_S202107161056567928708761".into(),
            drop_pslnkid: "CS_S201607141805059946044413".into(),
            swap_pslnkid: "ADMN_S202204141107003454695272".into(),
            max_body_bytes: 8 * 1024 * 1024,
        }
    }

    pub fn host_allowed(&self, u: &Url) -> bool {
        u.scheme() == "https"
            && u.port().is_none()
            && u.username().is_empty()
            && u.password().is_none()
            && u.host_str()
                .is_some_and(|h| self.allowed_hosts.iter().any(|a| a.eq_ignore_ascii_case(h)))
    }

    /// `https://host/psc/<site>/EMPLOYEE/<node>/c/<COMPONENT>.GBL` with the given query pairs.
    pub fn component_url(&self, component: &str, query: &[(&str, &str)]) -> Url {
        self.component_url_node(&self.node, component, query)
    }

    /// Same, for a page that lives on another portal node (the swap pages used `PSFT_CS` in the capture).
    pub fn component_url_node(&self, node: &str, component: &str, query: &[(&str, &str)]) -> Url {
        let mut u = self.origin.clone();
        u.set_path(&format!(
            "/psc/{}/EMPLOYEE/{}/c/{}.GBL",
            self.site, node, component
        ));
        if !query.is_empty() {
            let mut q = u.query_pairs_mut();
            for (k, v) in query {
                q.append_pair(k, v);
            }
        }
        u
    }

    /// `https://host/psc/<site>/EMPLOYEE/<node>/s/<WEBLIB...>` (an iScript).
    pub fn iscript_url(&self, name: &str, query: &[(&str, &str)]) -> Url {
        let mut u = self.origin.clone();
        u.set_path(&format!(
            "/psc/{}/EMPLOYEE/{}/s/{}",
            self.site, self.node, name
        ));
        if !query.is_empty() {
            let mut q = u.query_pairs_mut();
            for (k, v) in query {
                q.append_pair(k, v);
            }
        }
        u
    }

    pub fn login_url(&self) -> Url {
        let mut u = self.origin.clone();
        u.set_path(&format!("/psp/{}/", self.site));
        u.query_pairs_mut()
            .append_pair("cmd", "login")
            .append_pair("languageCd", "ENG");
        u
    }
}
