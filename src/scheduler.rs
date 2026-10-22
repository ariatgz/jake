//! Background loop that scrapes for every profile on a fixed interval and
//! only speaks up when a new matching job appears.

use crate::resume::latex::Document;
use crate::{db, outreach, pipeline};
use poise::serenity_prelude::{ChannelId, Http};
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

/// How often due follow-up reminders are checked.
const FOLLOW_UP_CHECK: Duration = Duration::from_secs(10 * 60);

/// Posts "time to follow up" reminders into job threads for outreach logged
/// with `/outreach sent` that hasn't been marked as replied.
pub fn spawn_follow_ups(http: Arc<Http>, pool: PgPool) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(FOLLOW_UP_CHECK);
        loop {
            interval.tick().await;
            let due = match db::due_follow_ups(&pool).await {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("Follow-ups: failed to load: {e}");
                    continue;
                }
            };
            for item in due {
                // Signed with the resume's name when there is one.
                let sender = match db::resume(&pool, item.user_id).await {
                    Ok(Some(r)) => Document::parse(&r.tex).ok().and_then(|d| d.person_name()),
                    _ => None,
                }
                .unwrap_or_else(|| item.sender.clone());
                let draft = outreach::follow_up(&item.contact_name, &item.job_title, &item.company_name, &sender);
                let last = item.follow_ups_sent >= 1;
                let content = format!(
                    "<@{}> Time to follow up with **{}** about the {} role at {}. It's been {} day{} and no reply is marked yet. \
                     A short nudge often gets a response:\n```\n{}\n```\n-# Heard back? `/outreach replied`. {}",
                    item.user_id,
                    item.contact_name,
                    item.job_title,
                    item.company_name,
                    item.days_since,
                    if item.days_since == 1 { "" } else { "s" },
                    draft.replace("```", "'''"),
                    if last { "This is the last reminder for them." } else { "I'll check once more in a week." },
                );
                if let Err(e) = ChannelId::new(item.thread_id as u64).say(&http, content).await {
                    eprintln!("Follow-ups: posting to thread {} failed: {e}", item.thread_id);
                }
                // Recorded even if posting failed (e.g. the thread was deleted) so it isn't retried forever.
                if let Err(e) = db::record_follow_up_reminder(&pool, item.id).await {
                    eprintln!("Follow-ups: saving reminder {} failed: {e}", item.id);
                }
            }
        }
    });
}
