//! Generic Greenhouse job boards via the public boards API.

use super::detect::Board;
use super::location::parse_location;
use super::text::{html_to_text, parse_terms, wage_pair};
use super::DETAIL_CONCURRENCY;
use crate::{Application, City, Error, ScrapeEvent};
use futures::{stream, StreamExt};
use reqwest::Client;
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Deserialize, Debug)]
struct ListResponse {
    #[serde(default)]
    jobs: Vec<Job>,
}

#[derive(Deserialize, Debug)]
struct Job {
    id: u64,
    absolute_url: String,
    title: String,
    location: Option<Location>,
    content: Option<String>,
    pay_input_ranges: Option<Vec<PayRange>>,
}

#[derive(Deserialize, Debug)]
struct Location {
    name: Option<String>,
}

#[derive(Deserialize, Debug)]
struct PayRange {
    min_cents: i64,
    max_cents: i64,
    #[serde(default)]
    title: String,
}

fn to_hourly(cents: i64, title: &str) -> Option<i16> {
    let lower = title.to_lowercase();
    let hourly = if lower.contains("hour") {
        cents
    } else if lower.contains("month") {
        cents / (40 * 52 / 12)
    } else if lower.contains("week") {
        cents / 40
    } else {
        cents / (40 * 52)
    };
    i16::try_from(hourly).ok().filter(|v| *v > 0)
}

async fn fetch_pay(client: &Client, api: &str, id: u64) -> Option<Vec<PayRange>> {
    let response = client
        .get(format!("{api}/{id}"))
        .query(&[("pay_transparency", "true")])
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?;
    response.json::<Job>().await.ok()?.pay_input_ranges
}

pub async fn scrape(company_id: i64, board: &Board, seen: &HashSet<String>, client: &Client) -> Result<Vec<ScrapeEvent>, Error> {
    let Board::Greenhouse { token } = board else {
        return Err("not a Greenhouse board".into());
    };

    let api = format!("https://boards-api.greenhouse.io/v1/boards/{token}/jobs");

    let list: ListResponse = client
        .get(&api)
        .query(&[("content", "true")])
        .send()
        .await?
        .error_for_status()
        .map_err(|e| format!("Greenhouse board `{token}` request failed: {e}"))?
        .json()
        .await?;

    let new_jobs: Vec<Job> = list.jobs.into_iter().filter(|j| !seen.contains(&j.absolute_url)).collect();

    // Pay ranges only come back from the single-job endpoint with pay_transparency=true.
    let events: Vec<ScrapeEvent> = stream::iter(new_jobs)
        .map(|job| {
            let client = client.clone();
            let api = api.clone();
            async move {
                let pay = fetch_pay(&client, &api, job.id).await;
                to_event(company_id, job, pay)
            }
        })
        .buffer_unordered(DETAIL_CONCURRENCY)
        .collect()
        .await;

    Ok(events)
}

fn to_event(company_id: i64, job: Job, pay: Option<Vec<PayRange>>) -> ScrapeEvent {
    let cities: Vec<City> = job
        .location
        .and_then(|l| l.name)
        .map(|name| {
            name.split(';')
                .map(|part| part.trim())
                .filter(|part| !part.is_empty())
                .map(parse_location)
                .collect()
        })
        .unwrap_or_default();

    let mut lower: Option<i16> = None;
    let mut upper: Option<i16> = None;
    for range in pay.unwrap_or_default() {
        if let Some(min) = to_hourly(range.min_cents, &range.title) {
            lower = Some(lower.map_or(min, |l| l.min(min)));
        }
        if let Some(max) = to_hourly(range.max_cents, &range.title) {
            upper = Some(upper.map_or(max, |u| u.max(max)));
        }
    }
    let (lower, upper) = wage_pair(lower, upper);

    let terms = parse_terms(&job.title);

    ScrapeEvent {
        terms: if terms.is_empty() { None } else { Some(terms) },
        cities,
        application: Application {
            company_id,
            job_title: job.title.trim().to_string(),
            url: job.absolute_url,
            page_content: html_to_text(job.content.as_deref().unwrap_or("")),
            lower_wage_cents: lower,
            upper_wage_cents: upper,
            currency: "USD".to_string(),
        },
    }
}
