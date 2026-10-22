use crate::resume::compile::{self, CompileError};
use crate::resume::latex::{Document, SectionKind};
use crate::resume::{self, ResumeKnowledge};
use crate::{db, Context, Error};
use poise::reply::CreateReply;
use poise::serenity_prelude as serenity;
use serenity::{ChannelId, ChannelType, CreateAttachment, CreateEmbed, CreateMessage, Http};
use sqlx::PgPool;

const MAX_UPLOAD_BYTES: u32 = 200_000;
/// How many recent postings the gap report looks at.
const GAP_SAMPLE: i64 = 150;

/// Your master resume, tailored to each job you apply to
#[poise::command(slash_command, subcommands("upload", "show", "tailor", "gaps", "settings", "remove"), guild_only)]
pub async fn resume(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

fn summary_embed(doc: &Document, knowledge: &ResumeKnowledge, file_name: &str, pages: Option<u32>, max_pages: i16) -> CreateEmbed {
    let sections = doc
        .sections
        .iter()
        .map(|s| {
            let what = match s.kind {
                SectionKind::Projects => "reordered, trimmed to fit",
                SectionKind::Experience | SectionKind::Other => "bullets reordered, trimmed to fit",
                SectionKind::Skills => "matching skills moved first",
                SectionKind::Education => "left as is",
            };
            format!("**{}** · {what}", s.title)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let pinned = doc.bullets.iter().filter(|b| b.pinned).count() + doc.entries.iter().filter(|e| e.pinned).count();
    let categories = doc.skill_lines.iter().map(|l| l.category.as_str()).collect::<Vec<_>>().join(", ");
    let mut have: Vec<&String> = knowledge.have.iter().collect();
    have.sort_by_key(|s| s.to_lowercase());
    let have_text = if have.is_empty() {
        "*none recognised*".to_string()
    } else {
        let shown: Vec<&str> = have.iter().take(40).map(|s| s.as_str()).collect();
        let more = if have.len() > 40 { format!(" +{} more", have.len() - 40) } else { String::new() };
        format!("{}{more}", shown.join(", "))
    };
    let pages_text = match pages {
        Some(p) => format!("{p} page{} as uploaded; tailored copies are trimmed to {max_pages}", if p == 1 { "" } else { "s" }),
        None => format!("not compiled here; tailored copies target {max_pages} page(s)"),
    };

    CreateEmbed::new()
        .title(format!("Resume: {file_name}"))
        .colour(serenity::Colour::from(0x5865F2u32))
        .field("Sections", if sections.is_empty() { "*none found*".into() } else { resume::short(&sections, 1000) }, false)
        .field(
            "Found",
            format!(
                "{} entries, {} bullets, {} pinned with `@keep`\nSkill lines: {}",
                doc.entries.len(),
                doc.bullets.len(),
                pinned,
                if categories.is_empty() { "*none found*" } else { categories.as_str() }
            ),
            false,
        )
        .field("Skills detected", have_text, false)
        .field("Length", pages_text, false)
        .field(
            "Tips",
            "Keep *every* bullet and project you have in this file; each job gets the most relevant ones and the rest are \
             trimmed to fit. Add `% @keep` above a bullet or project to always include it, or `% @tags: FPGA, RTL` to give \
             it keywords that aren't in its text.",
            false,
        )
}

/// Upload your resume's LaTeX source (Jake's Resume template or similar)
#[poise::command(slash_command, guild_only)]
pub async fn upload(ctx: Context<'_>, #[description = "Your resume .tex file"] file: serenity::Attachment) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let pool = &ctx.data().pool;
    let user_id = ctx.author().id.get() as i64;

    if db::profile(pool, user_id).await?.is_none() {
        ctx.say("You don't have a profile yet. Run `/profile setup` first.").await?;
        return Ok(());
    }
    if !file.filename.to_lowercase().ends_with(".tex") {
        ctx.say("Upload the `.tex` source, not a PDF. On Overleaf: Menu → Download → Source, then pick the main .tex file.").await?;
        return Ok(());
    }
    if file.size > MAX_UPLOAD_BYTES {
        ctx.say(format!("That file is over {} KB; a resume .tex should be much smaller.", MAX_UPLOAD_BYTES / 1000)).await?;
        return Ok(());
    }

    let bytes = file.download().await?;
    let Ok(tex) = String::from_utf8(bytes) else {
        ctx.say("The file isn't UTF-8 text.").await?;
        return Ok(());
    };
    if let Err(reason) = compile::check_safe(&tex) {
        ctx.say(reason).await?;
        return Ok(());
    }
    let doc = match Document::parse(&tex) {
        Ok(doc) => doc,
        Err(reason) => {
            ctx.say(reason).await?;
            return Ok(());
        }
    };
    if doc.bullets.is_empty() {
        ctx.say("I couldn't find any `\\resumeItem{...}` bullets. Tailoring works with Jake's Resume template and others that use the same macros.").await?;
        return Ok(());
    }

    let mut warning = None;
    let pages = match compile::compile(&tex).await {
        Ok(c) => Some(c.pages),
        Err(e @ (CompileError::NotInstalled | CompileError::NoSandbox | CompileError::Sandbox(_))) => {
            warning = Some(format!("Tailored resumes will come as .tex only for now: {e}."));
            None
        }
        Err(e) => {
            ctx.say(format!("Your resume doesn't compile, so it wasn't saved.\n```\n{e}\n```")).await?;
            return Ok(());
        }
    };

    db::save_resume(pool, user_id, &file.filename, &tex).await?;
    let stored = db::resume(pool, user_id).await?;
    let max_pages = stored.map_or(1, |r| r.max_pages);

    let knowledge = ResumeKnowledge::new(&doc);
    let mut reply = CreateReply::default()
        .content("Saved. Clicking **Apply** on a job now posts a resume tailored to it in the thread.")
        .embed(summary_embed(&doc, &knowledge, &file.filename, pages, max_pages));
    if let Some(w) = warning {
        reply = reply.content(format!("Saved. {w}"));
    }
    ctx.send(reply).await?;
    Ok(())
}

/// Show what the bot understood from your resume
#[poise::command(slash_command, guild_only)]
pub async fn show(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let user_id = ctx.author().id.get() as i64;
    let Some(stored) = db::resume(&ctx.data().pool, user_id).await? else {
        ctx.say("No resume yet. Upload your .tex with `/resume upload`.").await?;
        return Ok(());
    };
    let doc = Document::parse(&stored.tex)?;
    let knowledge = ResumeKnowledge::new(&doc);
    let auto = if stored.auto_tailor { "on" } else { "off" };
    ctx.send(
        CreateReply::default()
            .content(format!("Auto-tailor on Apply: **{auto}** (change with `/resume settings`)."))
            .embed(summary_embed(&doc, &knowledge, &stored.file_name, None, stored.max_pages))
            .attachment(CreateAttachment::bytes(stored.tex.into_bytes(), stored.file_name)),
    )
    .await?;
    Ok(())
}

/// Builds the tailored resume and its report. Shared by `/resume tailor` and the Apply button.
async fn build(client: &reqwest::Client, stored: &db::StoredResume, job: &db::JobDetail, display_name: &str, max_pages: Option<u8>) -> Result<(CreateEmbed, Vec<CreateAttachment>), String> {
    let pages = max_pages.map(u32::from).unwrap_or(stored.max_pages.max(1) as u32);
    // Gemini when a key is configured; the built-in scorer otherwise, or if Gemini fails.
    let tailored = if resume::gemini::enabled() {
        match resume::gemini::tailor(client, &stored.tex, &job.job_title, &job.page_content, pages).await {
            Ok(t) => t,
            Err(reason) => {
                eprintln!("Gemini tailoring failed, using built-in: {reason}");
                let mut t = resume::tailor(&stored.tex, &job.job_title, &job.page_content, pages).await?;
                t.notes.insert(0, format!("Gemini failed ({}), so this used the built-in tailoring.", resume::short(&reason, 300)));
                t
            }
        }
    } else {
        resume::tailor(&stored.tex, &job.job_title, &job.page_content, pages).await?
    };
    let stem = resume::file_stem(display_name);
    let embed = resume::report_embed(&tailored, &format!("{} @ {}", job.job_title, job.company_name)).url(&job.url);
    let mut files = Vec::new();
    if let Some(pdf) = tailored.pdf {
        files.push(CreateAttachment::bytes(pdf, format!("{stem}.pdf")));
    }
    files.push(CreateAttachment::bytes(tailored.tex.into_bytes(), format!("{stem}.tex")));
    Ok((embed, files))
}

/// Posts a tailored resume into a job thread. Used when Apply is clicked.
pub async fn tailor_into_thread(http: &Http, pool: &PgPool, client: &reqwest::Client, thread: ChannelId, user_id: i64, display_name: &str, job: &db::JobDetail) {
    let stored = match db::resume(pool, user_id).await {
        Ok(Some(r)) if r.auto_tailor => r,
        Ok(_) => return,
        Err(e) => {
            eprintln!("Loading resume for {user_id} failed: {e}");
            return;
        }
    };
    let message = match build(client, &stored, job, display_name, None).await {
        Ok((embed, files)) => CreateMessage::new().embed(embed).add_files(files),
        Err(reason) => CreateMessage::new().content(format!("Couldn't tailor your resume: {reason}")),
    };
    if let Err(e) = thread.send_message(http, message).await {
        eprintln!("Posting tailored resume failed: {e}");
    }
}

/// Tailor your resume to a job (run inside a job thread, or give the job's URL)
#[poise::command(slash_command, guild_only)]
pub async fn tailor(
    ctx: Context<'_>,
    #[description = "Job posting URL (not needed inside a job thread)"] url: Option<String>,
    #[description = "Page limit for this copy (default from /resume settings)"]
    #[min = 1]
    #[max = 3]
    pages: Option<u8>,
) -> Result<(), Error> {
    let pool = &ctx.data().pool;
    let user_id = ctx.author().id.get() as i64;

    // Acknowledge right away (Discord allows 3s): results are public inside a
    // job thread and only visible to you anywhere else.
    let in_thread = matches!(ctx.guild_channel().await, Some(c) if c.kind == ChannelType::PublicThread);
    if in_thread && url.is_none() {
        ctx.defer().await?;
    } else {
        ctx.defer_ephemeral().await?;
    }
    let thread_job = if in_thread { db::job_for_message(pool, ctx.channel_id().get()).await? } else { None };

    let job = match (url, thread_job) {
        (Some(url), _) => match db::job_by_url(pool, &url).await? {
            Some(job) => job,
            None => {
                ctx.say("That URL isn't a job the bot has stored. Use the exact link from a job alert.").await?;
                return Ok(());
            }
        },
        (None, Some((job, _))) => job,
        (None, None) => {
            ctx.say("Run this inside a job's thread, or pass the job's `url`.").await?;
            return Ok(());
        }
    };

    let Some(stored) = db::resume(pool, user_id).await? else {
        ctx.say("No resume yet. Upload your .tex with `/resume upload`.").await?;
        return Ok(());
    };

    let author = ctx.author();
    let display_name = author.global_name.clone().unwrap_or_else(|| author.name.clone());
    match build(&ctx.data().client, &stored, &job, &display_name, pages).await {
        Ok((embed, files)) => {
            let mut reply = CreateReply::default().embed(embed);
            for file in files {
                reply = reply.attachment(file);
            }
            ctx.send(reply).await?;
        }
        Err(reason) => {
            ctx.say(format!("Couldn't tailor your resume: {reason}")).await?;
        }
    }
    Ok(())
}

/// Skills your recent job matches ask for most, and which ones your resume is missing
#[poise::command(slash_command, guild_only)]
pub async fn gaps(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let pool = &ctx.data().pool;
    let user_id = ctx.author().id.get() as i64;

    let Some(stored) = db::resume(pool, user_id).await? else {
        ctx.say("No resume yet. Upload your .tex with `/resume upload`.").await?;
        return Ok(());
    };
    let postings = db::recent_posted_jobs(pool, user_id, GAP_SAMPLE).await?;
    if postings.len() < 3 {
        ctx.say("Not enough job alerts yet; check back after a few more have been posted to you.").await?;
        return Ok(());
    }

    let knowledge = ResumeKnowledge::from_tex(&stored.tex)?;
    let rows = resume::gaps(&knowledge, &postings);
    let total = postings.len();
    let pct = |n: usize| (n * 100 + total / 2) / total;

    let missing: Vec<String> = rows
        .iter()
        .filter(|r| !r.have && pct(r.postings) >= 5)
        .take(12)
        .map(|r| format!("**{}** · {}% of postings", r.skill, pct(r.postings)))
        .collect();
    let strong: Vec<String> = rows.iter().filter(|r| r.have).take(10).map(|r| format!("{} · {}%", r.skill, pct(r.postings))).collect();

    let embed = CreateEmbed::new()
        .title("Skill gaps across your job matches")
        .description(format!("Based on the last {total} postings sent to you (ignored ones excluded)."))
        .colour(serenity::Colour::from(0xFEE75Cu32))
        .field(
            "Most requested, not on your resume",
            if missing.is_empty() { "*nothing major; your resume covers what these jobs ask for*".to_string() } else { missing.join("\n") },
            false,
        )
        .field("Your best-covered skills", if strong.is_empty() { "*none*".to_string() } else { strong.join("\n") }, false)
        .field(
            "What to do with this",
            "If you already have one of the missing skills, add it to a bullet or your skills line, since applicant \
             tracking systems often filter on exact keywords. If you don't, the top few are worth a weekend project.",
            false,
        );
    ctx.send(CreateReply::default().embed(embed)).await?;
    Ok(())
}

/// Change the page limit or turn automatic tailoring on/off
#[poise::command(slash_command, guild_only)]
pub async fn settings(
    ctx: Context<'_>,
    #[description = "Maximum pages for tailored copies"]
    #[min = 1]
    #[max = 3]
    pages: Option<u8>,
    #[description = "Post a tailored resume automatically when you click Apply"] auto_tailor: Option<bool>,
) -> Result<(), Error> {
    let user_id = ctx.author().id.get() as i64;
    if pages.is_none() && auto_tailor.is_none() {
        ctx.send(CreateReply::default().ephemeral(true).content("Give `pages` and/or `auto_tailor`.")).await?;
        return Ok(());
    }
    let updated = db::set_resume_settings(&ctx.data().pool, user_id, pages.map(i16::from), auto_tailor).await?;
    let text = if updated { "Resume settings updated." } else { "No resume yet. Upload your .tex with `/resume upload`." };
    ctx.send(CreateReply::default().ephemeral(true).content(text)).await?;
    Ok(())
}

/// Delete your stored resume
#[poise::command(slash_command, guild_only)]
pub async fn remove(ctx: Context<'_>) -> Result<(), Error> {
    let removed = db::delete_resume(&ctx.data().pool, ctx.author().id.get() as i64).await?;
    let text = if removed { "Your resume was deleted." } else { "You don't have a resume stored." };
    ctx.send(CreateReply::default().ephemeral(true).content(text)).await?;
    Ok(())
}
