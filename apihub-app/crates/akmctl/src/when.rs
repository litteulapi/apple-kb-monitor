//! Date arguments (`--since 7d`, `--until 2026-10-01`) without a date crate.

/// Days since 1970-01-01 of a proleptic Gregorian date (H. Hinnant); the
/// inverse of `akm_core::history::format_utc`.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn num(s: &str) -> Option<i64> {
    (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse().ok())?
}

/// `YYYY-MM-DD[ T]HH:MM[:SS]` or `YYYY-MM-DD` split into calendar fields.
fn civil(s: &str) -> Option<(i64, i64, i64, i64, i64, i64)> {
    let s = s.trim();
    let (date, time) = match s.split_once([' ', 'T']) {
        Some((d, t)) => (d, Some(t)),
        None => (s, None),
    };
    let mut dp = date.split('-');
    let (y, m, d) = (num(dp.next()?)?, num(dp.next()?)?, num(dp.next()?)?);
    if dp.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) || !(1970..=9999).contains(&y) {
        return None;
    }
    let (mut hh, mut mm, mut ss) = (0, 0, 0);
    if let Some(t) = time {
        // Drop fractional seconds and any zone suffix.
        let t = t.split(['.', '+', 'Z']).next()?;
        let mut tp = t.split(':');
        hh = num(tp.next()?)?;
        mm = num(tp.next()?)?;
        if let Some(x) = tp.next() {
            ss = num(x)?;
        }
        if tp.next().is_some() || hh > 23 || mm > 59 || ss > 60 {
            return None;
        }
    }
    Some((y, m, d, hh, mm, ss))
}

/// Unix time of a UTC date (`2026-10-01` or `2026-10-01 14:30`).
pub fn parse_utc(s: &str) -> Option<u64> {
    let (y, m, d, hh, mm, ss) = civil(s)?;
    let t = days_from_civil(y, m, d) * 86_400 + hh * 3600 + mm * 60 + ss;
    u64::try_from(t).ok()
}

/// Unix time of a naive LOCAL date-time, as written by the old Python CLI
/// (`datetime.now().isoformat()`).
pub fn parse_iso_local(s: &str) -> Option<u64> {
    let (y, m, d, hh, mm, ss) = civil(s)?;
    // SAFETY: `tm` is plain data, zero is a valid initial value; mktime only
    // reads/normalises the struct we own.
    let t = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        tm.tm_year = (y - 1900) as i32;
        tm.tm_mon = (m - 1) as i32;
        tm.tm_mday = d as i32;
        tm.tm_hour = hh as i32;
        tm.tm_min = mm as i32;
        tm.tm_sec = ss as i32;
        tm.tm_isdst = -1;
        libc::mktime(&mut tm)
    };
    u64::try_from(t).ok()
}

/// A `--since` / `--until` value: relative to `now` (`90m`, `24h`, `7d`, `2w`)
/// or an absolute UTC date.
pub fn parse_when(s: &str, now: u64) -> Result<u64, String> {
    let t = s.trim();
    if let Some(unit) = t.chars().last().filter(char::is_ascii_alphabetic) {
        let mul = match unit {
            'm' => 60,
            'h' => 3600,
            'd' => 86_400,
            'w' => 7 * 86_400,
            _ => return Err(bad(s)),
        };
        let n = num(&t[..t.len() - 1]).ok_or_else(|| bad(s))?;
        return Ok(now.saturating_sub(u64::try_from(n).map_err(|_| bad(s))?.saturating_mul(mul)));
    }
    parse_utc(t).ok_or_else(|| bad(s))
}

fn bad(s: &str) -> String {
    format!("invalid date {s:?}: use 90m, 24h, 7d, 2w, YYYY-MM-DD or \"YYYY-MM-DD HH:MM\" (UTC)")
}

#[cfg(test)]
mod tests {
    use super::*;
    use akm_core::history::format_utc;

    #[test]
    fn utc_round_trips_with_format_utc() {
        for ts in [0u64, 86_399, 1_790_000_000, 1_780_000_020, 951_782_400] {
            let f = format_utc(ts);
            assert_eq!(parse_utc(&f), Some(ts - ts % 60), "{f}");
        }
        assert_eq!(parse_utc("2026-10-01"), Some(1_790_812_800));
    }

    #[test]
    fn relative_and_absolute_values() {
        let now = 1_790_000_000;
        assert_eq!(parse_when("24h", now), Ok(now - 86_400));
        assert_eq!(parse_when("7d", now), Ok(now - 7 * 86_400));
        assert_eq!(parse_when("90m", now), Ok(now - 5400));
        assert_eq!(parse_when("2w", now), Ok(now - 14 * 86_400));
        assert_eq!(parse_when("2026-10-01 01:30", now), Ok(1_790_812_800 + 5400));
        for bad in ["", "d", "7x", "2026-13-01", "2026-10-32", "1969-01-01", "yesterday", "2026-10", "2026-10-01 25:00"] {
            assert!(parse_when(bad, now).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn python_local_timestamps_keep_their_spacing() {
        let a = parse_iso_local("2026-09-01T10:00:00.123456").unwrap();
        let b = parse_iso_local("2026-09-01T09:00:00").unwrap();
        assert_eq!(a - b, 3600);
        assert!(parse_iso_local("garbage").is_none());
    }
}
