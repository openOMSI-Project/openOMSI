//! Pairing a phone or tablet with the game.
//!
//! The companion's server listens on the home network, and everyone on the same Wi-Fi can
//! see its port. So nothing but the page itself answers without a key: the game shows a
//! six-digit pairing code (on screen, for the integrator's navigator: `companion::state()`),
//! the device sends it once and gets a device key of 128 random bits back, which it keeps
//! and sends with every request. A code that was used is replaced by a new one, so a code
//! read over someone's shoulder is worth nothing after the pairing it was shown for.
//!
//! Ten wrong codes within a minute and the door stays shut for the rest of that minute:
//! a million codes at ten a minute is more than two months of trying.
//!
//! Through the Cloudflare tunnel the page is reachable from the whole internet, and the
//! door is stricter (`strict`): the code has nine digits, and five wrong ones shut the door for
//! five minutes and replace the code - a billion codes at one a minute, and each lockout throws
//! away what was tried.
//!
//! The device keys are kept (as SHA-256, never the keys themselves) in
//! `~/.openomsi/companion-devices.json`, so that a phone that put the page on its home screen
//! is still paired after the game was restarted.

use sha2::{Digest, Sha256};

/// Wrong codes allowed within [`MISS_WINDOW`] seconds.
pub(crate) const MAX_MISSES: u32 = 10;
pub(crate) const MISS_WINDOW: f64 = 60.0;
/// Reachable from the internet: wrong codes allowed before the door shuts for
/// [`STRICT_LOCK`] seconds (and the code is replaced).
pub(crate) const STRICT_MISSES: u32 = 5;
pub(crate) const STRICT_LOCK: f64 = 300.0;
/// The pairing code's digits on the home network, and reachable from the internet.
pub(crate) const CODE_LEN: usize = 6;
pub(crate) const STRICT_CODE_LEN: usize = 9;
/// Devices kept paired; a further one pushes out the one paired longest ago.
pub(crate) const MAX_DEVICES: usize = 8;

/// A paired device.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Paired {
    /// SHA-256 of the device key, as hex.
    pub hash: String,
    /// What the device said it is (its browser), for the list of devices.
    pub name: String,
    /// Seconds since 1970 when it was paired.
    pub since: u64,
}

/// Why a pairing was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refused {
    Wrong,
    /// Too many wrong codes lately.
    Locked,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Pairing {
    code: String,
    pub devices: Vec<Paired>,
    misses: u32,
    misses_since: Option<f64>,
    /// Reachable from the internet (the tunnel): longer codes, fewer tries.
    strict: bool,
    /// The door is shut until then (steady seconds).
    locked_until: Option<f64>,
}

pub(crate) fn hash_key(key: &str) -> String {
    let d = Sha256::digest(key.as_bytes());
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// Compare two secrets in a time that does not tell how much of them matched.
fn same_secret(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut d = (a.len() != b.len()) as u8;
    for i in 0..a.len().max(b.len()) {
        d |= a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0);
    }
    d == 0
}

impl Pairing {
    pub(crate) fn new(code: String, devices: Vec<Paired>) -> Pairing {
        Pairing { code, devices, ..Default::default() }
    }

    /// The code the game shows now.
    pub(crate) fn code(&self) -> &str {
        &self.code
    }

    pub(crate) fn set_code(&mut self, code: String) {
        self.code = code;
    }

    /// How many digits a pairing code has now.
    pub(crate) fn code_len(&self) -> usize {
        if self.strict { STRICT_CODE_LEN } else { CODE_LEN }
    }

    pub(crate) fn strict(&self) -> bool {
        self.strict
    }

    /// Reachable from the internet or not: the code is replaced by one of the length that
    /// goes with it (`new_code` makes it), and the count of wrong ones starts again.
    pub(crate) fn set_strict(&mut self, on: bool, new_code: &mut dyn FnMut(usize) -> String) {
        if self.strict != on {
            self.strict = on;
            self.code = new_code(self.code_len());
            self.misses = 0;
            self.misses_since = None;
            self.locked_until = None;
        }
    }

    /// A device offers `code`: its new device key, or why not. `now` in seconds of a steady
    /// clock, `unix` the date for the list. `new_code` and `new_key` make the next pairing
    /// code (of the digits asked for) and the device key (random, see the companion's
    /// `random_u64`).
    pub(crate) fn pair(&mut self, code: &str, name: &str, now: f64, unix: u64, new_code: &mut dyn FnMut(usize) -> String, new_key: &mut dyn FnMut() -> String) -> Result<String, Refused> {
        if let Some(t) = self.locked_until {
            if now < t {
                return Err(Refused::Locked);
            }
            self.locked_until = None;
            self.misses_since = None;
        }
        let (most, window) = if self.strict { (STRICT_MISSES, STRICT_LOCK) } else { (MAX_MISSES, MISS_WINDOW) };
        if self.misses_since.is_none_or(|t| now - t > window) {
            self.misses_since = Some(now);
            self.misses = 0;
        }
        if self.code.is_empty() || !same_secret(code.trim(), &self.code) {
            self.misses += 1;
            if self.misses >= most {
                // on the home network for the rest of the minute; from the internet five
                // minutes from now, and what was tried is worth nothing after it
                self.locked_until = Some(if self.strict { now + STRICT_LOCK } else { self.misses_since.unwrap_or(now) + window });
                if self.strict {
                    self.code = new_code(self.code_len());
                }
            }
            return Err(Refused::Wrong);
        }
        self.misses = 0;
        let key = new_key();
        if self.devices.len() >= MAX_DEVICES {
            // (the oldest pairing goes)
            if let Some(oldest) = self.devices.iter().enumerate().min_by_key(|(_, d)| d.since).map(|(i, _)| i) {
                self.devices.remove(oldest);
            }
        }
        let name: String = name.chars().filter(|c| !c.is_control()).take(60).collect();
        self.devices.push(Paired { hash: hash_key(&key), name, since: unix });
        self.code = new_code(self.code_len());
        Ok(key)
    }

    /// The device of this key, if it is paired.
    pub(crate) fn device_of(&self, key: &str) -> Option<usize> {
        if key.len() < 16 || key.len() > 128 {
            return None;
        }
        let h = hash_key(key);
        self.devices.iter().position(|d| same_secret(&d.hash, &h))
    }

    pub(crate) fn forget_all(&mut self) {
        self.devices.clear();
    }
}

/// The paired devices as `companion-devices.json` holds them.
pub(crate) fn devices_json(devices: &[Paired]) -> String {
    let list: Vec<serde_json::Value> = devices.iter().map(|d| serde_json::json!({ "key_sha256": d.hash, "name": d.name, "since": d.since })).collect();
    serde_json::to_string_pretty(&serde_json::json!({ "devices": list })).unwrap_or_default()
}

pub(crate) fn parse_devices(text: &str) -> Vec<Paired> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else { return Vec::new() };
    let Some(list) = v.get("devices").and_then(|d| d.as_array()) else { return Vec::new() };
    list.iter()
        .filter_map(|d| {
            let hash = d.get("key_sha256")?.as_str()?.to_ascii_lowercase();
            (hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())).then(|| Paired {
                hash,
                name: d.get("name").and_then(|n| n.as_str()).unwrap_or("").chars().take(60).collect(),
                since: d.get("since").and_then(|s| s.as_u64()).unwrap_or(0),
            })
        })
        .take(MAX_DEVICES)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> impl FnMut() -> String {
        let mut n = 0;
        move || {
            n += 1;
            format!("{n:032x}")
        }
    }

    #[test]
    fn the_right_code_pairs_once_and_is_replaced() {
        let mut p = Pairing::new("123456".into(), Vec::new());
        let (mut code, mut key) = (|_: usize| "654321".to_string(), keys());
        let k = p.pair("123456", "Safari", 0.0, 1, &mut code, &mut key).unwrap();
        assert_eq!(p.device_of(&k), Some(0));
        assert_eq!(p.code(), "654321");
        // the old code is worth nothing now
        assert_eq!(p.pair("123456", "x", 1.0, 1, &mut code, &mut key), Err(Refused::Wrong));
        assert_eq!(p.device_of("00000000000000000000000000000099"), None);
        assert_eq!(p.device_of("short"), None);
    }

    #[test]
    fn ten_wrong_codes_lock_the_door_for_the_minute() {
        let mut p = Pairing::new("123456".into(), Vec::new());
        let (mut code, mut key) = (|_: usize| "111111".to_string(), keys());
        for i in 0..MAX_MISSES {
            assert_eq!(p.pair(&format!("{i:06}"), "x", i as f64, 0, &mut code, &mut key), Err(Refused::Wrong));
        }
        assert_eq!(p.pair("123456", "x", 20.0, 0, &mut code, &mut key), Err(Refused::Locked));
        assert!(p.pair("123456", "x", 61.0, 0, &mut code, &mut key).is_ok());
    }

    #[test]
    fn from_the_internet_the_code_is_longer_and_five_wrong_ones_shut_the_door_and_change_it() {
        let mut made = 0;
        let mut code = |n: usize| {
            made += 1;
            format!("{made}").repeat(n)
        };
        let mut key = keys();
        let mut p = Pairing::new("123456".into(), Vec::new());
        assert_eq!(p.code_len(), CODE_LEN);
        p.set_strict(true, &mut code);
        assert!(p.strict());
        assert_eq!((p.code_len(), p.code()), (STRICT_CODE_LEN, "111111111"), "a longer code at once");
        for i in 0..STRICT_MISSES {
            assert_eq!(p.pair(&format!("{i:09}"), "x", i as f64, 0, &mut code, &mut key), Err(Refused::Wrong));
        }
        // shut, and the code that was on show is no longer the code
        assert_eq!(p.code(), "222222222");
        assert_eq!(p.pair("222222222", "x", 100.0, 0, &mut code, &mut key), Err(Refused::Locked));
        assert_eq!(p.pair("222222222", "x", 4.0 + STRICT_LOCK - 1.0, 0, &mut code, &mut key), Err(Refused::Locked));
        assert!(p.pair("222222222", "x", 4.0 + STRICT_LOCK + 1.0, 0, &mut code, &mut key).is_ok());
        assert_eq!(p.code().len(), STRICT_CODE_LEN, "the next code is as long");
        // back on the home network: six digits again
        p.set_strict(false, &mut code);
        assert_eq!(p.code().len(), CODE_LEN);
    }

    #[test]
    fn the_oldest_device_goes_when_the_list_is_full() {
        let mut p = Pairing::new("1".into(), Vec::new());
        let (mut code, mut key) = (|_: usize| "1".to_string(), keys());
        let first = p.pair("1", "first", 0.0, 10, &mut code, &mut key).unwrap();
        for i in 1..MAX_DEVICES as u64 + 1 {
            p.pair("1", "next", 0.0, 10 + i, &mut code, &mut key).unwrap();
        }
        assert_eq!(p.devices.len(), MAX_DEVICES);
        assert_eq!(p.device_of(&first), None);
    }

    #[test]
    fn devices_are_kept_as_hashes_and_read_back() {
        let mut p = Pairing::new("1".into(), Vec::new());
        let (mut code, mut key) = (|_: usize| "2".to_string(), keys());
        let k = p.pair("1", "Chrome\u{7}", 0.0, 99, &mut code, &mut key).unwrap();
        let text = devices_json(&p.devices);
        assert!(!text.contains(&k), "the key itself is not written");
        let back = parse_devices(&text);
        assert_eq!(back, p.devices);
        assert_eq!(back[0].name, "Chrome");
        let again = Pairing::new("3".into(), back);
        assert_eq!(again.device_of(&k), Some(0));
        assert!(parse_devices("{\"devices\":[{\"key_sha256\":\"nothex\"}]}").is_empty());
    }
}
