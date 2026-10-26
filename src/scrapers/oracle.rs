//! Oracle Cloud HCM career sites (e.g. onsemi), through the same REST API the
//! site's own front end calls:
//! `GET /hcmRestApi/resources/latest/recruitingCEJobRequisitions?finder=findReqs;siteNumber=...`
//! for the list and `recruitingCEJobRequisitionDetails?finder=ById;Id=...` for one posting.

use super::detect::Board;
use super::location::parse_location;
use super::text::{html_to_text, parse_terms};
use super::DETAIL_CONCURRENCY;
use crate::{Application, City, Error, ScrapeEvent};
use futures::{stream, StreamExt};
use reqwest::header::ACCEPT;
use reqwest::Client;
use serde::Deserialize;
use std::collections::HashSet;
use std::time::Duration;

const PAGE_SIZE: u32 = 25;
const MAX_POSTINGS: u32 = 2000;

#[derive(Deserialize, Debug)]
struct ListResponse {
    #[serde(default)]
    items: Vec<ListItem>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct ListItem {
    total_jobs_count: Option<u32>,
    #[serde(default, rename = "requisitionList")]
    requisition_list: Vec<Summary>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct Summary {
    id: String,
}

#[derive(Deserialize, Debug)]
struct DetailResponse {
    #[serde(default)]
    items: Vec<Detail>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct Detail {
    title: String,
    primary_location: Option<String>,
    primary_location_country: Option<String>,
    #[serde(default)]
    external_description_str: Option<String>,
    #[serde(default)]
    external_qualifications_str: Option<String>,
    #[serde(default)]
    external_responsibilities_str: Option<String>,
    #[serde(default, rename = "secondaryLocations")]
    secondary_locations: Vec<SecondaryLocation>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct SecondaryLocation {
    name: Option<String>,
    country_code: Option<String>,
}

fn job_url(base: &str, site: &str, id: &str) -> String {
    format!("{base}/hcmUI/CandidateExperience/en/sites/{site}/job/{id}")
}

pub async fn scrape(company_id: i64, board: &Board, seen: &HashSet<String>, client: &Client) -> Result<Vec<ScrapeEvent>, Error> {
    let Board::Oracle { base, site, keyword } = board else {
        return Err("not an Oracle HCM board".into());
    };
    let api = format!("{base}/hcmRestApi/resources/latest");

    let mut offset: u32 = 0;
    let mut total: Option<u32> = None;
    let mut new_ids: Vec<String> = Vec::new();
    loop {
        let mut finder = format!("findReqs;siteNumber={site},limit={PAGE_SIZE},offset={offset},sortBy=POSTING_DATES_DESC");
        if !keyword.is_empty() {
            finder.push_str(&format!(",keyword={keyword}"));
        }
        let page: ListResponse = client
            .get(format!("{api}/recruitingCEJobRequisitions"))
            .query(&[("onlyData", "true"), ("expand", "requisitionList.secondaryLocations"), ("finder", finder.as_str())])
            .header(ACCEPT, "application/json")
            .send()
            .await?
            .error_for_status()
            .map_err(|e| format!("Oracle HCM list request failed for {base} site {site}: {e}"))?
            .json()
            .await?;

        let Some(item) = page.items.into_iter().next() else { break };
        if total.is_none() {
            total = item.total_jobs_count;
        }
        let count = item.requisition_list.len() as u32;
        if std::env::var_os("SCRAPER_DEBUG").is_some() {
            eprintln!("oracle {base}/{site}: offset={offset} got={count} total={total:?}");
        }
        for job in item.requisition_list {
            if !seen.contains(&job_url(base, site, &job.id)) {
                new_ids.push(job.id);
            }
        }
        offset += count;
        if count < PAGE_SIZE || offset >= total.unwrap_or(MAX_POSTINGS) || offset >= MAX_POSTINGS {
            break;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    let results: Vec<Result<ScrapeEvent, Error>> = stream::iter(new_ids)
        .map(|id| {
            let (client, api, base, site) = (client.clone(), api.clone(), base.clone(), site.clone());
            async move { fetch_detail(company_id, &client, &api, &base, &site, &id).await }
        })
        .buffer_unordered(DETAIL_CONCURRENCY)
        .collect()
        .await;

    let mut events = Vec::new();
    for result in results {
        match result {
            Ok(event) => events.push(event),
            Err(e) => eprintln!("Oracle HCM detail failed ({base}/{site}): {e}"),
        }
    }
    Ok(events)
}

async fn fetch_detail(company_id: i64, client: &Client, api: &str, base: &str, site: &str, id: &str) -> Result<ScrapeEvent, Error> {
    let finder = format!("ById;Id=\"{id}\",siteNumber={site}");
    let detail: DetailResponse = client
        .get(format!("{api}/recruitingCEJobRequisitionDetails"))
        .query(&[("expand", "all"), ("onlyData", "true"), ("finder", finder.as_str())])
        .header(ACCEPT, "application/json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let info = detail.items.into_iter().next().ok_or_else(|| format!("requisition {id} has no detail"))?;

    let mut cities: Vec<City> = Vec::new();
    let mut add = |raw: &str, country: Option<&str>| {
        let mut city = parse_location(raw);
        if city.country.is_empty() {
            if let Some(code) = country {
                city.country = code.to_uppercase();
            }
        }
        if !cities.contains(&city) {
            cities.push(city);
        }
    };
    if let Some(primary) = info.primary_location.as_deref().filter(|s| !s.trim().is_empty()) {
        add(primary, info.primary_location_country.as_deref());
    }
    for extra in &info.secondary_locations {
        if let Some(name) = extra.name.as_deref().filter(|s| !s.trim().is_empty()) {
            add(name, extra.country_code.as_deref());
        }
    }

    let html = [info.external_description_str, info.external_responsibilities_str, info.external_qualifications_str]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("\n");
    let title = info.title.trim().to_string();
    let terms = parse_terms(&title);

    Ok(ScrapeEvent {
        terms: if terms.is_empty() { None } else { Some(terms) },
        cities,
        application: Application {
            company_id,
            job_title: title,
            url: job_url(base, site, id),
            page_content: html_to_text(&html),
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
    fn parses_list_and_detail_shapes() {
        let list: ListResponse = serde_json::from_str(
            r#"{"items":[{"TotalJobsCount":361,"requisitionList":[{"Id":"2506918","Title":"Spring 2027 - Intern","PrimaryLocation":"Gresham, OR, United States","secondaryLocations":[]}]}]}"#,
        )
        .unwrap();
        assert_eq!(list.items[0].total_jobs_count, Some(361));
        assert_eq!(list.items[0].requisition_list[0].id, "2506918");

        let detail: DetailResponse = serde_json::from_str(
            r#"{"items":[{"Title":"Intern","PrimaryLocation":"Gresham, OR, United States","PrimaryLocationCountry":"US","ExternalDescriptionStr":"<p>Hi</p>","ExternalQualificationsStr":null,"secondaryLocations":[{"Name":"Phoenix, AZ, United States","CountryCode":"US"}]}]}"#,
        )
        .unwrap();
        let d = &detail.items[0];
        assert_eq!(d.primary_location.as_deref(), Some("Gresham, OR, United States"));
        assert_eq!(d.secondary_locations[0].name.as_deref(), Some("Phoenix, AZ, United States"));
        assert_eq!(
            job_url("https://hctz.fa.us2.oraclecloud.com", "CX_1001", "2506918"),
            "https://hctz.fa.us2.oraclecloud.com/hcmUI/CandidateExperience/en/sites/CX_1001/job/2506918"
        );
    }
}
