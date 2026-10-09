//! Rings of cars that wait on each other: found on the waits-for graph (`AiCar::waits_on`,
//! who stands for whom) and broken by letting one of them go.
//!
//! Inside a big junction a car can end up standing in the meeting place of another path -
//! its queue stopped while it was crossing - and the car on that path waits for it, while
//! the queue it is in waits for the one that gives way to it further on. Nobody in such a
//! ring ever moves by the rules alone: before, it stood until every car in it had given up
//! (60-150 s) and was taken away out of sight, and in view it stood for good. Now one car
//! of the ring that only gives way by the rules of the junction (not to a body in its way)
//! goes after a few seconds - the one that has waited longest, as drivers sort it out by
//! hand signals - while its own body check (`body_in_way`) still keeps it from driving into
//! anybody. If that does not get the ring moving, the next one tries.

use super::stats::waits_for_cycles;
use super::*;

/// Seconds every car of a ring has stood before one of them goes.
pub const DEADLOCK_WAIT: f32 = 6.0;
/// Seconds the one chosen may disregard the junction's rules.
pub const DEADLOCK_PASS: f32 = 5.0;

/// Which member of a ring goes: of those held only by a junction's rules (`yielding`), the
/// one that has stood longest and was not tried last (`tried` the time it last was) - a
/// tie to the lower id. `members`: (index, stood s, yielding, tried, id).
pub fn ring_breaker(members: &[(usize, f32, bool, f32, u64)]) -> Option<usize> {
    members
        .iter()
        .filter(|m| m.2)
        .min_by(|a, b| {
            a.3.total_cmp(&b.3)
                .then(b.1.total_cmp(&a.1))
                .then(a.4.cmp(&b.4))
        })
        .map(|m| m.0)
}

impl TrafficSim {
    /// Find the rings of cars waiting on each other and let one of each go (see the module).
    pub fn break_rings(&mut self) {
        let stood = |c: &AiCar| c.stopped.max(c.progress.1);
        let waits: Vec<Option<usize>> = self
            .cars
            .iter()
            .map(|c| {
                if stood(c) < DEADLOCK_WAIT {
                    return None;
                }
                c.waits_on
                    .and_then(|id| self.index_of.get(&id).copied())
                    .filter(|&k| k < self.cars.len() && stood(&self.cars[k]) >= DEADLOCK_WAIT)
            })
            .collect();
        // (cheap: most frames nobody waits on anybody standing)
        if waits.iter().all(|w| w.is_none()) {
            return;
        }
        for ring in waits_for_cycles(&waits) {
            if ring.iter().any(|&k| self.cars[k].deadlock_pass > self.time) {
                continue; // one of them is going already
            }
            let members: Vec<(usize, f32, bool, f32, u64)> = ring
                .iter()
                .map(|&k| {
                    let c = &self.cars[k];
                    (k, stood(c), c.why.0 == "yield", c.deadlock_tried, c.id)
                })
                .collect();
            let Some(go) = ring_breaker(&members) else { continue };
            let c = &mut self.cars[go];
            c.deadlock_pass = self.time + DEADLOCK_PASS;
            c.deadlock_tried = self.time;
            if omsi_cfg::flags::OMSI_DEBUG_STUCK.is_set() || omsi_cfg::flags::OMSI_DEBUG_TRAFFIC.is_set() {
                let ids: Vec<u64> = ring.iter().map(|&k| self.cars[k].id).collect();
                log::info!("t={:.1}: cars {ids:?} wait on each other in a ring: car {} goes", self.time, self.cars[go].id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_longest_waiter_held_by_the_rules_goes() {
        // (index, stood, yielding, tried, id)
        let ring = [(0, 30.0, false, f32::MIN, 1), (1, 20.0, true, f32::MIN, 2), (2, 25.0, true, f32::MIN, 3)];
        assert_eq!(ring_breaker(&ring), Some(2));
        // the one tried last waits for the others' turn
        let ring = [(1, 20.0, true, f32::MIN, 2), (2, 25.0, true, 100.0, 3)];
        assert_eq!(ring_breaker(&ring), Some(1));
        // nobody held only by the rules: a body is in everybody's way
        assert_eq!(ring_breaker(&[(0, 9.0, false, f32::MIN, 1)]), None);
    }
}
