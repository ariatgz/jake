use crate::outreach::{self, Draft};
use crate::resume::{self, OutreachFacts};
use crate::{contacts, db, Context, Error};
use poise::reply::CreateReply;
use poise::serenity_prelude as serenity;
use sqlx::PgPool;

const NOT_A_JOB_THREAD: &str = "Run this inside a job's thread (the one that opens when you click **Apply**).";

/// Track who you've reached out to about a job, and get follow-up reminders
#[poise::command(slash_command, subcommands("draft", "sent", "replied", "list"), guild_only)]
pub async fn outreach(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

/// Draft text for one job. Shared by `/outreach draft` and the Draft button.
pub async fn draft_text(pool: &PgPool, user: &serenity::User, job: &db::JobDetail, contact: Option<&str>) -> Result<String, Error> {
    let stored = db::resume(pool, user.id.get() as i64).await?;
    let facts = stored
        .as_ref()
        .and_then(|r| resume::outreach_facts(&r.tex, &job.job_title, &job.page_content).ok())
        .unwrap_or_else(OutreachFacts::default);
    let req = contacts::from_posting(&job.page_content, &job.url, &job.company_name).req_id;
    let display = user.global_name.clone().unwrap_or_else(|| user.name.clone());
    let draft = Draft {
        contact,
        role: &job.job_title,
        company: &job.company_name,
        req_id: req.as_deref(),
        facts: &facts,
        fallback_sender: &display,
    };
    let mut text = outreach::render(&draft);
    if stored.is_none() {
        text.push_str("\n-# Upload your resume with `/resume upload` and drafts will mention your most relevant experience.");
    }
    if contact.is_none() {
        text.push_str("\n-# Tip: `/outreach draft name:<their name>` puts their name in.");
    }
    Ok(text)
}

/// Draft a LinkedIn note and a message for this job, based on your resume
#[poise::command(slash_command, guild_only)]
pub async fn draft(ctx: Context<'_>, #[description = "Who you're writing to"] name: Option<String>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let Some((job, _)) = db::job_for_message(&ctx.data().pool, ctx.channel_id().get()).await? else {
        ctx.say(NOT_A_JOB_THREAD).await?;
        return Ok(());
    };
    let text = draft_text(&ctx.data().pool, ctx.author(), &job, name.as_deref().map(str::trim).filter(|n| !n.is_empty())).await?;
    ctx.say(text).await?;
    Ok(())
}

/// Log that you messaged someone about this job; you'll get a follow-up reminder
#[poise::command(slash_command, guild_only)]
pub async fn sent(
    ctx: Context<'_>,
    #[description = "Who you messaged"] name: String,
    #[description = "Their LinkedIn or email (optional)"] link: Option<String>,
    #[description = "Remind me after this many days (default 5)"]
    #[min = 1]
    #[max = 30]
    follow_up_days: Option<i32>,
) -> Result<(), Error> {
    let pool = &ctx.data().pool;
    let user_id = ctx.author().id.get() as i64;
    let Some((notification_id, owner)) = db::notification_for_thread(pool, ctx.channel_id().get()).await? else {
        ctx.send(CreateReply::default().ephemeral(true).content(NOT_A_JOB_THREAD)).await?;
        return Ok(());
    };
    if owner != user_id && owner != 0 {
        ctx.send(CreateReply::default().ephemeral(true).content("Log outreach from your own job threads.")).await?;
        return Ok(());
    }
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        ctx.send(CreateReply::default().ephemeral(true).content("Give the person's name (up to 100 characters).")).await?;
        return Ok(());
    }
    let link = link.map(|l| l.trim().to_string()).filter(|l| !l.is_empty() && l.len() <= 300);
    let days = follow_up_days.unwrap_or(5);
    let at = db::insert_outreach(pool, notification_id, user_id, name, link.as_deref(), days).await?;
    ctx.say(format!(
        "Logged your message to **{name}**. If you haven't marked a reply by <t:{at}:D> (<t:{at}:R>), I'll remind you here with a follow-up draft."
    ))
    .await?;
    Ok(())
}

/// Mark that someone replied, which stops their follow-up reminders
#[poise::command(slash_command, guild_only)]
pub async fn replied(ctx: Context<'_>, #[description = "Who replied (default: the most recent person you logged)"] name: Option<String>) -> Result<(), Error> {
    let pool = &ctx.data().pool;
    let user_id = ctx.author().id.get() as i64;
    let Some((notification_id, _)) = db::notification_for_thread(pool, ctx.channel_id().get()).await? else {
        ctx.send(CreateReply::default().ephemeral(true).content(NOT_A_JOB_THREAD)).await?;
        return Ok(());
    };
    let name = name.as_deref().map(str::trim).filter(|n| !n.is_empty());
    match db::mark_outreach_replied(pool, notification_id, user_id, name).await? {
        Some(contact) => {
            ctx.say(format!("Marked **{contact}** as replied, so no more reminders for them. Good luck with the conversation.")).await?;
        }
        None => {
            ctx.send(CreateReply::default().ephemeral(true).content("No open outreach in this thread matches that. Log one with `/outreach sent`."))
                .await?;
        }
    }
    Ok(())
}

/// Everyone you're waiting to hear back from
#[poise::command(slash_command, guild_only)]
pub async fn list(ctx: Context<'_>) -> Result<(), Error> {
    let rows = db::open_outreach(&ctx.data().pool, ctx.author().id.get() as i64).await?;
    if rows.is_empty() {
        ctx.send(CreateReply::default().ephemeral(true).content("Nobody pending. Log messages with `/outreach sent` inside a job thread.")).await?;
        return Ok(());
    }
    let lines: Vec<String> = rows
        .iter()
        .map(|r| {
            let who = match &r.contact_url {
                Some(url) if url.starts_with("http") => format!("[{}]({url})", r.contact_name),
                _ => format!("**{}**", r.contact_name),
            };
            let next = r.follow_up_unix.map(|t| format!("nudge <t:{t}:R>")).unwrap_or_else(|| "no more reminders".into());
            format!("{who} · {} @ {} · sent <t:{}:R> · {next} · <#{}>", r.job_title, r.company_name, r.sent_unix, r.thread_id)
        })
        .collect();
    let embed = serenity::CreateEmbed::new()
        .title("Waiting to hear back")
        .colour(serenity::Colour::from(0x5865F2u32))
        .description(resume::short(&lines.join("\n"), 4000))
        .footer(serenity::CreateEmbedFooter::new("Mark replies with /outreach replied inside the job's thread"));
    ctx.send(CreateReply::default().ephemeral(true).embed(embed)).await?;
    Ok(())
}
