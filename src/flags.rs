//! Spots requirements in a posting that can rule you out (citizenship,
//! clearance, graduation window, GPA, degree level) so they show on the alert.

use regex::Regex;
use std::sync::OnceLock;

struct Phrase {
    label: &'static str,
    needles: &'static [&'static str],
}

const PHRASES: &[Phrase] = &[
    Phrase {
        label: "US citizenship / export control",
        needles: &[
            "u.s. citizen", "u.s. citizens", "u.s. citizenship", "us citizen", "us citizens", "us citizenship",
            "united states citizen", "united states citizenship", "u.s. person", "us person", "itar", "export control",
            "export controls", "export controlled", "export-controlled",
        ],
    },
    Phrase {
        label: "Security clearance",
        needles: &["security clearance", "security clearances", "secret clearance", "top secret", "ts/sci", "clearance is required"],
    },
    Phrase { label: "Canadian citizen/PR", needles: &["canadian citizen", "canadian citizens", "canadian citizenship", "canadian permanent resident"] },
    Phrase { label: "Must return to school after the internship", needles: &["returning to school", "return to school", "returning to your studies"] },
    Phrase { label: "Aimed at PhD students", needles: &["pursuing a phd", "pursuing a ph.d", "phd candidate", "ph.d. candidate", "phd student"] },
    Phrase { label: "Aimed at Master's students", needles: &["pursuing a master", "master's student", "masters student", "graduate students only"] },
    Phrase { label: "French / bilingual", needles: &["bilingual", "fluent in french", "french is required", "french language"] },
    Phrase { label: "No visa sponsorship", needles: &["not able to sponsor", "unable to sponsor", "no sponsorship", "will not sponsor", "without sponsorship", "not provide sponsorship"] },
];

fn grad_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)graduat(?:e|ing|ion)\b[^.;\n]{0,80}").unwrap())
}

fn year_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b(20\d{2})\b").unwrap())
}

fn gpa_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)(?:\bgpa\b|grade point average)[^.;\n]{0,40}?\b([2-4]\.\d{1,2})\b").unwrap())
}

/// Short labels for the "check before applying" field, most important first.
pub fn detect(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    let mut out: Vec<String> = PHRASES
        .iter()
        .filter(|p| p.needles.iter().any(|n| crate::filter::contains_word(&lower, n)))
        .map(|p| p.label.to_string())
        .collect();

    // "graduating between December 2026 and June 2027" -> 2026/2027
    let mut years: Vec<&str> = grad_regex()
        .find_iter(text)
        .flat_map(|m| year_regex().find_iter(m.as_str()).map(|y| y.as_str()))
        .collect();
    years.sort_unstable();
    years.dedup();
    if !years.is_empty() {
        out.push(format!("Graduation {}", years.join("/")));
    }
    if let Some(gpa) = gpa_regex().captures(text).and_then(|c| c.get(1)) {
        out.push(format!("GPA {}+", gpa.as_str()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_flags() {
        let text = "Applicants must be a U.S. citizen due to ITAR. Candidates graduating between December 2026 and June 2027. \
                    Minimum GPA of 3.0. Must be returning to school after the internship.";
        let flags = detect(text);
        assert!(flags.contains(&"US citizenship / export control".to_string()));
        assert!(flags.contains(&"Must return to school after the internship".to_string()));
        assert!(flags.contains(&"Graduation 2026/2027".to_string()), "{flags:?}");
        assert!(flags.contains(&"GPA 3.0+".to_string()));
    }

    #[test]
    fn ignores_ordinary_text() {
        assert!(detect("Build firmware for our sensors. The team values curiosity. Founded in 2015.").is_empty());
        assert!(detect("We care about our users and their trust").is_empty(), "'us' alone isn't citizenship");
        assert_eq!(detect("US citizenship is required for this role."), vec!["US citizenship / export control"]);
    }
}
