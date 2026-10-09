//! Public dedicated-server lobby: servers post where they are reached to a shared relay
//! topic; the launcher lists fresh posts and verifies each with `GET /status` before showing
//! it as online (anybody can post; a dead or forged address fails that check).
//!
//! Post format (one line, size-capped):
//! `LOBBY <https-url> <unix-secs> <name> | <map> | <players>/<max> | <version>`

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Whether a dedicated server posts to the public lobby (`server.cfg`'s `public`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Never announce.
    Off,
    /// Announce when a tunnel URL exists (the default).
    #[default]
    Auto,
    /// Announce whenever a join URL is known (today: the tunnel URL).
    On,
}

impl Mode {
    /// Parse `server.cfg`'s `public` value (`auto` / `0` / `1` / bool words).
    pub fn parse(s: &str) -> Mode {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "auto" => Mode::Auto,
            "0" | "false" | "no" | "off" => Mode::Off,
            "1" | "true" | "yes" | "on" => Mode::On,
            _ => Mode::Auto,
        }
    }

    /// Whether this mode wants lobby posts when a tunnel URL is available.
    pub fn wants_announce(self) -> bool {
        matches!(self, Mode::Auto | Mode::On)
    }
}

/// The relay topic public dedicated servers post under.
const TOPIC: &str = "openomsi-public-servers-v1";
const RELAY: &str = "https://ntfy.sh";
/// A post older than this is not listed (servers post every five minutes).
const FRESH: Duration = Duration::from_secs(40 * 60);
const MAX_URL: usize = 300;
const MAX_NAME: usize = 80;
const MAX_MAP: usize = 200;
const MAX_VERSION: usize = 64;
const MAX_POST: usize = 500;

/// One fresh lobby post (newest kept per URL).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub url: String,
    pub at: u64,
    pub name: String,
    pub map: String,
    pub players: usize,
    pub max_players: usize,
    pub version: String,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn clean_field(s: &str, max: usize) -> String {
    s.chars()
        .filter(|c| !c.is_control() && *c != '|' && *c != '\n')
        .take(max)
        .collect::<String>()
        .trim()
        .to_string()
}

/// Build the relay post body for a server.
pub fn format_post(url: &str, at: u64, name: &str, map: &str, players: usize, max_players: usize, version: &str) -> Option<String> {
    if !url.starts_with("https://") || url.len() > MAX_URL || url.contains(' ') {
        return None;
    }
    let name = clean_field(name, MAX_NAME);
    let map = clean_field(map, MAX_MAP);
    let version = clean_field(version, MAX_VERSION);
    if name.is_empty() {
        return None;
    }
    let body = format!("LOBBY {url} {at} {name} | {map} | {players}/{max_players} | {version}");
    (body.len() <= MAX_POST).then_some(body)
}

/// Parse one lobby message; `now` is used for the freshness window.
pub fn parse(msg: &str, now: u64) -> Option<Entry> {
    let msg = msg.trim();
    let mut parts = msg.splitn(4, ' ');
    if parts.next()? != "LOBBY" {
        return None;
    }
    let url = parts.next()?.to_string();
    let at: u64 = parts.next()?.parse().ok()?;
    let rest = parts.next()?;
    if !url.starts_with("https://") || url.len() > MAX_URL || url.contains(' ') {
        return None;
    }
    if at > now + 300 || now.saturating_sub(at) > FRESH.as_secs() {
        return None;
    }
    let mut fields = rest.splitn(4, " | ");
    let name = clean_field(fields.next()?, MAX_NAME);
    let map = clean_field(fields.next().unwrap_or(""), MAX_MAP);
    let counts = fields.next().unwrap_or("0/0");
    let version = clean_field(fields.next().unwrap_or(""), MAX_VERSION);
    if name.is_empty() {
        return None;
    }
    let (players, max_players) = {
        let (a, b) = counts.split_once('/')?;
        (a.trim().parse().ok()?, b.trim().parse().ok()?)
    };
    Some(Entry { url, at, name, map, players, max_players, version })
}

fn agent() -> ureq::Agent {
    // ntfy.sh is sometimes slow from some networks (connect is fine, the body lags past 8 s)
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        .user_agent("openOMSI")
        .build()
}

/// Post this dedicated server to the public lobby. Call every few minutes while the tunnel
/// URL is known.
pub fn announce(url: &str, name: &str, map: &str, players: usize, max_players: usize, version: &str) -> Result<(), String> {
    let text = format_post(url, now(), name, map, players, max_players, version).ok_or_else(|| "lobby post rejected".to_string())?;
    agent().post(&format!("{RELAY}/{TOPIC}")).set("Cache", "yes").send_string(&text).map_err(|e| e.to_string())?;
    Ok(())
}

/// Fresh lobby entries from the relay (newest post per URL). The launcher still verifies
/// each with `ws::query` before showing it as online.
pub fn fetch() -> Result<Vec<Entry>, String> {
    let body = agent()
        .get(&format!("{RELAY}/{TOPIC}/json?poll=1&since=1h"))
        .call()
        .map_err(|e| format!("the public server list could not be read: {e}"))?
        .into_string()
        .map_err(|e| e.to_string())?;
    Ok(merge(body.lines().filter_map(|l| crate::bridge::json_field(l, "message")), now()))
}

/// Newest fresh entry per URL from raw post messages.
fn merge(messages: impl Iterator<Item = String>, now: u64) -> Vec<Entry> {
    let mut by_url: HashMap<String, Entry> = HashMap::new();
    for m in messages {
        let Some(e) = parse(&m, now) else { continue };
        match by_url.get(&e.url) {
            Some(old) if old.at >= e.at => {}
            _ => {
                by_url.insert(e.url.clone(), e);
            }
        }
    }
    let mut list: Vec<Entry> = by_url.into_values().collect();
    list.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| a.name.cmp(&b.name)));
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_freshness() {
        let t = 1_800_000_000;
        let post = format_post("https://a.trycloudflare.com", t - 60, "Spandau", "maps/Berlin-Spandau/global.cfg", 2, 16, "0.1").unwrap();
        let e = parse(&post, t).unwrap();
        assert_eq!(e.url, "https://a.trycloudflare.com");
        assert_eq!(e.name, "Spandau");
        assert_eq!(e.players, 2);
        assert_eq!(e.max_players, 16);
        assert!(parse(&format_post("https://a.trycloudflare.com", t - 3 * 3600, "Old", "m", 0, 8, "1").unwrap(), t).is_none());
        assert!(format_post("http://insecure.example", t, "X", "m", 0, 8, "1").is_none());
        assert!(format_post("https://a.example bad", t, "X", "m", 0, 8, "1").is_none());
        assert!(parse("LOBBY https://x.example 1", t).is_none());
    }

    #[test]
    fn newest_per_url_wins() {
        let t = 1_800_000_000;
        let a1 = format_post("https://a.trycloudflare.com", t - 120, "A", "m", 1, 8, "1").unwrap();
        let a2 = format_post("https://a.trycloudflare.com", t - 30, "A2", "m", 3, 8, "1").unwrap();
        let b = format_post("https://b.trycloudflare.com", t - 10, "B", "m", 0, 16, "1").unwrap();
        let list = merge([a1, b, a2].into_iter(), t);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "B");
        let a = list.iter().find(|e| e.url.contains("a.")).unwrap();
        assert_eq!(a.name, "A2");
        assert_eq!(a.players, 3);
    }

    #[test]
    fn strips_separators_from_fields() {
        let t = 1_800_000_000;
        let post = format_post("https://a.example", t, "Name|Bad", "map|x", 0, 4, "v|1").unwrap();
        let e = parse(&post, t).unwrap();
        assert_eq!(e.name, "NameBad");
        assert_eq!(e.map, "mapx");
        assert_eq!(e.version, "v1");
    }

    #[test]
    fn mode_parse() {
        assert_eq!(Mode::parse("auto"), Mode::Auto);
        assert_eq!(Mode::parse(""), Mode::Auto);
        assert_eq!(Mode::parse("0"), Mode::Off);
        assert_eq!(Mode::parse("false"), Mode::Off);
        assert_eq!(Mode::parse("1"), Mode::On);
        assert_eq!(Mode::parse("yes"), Mode::On);
        assert!(Mode::Auto.wants_announce());
        assert!(Mode::On.wants_announce());
        assert!(!Mode::Off.wants_announce());
    }
}
