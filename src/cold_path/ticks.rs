//! Decimal string → integer tick conversion (ColdPath only).

use num_decimal::Num;

/// Number of decimal places preserved in [`num_to_tick_u64`].
pub const PRICE_DECIMALS: u32 = 8;

/// Scale factor `10^PRICE_DECIMALS` for tick integers.
pub const PRICE_TICK_SCALE: u128 = 100_000_000;

/// Convert Alpaca `Num` to unsigned ticks at [`PRICE_TICK_SCALE`].
///
/// Uses `Num`’s string rendering on the **cold** path (acceptable vs JSON parse cost).
pub fn num_to_tick_u64(n: &Num) -> Option<u64> {
    let s = n.to_string();
    parse_decimal_to_scaled_u64(&s, PRICE_DECIMALS)
}

fn parse_decimal_to_scaled_u64(s: &str, decimals: u32) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let neg = s.starts_with('-');
    let s = s.strip_prefix('-').unwrap_or(s);

    let (int_part, frac_part) = match s.split_once('.') {
        Some((a, b)) => (a, b),
        None => (s, ""),
    };

    if int_part.is_empty() && frac_part.is_empty() {
        return None;
    }

    let int_part = if int_part.is_empty() {
        "0"
    } else {
        int_part
    };

    let mut frac = frac_part.to_string();
    if (frac.len() as u32) < decimals {
        frac.push_str(&"0".repeat(decimals as usize - frac.len()));
    } else if (frac.len() as u32) > decimals {
        frac.truncate(decimals as usize);
    }

    let combined = format!("{int_part}{frac}");
    let v: u128 = combined.parse().ok()?;
    if neg {
        return None;
    }
    u64::try_from(v).ok()
}

/// Floor a positive decimal `Num` to a whole-lot `u64` (cold path).
pub fn num_to_whole_shares(n: &Num) -> u64 {
    let v: f64 = n.to_string().parse().unwrap_or(0.0);
    if v <= 0.0 {
        1
    } else {
        (v.floor() as u64).max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn parses_price() {
        let n = Num::from_str("123.45").unwrap();
        let t = num_to_tick_u64(&n).unwrap();
        assert_eq!(t as u128, 123_45000000u128);
    }

    #[test]
    fn parse_decimal_to_scaled() {
        assert_eq!(
            parse_decimal_to_scaled_u64("10.5", 8).unwrap() as u128,
            1_050_000_000u128
        );
    }
}
