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
}

impl Board {
    pub fn kind(&self) -> &'static str {
        match self {
            Board::Workday { .. } => "WORKDAY",
            Board::Jibe { .. } => "JIBE",
            Board::Greenhouse { .. } => "GREENHOUSE",
            Board::Lever { .. } => "LEVER",
            Board::Ashby { .. } => "ASHBY",
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
                url.to_string()
            }
            Board::Jibe { base, query } => {
                let mut url = Url::parse(&format!("{base}/careers-home/jobs")).expect("valid jibe url");
                {
                    let mut pairs = url.query_pairs_mut();
                    for (name, value) in query {
                        pairs.append_pair(name, value);
                    }
                }
                url.to_string()
            }
            Board::Greenhouse { token } => format!("https://job-boards.greenhouse.io/{token}"),
            Board::Lever { token } => format!("https://jobs.lever.co/{token}"),
            Board::Ashby { token } => format!("https://jobs.ashbyhq.com/{token}"),
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
        }
    }
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
            if value.is_empty() {
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

    if segments.first() == Some(&"careers-home") {
        let base = format!("{}://{}", url.scheme(), host);
        let query = url
            .query_pairs()
            .filter(|(k, v)| k != "page" && !v.is_empty())
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        return Some(Board::Jibe { base, query });
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
    let html = client
        .get(url.clone())
        .header(reqwest::header::ACCEPT, "text/html,*/*")
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;

    if let Some(board) = from_html(&html, &url) {
        return Ok(board);
    }

    Err(format!(
        "Couldn't find a supported job board (Workday, Jibe/iCIMS, Greenhouse, Lever, Ashby) at {raw}. \
         Try passing the board URL itself, e.g. the `myworkdayjobs.com`, `greenhouse.io`, `lever.co` or `ashbyhq.com` link."
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

    let workday = Regex::new(r"https?://([a-z0-9-]+\.wd\d+\.myworkdayjobs\.com)/(?:[a-z]{2}-[A-Z]{2}/)?([A-Za-z0-9_-]+)").unwrap();
    for caps in workday.captures_iter(html) {
        let site = &caps[2];
        if site == "wday" || site == "job" {
            continue;
        }
        return from_board_url(&format!("https://{}/{}", &caps[1], site));
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
        let amd = from_board_url("https://careers.amd.com/careers-home/jobs?country=Canada&page=1").unwrap();
        assert_eq!(amd, Board::Jibe { base: "https://careers.amd.com".into(), query: vec![("country".into(), "Canada".into())] });
        assert_eq!(amd.suggested_name(), "Amd");
        assert_eq!(from_board_url("https://www.example.com/careers"), None);
    }

    #[test]
    fn html_scan() {
        let page = Url::parse("https://www.etched.com/careers").unwrap();
        let html = r#"<a href="https://jobs.ashbyhq.com/etched/0338e794-38f8">Apply</a>"#;
        assert_eq!(from_html(html, &page), Some(Board::Ashby { token: "etched".into() }));
        let html = r#"<script src="https://app.jibecdn.com/prod/cdn.js"></script>"#;
        assert!(matches!(from_html(html, &page), Some(Board::Jibe { .. })));
    }
}
