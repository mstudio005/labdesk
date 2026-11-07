// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

//! Money and date formatting for the screen.
use chrono::{DateTime, FixedOffset};

/// 1234567 paise -> "₹12,345.67"; whole rupees drop the decimals. Indian digit grouping.
pub fn rupees(paise: i64) -> String {
    let neg = paise < 0;
    let p = paise.unsigned_abs();
    let (r, ps) = (p / 100, p % 100);
    let s = r.to_string();
    let grouped = if s.len() <= 3 { s } else {
        let (head, tail) = s.split_at(s.len() - 3);
        let mut parts: Vec<String> = Vec::new();
        let hb = head.as_bytes();
        let mut end = hb.len();
        while end > 0 { let start = end.saturating_sub(2); parts.push(head[start..end].to_string()); end = start; }
        parts.reverse();
        format!("{},{}", parts.join(","), tail)
    };
    format!("{}₹{}{}", if neg { "-" } else { "" }, grouped, if ps > 0 { format!(".{ps:02}") } else { String::new() })
}

/// "1500", "1,500.50", "" -> paise. None if it is not a valid amount.
pub fn parse_rupees(s: &str) -> Option<i64> {
    let s = s.trim().replace([',', '₹'], "");
    if s.is_empty() { return Some(0); }
    let (whole, frac) = s.split_once('.').unwrap_or((&s, ""));
    if frac.len() > 2 || !whole.chars().all(|c| c.is_ascii_digit()) || !frac.chars().all(|c| c.is_ascii_digit()) { return None; }
    let w: i64 = if whole.is_empty() { 0 } else { whole.parse().ok()? };
    let f: i64 = if frac.is_empty() { 0 } else { format!("{frac:0<2}").parse().ok()? };
    w.checked_mul(100)?.checked_add(f)
}

/// ISO time from the database -> "12 Oct, 14:41" in Indian Standard Time.
pub fn when(iso: &str) -> String {
    let ist = FixedOffset::east_opt(5 * 3600 + 1800).expect("valid offset");
    DateTime::parse_from_rfc3339(iso).map(|d| d.with_timezone(&ist).format("%d %b, %H:%M").to_string()).unwrap_or_else(|_| iso.to_string())
}

/// ISO time -> "19 Oct 2026" in Indian Standard Time.
pub fn day(iso: &str) -> String {
    let ist = FixedOffset::east_opt(5 * 3600 + 1800).expect("valid offset");
    DateTime::parse_from_rfc3339(iso).map(|d| d.with_timezone(&ist).format("%d %b %Y").to_string()).unwrap_or_else(|_| iso.to_string())
}
/// ISO time -> "19 Oct 2026, 09:15" in Indian Standard Time.
pub fn when_full(iso: &str) -> String {
    let ist = FixedOffset::east_opt(5 * 3600 + 1800).expect("valid offset");
    DateTime::parse_from_rfc3339(iso).map(|d| d.with_timezone(&ist).format("%d %b %Y, %H:%M").to_string()).unwrap_or_else(|_| iso.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn money() {
        assert_eq!(rupees(0), "₹0");
        assert_eq!(rupees(35000), "₹350");
        assert_eq!(rupees(123456700), "₹12,34,567");
        assert_eq!(rupees(100050), "₹1,000.50");
        assert_eq!(rupees(-5000), "-₹50");
        assert_eq!(parse_rupees("1,500.5"), Some(150050));
        assert_eq!(parse_rupees(" "), Some(0));
        assert_eq!(parse_rupees("12.345"), None);
        assert_eq!(parse_rupees("abc"), None);
        assert_eq!(parse_rupees("-5"), None);
        assert_eq!(parse_rupees(".5"), Some(50));
    }
    #[test]
    fn dates() { assert_eq!(when("2026-10-12T09:11:00.000Z"), "12 Oct, 14:41"); }
}
