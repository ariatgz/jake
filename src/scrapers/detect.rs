//! Works out which job-board platform a URL belongs to, either from the URL
//! itself or by scanning a company's careers page for an embedded board.

use crate::Error;
use regex::Regex;
use reqwest::Client;
use url::Url;

#[derive(Debug, Clone, PartialEq)]
pub enum Board {
    Workday {
        host: String,
        tenant: String,
        site: String,
        /// Facet name -> selected value ids, taken from the URL query string.
        facets: Vec<(String, Vec<String>)>,
        search_text: String,
    },
    /// Jibe / iCIMS career sites (e.g. careers.amd.com).
    Jibe {
        base: String,
        query: Vec<(String, String)>,
    },
    Greenhouse { token: String },
    Lever { token: String },
    Ashby { token: String },
    /// Oracle Cloud HCM candidate experience sites (e.g. onsemi).
    Oracle {
        /// `https://hctz.fa.us2.oraclecloud.com`
        base: String,
        /// `CX_1001`
        site: String,
        /// Free-text search applied to the listing, usually "Intern".
        keyword: String,
    },
    SmartRecruiters {
        /// Company identifier, e.g. `BoschGroup`.
        company: String,
        /// Optional search (`?q=` in the board URL) that narrows a large listing.
        query: String,
    },
    Workable { account: String },
}

impl Board {
    pub fn kind(&self) -> &'static str {
        match self {
            Board::Workday { .. } => "WORKDAY",
            Board::Jibe { .. } => "JIBE",
            Board::Greenhouse { .. } => "GREENHOUSE",
            Board::Lever { .. } => "LEVER",
            Board::Ashby { .. } => "ASHBY",
            Board::Oracle { .. } => "ORACLE",
            Board::SmartRecruiters { .. } => "SMARTRECRUITERS",
            Board::Workable { .. } => "WORKABLE",
        }
    }

    /// Stable URL stored in `companies.board_url`; parsing it again yields the same board.
    pub fn canonical_url(&self) -> String {
        match self {
            Board::Workday { host, site, facets, search_text, .. } => {
                let mut url = Url::parse(&format!("https://{host}/{site}")).expect("valid workday url");
                {
                    let mut pairs = url.query_pairs_mut();
                    for (name, values) in facets {
                        for value in values {
                            pairs.append_pair(name, value);
                        }
                    }
                    if !search_text.is_empty() {
                        pairs.append_pair("q", search_text);
                    }
                }
                without_empty_query(url)
            }
            Board::Jibe { base, query } => {
                let mut url = Url::parse(&format!("{base}/careers-home/jobs")).expect("valid jibe url");
                {
                    let mut pairs = url.query_pairs_mut();
                    for (name, value) in query {
                        pairs.append_pair(name, value);
                    }
                }
                without_empty_query(url)
            }
            Board::Greenhouse { token } => format!("https://job-boards.greenhouse.io/{token}"),
            Board::Lever { token } => format!("https://jobs.lever.co/{token}"),
            Board::Ashby { token } => format!("https://jobs.ashbyhq.com/{token}"),
            Board::Oracle { base, site, keyword } => {
                let mut url = Url::parse(&format!("{base}/hcmUI/CandidateExperience/en/sites/{site}/requisitions")).expect("valid oracle url");
                if !keyword.is_empty() {
                    url.query_pairs_mut().append_pair("keyword", keyword);
                }
                without_empty_query(url)
            }
            Board::SmartRecruiters { company, query } => {
                let mut url = Url::parse(&format!("https://jobs.smartrecruiters.com/{company}")).expect("valid smartrecruiters url");
                if !query.is_empty() {
                    url.query_pairs_mut().append_pair("q", query);
                }
                without_empty_query(url)
            }
            Board::Workable { account } => format!("https://apply.workable.com/{account}"),
        }
    }

    /// A readable default company name when the user doesn't supply one.
    pub fn suggested_name(&self) -> String {
        match self {
            Board::Workday { tenant, .. } => titleize(tenant),
            Board::Jibe { base, .. } => Url::parse(base)
                .ok()
                .and_then(|u| u.host_str().map(second_level_domain))
                .map(|d| titleize(&d))
                .unwrap_or_else(|| "Company".to_string()),
            Board::Greenhouse { token } | Board::Lever { token } | Board::Ashby { token } => titleize(token),
            Board::SmartRecruiters { company, .. } => titleize(company),
            Board::Workable { account } => titleize(account),
            // Oracle hosts are shared (hctz.fa.us2...), so the URL says nothing about the company.
            Board::Oracle { .. } => "Company".to_string(),
        }
    }
}

/// Analytics and ad-click parameters that sites append to links. They are not
/// board facets, and Workday answers HTTP 400 when they are sent as one.
pub fn is_tracking_param(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    k.starts_with('_')
        || k.starts_with("utm_")
        || matches!(k.as_str(), "gclid" | "gbraid" | "wbraid" | "fbclid" | "msclkid" | "dclid" | "mc_cid" | "mc_eid" | "ref" | "source" | "src")
}

/// `query_pairs_mut` leaves a bare "?" when nothing was appended.
fn without_empty_query(mut url: Url) -> String {
    if url.query() == Some("") {
        url.set_query(None);
    }
    url.to_string()
}

fn titleize(slug: &str) -> String {
    slug.split(['-', '_', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// "careers.amd.com" -> "amd"
pub fn second_level_domain(host: &str) -> String {
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() >= 2 { parts[parts.len() - 2].to_string() } else { host.to_string() }
}

fn is_locale_segment(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    bytes.len() == 5
        && bytes[2] == b'-'
        && bytes[..2].iter().all(|b| b.is_ascii_lowercase())
        && bytes[3..].iter().all(|b| b.is_ascii_uppercase())
}

/// SmartRecruiters paths that aren't a company identifier.
fn is_smartrecruiters_page(segment: &str) -> bool {
    matches!(segment, "oneclick-ui" | "external-referrals" | "sr-jobs" | "v1")
}

/// Workable paths and subdomains that aren't an account name.
fn is_workable_page(segment: &str) -> bool {
    matches!(segment, "j" | "api" | "www" | "apply" | "jobs" | "help" | "resources")
}

/// Recognises a URL that points directly at a supported board. No network access.
pub fn from_board_url(raw: &str) -> Option<Board> {
    let url = Url::parse(raw.trim()).ok()?;
    let host = url.host_str()?.to_lowercase();
    let segments: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();

    if host.ends_with(".myworkdayjobs.com") {
        let tenant = host.split('.').next()?.to_string();
        let site = segments.iter().find(|s| !is_locale_segment(s))?.to_string();
        if site == "wday" {
            return None;
        }
        let mut facets: Vec<(String, Vec<String>)> = Vec::new();
        let mut search_text = String::new();
        for (key, value) in url.query_pairs() {
            if value.is_empty() || is_tracking_param(&key) {
                continue;
            }
            if key == "q" {
                search_text = value.to_string();
                continue;
            }
            if let Some((_, values)) = facets.iter_mut().find(|(k, _)| *k == key) {
                values.push(value.to_string());
            } else {
                facets.push((key.to_string(), vec![value.to_string()]));
            }
        }
        return Some(Board::Workday { host, tenant, site, facets, search_text });
    }

    if host.ends_with("greenhouse.io") {
        let token = match segments.as_slice() {
            ["v1", "boards", token, ..] => token.to_string(),
            ["embed", ..] => url.query_pairs().find(|(k, _)| k == "for").map(|(_, v)| v.to_string())?,
            [token, ..] => token.to_string(),
            [] => return None,
        };
        return Some(Board::Greenhouse { token: token.to_lowercase() });
    }

    if host == "jobs.lever.co" || host == "api.lever.co" {
        let token = match segments.as_slice() {
            ["v0", "postings", token, ..] => token,
            [token, ..] => token,
            [] => return None,
        };
        return Some(Board::Lever { token: token.to_lowercase() });
    }

    if host == "jobs.ashbyhq.com" || host == "api.ashbyhq.com" {
        let token = match segments.as_slice() {
            ["posting-api", "job-board", token, ..] => token,
            [token, ..] => token,
            [] => return None,
        };
        return Some(Board::Ashby { token: token.to_string() });
    }

    if matches!(host.as_str(), "jobs.smartrecruiters.com" | "careers.smartrecruiters.com" | "api.smartrecruiters.com") {
        let company = match segments.as_slice() {
            ["v1", "companies", company, ..] => company,
            [company, ..] if !is_smartrecruiters_page(company) => company,
            _ => return None,
        };
        let query = url.query_pairs().find(|(k, _)| k == "q").map(|(_, v)| v.trim().to_string()).unwrap_or_default();
        return Some(Board::SmartRecruiters { company: company.to_string(), query });
    }

    if host == "apply.workable.com" {
        let account = match segments.as_slice() {
            ["api", _, "widget", "accounts", account, ..] => account,
            [account, ..] if !is_workable_page(account) => account,
            _ => return None,
        };
        return Some(Board::Workable { account: account.to_string() });
    }

    // Older Workable boards live on a subdomain: https://acme.workable.com
    if let Some(account) = host.strip_suffix(".workable.com") {
        if !account.contains('.') && !is_workable_page(account) {
            return Some(Board::Workable { account: account.to_string() });
        }
    }

    if segments.first() == Some(&"careers-home") {
        let base = format!("{}://{}", url.scheme(), host);
        let query = url
            .query_pairs()
            .filter(|(k, v)| k != "page" && !v.is_empty() && !is_tracking_param(k))
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        return Some(Board::Jibe { base, query });
    }

    // https://hctz.fa.us2.oraclecloud.com/hcmUI/CandidateExperience/en/sites/CX_1001/requisitions?keyword=Intern
    if host.ends_with(".oraclecloud.com") {
        let site = segments.iter().position(|s| *s == "sites").and_then(|i| segments.get(i + 1))?.to_string();
        let keyword = url.query_pairs().find(|(k, _)| k == "keyword").map(|(_, v)| v.trim().to_string()).unwrap_or_default();
        return Some(Board::Oracle { base: format!("https://{host}"), site, keyword });
    }

    None
}

/// Like `from_board_url`, but falls back to downloading the page and looking
/// for a board embedded in it (e.g. a company careers page that links to Greenhouse).
pub async fn detect(client: &Client, raw: &str) -> Result<Board, Error> {
    if let Some(board) = from_board_url(raw) {
        return Ok(board);
    }

    let url = Url::parse(raw.trim()).map_err(|_| format!("`{raw}` is not a valid URL"))?;
    let response = client
        .get(url.clone())
        .header(reqwest::header::ACCEPT, "text/html,application/xhtml+xml,*/*;q=0.8")
        .send()
        .await
        .map_err(|e| format!("Couldn't load {raw}: {e}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "{raw} answered HTTP {}. Some careers sites block bots; paste the job board link itself instead \
             (the `myworkdayjobs.com`, `greenhouse.io`, `lever.co`, `ashbyhq.com`, `smartrecruiters.com`, `workable.com` or `oraclecloud.com` URL behind the page).",
            response.status()
        )
        .into());
    }
    let html = response.text().await?;

    if let Some(board) = from_html(&html, &url) {
        return Ok(board);
    }

    Err(format!(
        "Couldn't find a supported job board (Workday, Jibe/iCIMS, Greenhouse, Lever, Ashby, SmartRecruiters, Workable, Oracle HCM) at {raw}. \
         Try passing the board URL itself, e.g. the `myworkdayjobs.com`, `greenhouse.io`, `lever.co`, `ashbyhq.com`, `smartrecruiters.com` or `workable.com` link."
    )
    .into())
}

fn from_html(html: &str, page: &Url) -> Option<Board> {
    let greenhouse = Regex::new(
        r"(?:boards|job-boards)\.greenhouse\.io/([A-Za-z0-9_-]+)|boards-api\.greenhouse\.io/v1/boards/([A-Za-z0-9_-]+)|greenhouse\.io/embed/job_board(?:/js)?\?for=([A-Za-z0-9_-]+)",
    )
    .unwrap();
    if let Some(caps) = greenhouse.captures(html) {
        let token = caps.get(1).or(caps.get(2)).or(caps.get(3))?.as_str();
        return Some(Board::Greenhouse { token: token.to_lowercase() });
    }

    let lever = Regex::new(r"(?:jobs\.lever\.co|api\.lever\.co/v0/postings)/([A-Za-z0-9-]+)").unwrap();
    if let Some(caps) = lever.captures(html) {
        return Some(Board::Lever { token: caps[1].to_lowercase() });
    }

    let ashby = Regex::new(r"(?:jobs\.ashbyhq\.com|api\.ashbyhq\.com/posting-api/job-board)/([A-Za-z0-9_.-]+)").unwrap();
    if let Some(caps) = ashby.captures(html) {
        return Some(Board::Ashby { token: caps[1].to_string() });
    }

    let smartrecruiters =
        Regex::new(r"(?:jobs|careers)\.smartrecruiters\.com/([A-Za-z0-9_-]+)|api\.smartrecruiters\.com/v1/companies/([A-Za-z0-9_-]+)").unwrap();
    for caps in smartrecruiters.captures_iter(html) {
        if let Some(company) = caps.get(1).or(caps.get(2)).map(|m| m.as_str()).filter(|c| !is_smartrecruiters_page(c)) {
            return Some(Board::SmartRecruiters { company: company.to_string(), query: String::new() });
        }
    }

    // Separate patterns: one alternation would let the subdomain form swallow `https://apply.workable.com`.
    let workable = [
        Regex::new(r"apply\.workable\.com/(?:api/v1/widget/accounts/)?([A-Za-z0-9_-]+)").unwrap(),
        Regex::new(r"https?://([a-z0-9-]+)\.workable\.com").unwrap(),
    ];
    for pattern in &workable {
        for caps in pattern.captures_iter(html) {
            if !is_workable_page(&caps[1]) {
                return Some(Board::Workable { account: caps[1].to_string() });
            }
        }
    }

    let workday = Regex::new(r"https?://([a-z0-9-]+\.wd\d+\.myworkdayjobs\.com)/(?:[a-z]{2}-[A-Z]{2}/)?([A-Za-z0-9_-]+)").unwrap();
    for caps in workday.captures_iter(html) {
        let site = &caps[2];
        if site == "wday" || site == "job" {
            continue;
        }
        return from_board_url(&format!("https://{}/{}", &caps[1], site));
    }

    let oracle = Regex::new(r"https?://([a-z0-9-]+\.fa\.[a-z0-9-]+\.oraclecloud\.com)/hcmUI/CandidateExperience/[a-z]{2}/sites/([A-Za-z0-9_]+)").unwrap();
    if let Some(caps) = oracle.captures(html) {
        return Some(Board::Oracle { base: format!("https://{}", &caps[1]), site: caps[2].to_string(), keyword: "Intern".to_string() });
    }

    if html.contains("jibecdn.com") {
        return Some(Board::Jibe {
            base: format!("{}://{}", page.scheme(), page.host_str()?),
            query: Vec::new(),
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workday_urls() {
        let board = from_board_url("https://nvidia.wd5.myworkdayjobs.com/NVIDIAExternalCareerSite?jobFamilyGroup=0c40f6bd1d8f10ae43ffda1e8d447e94").unwrap();
        match &board {
            Board::Workday { host, tenant, site, facets, .. } => {
                assert_eq!(host, "nvidia.wd5.myworkdayjobs.com");
                assert_eq!(tenant, "nvidia");
                assert_eq!(site, "NVIDIAExternalCareerSite");
                assert_eq!(facets, &vec![("jobFamilyGroup".to_string(), vec!["0c40f6bd1d8f10ae43ffda1e8d447e94".to_string()])]);
            }
            _ => panic!("expected workday"),
        }
        assert_eq!(from_board_url(&board.canonical_url()), Some(board));

        // Analytics parameters copied from the address bar are not facets.
        let marvell = from_board_url(
            "https://marvell.wd1.myworkdayjobs.com/MarvellCareers?_gl=1*12801fn*_gcl_au*NTA0&utm_source=x&workerSubType=65dea26481d001e09dfbab4927173419",
        )
        .unwrap();
        assert_eq!(
            marvell.canonical_url(),
            "https://marvell.wd1.myworkdayjobs.com/MarvellCareers?workerSubType=65dea26481d001e09dfbab4927173419"
        );

        let intel = from_board_url("https://intel.wd1.myworkdayjobs.com/en-US/External/job/x/y").unwrap();
        assert!(matches!(intel, Board::Workday { ref site, .. } if site == "External"));
        assert_eq!(intel.suggested_name(), "Intel");
    }

    #[test]
    fn other_boards() {
        assert_eq!(from_board_url("https://job-boards.greenhouse.io/asteralabs/jobs/123"), Some(Board::Greenhouse { token: "asteralabs".into() }));
        assert_eq!(from_board_url("https://boards-api.greenhouse.io/v1/boards/doordashusa/jobs"), Some(Board::Greenhouse { token: "doordashusa".into() }));
        assert_eq!(from_board_url("https://jobs.lever.co/palantir"), Some(Board::Lever { token: "palantir".into() }));
        assert_eq!(from_board_url("https://jobs.ashbyhq.com/etched/7aeafe31"), Some(Board::Ashby { token: "etched".into() }));
        let onsemi = from_board_url("https://hctz.fa.us2.oraclecloud.com/hcmUI/CandidateExperience/en/sites/CX_1001/requisitions?keyword=Intern&mode=location").unwrap();
        assert_eq!(onsemi, Board::Oracle { base: "https://hctz.fa.us2.oraclecloud.com".into(), site: "CX_1001".into(), keyword: "Intern".into() });
        assert_eq!(from_board_url(&onsemi.canonical_url()), Some(onsemi));
        let amd = from_board_url("https://careers.amd.com/careers-home/jobs?country=Canada&page=1").unwrap();
        assert_eq!(amd, Board::Jibe { base: "https://careers.amd.com".into(), query: vec![("country".into(), "Canada".into())] });
        assert_eq!(amd.suggested_name(), "Amd");
        assert_eq!(from_board_url("https://www.example.com/careers"), None);
    }

    #[test]
    fn smartrecruiters_and_workable_urls() {
        let bosch = Board::SmartRecruiters { company: "BoschGroup".into(), query: String::new() };
        assert_eq!(from_board_url("https://jobs.smartrecruiters.com/BoschGroup"), Some(bosch.clone()));
        assert_eq!(from_board_url("https://jobs.smartrecruiters.com/BoschGroup/744000157754859-csr-lead"), Some(bosch.clone()));
        assert_eq!(from_board_url("https://api.smartrecruiters.com/v1/companies/BoschGroup/postings"), Some(bosch.clone()));
        assert_eq!(from_board_url("https://jobs.smartrecruiters.com/oneclick-ui/company/BoschGroup/publication/x"), None);
        let interns = from_board_url("https://careers.smartrecruiters.com/BoschGroup?q=intern").unwrap();
        assert_eq!(interns, Board::SmartRecruiters { company: "BoschGroup".into(), query: "intern".into() });
        assert_eq!(interns.canonical_url(), "https://jobs.smartrecruiters.com/BoschGroup?q=intern");
        assert_eq!(from_board_url(&interns.canonical_url()), Some(interns));

        let hf = Board::Workable { account: "huggingface".into() };
        assert_eq!(from_board_url("https://apply.workable.com/huggingface/"), Some(hf.clone()));
        assert_eq!(from_board_url("https://apply.workable.com/huggingface/j/81B46579FE/"), Some(hf.clone()));
        assert_eq!(from_board_url("https://apply.workable.com/api/v1/widget/accounts/huggingface"), Some(hf.clone()));
        assert_eq!(from_board_url("https://huggingface.workable.com"), Some(hf.clone()));
        assert_eq!(from_board_url("https://apply.workable.com/j/81B46579FE"), None);
        assert_eq!(from_board_url(&hf.canonical_url()), Some(hf));
    }

    #[test]
    fn html_scan() {
        let page = Url::parse("https://www.etched.com/careers").unwrap();
        let html = r#"<a href="https://jobs.ashbyhq.com/etched/0338e794-38f8">Apply</a>"#;
        assert_eq!(from_html(html, &page), Some(Board::Ashby { token: "etched".into() }));
        let html = r#"<script src="https://app.jibecdn.com/prod/cdn.js"></script>"#;
        assert!(matches!(from_html(html, &page), Some(Board::Jibe { .. })));
        let html = r#"<a href="https://jobs.smartrecruiters.com/oneclick-ui/x">x</a><a href="https://jobs.smartrecruiters.com/BoschGroup">Jobs</a>"#;
        assert_eq!(from_html(html, &page), Some(Board::SmartRecruiters { company: "BoschGroup".into(), query: String::new() }));
        let html = r#"<a href="https://apply.workable.com/j/81B46579FE">x</a><a href="https://apply.workable.com/huggingface/">Jobs</a>"#;
        assert_eq!(from_html(html, &page), Some(Board::Workable { account: "huggingface".into() }));
    }
}
