use crate::telegram as tg;
use crate::{db, Context, Error};
use poise::reply::CreateReply;

/// Get your job alerts on Telegram as well as Discord
#[poise::command(slash_command, subcommands("link", "unlink"))]
pub async fn telegram(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

async fn reply(ctx: Context<'_>, text: impl Into<String>) -> Result<(), Error> {
    ctx.send(CreateReply::default().ephemeral(true).content(text)).await?;
    Ok(())
}

/// Send copies of your job alerts to your Telegram
#[poise::command(slash_command)]
pub async fn link(
    ctx: Context<'_>,
    #[description = "Your Telegram user ID (message @userinfobot on Telegram to see it)"] chat_id: i64,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let pool = &ctx.data().pool;
    let client = &ctx.data().client;
    let user_id = ctx.author().id.get() as i64;

    let Some(token) = tg::token() else {
        return reply(ctx, "Telegram isn't set up on this bot yet. Whoever hosts it needs to create a bot with @BotFather and set `TELEGRAM_BOT_TOKEN`.").await;
    };
    if db::profile(pool, user_id).await?.is_none() {
        return reply(ctx, "You don't have a profile yet. Run `/profile setup` first.").await;
    }

    // Telegram bots can only message people who pressed Start on them, so this doubles as a check.
    let test = "✅ Linked to your job alerts. New matches will show up here as well as in Discord.";
    if let Err(e) = tg::send(client, &token, chat_id, test).await {
        let open = match tg::bot_username(client, &token).await {
            Some(name) => format!("Open https://t.me/{name} and press **Start**"),
            None => "Open the bot on Telegram and press **Start**".to_string(),
        };
        return reply(ctx, format!("Couldn't message that Telegram account ({e}).\n{open}, check the ID with @userinfobot, then run this again.")).await;
    }

    db::set_telegram_chat(pool, user_id, Some(chat_id)).await?;
    reply(ctx, "Linked. You should have a test message on Telegram. Turn it off with `/telegram unlink`.").await
}

/// Stop sending your job alerts to Telegram
#[poise::command(slash_command)]
pub async fn unlink(ctx: Context<'_>) -> Result<(), Error> {
    let user_id = ctx.author().id.get() as i64;
    match db::profile(&ctx.data().pool, user_id).await? {
        Some(profile) if profile.telegram_chat_id.is_some() => {
            db::set_telegram_chat(&ctx.data().pool, user_id, None).await?;
            reply(ctx, "Unlinked. Alerts go to Discord only now.").await
        }
        _ => reply(ctx, "Telegram isn't linked to your profile.").await,
    }
}
