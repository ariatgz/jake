//! Background loop that scrapes for every profile on a fixed interval and
//! only speaks up when a new matching job appears.

use crate::{db, pipeline};
use poise::serenity_prelude::Http;
use reqwest::Client;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

pub fn spawn(http: Arc<Http>, pool: PgPool, client: Client, lock: Arc<Mutex<()>>, every: Duration) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(every);
        interval.tick().await; // the first tick fires immediately; wait a full period instead
        println!("Scheduler running every {} minutes", every.as_secs() / 60);

        loop {
            interval.tick().await;
            let _guard = lock.lock().await;

            let profiles = match db::all_profiles(&pool).await {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("Scheduler: failed to load profiles: {e}");
                    continue;
                }
            };
            if profiles.is_empty() {
                continue;
            }

            let summary = pipeline::run(&http, &pool, &client, &profiles, None).await;
            println!(
                "Scheduled scrape: {} companies, {} new jobs, {} posted, {} errors",
                summary.companies_scraped, summary.new_jobs, summary.posted, summary.errors.len()
            );
            for error in &summary.errors {
                eprintln!("  - {error}");
            }
        }
    });
}
