//! Minimal ISO-8601 timestamp parsing.
//!
//! The workspace has no date crate, and pulling one in for a single field is
//! not worth it. The parser accepts the subset the fleet harness writes:
//! `YYYY-MM-DDTHH:MM:SS` with an optional `.fraction` and a zone of `Z` or a
//! numeric offset (`±HH:MM`, `±HHMM`, `±HH`). The result is whole epoch
//! seconds — a fraction truncates — which is all the window filter compares.
//!
//! It is strict on shape: two-digit month/day, four-digit year, validated
//! calendar dates (no 31 February), and bounded hour/minute/second fields. A
//! timestamp the aggregator cannot place must not silently drop the run from
//! the window.

/// Parse `value` as ISO-8601 and return epoch seconds.
pub fn parse_iso8601(value: &str) -> Result<i64, String> {
    const ERR: &str =
        "timestamp expects ISO-8601 (e.g. 2026-09-01T12:00:00Z; offsets like +02:00 allowed)";
    let (date, time) = value
        .split_once('T')
        .ok_or_else(|| format!("{ERR}, got {value}"))?;
    let (year, month, day) = parse_date(date).ok_or_else(|| format!("{ERR}, got {value}"))?;
    let (secs_of_day, offset_secs) =
        parse_time_zone(time).ok_or_else(|| format!("{ERR}, got {value}"))?;
    Ok(days_from_civil(year, month as i64, day as i64) * 86_400 + secs_of_day - offset_secs)
}

fn parse_date(text: &str) -> Option<(i64, u32, u32)> {
    let parts: Vec<&str> = text.split('-').collect();
    if parts.len() != 3 || parts[0].len() != 4 || parts[1].len() != 2 || parts[2].len() != 2 {
        return None;
    }
    let year: i64 = parts[0].parse().ok()?;
    let month: u32 = parts[1].parse().ok()?;
    let day: u32 = parts[2].parse().ok()?;
    if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
        return None;
    }
    Some((year, month, day))
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
            if leap {
                29
            } else {
                28
            }
        }
    }
}

/// Split `HH:MM:SS[.frac](Z|±HH:MM|±HHMM|±HH)?` into its clock seconds and
/// its zone offset in seconds.
fn parse_time_zone(text: &str) -> Option<(i64, i64)> {
    let (clock, zone) = split_zone(text)?;
    let secs_of_day = parse_clock(clock)?;
    let offset_secs = parse_zone(zone)?;
    Some((secs_of_day, offset_secs))
}

fn split_zone(text: &str) -> Option<(&str, &str)> {
    if let Some(stripped) = text.strip_suffix(['Z', 'z']) {
        return Some((stripped, "Z"));
    }
    // The offset sign is the first `+`/`-` after the `HH:MM` prefix; an
    // earlier one would be inside the clock and is not a zone.
    for (index, byte) in text.as_bytes().iter().enumerate() {
        if index >= 6 && (*byte == b'+' || *byte == b'-') {
            return Some((&text[..index], &text[index..]));
        }
    }
    Some((text, ""))
}

fn parse_clock(text: &str) -> Option<i64> {
    let (clock, _fraction) = match text.split_once('.') {
        Some((clock, fraction)) => {
            if !fraction.is_empty() && !fraction.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            (clock, fraction)
        }
        None => (text, ""),
    };
    let parts: Vec<&str> = clock.split(':').collect();
    if parts.len() != 3 || parts[0].len() != 2 || parts[1].len() != 2 || parts[2].len() != 2 {
        return None;
    }
    let hour: u32 = parts[0].parse().ok()?;
    let minute: u32 = parts[1].parse().ok()?;
    let second: u32 = parts[2].parse().ok()?;
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    Some(i64::from(hour) * 3600 + i64::from(minute) * 60 + i64::from(second))
}

fn parse_zone(zone: &str) -> Option<i64> {
    if zone.is_empty() || zone == "Z" || zone == "z" {
        return Some(0);
    }
    let (sign, digits) = zone.split_at(1);
    let sign = if sign == "+" {
        1
    } else if sign == "-" {
        -1
    } else {
        return None;
    };
    let digits: String = digits.chars().filter(|c| *c != ':').collect();
    if (digits.len() != 2 && digits.len() != 4) || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let hours: i64 = digits[..2].parse().ok()?;
    let minutes: i64 = if digits.len() == 4 {
        digits[2..].parse().ok()?
    } else {
        0
    };
    if hours > 23 || minutes > 59 {
        return None;
    }
    Some(sign * (hours * 3600 + minutes * 60))
}

/// Days between the civil calendar date and 1970-01-01 (Howard Hinnant's
/// `days_from_civil`), valid for all proleptic Gregorian dates.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-09-01T00:00:00Z, checked against 2020-01-01T00:00:00Z
    /// (1_577_836_800) plus 2192 days through 2025 and 243 days into 2026.
    const SEP_1_2026: i64 = 1_788_220_800;

    #[test]
    fn parses_zulu_and_offsets_to_the_same_instant() {
        assert_eq!(parse_iso8601("2026-09-01T00:00:00Z"), Ok(SEP_1_2026));
        assert_eq!(
            parse_iso8601("2026-09-01T12:00:00Z"),
            Ok(SEP_1_2026 + 43_200)
        );
        assert_eq!(
            parse_iso8601("2026-09-01T14:00:00+02:00"),
            Ok(SEP_1_2026 + 43_200)
        );
        assert_eq!(
            parse_iso8601("2026-09-01T07:00:00-05:00"),
            Ok(SEP_1_2026 + 43_200)
        );
        assert_eq!(
            parse_iso8601("2026-09-01T16:00:00+04"),
            Ok(SEP_1_2026 + 43_200)
        );
        assert_eq!(
            parse_iso8601("2026-09-01T13:00:00.250Z"),
            Ok(SEP_1_2026 + 46_800) // 13:00:00Z, fraction truncated
        );
        assert_eq!(parse_iso8601("2026-01-01T00:00:00Z"), Ok(1_767_225_600));
        assert_eq!(parse_iso8601("1969-12-31T23:59:59Z"), Ok(-1));
    }

    #[test]
    fn leap_days_are_validated() {
        assert!(parse_iso8601("2024-02-29T00:00:00Z").is_ok());
        assert!(parse_iso8601("2023-02-29T00:00:00Z").is_err());
        assert!(parse_iso8601("2000-02-29T00:00:00Z").is_ok());
        assert!(parse_iso8601("1900-02-29T00:00:00Z").is_err());
    }

    #[test]
    fn malformed_input_is_rejected() {
        let cases = [
            "2026-02-30T00:00:00Z",
            "2026-13-01T00:00:00Z",
            "2026-09-01 12:00:00Z",
            "2026-9-1T12:00:00Z",
            "2026-09-01T25:00:00Z",
            "2026-09-01T12:60:00Z",
            "2026-09-01T12:00:60Z",
            "2026-09-01T12:00:00+24:00",
            "2026-09-01T12:00:00+02:60",
            "2026-09-01T12:00:00.5xZ",
            "2026-09-01T12:00:00Q",
            "2026-09-01T12:00Z",
            "2026-09-01T16:00+04",
            "yesterday",
            "",
        ];
        for case in &cases {
            assert!(parse_iso8601(case).is_err(), "{case:?} must be rejected");
        }
    }
}
