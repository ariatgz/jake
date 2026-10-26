//! Scrapers. Hand-written ones (Google, Amazon, ...) live next to the generic
//! board scrapers (Workday, Jibe, Greenhouse, Lever, Ashby, Oracle, SmartRecruiters,
//! Workable). `scrape_company` picks the right one from `companies.board_type`.

pub mod amazon;
pub mod doordash;
pub mod google;
pub mod jane_street;
pub mod stripe;

pub mod ashby;
pub mod detect;
pub mod greenhouse;
pub mod jibe;
pub mod lever;
pub mod location;
pub mod oracle;
pub mod smartrecruiters;
pub mod text;
pub mod workable;
pub mod workday;

use crate::{Company, Error, ScrapeEvent};
use detect::Board;
use reqwest::Client;
use std::collections::HashSet;
use tokio::sync::mpsc;

/// How many job-detail requests a board scraper has in flight at once.
pub const DETAIL_CONCURRENCY: usize = 4;

/// Returns only jobs whose URL is not in `seen`.
pub async fn scrape_company(company: &Company, seen: HashSet<String>, client: Client) -> Result<Vec<ScrapeEvent>, Error> {
    if company.board_type == "CUSTOM" {
        return legacy(company, seen, client).await;
    }

    let board_url = company
        .board_url
        .as_deref()
        .ok_or_else(|| format!("{} has no board URL", company.display_name))?;
    let board = detect::from_board_url(board_url)
        .ok_or_else(|| format!("{} has an unrecognised board URL: {board_url}", company.display_name))?;

    match &board {
        Board::Workday { .. } => workday::scrape(company.id, &board, &seen, &client).await,
        Board::Jibe { .. } => jibe::scrape(company.id, &board, &seen, &client).await,
        Board::Greenhouse { .. } => greenhouse::scrape(company.id, &board, &seen, &client).await,
        Board::Lever { .. } => lever::scrape(company.id, &board, &seen, &client).await,
        Board::Ashby { .. } => ashby::scrape(company.id, &board, &seen, &client).await,
        Board::Oracle { .. } => oracle::scrape(company.id, &board, &seen, &client).await,
        Board::SmartRecruiters { .. } => smartrecruiters::scrape(company.id, &board, &seen, &client).await,
        Board::Workable { .. } => workable::scrape(company.id, &board, &seen, &client).await,
    }
}

/// Runs one of the original channel-based scrapers on its own task so a panic
/// inside it (they `unwrap` liberally) only fails that company.
async fn legacy(company: &Company, seen: HashSet<String>, client: Client) -> Result<Vec<ScrapeEvent>, Error> {
    let (tx, mut rx) = mpsc::channel::<ScrapeEvent>(32);
    let id = company.id;
    let name = company.display_name.clone();

    let handle = tokio::spawn(async move {
        match name.as_str() {
            "Google" => google::main(id, seen, tx).await,
            "Amazon" => amazon::main(id, seen, tx, client).await,
            "Jane Street" => jane_street::main(id, seen, tx, client).await,
            "DoorDash" => doordash::main(id, seen, tx, client).await,
            "Stripe" => stripe::main(id, seen, tx, client).await,
            other => Err(format!("No hand-written scraper for `{other}`").into()),
        }
    });

    let mut events = Vec::new();
    while let Some(event) = rx.recv().await {
        events.push(event);
    }

    match handle.await {
        Ok(Ok(())) => Ok(events),
        Ok(Err(e)) => Err(e),
        Err(join_error) => Err(format!("scraper crashed: {join_error}").into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn company(name: &str, board_type: &str, board_url: &str) -> Company {
        Company {
            id: 0,
            display_name: name.into(),
            url: String::new(),
            hex_code: "#000000".into(),
            icon_url: String::new(),
            board_type: board_type.into(),
            board_url: Some(board_url.into()),
        }
    }

    /// Hits the real boards the seed migrations point at. Run with
    /// `cargo test live_seeded_boards -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "network"]
    async fn live_seeded_boards() {
        // Same headers as `http_client` in main.rs (not reachable from the tests/ harness).
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::ACCEPT_LANGUAGE, reqwest::header::HeaderValue::from_static("en-US,en;q=0.9"));
        let client = Client::builder()
            .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/150.0.0.0 Safari/537.36")
            .default_headers(headers)
            .build()
            .unwrap();
        let boards = [
            company("Marvell", "WORKDAY", "https://marvell.wd1.myworkdayjobs.com/MarvellCareers?workerSubType=65dea26481d001e09dfbab4927173419"),
            company("Astera Labs", "GREENHOUSE", "https://job-boards.greenhouse.io/asteraearlycareer2027"),
            company("onsemi", "ORACLE", "https://hctz.fa.us2.oraclecloud.com/hcmUI/CandidateExperience/en/sites/CX_1001/requisitions?keyword=Intern"),
        ];
        for c in boards {
            let events = scrape_company(&c, HashSet::new(), client.clone()).await.unwrap_or_else(|e| panic!("{}: {e}", c.display_name));
            assert!(!events.is_empty(), "{} returned no jobs", c.display_name);
            let with_location = events.iter().filter(|e| !e.cities.is_empty()).count();
            let with_text = events.iter().filter(|e| e.application.page_content.len() > 200).count();
            println!("{}: {} jobs, {with_location} with a location, {with_text} with a description", c.display_name, events.len());
            println!("  e.g. {} | {:?} | {}", events[0].application.job_title, events[0].cities.first().map(|c| &c.display_name), events[0].application.url);
            assert!(with_text * 10 >= events.len() * 8, "{}: most postings should have text", c.display_name);
        }
    }
}
