//! Outreach message drafts. Everything said about the sender comes from their
//! own resume; the user edits and sends the message themselves.

use crate::resume::OutreachFacts;

/// LinkedIn's limit for a personalised invitation note on a free account.
pub const CONNECTION_NOTE_LIMIT: usize = 200;

pub struct Draft<'a> {
    /// The contact's name as typed by the user; only the first word is used.
    pub contact: Option<&'a str>,
    pub role: &'a str,
    pub company: &'a str,
    pub req_id: Option<&'a str>,
    pub facts: &'a OutreachFacts,
    /// Used when the resume has no name in its header.
    pub fallback_sender: &'a str,
}

impl Draft<'_> {
    fn greeting_name(&self) -> String {
        self.contact.and_then(|c| c.split_whitespace().next()).map(str::to_string).unwrap_or_else(|| "there".to_string())
    }

    fn sender(&self) -> &str {
        self.facts.name.as_deref().unwrap_or(self.fallback_sender)
    }
}

/// "A", "A and B", "A, B and C"
fn join_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// "Architected bare-metal C drivers." -> "architected bare-metal C drivers",
/// leaving acronyms like "FPGA" capitalised.
fn as_clause(bullet: &str) -> String {
    let text = bullet.trim().trim_end_matches(['.', ';']).to_string();
    let first_word = text.split_whitespace().next().unwrap_or("");
    let acronym = first_word.chars().skip(1).any(|c| c.is_uppercase());
    let mut out = if acronym {
        text.clone()
    } else {
        let mut chars = text.chars();
        match chars.next() {
            Some(c) => c.to_lowercase().collect::<String>() + chars.as_str(),
            None => String::new(),
        }
    };
    // Long bullets read badly in a message; cut at a natural break.
    if out.chars().count() > 200 {
        let cut: String = out.chars().take(190).collect();
        let at = cut.rfind(", ").or_else(|| cut.rfind(" and ")).or_else(|| cut.rfind(' ')).unwrap_or(cut.len());
        out = cut[..at].to_string();
    }
    out
}

pub fn connection_note(d: &Draft) -> String {
    let hi = d.greeting_name();
    let role = d.role;
    let company = d.company;
    let skills = &d.facts.skills;

    let mut candidates = Vec::new();
    if skills.len() >= 2 {
        let at = d.facts.highlight.as_ref().filter(|h| !h.is_project).map(|h| format!(" at {}", h.org)).unwrap_or_default();
        candidates.push(format!(
            "Hi {hi}, I just applied for the {role} role at {company}. I've worked on {} and {}{at} and would love to connect and learn more about the team!",
            skills[0], skills[1]
        ));
        candidates.push(format!("Hi {hi}, I just applied for {role} at {company} and have hands-on {} and {} experience. Would love to connect!", skills[0], skills[1]));
    }
    candidates.push(format!("Hi {hi}, I just applied for the {role} role at {company} and would love to connect and learn more about the team!"));
    candidates.push(format!("Hi {hi}, I just applied for {role} at {company}. Would love to connect!"));

    candidates
        .into_iter()
        .find(|c| c.chars().count() <= CONNECTION_NOTE_LIMIT)
        .unwrap_or_else(|| format!("Hi {hi}, I just applied for a role at {company}. Would love to connect!"))
}

pub fn message(d: &Draft) -> String {
    let hi = d.greeting_name();
    let job_id = d.req_id.map(|r| format!(" (Job ID {r})")).unwrap_or_default();
    let mut paragraphs = vec![
        format!("Hi {hi},"),
        format!("I just applied for the {} role at {}{job_id} and wanted to reach out directly.", d.role, d.company),
    ];

    let skills = join_list(&d.facts.skills);
    let middle = match (&d.facts.highlight, skills.is_empty()) {
        (Some(h), false) => {
            let lead = if h.is_project { format!("In my {} project, I", h.org) } else { format!("At {}, I", h.org) };
            format!("{lead} {}. That lines up closely with the role's focus on {skills}.", as_clause(&h.text))
        }
        (Some(h), true) => {
            let lead = if h.is_project { format!("In my {} project, I", h.org) } else { format!("At {}, I", h.org) };
            format!("{lead} {}, which is very close to what this role involves.", as_clause(&h.text))
        }
        (None, false) => format!("I've worked hands-on with {skills}, which lines up with what the role asks for."),
        (None, true) => String::new(),
    };
    if !middle.is_empty() {
        paragraphs.push(middle);
    }

    paragraphs.push(
        "I'd really appreciate it if you could take a look at my application, or point me to the right person on the team. Thanks for your time!"
            .to_string(),
    );
    let signature = match &d.facts.school {
        Some(school) => format!("{}\n{school}", d.sender()),
        None => d.sender().to_string(),
    };
    paragraphs.push(signature);
    paragraphs.join("\n\n")
}

pub fn follow_up(contact: &str, role: &str, company: &str, sender: &str) -> String {
    let hi = contact.split_whitespace().next().unwrap_or("there");
    format!(
        "Hi {hi}, just following up on my note about the {role} role at {company}. I'm still very interested and happy to share \
         anything that would help. Thanks again for your time!\n{}",
        sender.split_whitespace().next().unwrap_or(sender)
    )
}

/// Discord-ready text with both drafts in copyable code blocks.
pub fn render(d: &Draft) -> String {
    let fence = |s: String| s.replace("```", "'''");
    let note = connection_note(d);
    format!(
        "**Connection note** ({} / {CONNECTION_NOTE_LIMIT} characters)\n```\n{}\n```\n**Message or email**\n```\n{}\n```\n\
         -# Make it sound like you, then send it yourself. Afterwards: `/outreach sent name:<who>` and I'll remind you to follow up.",
        note.chars().count(),
        fence(note),
        fence(message(d)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resume::Highlight;

    fn facts() -> OutreachFacts {
        OutreachFacts {
            name: Some("Alex Chen".into()),
            school: Some("University of Waterloo".into()),
            skills: vec!["C".into(), "Microcontrollers".into(), "RTOS".into()],
            highlight: Some(Highlight {
                org: "Northwind Robotics".into(),
                is_project: false,
                text: "Wrote C drivers for I2C and SPI sensors on an STM32 microcontroller, cutting sensor read latency by 35%.".into(),
            }),
        }
    }

    #[test]
    fn drafts_read_naturally() {
        let f = facts();
        let d = Draft { contact: Some("Priya Sharma"), role: "Embedded Firmware Intern", company: "onsemi", req_id: Some("JR2018381"), facts: &f, fallback_sender: "alex" };
        let note = connection_note(&d);
        assert!(note.starts_with("Hi Priya,"));
        assert!(note.chars().count() <= CONNECTION_NOTE_LIMIT, "{note}");

        let msg = message(&d);
        assert!(msg.contains("(Job ID JR2018381)"));
        assert!(msg.contains("At Northwind Robotics, I wrote C drivers for I2C and SPI sensors"), "{msg}");
        assert!(msg.contains("focus on C, Microcontrollers and RTOS."));
        assert!(msg.ends_with("Alex Chen\nUniversity of Waterloo"));
    }

    #[test]
    fn long_roles_still_fit_and_no_facts_still_work() {
        let f = OutreachFacts::default();
        let role = "Summer 2027 Long Term Firmware Engineering Intern/Co-op (16 months, Power Solutions Group, Waterloo)";
        let d = Draft { contact: None, role, company: "onsemi", req_id: None, facts: &f, fallback_sender: "Discord Name" };
        assert!(connection_note(&d).chars().count() <= CONNECTION_NOTE_LIMIT);
        let msg = message(&d);
        assert!(msg.starts_with("Hi there,"));
        assert!(msg.ends_with("Discord Name"));
    }

    #[test]
    fn clauses_keep_acronyms() {
        assert_eq!(as_clause("FPGA prototyping of a CRC block."), "FPGA prototyping of a CRC block");
        assert_eq!(as_clause("Built a test rig."), "built a test rig");
        assert!(as_clause(&"word, ".repeat(80)).chars().count() <= 200);
    }

    #[test]
    fn follow_up_is_short() {
        let text = follow_up("Priya Sharma", "Firmware Intern", "onsemi", "Alex Chen");
        assert!(text.starts_with("Hi Priya, just following up"));
        assert!(text.ends_with("\nAlex"));
    }
}
