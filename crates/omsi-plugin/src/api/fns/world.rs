//! `world.*` and `weather.*`: the game's clock and date, its speed and pause, the season, and
//! the weather - read, and set as the game menu sets it.

use super::{def, multi, o, p, rec};
use crate::api::{ApiError, ApiFn, Value};
use crate::io::{Weather, WeatherChange};

const NEW: &str = crate::api::VERSION;

fn weather(w: Weather) -> Value {
    rec(vec![
        ("name", w.name.into()),
        ("visibility", Value::from(w.visibility_m)),
        ("wind_direction", Value::from(w.wind_dir)),
        ("wind_speed", Value::from(w.wind_ms)),
        ("temperature", Value::from(w.temperature)),
        ("humidity", Value::from(w.humidity_rel)),
        ("absolute_humidity", Value::from(w.humidity_abs)),
        ("pressure", Value::from(w.pressure)),
        ("clouds", w.clouds.into()),
        ("cloud_base", Value::from(w.cloud_base_m)),
        ("precipitation", ["none", "rain", "snow"].get(w.precip_kind.clamp(0, 2) as usize).copied().unwrap_or("none").into()),
        ("precipitation_rate", Value::from(w.precip_rate)),
        ("snow_cover", w.snow_cover.into()),
        ("snow_on_road", w.snow_on_road.into()),
        ("wetness", Value::from(w.wetness)),
        ("changing", w.changing.into()),
        ("locked", w.locked.into()),
    ])
}

/// `weather.set`'s table.
fn change(t: &Value) -> Result<WeatherChange, ApiError> {
    let num = |k: &str| -> Result<Option<f32>, ApiError> {
        match t.get(k) {
            None | Some(Value::Nil) => Ok(None),
            Some(v) => v.as_f64().map(|n| Some(n as f32)).ok_or_else(|| ApiError(format!("omsi.weather.set: {k} must be a number"))),
        }
    };
    let flag = |k: &str| t.get(k).filter(|v| !v.is_nil()).map(Value::truthy);
    let precip_kind = match t.get("precipitation").and_then(Value::to_text).as_deref() {
        None => None,
        Some("none") => Some(0),
        Some("rain") => Some(1),
        Some("snow") => Some(2),
        Some(s) => return Err(ApiError(format!("omsi.weather.set: precipitation \"{s}\" is none of none, rain, snow"))),
    };
    Ok(WeatherChange {
        visibility_m: num("visibility")?,
        wind_dir: num("wind_direction")?,
        wind_ms: num("wind_speed")?,
        temperature: num("temperature")?,
        pressure: num("pressure")?,
        clouds: t.get("clouds").and_then(Value::to_text),
        cloud_base_m: num("cloud_base")?,
        precip_kind,
        precip_rate: num("precipitation_rate")?,
        snow_cover: flag("snow_cover"),
        snow_on_road: flag("snow_on_road"),
        wetness: num("wetness")?,
    })
}

/// "HH:MM" or "HH:MM:SS", or seconds.
fn time_of(v: &Value) -> Option<f64> {
    if let Some(n) = v.as_f64() {
        return Some(n);
    }
    let s = v.as_str()?;
    let parts: Vec<f64> = s.split(':').map(|p| p.trim().parse::<f64>()).collect::<Result<_, _>>().ok()?;
    match parts.as_slice() {
        [h, m] => Some(h * 3600.0 + m * 60.0),
        [h, m, s] => Some(h * 3600.0 + m * 60.0 + s),
        _ => None,
    }
}

pub static FNS: &[ApiFn] = &[
    def!("world.time", "time", [], "number or nil", "The game's time of day, seconds since midnight.", NEW, None, false, |c, a| c.io().clock().map(|c| c.time)),
    def!("world.date", "time", [], "table or nil", "The game's date: `{year, month, day, weekday, day_of_year}` (`weekday` 1 Monday to 7 Sunday).", NEW, None, false, |c, a| {
        c.io().clock().map(|c| rec(vec![("year", c.year.into()), ("month", c.month.into()), ("day", c.day.into()), ("weekday", (c.weekday + 1).into()), ("day_of_year", c.day_of_year.into())]))
    }),
    def!("world.play_time", "time", [], "number or nil", "Seconds played this session (game time, never wraps).", NEW, None, false, |c, a| c.io().clock().map(|c| c.run_time)),
    def!("world.set_time", "time", [p("time", "any")], "boolean", "Sets the time of day: seconds since midnight or `\"HH:MM\"` / `\"HH:MM:SS\"`, as the game menu's clock does (the timetable starts again after a jump of minutes). Not in a LAN game as a client, nor while the clock follows the computer's.", NEW, WorldWrite, false, |c, a| {
        let t = time_of(a.get(0)).ok_or_else(|| ApiError("omsi.world.set_time: a time is seconds or \"HH:MM\"".into()))?;
        Ok::<_, ApiError>(c.io().set_time(t.rem_euclid(86400.0)))
    }),
    def!("world.set_date", "time", [p("year", "integer"), p("month", "integer"), p("day", "integer")], "boolean", "Sets the game's date (the season's textures follow).", NEW, WorldWrite, false, |c, a| {
        let (y, m, d) = (a.int(0)?, a.int(1)?, a.int(2)?);
        if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
            return Err(ApiError("omsi.world.set_date: month 1-12, day 1-31".into()));
        }
        Ok(c.io().set_date(y as i32, m as u32, d as u32))
    }),
    def!("world.time_speed", "time", [], "number or nil", "How much faster than real time the game's clock runs (1 to 30).", NEW, None, false, |c, a| c.io().time_speed()),
    def!("world.set_time_speed", "time", [p("factor", "number")], "boolean", "Sets how much faster the clock runs (1 to 30; not in a LAN game).", NEW, WorldWrite, false, |c, a| {
        let x = a.num(0)?.clamp(1.0, 30.0);
        Ok::<_, ApiError>(c.io().set_time_speed(x))
    }),
    def!("world.paused", "time", [], "boolean", "Whether the game stands still (a plugin hears `pause` and runs no more until `resume`).", NEW, None, false, |c, a| c.io().paused()),
    def!("world.pause", "time", [], "boolean", "Pauses the game, as P does (not in a LAN game). The plugins stand still with it: the player resumes it.", NEW, WorldWrite, false, |c, a| c.io().set_paused(true)),
    def!("world.season", "time", [], "folder, snow", "The season's texture folder (`nil`: the base textures) and whether snow lies.", NEW, None, true, |c, a| multi(c.io().season().map(|(f, s)| [Value::opt(f), Value::Bool(s)]))),
    def!("world.sun_altitude", "time", [], "number or nil", "The sun's height over the horizon, degrees.", NEW, None, false, |c, a| c.io().system("SunAlt").map(Value::from)),
    def!("weather.get", "weather", [], "table or nil", "The weather now: `{name, visibility (m), wind_direction (°), wind_speed (m/s), temperature (°C), humidity (%), absolute_humidity (g/m³), pressure (hPa), clouds, cloud_base (m), precipitation (\"none\", \"rain\", \"snow\"), precipitation_rate (0..1), snow_cover, snow_on_road, wetness (the roads, 0..1), changing, locked}` (`locked`: it follows a real weather station and cannot be set).", NEW, None, false, |c, a| c.io().weather().map(weather)),
    def!("weather.temperature", "weather", [], "number or nil", "The air temperature, °C.", NEW, None, false, |c, a| c.io().weather().map(|w| w.temperature as f64)),
    def!("weather.visibility", "weather", [], "number or nil", "How far one sees, metres.", NEW, None, false, |c, a| c.io().weather().map(|w| w.visibility_m as f64)),
    def!("weather.wind", "weather", [], "direction, speed", "The wind: where it comes from (degrees) and its speed (m/s).", NEW, None, true, |c, a| multi(c.io().weather().map(|w| [Value::from(w.wind_dir), Value::from(w.wind_ms)]))),
    def!("weather.precipitation", "weather", [], "kind, rate", "`\"none\"`, `\"rain\"` or `\"snow\"`, and how hard (0 to 1).", NEW, None, true, |c, a| multi(c.io().weather().map(|w| [Value::from(["none", "rain", "snow"].get(w.precip_kind.clamp(0, 2) as usize).copied().unwrap_or("none")), Value::from(w.precip_rate)]))),
    def!("weather.wetness", "weather", [], "number or nil", "How wet the roads are, 0 (dry) to 1.", NEW, None, false, |c, a| c.io().weather().map(|w| w.wetness as f64)),
    def!("weather.set", "weather", [p("values", "table")], "true, or false and the reason", "Changes the weather as the game menu's sliders do; any of `visibility`, `wind_direction`, `wind_speed`, `temperature`, `pressure`, `clouds` (`\"-1\"` none, `\"Cumulus 1\"`..`\"3\"`, `\"Overcast 1\"`), `cloud_base`, `precipitation` (`\"none\"`, `\"rain\"`, `\"snow\"`), `precipitation_rate` (0..1), `snow_cover`, `snow_on_road`, `wetness`; the rest stays. Held to the game's ranges; refused in a LAN game as a client or while the weather follows a station.", NEW, WorldWrite, true, |c, a| {
        let ch = change(a.table(0)?)?;
        Ok::<_, ApiError>(super::core::ok_or_reason(c.io().set_weather(&ch)))
    }),
    def!("weather.presets", "weather", [], "list of tables", "The weather files installed: `{file, name}`.", NEW, None, false, |c, a| Value::List(c.io().weather_presets().into_iter().map(|(f, n)| rec(vec![("file", f.into()), ("name", n.into())])).collect())),
    def!("weather.preset", "weather", [p("file", "string"), o("seconds", "number")], "true, or false and the reason", "Changes to a weather file (as `weather.presets` names it; `\"\"`: the map's own, changing with the day) over `seconds` (default 1).", NEW, WorldWrite, true, |c, a| {
        let f = a.str(0)?;
        let s = a.opt_num(1)?.unwrap_or(1.0).clamp(0.0, 3600.0) as f32;
        let file = (!f.is_empty()).then_some(f.as_str());
        Ok::<_, ApiError>(super::core::ok_or_reason(c.io().set_weather_preset(file, s)))
    }),
];
