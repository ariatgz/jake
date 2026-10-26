use crate::{db, worker, Context, Error};

/// Scrape your tracked companies now and post matching jobs here
#[poise::command(slash_command, guild_only)]
pub async fn scrape(ctx: Context<'_>) -> Result<(), Error> {
    // Send an immediate 'pending' response so Discord doesn't time out.
    ctx.defer().await?;

    let pool = &ctx.data().pool;
    let user_id = ctx.author().id.get() as i64;

    let Some(profile) = db::profile(pool, user_id).await? else {
        ctx.say("You don't have a profile yet. Run `/profile setup` first.").await?;
        return Ok(());
    };

    if db::tracked_companies(pool, user_id).await?.is_empty() {
        ctx.say("You aren't tracking any companies yet. Add one with `/add-company <url>` or `/track-company`.").await?;
        return Ok(());
    }

    // The work happens on the scraper thread; this handler only waits for the summary.
    let summary = match ctx.data().scraper.run(vec![profile], Some(ctx.channel_id())).await {
        Ok(summary) => summary,
        Err(worker::Busy) => {
            ctx.say("A scrape is already running. Try again in a minute.").await?;
            return Ok(());
        }
    };

    ctx.say(summary.describe()).await?;
    Ok(())
}
