//! `vec.*`, `fmt.*` and `util.*`: the small helpers every plugin writes again otherwise -
//! distances and headings on the map, times and numbers as text, the real clock.

use super::{def, o, p};
use crate::api::{ApiError, ApiFn, Args, Value};

const NEW: &str = crate::api::VERSION;

/// Arguments `from..` as an (x, y) or (x, y, z) vector.
fn vecn(a: &Args, from: usize, n: usize) -> Result<Vec<f64>, ApiError> {
    (from..from + n).map(|i| a.num(i)).collect()
}

/// A number with `d` decimals and `sep` between the thousands.
fn number(n: f64, d: usize, sep: &str) -> String {
    let s = format!("{:.*}", d, n.abs());
    let (int, frac) = s.split_once('.').map_or((s.as_str(), None), |(i, f)| (i, Some(f)));
    let mut out = String::new();
    for (i, ch) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push_str(sep);
        }
        out.push(ch);
    }
    if let Some(f) = frac {
        out.push('.');
        out.push_str(f);
    }
    if n < 0.0 && out.chars().any(|c| c.is_ascii_digit() && c != '0') {
        out.insert(0, '-');
    }
    out
}

/// Seconds since midnight as `HH:MM:SS` (or `HH:MM`), past midnight and before it wrapped.
pub(crate) fn clock_text(t: f64, seconds: bool) -> String {
    let t = t.rem_euclid(86400.0).floor() as i64;
    if seconds {
        format!("{:02}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60)
    } else {
        format!("{:02}:{:02}", t / 3600, t / 60 % 60)
    }
}

fn duration(s: f64) -> String {
    let neg = s < 0.0;
    let t = s.abs().round() as i64;
    let body = if t >= 3600 {
        format!("{} h {:02} min", t / 3600, t / 60 % 60)
    } else if t >= 60 {
        format!("{} min {:02} s", t / 60, t % 60)
    } else {
        format!("{t} s")
    };
    if neg { format!("-{body}") } else { body }
}

/// The days since 1970-01-01 of a date, and back (proleptic Gregorian).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub static FNS: &[ApiFn] = &[
    def!("vec.length", "util", [p("x", "number"), p("y", "number"), o("z", "number")], "number", "The length of a vector (2D, or 3D with `z`).", NEW, None, false, |c, a| {
        let (x, y, z) = (a.num(0)?, a.num(1)?, a.opt_num(2)?.unwrap_or(0.0));
        Ok::<_, ApiError>((x * x + y * y + z * z).sqrt())
    }),
    def!("vec.distance", "util", [p("x1", "number"), p("y1", "number"), p("x2", "number"), p("y2", "number")], "number", "Metres between two map points (on the ground: x and y).", NEW, None, false, |c, a| {
        let v = vecn(&a, 0, 4)?;
        Ok::<_, ApiError>((v[2] - v[0]).hypot(v[3] - v[1]))
    }),
    def!("vec.distance3", "util", [p("x1", "number"), p("y1", "number"), p("z1", "number"), p("x2", "number"), p("y2", "number"), p("z2", "number")], "number", "Metres between two points in space.", NEW, None, false, |c, a| {
        let v = vecn(&a, 0, 6)?;
        Ok::<_, ApiError>(((v[3] - v[0]).powi(2) + (v[4] - v[1]).powi(2) + (v[5] - v[2]).powi(2)).sqrt())
    }),
    def!("vec.heading", "util", [p("dx", "number"), p("dy", "number")], "number", "The heading of a direction on the map, degrees clockwise from north (0 to 360), as the game's headings.", NEW, None, false, |c, a| {
        let (dx, dy) = (a.num(0)?, a.num(1)?);
        Ok::<_, ApiError>(dx.atan2(dy).to_degrees().rem_euclid(360.0))
    }),
    def!("vec.bearing", "util", [p("x1", "number"), p("y1", "number"), p("x2", "number"), p("y2", "number")], "number", "The heading from one map point to another.", NEW, None, false, |c, a| {
        let v = vecn(&a, 0, 4)?;
        Ok::<_, ApiError>((v[2] - v[0]).atan2(v[3] - v[1]).to_degrees().rem_euclid(360.0))
    }),
    def!("vec.angle_diff", "util", [p("a", "number"), p("b", "number")], "number", "The turn from heading `a` to heading `b`, -180 to 180 degrees (right positive).", NEW, None, false, |c, a| {
        let (x, y) = (a.num(0)?, a.num(1)?);
        Ok::<_, ApiError>((y - x + 180.0).rem_euclid(360.0) - 180.0)
    }),
    def!("vec.rotate", "util", [p("x", "number"), p("y", "number"), p("degrees", "number")], "x, y", "A vector turned clockwise by `degrees` (as headings turn).", NEW, None, true, |c, a| {
        let (x, y, d) = (a.num(0)?, a.num(1)?, a.num(2)?.to_radians());
        Ok::<_, ApiError>(Value::List(vec![Value::Num(x * d.cos() + y * d.sin()), Value::Num(-x * d.sin() + y * d.cos())]))
    }),
    def!("vec.normalize", "util", [p("x", "number"), p("y", "number"), o("z", "number")], "x, y, z", "The vector made 1 long (0 stays 0).", NEW, None, true, |c, a| {
        let (x, y, z) = (a.num(0)?, a.num(1)?, a.opt_num(2)?.unwrap_or(0.0));
        let l = (x * x + y * y + z * z).sqrt();
        let k = if l > 0.0 { 1.0 / l } else { 0.0 };
        Ok::<_, ApiError>(Value::List(vec![Value::Num(x * k), Value::Num(y * k), Value::Num(z * k)]))
    }),
    def!("vec.dot", "util", [p("x1", "number"), p("y1", "number"), p("x2", "number"), p("y2", "number")], "number", "The dot product of two 2D vectors.", NEW, None, false, |c, a| {
        let v = vecn(&a, 0, 4)?;
        Ok::<_, ApiError>(v[0] * v[2] + v[1] * v[3])
    }),
    def!("vec.lerp", "util", [p("a", "number"), p("b", "number"), p("t", "number")], "number", "From `a` to `b` by `t` (0 to 1, not held to it).", NEW, None, false, |c, a| {
        let (x, y, t) = (a.num(0)?, a.num(1)?, a.num(2)?);
        Ok::<_, ApiError>(x + (y - x) * t)
    }),
    def!("vec.clamp", "util", [p("x", "number"), p("min", "number"), p("max", "number")], "number", "`x` held between `min` and `max`.", NEW, None, false, |c, a| {
        let (x, lo, hi) = (a.num(0)?, a.num(1)?, a.num(2)?);
        Ok::<_, ApiError>(x.max(lo).min(hi))
    }),
    def!("vec.to_local", "util", [p("x", "number"), p("y", "number"), p("ox", "number"), p("oy", "number"), p("heading", "number")], "right, ahead", "A map point seen from a place facing `heading`: metres to the right and ahead of it.", NEW, None, true, |c, a| {
        let v = vecn(&a, 0, 5)?;
        let (dx, dy, h) = (v[0] - v[2], v[1] - v[3], v[4].to_radians());
        Ok::<_, ApiError>(Value::List(vec![Value::Num(dx * h.cos() - dy * h.sin()), Value::Num(dx * h.sin() + dy * h.cos())]))
    }),
    def!("fmt.clock", "util", [p("seconds", "number"), o("with_seconds", "bool")], "string", "Seconds since midnight as `\"HH:MM\"` (`\"HH:MM:SS\"` with `true`); past midnight wraps.", NEW, None, false, |c, a| {
        let t = a.num(0)?;
        Ok::<_, ApiError>(clock_text(t, a.flag(1, false)))
    }),
    def!("fmt.duration", "util", [p("seconds", "number")], "string", "A length of time as people read it: `\"45 s\"`, `\"3 min 05 s\"`, `\"1 h 20 min\"` (negative with a minus).", NEW, None, false, |c, a| a.num(0).map(duration)),
    def!("fmt.delay", "util", [p("seconds", "number")], "string", "A delay as a timetable display shows it: `\"+2:30\"` late, `\"-0:45\"` early, `\"0:00\"`.", NEW, None, false, |c, a| {
        let s = a.num(0)?.round() as i64;
        let sign = if s > 0 { "+" } else if s < 0 { "-" } else { "" };
        Ok::<_, ApiError>(format!("{sign}{}:{:02}", s.abs() / 60, s.abs() % 60))
    }),
    def!("fmt.number", "util", [p("x", "number"), o("decimals", "integer"), o("separator", "string")], "string", "A number with `decimals` (0) and a thousands `separator` (`\",\"`, `\" \"`; none by default).", NEW, None, false, |c, a| {
        let x = a.num(0)?;
        let d = a.opt_int(1)?.unwrap_or(0).clamp(0, 12) as usize;
        let sep = a.opt_str(2)?.unwrap_or_default();
        Ok::<_, ApiError>(number(x, d, &sep))
    }),
    def!("fmt.money", "util", [p("amount", "number"), o("symbol", "string")], "string", "An amount with two decimals and a currency symbol after it (`\"12.50 €\"`; the game knows no currency of its own).", NEW, None, false, |c, a| {
        let x = a.num(0)?;
        let sym = a.opt_str(1)?.unwrap_or_default();
        let n = number(x, 2, "");
        Ok::<_, ApiError>(if sym.is_empty() { n } else { format!("{n} {sym}") })
    }),
    def!("fmt.speed", "util", [p("kmh", "number"), o("unit", "string")], "string", "A speed as `\"42 km/h\"`, or in `\"mph\"` or `\"m/s\"`.", NEW, None, false, |c, a| {
        let k = a.num(0)?;
        Ok::<_, ApiError>(match a.opt_str(1)?.as_deref() {
            Some("mph") => format!("{:.0} mph", k / 1.609_344),
            Some("m/s") => format!("{:.1} m/s", k / 3.6),
            _ => format!("{k:.0} km/h"),
        })
    }),
    def!("fmt.distance", "util", [p("metres", "number")], "string", "A distance as `\"350 m\"` or `\"2.4 km\"`.", NEW, None, false, |c, a| {
        let m = a.num(0)?;
        Ok::<_, ApiError>(if m.abs() >= 1000.0 { format!("{:.1} km", m / 1000.0) } else { format!("{m:.0} m") })
    }),
    def!("fmt.pad", "util", [p("text", "string"), p("width", "integer"), o("right", "bool")], "string", "A text made `width` characters long with spaces (on the left with `right` = true, to line numbers up), or cut to it.", NEW, None, false, |c, a| {
        let (t, w) = (a.str(0)?, a.int(1)?.clamp(0, 500) as usize);
        let n = t.chars().count();
        Ok::<_, ApiError>(if n >= w { t.chars().take(w).collect() } else if a.flag(2, false) { format!("{}{t}", " ".repeat(w - n)) } else { format!("{t}{}", " ".repeat(w - n)) })
    }),
    def!("fmt.split", "util", [p("text", "string"), o("separator", "string")], "list of strings", "A text cut at every `separator` (`\",\"` by default; plainly, no patterns).", NEW, None, false, |c, a| {
        let t = a.str(0)?;
        let sep = a.opt_str(1)?.unwrap_or_else(|| ",".into());
        if sep.is_empty() {
            return Ok(Value::List(t.chars().map(|c| Value::Str(c.to_string())).collect()));
        }
        Ok::<_, ApiError>(Value::List(t.split(sep.as_str()).map(|s| Value::Str(s.to_string())).collect()))
    }),
    def!("fmt.trim", "util", [p("text", "string")], "string", "A text without the spaces at its ends.", NEW, None, false, |c, a| a.str(0).map(|s| s.trim().to_string())),
    def!("util.now", "util", [], "number", "The real time: seconds since 1970 (UTC), with fractions.", NEW, None, false, |c, a| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())),
    def!("util.date", "util", [o("seconds", "number")], "table", "A real time (`util.now()` by default) as `{year, month, day, hour, minute, second, weekday}` in UTC (`weekday` 1 Monday).", NEW, None, false, |c, a| {
        let t = a.opt_num(0)?.unwrap_or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64()));
        let days = (t / 86400.0).floor() as i64;
        let (y, m, d) = civil(days);
        let s = (t - days as f64 * 86400.0).floor() as i64;
        Ok::<_, ApiError>(super::rec(vec![("year", Value::Int(y)), ("month", m.into()), ("day", d.into()), ("hour", Value::Int(s / 3600)), ("minute", Value::Int(s / 60 % 60)), ("second", Value::Int(s % 60)), ("weekday", Value::Int((days + 3).rem_euclid(7) + 1))]))
    }),
    def!("util.ms", "util", [], "number", "Milliseconds of real time since the game started: for timing a plugin's own work.", NEW, None, false, |c, a| {
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64() * 1000.0
    }),
    def!("util.random", "util", [o("min", "number"), o("max", "number")], "number", "A random number: 0 to 1 without arguments, else a whole number from `min` to `max` (as `math.random`, but not repeating the same row in every plugin).", NEW, None, false, |c, a| {
        use std::hash::{BuildHasher, Hasher};
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64));
        let r = (h.finish() >> 11) as f64 / (1u64 << 53) as f64;
        Ok::<_, ApiError>(match (a.opt_int(0)?, a.opt_int(1)?) {
            (Some(lo), Some(hi)) if hi >= lo => Value::Int(lo + (r * (hi - lo + 1) as f64).floor() as i64),
            (Some(hi), None) if hi >= 1 => Value::Int(1 + (r * hi as f64).floor() as i64),
            _ => Value::Num(r),
        })
    }),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texts_of_numbers_and_times() {
        assert_eq!(number(1234567.891, 2, ","), "1,234,567.89");
        assert_eq!(number(-12.0, 0, " "), "-12");
        assert_eq!(number(-0.001, 1, ""), "0.0");
        assert_eq!(clock_text(90061.0, true), "01:01:01");
        assert_eq!(clock_text(-60.0, false), "23:59");
        assert_eq!(duration(3725.0), "1 h 02 min");
        assert_eq!(duration(-185.0), "-3 min 05 s");
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(19_723), (2024, 1, 1));
    }
}
