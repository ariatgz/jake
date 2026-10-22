//! Finding people to contact about a job: anything the posting itself names,
//! ready-made searches the user opens in their own browser, and (with a Brave
//! Search API key) a short list of likely recruiters shown inline.
//!
//! Nothing here scrapes LinkedIn or messages anyone; it builds links and, at
//! most, runs one search-engine query per job through an official API.

use crate::scrapers::location::{country_name, region_name};
use crate::{db, City};
use poise::serenity_prelude as serenity;
use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serenity::{ButtonStyle, CreateActionRow, CreateButton, CreateEmbed};
use sqlx::PgPool;
use std::sync::OnceLock;
use std::time::Duration;
use url::form_urlencoded::byte_serialize;

/// Contact details that appear in the posting text.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct PostingContacts {
    pub emails: Vec<String>,
    /// "Jane Doe (Recruiter)"
    pub people: Vec<String>,
    pub team: Option<String>,
    pub req_id: Option<String>,
}

impl PostingContacts {
    pub fn is_empty(&self) -> bool {
        self.emails.is_empty() && self.people.is_empty() && self.team.is_none() && self.req_id.is_none()
    }
}

fn regex(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("valid regex"))
}

/// Mailboxes that never reach the hiring team.
const GENERIC_MAILBOXES: &[&str] = &[
    "noreply", "no-reply", "donotreply", "privacy", "accommodat", "accessib", "disabilit", "eeo", "ethics", "benefits",
    "payroll", "fraud", "legal", "compliance", "dataprotection", "gdpr", "dpo", "security", "support",
];

/// Capitalised words that the "Recruiter: Name" pattern can mistake for a name.
const NOT_NAMES: &[&str] = &[
    "The", "Our", "This", "Team", "Please", "For", "If", "Talent", "Acquisition", "Partner", "Manager", "Hiring", "Contact",
    "Recruiting", "Information", "Us", "Department", "Human", "Resources", "Will", "You", "We", "All", "Any",
];

pub fn from_posting(text: &str, url: &str, company: &str) -> PostingContacts {
    static EMAIL: OnceLock<Regex> = OnceLock::new();
    static PERSON: OnceLock<Regex> = OnceLock::new();
    static TEAM: OnceLock<Regex> = OnceLock::new();
    static REQ_TEXT: OnceLock<Regex> = OnceLock::new();
    static REQ_URL: OnceLock<Regex> = OnceLock::new();

    let mut out = PostingContacts::default();

    for m in regex(&EMAIL, r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b").find_iter(text) {
        let email = m.as_str().trim_end_matches('.').to_lowercase();
        let local = email.split('@').next().unwrap_or("");
        if !GENERIC_MAILBOXES.iter().any(|g| local.contains(g)) && !out.emails.contains(&email) && out.emails.len() < 3 {
            out.emails.push(email);
        }
    }

    let person = regex(
        &PERSON,
        r"\b([Rr]ecruiter|[Rr]ecruiting [Cc]ontact|[Tt]alent [Aa]cquisition(?: [Pp]artner)?|[Hh]iring [Mm]anager|[Cc]ontact [Pp]erson)\s*(?:is\s+|:\s*|-\s*|–\s*)([A-Z][a-z]+(?:[ '-][A-Z][a-z]+){1,2})\b",
    );
    for caps in person.captures_iter(text) {
        let name = &caps[2];
        if name.split([' ', '\'', '-']).any(|w| NOT_NAMES.contains(&w)) {
            continue;
        }
        let entry = format!("{name} ({})", capitalize(&caps[1]));
        if !out.people.contains(&entry) && out.people.len() < 3 {
            out.people.push(entry);
        }
    }

    let team = regex(
        &TEAM,
        r"\b(?:[Jj]oin(?:ing)?|[Pp]art of|[Ww]ithin)\s+(?:the\s+|our\s+)?((?:[A-Z][A-Za-z0-9&+/]*(?:-[A-Za-z0-9]+)?\s+){1,5}(?:[Tt]eam|[Gg]roup|[Dd]ivision|[Oo]rganization|[Ll]ab|[Bb]usiness [Uu]nit))\b",
    );
    let company_l = company.to_lowercase();
    for caps in team.captures_iter(text) {
        let found = caps[1].trim().to_string();
        let words: Vec<&str> = found.split_whitespace().collect();
        let qualifier = words[..words.len() - 1].join(" ").to_lowercase();
        if qualifier != company_l && !qualifier.is_empty() {
            out.team = Some(found);
            break;
        }
    }

    let req_text = regex(
        &REQ_TEXT,
        r"(?i)\b(?:job|requisition|req)\s*(?:id|#|number|no\.?)\s*[:#]?\s*([A-Z]{0,4}-?\d{3,}(?:-\d+)?)\b",
    );
    let req_url = regex(&REQ_URL, r"(?:[_/])((?:JR|REQ|R)-?\d{4,}(?:-\d+)?)(?:[/?#_]|$)");
    out.req_id = req_text
        .captures(text)
        .or_else(|| req_url.captures(url))
        .map(|c| c[1].to_string());

    out
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// "new york" -> "New York"
fn title_case(s: &str) -> String {
    s.split(' ').map(capitalize).collect::<Vec<_>>().join(" ")
}

/// Words in a job title that say nothing about the team's area.
const TITLE_NOISE: &[&str] = &[
    "intern", "interns", "internship", "co-op", "coop", "co", "op", "student", "students", "summer", "fall", "winter", "spring",
    "autumn", "term", "long", "short", "months", "month", "new", "grad", "graduate", "university", "early", "career",
    "talent", "program", "engineer", "engineering", "developer", "development", "specialist", "associate", "position",
    "role", "trainee", "apprentice", "apprenticeship", "placement", "work", "the", "and", "of", "for", "in", "with", "&",
];

/// The area of a job title, for finding the team: "Embedded Firmware Developer
/// Intern" -> "Embedded Firmware".
pub fn role_focus(title: &str) -> String {
    let words: Vec<&str> = title
        .split(|c: char| c.is_whitespace() || matches!(c, '/' | '(' | ')' | ',' | '|' | ':' | '–'))
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric() && c != '+' && c != '#'))
        .filter(|w| !w.is_empty() && !w.chars().all(|c| c.is_ascii_digit() || c == '-'))
        .filter(|w| !TITLE_NOISE.contains(&w.to_lowercase().as_str()))
        .collect();
    if words.is_empty() {
        title.split_whitespace().take(2).collect::<Vec<_>>().join(" ")
    } else {
        words.into_iter().take(3).collect::<Vec<_>>().join(" ")
    }
}

/// Who and where to search for.
#[derive(Debug, Clone)]
pub struct SearchContext {
    pub company: String,
    pub focus: String,
    pub region: Option<String>,
    pub school: Option<String>,
}

impl SearchContext {
    pub fn new(company: &str, title: &str, cities: &[City], school: Option<String>) -> Self {
        // Campus recruiting is usually organised by province/state or country.
        let region = cities.first().and_then(|c| {
            region_name(&c.region)
                .or_else(|| country_name(&c.country))
                .map(title_case)
                .or_else(|| (!c.display_name.is_empty()).then(|| c.display_name.clone()))
        });
        SearchContext { company: company.trim().to_string(), focus: role_focus(title), region, school }
    }

    fn recruiter_query(&self) -> String {
        let mut q = format!(
            "site:linkedin.com/in \"{}\" (recruiter OR \"talent acquisition\") (university OR campus OR \"early talent\" OR \"early career\" OR intern)",
            self.company
        );
        if let Some(region) = &self.region {
            q.push_str(&format!(" \"{region}\""));
        }
        q
    }
}

pub struct SearchLink {
    pub label: String,
    pub url: String,
}

fn linkedin_people(keywords: &str) -> String {
    let encoded: String = byte_serialize(keywords.as_bytes()).collect();
    format!("https://www.linkedin.com/search/results/people/?keywords={encoded}")
}

fn google(query: &str) -> String {
    let encoded: String = byte_serialize(query.as_bytes()).collect();
    format!("https://www.google.com/search?q={encoded}")
}

/// Searches the user opens themselves, logged in to their own accounts.
pub fn links(ctx: &SearchContext) -> Vec<SearchLink> {
    let company = &ctx.company;
    let mut out = vec![
        SearchLink {
            label: "Campus recruiters".into(),
            url: linkedin_people(&format!(
                "\"{company}\" recruiter (university OR campus OR \"early talent\" OR \"early career\" OR intern)"
            )),
        },
        SearchLink {
            label: format!("{} team leads", short_label(&ctx.focus, 30)),
            url: linkedin_people(&format!("\"{company}\" {} (manager OR lead)", ctx.focus)),
        },
    ];
    if let Some(school) = &ctx.school {
        out.push(SearchLink { label: short_label(&format!("{school} alumni"), 40), url: linkedin_people(&format!("\"{company}\" \"{school}\"")) });
    }
    out.push(SearchLink { label: "Web search".into(), url: google(&ctx.recruiter_query()) });
    out
}

fn short_label(text: &str, max: usize) -> String {
    if text.chars().count() <= max { text.to_string() } else { format!("{}…", text.chars().take(max - 1).collect::<String>().trim_end()) }
}

/// A likely contact from a search result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Person {
    pub name: String,
    pub headline: String,
    pub url: String,
}

/// "Jane Doe - University Recruiter - onsemi | LinkedIn" -> Jane Doe, "University Recruiter · onsemi".
fn parse_result(title: &str, url: &str) -> Option<Person> {
    if !url.contains("linkedin.com/in/") {
        return None;
    }
    let title = title.replace('\u{a0}', " ").replace(" – ", " - ").replace(" — ", " - ");
    let title = title.split(" | ").next().unwrap_or("").trim();
    let mut parts: Vec<&str> = title.split(" - ").map(str::trim).collect();
    let name = parts.remove(0).to_string();
    let words = name.split_whitespace().count();
    if !(2..=4).contains(&words) || name.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    let headline = parts.into_iter().filter(|p| !p.is_empty() && *p != "LinkedIn").collect::<Vec<_>>().join(" · ");
    Some(Person { name, headline, url: url.to_string() })
}

#[derive(Deserialize)]
struct BraveResponse {
    web: Option<BraveWeb>,
}

#[derive(Deserialize)]
struct BraveWeb {
    #[serde(default)]
    results: Vec<BraveResult>,
}

#[derive(Deserialize)]
struct BraveResult {
    title: String,
    url: String,
}

const CACHE_DAYS: i32 = 14;

/// Likely recruiters via the Brave Search API, if `BRAVE_API_KEY` is set.
/// Results are cached per query, so each company/region costs one request
/// every couple of weeks.
pub async fn search_people(client: &Client, pool: &PgPool, ctx: &SearchContext) -> Option<Vec<Person>> {
    let key = std::env::var("BRAVE_API_KEY").ok().filter(|k| !k.trim().is_empty())?;
    let query = ctx.recruiter_query();

    if let Ok(Some(cached)) = db::cached_search(pool, &query, CACHE_DAYS).await {
        if let Ok(people) = serde_json::from_value::<Vec<Person>>(cached) {
            return Some(people);
        }
    }

    let response = client
        .get("https://api.search.brave.com/res/v1/web/search")
        .query(&[("q", query.as_str()), ("count", "10")])
        .header("Accept", "application/json")
        .header("X-Subscription-Token", key.trim())
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .and_then(|r| r.error_for_status());
    let body: BraveResponse = match response {
        Ok(r) => match r.json().await {
            Ok(b) => b,
            Err(e) => {
                eprintln!("Brave search returned unexpected data: {e}");
                return None;
            }
        },
        Err(e) => {
            eprintln!("Brave search failed: {e}");
            return None;
        }
    };

    let company = ctx.company.to_lowercase();
    let mut people: Vec<Person> = body
        .web
        .map(|w| w.results)
        .unwrap_or_default()
        .iter()
        .filter_map(|r| parse_result(&r.title, &r.url))
        .collect();
    // People whose headline names the company and a recruiting role first.
    let rank = |p: &Person| {
        let h = p.headline.to_lowercase();
        let recruiting = ["recruit", "talent", "campus", "university", "early"].iter().any(|k| h.contains(k));
        (!h.contains(&company), !recruiting)
    };
    people.sort_by_key(rank);
    people.dedup_by(|a, b| a.url == b.url);
    people.truncate(5);

    if let Ok(value) = serde_json::to_value(&people) {
        let _ = db::store_search(pool, &query, &value).await;
    }
    Some(people)
}

pub const DRAFT_BUTTON: &str = "DRAFT";

/// The "People to contact" card posted in a job thread after Apply.
pub fn panel(ctx: &SearchContext, found: &PostingContacts, people: Option<&[Person]>) -> (CreateEmbed, Vec<CreateActionRow>) {
    let mut embed = CreateEmbed::new()
        .title("People to contact")
        .colour(serenity::Colour::from(0x5865F2u32))
        .description(
            "Most applicants never talk to a person. A short, specific note to a campus recruiter or someone on the team \
             (an alum is best) gets your application looked at.",
        );

    if !found.is_empty() {
        let mut lines = Vec::new();
        lines.extend(found.people.iter().cloned());
        lines.extend(found.emails.iter().map(|e| format!("Email: {e}")));
        if let Some(team) = &found.team {
            lines.push(format!("Team: {team}"));
        }
        if let Some(req) = &found.req_id {
            lines.push(format!("Job ID: {req} (mention it when you reach out)"));
        }
        embed = embed.field("From the posting", lines.join("\n"), false);
    }

    match people {
        Some(list) if !list.is_empty() => {
            let lines: Vec<String> = list
                .iter()
                .map(|p| {
                    let headline = if p.headline.is_empty() { String::new() } else { format!(" · {}", short_label(&p.headline, 70)) };
                    format!("[{}]({}){headline}", p.name, p.url)
                })
                .collect();
            embed = embed.field("Likely recruiters", lines.join("\n"), false);
        }
        Some(_) => {
            embed = embed.field("Likely recruiters", "*No public profiles found; try the searches below.*", false);
        }
        None => {}
    }

    embed = embed.field(
        "How to use this",
        "1. Open a search below and pick one or two people (a recruiter plus someone on the team).\n\
         2. Press **Draft message**, make it sound like you, and send it yourself.\n\
         3. Log it with `/outreach sent name:<who>` and I'll remind you here to follow up.",
        false,
    );

    let link_buttons: Vec<CreateButton> = links(ctx)
        .into_iter()
        .filter(|l| crate::pipeline::link_button_ok(&l.url))
        .take(5)
        .map(|l| CreateButton::new_link(l.url).label(l.label))
        .collect();
    let mut rows = Vec::new();
    if !link_buttons.is_empty() {
        rows.push(CreateActionRow::Buttons(link_buttons));
    }
    rows.push(CreateActionRow::Buttons(vec![CreateButton::new(DRAFT_BUTTON).label("Draft message").style(ButtonStyle::Primary)]));
    (embed, rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_contacts_from_posting() {
        let text = "Join the Power Solutions Group in Waterloo. Questions? Your recruiter: Priya Sharma, or email \
                    priya.sharma@onsemi.com. For accommodations contact accommodations@onsemi.com. Job ID: R-12345.";
        let c = from_posting(text, "https://onsemi.wd5.myworkdayjobs.com/x/job/Waterloo/Firmware-Intern_JR2018381", "onsemi");
        assert_eq!(c.emails, vec!["priya.sharma@onsemi.com"]);
        assert_eq!(c.people, vec!["Priya Sharma (Recruiter)"]);
        assert_eq!(c.team.as_deref(), Some("Power Solutions Group"));
        assert_eq!(c.req_id.as_deref(), Some("R-12345"), "text beats URL");

        let c = from_posting("Join the NVIDIA team! The Team is great.", "https://x.myworkdayjobs.com/a/job/b/Intern_JR2018381", "NVIDIA");
        assert_eq!(c.team, None, "company name isn't a team");
        assert_eq!(c.req_id.as_deref(), Some("JR2018381"));
        assert!(c.people.is_empty() && c.emails.is_empty());
    }

    #[test]
    fn ignores_non_names() {
        let c = from_posting("Your Talent Acquisition Partner will reach out. Hiring Manager: The Team", "", "x");
        assert!(c.people.is_empty(), "{:?}", c.people);
    }

    #[test]
    fn focus_from_title() {
        assert_eq!(role_focus("Embedded Firmware Developer Intern"), "Embedded Firmware");
        assert_eq!(role_focus("Summer 2027 Long Term Firmware Engineering Intern/Co-op"), "Firmware");
        assert_eq!(role_focus("FPGA IP Design Engineer - Intern"), "FPGA IP Design");
        assert_eq!(role_focus("Intern"), "Intern");
    }

    #[test]
    fn builds_links() {
        let cities = [City { display_name: "Waterloo".into(), region: "ON".into(), country: "CA".into() }];
        let ctx = SearchContext::new("onsemi", "Embedded Firmware Developer Intern", &cities, Some("University of Waterloo".into()));
        assert_eq!(ctx.region.as_deref(), Some("Ontario"));
        let links = links(&ctx);
        assert_eq!(links.len(), 4);
        assert!(links[0].url.starts_with("https://www.linkedin.com/search/results/people/?keywords=%22onsemi%22+recruiter"));
        assert!(links[1].url.contains("Embedded+Firmware"));
        assert!(links[2].label.contains("Waterloo alumni"));
        assert!(links[3].url.contains("site%3Alinkedin.com%2Fin") && links[3].url.contains("Ontario"));
        assert!(links.iter().all(|l| l.url.len() < 512 && l.label.chars().count() <= 80), "Discord button limits");
    }

    #[test]
    fn parses_linkedin_results() {
        let p = parse_result("Priya Sharma - University Recruiter - onsemi | LinkedIn", "https://ca.linkedin.com/in/priya").unwrap();
        assert_eq!(p.name, "Priya Sharma");
        assert_eq!(p.headline, "University Recruiter · onsemi");
        assert!(parse_result("Careers at onsemi", "https://www.onsemi.com/careers").is_none());
        assert!(parse_result("onsemi | LinkedIn", "https://linkedin.com/in/x").is_none());
    }
}
