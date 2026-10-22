mod commands;
mod db;
mod filter;
mod pipeline;
mod scheduler;
mod scrapers;

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
    /// Held while a scrape (manual or scheduled) is running so two never overlap.
    pub scrape_lock: Arc<tokio::sync::Mutex<()>>,
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

async fn handle_button(
    ctx: &serenity::Context,
    data: &serenity::ComponentInteraction,
    pool: &sqlx::PgPool,
) -> Result<(), Error> {
    let action = data.data.custom_id.as_str();
    if action != "ACTIVE" && action != "IGNORED" {
        println!("Unknown interaction type: `{:?}`", action);
        return Ok(());
    }

    let message_id = data.message.id.get();

    // Only the person the job was posted for gets to act on it.
    if let Some(owner) = db::notification_owner(pool, message_id).await? {
        if owner != 0 && owner != data.user.id.get() as i64 {
            data.create_response(
                ctx,
                serenity::CreateInteractionResponse::Message(
                    serenity::CreateInteractionResponseMessage::new()
                        .content("This posting belongs to someone else's profile.")
                        .ephemeral(true),
                ),
            )
            .await?;
            return Ok(());
        }
    }

    // Remove the buttons so it can't be clicked twice.
    data.create_response(
        ctx,
        serenity::CreateInteractionResponse::UpdateMessage(
            serenity::CreateInteractionResponseMessage::new().components(vec![]),
        ),
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

        let thread_builder =
            serenity::CreateThread::new(title).kind(serenity::ChannelType::PublicThread);

        let thread = data
            .channel_id
            .create_thread_from_message(ctx, data.message.id, thread_builder)
            .await;

        if let Ok(thread) = thread {
            let _ = thread.say(ctx, "Starting application... Good luck!").await;
        }
    }

    db::update_notification_state(pool, message_id, action).await?;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    dotenvy::dotenv().ok();

    let conn_str = var("DATABASE_URL").expect("Missing `DATABASE_URL` env var, see README for more information");
    let pool = sqlx::PgPool::connect(&conn_str).await?;

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/150.0.0.0 Safari/537.36")
        .timeout(Duration::from_secs(40))
        .build()?;

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
                    if let Err(e) = handle_button(ctx, data, &shared_data.pool).await {
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

                scheduler::spawn(
                    ctx.http.clone(),
                    pool.clone(),
                    client.clone(),
                    scrape_lock.clone(),
                    Duration::from_secs(interval_minutes * 60),
                );

                Ok(Data { pool, client, scrape_lock })
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
