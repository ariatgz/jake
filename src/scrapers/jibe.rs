//! Jibe / iCIMS career sites (careers.amd.com and similar). The public
//! `/api/jobs` endpoint returns full job records including the description,
//! so no per-job request is needed.

use super::detect::Board;
use super::location::{normalize_country, normalize_region, tidy_city_name};
use super::text::{annual_to_hourly_cents, html_to_text, hourly_to_cents, parse_money, parse_terms, wage_pair};
use crate::{Application, City, Error, ScrapeEvent};
use regex::Regex;
use reqwest::header::ACCEPT;
use reqwest::Client;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

const MAX_PAGES: u32 = 100;

#[derive(Deserialize, Debug)]
struct ListResponse {
    #[serde(default)]
    jobs: Vec<Wrapper>,
    #[serde(rename = "totalCount")]
    total_count: Option<u32>,
}

#[derive(Deserialize, Debug)]
struct Wrapper {
    data: Job,
}

#[derive(Deserialize, Debug)]
struct Job {
    slug: String,
    title: String,
    #[serde(default)]
    description: String,
    city: Option<String>,
    state: Option<String>,
    country: Option<String>,
    country_code: Option<String>,
    /// Tenant-specific extras such as `tags5: ["CAD $44,500.00/Yr."]`.
    #[serde(flatten)]
    extra: HashMap<String, serde_json::Value>,
}

pub async fn scrape(company_id: i64, board: &Board, seen: &HashSet<String>, client: &Client) -> Result<Vec<ScrapeEvent>, Error> {
    let Board::Jibe { base, query } = board else {
        return Err("not a Jibe board".into());
    };

    let money = Regex::new(r"^([A-Z]{3})\s*\$?\s*([0-9][0-9,]*(?:\.[0-9]+)?)\s*/\s*(Yr|Hr)\.?$").unwrap();

    let mut events = Vec::new();
    let mut page: u32 = 1;
    let mut fetched: u32 = 0;

    loop {
        let mut params: Vec<(String, String)> = vec![
            ("page".into(), page.to_string()),
            ("sortBy".into(), "relevance".into()),
            ("descending".into(), "false".into()),
            ("internal".into(), "false".into()),
        ];
        params.extend(query.iter().cloned());

        let res: ListResponse = client
            .get(format!("{base}/api/jobs"))
            .query(&params)
            .header(ACCEPT, "application/json")
            .send()
            .await?
            .error_for_status()
            .map_err(|e| format!("Jibe list request failed for {base}: {e}"))?
            .json()
            .await?;

        let count = res.jobs.len() as u32;
        fetched += count;

        for wrapper in res.jobs {
            let job = wrapper.data;
            let url = format!("{base}/careers-home/jobs/{}", job.slug);
            if seen.contains(&url) {
                continue;
            }

            let city = City {
                display_name: tidy_city_name(job.city.as_deref().unwrap_or("")),
                region: normalize_region(job.state.as_deref().unwrap_or("")),
                country: match job.country_code.as_deref() {
                    Some(code) if !code.is_empty() => code.to_uppercase(),
                    _ => normalize_country(job.country.as_deref().unwrap_or("")),
                },
            };
            let cities = if city.display_name.is_empty() && city.country.is_empty() { vec![] } else { vec![city] };

            // Salary tags: collect every "CUR $N/Yr." style value and take the spread.
            let mut currency: Option<String> = None;
            let mut hourly: Vec<i16> = Vec::new();
            for (key, value) in &job.extra {
                if !key.starts_with("tags") {
                    continue;
                }
                let serde_json::Value::Array(items) = value else { continue };
                for item in items {
                    let Some(text) = item.as_str() else { continue };
                    let Some(caps) = money.captures(text.trim()) else { continue };
                    let Some(amount) = parse_money(&caps[2]) else { continue };
                    let cents = if &caps[3] == "Hr" { hourly_to_cents(amount) } else { annual_to_hourly_cents(amount) };
                    if let Some(cents) = cents {
                        hourly.push(cents);
                        currency.get_or_insert(caps[1].to_string());
                    }
                }
            }
            let (lower, upper) = wage_pair(hourly.iter().min().copied(), hourly.iter().max().copied());

            let terms = parse_terms(&job.title);

            events.push(ScrapeEvent {
                terms: if terms.is_empty() { None } else { Some(terms) },
                cities,
                application: Application {
                    company_id,
                    job_title: job.title.trim().to_string(),
                    url,
                    page_content: html_to_text(&job.description),
                    lower_wage_cents: lower,
                    upper_wage_cents: upper,
                    currency: currency.unwrap_or_else(|| "USD".to_string()),
                },
            });
        }

        if count == 0 || fetched >= res.total_count.unwrap_or(0) || page >= MAX_PAGES {
            break;
        }
        page += 1;
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    Ok(events)
}
