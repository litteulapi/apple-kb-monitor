//! Date arguments (`--since 7d`, `--until 2026-10-01`) without a date crate.

use akm_core::tr;

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

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Splits `HH:MM[:SS][.frac]` from a trailing `Z` or `±HH[:]MM` (offset in seconds east of UTC).
fn split_offset(t: &str) -> Option<(&str, Option<i64>)> {
    if let Some(c) = t.strip_suffix(['Z', 'z']) {
        return Some((c, Some(0)));
    }
    let Some(i) = t.find(['+', '-']) else {
        return Some((t, None));
    };
    let o = &t[i + 1..];
    let (oh, om) = o
        .split_once(':')
        .unwrap_or_else(|| o.split_at(o.len().min(2)));
    if oh.len() != 2 || om.len() != 2 {
        return None;
    }
    let (oh, om) = (num(oh)?, num(om)?);
    if oh > 23 || om > 59 {
        return None;
    }
    let sign = if t.as_bytes()[i] == b'+' { 1 } else { -1 };
    Some((&t[..i], Some(sign * (oh * 3600 + om * 60))))
}

type Civil = (i64, i64, i64, i64, i64, i64, Option<i64>);

#[allow(clippy::many_single_char_names)] // calendar fields: y m d
fn civil(s: &str) -> Option<Civil> {
    let s = s.trim();
    let (date, time) = match s.split_once([' ', 'T']) {
        Some((d, t)) => (d, Some(t)),
        None => (s, None),
    };
    let mut dp = date.split('-');
    let (y, m, d) = (num(dp.next()?)?, num(dp.next()?)?, num(dp.next()?)?);
    if dp.next().is_some()
        || !(1..=12).contains(&m)
        || !(1..=days_in_month(y, m)).contains(&d)
        || !(1970..=9999).contains(&y)
    {
        return None;
    }
    let (mut hh, mut mm, mut ss, mut off) = (0, 0, 0, None);
    if let Some(t) = time {
        let (t, o) = split_offset(t)?;
        off = o;
        let t = match t.split_once('.') {
            Some((t, frac)) => num(frac).map(|_| t)?,
            None => t,
        };
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
    Some((y, m, d, hh, mm, ss, off))
}

#[allow(clippy::many_single_char_names)] // calendar fields: y m d
pub fn parse_utc(s: &str) -> Option<u64> {
    let (y, m, d, hh, mm, ss, off) = civil(s)?;
    let t = days_from_civil(y, m, d) * 86_400 + hh * 3600 + mm * 60 + ss - off.unwrap_or(0);
    u64::try_from(t).ok()
}

#[allow(clippy::many_single_char_names)] // calendar fields: y m d
pub fn parse_iso_local(s: &str) -> Option<u64> {
    let (y, m, d, hh, mm, ss, off) = civil(s)?;
    if off.is_some() {
        return parse_utc(s);
    }
    let field = |v: i64| i32::try_from(v).ok();
    let (year, mon, mday) = (field(y - 1900)?, field(m - 1)?, field(d)?);
    let (hour, min, sec) = (field(hh)?, field(mm)?, field(ss)?);
    // SAFETY: `tm` is plain data, zero is a valid initial value; mktime only
    let t = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        tm.tm_year = year;
        tm.tm_mon = mon;
        tm.tm_mday = mday;
        tm.tm_hour = hour;
        tm.tm_min = min;
        tm.tm_sec = sec;
        tm.tm_isdst = -1;
        libc::mktime(&raw mut tm)
    };
    u64::try_from(t).ok()
}

pub fn parse_when(s: &str, now: u64) -> Result<u64, String> {
    let t = s.trim();
    let unit = t
        .chars()
        .last()
        .filter(|c| c.is_ascii_alphabetic() && !matches!(c, 'Z' | 'z'));
    if let Some(unit) = unit {
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
    tr!(
        "invalid date {s}: use 90m, 24h, 7d, 2w, YYYY-MM-DD or \"YYYY-MM-DD HH:MM\" (UTC)",
        s = format!("{s:?}")
    )
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
        assert_eq!(
            parse_when("2026-10-01 01:30", now),
            Ok(1_790_812_800 + 5400)
        );
        for bad in [
            "",
            "d",
            "7x",
            "2026-13-01",
            "2026-10-32",
            "1969-01-01",
            "yesterday",
            "2026-10",
            "2026-10-01 25:00",
            "2026-02-31",
            "2026-04-31",
            "2026-10-01T01:00:00+2",
            "2026-10-01T01:00:00+02:00x",
            "2026-10-01T01:00:00.x",
            "2026-10-01T01:00:00Zx",
        ] {
            assert!(parse_when(bad, now).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn offsets_and_z_are_applied_and_leap_days_checked() {
        let now = 1_790_000_000;
        let utc = 1_790_812_800 + 3600;
        assert_eq!(parse_when("2026-10-01T01:00:00Z", now), Ok(utc));
        assert_eq!(parse_when("2026-10-01T03:00:00+02:00", now), Ok(utc));
        assert_eq!(parse_when("2026-10-01T03:00+0200", now), Ok(utc));
        assert_eq!(parse_when("2026-09-30T23:00:00.5-02:00", now), Ok(utc));
        assert!(parse_when("2028-02-29", now).is_ok());
        assert!(parse_when("2026-02-29", now).is_err());
        assert_eq!(parse_iso_local("2026-10-01T03:00:00+02:00"), Some(utc));
    }

    #[test]
    fn python_local_timestamps_keep_their_spacing() {
        let a = parse_iso_local("2026-09-01T10:00:00.123456").unwrap();
        let b = parse_iso_local("2026-09-01T09:00:00").unwrap();
        assert_eq!(a - b, 3600);
        assert!(parse_iso_local("garbage").is_none());
    }
}
