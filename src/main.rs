mod commands;
mod contacts;
mod db;
mod filter;
mod flags;
mod outreach;
mod pipeline;
mod resume;
mod scheduler;
mod scrapers;
mod telegram;
mod worker;

use poise::serenity_prelude as serenity;
use std::env::var;
use std::sync::Arc;
use std::time::Duration;

pub use db::{Company, Profile};

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
    pub lower_wage_cents: Option<i16>, // hourly, None when the board doesn't publish pay
    pub upper_wage_cents: Option<i16>, // None when there is no range
    pub currency: String,
}

#[derive(Debug, Clone)]
pub struct ScrapeEvent {
    pub terms: Option<Vec<Term>>,
    pub cities: Vec<City>,
    pub application: Application,
}

pub struct Data {
    pub pool: sqlx::PgPool,
    pub client: reqwest::Client,
    /// Held by the scraper thread while a scrape is running, and briefly by
    /// the "Show next" button, so a batch is never posted twice at once.
    pub scrape_lock: Arc<tokio::sync::Mutex<()>>,
    /// Runs scrapes on their own thread; see `worker`.
    pub scraper: worker::Handle,
}

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/150.0.0.0 Safari/537.36";

/// Browser-like client. Careers sites behind Cloudflare and friends answer 403
/// to requests without an Accept-Language header, so the defaults look like a
/// real browser; API scrapers override Accept per request.
pub fn http_client(user_agent: &str) -> reqwest::Result<reqwest::Client> {
    use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, ACCEPT_LANGUAGE};
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("text/html,application/xhtml+xml,application/json,*/*;q=0.8"));
    headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("en-US,en;q=0.9"));
    reqwest::Client::builder().user_agent(user_agent).default_headers(headers).timeout(Duration::from_secs(40)).build()
}

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Context<'a> = poise::Context<'a, Data, Error>;

const DEFAULT_SCRAPE_INTERVAL_MINUTES: u64 = 30;

async fn on_error(error: poise::FrameworkError<'_, Data, Error>) {
    match error {
        poise::FrameworkError::Setup { error, .. } => panic!("Failed to start bot: {:?}", error),
        poise::FrameworkError::Command { error, ctx, .. } => {
            println!("Error in command `{}`: {:?}", ctx.command().name, error);
            let _ = ctx.say(format!("Something went wrong: {}", error)).await;
        }
        error => {
            if let Err(e) = poise::builtins::on_error(error).await {
                println!("Error while handling error: {}", e)
            }
        }
    }
}

fn ephemeral(content: impl Into<String>) -> serenity::CreateInteractionResponse {
    serenity::CreateInteractionResponse::Message(serenity::CreateInteractionResponseMessage::new().content(content).ephemeral(true))
}

async fn handle_button(ctx: &serenity::Context, data: &serenity::ComponentInteraction, shared: &Data) -> Result<(), Error> {
    let action = data.data.custom_id.as_str();
    if let Some(owner) = action.strip_prefix(pipeline::MORE_PREFIX) {
        return show_more(ctx, data, shared, owner).await;
    }
    if action == contacts::DRAFT_BUTTON {
        return draft_button(ctx, data, shared).await;
    }
    if action != "ACTIVE" && action != "IGNORED" {
        println!("Unknown interaction type: `{:?}`", action);
        return Ok(());
    }

    let pool = &shared.pool;
    let message_id = data.message.id.get();

    // Only the person the job was posted for gets to act on it.
    if let Some(owner) = db::notification_owner(pool, message_id).await? {
        if owner != 0 && owner != data.user.id.get() as i64 {
            data.create_response(ctx, ephemeral("This posting belongs to someone else's profile.")).await?;
            return Ok(());
        }
    }

    // Swap Apply/Ignore for just the posting link so they can't be clicked twice.
    let posting_url = data.message.embeds.first().and_then(|e| e.url.clone()).filter(|u| pipeline::link_button_ok(u));
    let remaining = match &posting_url {
        Some(url) => vec![serenity::CreateActionRow::Buttons(vec![serenity::CreateButton::new_link(url).label("Open posting")])],
        None => vec![],
    };
    data.create_response(
        ctx,
        serenity::CreateInteractionResponse::UpdateMessage(serenity::CreateInteractionResponseMessage::new().components(remaining)),
    )
    .await?;

    if action == "ACTIVE" {
        let title = data
            .message
            .embeds
            .first()
            .map(|embed| {
                let job = embed.title.clone().unwrap_or_else(|| "Job".to_string());
                match embed.author.as_ref() {
                    Some(author) => format!("{} @ {}", job, author.name),
                    None => job,
                }
            })
            .unwrap_or_else(|| "Application".to_string());
        let title: String = title.chars().take(100).collect();

        let thread_builder = serenity::CreateThread::new(title).kind(serenity::ChannelType::PublicThread);
        if let Ok(thread) = data.channel_id.create_thread_from_message(ctx, data.message.id, thread_builder).await {
            // Contacts and the tailored resume take a few seconds (a search, a
            // few LaTeX compiles on a Pi), so they're posted in the background.
            let http = ctx.http.clone();
            let pool = pool.clone();
            let client = shared.client.clone();
            let user = data.user.clone();
            tokio::spawn(async move {
                if let Err(e) = start_application(&http, &pool, &client, thread.id, &user, message_id).await {
                    eprintln!("Setting up thread for message {message_id} failed: {e}");
                }
            });
        }
    }

    db::update_notification_state(pool, message_id, action).await?;

    Ok(())
}

/// Fills a fresh job thread: next steps, people to contact, tailored resume.
async fn start_application(
    http: &serenity::Http,
    pool: &sqlx::PgPool,
    client: &reqwest::Client,
    thread: serenity::ChannelId,
    user: &serenity::User,
    message_id: u64,
) -> Result<(), Error> {
    let Some((job, _)) = db::job_for_message(pool, message_id).await? else {
        thread.say(http, "Good luck with the application!").await?;
        return Ok(());
    };
    let user_id = user.id.get() as i64;
    let stored = db::resume(pool, user_id).await?;
    let tailoring = stored.as_ref().is_some_and(|r| r.auto_tailor);

    let resume_step = if tailoring {
        "Your tailored resume is being built and will show up below."
    } else if stored.is_some() {
        "Run `/resume tailor` here for a resume tailored to this job."
    } else {
        "Upload your resume with `/resume upload` to get a tailored copy for every job."
    };
    let steps = serenity::CreateEmbed::new()
        .title("Application started")
        .colour(serenity::Colour::from(0x3BA55Cu32))
        .description(format!(
            "**{}** at **{}**\n\n1. {resume_step}\n2. Apply on the company site: [open the posting]({})\n\
             3. Reach out to someone (see People to contact below)\n4. Once you've applied, run `/update SUBMITTED`",
            job.job_title, job.company_name, job.url
        ));
    thread.send_message(http, serenity::CreateMessage::new().content(format!("<@{user_id}>")).embed(steps)).await?;

    let school = stored.as_ref().and_then(|r| resume::latex::Document::parse(&r.tex).ok()).and_then(|d| d.school());
    let ctx = contacts::SearchContext::new(&job.company_name, &job.job_title, &job.cities, school);
    let found = contacts::from_posting(&job.page_content, &job.url, &job.company_name);
    let people = contacts::search_people(client, pool, &ctx).await;
    let (embed, rows) = contacts::panel(&ctx, &found, people.as_deref());
    thread.send_message(http, serenity::CreateMessage::new().embed(embed).components(rows)).await?;

    let name = user.global_name.clone().unwrap_or_else(|| user.name.clone());
    commands::resume::tailor_into_thread(http, pool, client, thread, user_id, &name, &job).await;
    Ok(())
}

/// "Show next 10": posts the next batch of matches into the same channel.
async fn show_more(ctx: &serenity::Context, data: &serenity::ComponentInteraction, shared: &Data, owner: &str) -> Result<(), Error> {
    let Ok(owner) = owner.parse::<i64>() else { return Ok(()) };
    if owner != data.user.id.get() as i64 {
        data.create_response(ctx, ephemeral("These are someone else's matches.")).await?;
        return Ok(());
    }
    // Same lock as scrapes, so a batch is never posted twice at once.
    let Ok(_guard) = shared.scrape_lock.try_lock() else {
        data.create_response(ctx, ephemeral("A scrape is running right now. Try again in a minute.")).await?;
        return Ok(());
    };
    data.create_response(
        ctx,
        serenity::CreateInteractionResponse::UpdateMessage(
            serenity::CreateInteractionResponseMessage::new().content("Loading the next batch...").components(vec![]),
        ),
    )
    .await?;

    let Some(profile) = db::profile(&shared.pool, owner).await? else { return Ok(()) };
    let mut summary = pipeline::RunSummary::default();
    pipeline::notify_profile(&ctx.http, &shared.pool, &shared.client, &profile, data.channel_id, &mut summary).await;
    for error in &summary.errors {
        eprintln!("Show next: {error}");
    }
    let done = if summary.posted == 0 { "Nothing new left to show.".to_string() } else { format!("Showed {} more.", summary.posted) };
    data.edit_response(ctx, serenity::EditInteractionResponse::new().content(done)).await?;
    Ok(())
}

/// "Draft message" in a job thread: drafts only the clicker sees.
async fn draft_button(ctx: &serenity::Context, data: &serenity::ComponentInteraction, shared: &Data) -> Result<(), Error> {
    data.create_response(
        ctx,
        serenity::CreateInteractionResponse::Defer(serenity::CreateInteractionResponseMessage::new().ephemeral(true)),
    )
    .await?;
    let text = match db::job_for_message(&shared.pool, data.channel_id.get()).await? {
        Some((job, _)) => commands::outreach::draft_text(&shared.pool, &data.user, &job, None).await?,
        None => "This button only works inside a job thread.".to_string(),
    };
    data.edit_response(ctx, serenity::EditInteractionResponse::new().content(text)).await?;
    Ok(())
}
#[tokio::main]
async fn main() -> Result<(), Error> {
    dotenvy::dotenv().ok();

    let conn_str = var("DATABASE_URL").expect("Missing `DATABASE_URL` env var, see README for more information");
    // A handful of connections covers the scheduler plus a few commands at
    // once, and stays well inside Supabase's pooler limits.
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&conn_str)
        .await?;

    let client = http_client(USER_AGENT)?;

    let interval_minutes = var("SCRAPE_INTERVAL_MINUTES")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(DEFAULT_SCRAPE_INTERVAL_MINUTES);

    let options = poise::FrameworkOptions {
        commands: vec![
            commands::scrape::scrape(),
            commands::update::update(),
            commands::company::add_company(),
            commands::company::remove_company(),
            commands::company::track_company(),
            commands::company::companies(),
            commands::profile::profile(),
            commands::profile::filters(),
            commands::profile::status(),
            commands::resume::resume(),
            commands::outreach::outreach(),
            commands::telegram::telegram(),
        ],
        on_error: |error| Box::pin(on_error(error)),
        pre_command: |ctx| {
            Box::pin(async move {
                println!("Executing command {}...", ctx.command().qualified_name);
            })
        },
        post_command: |ctx| {
            Box::pin(async move {
                println!("Executed command {}!", ctx.command().qualified_name);
            })
        },
        event_handler: |ctx, event, _framework, shared_data| {
            Box::pin(async move {
                if let serenity::FullEvent::InteractionCreate {
                    interaction: serenity::Interaction::Component(data),
                } = event
                {
                    if let Err(e) = handle_button(ctx, data, shared_data).await {
                        eprintln!("Button handler failed: {:?}", e);
                    }
                }
                Ok(())
            })
        },
        ..Default::default()
    };

    let scrape_lock = Arc::new(tokio::sync::Mutex::new(()));

    let framework = poise::Framework::builder()
        .setup(move |ctx, ready, framework| {
            Box::pin(async move {
                println!("Logged in as {}", ready.user.name);
                poise::builtins::register_globally(ctx, &framework.options().commands).await?;

                let scraper = worker::spawn(ctx.http.clone(), conn_str.clone(), USER_AGENT.to_string(), scrape_lock.clone());
                scheduler::spawn(scraper.clone(), pool.clone(), Duration::from_secs(interval_minutes * 60));
                scheduler::spawn_follow_ups(ctx.http.clone(), pool.clone());
                scheduler::spawn_cleanup(pool.clone());

                Ok(Data { pool, client, scrape_lock, scraper })
            })
        })
        .options(options)
        .build();

    let token = var("DISCORD_TOKEN").expect("Missing `DISCORD_TOKEN` env var, see README for more information.");
    let intents = serenity::GatewayIntents::non_privileged() | serenity::GatewayIntents::MESSAGE_CONTENT;

    let mut client = serenity::ClientBuilder::new(token, intents)
        .framework(framework)
        .await?;

    client.start().await?;

    Ok(())
}
