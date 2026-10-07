//! One file per command.

pub mod doctor;
pub mod setup;
pub mod start;
pub mod status;
pub mod stop;
pub mod wallet;

/// Shortens an address for display: `bc1qjxn0…yms0`.
pub fn short_address(address: &str) -> String {
    if address.len() <= 16 {
        return address.to_owned();
    }
    format!("{}…{}", &address[..8], &address[address.len() - 4..])
}

/// Groups digits in threes: `976,039`.
pub fn grouped(number: u64) -> String {
    let digits = number.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_for_people() {
        assert_eq!(grouped(976_039), "976,039");
        assert_eq!(grouped(12), "12");
        assert_eq!(short_address("bc1qjxn0hvcv58wyagt77vv0ff0ld6jhy6ch9myms0"), "bc1qjxn0…yms0");
    }
}
