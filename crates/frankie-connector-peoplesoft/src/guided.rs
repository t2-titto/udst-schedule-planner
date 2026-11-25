//! Activity-guide (guided process) steps. The ids below are fixed definitions seen in the capture
//! (`PTAI_LIST_ID`, `PTAI_ITEM_ID`, `PTAI_POSTPROC`); they are tenant configuration, not session values.
//! UNVERIFIED beyond the single capture: if UDST changes the guide, these pages answer with an unexpected shape
//! and the connector fails closed.
use crate::config::PeopleSoftConfig;
use url::Url;

pub struct GuidedStep {
    pub node: &'static str,
    pub component: &'static str,
    pub page: &'static str,
    pub list_id: &'static str,
    pub item_id: &'static str,
    pub postproc: Option<&'static str>,
}

pub const DROP_SELECT: GuidedStep = GuidedStep {
    node: "SA",
    component: "SSR_STUDENT_FL.SSR_DROP_SELECT_FL",
    page: "SSR_DROP_SELECT_FL",
    list_id: "SR_DROP_FL345306",
    item_id: "SR_DROP_FL1_SR_DROP_FL345306",
    postproc: Some("SSR_AG_DROP_CLASSES_FL:ActionItemProcessingAdapter:SelectClassesToDrop"),
};
pub const DROP_REVIEW: GuidedStep = GuidedStep {
    node: "SA",
    component: "SSR_STUDENT_FL.SSR_DROP_REVIEW_FL",
    page: "SSR_DROP_REVIEW_FL",
    list_id: "SR_DROP_FL345306",
    item_id: "SR_DROP_FL2_SR_DROP_FL345306",
    postproc: Some("SSR_AG_DROP_CLASSES_FL:ActionItemProcessingAdapter:ReviewClassesToDrop"),
};
pub const SWAP_SELECT: GuidedStep = GuidedStep {
    node: "PSFT_CS",
    component: "CNAQ_MENU_FL.CQ_SS_SWAP_FL",
    page: "CQ_SS_SWAP_SEL_FL",
    list_id: "CQ_SS_SWAP247106",
    item_id: "CQ_SS_SWAP3_CQ_SS_SWAP247106",
    postproc: None,
};
pub const SWAP_REVIEW: GuidedStep = GuidedStep {
    node: "PSFT_CS",
    component: "CNAQ_MENU_FL.CQ_SS_SWAP_FL",
    page: "CQ_SS_SWAP_REV_FL",
    list_id: "CQ_SS_SWAP247106",
    item_id: "CQ_SS_SWAP4_CQ_SS_SWAP247106",
    postproc: None,
};

/// The drop guide's ids for THIS session. The numeric suffix of `SR_DROP_FL<n>` is an instance id that PeopleSoft hands out when
/// the Drop Classes guide's grouplet loads; the fixed constants above came from one capture and are NOT valid for another
/// session. A drop sent with a stale id is answered normally but silently does nothing (seen against the real site: the final
/// yes was accepted, 200, and the class stayed enrolled). The browser's HAR shows the real order: the start page lists
/// grouplet URLs, the browser loads them, and the step links in their answer carry the live ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropGuide {
    pub list_id: String,
    pub select_item: String,
    pub review_item: String,
}

/// Grouplet URLs (header, then navigation) for a template, as listed on the guide's start page. Each has `bReload=y` added, as
/// the browser does.
pub fn grouplet_urls(html: &str, template_id: &str) -> Vec<String> {
    let re = regex::Regex::new(r#"https://[^'"\s<>\\]*PTGP_GROUPLETS_FL\.GBL\?[^'"\s<>\\]*"#).expect("static regex");
    let mut hv = None;
    let mut nv = None;
    for m in re.find_iter(html) {
        let u = m.as_str().replace("&amp;", "&");
        if !u.contains(&format!("TEMPLATE_ID={template_id}")) || !u.contains("PTGP_TYPE=ActivityGuideDataSource") {
            continue;
        }
        if u.contains("PAGE=PTGP_GPLT_HV_FL") && hv.is_none() {
            hv = Some(u);
        } else if u.contains("PAGE=PTGP_GPLT_NV_FL") && nv.is_none() {
            nv = Some(u);
        }
    }
    hv.into_iter().chain(nv).map(|u| if u.contains("bReload=") { u } else { format!("{u}&bReload=y") }).collect()
}

/// The iScript the browser calls first, listed on the same start page. It creates this session's guide instance; without it the
/// grouplets answer "A new Activity Guide Instance cannot be created from the context data" (seen against the real site).
pub fn preload_url(html: &str, template_id: &str) -> Option<String> {
    let re = regex::Regex::new(r#"https://[^'"\s<>\\]*WEBLIB_PTAI\.ISCRIPT3\.FieldFormula\.IScript_FluidAGPreLoad\?[^'"\s<>\\]*"#).expect("static regex");
    let found = re
        .find_iter(html)
        .map(|m| m.as_str().replace("&amp;", "&"))
        .find(|u| u.contains(&format!("TEMPLATE_ID={template_id}")));
    found.map(|u| if u.contains("nWidth=") { u } else { format!("{u}&nWidth=0&nHeight=0") })
}

/// Read the live ids from a navigation grouplet's step links. Fails closed: every id must be present and share one instance.
pub fn parse_drop_guide(html: &str) -> Option<DropGuide> {
    let html = html.replace("&amp;", "&").replace("&#039;", "'");
    let one = |pat: &str| -> Option<String> {
        regex::Regex::new(pat).expect("static regex").captures(&html).map(|c| c[1].to_string())
    };
    let list_id = one(r"PTAI_LIST_ID=(SR_DROP_FL\d+)")?;
    let select_item = one(r"PTAI_ITEM_ID=(SR_DROP_FL1_SR_DROP_FL\d+)")?;
    let review_item = one(r"PTAI_ITEM_ID=(SR_DROP_FL2_SR_DROP_FL\d+)")?;
    let n = list_id.trim_start_matches("SR_DROP_FL");
    if select_item != format!("SR_DROP_FL1_{list_id}") || review_item != format!("SR_DROP_FL2_{list_id}") || n.is_empty() {
        return None;
    }
    Some(DropGuide { list_id, select_item, review_item })
}

impl GuidedStep {
    pub fn url(&self, cfg: &PeopleSoftConfig) -> Url {
        self.url_with(cfg, self.list_id, self.item_id)
    }

    /// The same step with this session's guide ids.
    pub fn url_with(&self, cfg: &PeopleSoftConfig, list_id: &str, item_id: &str) -> Url {
        let mut q: Vec<(&str, &str)> = vec![
            ("PAGE", self.page),
            ("PTAI", "T"),
            ("PTAI_LIST_ID", list_id),
            ("PTAI_ITEM_ID", item_id),
        ];
        if let Some(p) = self.postproc {
            q.push(("PTAI_POSTPROC", p));
        }
        q.extend([
            ("PTAI_SRCNODE", "PSFT_CS"),
            ("ICAJAX", "1"),
            ("ICAGTarget", "start"),
            ("ICGuided", "true"),
            ("ICPanelControlStyle", "pst_side1-fixed pst_panel-mode "),
        ]);
        cfg.component_url_node(self.node, self.component, &q)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NV: &str = "<a href=\"https://campus.udst.edu.qa/psc/csqaprd9_newwin/EMPLOYEE/SA/c/SSR_STUDENT_FL.SSR_DROP_SELECT_FL.GBL?PAGE=SSR_DROP_SELECT_FL&amp;PTAI=T&amp;PTAI_LIST_ID=SR_DROP_FL345474&amp;PTAI_ITEM_ID=SR_DROP_FL1_SR_DROP_FL345474&amp;PTAI_POSTPROC=x\"></a><a href=\"https://h/c/SSR_STUDENT_FL.SSR_DROP_REVIEW_FL.GBL?PAGE=SSR_DROP_REVIEW_FL&amp;PTAI_LIST_ID=SR_DROP_FL345474&amp;PTAI_ITEM_ID=SR_DROP_FL2_SR_DROP_FL345474\"></a>";

    #[test]
    fn reads_the_live_ids_and_not_the_captured_constants() {
        let g = parse_drop_guide(NV).unwrap();
        assert_eq!(g.list_id, "SR_DROP_FL345474");
        assert_eq!(g.select_item, "SR_DROP_FL1_SR_DROP_FL345474");
        assert_eq!(g.review_item, "SR_DROP_FL2_SR_DROP_FL345474");
        assert_ne!(g.list_id, DROP_SELECT.list_id);
    }

    #[test]
    fn mixed_instances_or_missing_steps_fail_closed() {
        assert!(parse_drop_guide(&NV.replace("FL2_SR_DROP_FL345474", "FL2_SR_DROP_FL111111")).is_none());
        assert!(parse_drop_guide("<html>nothing</html>").is_none());
        assert!(parse_drop_guide(&NV.replace("PTAI_ITEM_ID=SR_DROP_FL2_SR_DROP_FL345474", "")).is_none());
    }

    #[test]
    fn finds_the_preload_url_on_the_start_page() {
        let page = "['c','5','https://campus.udst.edu.qa/psc/csqaprd9_newwin/EMPLOYEE/SA/s/WEBLIB_PTAI.ISCRIPT3.FieldFormula.IScript_FluidAGPreLoad?TEMPLATE_ID=SR_DROP_FL&OPRID=1&ICDoModal=1&ICGrouplet=1&ICLoc=1','','0']";
        let u = preload_url(page, "SR_DROP_FL").unwrap();
        assert!(u.ends_with("&nWidth=0&nHeight=0") && u.contains("TEMPLATE_ID=SR_DROP_FL&"));
        assert!(preload_url(page, "OTHER").is_none());
        assert!(preload_url("nothing", "SR_DROP_FL").is_none());
    }

    #[test]
    fn finds_the_two_grouplet_urls_on_the_start_page() {
        let page = "x=[['a','5','https://campus.udst.edu.qa/psc/csqaprd9_newwin/EMPLOYEE/SA/c/PTGP_MENU.PTGP_GROUPLETS_FL.GBL?PTGP_TYPE=ActivityGuideDataSource&PAGE=PTGP_GPLT_HV_FL&AGComp=H&TEMPLATE_ID=SR_DROP_FL&OPRID=1&ICDoModal=1&ICGrouplet=1&ICAction=GROUPLET_CONTENTS$hmodal&ICLoc=0','','0'],['b','5','https://campus.udst.edu.qa/psc/csqaprd9_newwin/EMPLOYEE/SA/c/PTGP_MENU.PTGP_GROUPLETS_FL.GBL?PTGP_TYPE=ActivityGuideDataSource&PAGE=PTGP_GPLT_NV_FL&AGComp=N&TEMPLATE_ID=SR_DROP_FL&OPRID=1&ICDoModal=1&ICGrouplet=1&ICAction=GROUPLET_CONTENTS$hmodal&ICLoc=1','','0'],['c','5','https://campus.udst.edu.qa/psc/csqaprd9_newwin/EMPLOYEE/SA/c/PTGP_MENU.PTGP_GROUPLETS_FL.GBL?PTGP_TYPE=ActivityGuideDataSource&PAGE=PTGP_GPLT_NV_FL&AGComp=N&TEMPLATE_ID=OTHER&ICLoc=1']]";
        let u = grouplet_urls(page, "SR_DROP_FL");
        assert_eq!(u.len(), 2);
        assert!(u[0].contains("PTGP_GPLT_HV_FL") && u[1].contains("PTGP_GPLT_NV_FL"));
        assert!(u.iter().all(|x| x.ends_with("&bReload=y") && x.contains("TEMPLATE_ID=SR_DROP_FL&")));
    }
}
