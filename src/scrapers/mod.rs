//! Scrapers. Hand-written ones (Google, Amazon, ...) live next to the generic
//! board scrapers (Workday, Jibe, Greenhouse, Lever, Ashby). `scrape_company`
//! picks the right one from `companies.board_type`.

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
pub mod text;
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
