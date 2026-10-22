//! Decides whether a stored job matches a user's profile.

use crate::scrapers::location::{country_name, region_name};
use crate::{City, Profile};

/// Case-sensitive whole-word search; pass lowercase strings for a
/// case-insensitive match. "intern" matches "Intern/Co-op" but not "International".
pub fn contains_word(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut start = 0;
    while let Some(pos) = haystack[start..].find(needle) {
        let begin = start + pos;
        let end = begin + needle.len();
        let before_ok = haystack[..begin].chars().next_back().map_or(true, |c| !c.is_alphanumeric());
        let after_ok = haystack[end..].chars().next().map_or(true, |c| !c.is_alphanumeric());
        if before_ok && after_ok {
            return true;
        }
        start = end;
    }
    false
}

/// Everything a location filter term can be matched against, lowercased.
pub fn location_haystack(cities: &[City]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for city in cities {
        parts.push(city.display_name.clone());
        parts.push(city.region.clone());
        parts.push(city.country.clone());
        if let Some(name) = region_name(&city.region) {
            parts.push(name.to_string());
        }
        if let Some(name) = country_name(&city.country) {
            parts.push(name.to_string());
        }
    }
    parts.retain(|p| !p.is_empty());
    parts.join(" | ").to_lowercase()
}

fn lowercase_terms(terms: &[String]) -> Vec<String> {
    terms.iter().map(|t| t.trim().to_lowercase()).filter(|t| !t.is_empty()).collect()
}

/// A profile's filters, lowercased once so a whole run of candidates can be
/// checked without re-allocating every term for every job.
pub struct Matcher {
    seniority: Vec<String>,
    keywords: Vec<String>,
    exclude: Vec<String>,
    locations: Vec<String>,
    keywords_title_only: bool,
}

impl Matcher {
    pub fn new(profile: &Profile) -> Self {
        Self {
            seniority: lowercase_terms(&profile.seniority),
            keywords: lowercase_terms(&profile.keywords),
            exclude: lowercase_terms(&profile.exclude_keywords),
            locations: lowercase_terms(&profile.locations),
            keywords_title_only: profile.keywords_title_only,
        }
    }

    /// `skip_seniority` is used for the hand-written scrapers, which already only
    /// return internships and sometimes strip "Intern" from the title.
    pub fn matches(&self, title: &str, body: &str, cities: &[City], skip_seniority: bool) -> bool {
        let title_l = title.to_lowercase();

        // Exclusions only look at the title: words like "senior" or "sales"
        // show up in the description of almost every posting.
        if self.exclude.iter().any(|e| contains_word(&title_l, e)) {
            return false;
        }

        if !skip_seniority && !self.seniority.is_empty() && !self.seniority.iter().any(|s| contains_word(&title_l, s)) {
            return false;
        }

        if !self.locations.is_empty() {
            let hay = location_haystack(cities);
            if !self.locations.iter().any(|l| hay.contains(l.as_str())) {
                return false;
            }
        }

        if !self.keywords.is_empty() && !self.keywords.iter().any(|k| contains_word(&title_l, k)) {
            if self.keywords_title_only {
                return false;
            }
            // Lowercasing the description is the expensive part, so it only
            // happens when the title alone didn't decide the match.
            let body_l = body.to_lowercase();
            if !self.keywords.iter().any(|k| contains_word(&body_l, k)) {
                return false;
            }
        }

        true
    }
}

/// Splits a comma-separated slash-command argument into trimmed, de-duplicated terms.
pub fn parse_terms_arg(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() || out.iter().any(|p| p.eq_ignore_ascii_case(part)) {
            continue;
        }
        out.push(part.to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(keywords: &[&str], seniority: &[&str], locations: &[&str]) -> Profile {
        Profile {
            user_id: 1,
            display_name: "t".into(),
            channel_id: 1,
            keywords: keywords.iter().map(|s| s.to_string()).collect(),
            seniority: seniority.iter().map(|s| s.to_string()).collect(),
            locations: locations.iter().map(|s| s.to_string()).collect(),
            exclude_keywords: Vec::new(),
            keywords_title_only: false,
            evaluated_through: 0,
            filters_version: 0,
        }
    }

    fn matches(p: &Profile, title: &str, body: &str, cities: &[City], skip_seniority: bool) -> bool {
        Matcher::new(p).matches(title, body, cities, skip_seniority)
    }

    fn city(name: &str, region: &str, country: &str) -> City {
        City { display_name: name.into(), region: region.into(), country: country.into() }
    }

    #[test]
    fn word_boundaries() {
        assert!(contains_word("fpga ip design engineer - intern", "intern"));
        assert!(contains_word("summer 2027 firmware intern/co-op", "co-op"));
        assert!(!contains_word("international sales", "intern"));
        assert!(!contains_word("internship", "intern"));
    }

    #[test]
    fn hardware_canada_profile() {
        let p = profile(
            &["Hardware", "Firmware", "FPGA", "RTL", "ASIC", "Electrical", "Embedded"],
            &["Co-op", "Internship", "Intern", "Apprentice"],
            &["Toronto", "Waterloo", "Markham", "Vancouver", "Canada"],
        );
        let toronto = [city("Toronto", "ON", "CA")];
        assert!(matches(&p, "FPGA IP Design Engineer - Intern", "", &toronto, false));
        assert!(matches(&p, "Summer 2027 Long Term Firmware Engineering Intern/Co-op", "", &[city("Markham", "ON", "CA")], false));
        assert!(!matches(&p, "FPGA IP Design Engineer", "", &toronto, false), "no seniority term");
        assert!(!matches(&p, "Finance Intern", "spreadsheets", &toronto, false), "no domain keyword");
        assert!(!matches(&p, "RTL Design Intern", "", &[city("Santa Clara", "CA", "US")], false), "wrong location");
        assert!(matches(&p, "Design Verification Intern", "work on asic rtl", &[city("Ottawa", "ON", "CA")], false), "country matches, keyword in body");
    }

    #[test]
    fn empty_filters_match_everything() {
        let p = profile(&[], &[], &[]);
        assert!(matches(&p, "Anything", "", &[], false));
    }

    #[test]
    fn exclude_keywords_check_title_only() {
        let mut p = profile(&["Firmware"], &["Intern"], &[]);
        p.exclude_keywords = vec!["PhD".into(), "Sales".into()];
        assert!(!matches(&p, "PhD Firmware Intern", "", &[], false));
        assert!(!matches(&p, "Firmware Sales Intern", "", &[], false));
        assert!(matches(&p, "Firmware Intern", "you will work with PhD researchers and sales", &[], false));
        assert!(!matches(&p, "Firmware Intern (PhD)", "", &[], true), "exclusions apply to hand-written scrapers too");
        assert!(matches(&p, "Firmware Intern - Salesforce tooling", "", &[], false), "whole words only");
    }

    #[test]
    fn keywords_title_only() {
        let mut p = profile(&["FPGA"], &["Intern"], &[]);
        assert!(matches(&p, "Design Intern", "prototype on fpga boards", &[], false));
        p.keywords_title_only = true;
        assert!(!matches(&p, "Design Intern", "prototype on fpga boards", &[], false));
        assert!(matches(&p, "FPGA Design Intern", "", &[], false));
    }

    #[test]
    fn terms_arg_parsing() {
        assert_eq!(parse_terms_arg(" a, b ,,A, c"), vec!["a", "b", "c"]);
    }
}
