use crate::filter::parse_terms_arg;
use crate::{db, Context, Error};
use poise::reply::CreateReply;
use poise::serenity_prelude as serenity;

#[derive(Debug, poise::ChoiceParameter)]
pub enum Preset {
    /// Internships in hardware/firmware/FPGA roles around Toronto, Waterloo, Markham and Vancouver, tracking the seeded chip companies
    #[name = "hardware_canada"]
    HardwareCanada,
    /// Only the default seniority filter (Intern, Internship, Co-op, Apprentice); add keywords and locations yourself
    #[name = "blank"]
    Blank,
}

#[derive(Debug, poise::ChoiceParameter)]
pub enum FilterField {
    #[name = "keywords"]
    Keywords,
    #[name = "exclude"]
    Exclude,
    #[name = "locations"]
    Locations,
    #[name = "seniority"]
    Seniority,
}

#[derive(Debug, Clone, Copy, poise::ChoiceParameter)]
pub enum KeywordScope {
    /// A keyword in the title or anywhere in the description counts
    #[name = "title_and_description"]
    TitleAndDescription,
    /// Only keywords in the job title count
    #[name = "title_only"]
    TitleOnly,
}

const HARDWARE_KEYWORDS: &[&str] = &["Hardware", "Firmware", "FPGA", "RTL", "ASIC", "Electrical", "Embedded"];
const HARDWARE_LOCATIONS: &[&str] = &["Toronto", "Waterloo", "Markham", "Vancouver", "Canada"];
const HARDWARE_COMPANIES: &[&str] = &[
    "AMD",
    "Altera",
    "Intel",
    "NVIDIA",
    "onsemi",
    "Astera Labs",
    "Etched",
    "Marvell",
    "Cadence",
    "Ciena",
    "Analog Devices",
    "Microchip",
    "General Motors",
    "BlackBerry",
    "Tenstorrent",
    "Lightmatter",
    "Cerebras",
    "Kepler Communications",
];

fn list_or_any(items: &[String]) -> String {
    if items.is_empty() { "*any*".to_string() } else { items.join(", ") }
}

async fn status_reply(ctx: Context<'_>, user_id: i64) -> Result<CreateReply, Error> {
    let pool = &ctx.data().pool;

    let Some(profile) = db::profile(pool, user_id).await? else {
        return Ok(CreateReply::default().content("You don't have a profile yet. Run `/profile setup` first."));
    };

    let companies = db::tracked_companies(pool, user_id).await?;
    let company_text = if companies.is_empty() {
        "*none* — add one with `/add-company <url>`".to_string()
    } else {
        companies.iter().map(|c| format!("{} ({})", c.display_name, c.board_type)).collect::<Vec<_>>().join("\n")
    };

    let counts = db::notification_counts(pool, user_id).await?;
    let counts_text = if counts.is_empty() {
        "*none yet*".to_string()
    } else {
        counts.iter().map(|(state, n)| format!("{state}: {n}")).collect::<Vec<_>>().join(" · ")
    };

    let keyword_label = if profile.keywords_title_only { "Keywords (title only)" } else { "Keywords (title or description)" };

    let embed = serenity::CreateEmbed::new()
        .title(format!("Profile for {}", profile.display_name))
        .field("Notifications go to", format!("<#{}>", profile.channel_id), false)
        .field("Telegram", if profile.telegram_chat_id.is_some() { "linked" } else { "off (`/telegram link`)" }, false)
        .field("Seniority (title must contain one)", list_or_any(&profile.seniority), false)
        .field(keyword_label, list_or_any(&profile.keywords), false)
        .field(
            "Excluded (title must not contain)",
            if profile.exclude_keywords.is_empty() { "*none*".to_string() } else { profile.exclude_keywords.join(", ") },
            false,
        )
        .field("Locations", list_or_any(&profile.locations), false)
        .field("Tracked companies", company_text, false)
        .field("Jobs posted to you", counts_text, false)
        .colour(serenity::Colour::from(0x5865F2u32));

    Ok(CreateReply::default().embed(embed))
}

/// Manage your job-alert profile
#[poise::command(slash_command, subcommands("setup", "show"), guild_only)]
pub async fn profile(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

/// Create or update your profile; job alerts are sent to this channel
#[poise::command(slash_command, guild_only)]
pub async fn setup(
    ctx: Context<'_>,
    #[description = "Starting filters and companies"] preset: Option<Preset>,
    #[description = "Channel for alerts (defaults to this one)"]
    #[channel_types("Text")]
    channel: Option<serenity::GuildChannel>,
) -> Result<(), Error> {
    ctx.defer().await?;

    let pool = &ctx.data().pool;
    let author = ctx.author();
    let user_id = author.id.get() as i64;
    let channel_id = channel.map(|c| c.id).unwrap_or(ctx.channel_id());
    let display_name = author.global_name.clone().unwrap_or_else(|| author.name.clone());

    let existed = db::profile(pool, user_id).await?.is_some();
    db::upsert_profile(pool, user_id, &display_name, channel_id.get() as i64).await?;

    let mut notes: Vec<String> = Vec::new();

    if let Some(Preset::HardwareCanada) = preset {
        let keywords: Vec<String> = HARDWARE_KEYWORDS.iter().map(|s| s.to_string()).collect();
        let locations: Vec<String> = HARDWARE_LOCATIONS.iter().map(|s| s.to_string()).collect();
        db::set_profile_filters(
            pool,
            user_id,
            &db::FilterUpdate { keywords: Some(&keywords), locations: Some(&locations), ..Default::default() },
        )
        .await?;

        let mut tracked = 0;
        for name in HARDWARE_COMPANIES {
            if let Some(company) = db::company_by_name(pool, name).await? {
                db::track_company(pool, user_id, company.id).await?;
                tracked += 1;
            }
        }
        notes.push(format!("Applied the hardware/Canada preset and tracked {tracked} companies."));
    }

    let verb = if existed { "Updated" } else { "Created" };
    notes.insert(0, format!("{verb} your profile. Alerts will be posted in <#{}>.", channel_id.get()));
    notes.push(
        "Use `/filters set` to change keywords, locations or seniority, `/add-company <url>` to follow more companies, and `/scrape` to run a check right now."
            .to_string(),
    );

    ctx.say(notes.join("\n")).await?;
    Ok(())
}

/// Show your filters and tracked companies
#[poise::command(slash_command, guild_only)]
pub async fn show(ctx: Context<'_>) -> Result<(), Error> {
    let reply = status_reply(ctx, ctx.author().id.get() as i64).await?;
    ctx.send(reply).await?;
    Ok(())
}

/// Show your filters and tracked companies
#[poise::command(slash_command, guild_only)]
pub async fn status(ctx: Context<'_>) -> Result<(), Error> {
    let reply = status_reply(ctx, ctx.author().id.get() as i64).await?;
    ctx.send(reply).await?;
    Ok(())
}

/// Change your job filters
#[poise::command(slash_command, subcommands("set", "clear"), guild_only)]
pub async fn filters(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

/// Replace one or more filter lists (comma-separated). Fields you leave out are unchanged.
#[poise::command(slash_command, guild_only)]
pub async fn set(
    ctx: Context<'_>,
    #[description = "Domain keywords, e.g. \"Hardware, Firmware, FPGA\""] keywords: Option<String>,
    #[description = "Skip jobs whose title contains any of these, e.g. \"Senior, PhD, Sales\""] exclude: Option<String>,
    #[description = "Cities/regions/countries, e.g. \"Toronto, Vancouver, Canada\""] locations: Option<String>,
    #[description = "Title must contain one, e.g. \"Intern, Co-op\""] seniority: Option<String>,
    #[description = "Where keywords must appear (default: title or description)"] keyword_scope: Option<KeywordScope>,
) -> Result<(), Error> {
    let pool = &ctx.data().pool;
    let user_id = ctx.author().id.get() as i64;

    if db::profile(pool, user_id).await?.is_none() {
        ctx.say("You don't have a profile yet. Run `/profile setup` first.").await?;
        return Ok(());
    }

    if keywords.is_none() && exclude.is_none() && locations.is_none() && seniority.is_none() && keyword_scope.is_none() {
        ctx.say("Give at least one of `keywords`, `exclude`, `locations`, `seniority` or `keyword_scope`.").await?;
        return Ok(());
    }

    let keywords = keywords.map(|k| parse_terms_arg(&k));
    let exclude = exclude.map(|e| parse_terms_arg(&e));
    let locations = locations.map(|l| parse_terms_arg(&l));
    let seniority = seniority.map(|s| parse_terms_arg(&s));

    let update = db::FilterUpdate {
        keywords: keywords.as_deref(),
        exclude_keywords: exclude.as_deref(),
        seniority: seniority.as_deref(),
        locations: locations.as_deref(),
        keywords_title_only: keyword_scope.map(|s| matches!(s, KeywordScope::TitleOnly)),
    };
    db::set_profile_filters(pool, user_id, &update).await?;

    let reply = status_reply(ctx, user_id).await?;
    ctx.send(reply.content("Filters updated.")).await?;
    Ok(())
}

/// Clear one filter list so it no longer restricts matches
#[poise::command(slash_command, guild_only)]
pub async fn clear(ctx: Context<'_>, #[description = "Which filter to clear"] field: FilterField) -> Result<(), Error> {
    let pool = &ctx.data().pool;
    let user_id = ctx.author().id.get() as i64;

    if db::profile(pool, user_id).await?.is_none() {
        ctx.say("You don't have a profile yet. Run `/profile setup` first.").await?;
        return Ok(());
    }

    let empty: Vec<String> = Vec::new();
    let update = match field {
        FilterField::Keywords => db::FilterUpdate { keywords: Some(&empty), ..Default::default() },
        FilterField::Exclude => db::FilterUpdate { exclude_keywords: Some(&empty), ..Default::default() },
        FilterField::Locations => db::FilterUpdate { locations: Some(&empty), ..Default::default() },
        FilterField::Seniority => db::FilterUpdate { seniority: Some(&empty), ..Default::default() },
    };
    db::set_profile_filters(pool, user_id, &update).await?;

    let reply = status_reply(ctx, user_id).await?;
    ctx.send(reply.content("Filter cleared.")).await?;
    Ok(())
}
