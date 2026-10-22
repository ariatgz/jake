//! The scrape -> store -> match -> notify flow shared by `/scrape` and the scheduler.

use crate::scrapers::location::format_city;
use crate::{db, filter, scrapers, Company, Error, Profile};
use db::Candidate;
use futures::future::join_all;
use poise::serenity_prelude as serenity;
use reqwest::Client;
use serenity::{ChannelId, Http};
use sqlx::PgPool;
use std::collections::HashMap;

/// Upper bound on postings sent to one user in one run, to stay clear of
/// Discord rate limits when someone adds a company with a big backlog.
pub const MAX_POSTS_PER_USER_PER_RUN: usize = 25;

#[derive(Debug, Default)]
pub struct RunSummary {
    pub companies_scraped: usize,
    pub new_jobs: usize,
    pub posted: usize,
    pub errors: Vec<String>,
}

/// Scrapes every company tracked by `profiles`, stores new jobs, then posts the
/// ones matching each profile. `channel_override` sends everything to one
/// channel (used by `/scrape`) instead of each profile's own channel.
pub async fn run(http: &Http, pool: &PgPool, client: &Client, profiles: &[Profile], channel_override: Option<ChannelId>) -> RunSummary {
    let mut summary = RunSummary::default();
    if profiles.is_empty() {
        return summary;
    }

    let user_ids: Vec<i64> = profiles.iter().map(|p| p.user_id).collect();
    let companies = match db::companies_for_profiles(pool, &user_ids).await {
        Ok(c) => c,
        Err(e) => {
            summary.errors.push(format!("loading companies: {e}"));
            return summary;
        }
    };
    summary.companies_scraped = companies.len();

    // 1. Scrape every company concurrently.
    let scrapes = companies.iter().map(|company| async move {
        let seen = db::seen_urls(pool, company.id).await?;
        scrapers::scrape_company(company, seen, client.clone()).await
    });
    let results = join_all(scrapes).await;

    // 2. Store new jobs.
    for (company, result) in companies.iter().zip(results) {
        match result {
            Ok(events) => {
                for event in events {
                    match db::insert_application(pool, &event).await {
                        Ok(Some(_)) => summary.new_jobs += 1,
                        Ok(None) => {}
                        Err(e) => summary.errors.push(format!("{}: storing `{}`: {e}", company.display_name, event.application.job_title)),
                    }
                }
            }
            Err(e) => summary.errors.push(format!("{}: {e}", company.display_name)),
        }
    }

    // 3. Match and notify per profile.
    let company_map: HashMap<i64, &Company> = companies.iter().map(|c| (c.id, c)).collect();

    for profile in profiles {
        let tracked = match db::tracked_companies(pool, profile.user_id).await {
            Ok(t) => t,
            Err(e) => {
                summary.errors.push(format!("{}: loading tracked companies: {e}", profile.display_name));
                continue;
            }
        };
        let tracked_ids: Vec<i64> = tracked.iter().map(|c| c.id).collect();
        if tracked_ids.is_empty() {
            continue;
        }

        let candidates = match db::candidates_for_profile(pool, profile.user_id, &tracked_ids).await {
            Ok(c) => c,
            Err(e) => {
                summary.errors.push(format!("{}: loading candidates: {e}", profile.display_name));
                continue;
            }
        };

        let channel = channel_override.unwrap_or(ChannelId::new(profile.channel_id as u64));
        let mut posted_for_user = 0;

        for candidate in candidates {
            if posted_for_user >= MAX_POSTS_PER_USER_PER_RUN {
                break;
            }
            let skip_seniority = candidate.board_type == "CUSTOM";
            if !filter::matches(profile, &candidate.job_title, &candidate.page_content, &candidate.cities, skip_seniority) {
                continue;
            }
            let Some(company) = company_map.get(&candidate.company_id) else { continue };

            match post(http, channel, profile, company, &candidate).await {
                Ok(message_id) => {
                    if let Err(e) = db::record_notification(pool, candidate.application_id, profile.user_id, channel.get() as i64, message_id as i64).await {
                        summary.errors.push(format!("{}: recording notification: {e}", profile.display_name));
                    }
                    posted_for_user += 1;
                    summary.posted += 1;
                }
                Err(e) => {
                    summary.errors.push(format!("{}: posting `{}`: {e}", profile.display_name, candidate.job_title));
                    break; // most likely a permissions problem; don't spam retries
                }
            }
        }
    }

    summary
}

pub fn build_embed(company: &Company, candidate: &Candidate) -> serenity::CreateEmbed {
    let term_text = if candidate.terms.is_empty() {
        String::new()
    } else {
        format!("{} - ", candidate.terms.iter().map(|t| t.display_name.as_str()).collect::<Vec<_>>().join(", "))
    };
    let title: String = format!("{term_text}{}", candidate.job_title).chars().take(256).collect();

    let location_text = match candidate.cities.first() {
        None => "Not listed".to_string(),
        Some(first) => {
            let mut text = format_city(first);
            if candidate.cities.len() > 1 {
                text.push_str(&format!(" (+{} more)", candidate.cities.len() - 1));
            }
            text
        }
    };

    let compensation = match (candidate.lower_wage_cents, candidate.upper_wage_cents) {
        (Some(lower), Some(upper)) => format!("${}-${}/hr ({})", lower / 100, upper / 100, candidate.currency),
        (Some(lower), None) => format!("${}/hr ({})", lower / 100, candidate.currency),
        (None, _) => "Not listed".to_string(),
    };

    let snippet: String = candidate.page_content.chars().take(300).collect();
    let description = if candidate.page_content.chars().count() > 300 { format!("{snippet}...") } else { snippet };

    let colour = u32::from_str_radix(&company.hex_code.replace('#', ""), 16).unwrap_or(0x5865F2);

    serenity::CreateEmbed::new()
        .title(title)
        .url(&candidate.url)
        .author(
            serenity::CreateEmbedAuthor::new(&company.display_name)
                .url(&company.url)
                .icon_url(&company.icon_url),
        )
        .colour(serenity::Colour::from(colour))
        .field("Compensation", compensation, true)
        .field("Location(s)", location_text, true)
        .description(description)
}

async fn post(http: &Http, channel: ChannelId, profile: &Profile, company: &Company, candidate: &Candidate) -> Result<u64, Error> {
    let message = serenity::CreateMessage::new()
        .content(format!("<@{}>", profile.user_id))
        .embed(build_embed(company, candidate))
        .components(vec![serenity::CreateActionRow::Buttons(vec![
            serenity::CreateButton::new("ACTIVE").label("Apply").style(serenity::ButtonStyle::Secondary),
            serenity::CreateButton::new("IGNORED").label("Ignore").style(serenity::ButtonStyle::Secondary),
        ])]);

    let sent = channel.send_message(http, message).await?;
    Ok(sent.id.get())
}

impl RunSummary {
    /// Short human-readable report for the `/scrape` reply.
    pub fn describe(&self) -> String {
        let mut text = format!(
            "Checked {} compan{}, found {} new job{}, posted {} match{} for you.",
            self.companies_scraped,
            if self.companies_scraped == 1 { "y" } else { "ies" },
            self.new_jobs,
            if self.new_jobs == 1 { "" } else { "s" },
            self.posted,
            if self.posted == 1 { "" } else { "es" },
        );
        if !self.errors.is_empty() {
            text.push_str("\n\nProblems:");
            for error in self.errors.iter().take(5) {
                let line: String = error.chars().take(300).collect();
                text.push_str(&format!("\n- {line}"));
            }
            if self.errors.len() > 5 {
                text.push_str(&format!("\n- ...and {} more", self.errors.len() - 5));
            }
        }
        text
    }
}
