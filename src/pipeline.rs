//! The scrape -> store -> match -> notify flow shared by `/scrape` and the scheduler.

use crate::resume::{self, ResumeKnowledge};
use crate::scrapers::location::format_city;
use crate::{contacts, db, filter, flags, scrapers, Company, Error, Profile};
use db::Candidate;
use futures::{stream, StreamExt};
use poise::serenity_prelude as serenity;
use reqwest::Client;
use serenity::{ChannelId, Http};
use sqlx::PgPool;
use std::collections::HashMap;

/// Jobs posted to one user at a time. Anything beyond waits behind a
/// "Show next" button so a big backlog doesn't flood the channel.
pub const BATCH_SIZE: usize = 10;

/// Custom id prefix of the "Show next" button; followed by the user id.
pub const MORE_PREFIX: &str = "MORE:";

/// How many companies are scraped at the same time.
const COMPANY_CONCURRENCY: usize = 4;

#[derive(Debug, Default)]
pub struct RunSummary {
    pub companies_scraped: usize,
    pub new_jobs: usize,
    pub posted: usize,
    /// Matches found but held back for the next batch.
    pub remaining: usize,
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

    // 1. Scrape companies a few at a time; keeps CPU, memory and outbound
    // connections flat on a small board like a Raspberry Pi.
    let scrapes: Vec<_> = companies
        .iter()
        .map(|company| async move {
            let seen = db::seen_urls(pool, company.id).await?;
            scrapers::scrape_company(company, seen, client.clone()).await
        })
        .collect();
    let results: Vec<Result<Vec<_>, Error>> = stream::iter(scrapes).buffered(COMPANY_CONCURRENCY).collect().await;

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
    for profile in profiles {
        let channel = channel_override.unwrap_or(ChannelId::new(profile.channel_id as u64));
        notify_profile(http, pool, profile, channel, &mut summary).await;
    }

    summary
}

/// Posts the next batch of matches for one profile, then a "Show next" button
/// if more are waiting. Used after a scrape and by that button.
pub async fn notify_profile(http: &Http, pool: &PgPool, profile: &Profile, channel: ChannelId, summary: &mut RunSummary) {
    let who = &profile.display_name;
    let tracked = match db::tracked_companies(pool, profile.user_id).await {
        Ok(t) => t,
        Err(e) => {
            summary.errors.push(format!("{who}: loading tracked companies: {e}"));
            return;
        }
    };
    if tracked.is_empty() {
        return;
    }
    let company_map: HashMap<i64, &Company> = tracked.iter().map(|c| (c.id, c)).collect();
    let tracked_ids: Vec<i64> = company_map.keys().copied().collect();

    // Only jobs stored since this profile was last evaluated; the watermark is
    // reset whenever filters or tracked companies change.
    let candidates = match db::candidates_for_profile(pool, profile.user_id, &tracked_ids, profile.evaluated_through).await {
        Ok(c) => c,
        Err(e) => {
            summary.errors.push(format!("{who}: loading candidates: {e}"));
            return;
        }
    };
    let newest = candidates.iter().map(|c| c.application_id).max();

    let matcher = filter::Matcher::new(profile);
    let matches: Vec<Candidate> = candidates
        .into_iter()
        .filter(|c| company_map.contains_key(&c.company_id))
        .filter(|c| matcher.matches(&c.job_title, &c.page_content, &c.cities, c.board_type == "CUSTOM"))
        .collect();

    // Loaded once per batch so each alert can show how well the resume fits.
    let knowledge = if matches.is_empty() {
        None
    } else {
        match db::resume(pool, profile.user_id).await {
            Ok(Some(r)) => ResumeKnowledge::from_tex(&r.tex).ok(),
            _ => None,
        }
    };

    let mut posted = 0;
    let mut failed = false;
    for candidate in matches.iter().take(BATCH_SIZE) {
        let company = company_map[&candidate.company_id];
        match post(http, channel, profile, company, candidate, knowledge.as_ref()).await {
            Ok(message_id) => {
                if let Err(e) = db::record_notification(pool, candidate.application_id, profile.user_id, channel.get() as i64, message_id as i64).await {
                    summary.errors.push(format!("{who}: recording notification: {e}"));
                }
                posted += 1;
            }
            Err(e) => {
                summary.errors.push(format!("{who}: posting `{}`: {e}", candidate.job_title));
                failed = true;
                break; // most likely a permissions problem; don't spam retries
            }
        }
    }
    let remaining = matches.len() - posted;
    summary.posted += posted;
    summary.remaining += remaining;

    if remaining == 0 && !failed {
        if let Some(newest) = newest {
            if let Err(e) = db::advance_evaluated_through(pool, profile.user_id, profile.filters_version, newest).await {
                summary.errors.push(format!("{who}: saving progress: {e}"));
            }
        }
    } else if !failed {
        // Leftovers stay unrecorded, so the button (or the next run) picks them up.
        let next = remaining.min(BATCH_SIZE);
        let message = serenity::CreateMessage::new()
            .content(format!(
                "<@{}> {remaining} more match{} waiting.",
                profile.user_id,
                if remaining == 1 { "" } else { "es" }
            ))
            .components(vec![serenity::CreateActionRow::Buttons(vec![serenity::CreateButton::new(format!("{MORE_PREFIX}{}", profile.user_id))
                .label(format!("Show next {next}"))
                .style(serenity::ButtonStyle::Primary)])]);
        if let Err(e) = channel.send_message(http, message).await {
            summary.errors.push(format!("{who}: posting the 'more' button: {e}"));
        }
    }
}

fn board_label(board_type: &str) -> &str {
    match board_type {
        "WORKDAY" => "Workday",
        "JIBE" => "iCIMS/Jibe",
        "GREENHOUSE" => "Greenhouse",
        "LEVER" => "Lever",
        "ASHBY" => "Ashby",
        _ => "Careers site",
    }
}

/// Phrases that usually start the part of a posting worth previewing.
const SNIPPET_STARTS: &[&str] = &[
    "what you'll do", "what you will do", "what you’ll do", "responsibilities", "about the role", "the role", "your role",
    "in this role", "job description", "position overview", "overview",
];

/// A short preview, starting at the description of the work when the posting
/// has one rather than at the company boilerplate.
pub fn snippet(text: &str, max: usize) -> String {
    let lower = text.to_lowercase();
    let start = SNIPPET_STARTS
        .iter()
        .filter_map(|m| lower.find(m).map(|i| i + m.len()))
        .min()
        // Lowercasing can shift byte offsets for some characters; fall back to the start.
        .filter(|&i| i < text.len() && text.is_char_boundary(i))
        .unwrap_or(0);
    let body = text[start..].trim_start_matches(|c: char| c == ':' || c == '-' || c.is_whitespace());
    let body = if body.chars().count() < 40 { text } else { body };
    if body.chars().count() <= max {
        return body.to_string();
    }
    let cut: String = body.chars().take(max).collect();
    let at = cut.rfind(' ').unwrap_or(cut.len());
    format!("{}…", cut[..at].trim_end_matches([',', ';', ':']))
}

fn names(skills: &[resume::skills::PostingSkill], limit: usize) -> String {
    let mut out: Vec<&str> = skills.iter().take(limit).map(|s| s.name.as_str()).collect();
    let more = skills.len().saturating_sub(limit);
    let extra = if more > 0 { format!(" and {more} more") } else { String::new() };
    if out.is_empty() {
        out.push("none");
    }
    format!("{}{extra}", out.join(", "))
}

pub fn build_embed(company: &Company, candidate: &Candidate, knowledge: Option<&ResumeKnowledge>) -> serenity::CreateEmbed {
    let location = match candidate.cities.first() {
        None => "Not listed".to_string(),
        Some(first) if candidate.cities.len() > 1 => format!("{} (+{} more)", format_city(first), candidate.cities.len() - 1),
        Some(first) => format_city(first),
    };
    let pay = match (candidate.lower_wage_cents, candidate.upper_wage_cents) {
        (Some(lower), Some(upper)) => format!("${}–{}/hr {}", lower / 100, upper / 100, candidate.currency),
        (Some(lower), None) => format!("${}/hr {}", lower / 100, candidate.currency),
        (None, _) => "Not listed".to_string(),
    };

    let title: String = candidate.job_title.chars().take(256).collect();
    let mut colour = u32::from_str_radix(&company.hex_code.replace('#', ""), 16).unwrap_or(0x5865F2);
    let mut embed = serenity::CreateEmbed::new()
        .title(title)
        .url(&candidate.url)
        .author(serenity::CreateEmbedAuthor::new(&company.display_name).url(&company.url).icon_url(&company.icon_url))
        .field("Location", location, true)
        .field("Pay", pay, true);
    if !candidate.terms.is_empty() {
        embed = embed.field("Term", candidate.terms.iter().map(|t| t.display_name.as_str()).collect::<Vec<_>>().join(", "), true);
    }

    let preview = snippet(&candidate.page_content, 260);
    if !preview.is_empty() {
        embed = embed.description(preview.replace('\n', " "));
    }

    if let Some(knowledge) = knowledge {
        let m = resume::match_job(knowledge, &candidate.job_title, &candidate.page_content);
        if let Some(score) = m.score {
            // The side bar shows fit at a glance: green, yellow, red.
            colour = match score {
                75.. => 0x3BA55C,
                50..=74 => 0xFAA61A,
                _ => 0xED4245,
            };
            let mut text = format!("**{score}%**\nHas: {}", names(&m.matched, 6));
            if !m.missing.is_empty() {
                text.push_str(&format!("\nMissing: {}", names(&m.missing, 4)));
            }
            embed = embed.field("Resume match", text, false);
        }
    }

    let flags = flags::detect(&candidate.page_content);
    if !flags.is_empty() {
        embed = embed.field("Check before applying", flags.join("\n"), false);
    }

    let mut footer = board_label(&candidate.board_type).to_string();
    if let Some(req) = contacts::from_posting(&candidate.page_content, &candidate.url, &company.display_name).req_id {
        footer.push_str(&format!(" · Job ID {req}"));
    }

    embed
        .colour(serenity::Colour::from(colour))
        .footer(serenity::CreateEmbedFooter::new(footer))
        .timestamp(serenity::Timestamp::now())
}

/// Discord rejects a whole message if a link button's URL is invalid or over 512 characters.
pub fn link_button_ok(url: &str) -> bool {
    url.len() <= 512 && (url.starts_with("https://") || url.starts_with("http://"))
}

/// Buttons on a job alert. The posting link is kept after Apply/Ignore is pressed.
pub fn job_buttons(url: &str) -> Vec<serenity::CreateActionRow> {
    let mut buttons = vec![
        serenity::CreateButton::new("ACTIVE").label("Apply").style(serenity::ButtonStyle::Success),
        serenity::CreateButton::new("IGNORED").label("Ignore").style(serenity::ButtonStyle::Secondary),
    ];
    if link_button_ok(url) {
        buttons.push(serenity::CreateButton::new_link(url).label("Open posting"));
    }
    vec![serenity::CreateActionRow::Buttons(buttons)]
}

async fn post(
    http: &Http,
    channel: ChannelId,
    profile: &Profile,
    company: &Company,
    candidate: &Candidate,
    knowledge: Option<&ResumeKnowledge>,
) -> Result<u64, Error> {
    let message = serenity::CreateMessage::new()
        .content(format!("<@{}>", profile.user_id))
        .embed(build_embed(company, candidate, knowledge))
        .components(job_buttons(&candidate.url));
    let sent = channel.send_message(http, message).await?;
    Ok(sent.id.get())
}

impl RunSummary {
    /// Short human-readable report for the `/scrape` reply.
    pub fn describe(&self) -> String {
        let plural = |n: usize, one: &str, many: &str| if n == 1 { one.to_string() } else { many.to_string() };
        let mut text = format!(
            "Checked {} {} · {} new {} · posted {} {}",
            self.companies_scraped,
            plural(self.companies_scraped, "company", "companies"),
            self.new_jobs,
            plural(self.new_jobs, "job", "jobs"),
            self.posted,
            plural(self.posted, "match", "matches"),
        );
        if self.remaining > 0 {
            text.push_str(&format!(" · **{} more waiting**, press *Show next* to see them", self.remaining));
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alert_card_layout() {
        let company = Company {
            id: 1,
            display_name: "onsemi".into(),
            url: "https://www.onsemi.com/".into(),
            hex_code: "#E65300".into(),
            icon_url: "https://www.google.com/s2/favicons?domain=onsemi.com&sz=128".into(),
            board_type: "WORKDAY".into(),
            board_url: None,
        };
        let candidate = Candidate {
            application_id: 1,
            company_id: 1,
            board_type: "WORKDAY".into(),
            job_title: "Embedded Firmware Intern".into(),
            url: "https://onsemi.wd5.myworkdayjobs.com/onsemi/job/Waterloo/Embedded-Firmware-Intern_JR2018381".into(),
            page_content: "onsemi is a leader in intelligent power. What you'll do: Write C firmware for STM32 microcontrollers \
                           and bring up new boards. Requirements: C, RTOS, I2C, SPI. Nice to have: UVM. Applicants must be \
                           returning to school after the term."
                .into(),
            lower_wage_cents: Some(3500),
            upper_wage_cents: Some(4500),
            currency: "CAD".into(),
            cities: vec![crate::City { display_name: "Waterloo".into(), region: "ON".into(), country: "CA".into() }],
            terms: vec![crate::Term { display_name: "S27".into() }],
        };
        let knowledge = ResumeKnowledge::from_tex(include_str!("../docs/master_resume_example.tex")).unwrap();
        let json = serde_json::to_value(build_embed(&company, &candidate, Some(&knowledge))).unwrap();
        let field = |name: &str| json["fields"].as_array().unwrap().iter().find(|f| f["name"] == name).map(|f| f["value"].as_str().unwrap().to_string());
        println!("{}", serde_json::to_string_pretty(&json).unwrap());

        assert_eq!(field("Location").as_deref(), Some("Waterloo, ON, CA"));
        assert_eq!(field("Pay").as_deref(), Some("$35–45/hr CAD"));
        assert_eq!(field("Term").as_deref(), Some("S27"));
        let resume_match = field("Resume match").unwrap();
        assert!(resume_match.contains("%**\nHas: ") && resume_match.contains("\nMissing: "), "{resume_match}");
        let missing = resume_match.lines().find(|l| l.starts_with("Missing:")).unwrap();
        assert!(!missing.contains("UVM"), "the example resume tags UVM, so it isn't missing");
        assert_eq!(field("Check before applying").as_deref(), Some("Must return to school after the internship"));
        assert!(json["description"].as_str().unwrap().starts_with("Write C firmware for STM32"), "preview skips boilerplate");
        assert_eq!(json["footer"]["text"], "Workday · Job ID JR2018381");

        // No emoji anywhere in the card.
        let text = json.to_string();
        assert!(!text.chars().any(|c| matches!(c as u32, 0x1F300..=0x1FAFF | 0x2600..=0x27BF)), "{text}");
    }

    #[test]
    fn link_buttons_are_validated() {
        assert!(link_button_ok("https://example.com/job/1"));
        assert!(!link_button_ok("javascript:alert(1)"));
        assert!(!link_button_ok(&format!("https://example.com/{}", "a".repeat(600))));
        let rows = job_buttons(&format!("https://example.com/{}", "a".repeat(600)));
        assert_eq!(rows.len(), 1, "Apply/Ignore still posted without the link");
    }

    #[test]
    fn snippet_starts_at_the_work() {
        let text = "Acme is a leading provider of things since 1901. We value people. \
                    What you'll do: Write firmware for our battery management systems and bring up new boards in the lab.";
        assert_eq!(snippet(text, 300), "Write firmware for our battery management systems and bring up new boards in the lab.");
        let cut = snippet(text, 30);
        assert!(cut.ends_with('…') && cut.chars().count() <= 31, "{cut}");
        assert_eq!(snippet("Short.", 50), "Short.");
    }
}