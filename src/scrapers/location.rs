//! Best-effort parsing of free-text job locations into (city, region, country).

use crate::City;

/// (lowercase name, ISO 3166-1 alpha-2)
const COUNTRIES: &[(&str, &str)] = &[
    ("canada", "CA"),
    ("united states", "US"), ("united states of america", "US"), ("usa", "US"), ("u.s.", "US"), ("u.s.a.", "US"),
    ("united kingdom", "GB"), ("uk", "GB"), ("england", "GB"), ("scotland", "GB"),
    ("germany", "DE"), ("india", "IN"), ("singapore", "SG"), ("israel", "IL"), ("taiwan", "TW"),
    ("china", "CN"), ("japan", "JP"), ("south korea", "KR"), ("korea", "KR"), ("malaysia", "MY"),
    ("ireland", "IE"), ("france", "FR"), ("netherlands", "NL"), ("poland", "PL"), ("australia", "AU"),
    ("mexico", "MX"), ("brazil", "BR"), ("costa rica", "CR"), ("vietnam", "VN"), ("philippines", "PH"),
    ("czech republic", "CZ"), ("czechia", "CZ"), ("spain", "ES"), ("italy", "IT"), ("sweden", "SE"),
    ("switzerland", "CH"), ("finland", "FI"), ("denmark", "DK"), ("belgium", "BE"), ("austria", "AT"),
    ("hungary", "HU"), ("romania", "RO"), ("turkey", "TR"), ("united arab emirates", "AE"),
    ("saudi arabia", "SA"), ("egypt", "EG"), ("south africa", "ZA"), ("argentina", "AR"),
    ("colombia", "CO"), ("chile", "CL"), ("new zealand", "NZ"), ("thailand", "TH"), ("indonesia", "ID"),
    ("hong kong", "HK"), ("slovakia", "SK"), ("portugal", "PT"), ("norway", "NO"), ("serbia", "RS"),
    ("ukraine", "UA"), ("greece", "GR"), ("armenia", "AM"), ("bulgaria", "BG"), ("lithuania", "LT"),
    ("estonia", "EE"), ("latvia", "LV"), ("croatia", "HR"), ("slovenia", "SI"), ("luxembourg", "LU"),
];

/// (lowercase name, code, country)
const REGIONS: &[(&str, &str, &str)] = &[
    ("alberta", "AB", "CA"), ("british columbia", "BC", "CA"), ("manitoba", "MB", "CA"),
    ("new brunswick", "NB", "CA"), ("newfoundland and labrador", "NL", "CA"), ("nova scotia", "NS", "CA"),
    ("ontario", "ON", "CA"), ("prince edward island", "PE", "CA"), ("quebec", "QC", "CA"), ("québec", "QC", "CA"),
    ("saskatchewan", "SK", "CA"), ("northwest territories", "NT", "CA"), ("nunavut", "NU", "CA"), ("yukon", "YT", "CA"),
    ("alabama", "AL", "US"), ("alaska", "AK", "US"), ("arizona", "AZ", "US"), ("arkansas", "AR", "US"),
    ("california", "CA", "US"), ("colorado", "CO", "US"), ("connecticut", "CT", "US"), ("delaware", "DE", "US"),
    ("district of columbia", "DC", "US"), ("florida", "FL", "US"), ("georgia", "GA", "US"), ("hawaii", "HI", "US"),
    ("idaho", "ID", "US"), ("illinois", "IL", "US"), ("indiana", "IN", "US"), ("iowa", "IA", "US"),
    ("kansas", "KS", "US"), ("kentucky", "KY", "US"), ("louisiana", "LA", "US"), ("maine", "ME", "US"),
    ("maryland", "MD", "US"), ("massachusetts", "MA", "US"), ("michigan", "MI", "US"), ("minnesota", "MN", "US"),
    ("mississippi", "MS", "US"), ("missouri", "MO", "US"), ("montana", "MT", "US"), ("nebraska", "NE", "US"),
    ("nevada", "NV", "US"), ("new hampshire", "NH", "US"), ("new jersey", "NJ", "US"), ("new mexico", "NM", "US"),
    ("new york", "NY", "US"), ("north carolina", "NC", "US"), ("north dakota", "ND", "US"), ("ohio", "OH", "US"),
    ("oklahoma", "OK", "US"), ("oregon", "OR", "US"), ("pennsylvania", "PA", "US"), ("rhode island", "RI", "US"),
    ("south carolina", "SC", "US"), ("south dakota", "SD", "US"), ("tennessee", "TN", "US"), ("texas", "TX", "US"),
    ("utah", "UT", "US"), ("vermont", "VT", "US"), ("virginia", "VA", "US"), ("washington", "WA", "US"),
    ("west virginia", "WV", "US"), ("wisconsin", "WI", "US"), ("wyoming", "WY", "US"),
];

fn country_code_from_name(lower: &str) -> Option<&'static str> {
    COUNTRIES.iter().find(|(name, _)| *name == lower).map(|(_, code)| *code)
}

fn is_country_code(upper: &str) -> bool {
    COUNTRIES.iter().any(|(_, code)| *code == upper)
}

fn region_from_name(lower: &str) -> Option<(&'static str, &'static str)> {
    REGIONS.iter().find(|(name, _, _)| *name == lower).map(|(_, code, country)| (*code, *country))
}

fn region_from_code(upper: &str) -> Option<(&'static str, &'static str)> {
    REGIONS.iter().find(|(_, code, _)| *code == upper).map(|(_, code, country)| (*code, *country))
}

/// Full country name for a code, e.g. "CA" -> "canada".
pub fn country_name(code: &str) -> Option<&'static str> {
    COUNTRIES.iter().find(|(_, c)| c.eq_ignore_ascii_case(code)).map(|(name, _)| *name)
}

/// Full region name for a code, e.g. "ON" -> "ontario".
pub fn region_name(code: &str) -> Option<&'static str> {
    REGIONS.iter().find(|(_, c, _)| c.eq_ignore_ascii_case(code)).map(|(name, _, _)| *name)
}

/// Region code for a full name or code, e.g. "Ontario" -> "ON". Unknown values pass through.
pub fn normalize_region(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some((code, _)) = region_from_name(&trimmed.to_lowercase()) {
        return code.to_string();
    }
    trimmed.to_string()
}

/// Country code for a full name or code, e.g. "Canada" -> "CA". Unknown values pass through.
pub fn normalize_country(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(code) = country_code_from_name(&trimmed.to_lowercase()) {
        return code.to_string();
    }
    if trimmed.len() == 2 && is_country_code(&trimmed.to_uppercase()) {
        return trimmed.to_uppercase();
    }
    trimmed.to_string()
}

/// "MARKHAM" -> "Markham"; mixed-case input is left alone.
pub fn tidy_city_name(raw: &str) -> String {
    let trimmed = raw.trim();
    let all_caps = trimmed.chars().filter(|c| c.is_alphabetic()).all(|c| c.is_uppercase());
    if !all_caps {
        return trimmed.to_string();
    }
    trimmed
        .split(' ')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Parses strings such as "Toronto, Ontario, Canada", "US, CA, Santa Clara",
/// "CA, ON, Markham", "Santa Clara, CA" or "San Jose".
pub fn parse_location(raw: &str) -> City {
    let tokens: Vec<&str> = raw.split(',').map(|t| t.trim()).filter(|t| !t.is_empty()).collect();

    let mut city = String::new();
    let mut region = String::new();
    let mut country = String::new();
    // Set when a region name was provisionally taken as the city ("California"
    // in "US, California, Santa Clara"); a later plain token demotes it to region.
    let mut city_is_region: Option<(&str, &str)> = None;

    for token in &tokens {
        let lower = token.to_lowercase();

        if let Some(code) = country_code_from_name(&lower) {
            if country.is_empty() {
                country = code.to_string();
            }
            continue;
        }

        if token.len() == 2 && token.chars().all(|c| c.is_ascii_alphabetic()) {
            let upper = token.to_uppercase();
            if city.is_empty() && country.is_empty() && is_country_code(&upper) {
                country = upper;
            } else if region.is_empty() && region_from_code(&upper).is_some() {
                let (code, ctry) = region_from_code(&upper).unwrap();
                region = code.to_string();
                if country.is_empty() {
                    country = ctry.to_string();
                }
            } else if country.is_empty() && is_country_code(&upper) {
                country = upper;
            } else if city.is_empty() {
                city = token.to_string();
            }
            continue;
        }

        if let Some((code, ctry)) = region_from_name(&lower) {
            // A region name before any city ("New York, NY") is usually the city itself.
            if city.is_empty() {
                city = token.to_string();
                city_is_region = Some((code, ctry));
            } else if region.is_empty() {
                region = code.to_string();
                if country.is_empty() {
                    country = ctry.to_string();
                }
            }
            continue;
        }

        if city.is_empty() {
            city = token.to_string();
        } else if let Some((code, ctry)) = city_is_region.take() {
            // "US, California, Santa Clara": the earlier region name was not the city after all.
            if region.is_empty() {
                region = code.to_string();
                if country.is_empty() {
                    country = ctry.to_string();
                }
            }
            city = token.to_string();
        }
    }

    if city.is_empty() {
        city = if !region.is_empty() {
            region_name(&region).map(|n| tidy_city_name(&n.to_uppercase())).unwrap_or_else(|| region.clone())
        } else if !country.is_empty() {
            country_name(&country).map(|n| tidy_city_name(&n.to_uppercase())).unwrap_or_else(|| country.clone())
        } else {
            raw.trim().to_string()
        };
    }

    City { display_name: tidy_city_name(&city), region, country }
}

/// "Toronto, ON, CA" style label for embeds; skips empty parts.
pub fn format_city(city: &City) -> String {
    [city.display_name.as_str(), city.region.as_str(), city.country.as_str()]
        .iter()
        .filter(|p| !p.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(raw: &str, city: &str, region: &str, country: &str) {
        let parsed = parse_location(raw);
        assert_eq!(
            (parsed.display_name.as_str(), parsed.region.as_str(), parsed.country.as_str()),
            (city, region, country),
            "input: {raw}"
        );
    }

    #[test]
    fn common_shapes() {
        check("Toronto, Ontario, Canada", "Toronto", "ON", "CA");
        check("US, CA, Santa Clara", "Santa Clara", "CA", "US");
        check("CA, ON, Markham", "Markham", "ON", "CA");
        check("Santa Clara, CA", "Santa Clara", "CA", "US");
        check("Toronto, ON", "Toronto", "ON", "CA");
        check("San Jose", "San Jose", "", "");
        check("MARKHAM, Canada", "Markham", "", "CA");
        check("New York, NY, United States", "New York", "NY", "US");
        check("Malaysia, Penang", "Penang", "", "MY");
        check("Ontario, Canada", "Ontario", "", "CA");
        check("Vancouver, British Columbia, Canada", "Vancouver", "BC", "CA");
        check("US, California, Santa Clara", "Santa Clara", "CA", "US");
        check("US, Oregon, Hillsboro", "Hillsboro", "OR", "US");
        check("Canada, Ontario, Toronto", "Toronto", "ON", "CA");
    }

    #[test]
    fn normalizers() {
        assert_eq!(normalize_region("Ontario"), "ON");
        assert_eq!(normalize_region("British Columbia"), "BC");
        assert_eq!(normalize_country("Canada"), "CA");
        assert_eq!(normalize_country("United States"), "US");
        assert_eq!(tidy_city_name("MARKHAM"), "Markham");
        assert_eq!(tidy_city_name("Santa Clara"), "Santa Clara");
    }
}
