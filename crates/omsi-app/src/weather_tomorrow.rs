//! Opt-in real weather: map groups select a location, never a credential.
use crate::weather_model::Observations;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    io::Read,
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const CFG: &str = "openomsi_weather.cfg";
pub(crate) fn selected(value: Option<&str>) -> bool {
    value.is_some_and(|v| v.eq_ignore_ascii_case("tomorrow"))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MapConfig {
    version: u32,
    refresh_minutes: u64,
    groups: Vec<Group>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Group {
    name: String,
    latitude: f64,
    longitude: f64,
    spawnpoints: Vec<Spawn>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Spawn {
    object_id: i64,
    tile: [i32; 2],
    #[serde(default)]
    name: String,
}
impl MapConfig {
    fn parse(bytes: &[u8]) -> Result<Self, &'static str> {
        let text = std::str::from_utf8(bytes)
            .map_err(|_| "map cfg must be UTF-8")?
            .trim_start_matches('\u{feff}');
        let cfg: Self = serde_json::from_str(text).map_err(|_| "invalid map weather cfg")?;
        if cfg.version != 1
            || !(15..=1440).contains(&cfg.refresh_minutes)
            || cfg.groups.is_empty()
            || cfg.groups.len() > 1000
        {
            return Err("invalid weather cfg version, interval or groups");
        }
        let mut used = HashSet::new();
        for g in &cfg.groups {
            if g.name.trim().is_empty()
                || !g.latitude.is_finite()
                || !g.longitude.is_finite()
                || g.latitude.abs() > 90.0
                || g.longitude.abs() > 180.0
                || g.spawnpoints.is_empty()
            {
                return Err("invalid group name, coordinates or spawnpoints");
            }
            for s in &g.spawnpoints {
                if !used.insert((s.tile, s.object_id)) {
                    return Err("a spawnpoint belongs to more than one group");
                }
            }
        }
        Ok(cfg)
    }
}
struct Region {
    name: String,
    coord: String,
    points: Vec<[f64; 2]>,
}
fn coord(lat: f64, lon: f64) -> String {
    format!(
        "{:.6},{:.6}",
        if lat == 0.0 { 0.0 } else { lat },
        if lon == 0.0 { 0.0 } else { lon }
    )
}
fn nearest(regions: &[Region], at: [f64; 2], current: Option<usize>) -> Option<usize> {
    let distance = |r: &Region| {
        r.points
            .iter()
            .map(|p| ((p[0] - at[0]).powi(2) + (p[1] - at[1]).powi(2)).sqrt())
            .fold(f64::INFINITY, f64::min)
    };
    let best = regions
        .iter()
        .enumerate()
        .filter(|(_, r)| !r.points.is_empty())
        .min_by(|(_, a), (_, b)| distance(a).total_cmp(&distance(b)))
        .map(|(i, _)| i)?;
    Some(
        current
            .filter(|i| {
                *i < regions.len() && distance(&regions[*i]) <= distance(&regions[best]) + 200.0
            })
            .unwrap_or(best),
    )
}
#[derive(Clone, Serialize, Deserialize)]
struct Cached {
    at: u64,
    values: Value,
}
#[derive(Default, Serialize, Deserialize)]
struct Cache {
    requests: Vec<u64>,
    blocked_until: u64,
    values: HashMap<String, Cached>,
}
impl Cache {
    fn reserve(&mut self, now: u64) -> Result<(), &'static str> {
        self.requests.retain(|t| now.saturating_sub(*t) < 86400);
        if now < self.blocked_until
            || self.requests.len() >= 450
            || self
                .requests
                .iter()
                .filter(|t| now.saturating_sub(**t) < 3600)
                .count()
                >= 20
            || self
                .requests
                .last()
                .is_some_and(|t| now.saturating_sub(*t) < 180)
        {
            return Err("local API budget or retry delay reached; keeping current weather");
        }
        self.requests.push(now);
        Ok(())
    }
}
struct Lock(PathBuf);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn cache_path(key: &str) -> Option<PathBuf> {
    use sha2::{Digest, Sha256};
    let hash = format!("{:x}", Sha256::digest(key.as_bytes()));
    let base = omsi_cfg::flags::OMSI_TOMORROW_CACHE_DIR
        .live_os()
        .map(PathBuf::from)
        .or_else(|| {
            crate::settings::Settings::path()
                .and_then(|p| p.parent().map(|d| d.join("weather-cache")))
        })?;
    Some(base.join(format!("tomorrow-{}.json", &hash[..16])))
}
fn credential() -> Option<String> {
    let key = omsi_cfg::flags::OMSI_TOMORROW_API_KEY
        .var()
        .map(str::to_owned)
        .or_else(|| {
            let path = omsi_cfg::flags::OMSI_TOMORROW_KEY_FILE
                .os()
                .map(PathBuf::from)
                .or_else(|| {
                    crate::settings::Settings::path()
                        .and_then(|p| p.parent().map(|d| d.join("tomorrow-api-key.txt")))
                })?;
            std::fs::read_to_string(path).ok()
        })?;
    let key = key.trim().to_string();
    (!key.is_empty()
        && key.len() < 512
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'))
    .then_some(key)
}
fn write_cache(path: &PathBuf, cache: &Cache) -> Result<(), &'static str> {
    let temp = path.with_extension("tmp");
    std::fs::write(
        &temp,
        serde_json::to_vec(cache).map_err(|_| "cannot encode weather cache")?,
    )
    .map_err(|_| "cannot save weather cache")?;
    std::fs::rename(&temp, path).map_err(|_| "cannot replace weather cache")
}
fn fetch(key: String, coord: &str, ttl: u64) -> Result<Loaded, &'static str> {
    let path = cache_path(&key).ok_or("no private cache directory")?;
    std::fs::create_dir_all(path.parent().unwrap())
        .map_err(|_| "cannot create private cache directory")?;
    let lock = path.with_extension("lock");
    if std::fs::metadata(&lock)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|t| t.as_secs() > 120)
    {
        let _ = std::fs::remove_file(&lock);
    }
    let _file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock)
        .map_err(|_| "another weather request is running")?;
    let _lock = Lock(lock);
    let mut cache: Cache = match std::fs::read(&path) {
        Ok(b) => {
            serde_json::from_slice(&b).map_err(|_| "weather cache is invalid; no request sent")?
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Cache::default(),
        Err(_) => return Err("cannot read weather cache"),
    };
    let time = now();
    if let Some(c) = cache
        .values
        .get(coord)
        .filter(|c| time >= c.at && time - c.at < ttl)
    {
        return Loaded::parse(c.clone());
    }
    cache.reserve(time)?;
    // Persist the reservation before networking; failed calls count too.
    write_cache(&path, &cache)?;
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout(Duration::from_secs(8))
        .build();
    let response = agent
        .get("https://api.tomorrow.io/v4/weather/realtime")
        .query("location", coord)
        .query("units", "metric")
        .set("apikey", &key)
        .call();
    let response = match response {
        Ok(r) => r,
        Err(ureq::Error::Status(status, _)) => {
            if matches!(status, 401 | 403 | 429) {
                cache.blocked_until = time + 3600;
                write_cache(&path, &cache)?;
            }
            return Err(match status {
                429 => "Tomorrow.io quota reached; requests paused for an hour",
                401 | 403 => "Tomorrow.io rejected the private API key",
                _ => "Tomorrow.io returned an HTTP error",
            });
        }
        Err(_) => return Err("Tomorrow.io could not be reached"),
    };
    let mut body = String::new();
    response
        .into_reader()
        .take(128 * 1024)
        .read_to_string(&mut body)
        .map_err(|_| "cannot read Tomorrow.io response")?;
    let data: Value = serde_json::from_str(&body).map_err(|_| "invalid Tomorrow.io JSON")?;
    let values = data
        .get("data")
        .and_then(|d| d.get("values"))
        .filter(|v| v.is_object())
        .ok_or("Tomorrow.io response has no weather values")?
        .clone();
    let observed = observations(&values)?;
    let cached = Cached { at: time, values };
    cache.values.insert(coord.into(), cached.clone());
    write_cache(&path, &cache)?;
    Ok(Loaded { cached, observed })
}
// Parsing stays on the worker, including a cache hit. The provider adapter
// converts API units only; the weather model chooses the game representation.
struct Loaded {
    cached: Cached,
    observed: Observations,
}
impl Loaded {
    fn parse(cached: Cached) -> Result<Self, &'static str> {
        let observed = observations(&cached.values)?;
        Ok(Self { cached, observed })
    }
}
fn observations(v: &Value) -> Result<Observations, &'static str> {
    let n = |key: &str| {
        v.get(key)
            .and_then(Value::as_f64)
            .filter(|x| x.is_finite())
            .map(|x| x as f32)
            .filter(|x| x.is_finite())
    };
    Ok(Observations {
        temperature_c: n("temperature").ok_or("response has no valid temperature")?,
        humidity_percent: n("humidity").ok_or("response has no valid humidity")?,
        wind: (
            n("windDirection").unwrap_or(0.0),
            n("windSpeed").ok_or("response has no valid wind speed")?,
        ),
        pressure_hpa: n("pressureSurfaceLevel")
            .or_else(|| n("pressureSeaLevel"))
            .unwrap_or(1013.0),
        visibility_m: n("visibility").unwrap_or(50.0) * 1000.0,
        cloud_cover: n("cloudCover").unwrap_or(0.0) / 100.0,
        cloud_base_m: n("cloudBase").map(|n| n * 1000.0).unwrap_or(1000.0),
        rain_mm_h: n("rainIntensity").unwrap_or(0.0),
        snow_mm_h: n("snowIntensity").unwrap_or(0.0),
    })
}
#[derive(Default)]
pub(crate) struct Tomorrow {
    map: Option<PathBuf>,
    regions: Vec<Region>,
    current: Option<usize>,
    ttl: u64,
    key: Option<String>,
    receiver: Option<mpsc::Receiver<(String, Result<Loaded, &'static str>)>>,
    check: Option<Instant>,
    poll_at: Option<Instant>,
    area_at: Option<Instant>,
    fresh_until: Option<Instant>,
    applied: Option<(String, u64)>,
    disabled: bool,
}
impl Tomorrow {
    fn poll_due(&mut self, time: Instant) -> bool {
        if self.poll_at.is_some_and(|at| time < at) {
            return false;
        }
        self.poll_at = Some(time + Duration::from_millis(250));
        true
    }
    fn area_due(&mut self, time: Instant) -> bool {
        if self.area_at.is_some_and(|at| time < at) {
            return false;
        }
        self.area_at = Some(time + Duration::from_secs(1));
        true
    }
}
impl crate::App {
    pub(crate) fn tick_tomorrow(&mut self) {
        if !selected(self.args.weather.as_deref())
            || self
                .net
                .lan
                .as_ref()
                .is_some_and(|l| l.role == omsi_net::Role::Client)
        {
            self.session.tomorrow = Default::default();
            return;
        }
        let time = Instant::now();
        if !self.session.tomorrow.poll_due(time) {
            return;
        }
        let Some(world) = self.world.as_ref() else {
            return;
        };
        if self.session.tomorrow.map.as_ref() != Some(&world.global.path) {
            self.session.tomorrow = Tomorrow {
                map: Some(world.global.path.clone()),
                poll_at: Some(time + Duration::from_millis(250)),
                ..Default::default()
            };
            let rel = std::path::Path::new(&self.args.map)
                .parent()
                .unwrap_or(std::path::Path::new(""))
                .join(CFG);
            let rel = rel.strip_prefix(&self.args.root).unwrap_or(&rel);
            let path = omsi_cfg::find_in_roots(&rel.to_string_lossy())
                .map(|p| p.1)
                .unwrap_or_else(|| world.global.path.parent().unwrap().join(CFG));
            let cfg = std::fs::read(path)
                .map_err(|_| "this map has no openomsi_weather.cfg")
                .and_then(|b| MapConfig::parse(&b));
            match cfg {
                Ok(cfg) => {
                    self.session.tomorrow.ttl = cfg.refresh_minutes * 60;
                    for g in cfg.groups {
                        let points = g
                            .spawnpoints
                            .iter()
                            .filter_map(|s| {
                                let _ = &s.name;
                                let ep = world.global.entry_points.iter().find(|e| {
                                    e.object_id == s.object_id
                                        && usize::try_from(e.group)
                                            .ok()
                                            .and_then(|i| world.global.raw_tiles.get(i))
                                            .is_some_and(|t| *t == (s.tile[0], s.tile[1]))
                                })?;
                                let p = world.entry_point_place(ep)?.0;
                                Some([p.x, p.y])
                            })
                            .collect::<Vec<_>>();
                        if points.len() != g.spawnpoints.len() {
                            self.session.tomorrow.disabled = true;
                            log::warn!(
                                "Tomorrow.io: cfg contains a spawnpoint not present in this map"
                            );
                            break;
                        }
                        self.session.tomorrow.regions.push(Region {
                            name: g.name,
                            coord: coord(g.latitude, g.longitude),
                            points,
                        });
                    }
                    self.session.tomorrow.key = credential();
                    if self.session.tomorrow.key.is_none() {
                        self.session.tomorrow.disabled = true;
                        self.service_msg =
                            Some(("Tomorrow.io: set your private API key first".into(), 6.0));
                    }
                }
                Err(e) => {
                    self.session.tomorrow.disabled = true;
                    log::warn!("Tomorrow.io: {e}");
                    self.service_msg = Some((format!("Tomorrow.io: {e}"), 6.0));
                }
            }
        }
        if self.session.tomorrow.disabled {
            return;
        }
        let Some(player) = self.player.as_ref() else {
            return;
        };
        if self.session.tomorrow.area_due(time) {
            self.session.tomorrow.current = nearest(
                &self.session.tomorrow.regions,
                [player.vehicle.position.x, player.vehicle.position.y],
                self.session.tomorrow.current,
            );
        }
        let Some(index) = self.session.tomorrow.current else {
            return;
        };
        let site = self.session.tomorrow.regions[index].coord.clone();
        if let Some(rx) = self.session.tomorrow.receiver.as_ref() {
            match rx.try_recv() {
                Ok((location, result)) => {
                    self.session.tomorrow.receiver = None;
                    match result {
                        Ok(c) if location == site => {
                            if self.session.tomorrow.applied.as_ref()
                                != Some(&(site.clone(), c.cached.at))
                            {
                                let name = format!(
                                    "Tomorrow.io · {}",
                                    self.session.tomorrow.regions[index].name
                                );
                                let mut to = c.observed.weather(self.session.wetness);
                                to.name = name;
                                let wire = crate::weather_setup::CustomWeather::from_weather(
                                    &to,
                                    1.0,
                                    self.session.wetness,
                                )
                                .encode();
                                self.apply_report_weather(to, None, &wire);
                                self.session.tomorrow.applied = Some((site.clone(), c.cached.at));
                            }
                            let remaining = self
                                .session
                                .tomorrow
                                .ttl
                                .saturating_sub(now().saturating_sub(c.cached.at));
                            self.session.tomorrow.fresh_until =
                                Some(time + Duration::from_secs(remaining));
                            self.session.tomorrow.check =
                                Some(Instant::now() + Duration::from_secs(5));
                        }
                        Ok(_) => self.session.tomorrow.check = None,
                        Err(e) => {
                            log::warn!("Tomorrow.io: {e}");
                            self.service_msg = Some((format!("Tomorrow.io: {e}"), 5.0));
                            self.session.tomorrow.check =
                                Some(Instant::now() + Duration::from_secs(180));
                        }
                    }
                }
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.session.tomorrow.receiver = None;
                    self.session.tomorrow.check = Some(Instant::now() + Duration::from_secs(180));
                }
            }
        }
        if self.session.tomorrow.receiver.is_some()
            || self
                .session
                .tomorrow
                .check
                .is_some_and(|t| t > Instant::now())
            || self
                .session
                .tomorrow
                .applied
                .as_ref()
                .is_some_and(|(c, _)| {
                    *c == site
                        && self
                            .session
                            .tomorrow
                            .fresh_until
                            .is_some_and(|at| time < at)
                })
        {
            return;
        }
        let Some(key) = self.session.tomorrow.key.clone() else {
            return;
        };
        let ttl = self.session.tomorrow.ttl;
        let (tx, rx) = mpsc::channel();
        self.session.tomorrow.receiver = Some(rx);
        std::thread::spawn(move || {
            let result = fetch(key, &site, ttl);
            let _ = tx.send((site, result));
        });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frame_timers_throttle_response_and_anchor_work() {
        let mut state = Tomorrow::default();
        let time = Instant::now();
        assert!(state.poll_due(time));
        assert!(!state.poll_due(time + Duration::from_millis(1)));
        assert!(state.poll_due(time + Duration::from_millis(250)));
        assert!(state.area_due(time));
        assert!(!state.area_due(time + Duration::from_millis(250)));
        assert!(state.area_due(time + Duration::from_secs(1)));
    }
    #[test]
    fn shared_sites_and_boundary_hysteresis() {
        assert_eq!(coord(50.0, 14.0), coord(50.00000001, 14.00000001));
        let regions = vec![
            Region {
                name: "A".into(),
                coord: "a".into(),
                points: vec![[0.0, 0.0], [100.0, 0.0]],
            },
            Region {
                name: "B".into(),
                coord: "b".into(),
                points: vec![[1000.0, 0.0]],
            },
        ];
        assert_eq!(nearest(&regions, [600.0, 0.0], Some(0)), Some(0));
        assert_eq!(nearest(&regions, [900.0, 0.0], Some(0)), Some(1));
    }
    #[test]
    fn budgets_count_failures_and_survive_serialization() {
        let mut c = Cache::default();
        c.reserve(100000).unwrap();
        assert!(c.reserve(100001).is_err());
        c.requests = (0..20).map(|i| 100000 + i * 180).collect();
        assert!(c.reserve(103599).is_err());
        let text = serde_json::to_vec(&c).unwrap();
        let mut c: Cache = serde_json::from_slice(&text).unwrap();
        c.blocked_until = 200000;
        assert!(c.reserve(104000).is_err());
        c.blocked_until = 0;
        c.requests = (0..450).map(|i| 100000 + i * 180).collect();
        assert!(c.reserve(181000).is_err());
    }
    #[test]
    fn response_uses_metric_units_and_rejects_missing_values() {
        let v = serde_json::json!({"temperature":12,"humidity":80,"windSpeed":6.6,"windDirection":370,"visibility":2,"cloudCover":90,"weatherCode":5101,"snowIntensity":2});
        let observed = observations(&v).unwrap();
        assert_eq!(observed.snow_mm_h, 2.0);
        assert_eq!(observed.cloud_cover, 0.9);
        let c = observed.weather(0.0);
        assert_eq!(c.wind.1, 6.6);
        assert_eq!(c.wind.0, 10.0);
        assert_eq!(c.fog.0, 2000.0);
        assert_eq!(c.precip[0], 2.0);
        assert_eq!(c.clouds.0, "Overcast 1");
        assert!(observations(&serde_json::json!({"temperature":12})).is_err());
    }
    #[test]
    fn cfg_rejects_credentials_duplicates_and_invalid_coordinates() {
        let s = r#"{"version":1,"refresh_minutes":15,"groups":[{"name":"A","latitude":50,"longitude":14,"spawnpoints":[{"object_id":7,"tile":[0,0]}]}]}"#;
        assert!(MapConfig::parse(s.as_bytes()).is_ok());
        assert!(
            MapConfig::parse(s.replace("\"latitude\":50", "\"latitude\":95").as_bytes()).is_err()
        );
        assert!(MapConfig::parse(
            s.replace("\"version\":1", "\"version\":1,\"apikey\":\"never here\"")
                .as_bytes()
        )
        .is_err());
        assert!(MapConfig::parse(
            s.replace(
                "{\"object_id\":7,\"tile\":[0,0]}",
                "{\"object_id\":7,\"tile\":[0,0]},{\"object_id\":7,\"tile\":[0,0]}"
            )
            .as_bytes()
        )
        .is_err());
    }
}
