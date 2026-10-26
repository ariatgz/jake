use crate::scrapers::detect;
use crate::{db, Context, Error};
use futures::Stream;
use url::Url;

const DEFAULT_COLOUR: &str = "#5865F2";

fn favicon_for(domain: &str) -> String {
    format!("https://www.google.com/s2/favicons?domain={domain}&sz=128")
}

/// Homepage + favicon guesses for a newly added company.
fn homepage_for(board: &detect::Board, given_url: &str) -> (String, String) {
    let given = Url::parse(given_url).ok();
    let given_host = given.as_ref().and_then(|u| u.host_str()).map(|h| h.to_lowercase());

    let is_board_host = given_host.as_deref().map_or(true, |h| {
        h.ends_with("myworkdayjobs.com") || h.ends_with("greenhouse.io") || h.ends_with("lever.co") || h.ends_with("ashbyhq.com")
    });

    let domain = match (&board, is_board_host) {
        (detect::Board::Jibe { base, .. }, _) => Url::parse(base)
            .ok()
            .and_then(|u| u.host_str().map(|h| format!("{}.com", detect::second_level_domain(h))))
            .unwrap_or_else(|| "example.com".to_string()),
        (_, false) => {
            let host = given_host.unwrap_or_default();
            let parts: Vec<&str> = host.split('.').collect();
            if parts.len() >= 2 { parts[parts.len() - 2..].join(".") } else { host }
        }
        (detect::Board::Workday { tenant, .. }, true) => format!("{tenant}.com"),
        // Shared Oracle host; nothing in the URL names the company.
        (detect::Board::Oracle { .. }, true) => "example.com".to_string(),
        (detect::Board::Greenhouse { token } | detect::Board::Lever { token } | detect::Board::Ashby { token }, true) => {
            format!("{}.com", token.replace(['-', '_'], ""))
        }
    };

    (format!("https://www.{domain}/"), favicon_for(&domain))
}

async fn autocomplete_tracked<'a>(ctx: Context<'_>, partial: &'a str) -> impl Stream<Item = String> + 'a {
    let partial = partial.to_lowercase();
    let names: Vec<String> = db::tracked_companies(&ctx.data().pool, ctx.author().id.get() as i64)
        .await
        .map(|companies| companies.into_iter().map(|c| c.display_name).collect())
        .unwrap_or_default();
    futures::stream::iter(names.into_iter().filter(move |n| n.to_lowercase().contains(&partial)).take(25))
}

async fn autocomplete_all<'a>(ctx: Context<'_>, partial: &'a str) -> impl Stream<Item = String> + 'a {
    let partial = partial.to_lowercase();
    let names: Vec<String> = db::all_companies(&ctx.data().pool)
        .await
        .map(|companies| companies.into_iter().map(|c| c.display_name).collect())
        .unwrap_or_default();
    futures::stream::iter(names.into_iter().filter(move |n| n.to_lowercase().contains(&partial)).take(25))
}

/// Add a company's careers page to your profile (Workday, Greenhouse, Lever, Ashby, Jibe/iCIMS)
#[poise::command(slash_command, rename = "add-company", guild_only)]
pub async fn add_company(
    ctx: Context<'_>,
    #[description = "Careers page or job board URL"] url: String,
    #[description = "Display name (defaults to a guess from the URL)"] name: Option<String>,
) -> Result<(), Error> {
    ctx.defer().await?;

    let pool = &ctx.data().pool;
    let user_id = ctx.author().id.get() as i64;

    if db::profile(pool, user_id).await?.is_none() {
        ctx.say("You don't have a profile yet. Run `/profile setup` first.").await?;
        return Ok(());
    }

    let board = match detect::detect(&ctx.data().client, &url).await {
        Ok(board) => board,
        Err(e) => {
            ctx.say(format!("{e}")).await?;
            return Ok(());
        }
    };
    let board_url = board.canonical_url();

    let mut created = false;
    let company = match db::company_by_board_url(pool, &board_url).await? {
        Some(existing) => existing,
        None => {
            let display_name = name
                .map(|n| n.trim().to_string())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| board.suggested_name());
            if let Some(existing) = db::company_by_name(pool, &display_name).await? {
                existing
            } else {
                created = true;
                let (homepage, icon) = homepage_for(&board, &url);
                db::insert_company(pool, &display_name, &homepage, DEFAULT_COLOUR, &icon, board.kind(), &board_url).await?
            }
        }
    };

    let tracked_now = db::track_company(pool, user_id, company.id).await?;

    let mut message = if created {
        format!("Added **{}** as a {} board and started tracking it for you.", company.display_name, board.kind())
    } else if tracked_now {
        format!("**{}** was already known ({}). Now tracking it for you.", company.display_name, company.board_type)
    } else {
        format!("You're already tracking **{}**.", company.display_name)
    };
    message.push_str(&format!("\nBoard: <{board_url}>"));
    if created || tracked_now {
        message.push_str("\nRun `/scrape` to pull its jobs now, or wait for the next scheduled check.");
    }

    ctx.say(message).await?;
    Ok(())
}

/// Stop tracking a company
#[poise::command(slash_command, rename = "remove-company", guild_only)]
pub async fn remove_company(
    ctx: Context<'_>,
    #[description = "Company name"]
    #[autocomplete = "autocomplete_tracked"]
    name: String,
) -> Result<(), Error> {
    let pool = &ctx.data().pool;
    let user_id = ctx.author().id.get() as i64;

    let Some(company) = db::company_by_name(pool, name.trim()).await? else {
        ctx.say(format!("No company called `{}`.", name.trim())).await?;
        return Ok(());
    };

    if db::untrack_company(pool, user_id, company.id).await? {
        ctx.say(format!("Stopped tracking **{}**.", company.display_name)).await?;
    } else {
        ctx.say(format!("You weren't tracking **{}**.", company.display_name)).await?;
    }
    Ok(())
}

/// Track a company that's already in the database
#[poise::command(slash_command, rename = "track-company", guild_only)]
pub async fn track_company(
    ctx: Context<'_>,
    #[description = "Company name"]
    #[autocomplete = "autocomplete_all"]
    name: String,
) -> Result<(), Error> {
    let pool = &ctx.data().pool;
    let user_id = ctx.author().id.get() as i64;

    if db::profile(pool, user_id).await?.is_none() {
        ctx.say("You don't have a profile yet. Run `/profile setup` first.").await?;
        return Ok(());
    }

    let Some(company) = db::company_by_name(pool, name.trim()).await? else {
        ctx.say(format!(
            "No company called `{}`. Use `/companies` to see what's available or `/add-company` to add one.",
            name.trim()
        ))
        .await?;
        return Ok(());
    };

    if db::track_company(pool, user_id, company.id).await? {
        ctx.say(format!("Now tracking **{}** ({}).", company.display_name, company.board_type)).await?;
    } else {
        ctx.say(format!("You're already tracking **{}**.", company.display_name)).await?;
    }
    Ok(())
}

/// List every company the bot knows about
#[poise::command(slash_command, guild_only)]
pub async fn companies(ctx: Context<'_>) -> Result<(), Error> {
    let pool = &ctx.data().pool;
    let user_id = ctx.author().id.get() as i64;

    let all = db::all_companies(pool).await?;
    let tracked: Vec<i64> = db::tracked_companies(pool, user_id).await?.into_iter().map(|c| c.id).collect();

    if all.is_empty() {
        ctx.say("No companies yet. Add one with `/add-company <url>`.").await?;
        return Ok(());
    }

    let mut lines: Vec<String> = all
        .iter()
        .map(|c| {
            let marker = if tracked.contains(&c.id) { "✅" } else { "▫️" };
            format!("{marker} **{}** · {}", c.display_name, c.board_type)
        })
        .collect();
    lines.insert(0, "✅ = tracked by you. Use `/track-company <name>` to follow one.".to_string());

    ctx.say(lines.join("\n").chars().take(1900).collect::<String>()).await?;
    Ok(())
}
