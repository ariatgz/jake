//! Small text helpers shared by the board scrapers.

use crate::Term;
use html_escape::decode_html_entities;
use scraper::Html;

/// Strips tags and collapses whitespace. Entity-escaped HTML (Greenhouse) is decoded first.
pub fn html_to_text(html: &str) -> String {
    if html.trim().is_empty() {
        return String::new();
    }
    let decoded = decode_html_entities(html);
    // Block boundaries become spaces so words don't run together.
    let spaced = decoded.replace("><", "> <");
    let fragment = Html::parse_fragment(&spaced);
    let text: Vec<&str> = fragment.root_element().text().collect();
    text.join(" ").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// "Summer 2027 Firmware Intern" -> [S27]. Returns an empty list when no season
/// is present; a season with no year is reported without one ("S").
pub fn parse_terms(title: &str) -> Vec<Term> {
    let year: Option<String> = title
        .split(|c: char| !c.is_ascii_digit())
        .find(|w| w.len() == 4 && w.starts_with("20"))
        .map(|w| w[2..].to_string());

    let seasons = [("winter", "W"), ("summer", "S"), ("fall", "F"), ("autumn", "F"), ("spring", "P")];
    let lower = title.to_lowercase();

    let mut terms: Vec<Term> = Vec::new();
    for (season, prefix) in seasons {
        if lower.contains(season) {
            let name = match &year {
                Some(y) => format!("{prefix}{y}"),
                None => prefix.to_string(),
            };
            if !terms.iter().any(|t| t.display_name == name) {
                terms.push(Term { display_name: name });
            }
        }
    }
    terms
}

/// Annual salary in whole currency units -> hourly cents (2080 hours/year), if it fits.
pub fn annual_to_hourly_cents(annual: f64) -> Option<i16> {
    if !(annual > 0.0) {
        return None;
    }
    let cents = (annual * 100.0 / 2080.0).round();
    if cents > i16::MAX as f64 { None } else { Some(cents as i16) }
}

/// Hourly rate in whole currency units -> hourly cents, if it fits.
pub fn hourly_to_cents(hourly: f64) -> Option<i16> {
    if !(hourly > 0.0) {
        return None;
    }
    let cents = (hourly * 100.0).round();
    if cents > i16::MAX as f64 { None } else { Some(cents as i16) }
}

/// Collapses an (optional lower, optional upper) pair so an equal range is a single value.
pub fn wage_pair(lower: Option<i16>, upper: Option<i16>) -> (Option<i16>, Option<i16>) {
    match (lower, upper) {
        (Some(l), Some(u)) if l == u => (Some(l), None),
        (None, Some(u)) => (Some(u), None),
        other => other,
    }
}

/// Parses the first amount in "$44,500.00/Yr." / "$150K" / "71000" into a number.
pub fn parse_money(raw: &str) -> Option<f64> {
    let start = raw.find(|c: char| c.is_ascii_digit())?;
    let rest = &raw[start..];
    let end = rest.find(|c: char| !(c.is_ascii_digit() || c == ',' || c == '.')).unwrap_or(rest.len());
    let number = rest[..end].trim_end_matches('.').replace(',', "");
    let value: f64 = number.parse().ok()?;
    let thousands = rest[end..].trim_start().starts_with(['k', 'K']);
    Some(if thousands { value * 1000.0 } else { value })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_html() {
        assert_eq!(html_to_text("<p>Hello&nbsp;<b>world</b></p><ul><li>one</li><li>two</li></ul>"), "Hello world one two");
        assert_eq!(html_to_text("&lt;p&gt;escaped&lt;/p&gt;"), "escaped");
    }

    #[test]
    fn terms() {
        let t = parse_terms("Summer 2027 Long Term Firmware Engineering Intern/Co-op");
        assert_eq!(t.iter().map(|t| t.display_name.as_str()).collect::<Vec<_>>(), vec!["S27"]);
        assert!(parse_terms("FPGA IP Design Engineer - Intern").is_empty());
        let t = parse_terms("Winter/Summer 2026 Intern");
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn money() {
        assert_eq!(parse_money("CAD $44,500.00/Yr."), Some(44500.0));
        assert_eq!(parse_money("$150K"), Some(150000.0));
        assert_eq!(annual_to_hourly_cents(44500.0), Some(2139));
        assert_eq!(wage_pair(Some(5), Some(5)), (Some(5), None));
    }
}
