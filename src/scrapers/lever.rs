//! Generic Lever job boards via the public postings API.

use super::detect::Board;
use super::location::parse_location;
use super::text::{annual_to_hourly_cents, hourly_to_cents, html_to_text, parse_terms, wage_pair};
use crate::{Application, City, Error, ScrapeEvent};
use reqwest::Client;
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Job {
    text: String,
    hosted_url: String,
    #[serde(default)]
    description_plain: String,
    #[serde(default)]
    lists: Vec<List>,
    categories: Option<Categories>,
    country: Option<String>,
    salary_range: Option<SalaryRange>,
}

#[derive(Deserialize, Debug)]
struct List {
    #[serde(default)]
    text: String,
    #[serde(default)]
    content: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Categories {
    location: Option<String>,
    #[serde(default)]
    all_locations: Vec<String>,
}

#[derive(Deserialize, Debug)]
struct SalaryRange {
    min: Option<f64>,
    max: Option<f64>,
    currency: Option<String>,
    interval: Option<String>,
}

pub async fn scrape(company_id: i64, board: &Board, seen: &HashSet<String>, client: &Client) -> Result<Vec<ScrapeEvent>, Error> {
    let Board::Lever { token } = board else {
        return Err("not a Lever board".into());
    };

    let jobs: Vec<Job> = client
        .get(format!("https://api.lever.co/v0/postings/{token}"))
        .query(&[("mode", "json")])
        .send()
        .await?
        .error_for_status()
        .map_err(|e| format!("Lever board `{token}` request failed: {e}"))?
        .json()
        .await?;

    let mut events = Vec::new();
    for job in jobs {
        if seen.contains(&job.hosted_url) {
            continue;
        }

        let mut raw_locations: Vec<String> = Vec::new();
        if let Some(cats) = &job.categories {
            raw_locations.extend(cats.all_locations.iter().cloned());
            if raw_locations.is_empty() {
                if let Some(location) = &cats.location {
                    raw_locations.push(location.clone());
                }
            }
        }
        let mut cities: Vec<City> = Vec::new();
        for raw in &raw_locations {
            let mut city = parse_location(raw);
            if city.country.is_empty() {
                if let Some(code) = &job.country {
                    city.country = code.to_uppercase();
                }
            }
            if !cities.contains(&city) {
                cities.push(city);
            }
        }

        let mut body = job.description_plain.clone();
        for list in &job.lists {
            body.push('\n');
            body.push_str(&list.text);
            body.push('\n');
            body.push_str(&html_to_text(&list.content));
        }

        let mut currency = "USD".to_string();
        let (mut lower, mut upper) = (None, None);
        if let Some(range) = &job.salary_range {
            let per_hour = range.interval.as_deref().map(|i| i.to_lowercase().contains("hour")).unwrap_or(false);
            let convert = |v: f64| if per_hour { hourly_to_cents(v) } else { annual_to_hourly_cents(v) };
            lower = range.min.and_then(convert);
            upper = range.max.and_then(convert);
            if let Some(c) = &range.currency {
                currency = c.to_uppercase();
            }
        }
        let (lower, upper) = wage_pair(lower, upper);

        let terms = parse_terms(&job.text);

        events.push(ScrapeEvent {
            terms: if terms.is_empty() { None } else { Some(terms) },
            cities,
            application: Application {
                company_id,
                job_title: job.text.trim().to_string(),
                url: job.hosted_url,
                page_content: body.split_whitespace().collect::<Vec<_>>().join(" "),
                lower_wage_cents: lower,
                upper_wage_cents: upper,
                currency,
            },
        });
    }

    Ok(events)
}
