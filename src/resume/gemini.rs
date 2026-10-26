//! Resume tailoring through the Gemini API. The model gets the master `.tex`
//! and the posting, returns a full `.tex`, and that goes through the same
//! sandboxed pdflatex as everything else. If the result is over the page limit
//! or doesn't compile, the model is told and asked again, a few times.
//!
//! Enabled by `GEMINI_API_KEY`; `GEMINI_MODEL` picks the model (default
//! `gemini-3.5-flash`).

use super::compile::{self, CompileError, Compiled};
use super::latex::Document;
use super::{coverage, skills, Report, ResumeKnowledge, Tailored};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const DEFAULT_MODEL: &str = "gemini-3.5-flash";
/// Full-document regenerations the model gets per tailoring run.
const MAX_ATTEMPTS: usize = 3;
/// Postings longer than this are cut; the requirements are near the top anyway.
const MAX_POSTING_CHARS: usize = 14_000;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(150);

pub fn api_key() -> Option<String> {
    std::env::var("GEMINI_API_KEY").ok().map(|k| k.trim().to_string()).filter(|k| !k.is_empty())
}

pub fn enabled() -> bool {
    api_key().is_some()
}

pub fn model() -> String {
    std::env::var("GEMINI_MODEL").ok().map(|m| m.trim().to_string()).filter(|m| !m.is_empty()).unwrap_or_else(|| DEFAULT_MODEL.to_string())
}

#[derive(Serialize, Deserialize, Clone)]
struct Part {
    /// Missing on thought-signature parts that some models add.
    #[serde(default)]
    text: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct Content {
    role: String,
    parts: Vec<Part>,
}

impl Content {
    fn user(text: impl Into<String>) -> Self {
        Content { role: "user".into(), parts: vec![Part { text: text.into() }] }
    }
    fn model(text: impl Into<String>) -> Self {
        Content { role: "model".into(), parts: vec![Part { text: text.into() }] }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerationConfig {
    temperature: f32,
    max_output_tokens: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Request<'a> {
    system_instruction: Content,
    contents: &'a [Content],
    generation_config: GenerationConfig,
}

#[derive(Deserialize)]
struct Response {
    #[serde(default)]
    candidates: Vec<Candidate>,
    error: Option<ApiError>,
    #[serde(default)]
    prompt_feedback: Option<PromptFeedback>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Candidate {
    content: Option<Content>,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ApiError {
    message: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PromptFeedback {
    block_reason: Option<String>,
}

fn system_prompt(max_pages: u32) -> String {
    format!(
        "You are an expert resume writer and ATS (applicant tracking system) specialist. You rewrite a \
         candidate's LaTeX master resume into the strongest possible version for ONE job posting, so that \
         both the ATS keyword screen and a 10-second recruiter skim rank it highly.\n\
         You receive the posting, a list of target keywords pulled from it, and the complete master resume \
         (.tex). Reply with the complete, compilable .tex file and nothing else: no explanation, no \
         Markdown code fences.\n\
         \n\
         How to tailor (do all of these, not just reordering):\n\
         1. KEYWORDS. Work every target keyword and phrase the posting uses into the resume wherever the \
         candidate's experience genuinely covers it. Use the posting's exact wording, spelling and \
         capitalisation (\"Design Verification\" if the posting says that, not \"DV\"; include both when \
         the posting uses both). Spell out acronyms once with the acronym in brackets. Required keywords \
         matter more than nice-to-haves; the job title's keywords matter most. Required keywords should \
         appear in the skills section AND in at least one bullet when the evidence exists.\n\
         2. REWRITE bullets, don't just shuffle them. Rephrase each bullet you keep so it leads with the \
         action and technology the posting cares about, mirrors the posting's own verbs and vocabulary, \
         keeps every number and outcome, and stays one or two lines. Strong verb first, no first person, \
         no filler.\n\
         3. REARRANGE for relevance. Order bullets within each job most-relevant-first, order projects \
         most-relevant-first, and reorder the entries on the skills lines so matching skills come first. \
         If the template's sections can be reordered (for example Projects before Experience when the \
         projects fit the posting better), do it; the header and Education stay where they are.\n\
         4. SKILLS LINES. Add a target keyword to the skills lines when a bullet, project or listed skill \
         already shows the candidate used that thing or a direct equivalent (a UART driver in C is \
         evidence for \"Embedded C\" and \"Serial protocols\"; Verilog coursework is evidence for \"RTL \
         design\"). Rename listed skills to the posting's phrasing. Group related skills under the \
         existing categories.\n\
         5. TRIM. Drop whole off-topic projects first, then the weakest bullets from the longest lists, \
         until it fits on at most {max_pages} page{}. Do not shrink fonts, margins or spacing.\n\
         \n\
         Hard limits (never break these):\n\
         - Never fabricate. No new employers, titles, dates, degrees, certifications, projects, numbers, \
         outcomes or responsibilities. A keyword may only be added where the master resume contains \
         evidence for it; if there is no evidence, leave it out, even if it is required.\n\
         - Keep the preamble, document class, packages, custom macros and formatting exactly as they are. \
         Never add packages, and never use commands that read files, run programs or include images. \
         Only use LaTeX macros the master resume already uses.\n\
         - Leave the header, contact details and the Education section unchanged.\n\
         - Each job keeps at least two bullets. Keep at least two projects. A project's first bullet \
         stays first (it says what the project is). Items with a `% @keep` comment are never removed.\n\
         - Escape LaTeX special characters in any text you write (& % $ # _).",
        if max_pages == 1 { "" } else { "s" }
    )
}

/// "name (required)" lines for the keywords the bot extracted from the posting,
/// so the model knows exactly what the ATS will look for.
fn keyword_lines(posting: &[skills::PostingSkill]) -> String {
    let mut sorted: Vec<&skills::PostingSkill> = posting.iter().collect();
    sorted.sort_by(|a, b| b.weight.partial_cmp(&a.weight).unwrap_or(std::cmp::Ordering::Equal));
    let mut lines: Vec<String> = Vec::new();
    for skill in sorted {
        let tag = match skill.importance {
            skills::Importance::Required => "required",
            skills::Importance::Preferred => "nice to have",
            skills::Importance::Mentioned => "mentioned",
        };
        lines.push(format!("- {} ({tag})", skill.name));
    }
    if lines.is_empty() {
        "- (none detected automatically; take them from the posting text)".to_string()
    } else {
        lines.join("\n")
    }
}

fn first_prompt(master: &str, title: &str, body: &str, posting: &[skills::PostingSkill]) -> String {
    let body: String = body.chars().take(MAX_POSTING_CHARS).collect();
    format!(
        "JOB TITLE:\n{title}\n\nTARGET KEYWORDS (from the posting, most important first):\n{}\n\n\
         JOB POSTING:\n{body}\n\nMASTER RESUME (.tex):\n{master}",
        keyword_lines(posting)
    )
}

/// Pulls the LaTeX document out of a reply, tolerating code fences and chatter.
pub(crate) fn extract_tex(reply: &str) -> Option<String> {
    let start = reply.find("\\documentclass")?;
    let end_marker = "\\end{document}";
    let end = reply[start..].rfind(end_marker)? + start + end_marker.len();
    Some(reply[start..end].to_string())
}

/// Pauses between retries of a busy or rate-limited API (429/503, "high demand").
const BACKOFF: [Duration; 3] = [Duration::from_secs(5), Duration::from_secs(15), Duration::from_secs(40)];

/// One API call, retried a few times when the service is merely busy.
async fn generate(client: &Client, key: &str, model: &str, system: &str, history: &[Content]) -> Result<String, String> {
    let mut last = String::new();
    for (i, wait) in BACKOFF.iter().enumerate() {
        match generate_once(client, key, model, system, history).await {
            Ok(reply) => return Ok(reply),
            Err((true, msg)) => {
                eprintln!("Gemini busy (try {}): {msg}", i + 1);
                last = msg;
                tokio::time::sleep(*wait).await;
            }
            Err((false, msg)) => return Err(msg),
        }
    }
    match generate_once(client, key, model, system, history).await {
        Ok(reply) => Ok(reply),
        Err((_, msg)) => Err(if msg.is_empty() { last } else { msg }),
    }
}

/// `Err((retryable, message))`.
async fn generate_once(client: &Client, key: &str, model: &str, system: &str, history: &[Content]) -> Result<String, (bool, String)> {
    let url = format!("https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent");
    let request = Request {
        system_instruction: Content { role: "system".into(), parts: vec![Part { text: system.to_string() }] },
        contents: history,
        generation_config: GenerationConfig { temperature: 0.35, max_output_tokens: 32_768 },
    };
    let response = client
        .post(&url)
        .header("x-goog-api-key", key)
        .header("Content-Type", "application/json")
        .timeout(REQUEST_TIMEOUT)
        .json(&request)
        .send()
        .await
        .map_err(|e| (e.is_timeout() || e.is_connect(), format!("Gemini request failed: {e}")))?;
    let status = response.status();
    let busy = status.as_u16() == 429 || status.is_server_error();
    let text = response.text().await.map_err(|e| (busy, format!("Gemini response unreadable: {e}")))?;
    let parsed: Response =
        serde_json::from_str(&text).map_err(|_| (busy, format!("Gemini returned HTTP {status} with unexpected data: {}", super::short(&text, 200))))?;
    if let Some(err) = parsed.error {
        let lower = err.message.to_lowercase();
        let transient = busy || lower.contains("high demand") || lower.contains("overloaded") || lower.contains("try again");
        return Err((transient, format!("Gemini ({model}): {}", err.message)));
    }
    reply_text(parsed).map_err(|m| (false, m))
}

/// The model's text out of a successful response.
fn reply_text(parsed: Response) -> Result<String, String> {
    if let Some(reason) = parsed.prompt_feedback.and_then(|f| f.block_reason) {
        return Err(format!("Gemini blocked the request ({reason})."));
    }
    let candidate = parsed.candidates.into_iter().next().ok_or_else(|| "Gemini returned no answer.".to_string())?;
    let reply: String = candidate.content.map(|c| c.parts.into_iter().map(|p| p.text).collect::<Vec<_>>().join("")).unwrap_or_default();
    if reply.trim().is_empty() {
        return Err(format!("Gemini returned an empty answer (finish reason: {}).", candidate.finish_reason.unwrap_or_else(|| "unknown".into())));
    }
    Ok(reply)
}

/// Keyword report comparing the master to what the model produced. The model
/// may wander from the template, so the tailored side falls back to the
/// master's numbers when it can't be parsed.
fn report_for(master_doc: &Document, knowledge: &ResumeKnowledge, posting: &[skills::PostingSkill], tex: &str) -> Report {
    let before = coverage(posting, &knowledge.have);
    let after = match Document::parse(tex) {
        Ok(doc) => coverage(posting, &ResumeKnowledge::new(&doc).have),
        Err(_) => before.clone(),
    };
    let cut = before.matched.iter().filter(|s| !after.matched.iter().any(|a| a.name == s.name)).map(|s| s.name.clone()).collect();
    let count = |t: &str| t.matches("\\resumeItem").count();
    let dropped_bullets = count(&master_doc.render(&master_doc.identity_plan())).saturating_sub(count(tex));
    Report { before, after, cut, dropped_bullets, dropped_projects: Vec::new(), project_order: Vec::new(), new_leads: Vec::new() }
}

/// Tailors `master` for one posting with Gemini and compiles it to fit `max_pages`.
pub async fn tailor(client: &Client, master: &str, title: &str, body: &str, max_pages: u32) -> Result<Tailored, String> {
    let key = api_key().ok_or("GEMINI_API_KEY isn't set.")?;
    let model = model();
    compile::check_safe(master)?;
    let master_doc = Document::parse(master)?;
    let knowledge = ResumeKnowledge::new(&master_doc);
    let posting = skills::extract(&knowledge.vocab, title, body);

    let system = system_prompt(max_pages);
    let mut history = vec![Content::user(first_prompt(master, title, body, &posting))];
    let mut best: Option<(String, Compiled)> = None;
    let mut notes = Vec::new();
    let mut last_problem = String::new();

    for attempt in 1..=MAX_ATTEMPTS {
        let reply = generate(client, &key, &model, &system, &history).await?;
        history.push(Content::model(reply.clone()));

        let Some(tex) = extract_tex(&reply) else {
            last_problem = "the reply wasn't a LaTeX document".into();
            history.push(Content::user("That wasn't a complete LaTeX document. Reply with the full .tex from \\documentclass to \\end{document} and nothing else."));
            continue;
        };
        if let Err(reason) = compile::check_safe(&tex) {
            last_problem = reason.clone();
            history.push(Content::user(format!("Not allowed: {reason} Remove it and reply with the full .tex.")));
            continue;
        }

        match compile::compile(&tex).await {
            Ok(c) if c.pages <= max_pages => {
                let report = report_for(&master_doc, &knowledge, &posting, &tex);
                if posting.is_empty() {
                    notes.push("The posting doesn't name any skills the bot recognises; the keyword numbers above mean little.".into());
                }
                let gained: Vec<&str> = report
                    .after
                    .matched
                    .iter()
                    .filter(|s| !report.before.matched.iter().any(|b| b.name == s.name))
                    .map(|s| s.name.as_str())
                    .collect();
                if !gained.is_empty() {
                    notes.push(format!("Now mentions: {}. Check each one is true of you before sending.", gained.join(", ")));
                }
                return Ok(Tailored { tex, pages: Some(c.pages), pdf: Some(c.pdf), report, notes, by_ai: Some(model) });
            }
            Ok(c) => {
                last_problem = format!("it came out at {} pages", c.pages);
                let feedback = format!(
                    "That compiled to {} pages; the limit is {max_pages}. Remove the least relevant projects and bullets until it fits, keep everything else as is, and reply with the full .tex.",
                    c.pages
                );
                if best.as_ref().is_none_or(|(_, b)| c.pages < b.pages) {
                    best = Some((tex, c));
                }
                history.push(Content::user(feedback));
            }
            Err(CompileError::Latex(log)) => {
                last_problem = "the LaTeX didn't compile".into();
                history.push(Content::user(format!("pdflatex failed with:\n{log}\n\nFix that and reply with the full .tex.")));
            }
            Err(e @ (CompileError::NotInstalled | CompileError::NoSandbox | CompileError::Sandbox(_))) => {
                notes.push(format!("Only the tailored .tex is attached, it wasn't compiled: {e}."));
                let report = report_for(&master_doc, &knowledge, &posting, &tex);
                return Ok(Tailored { tex, pages: None, pdf: None, report, notes, by_ai: Some(model) });
            }
            Err(e) => return Err(e.to_string()),
        }
        if attempt == MAX_ATTEMPTS {
            break;
        }
    }

    match best {
        Some((tex, c)) => {
            notes.push(format!("Still {} pages after {MAX_ATTEMPTS} tries; this is the shortest version.", c.pages));
            let report = report_for(&master_doc, &knowledge, &posting, &tex);
            Ok(Tailored { tex, pages: Some(c.pages), pdf: Some(c.pdf), report, notes, by_ai: Some(model) })
        }
        None => Err(format!("Gemini ({model}) didn't produce a usable resume in {MAX_ATTEMPTS} tries: {last_problem}.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_document_from_fenced_reply() {
        let reply = "Here you go:\n```latex\n\\documentclass{article}\n\\begin{document}\nhi\n\\end{document}\n```\nDone.";
        let tex = extract_tex(reply).unwrap();
        assert!(tex.starts_with("\\documentclass"));
        assert!(tex.ends_with("\\end{document}"));
        assert!(!tex.contains("```"));
    }

    #[test]
    fn rejects_replies_without_a_document() {
        assert!(extract_tex("Sorry, I can't do that.").is_none());
        assert!(extract_tex("\\documentclass{article} but no end").is_none());
    }

    #[test]
    fn request_shape() {
        let history = [Content::user("hello")];
        let req = Request {
            system_instruction: Content { role: "system".into(), parts: vec![Part { text: "sys".into() }] },
            contents: &history,
            generation_config: GenerationConfig { temperature: 0.2, max_output_tokens: 10 },
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["systemInstruction"]["parts"][0]["text"], "sys");
        assert_eq!(json["contents"][0]["role"], "user");
        assert_eq!(json["generationConfig"]["maxOutputTokens"], 10);
    }

    #[test]
    fn parses_api_error() {
        let r: Response = serde_json::from_str(r#"{"error":{"code":400,"message":"API key not valid","status":"INVALID_ARGUMENT"}}"#).unwrap();
        assert_eq!(r.error.unwrap().message, "API key not valid");
        assert!(r.candidates.is_empty());
    }

    #[test]
    fn default_model_without_env() {
        assert!(!model().is_empty());
    }

    /// Real API call. Run with `GEMINI_API_KEY=... cargo test live_gemini -- --ignored --nocapture`.
    /// Without pdflatex+bubblewrap it still checks the round trip and returns the .tex only.
    #[tokio::test]
    #[ignore = "calls the Gemini API"]
    async fn live_gemini_tailor() {
        let master = include_str!("../../docs/master_resume_example.tex");
        let client = Client::new();
        let (title, body) = (
            "Firmware Engineering Intern",
            "Requirements: C, STM32 or similar microcontrollers, I2C/SPI drivers, FreeRTOS.\nNice to have: Python, Git, PCB layout.",
        );
        let t = tailor(&client, master, title, body, 1).await.expect("tailor");
        println!("pages={:?} notes={:?} by_ai={:?} cut={:?} dropped={}", t.pages, t.notes, t.by_ai, t.report.cut, t.report.dropped_bullets);
        assert!(t.tex.contains(r"\begin{document}") && t.tex.contains(r"\end{document}"));
        assert!(t.tex.contains("Education"), "education section kept");
        assert!(compile::check_safe(&t.tex).is_ok());
        if let Ok(dir) = std::env::var("RESUME_TEST_OUT") {
            std::fs::write(format!("{dir}/gemini_tailored.tex"), &t.tex).unwrap();
            if let Some(pdf) = &t.pdf {
                std::fs::write(format!("{dir}/gemini_tailored.pdf"), pdf).unwrap();
            }
        }
    }
}
