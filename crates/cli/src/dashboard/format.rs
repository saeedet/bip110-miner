//! Numbers, the way a person would say them.
//!
//! Mining numbers span thirty orders of magnitude. Raw digits at that scale
//! convey nothing, and scientific notation conveys nothing to most people, so
//! everything here turns a number into words: "4.3 billion", "≈ 5,946 years".

use crate::commands::grouped;

/// A hashrate: `26.1 MH/s`.
pub fn rate(hashes_per_second: f64) -> String {
    const UNITS: [(f64, &str); 4] = [(1e12, "TH/s"), (1e9, "GH/s"), (1e6, "MH/s"), (1e3, "kH/s")];
    for (scale, unit) in UNITS {
        if hashes_per_second >= scale {
            return format!("{:.1} {unit}", hashes_per_second / scale);
        }
    }
    format!("{hashes_per_second:.0} H/s")
}

/// A large count in words: `4.3 billion`, `812 thousand`, `17`.
pub fn words(value: f64) -> String {
    const UNITS: [(f64, &str); 7] = [
        (1e21, "sextillion"),
        (1e18, "quintillion"),
        (1e15, "quadrillion"),
        (1e12, "trillion"),
        (1e9, "billion"),
        (1e6, "million"),
        (1e3, "thousand"),
    ];
    if value >= 1e24 {
        return format!("10{}", superscript(value.log10().floor() as u32));
    }
    for (scale, unit) in UNITS {
        if value >= scale {
            return format!("{} {unit}", significant(value / scale));
        }
    }
    significant(value)
}

/// Two significant figures, without a pointless `.0`: `4.3`, `17`, `812`.
fn significant(value: f64) -> String {
    if value >= 100.0 {
        format!("{value:.0}")
    } else if value >= 10.0 || value.fract() < 0.05 {
        format!("{:.0}", value.round())
    } else {
        format!("{value:.1}")
    }
}

/// A span of time in the largest unit that fits: `5,946 years`, `3 hours`.
pub fn duration(seconds: f64) -> String {
    const YEAR: f64 = 365.25 * 86_400.0;
    let (amount, unit) = if seconds >= 1_000_000.0 * YEAR {
        return format!("{} years", words(seconds / YEAR));
    } else if seconds >= YEAR {
        (seconds / YEAR, "year")
    } else if seconds >= 86_400.0 {
        (seconds / 86_400.0, "day")
    } else if seconds >= 3_600.0 {
        (seconds / 3_600.0, "hour")
    } else if seconds >= 60.0 {
        (seconds / 60.0, "minute")
    } else {
        (seconds.max(1.0), "second")
    };
    let amount = amount.round() as u64;
    format!("{} {unit}{}", grouped(amount), if amount == 1 { "" } else { "s" })
}

/// Time since start, compact: `45s`, `12m`, `2h14m`, `3d4h`.
///
/// `spaced` adds a space between the units, for wide screens: `2h 14m`.
pub fn uptime(seconds: u64, spaced: bool) -> String {
    let gap = if spaced { " " } else { "" };
    let (days, hours, minutes) = (seconds / 86_400, seconds / 3_600 % 24, seconds / 60 % 60);
    if days > 0 {
        format!("{days}d{gap}{hours}h")
    } else if hours > 0 {
        format!("{hours}h{gap}{minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m")
    } else {
        format!("{seconds}s")
    }
}

/// A number as superscript digits: `³⁴`.
pub fn superscript(number: u32) -> String {
    const DIGITS: [char; 10] = ['⁰', '¹', '²', '³', '⁴', '⁵', '⁶', '⁷', '⁸', '⁹'];
    number.to_string().chars().map(|c| DIGITS[c.to_digit(10).unwrap_or(0) as usize]).collect()
}

/// Satoshis as coins, without trailing zeros: `3.1262`.
pub fn coins(sats: u64) -> String {
    let text = format!("{}.{:08}", sats / 100_000_000, sats % 100_000_000);
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// Breaks `text` into lines of at most `columns`, between words.
pub fn wrap(text: &str, columns: usize) -> Vec<String> {
    let mut lines = vec![String::new()];
    // A word longer than a whole line — a file path, say — is cut into
    // pieces, rather than running off the edge.
    let pieces = text.split_whitespace().flat_map(|word| {
        let chars: Vec<char> = word.chars().collect();
        chars.chunks(columns.max(1)).map(|chunk| chunk.iter().collect::<String>()).collect::<Vec<_>>()
    });
    for word in pieces {
        let word = word.as_str();
        let line = lines.last_mut().expect("never empty");
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > columns {
            lines.push(word.to_owned());
        } else {
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates() {
        assert_eq!(rate(26_100_000.0), "26.1 MH/s");
        assert_eq!(rate(950.0), "950 H/s");
        assert_eq!(rate(1_500.0), "1.5 kH/s");
    }

    #[test]
    fn counts_in_words() {
        assert_eq!(words(4.3e9), "4.3 billion");
        assert_eq!(words(1.7e10), "17 billion");
        assert_eq!(words(812_000.0), "812 thousand");
        assert_eq!(words(15e6), "15 million");
        assert_eq!(words(3.0), "3");
    }

    #[test]
    fn durations() {
        assert_eq!(duration(5_946.0 * 365.25 * 86_400.0), "5,946 years");
        assert_eq!(duration(1.8e8 * 365.25 * 86_400.0), "180 million years");
        assert_eq!(duration(3.0 * 3_600.0), "3 hours");
        assert_eq!(duration(60.0), "1 minute");
        assert_eq!(duration(0.2), "1 second");
    }

    #[test]
    fn uptimes() {
        assert_eq!(uptime(2 * 3_600 + 14 * 60, false), "2h14m");
        assert_eq!(uptime(2 * 3_600 + 14 * 60, true), "2h 14m");
        assert_eq!(uptime(45, false), "45s");
    }

    #[test]
    fn small_print() {
        assert_eq!(superscript(34), "³⁴");
        assert_eq!(coins(312_620_000), "3.1262");
        assert_eq!(coins(5_000_000_000), "50");
    }
}
