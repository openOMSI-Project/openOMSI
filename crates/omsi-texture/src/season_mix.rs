//! Two looks of the vegetation at once: a season chosen with its phase (early, mid, late)
//! is a time between two of the map's texture seasons - in early autumn most trees are
//! still green and some have turned, in late autumn many stand bare and some still carry
//! their brown leaves. The whole map shows the season that prevails ([`crate::season_folder`]);
//! each plant takes the other look or not by a number of its own (its object id), so that
//! the same tree looks the same every time and on every player's screen.

use parking_lot::Mutex;
use std::path::{Path, PathBuf};

/// The two looks of a phase: the texture season folders (`None` = summer, the textures
/// themselves) and the share of the plants that already wear the second.
#[derive(Debug, Clone, PartialEq)]
pub struct SeasonMix {
    pub from: Option<String>,
    pub to: Option<String>,
    pub share: f32,
}

static MIX: Mutex<Option<SeasonMix>> = Mutex::new(None);

/// The phase's mix of looks (none: every plant shows the map's season, as OMSI has it).
pub fn set_season_mix(mix: Option<SeasonMix>) {
    *MIX.lock() = mix;
}

pub fn season_mix() -> Option<SeasonMix> {
    MIX.lock().clone()
}

/// A stable number in 0..1 for a plant's own number (`key`: its object id and place).
pub fn unit_hash(key: u64) -> f32 {
    // splitmix64: neighbouring ids spread over the whole range
    let mut z = key.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 40) as f32 / (1u64 << 24) as f32
}

fn same(a: &Option<String>, b: &Option<String>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        _ => false,
    }
}

/// The look of the plant `key` under `mix` while the map shows the season `now`: `Some`
/// with the other season folder when it is not the map's (`Some(None)`: summer's), `None`
/// when the plant looks like the rest. Snow (a season of neither look) covers every plant.
pub fn look_of(mix: &SeasonMix, now: &Option<String>, key: u64) -> Option<Option<String>> {
    if !same(now, &mix.from) && !same(now, &mix.to) {
        return None;
    }
    let look = if unit_hash(key) < mix.share { &mix.to } else { &mix.from };
    (!same(look, now)).then(|| look.clone())
}

/// [`look_of`] under the mix and the season set now.
pub fn mixed_look(key: u64) -> Option<Option<String>> {
    let mix = season_mix()?;
    look_of(&mix, &crate::season_folder(), key)
}

/// A texture as it looks in the season `look` (`None`: summer), whatever season the map
/// shows: the file [`crate::find_texture`] would find were that the map's season.
pub fn find_texture_in_look(name: &str, dirs: &[&Path], look: Option<&str>) -> Option<PathBuf> {
    crate::find_texture_in_season(name, dirs, look)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mix(from: Option<&str>, to: Option<&str>, share: f32) -> SeasonMix {
        SeasonMix { from: from.map(String::from), to: to.map(String::from), share }
    }

    #[test]
    fn the_share_of_plants_takes_the_second_look() {
        let m = mix(None, Some("fall"), 0.25);
        let now = None;
        let n = 20_000u64;
        let turned = (0..n).filter(|k| look_of(&m, &now, *k).is_some()).count() as f32 / n as f32;
        assert!((turned - 0.25).abs() < 0.02, "{turned}");
        // seen from the other look (the map in Fall): the rest are the summer ones
        let now = Some("Fall".to_string());
        let green = (0..n).filter(|k| look_of(&m, &now, *k) == Some(None)).count() as f32 / n as f32;
        assert!((green - 0.75).abs() < 0.02, "{green}");
    }

    #[test]
    fn a_plant_keeps_its_look_and_turns_once() {
        // a tree turned in early autumn is still turned in mid autumn
        let early = mix(None, Some("fall"), 0.25);
        let mid = mix(None, Some("fall"), 0.8);
        for k in 0..5000u64 {
            assert_eq!(look_of(&early, &None, k), look_of(&early, &None, k));
            if look_of(&early, &None, k).is_some() {
                assert!(look_of(&mid, &None, k).is_some());
            }
        }
    }

    #[test]
    fn snow_covers_every_plant() {
        let m = mix(Some("fall"), Some("Winter"), 0.6);
        assert!((0..1000u64).all(|k| look_of(&m, &Some("WinterSnow".into()), k).is_none()));
    }

    #[test]
    fn unit_hash_stays_in_range() {
        assert!((0..10_000u64).map(|k| unit_hash(k.wrapping_mul(7919))).all(|u| (0.0..1.0).contains(&u)));
    }
}
