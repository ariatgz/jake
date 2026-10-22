//! Workday career sites. Uses the same JSON endpoints the site's own front end
//! calls: `POST /wday/cxs/{tenant}/{site}/jobs` for the list and
//! `GET /wday/cxs/{tenant}/{site}{externalPath}` for one posting.

use super::detect::Board;
use super::location::parse_location;
use super::text::{html_to_text, parse_terms};
use super::DETAIL_CONCURRENCY;
use crate::{Application, City, Error, ScrapeEvent};
use futures::{stream, StreamExt};
use reqwest::header::ACCEPT;
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashSet;
use std::time::Duration;

const PAGE_SIZE: u32 = 20;
const MAX_POSTINGS: u32 = 2000;

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct ListResponse {
    total: Option<u32>,
    #[serde(default)]
    job_postings: Vec<Posting>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Posting {
    external_path: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct DetailResponse {
    job_posting_info: Info,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Info {
    title: String,
    #[serde(default)]
    job_description: String,
    location: Option<String>,
    #[serde(default)]
    additional_locations: Vec<String>,
    country: Option<Country>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Country {
    alpha2_code: Option<String>,
}

fn job_url(host: &str, site: &str, external_path: &str) -> String {
    format!("https://{host}/{site}{external_path}")
}

pub async fn scrape(company_id: i64, board: &Board, seen: &HashSet<String>, client: &Client) -> Result<Vec<ScrapeEvent>, Error> {
    let Board::Workday { host, tenant, site, facets, search_text } = board else {
        return Err("not a Workday board".into());
    };

    let api = format!("https://{host}/wday/cxs/{tenant}/{site}");

    let mut applied = serde_json::Map::new();
    for (name, values) in facets {
        applied.insert(name.clone(), json!(values));
    }

    let mut offset: u32 = 0;
    let mut total: Option<u32> = None; // Workday only reports `total` on the first page
    let mut new_paths: Vec<String> = Vec::new();

    loop {
        let body = json!({
            "appliedFacets": applied,
            "limit": PAGE_SIZE,
            "offset": offset,
            "searchText": search_text,
        });

        let page: ListResponse = client
            .post(format!("{api}/jobs"))
            .header(ACCEPT, "application/json")
            .json(&body)
            .send()
            .await?
            .error_for_status()
            .map_err(|e| format!("Workday list request failed for {host}/{site}: {e}"))?
            .json()
            .await?;

        if let Some(t) = page.total.filter(|t| *t > 0) {
            total = Some(t); // later pages report total=0, so keep the first real value
        }

        let count = page.job_postings.len() as u32;
        if std::env::var_os("SCRAPER_DEBUG").is_some() {
            eprintln!("workday {host}/{site}: offset={offset} got={count} total={:?}", page.total);
        }
        for posting in page.job_postings {
            if !seen.contains(&job_url(host, site, &posting.external_path)) {
                new_paths.push(posting.external_path);
            }
        }

        offset += count;
        if count < PAGE_SIZE || offset >= total.unwrap_or(MAX_POSTINGS) || offset >= MAX_POSTINGS {
            break;
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }

    let results: Vec<Result<ScrapeEvent, Error>> = stream::iter(new_paths)
        .map(|path| {
            let client = client.clone();
            let api = api.clone();
            let host = host.clone();
            let site = site.clone();
            async move { fetch_detail(company_id, &client, &api, &host, &site, &path).await }
        })
        .buffer_unordered(DETAIL_CONCURRENCY)
        .collect()
        .await;

    let mut events = Vec::new();
    for result in results {
        match result {
            Ok(event) => events.push(event),
            Err(e) => eprintln!("Workday detail failed ({host}/{site}): {e}"),
        }
    }
    Ok(events)
}

async fn fetch_detail(company_id: i64, client: &Client, api: &str, host: &str, site: &str, path: &str) -> Result<ScrapeEvent, Error> {
    let detail: DetailResponse = client
        .get(format!("{api}{path}"))
        .header(ACCEPT, "application/json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let info = detail.job_posting_info;

    let country_code = info.country.and_then(|c| c.alpha2_code);

    let mut raw_locations: Vec<String> = Vec::new();
    if let Some(location) = info.location {
        raw_locations.push(location);
    }
    raw_locations.extend(info.additional_locations);

    let mut cities: Vec<City> = Vec::new();
    for raw in &raw_locations {
        let mut city = parse_location(raw);
        if city.country.is_empty() {
            if let Some(code) = &country_code {
                city.country = code.clone();
            }
        }
        if !cities.contains(&city) {
            cities.push(city);
        }
    }

    let title = info.title.trim().to_string();
    let terms = parse_terms(&title);

    Ok(ScrapeEvent {
        terms: if terms.is_empty() { None } else { Some(terms) },
        cities,
        application: Application {
            company_id,
            job_title: title,
            url: job_url(host, site, path),
            page_content: html_to_text(&info.job_description),
            lower_wage_cents: None,
            upper_wage_cents: None,
            currency: "USD".to_string(),
        },
    })
}
