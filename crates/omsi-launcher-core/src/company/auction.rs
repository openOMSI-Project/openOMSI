//! A tender's auction (Luc: when the player bids on a line, fictitious companies bid too; he
//! may buy the line directly, or bid less with the chance to lose it; and it lasts a few
//! hours, not days).
//!
//! A line is auctioned for a few hours of the company's time. The bidders offer a sum for the
//! concession; the authority weighs every sum with the bidder's quality (its reputation and
//! punctuality, see `weight`), and the best weighed offer when the auction closes wins. Each
//! new bid has to beat the best one by a step, so the last bid placed always leads.
//!
//! The rivals are the map's other operators, each with a character: an aggressive one answers
//! fast and goes high, a cautious one takes its time and stays low, a big one has deep pockets,
//! a small one not; some keep quiet until the last minutes and only then bid. Each knows how
//! far it will go (its limit, drawn per tender and kept secret); the player sees their
//! characters and their bids as they come, and what `chance` reckons from what he can know.
//!
//! Everything is replayed from the auction's start (`replay`): the same bids of the player
//! give the same auction, and a bid of his changes only what comes after it (the rivals act
//! before him within a minute). Minutes are the company clock's (`clock::now`).

use super::model::Cents;
use super::rng::Rng;
use serde::{Deserialize, Serialize};

/// How an operator bids.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Character {
    Aggressive,
    #[default]
    Cautious,
    Big,
    Small,
}

impl Character {
    pub const ALL: [Character; 4] = [Character::Aggressive, Character::Cautious, Character::Big, Character::Small];

    pub fn label(self) -> &'static str {
        match self {
            Character::Aggressive => "aggressive",
            Character::Cautious => "cautious",
            Character::Big => "big",
            Character::Small => "small",
        }
    }

    /// How far it goes, as a share of the line's value (from, to).
    pub fn limit(self) -> (f64, f64) {
        match self {
            Character::Aggressive => (0.85, 1.30),
            Character::Big => (0.80, 1.20),
            Character::Cautious => (0.55, 0.95),
            Character::Small => (0.45, 0.85),
        }
    }

    /// Minutes it takes to answer a bid that beat it (from, to).
    pub fn delay(self) -> (i64, i64) {
        match self {
            Character::Aggressive => (3, 15),
            Character::Big => (10, 35),
            Character::Cautious => (25, 80),
            Character::Small => (15, 50),
        }
    }

    /// How likely it keeps quiet until the last minutes.
    pub fn quiet(self) -> f64 {
        match self {
            Character::Aggressive => 0.35,
            Character::Cautious => 0.4,
            Character::Big => 0.1,
            Character::Small => 0.2,
        }
    }

    /// Steps above the least it raises by, at most.
    pub fn jump(self) -> i64 {
        match self {
            Character::Aggressive => 2,
            Character::Big => 1,
            _ => 0,
        }
    }

    /// Its quality in the authority's eyes (from, to).
    pub fn quality(self) -> (f64, f64) {
        match self {
            Character::Aggressive => (45.0, 70.0),
            Character::Big => (60.0, 85.0),
            Character::Cautious => (50.0, 75.0),
            Character::Small => (35.0, 60.0),
        }
    }
}

/// What a bid of a bidder of that quality counts for: its sum times this.
pub fn weight(quality: f64) -> f64 {
    0.8 + 0.4 * (quality / 100.0).clamp(0.0, 1.0)
}

/// The auction's frame: when it runs (minutes), the least first bid and the least step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lot {
    pub opens: i64,
    /// Bids are taken until the minute before.
    pub closes: i64,
    pub reserve: Cents,
    pub step: Cents,
}

/// A rival as the replay knows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Bidder {
    pub character: Character,
    /// The most it offers (secret).
    pub limit: Cents,
    pub weight: f64,
    /// Its own draws (its answers' delays and jumps).
    pub seed: u64,
    /// The minute of its opening bid, if it opens.
    pub opening: Option<i64>,
    /// The minute it bids at if it keeps quiet until then.
    pub quiet_until: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Who {
    Rival(usize),
    Player,
}

/// A bid placed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub at: i64,
    pub who: Who,
    pub amount: Cents,
}

/// Bids go in whole hundreds of euros.
pub const ROUND: Cents = 100_00;

fn round_up(x: f64) -> Cents {
    ((x / ROUND as f64).ceil() as Cents * ROUND).max(ROUND)
}

/// The least a bidder of `weight` must offer to lead, the best score being `best` (None: no
/// bid yet, the reserve).
pub fn to_lead(best: Option<f64>, weight: f64, lot: &Lot) -> Cents {
    match best {
        None => lot.reserve,
        Some(s) => round_up(s / weight.max(0.1) + lot.step as f64 * 0.999).max(lot.reserve),
    }
}

/// The auction from its start until the minute `until` (inclusive; at most its last minute):
/// the rivals' bids, and the player's (`player`: when, how much; a bid that does not lead
/// when it comes is left out).
pub fn replay(lot: &Lot, rivals: &[Bidder], player: &[(i64, Cents)], player_weight: f64, until: i64) -> Vec<Placed> {
    let end = until.min(lot.closes - 1);
    let n = rivals.len();
    let mut rngs: Vec<Rng> = rivals.iter().map(|b| Rng::new(b.seed)).collect();
    let mut next: Vec<Option<i64>> = rivals.iter().map(|b| if b.quiet_until.is_some() { None } else { b.opening }).collect();
    let mut out = vec![false; n];
    let mut woke = vec![false; n];
    let mut placed: Vec<Placed> = Vec::new();
    let mut best: Option<(Who, f64)> = None;
    let mut m = lot.opens;
    while m <= end {
        for i in 0..n {
            let wakes = rivals[i].quiet_until == Some(m) && !woke[i];
            if next[i] != Some(m) && !wakes {
                continue;
            }
            if wakes {
                woke[i] = true;
            }
            next[i] = None;
            if out[i] || matches!(best, Some((Who::Rival(k), _)) if k == i) {
                continue;
            }
            let need = to_lead(best.map(|b| b.1), rivals[i].weight, lot);
            if need > rivals[i].limit {
                out[i] = true;
                continue;
            }
            let extra = rngs[i].int(0, rivals[i].character.jump()) * lot.step;
            let amount = (need + extra).min(rivals[i].limit);
            placed.push(Placed { at: m, who: Who::Rival(i), amount });
            best = Some((Who::Rival(i), amount as f64 * rivals[i].weight));
            answer(&mut next, &mut rngs, rivals, &out, &woke, Some(i), m);
        }
        for &(_, amount) in player.iter().filter(|p| p.0 == m) {
            let s = amount as f64 * player_weight;
            if amount >= lot.reserve && best.is_none_or(|b| s > b.1) {
                placed.push(Placed { at: m, who: Who::Player, amount });
                best = Some((Who::Player, s));
                answer(&mut next, &mut rngs, rivals, &out, &woke, None, m);
            }
        }
        // (on to the next minute anything happens)
        let soonest = next.iter().flatten().copied().chain(rivals.iter().zip(&woke).filter(|(_, w)| !**w).filter_map(|(b, _)| b.quiet_until)).chain(player.iter().map(|p| p.0)).filter(|t| *t > m).min();
        m = match soonest {
            Some(t) => t,
            None => break,
        };
    }
    placed
}

/// The rivals beaten (not `by`) answer after their delay; a quiet one keeps waiting.
fn answer(next: &mut [Option<i64>], rngs: &mut [Rng], rivals: &[Bidder], out: &[bool], woke: &[bool], by: Option<usize>, m: i64) {
    for j in 0..rivals.len() {
        if Some(j) == by || out[j] || (rivals[j].quiet_until.is_some() && !woke[j]) {
            continue;
        }
        let (a, b) = rivals[j].character.delay();
        let t = m + rngs[j].int(a, b);
        if next[j].is_none_or(|x| x > t) {
            next[j] = Some(t);
        }
    }
}

/// The best score of the bids so far.
pub fn best_score(placed: &[Placed], rivals: &[Bidder], player_weight: f64) -> Option<f64> {
    placed.last().map(|p| p.amount as f64 * if let Who::Rival(i) = p.who { rivals.get(i).map(|b| b.weight).unwrap_or(1.0) } else { player_weight })
}

/// What a rival looks like to the player: its character, its weight and its last bid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seen {
    pub character: Character,
    pub weight: f64,
    pub last: Option<Cents>,
}

/// The player's chance to win with a bid of `amount` now, as he can reckon it: every rival's
/// limit lies somewhere in its character's range of the line's value (`value` times the
/// economy's `factor`), at least what it bid already; it beats him if it reaches past his
/// weighed bid by a step.
pub fn chance(amount: Cents, player_weight: f64, rivals: &[Seen], value: Cents, factor: f64, lot: &Lot) -> f64 {
    let mut p = 1.0;
    for r in rivals {
        let (lo, hi) = r.character.limit();
        let hi = hi * value as f64 * factor;
        let lo = (lo * value as f64 * factor).max(r.last.unwrap_or(0) as f64);
        let need = to_lead(Some(amount as f64 * player_weight), r.weight, lot) as f64;
        let below = if hi <= lo { if need > lo { 1.0 } else { 0.0 } } else { ((need - lo) / (hi - lo)).clamp(0.0, 1.0) };
        p *= below;
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lot() -> Lot {
        Lot { opens: 600, closes: 600 + 240, reserve: 4_000_00, step: 300_00 }
    }

    fn rival(ch: Character, limit: Cents, seed: u64, opening: Option<i64>, quiet: Option<i64>) -> Bidder {
        Bidder { character: ch, limit, weight: weight(60.0), seed, opening, quiet_until: quiet }
    }

    #[test]
    fn rivals_answer_until_their_limit_and_the_last_bid_leads() {
        // (time enough for all their answers)
        let l = Lot { closes: 600 + 900, ..lot() };
        let rivals = vec![rival(Character::Aggressive, 9_000_00, 1, Some(610), None), rival(Character::Cautious, 7_000_00, 2, None, None)];
        let p = replay(&l, &rivals, &[], weight(60.0), l.closes);
        // the opener bids the reserve; the cautious one answers, and so on to its limit
        assert_eq!((p[0].at, p[0].who), (610, Who::Rival(0)));
        assert!(p[0].amount >= l.reserve && p[0].amount <= l.reserve + 2 * l.step);
        assert!(p.len() >= 3);
        assert!(p.windows(2).all(|w| w[0].at <= w[1].at && w[1].amount > w[0].amount));
        assert!(p.iter().all(|b| b.at < l.closes));
        assert!(p.iter().all(|b| b.amount <= rivals[match b.who { Who::Rival(i) => i, _ => unreachable!() }].limit));
        assert_eq!(p.last().unwrap().who, Who::Rival(0), "the one going higher leads at the end");
        // the same auction replays the same; a part of it is its beginning
        assert_eq!(p, replay(&l, &rivals, &[], weight(60.0), l.closes));
        let half = replay(&l, &rivals, &[], weight(60.0), 700);
        assert_eq!(&p[..half.len()], &half[..]);
        assert!(half.iter().all(|b| b.at <= 700));
    }

    #[test]
    fn a_bid_of_the_player_changes_only_what_comes_after() {
        let l = lot();
        let rivals = vec![rival(Character::Big, 8_000_00, 7, Some(620), None), rival(Character::Small, 5_000_00, 8, Some(700), None)];
        let w = weight(60.0);
        let before = replay(&l, &rivals, &[], w, l.closes);
        let at = 650;
        let mine = replay(&l, &rivals, &[(at, 10_000_00)], w, l.closes);
        let n = before.iter().filter(|b| b.at <= at).count();
        assert_eq!(&before[..n], &mine[..n]);
        // a bid above every limit wins
        assert_eq!(mine.last().unwrap().who, Who::Player);
        // a bid that does not lead is not taken
        let low = replay(&l, &rivals, &[(at, 4_000_00)], w, l.closes);
        assert!(!low.iter().any(|b| b.who == Who::Player));
    }

    #[test]
    fn a_quiet_rival_bids_in_the_last_minutes() {
        let l = lot();
        let rivals = vec![rival(Character::Aggressive, 9_000_00, 3, None, Some(l.closes - 5))];
        let p = replay(&l, &rivals, &[(620, 6_000_00)], weight(60.0), l.closes);
        assert_eq!(p.len(), 2);
        assert_eq!((p[1].at, p[1].who), (l.closes - 5, Who::Rival(0)));
        // a better name counts: the same sum of a better bidder leads
        assert!(to_lead(Some(6_000_00 as f64 * weight(40.0)), weight(90.0), &l) < to_lead(Some(6_000_00 as f64 * weight(90.0)), weight(40.0), &l));
    }

    #[test]
    fn the_chance_is_what_comes_out() {
        // the chance reckoned for a bid at the opening, and how often it wins
        let value = 10_000_00;
        let l = Lot { opens: 0, closes: 300, reserve: 4_000_00, step: 300_00 };
        let chars = [Character::Aggressive, Character::Cautious, Character::Small];
        let w = weight(55.0);
        for amount in [6_000_00, 9_000_00, 12_000_00] {
            let seen: Vec<Seen> = chars.iter().map(|c| Seen { character: *c, weight: weight(55.0), last: None }).collect();
            let reckoned = chance(amount, w, &seen, value, 1.0, &l);
            let mut wins = 0;
            let n = 600;
            for k in 0..n {
                let mut rng = Rng::new(k as u64 * 7919 + amount as u64);
                let rivals: Vec<Bidder> = chars
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        let (lo, hi) = c.limit();
                        Bidder { character: *c, limit: (rng.range(lo, hi) * value as f64) as Cents, weight: weight(55.0), seed: rng.next_u64() ^ i as u64, opening: None, quiet_until: None }
                    })
                    .collect();
                let p = replay(&l, &rivals, &[(1, amount)], w, l.closes);
                if p.last().is_some_and(|b| b.who == Who::Player) {
                    wins += 1;
                }
            }
            let got = wins as f64 / n as f64;
            assert!((got - reckoned).abs() < 0.08, "{amount}: reckoned {reckoned:.2}, won {got:.2}");
        }
        // more is a better chance; past every limit it is certain
        let seen = [Seen { character: Character::Big, weight: 1.0, last: Some(9_000_00) }];
        assert!(chance(9_500_00, 1.0, &seen, value, 1.0, &l) < chance(11_000_00, 1.0, &seen, value, 1.0, &l));
        assert_eq!(chance(20_000_00, 1.0, &seen, value, 1.0, &l), 1.0);
        assert_eq!(chance(8_000_00, 1.0, &seen, value, 1.0, &l), 0.0, "below what it bid already");
    }
}
