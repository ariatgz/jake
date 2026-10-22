use crate::{db, Context, Error};
use futures::{Stream, StreamExt};
use poise::reply::CreateReply;
use poise::serenity_prelude as serenity;
use serenity::ChannelType;
use serenity::Timestamp;

async fn autocomplete_state<'a>(_ctx: Context<'_>, partial: &'a str) -> impl Stream<Item = String> + 'a {
    futures::stream::iter(&["ACTIVE", "SUBMITTED", "REJECTED", "DELETED", "IGNORED"])
        .filter(move |state| futures::future::ready(state.to_lowercase().starts_with(&partial.to_lowercase())))
        .map(|state| state.to_string())
}

/// Update the state of the application whose thread you're in
#[poise::command(slash_command, guild_only)]
pub async fn update(
    ctx: Context<'_>,
    #[description = "Application's new state"]
    #[autocomplete = "autocomplete_state"]
    after_state: String,
) -> Result<(), Error> {
    ctx.defer().await?;

    let after_state = after_state.trim().to_uppercase();
    if !["ACTIVE", "SUBMITTED", "REJECTED", "DELETED", "IGNORED"].contains(&after_state.as_str()) {
        ctx.say("State must be one of ACTIVE, SUBMITTED, REJECTED, DELETED or IGNORED.").await?;
        return Ok(());
    }

    let Some(guild_channel) = ctx.guild_channel().await else {
        ctx.say("This command only works inside a server.").await?;
        return Ok(());
    };

    if guild_channel.kind != ChannelType::PublicThread {
        ctx.say("Invalid use of /update. Use this command inside a job thread.").await?;
        return Ok(());
    }

    // A thread created from a message shares that message's id.
    let message_id = guild_channel.id.get();

    let Some(before_state) = db::update_notification_state(&ctx.data().pool, message_id, &after_state).await? else {
        ctx.say("This thread isn't attached to a tracked job posting.").await?;
        return Ok(());
    };

    let response = CreateReply::default().embed(
        serenity::CreateEmbed::new()
            .title("Application Updated")
            .description(format!("Application status changed from `{}` to `{}`.", before_state, after_state))
            .timestamp(Timestamp::now()),
    );

    ctx.send(response).await?;
    Ok(())
}
