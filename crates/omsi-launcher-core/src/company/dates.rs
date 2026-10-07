//! The company's calendar: dates as `YYYY-MM-DD` in the saved file (readable, and what the
//! timetable is asked for), as day numbers for counting. A week starts on Monday, as the
//! game's clock and the timetable's day masks have it.

/// Days since 1970-01-01 of a civil date (Howard Hinnant's algorithm).
pub fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let (y, m) = if m <= 2 { (y as i64 - 1, m as i64 + 9) } else { (y as i64, m as i64 - 3) };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + (153 * m + 2) / 5 + d as i64 - 1;
    era * 146097 + doe - 719468
}

/// The civil date of a day number.
pub fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((if m <= 2 { y + 1 } else { y }) as i32, m, d)
}

/// The day number of `YYYY-MM-DD` (None: not a date).
pub fn parse(s: &str) -> Option<i64> {
    let mut it = s.trim().split('-');
    let y: i32 = it.next()?.parse().ok()?;
    let m: u32 = it.next()?.parse().ok()?;
    let d: u32 = it.next()?.parse().ok()?;
    if !(1..=12).contains(&m) || d == 0 || d > days_in_month(y, m) || it.next().is_some() {
        return None;
    }
    Some(days_from_civil(y, m, d))
}

/// A day number as `YYYY-MM-DD`.
pub fn fmt(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{y:04}-{m:02}-{d:02}")
}

/// `date` moved by `n` days (a date that cannot be read stays as it is).
pub fn add(date: &str, n: i64) -> String {
    parse(date).map(|d| fmt(d + n)).unwrap_or_else(|| date.to_string())
}

/// Days from `a` to `b` (0 when either cannot be read).
pub fn between(a: &str, b: &str) -> i64 {
    match (parse(a), parse(b)) {
        (Some(a), Some(b)) => b - a,
        _ => 0,
    }
}

/// 0 = Monday … 6 = Sunday.
pub fn weekday(day: i64) -> u32 {
    // (1970-01-01 was a Thursday)
    ((day + 3).rem_euclid(7)) as u32
}

pub fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// `YYYY-MM` of a date: the month its bookings are counted in.
pub fn month_of(date: &str) -> String {
    date.get(..7).unwrap_or(date).to_string()
}

/// The date is the last of its month (the monthly bookings fall on it).
pub fn last_of_month(date: &str) -> bool {
    parse(date).map(|d| civil_from_days(d + 1).2 == 1).unwrap_or(false)
}

/// The Monday of the week a date is in, as a day number: the markets change with it.
pub fn week_of(date: &str) -> i64 {
    parse(date).map(|d| d - weekday(d) as i64).unwrap_or(0)
}

/// Whole years from `from` to `to` (fractional, for ages of buses).
pub fn years_between(from: &str, to: &str) -> f64 {
    (between(from, to) as f64 / 365.25).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_count_and_print() {
        assert_eq!(fmt(parse("1989-05-30").unwrap()), "1989-05-30");
        assert_eq!(add("2024-02-28", 1), "2024-02-29");
        assert_eq!(add("2023-02-28", 1), "2023-03-01");
        assert_eq!(add("2024-12-31", 1), "2025-01-01");
        assert_eq!(between("2024-01-01", "2024-03-01"), 60);
        // 1989-05-30 was a Tuesday, 2024-01-01 a Monday
        assert_eq!(weekday(parse("1989-05-30").unwrap()), 1);
        assert_eq!(weekday(parse("2024-01-01").unwrap()), 0);
        assert!(last_of_month("2024-02-29"));
        assert!(!last_of_month("2024-02-28"));
        assert_eq!(month_of("2024-02-29"), "2024-02");
        assert_eq!(week_of("2024-01-07"), parse("2024-01-01").unwrap());
        assert!(parse("2024-02-30").is_none());
        assert!(parse("nonsense").is_none());
    }
}
