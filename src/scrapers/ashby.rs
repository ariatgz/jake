//! Generic Ashby job boards via the public posting API.

use super::detect::Board;
use super::location::{normalize_country, normalize_region, parse_location, tidy_city_name};
use super::text::{annual_to_hourly_cents, parse_money, parse_terms, wage_pair};
use crate::{Application, City, Error, ScrapeEvent};
use reqwest::Client;
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Deserialize, Debug)]
struct ListResponse {
    #[serde(default)]
    jobs: Vec<Job>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Job {
    title: String,
    job_url: String,
    #[serde(default)]
    description_plain: String,
    location: Option<String>,
    #[serde(default)]
    secondary_locations: Vec<SecondaryLocation>,
    address: Option<Address>,
    is_listed: Option<bool>,
    compensation: Option<Compensation>,
}

#[derive(Deserialize, Debug)]
struct SecondaryLocation {
    location: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Address {
    postal_address: Option<PostalAddress>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct PostalAddress {
    address_region: Option<String>,
    address_country: Option<String>,
    address_locality: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Compensation {
    scrapeable_compensation_salary_summary: Option<String>,
}

/// "$150K - $350K" -> (lower, upper) hourly cents. Only annual summaries are understood.
fn parse_salary_summary(summary: &str) -> (Option<i16>, Option<i16>) {
    let mut values: Vec<f64> = summary
        .split(|c: char| c == '-' || c == '–' || c == '—')
        .filter_map(parse_money)
        .collect();
    values.retain(|v| *v >= 1000.0);
    let lower = values.first().copied().and_then(annual_to_hourly_cents);
    let upper = values.get(1).copied().and_then(annual_to_hourly_cents);
    wage_pair(lower, upper)
}

pub async fn scrape(company_id: i64, board: &Board, seen: &HashSet<String>, client: &Client) -> Result<Vec<ScrapeEvent>, Error> {
    let Board::Ashby { token } = board else {
        return Err("not an Ashby board".into());
    };

    let list: ListResponse = client
        .get(format!("https://api.ashbyhq.com/posting-api/job-board/{token}"))
        .query(&[("includeCompensation", "true")])
        .send()
        .await?
        .error_for_status()
        .map_err(|e| format!("Ashby board `{token}` request failed: {e}"))?
        .json()
        .await?;

    let mut events = Vec::new();
    for job in list.jobs {
        if job.is_listed == Some(false) || seen.contains(&job.job_url) {
            continue;
        }

        let mut cities: Vec<City> = Vec::new();
        match job.address.and_then(|a| a.postal_address) {
            Some(postal) => {
                let city = City {
                    display_name: tidy_city_name(postal.address_locality.as_deref().or(job.location.as_deref()).unwrap_or("")),
                    region: normalize_region(postal.address_region.as_deref().unwrap_or("")),
                    country: normalize_country(postal.address_country.as_deref().unwrap_or("")),
                };
                if !city.display_name.is_empty() || !city.country.is_empty() {
                    cities.push(city);
                }
            }
            None => {
                if let Some(location) = &job.location {
                    if !location.trim().is_empty() {
                        cities.push(parse_location(location));
                    }
                }
            }
        }
        for secondary in &job.secondary_locations {
            if let Some(location) = &secondary.location {
                let city = parse_location(location);
                if !cities.contains(&city) {
                    cities.push(city);
                }
            }
        }

        let (lower, upper) = job
            .compensation
            .and_then(|c| c.scrapeable_compensation_salary_summary)
            .map(|s| parse_salary_summary(&s))
            .unwrap_or((None, None));

        let terms = parse_terms(&job.title);

        events.push(ScrapeEvent {
            terms: if terms.is_empty() { None } else { Some(terms) },
            cities,
            application: Application {
                company_id,
                job_title: job.title.trim().to_string(),
                url: job.job_url,
                page_content: job.description_plain.split_whitespace().collect::<Vec<_>>().join(" "),
                lower_wage_cents: lower,
                upper_wage_cents: upper,
                currency: "USD".to_string(),
            },
        });
    }

    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn salary_summary() {
        let (lower, upper) = parse_salary_summary("$150K - $350K");
        assert_eq!(lower, annual_to_hourly_cents(150000.0));
        assert_eq!(upper, annual_to_hourly_cents(350000.0));
        assert_eq!(parse_salary_summary("Competitive"), (None, None));
    }
}
