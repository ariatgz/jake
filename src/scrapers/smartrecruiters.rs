//! SmartRecruiters company boards via the public postings API:
//! `GET /v1/companies/{company}/postings` for the list and
//! `GET /v1/companies/{company}/postings/{id}` for one posting.

use super::detect::Board;
use super::location::city_from_parts;
use super::text::{html_to_text, parse_terms};
use super::DETAIL_CONCURRENCY;
use crate::{Application, Error, ScrapeEvent};
use futures::{stream, StreamExt};
use reqwest::header::ACCEPT;
use reqwest::Client;
use serde::Deserialize;
use std::collections::HashSet;
use std::time::Duration;

const PAGE_SIZE: u32 = 100;
const MAX_POSTINGS: u32 = 2000;

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct ListResponse {
    total_found: Option<u32>,
    #[serde(default)]
    content: Vec<Posting>,
}

#[derive(Deserialize, Debug)]
struct Posting {
    id: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Detail {
    name: String,
    location: Option<Location>,
    job_ad: Option<JobAd>,
}

#[derive(Deserialize, Debug)]
struct Location {
    city: Option<String>,
    region: Option<String>,
    country: Option<String>,
}

#[derive(Deserialize, Debug)]
struct JobAd {
    sections: Option<Sections>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Sections {
    company_description: Option<Section>,
    job_description: Option<Section>,
    qualifications: Option<Section>,
    additional_information: Option<Section>,
}

#[derive(Deserialize, Debug)]
struct Section {
    title: Option<String>,
    text: Option<String>,
}

/// The short form of a posting's page; SmartRecruiters redirects it to the full one.
fn job_url(company: &str, id: &str) -> String {
    format!("https://jobs.smartrecruiters.com/{company}/{id}")
}

pub async fn scrape(company_id: i64, board: &Board, seen: &HashSet<String>, client: &Client) -> Result<Vec<ScrapeEvent>, Error> {
    let Board::SmartRecruiters { company, query } = board else {
        return Err("not a SmartRecruiters board".into());
    };

    let api = format!("https://api.smartrecruiters.com/v1/companies/{company}/postings");
    let mut offset: u32 = 0;
    let mut new_ids: Vec<String> = Vec::new();

    loop {
        let mut request = client
            .get(&api)
            .header(ACCEPT, "application/json")
            .query(&[("limit", PAGE_SIZE), ("offset", offset)]);
        if !query.is_empty() {
            request = request.query(&[("q", query)]);
        }
        let page: ListResponse = request
            .send()
            .await?
            .error_for_status()
            .map_err(|e| format!("SmartRecruiters board `{company}` request failed: {e}"))?
            .json()
            .await?;

        let count = page.content.len() as u32;
        for posting in page.content {
            if !seen.contains(&job_url(company, &posting.id)) {
                new_ids.push(posting.id);
            }
        }

        offset += count;
        if count < PAGE_SIZE || offset >= page.total_found.unwrap_or(0) || offset >= MAX_POSTINGS {
            break;
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }

    let results: Vec<Result<ScrapeEvent, Error>> = stream::iter(new_ids)
        .map(|id| {
            let client = client.clone();
            let api = api.clone();
            async move { fetch_detail(company_id, &client, &api, company, &id).await }
        })
        .buffer_unordered(DETAIL_CONCURRENCY)
        .collect()
        .await;

    let mut events = Vec::new();
    for result in results {
        match result {
            Ok(event) => events.push(event),
            Err(e) => eprintln!("SmartRecruiters detail failed ({company}): {e}"),
        }
    }
    Ok(events)
}

async fn fetch_detail(company_id: i64, client: &Client, api: &str, company: &str, id: &str) -> Result<ScrapeEvent, Error> {
    let detail: Detail = client
        .get(format!("{api}/{id}"))
        .header(ACCEPT, "application/json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let cities = detail
        .location
        .and_then(|l| city_from_parts(l.city.as_deref().unwrap_or(""), l.region.as_deref().unwrap_or(""), l.country.as_deref().unwrap_or("")))
        .into_iter()
        .collect();

    let mut body: Vec<String> = Vec::new();
    if let Some(sections) = detail.job_ad.and_then(|ad| ad.sections) {
        // Job description first so the alert preview starts with the work, not the company blurb.
        for section in [sections.job_description, sections.qualifications, sections.additional_information, sections.company_description]
            .into_iter()
            .flatten()
        {
            let text = html_to_text(section.text.as_deref().unwrap_or(""));
            if text.is_empty() {
                continue;
            }
            match section.title.filter(|t| !t.trim().is_empty()) {
                Some(title) => body.push(format!("{}: {text}", title.trim())),
                None => body.push(text),
            }
        }
    }

    let terms = parse_terms(&detail.name);

    Ok(ScrapeEvent {
        terms: if terms.is_empty() { None } else { Some(terms) },
        cities,
        application: Application {
            company_id,
            job_title: detail.name.trim().to_string(),
            url: job_url(company, id),
            page_content: body.join("\n"),
            lower_wage_cents: None,
            upper_wage_cents: None,
            currency: "USD".to_string(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_posting_detail() {
        let json = r#"{
            "id": "744000157754859",
            "name": "Hardware Design Intern - Summer 2027",
            "location": {"city": "bengaluru", "country": "in", "remote": false},
            "jobAd": {"sections": {
                "companyDescription": {"title": "Company Description", "text": "<p>We build things.</p>"},
                "jobDescription": {"title": "Job Description", "text": "<p>Design <b>PCBs</b>.</p>"},
                "qualifications": {"title": "Qualifications", "text": "<ul><li>Altium</li></ul>"},
                "additionalInformation": {"title": "Additional Information", "text": ""}
            }}
        }"#;
        let detail: Detail = serde_json::from_str(json).unwrap();
        assert_eq!(detail.name, "Hardware Design Intern - Summer 2027");
        let location = detail.location.unwrap();
        assert_eq!(location.city.as_deref(), Some("bengaluru"));
        let sections = detail.job_ad.unwrap().sections.unwrap();
        assert_eq!(sections.job_description.unwrap().text.as_deref(), Some("<p>Design <b>PCBs</b>.</p>"));

        let list: ListResponse = serde_json::from_str(r#"{"offset":0,"limit":1,"totalFound":1391,"content":[{"id":"744000157754859","name":"x"}]}"#).unwrap();
        assert_eq!(list.total_found, Some(1391));
        assert_eq!(list.content[0].id, "744000157754859");
        assert_eq!(job_url("BoschGroup", "744000157754859"), "https://jobs.smartrecruiters.com/BoschGroup/744000157754859");
    }
}
