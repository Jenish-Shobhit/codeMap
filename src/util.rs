//! Small helpers shared by every module: hashing, time formatting, text width.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

/// FNV-1a, 64 bit. Stable across runs and Rust versions, unlike `DefaultHasher`,
/// so it can key files on disk (reviewed hunks, repo keys).
pub fn fnv64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

pub fn fnv_hex(bytes: &[u8]) -> String {
    format!("{:016x}", fnv64(bytes))
}

static FIXED_NOW: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
static FORCE_UTC: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Freeze the clock and format times in UTC (snapshot tests).
pub fn freeze_time(now: i64) {
    FIXED_NOW.store(now, std::sync::atomic::Ordering::Relaxed);
    FORCE_UTC.store(true, std::sync::atomic::Ordering::Relaxed);
}

pub fn now_unix() -> i64 {
    let fixed = FIXED_NOW.load(std::sync::atomic::Ordering::Relaxed);
    if fixed != 0 {
        return fixed;
    }
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// "2m", "3h", "5d": how long ago `then` was.
pub fn ago(then: i64, now: i64) -> String {
    let secs = (now - then).max(0);
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else if secs < 86_400 * 60 {
        format!("{}d", secs / 86_400)
    } else if secs < 86_400 * 730 {
        format!("{}mo", secs / (86_400 * 30))
    } else {
        format!("{}y", secs / (86_400 * 365))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
}

/// Convert a unix timestamp to local wall-clock time with the C library.
pub fn local_time(unix: i64) -> LocalTime {
    let utc = FORCE_UTC.load(std::sync::atomic::Ordering::Relaxed);
    // SAFETY: localtime_r/gmtime_r only write into the provided struct.
    unsafe {
        let t: libc::time_t = unix as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        let res = if utc {
            libc::gmtime_r(&t, &mut tm)
        } else {
            libc::localtime_r(&t, &mut tm)
        };
        if res.is_null() {
            return LocalTime {
                year: 1970,
                month: 1,
                day: 1,
                hour: 0,
                minute: 0,
            };
        }
        LocalTime {
            year: tm.tm_year + 1900,
            month: (tm.tm_mon + 1) as u32,
            day: tm.tm_mday as u32,
            hour: tm.tm_hour as u32,
            minute: tm.tm_min as u32,
        }
    }
}

/// "9:44 PM"
pub fn clock(unix: i64) -> String {
    let t = local_time(unix);
    let (h, suffix) = match t.hour {
        0 => (12, "AM"),
        1..=11 => (t.hour, "AM"),
        12 => (12, "PM"),
        _ => (t.hour - 12, "PM"),
    };
    format!("{h}:{:02} {suffix}", t.minute)
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// "21 Sep 2026, 9:44 PM"
pub fn date_time(unix: i64) -> String {
    let t = local_time(unix);
    let month = MONTHS[(t.month.clamp(1, 12) - 1) as usize];
    format!("{} {} {}, {}", t.day, month, t.year, clock(unix))
}

/// Display width of a string in terminal cells.
pub fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

pub fn char_width(c: char) -> usize {
    UnicodeWidthChar::width(c).unwrap_or(0)
}

/// Cut `s` to at most `max` cells, adding `…` when it had to cut.
pub fn truncate(s: &str, max: usize) -> String {
    if width(s) <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = char_width(c);
        if used + w > max.saturating_sub(1) {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

/// Keep the end of `s` (useful for paths), prefixing `…` when cut.
pub fn truncate_left(s: &str, max: usize) -> String {
    if width(s) <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    let mut out: Vec<char> = Vec::new();
    let mut used = 0;
    for c in chars.iter().rev() {
        let w = char_width(*c);
        if used + w > max.saturating_sub(1) {
            break;
        }
        out.push(*c);
        used += w;
    }
    out.reverse();
    let mut s = String::from("…");
    s.extend(out);
    s
}

/// Pad with spaces on the right to exactly `w` cells (truncating if longer).
pub fn pad(s: &str, w: usize) -> String {
    let t = truncate(s, w);
    let used = width(&t);
    format!("{t}{}", " ".repeat(w.saturating_sub(used)))
}

/// Replace the home directory prefix with `~`.
pub fn tilde(path: &Path) -> String {
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        if let Ok(rest) = path.strip_prefix(&home) {
            if rest.as_os_str().is_empty() {
                return "~".to_string();
            }
            return format!("~/{}", rest.display());
        }
    }
    path.display().to_string()
}

/// Expand tabs to spaces (4 wide) and drop control characters so text can be
/// drawn cell by cell.
pub fn sanitize_line(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut col = 0;
    for c in s.chars() {
        if c == '\t' {
            let n = 4 - (col % 4);
            out.push_str(&" ".repeat(n));
            col += n;
        } else if c == '\r' || c == '\n' {
            continue;
        } else if c.is_control() {
            out.push('·');
            col += 1;
        } else {
            out.push(c);
            col += char_width(c);
        }
    }
    out
}

/// Thousands separator: 1204 -> "1,204".
pub fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// "1 file" / "2 files".
pub fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("{n} {one}")
    } else {
        format!("{n} {many}")
    }
}

/// Case-insensitive subsequence match score (higher is better); None if no match.
pub fn fuzzy_score(needle: &str, hay: &str) -> Option<i64> {
    if needle.is_empty() {
        return Some(0);
    }
    let n: Vec<char> = needle.to_lowercase().chars().collect();
    let h: Vec<char> = hay.to_lowercase().chars().collect();
    let hay_lower: String = h.iter().collect();
    let needle_lower: String = n.iter().collect();
    if let Some(pos) = hay_lower.find(&needle_lower) {
        // Substring matches win; earlier and exact matches win more.
        let exact = if hay_lower == needle_lower { 500 } else { 0 };
        return Some(1000 + exact - pos as i64 - (h.len() as i64 - n.len() as i64));
    }
    let mut score = 0i64;
    let mut hi = 0;
    let mut last = None;
    for c in &n {
        let mut found = false;
        while hi < h.len() {
            if h[hi] == *c {
                if let Some(l) = last {
                    if hi == l + 1 {
                        score += 5;
                    }
                }
                last = Some(hi);
                hi += 1;
                found = true;
                break;
            }
            hi += 1;
        }
        if !found {
            return None;
        }
        score += 1;
    }
    Some(score - h.len() as i64 / 4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv_is_stable() {
        assert_eq!(fnv_hex(b"codemap"), fnv_hex(b"codemap"));
        assert_ne!(fnv64(b"a"), fnv64(b"b"));
    }

    #[test]
    fn truncation_keeps_width() {
        assert_eq!(truncate("hello world", 5), "hell…");
        assert_eq!(truncate("hi", 5), "hi");
        assert_eq!(width(&pad("ab", 4)), 4);
        assert_eq!(truncate_left("src/very/long/path.rs", 10), "…g/path.rs");
        assert!(width(&truncate_left("src/very/long/path.rs", 10)) <= 10);
    }

    #[test]
    fn ago_units() {
        assert_eq!(ago(0, 30), "30s");
        assert_eq!(ago(0, 120), "2m");
        assert_eq!(ago(0, 7200), "2h");
        assert_eq!(ago(0, 86_400 * 3), "3d");
    }

    #[test]
    fn fuzzy_prefers_substring() {
        let a = fuzzy_score("sel", "selector").unwrap();
        let b = fuzzy_score("sel", "some_else_long").unwrap();
        assert!(a > b);
        assert!(fuzzy_score("xyz", "selector").is_none());
    }

    #[test]
    fn thousands_sep() {
        assert_eq!(thousands(1204), "1,204");
        assert_eq!(thousands(12), "12");
        assert_eq!(thousands(1_000_000), "1,000,000");
    }
}
