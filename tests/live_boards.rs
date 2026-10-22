//! Hits the real job boards for every seeded company and prints what the
//! scrapers get back. Network-dependent, so it is ignored by default:
//!
//!     cargo test --test live_boards -- --ignored --nocapture
//!
//! This exercises the same code the bot runs (`scrape_company`), with an empty
//! "seen" set so every posting is treated as new.

use std::collections::HashSet;

#[path = "../src/scrapers/mod.rs"]
#[allow(dead_code)]
mod scrapers;

// The scraper modules reach shared types through `crate::`; provide them here.
pub use types::*;
mod types {
    #[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
    pub struct City {
        pub display_name: String,
        pub region: String,
        pub country: String,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
    pub struct Term {
        pub display_name: String,
    }

    #[derive(Debug, Clone)]
    pub struct Application {
        pub company_id: i64,
        pub job_title: String,
        pub url: String,
        pub page_content: String,
        pub lower_wage_cents: Option<i16>,
        pub upper_wage_cents: Option<i16>,
        pub currency: String,
    }

    #[derive(Debug, Clone)]
    pub struct ScrapeEvent {
        pub terms: Option<Vec<Term>>,
        pub cities: Vec<City>,
        pub application: Application,
    }

    #[derive(Debug, Clone)]
    pub struct Company {
        pub id: i64,
        pub display_name: String,
        pub url: String,
        pub hex_code: String,
        pub icon_url: String,
        pub board_type: String,
        pub board_url: Option<String>,
    }

    pub type Error = Box<dyn std::error::Error + Send + Sync>;
}

const SEEDS: &[(&str, &str, &str)] = &[
    ("AMD", "JIBE", "https://careers.amd.com/careers-home/jobs?country=Canada&page=1"),
    ("Altera", "WORKDAY", "https://altera.wd1.myworkdayjobs.com/Altera?jobFamilyGroup=3a3ac0bdef3a100fb57f7caeff190000"),
    ("Intel", "WORKDAY", "https://intel.wd1.myworkdayjobs.com/External?jobFamilyGroup=dc8bf79476611087d67b36517cf17036"),
    ("NVIDIA", "WORKDAY", "https://nvidia.wd5.myworkdayjobs.com/NVIDIAExternalCareerSite?jobFamilyGroup=0c40f6bd1d8f10ae43ffda1e8d447e94"),
    ("onsemi", "WORKDAY", "https://onsemi.wd1.myworkdayjobs.com/onsemi"),
    ("Astera Labs", "GREENHOUSE", "https://job-boards.greenhouse.io/asteralabs"),
    ("Etched", "ASHBY", "https://jobs.ashbyhq.com/etched"),
    // Added in migration 20261022030000_more_hardware_companies.
    ("Marvell", "WORKDAY", "https://marvell.wd1.myworkdayjobs.com/MarvellCareers?workerSubType=65dea26481d001e09dfbab4927173419"),
    ("Cadence", "WORKDAY", "https://cadence.wd1.myworkdayjobs.com/External_Careers?workerSubType=631f97c41c2749729ac5abe683f281d0"),
    ("Ciena", "WORKDAY", "https://ciena.wd5.myworkdayjobs.com/Careers?workerSubType=ac99acc6fb6b108fe66503995497ae7c"),
    ("Analog Devices", "WORKDAY", "https://analogdevices.wd1.myworkdayjobs.com/External?workerSubType=633b03df4f5d1000ec10ab4627a80000"),
    ("Microchip", "WORKDAY", "https://microchiphr.wd5.myworkdayjobs.com/External?zz_LRV_Career_Level_Requisition_Extended=57f4a35d19da0166ec5d6a6365019c99"),
    ("General Motors", "WORKDAY", "https://generalmotors.wd5.myworkdayjobs.com/Careers_GM?workerSubType=81219c91208501e94c213185a61a320a&workerSubType=81219c91208501ec00823185a61a360a"),
    ("BlackBerry", "WORKDAY", "https://bb.wd3.myworkdayjobs.com/BlackBerry"),
    ("Tenstorrent", "GREENHOUSE", "https://job-boards.greenhouse.io/tenstorrent"),
    ("Lightmatter", "GREENHOUSE", "https://job-boards.greenhouse.io/lightmatter"),
    ("Cerebras", "ASHBY", "https://jobs.ashbyhq.com/cerebras"),
    ("Kepler Communications", "LEVER", "https://jobs.lever.co/kepler"),
];

/// Seed URLs must be in the canonical form the bot stores, or `/add-company`
/// wouldn't recognise them as already known.
#[test]
fn seed_urls_are_canonical() {
    for (name, kind, url) in SEEDS {
        let board = scrapers::detect::from_board_url(url).unwrap_or_else(|| panic!("{name}: unrecognised URL"));
        assert_eq!(board.kind(), *kind, "{name}");
        if *kind != "JIBE" {
            assert_eq!(board.canonical_url(), *url, "{name}");
        }
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/150.0.0.0 Safari/537.36")
        .timeout(std::time::Duration::from_secs(40))
        .build()
        .unwrap()
}

#[tokio::test]
#[ignore = "talks to live job boards"]
async fn live_boards() {
    // BOARD=name or BOARD=name1,name2 runs only those.
    let only: Option<Vec<String>> = std::env::var("BOARD").ok().map(|v| v.split(',').map(|s| s.trim().to_lowercase()).collect());
    let client = client();
    let mut failures = Vec::new();

    for (i, (name, kind, url)) in SEEDS.iter().enumerate() {
        if let Some(filter) = &only {
            if !filter.contains(&name.to_lowercase()) {
                continue;
            }
        }
        let company = Company {
            id: i as i64 + 1,
            display_name: name.to_string(),
            url: String::new(),
            hex_code: String::new(),
            icon_url: String::new(),
            board_type: kind.to_string(),
            board_url: Some(url.to_string()),
        };

        println!("\n=== {name} ({kind}) ===");
        match scrapers::scrape_company(&company, HashSet::new(), client.clone()).await {
            Ok(events) => {
                println!("{} postings", events.len());
                let no_location = events.iter().filter(|e| e.cities.is_empty()).count();
                let with_pay = events.iter().filter(|e| e.application.lower_wage_cents.is_some()).count();
                let empty_body = events.iter().filter(|e| e.application.page_content.len() < 50).count();
                println!("no location: {no_location}, with pay: {with_pay}, short/empty description: {empty_body}");
                for event in events.iter().take(6) {
                    let cities: Vec<String> = event.cities.iter().map(scrapers::location::format_city).collect();
                    println!(
                        "  - {} | {} | {:?} | pay {:?}-{:?} {} | {} chars",
                        event.application.job_title,
                        cities.join(" / "),
                        event.terms.as_ref().map(|t| t.iter().map(|t| t.display_name.clone()).collect::<Vec<_>>()).unwrap_or_default(),
                        event.application.lower_wage_cents,
                        event.application.upper_wage_cents,
                        event.application.currency,
                        event.application.page_content.len()
                    );
                }
                if events.is_empty() {
                    failures.push(format!("{name}: zero postings"));
                }
            }
            Err(e) => {
                println!("ERROR: {e}");
                failures.push(format!("{name}: {e}"));
            }
        }
    }

    // onsemi's Workday site was returning HTTP 500/422 when this was written; don't fail the run on it alone.
    failures.retain(|f| !f.starts_with("onsemi"));
    assert!(failures.is_empty(), "board failures: {failures:#?}");
}
