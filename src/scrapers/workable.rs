//! Workable boards via the public widget API, which returns every published job
//! with its description in one request.

use super::detect::Board;
use super::location::city_from_parts;
use super::text::{html_to_text, parse_terms};
use crate::{Application, City, Error, ScrapeEvent};
use reqwest::header::ACCEPT;
use reqwest::Client;
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Deserialize, Debug)]
struct Account {
    #[serde(default)]
    jobs: Vec<Job>,
}

#[derive(Deserialize, Debug)]
struct Job {
    title: String,
    shortcode: String,
    url: Option<String>,
    city: Option<String>,
    state: Option<String>,
    country: Option<String>,
    #[serde(default)]
    locations: Vec<Location>,
    description: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Location {
    city: Option<String>,
    region: Option<String>,
    country: Option<String>,
    country_code: Option<String>,
}

pub async fn scrape(company_id: i64, board: &Board, seen: &HashSet<String>, client: &Client) -> Result<Vec<ScrapeEvent>, Error> {
    let Board::Workable { account } = board else {
        return Err("not a Workable board".into());
    };

    let list: Account = client
        .get(format!("https://apply.workable.com/api/v1/widget/accounts/{account}"))
        .header(ACCEPT, "application/json")
        .query(&[("details", "true")])
        .send()
        .await?
        .error_for_status()
        .map_err(|e| format!("Workable board `{account}` request failed: {e}"))?
        .json()
        .await?;

    let mut events = Vec::new();
    for job in list.jobs {
        let url = job.url.clone().filter(|u| u.starts_with("https://")).unwrap_or_else(|| format!("https://apply.workable.com/j/{}", job.shortcode));
        if seen.contains(&url) {
            continue;
        }
        events.push(to_event(company_id, job, url));
    }
    Ok(events)
}

fn to_event(company_id: i64, job: Job, url: String) -> ScrapeEvent {
    let mut cities: Vec<City> = Vec::new();
    for location in &job.locations {
        let country = location.country_code.as_deref().or(location.country.as_deref()).unwrap_or("");
        if let Some(city) = city_from_parts(location.city.as_deref().unwrap_or(""), location.region.as_deref().unwrap_or(""), country) {
            if !cities.contains(&city) {
                cities.push(city);
            }
        }
    }
    if cities.is_empty() {
        let fallback = city_from_parts(job.city.as_deref().unwrap_or(""), job.state.as_deref().unwrap_or(""), job.country.as_deref().unwrap_or(""));
        cities.extend(fallback);
    }

    let terms = parse_terms(&job.title);

    ScrapeEvent {
        terms: if terms.is_empty() { None } else { Some(terms) },
        cities,
        application: Application {
            company_id,
            job_title: job.title.trim().to_string(),
            url,
            page_content: html_to_text(job.description.as_deref().unwrap_or("")),
            lower_wage_cents: None,
            upper_wage_cents: None,
            currency: "USD".to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_widget_jobs() {
        let json = r#"{"name":"Hugging Face","jobs":[{
            "title":"Firmware Intern - Summer 2027","shortcode":"81B46579FE","telecommuting":false,
            "url":"https://apply.workable.com/j/81B46579FE","country":"France","city":"Paris","state":"Île-de-France",
            "locations":[{"country":"France","countryCode":"FR","city":"Paris","region":"Île-de-France","hidden":false},
                         {"country":"United States","countryCode":"US","city":"","region":null,"hidden":true}],
            "description":"<p>Write <b>C</b> for MCUs.</p>"
        }]}"#;
        let account: Account = serde_json::from_str(json).unwrap();
        let job = account.jobs.into_iter().next().unwrap();
        let event = to_event(7, job, "https://apply.workable.com/j/81B46579FE".into());
        assert_eq!(event.application.job_title, "Firmware Intern - Summer 2027");
        assert_eq!(event.application.page_content, "Write C for MCUs.");
        let places: Vec<(&str, &str)> = event.cities.iter().map(|c| (c.display_name.as_str(), c.country.as_str())).collect();
        assert_eq!(places, vec![("Paris", "FR"), ("United States", "US")]);
        assert_eq!(event.terms.unwrap()[0].display_name, "S27");
    }
}
