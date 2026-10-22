//! Resume tailoring: picks and orders the user's own bullets, projects and
//! skills for a specific posting, then fits the result onto the page limit.
//! Nothing is invented; content is only reordered or left out.

pub mod compile;
pub mod gemini;
pub mod latex;
pub mod skills;

use compile::{CompileError, Compiled};
use latex::{Document, Plan, SectionKind};
use skills::{Importance, PostingSkill, Vocabulary};
use std::collections::{HashMap, HashSet};

/// What the user's master resume covers, prepared once and reused per job.
pub struct ResumeKnowledge {
    pub vocab: Vocabulary,
    pub have: HashSet<String>,
}

impl ResumeKnowledge {
    pub fn new(doc: &Document) -> Self {
        let vocab = Vocabulary::new(&doc.listed_skills());
        let mut have = vocab.find(&doc.body_text());
        for raw in doc.listed_skills() {
            if let Some(name) = vocab.canonical(&raw) {
                have.insert(name.to_string());
            }
        }
        ResumeKnowledge { vocab, have }
    }

    pub fn from_tex(tex: &str) -> Result<Self, String> {
        Ok(Self::new(&Document::parse(tex)?))
    }
}

/// How well a resume covers one posting.
#[derive(Debug, Clone)]
pub struct JobMatch {
    /// Weighted share of the posting's skills that the resume mentions, 0-100.
    /// `None` when the posting names no recognisable skills.
    pub score: Option<u8>,
    pub matched: Vec<PostingSkill>,
    pub missing: Vec<PostingSkill>,
}

fn coverage(posting: &[PostingSkill], have: &HashSet<String>) -> JobMatch {
    let (matched, missing): (Vec<_>, Vec<_>) = posting.iter().cloned().partition(|s| have.contains(&s.name));
    let total: f32 = posting.iter().map(|s| s.weight).sum();
    let got: f32 = matched.iter().map(|s| s.weight).sum();
    let score = (total > 0.0).then(|| ((got / total) * 100.0).round().clamp(0.0, 100.0) as u8);
    JobMatch { score, matched, missing }
}

/// Quick match used on job alerts; no LaTeX involved.
pub fn match_job(knowledge: &ResumeKnowledge, title: &str, body: &str) -> JobMatch {
    coverage(&skills::extract(&knowledge.vocab, title, body), &knowledge.have)
}

struct Scorer<'a> {
    vocab: &'a Vocabulary,
    weights: HashMap<String, f32>,
    words: HashSet<String>,
    phrases: HashSet<String>,
}

impl<'a> Scorer<'a> {
    fn new(vocab: &'a Vocabulary, posting: &[PostingSkill], title: &str, body: &str) -> Self {
        let text = format!("{title}\n{body}");
        Scorer {
            vocab,
            weights: posting.iter().map(|s| (s.name.clone(), s.weight)).collect(),
            words: skills::significant_words(&text),
            phrases: skills::significant_phrases(&text),
        }
    }

    fn skills_in(&self, text: &str, tags: &[String]) -> HashSet<String> {
        let mut found = self.vocab.find(text);
        for tag in tags {
            found.insert(self.vocab.canonical(tag).map(str::to_string).unwrap_or_else(|| tag.clone()));
        }
        found
    }

    fn skill_weight(&self, found: &HashSet<String>) -> f32 {
        found.iter().filter_map(|s| self.weights.get(s)).sum()
    }

    fn overlap(&self, text: &str, cap: usize) -> f32 {
        skills::significant_words(text).intersection(&self.words).count().min(cap) as f32
    }

    fn phrase_overlap(&self, text: &str, cap: usize) -> f32 {
        skills::significant_phrases(text).intersection(&self.phrases).count().min(cap) as f32
    }

    /// `anchor` marks a list's original first bullet, which usually says what
    /// the role or project is; it gets a nudge so it isn't trimmed before
    /// bullets that only make sense after it.
    fn bullet(&self, text: &str, tags: &[String], anchor: bool) -> f32 {
        let quantified = if text.chars().any(|c| c.is_ascii_digit()) { 0.25 } else { 0.0 };
        let anchor = if anchor { 0.5 } else { 0.0 };
        self.skill_weight(&self.skills_in(text, tags)) + 0.35 * self.overlap(text, 4) + 0.75 * self.phrase_overlap(text, 3) + quantified + anchor
    }
}

#[derive(Debug, Clone, Copy)]
enum Unit {
    Bullet(usize),
    Entry(usize),
}

struct Layout {
    plan: Plan,
    bullet_scores: Vec<f32>,
    removals: Vec<Unit>,
}

fn layout(doc: &Document, scorer: &Scorer) -> Layout {
    let bullet_scores: Vec<f32> = doc
        .bullets
        .iter()
        .enumerate()
        .map(|(i, b)| scorer.bullet(&b.text, &b.tags, doc.lists[b.list].first() == Some(&i)))
        .collect();

    let entry_scores: Vec<f32> = doc
        .entries
        .iter()
        .map(|e| {
            let bullets: Vec<usize> = e.lists.iter().flat_map(|&l| doc.lists[l].iter().copied()).collect();
            let mut text = e.heading.clone();
            let mut tags = e.tags.clone();
            for &b in &bullets {
                text.push(' ');
                text.push_str(&doc.bullets[b].text);
                tags.extend(doc.bullets[b].tags.iter().cloned());
            }
            scorer.skill_weight(&scorer.skills_in(&text, &tags)) + 0.2 * scorer.overlap(&text, 8) + 0.5 * scorer.phrase_overlap(&text, 4)
        })
        .collect();

    let by_score_desc = |scores: &[f32], ids: &mut [usize]| {
        ids.sort_by(|a, b| scores[*b].total_cmp(&scores[*a])); // stable: ties keep document order
    };

    let mut plan = doc.identity_plan();
    let anchored = anchored_lists(doc);
    for (l, ids) in plan.list_order.iter_mut().enumerate() {
        // A project's first bullet introduces it ("Designed a CPU…"); later
        // ones often lean on it ("Verified it…"), so it stays on top.
        let skip = usize::from(anchored[l]).min(ids.len());
        by_score_desc(&bullet_scores, &mut ids[skip..]);
    }
    for (s, section) in doc.sections.iter().enumerate() {
        if section.kind == SectionKind::Projects {
            by_score_desc(&entry_scores, plan.entry_order[s].as_mut_slice());
        }
    }
    for (i, line) in doc.skill_lines.iter().enumerate() {
        let weight_of = |item: &String| -> f32 {
            let text = latex::plain_text(item);
            let base = text.split('(').next().unwrap_or(&text).trim().to_string();
            std::iter::once(base.clone())
                .chain(base.split('/').map(|p| p.trim().to_string()))
                .filter_map(|p| scorer.vocab.canonical(&p).map(str::to_string).or(Some(p)))
                .filter_map(|name| scorer.weights.get(&name).copied())
                .fold(0.0, f32::max)
        };
        let mut items = line.items.clone();
        items.sort_by(|a, b| weight_of(b).total_cmp(&weight_of(a)));
        plan.skill_order[i] = items;
    }

    let removals = removal_sequence(doc, &bullet_scores, &entry_scores);
    Layout { plan, bullet_scores, removals }
}

/// Bullet lists under a project heading, whose first bullet stays first.
fn anchored_lists(doc: &Document) -> Vec<bool> {
    let mut out = vec![false; doc.lists.len()];
    for entry in &doc.entries {
        if doc.sections[entry.section].kind == SectionKind::Projects {
            for &l in &entry.lists {
                out[l] = true;
            }
        }
    }
    out
}

/// Least relevant first. Each experience entry keeps at least two bullets (or
/// all it had, if fewer), each project at least one (its first), and at least
/// two projects stay. `@keep` items and Education are never removed. Among
/// similar scores, longer lists are trimmed first.
fn removal_sequence(doc: &Document, bullet_scores: &[f32], entry_scores: &[f32]) -> Vec<Unit> {
    let anchored = anchored_lists(doc);
    // list -> (owning entry, section kind)
    let mut owner: Vec<(Option<usize>, Option<SectionKind>)> = vec![(None, None); doc.lists.len()];
    for (e, entry) in doc.entries.iter().enumerate() {
        for &l in &entry.lists {
            owner[l] = (Some(e), Some(doc.sections[entry.section].kind));
        }
    }
    for section in &doc.sections {
        for &l in &section.loose_lists {
            owner[l] = (None, Some(section.kind));
        }
    }

    let mut kept_in_list: Vec<usize> = doc.lists.iter().map(Vec::len).collect();
    let mut kept_in_section: Vec<usize> = doc.sections.iter().map(|s| s.entries.len()).collect();
    let mut bullet_gone = vec![false; doc.bullets.len()];
    let mut entry_gone = vec![false; doc.entries.len()];
    let mut out = Vec::new();

    loop {
        // Ordering key: relevance band (half-point buckets, so tiny score
        // differences don't decide), then whole projects before single bullets,
        // then the most crowded list, then later in the document.
        type Key = (i32, u8, std::cmp::Reverse<usize>, std::cmp::Reverse<usize>);
        let band = |score: f32| (score * 2.0).floor() as i32;
        let mut best: Option<(Key, Unit)> = None;
        let mut consider = |key: Key, unit: Unit| {
            if best.as_ref().is_none_or(|(k, _)| key < *k) {
                best = Some((key, unit));
            }
        };

        for (b, bullet) in doc.bullets.iter().enumerate() {
            if bullet_gone[b] || bullet.pinned {
                continue;
            }
            if anchored[bullet.list] && doc.lists[bullet.list].first() == Some(&b) {
                continue;
            }
            let (entry, kind) = owner[bullet.list];
            let Some(kind) = kind else { continue };
            if matches!(kind, SectionKind::Education | SectionKind::Skills) || entry.is_some_and(|e| entry_gone[e]) {
                continue;
            }
            let original = doc.lists[bullet.list].len();
            let min_keep = if kind == SectionKind::Projects { 1 } else { original.min(2) };
            let kept = kept_in_list[bullet.list];
            if kept > min_keep.max(1) {
                let key = (band(bullet_scores[b]), 1, std::cmp::Reverse(kept), std::cmp::Reverse(b));
                consider(key, Unit::Bullet(b));
            }
        }
        for (e, entry) in doc.entries.iter().enumerate() {
            let section = &doc.sections[entry.section];
            if entry_gone[e] || entry.pinned || section.kind != SectionKind::Projects {
                continue;
            }
            if kept_in_section[entry.section] > section.entries.len().min(2) {
                consider((band(entry_scores[e]), 0, std::cmp::Reverse(0), std::cmp::Reverse(e)), Unit::Entry(e));
            }
        }

        let Some((_, unit)) = best else { break };
        match unit {
            Unit::Bullet(b) => {
                bullet_gone[b] = true;
                kept_in_list[doc.bullets[b].list] -= 1;
            }
            Unit::Entry(e) => {
                entry_gone[e] = true;
                kept_in_section[doc.entries[e].section] -= 1;
            }
        }
        out.push(unit);
    }
    out
}

fn plan_without(base: &Plan, removed: &[Unit]) -> Plan {
    let bullets: HashSet<usize> = removed.iter().filter_map(|u| if let Unit::Bullet(b) = u { Some(*b) } else { None }).collect();
    let entries: HashSet<usize> = removed.iter().filter_map(|u| if let Unit::Entry(e) = u { Some(*e) } else { None }).collect();
    let mut plan = base.clone();
    for ids in plan.list_order.iter_mut() {
        ids.retain(|b| !bullets.contains(b));
    }
    for ids in plan.entry_order.iter_mut() {
        ids.retain(|e| !entries.contains(e));
    }
    plan
}

#[derive(Debug, Clone)]
pub struct Report {
    pub before: JobMatch,
    pub after: JobMatch,
    /// Posting skills that the master resume has but the tailored one lost to space.
    pub cut: Vec<String>,
    pub dropped_bullets: usize,
    pub dropped_projects: Vec<String>,
    pub project_order: Vec<String>,
    /// "Entry: first bullet" for entries whose lead bullet changed.
    pub new_leads: Vec<String>,
}

pub struct Tailored {
    pub tex: String,
    pub pdf: Option<Vec<u8>>,
    pub pages: Option<u32>,
    pub report: Report,
    pub notes: Vec<String>,
    /// Model name when an AI did the tailoring instead of the built-in scorer.
    pub by_ai: Option<String>,
}

pub(crate) fn short(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        format!("{}…", text.chars().take(max - 1).collect::<String>().trim_end())
    }
}

fn entry_label(doc: &Document, e: usize) -> String {
    short(&doc.entries[e].label, 60)
}

fn report(doc: &Document, knowledge: &ResumeKnowledge, posting: &[PostingSkill], layout: &Layout, plan: &Plan, removed: &[Unit]) -> Report {
    let rendered = doc.render(plan);
    let body = rendered.find("\\begin{document}").map_or(rendered.as_str(), |i| &rendered[i..]);
    let mut have_after = knowledge.vocab.find(&latex::plain_text(body));

    let kept_entries: HashSet<usize> = plan.entry_order.iter().flatten().copied().collect();
    let entry_of_list: HashMap<usize, usize> =
        doc.entries.iter().enumerate().flat_map(|(e, en)| en.lists.iter().map(move |&l| (l, e))).collect();
    let mut kept_tags: Vec<&String> = doc.entries.iter().enumerate().filter(|(e, _)| kept_entries.contains(e)).flat_map(|(_, en)| &en.tags).collect();
    for ids in &plan.list_order {
        for &b in ids {
            let bullet = &doc.bullets[b];
            if entry_of_list.get(&bullet.list).is_none_or(|e| kept_entries.contains(e)) {
                kept_tags.extend(&bullet.tags);
            }
        }
    }
    for tag in kept_tags {
        have_after.insert(knowledge.vocab.canonical(tag).map(str::to_string).unwrap_or_else(|| tag.clone()));
    }

    let before = coverage(posting, &knowledge.have);
    let after = coverage(posting, &have_after);
    let cut = before.matched.iter().filter(|s| !have_after.contains(&s.name)).map(|s| s.name.clone()).collect();

    let dropped_projects = removed.iter().filter_map(|u| if let Unit::Entry(e) = u { Some(entry_label(doc, *e)) } else { None }).collect();
    let dropped_bullets = removed.iter().filter(|u| matches!(u, Unit::Bullet(_))).count();

    let project_order = doc
        .sections
        .iter()
        .enumerate()
        .filter(|(_, s)| s.kind == SectionKind::Projects)
        .flat_map(|(i, _)| plan.entry_order[i].iter().map(|&e| entry_label(doc, e)))
        .collect();

    let mut new_leads = Vec::new();
    for (e, entry) in doc.entries.iter().enumerate() {
        if !kept_entries.contains(&e) {
            continue;
        }
        for &l in &entry.lists {
            let (Some(&orig), Some(&now)) = (doc.lists[l].first(), plan.list_order[l].first()) else { continue };
            if orig != now && layout.bullet_scores[now] > 0.5 {
                new_leads.push(format!("**{}**: {}", entry_label(doc, e), short(&doc.bullets[now].text, 90)));
            }
        }
    }

    Report { before, after, cut, dropped_bullets, dropped_projects, project_order, new_leads }
}

enum Fit {
    /// Fewest removals that fit, with that compile.
    Fits(usize, Compiled),
    /// Every allowed removal applied and it still doesn't fit.
    StillTooLong(usize, Compiled),
    NothingToRemove,
}

/// Binary search for the fewest removals (out of `n`, least relevant first)
/// that bring the page count within `max_pages`. Takes about log2(n) compiles.
async fn fit(render: &impl Fn(usize) -> String, n: usize, max_pages: u32) -> Result<Fit, CompileError> {
    if n == 0 {
        return Ok(Fit::NothingToRemove);
    }
    let all = compile::compile(&render(n)).await?;
    if all.pages > max_pages {
        return Ok(Fit::StillTooLong(n, all));
    }
    let (mut lo, mut hi, mut best) = (1usize, n, all);
    while lo < hi {
        let mid = (lo + hi) / 2;
        let attempt = compile::compile(&render(mid)).await?;
        if attempt.pages <= max_pages {
            hi = mid;
            best = attempt;
        } else {
            lo = mid + 1;
        }
    }
    Ok(Fit::Fits(hi, best))
}

/// Tailors `master` for one posting and compiles it to fit `max_pages`.
pub async fn tailor(master: &str, title: &str, body: &str, max_pages: u32) -> Result<Tailored, String> {
    compile::check_safe(master)?;
    let doc = Document::parse(master)?;
    let knowledge = ResumeKnowledge::new(&doc);
    let posting = skills::extract(&knowledge.vocab, title, body);
    let scorer = Scorer::new(&knowledge.vocab, &posting, title, body);
    let layout = layout(&doc, &scorer);
    let render = |k: usize| doc.render(&plan_without(&layout.plan, &layout.removals[..k]));

    let mut notes = Vec::new();
    let mut k = 0usize;
    let mut compiled: Option<Compiled> = None;

    match compile::compile(&render(0)).await {
        Ok(c) if c.pages <= max_pages => compiled = Some(c),
        Ok(c) => match fit(&render, layout.removals.len(), max_pages).await {
            Ok(Fit::Fits(removals, best)) => {
                k = removals;
                compiled = Some(best);
            }
            Ok(Fit::StillTooLong(removals, best)) => {
                notes.push(format!(
                    "Even after removing everything allowed it's {} pages. Mark fewer items `% @keep` or shorten the header/education.",
                    best.pages
                ));
                k = removals;
                compiled = Some(best);
            }
            Ok(Fit::NothingToRemove) => {
                notes.push(format!("It's {} pages and there's nothing left that's allowed to be removed.", c.pages));
                compiled = Some(c);
            }
            Err(e) => {
                notes.push(format!("Couldn't trim it to fit, so it's {} pages. ({e})", c.pages));
                compiled = Some(c);
            }
        },
        Err(e @ (CompileError::NotInstalled | CompileError::NoSandbox | CompileError::Sandbox(_))) => {
            notes.push(format!("Only the tailored .tex is attached, nothing was trimmed to fit: {e}."));
        }
        Err(e @ (CompileError::Latex(_) | CompileError::Timeout)) => {
            // Make sure it's the tailoring, not the master, that broke.
            return match compile::compile(master).await {
                Ok(original) => Ok(Tailored {
                    tex: master.to_string(),
                    pages: Some(original.pages),
                    pdf: Some(original.pdf),
                    report: report(&doc, &knowledge, &posting, &layout, &doc.identity_plan(), &[]),
                    notes: vec![format!("The tailored version didn't compile, so this is your original resume. ({e})")],
                    by_ai: None,
                }),
                Err(master_err) => Err(format!("Your uploaded resume doesn't compile: {master_err}")),
            };
        }
        Err(e) => return Err(e.to_string()),
    }

    let removed = &layout.removals[..k];
    let plan = plan_without(&layout.plan, removed);
    let tex = doc.render(&plan);
    let report = report(&doc, &knowledge, &posting, &layout, &plan, removed);
    if posting.is_empty() {
        notes.push("The posting doesn't name any skills the bot recognises, so the order is mostly unchanged.".into());
    }
    Ok(Tailored { tex, pages: compiled.as_ref().map(|c| c.pages), pdf: compiled.map(|c| c.pdf), report, notes, by_ai: None })
}

/// The resume bullet most relevant to a posting, for outreach messages.
#[derive(Debug, Clone)]
pub struct Highlight {
    /// Company (for experience) or project name.
    pub org: String,
    pub is_project: bool,
    pub text: String,
}

/// What an outreach message can truthfully say about the sender for one job.
#[derive(Debug, Clone, Default)]
pub struct OutreachFacts {
    pub name: Option<String>,
    pub school: Option<String>,
    /// Posting skills the resume has, most important first.
    pub skills: Vec<String>,
    pub highlight: Option<Highlight>,
}

pub fn outreach_facts(master: &str, title: &str, body: &str) -> Result<OutreachFacts, String> {
    let doc = Document::parse(master)?;
    let knowledge = ResumeKnowledge::new(&doc);
    let posting = skills::extract(&knowledge.vocab, title, body);
    let scorer = Scorer::new(&knowledge.vocab, &posting, title, body);

    let mut best: Option<(f32, Highlight)> = None;
    for entry in &doc.entries {
        let kind = doc.sections[entry.section].kind;
        if !matches!(kind, SectionKind::Experience | SectionKind::Projects | SectionKind::Other) {
            continue;
        }
        for &l in &entry.lists {
            for &b in &doc.lists[l] {
                let bullet = &doc.bullets[b];
                let score = scorer.bullet(&bullet.text, &bullet.tags, false);
                if score > 0.5 && best.as_ref().is_none_or(|(s, _)| score > *s) {
                    let highlight = Highlight { org: entry.org.clone(), is_project: kind == SectionKind::Projects, text: bullet.text.clone() };
                    best = Some((score, highlight));
                }
            }
        }
    }

    Ok(OutreachFacts {
        name: doc.person_name(),
        school: doc.school(),
        skills: coverage(&posting, &knowledge.have).matched.into_iter().map(|s| s.name).take(3).collect(),
        highlight: best.map(|(_, h)| h),
    })
}

/// Skills that show up across many postings, and whether the resume has them.
pub struct GapRow {
    pub skill: String,
    pub postings: usize,
    pub have: bool,
}

pub fn gaps(knowledge: &ResumeKnowledge, postings: &[(String, String)]) -> Vec<GapRow> {
    let extracted: Vec<Vec<PostingSkill>> = postings
        .iter()
        .map(|(title, body)| {
            skills::extract(&knowledge.vocab, title, body)
                .into_iter()
                .filter(|s| s.importance != Importance::Preferred || s.weight >= 2.0)
                .collect()
        })
        .collect();
    let mut rows: Vec<GapRow> = skills::tally(&extracted)
        .into_iter()
        .map(|(skill, postings)| GapRow { have: knowledge.have.contains(&skill), skill, postings })
        .collect();
    rows.sort_by(|a, b| b.postings.cmp(&a.postings).then_with(|| a.skill.cmp(&b.skill)));
    rows
}

/// A tidy attachment name like "Jane_Doe_Resume.pdf".
pub fn file_stem(display_name: &str) -> String {
    let cleaned: String = display_name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>()
        .split('_')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    if cleaned.is_empty() { "Resume".to_string() } else { format!("{cleaned}_Resume") }
}

fn skill_list(skills: &[PostingSkill], limit: usize, mark_required: bool) -> String {
    if skills.is_empty() {
        return "*none*".to_string();
    }
    let mut parts: Vec<String> = skills
        .iter()
        .take(limit)
        .map(|s| if mark_required && s.importance == Importance::Required { format!("**{}**", s.name) } else { s.name.clone() })
        .collect();
    if skills.len() > limit {
        parts.push(format!("+{} more", skills.len() - limit));
    }
    parts.join(", ")
}

pub fn score_text(m: &JobMatch) -> String {
    match m.score {
        Some(s) => format!("{s}%"),
        None => "n/a".to_string(),
    }
}

/// Discord summary of a tailoring run.
pub fn report_embed(t: &Tailored, job_title: &str) -> poise::serenity_prelude::CreateEmbed {
    use poise::serenity_prelude::{Colour, CreateEmbed};
    let r = &t.report;
    let mut embed = CreateEmbed::new()
        .title(short(&format!("Tailored resume · {job_title}"), 250))
        .colour(Colour::from(0x57F287u32))
        .field("Keyword match", format!("{} of the posting's skills (your full resume: {})", score_text(&r.after), score_text(&r.before)), false)
        .field("Matched", skill_list(&r.after.matched, 15, true), false);

    let missing_text = if r.before.missing.is_empty() {
        "*none*, your resume covers everything it asks for".to_string()
    } else {
        format!("{}\n-# Bold = required. Only add these if you've actually used them.", skill_list(&r.before.missing, 12, true))
    };
    embed = embed.field("Not on your resume", missing_text, false);

    if !r.cut.is_empty() {
        embed = embed.field("Lost to the page limit", short(&r.cut.join(", "), 1000), false);
    }
    if !r.new_leads.is_empty() {
        embed = embed.field("Now leading with", short(&r.new_leads.iter().take(4).cloned().collect::<Vec<_>>().join("\n"), 1000), false);
    }
    if r.project_order.len() > 1 {
        embed = embed.field("Project order", short(&r.project_order.join(" → "), 1000), false);
    }
    let mut trimmed = Vec::new();
    if r.dropped_bullets > 0 {
        trimmed.push(format!("{} bullet{}", r.dropped_bullets, if r.dropped_bullets == 1 { "" } else { "s" }));
    }
    if !r.dropped_projects.is_empty() {
        trimmed.push(format!("project{} {}", if r.dropped_projects.len() == 1 { "" } else { "s" }, r.dropped_projects.join(", ")));
    }
    if !trimmed.is_empty() {
        embed = embed.field("Removed to fit", short(&trimmed.join("; "), 1000), false);
    }
    if !t.notes.is_empty() {
        embed = embed.field("Notes", short(&t.notes.join("\n"), 1000), false);
    }
    let pages = t.pages.map(|p| format!("{p} page{}", if p == 1 { "" } else { "s" })).unwrap_or_else(|| "not compiled".into());
    let how = match &t.by_ai {
        Some(model) => format!("tailored by {model} from your own content"),
        None => "reordered from your own content, nothing invented".to_string(),
    };
    embed.footer(poise::serenity_prelude::CreateEmbedFooter::new(format!("{pages} · {how}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use latex::tests::SAMPLE;

    fn fpga_posting() -> (&'static str, &'static str) {
        (
            "FPGA Design Intern",
            "You will design RTL for our FPGA products.\nRequirements: SystemVerilog, experience with Vivado, Python scripting.\nNice to have: UVM, Perl.",
        )
    }

    #[test]
    fn orders_by_relevance() {
        let doc = Document::parse(SAMPLE).unwrap();
        let knowledge = ResumeKnowledge::new(&doc);
        let (title, body) = fpga_posting();
        let posting = skills::extract(&knowledge.vocab, title, body);
        let scorer = Scorer::new(&knowledge.vocab, &posting, title, body);
        let l = layout(&doc, &scorer);
        let out = doc.render(&l.plan);
        assert!(out.find("RISC-V Core").unwrap() < out.find("Portfolio Site").unwrap(), "relevant project first");
        assert!(out.contains("{: Python, SystemVerilog, C/C++, JavaScript}"), "matched skills first, ties keep order:\n{out}");
        assert!(out.contains("{: Vivado, Git, Docker}"));

        // React bullet goes first when trimming; the pinned bullet never does.
        assert!(matches!(l.removals.first(), Some(Unit::Entry(_)) | Some(Unit::Bullet(0))));
        assert!(!l.removals.iter().any(|u| matches!(u, Unit::Bullet(1))));
    }

    #[test]
    fn removal_respects_minimums() {
        let doc = Document::parse(SAMPLE).unwrap();
        let knowledge = ResumeKnowledge::new(&doc);
        let scorer = Scorer::new(&knowledge.vocab, &[], "", "");
        let l = layout(&doc, &scorer);
        let plan = plan_without(&l.plan, &l.removals);
        // Experience keeps 2 of 3 bullets, both projects stay (minimum of two).
        assert_eq!(plan.list_order[0].len(), 2);
        assert_eq!(plan.entry_order[2].len(), 2);
        let again = Document::parse(&doc.render(&plan)).unwrap();
        assert!(again.lists.iter().all(|l| !l.is_empty()));
    }

    #[test]
    fn match_scores() {
        let knowledge = ResumeKnowledge::from_tex(SAMPLE).unwrap();
        let (title, body) = fpga_posting();
        let m = match_job(&knowledge, title, body);
        let missing: Vec<&str> = m.missing.iter().map(|s| s.name.as_str()).collect();
        assert!(missing.contains(&"UVM") || m.matched.iter().any(|s| s.name == "UVM"));
        assert!(missing.contains(&"Perl"));
        assert!(m.matched.iter().any(|s| s.name == "SystemVerilog"));
        assert!(m.score.unwrap() > 50);
    }

    #[test]
    fn outreach_facts_pick_relevant_bullet() {
        let (title, body) = FIRMWARE_POSTING;
        let facts = outreach_facts(EXAMPLE, title, body).unwrap();
        assert_eq!(facts.name.as_deref(), Some("Alex Chen"));
        assert_eq!(facts.school.as_deref(), Some("University of Waterloo"));
        let h = facts.highlight.expect("a relevant bullet");
        assert!(h.text.contains("STM32") || h.text.contains("FreeRTOS") || h.text.contains("UART"), "{h:?}");
        assert!(!facts.skills.is_empty() && facts.skills.len() <= 3);
    }

    #[test]
    fn file_names() {
        assert_eq!(file_stem("Jane Doe"), "Jane_Doe_Resume");
        assert_eq!(file_stem("🤖"), "Resume");
    }

    const EXAMPLE: &str = include_str!("../../docs/master_resume_example.tex");

    const FIRMWARE_POSTING: (&str, &str) = (
        "Embedded Firmware Intern",
        "Join our embedded team building firmware for battery management systems.\n\
         What you'll need: Strong C programming skills. Experience with microcontrollers such as STM32. \
         Familiarity with communication protocols like I2C, SPI, UART and CAN.\n\
         Nice to have: RTOS experience (FreeRTOS or Zephyr), Python for test automation, PCB bring-up with an oscilloscope.",
    );

    #[test]
    fn example_parses() {
        let doc = Document::parse(EXAMPLE).unwrap();
        assert_eq!(doc.entries.len(), 11);
        assert_eq!(doc.bullets.len(), 35);
        assert_eq!(doc.skill_lines.len(), 3);
        assert_eq!(doc.render(&doc.identity_plan()), EXAMPLE);
    }

    /// Needs pdflatex; skipped when it isn't installed. Set RESUME_TEST_OUT to a
    /// directory to keep the generated PDF for a look.
    #[tokio::test]
    async fn tailors_example_to_one_page() {
        let master = match compile::compile(EXAMPLE).await {
            Ok(c) => c,
            Err(CompileError::NotInstalled | CompileError::NoSandbox) => {
                eprintln!("pdflatex or bubblewrap not installed; skipping");
                return;
            }
            Err(e) => panic!("example resume should compile: {e}"),
        };
        assert!(master.pages > 1, "example is meant to overflow one page");

        let (title, body) = FIRMWARE_POSTING;
        let t = tailor(EXAMPLE, title, body, 1).await.unwrap();
        assert_eq!(t.pages, Some(1), "notes: {:?}", t.notes);
        assert!(t.pdf.as_ref().is_some_and(|p| p.starts_with(b"%PDF")));
        assert!(t.tex.contains("Wrote C drivers for I2C"), "@keep bullet survives");
        assert!(t.report.dropped_bullets + t.report.dropped_projects.len() > 0);
        // A kept project keeps the bullet that says what it is, and it stays first.
        if t.tex.contains("Line-Following Robot") {
            assert!(t.tex.contains("Built a line-following robot"));
        }
        let designed = t.tex.find("Designed a 5-stage pipelined").unwrap_or(0);
        if let Some(verified) = t.tex.find("Verified it against") {
            assert!(designed < verified);
        }
        // Shared phrases ("battery management system") lift a bullet.
        let bms = t.tex.find("battery management system").expect("BMS bullet kept");
        let social = t.tex.find("social media");
        assert!(social.is_none_or(|s| bms < s));

        // Firmware-relevant content leads; the web project goes or sinks.
        let weather = t.tex.find("Weather Station").expect("relevant project kept");
        if let Some(study) = t.tex.find("Study Buddy") {
            assert!(weather < study);
        }
        assert!(t.report.after.matched.iter().any(|s| s.name == "Microcontrollers"));
        assert!(t.report.after.score.unwrap() >= 80, "{:?}", t.report.after.score);

        // Irrelevant projects go before relevant experience.
        assert!(!t.tex.contains("Chess Engine") && !t.tex.contains("Study Buddy"), "irrelevant projects dropped first");
        assert!(t.tex.contains("cell-voltage sensing board"));

        if let Ok(dir) = std::env::var("RESUME_TEST_OUT") {
            let dir = std::path::Path::new(&dir);
            std::fs::write(dir.join("tailored.pdf"), t.pdf.as_ref().unwrap()).unwrap();
            std::fs::write(dir.join("tailored.tex"), &t.tex).unwrap();
        }
    }
}
