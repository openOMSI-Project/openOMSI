//! What the app asks of the people: counts, positions, door and stop requests, and the
//! things that happened since it last looked.

use super::*;

impl PeopleSim {
    /// The buses somebody stamped a ticket in since the last call (the app fires their
    /// `ev_Stamper` sound trigger): `None` the player's, else the AI car's id.
    pub fn take_stamped(&mut self) -> Vec<Option<u64>> {
        std::mem::take(&mut self.stamped)
            .into_iter()
            .map(|b| match b {
                BusId::Ai(id) => Some(id),
                _ => None,
            })
            .collect()
    }

    /// Lines passengers said since the last call (the app plays them where they stand).
    pub fn take_voice_lines(&mut self) -> Vec<VoiceLine> {
        std::mem::take(&mut self.voice_lines)
    }

    /// The footsteps taken since the last call, for the environment sounds. They pile up
    /// only between two frames; a run without audio never looks at them, so the list is
    /// dropped once it grows past a crowd's worth of steps.
    pub fn take_footfalls(&mut self) -> Vec<Footfall> {
        if self.footfalls.len() > 256 {
            self.footfalls.clear();
        }
        std::mem::take(&mut self.footfalls)
    }

    /// Everybody: (walking, waiting at a stop, in a bus).
    pub fn counts(&self) -> (usize, usize, usize) {
        let (mut walking, mut waiting, mut aboard) = (0, 0, 0);
        for p in &self.people {
            match (&p.place, &p.state) {
                (Place::Bus(..), _) => aboard += 1,
                (Place::Ground, State::Pax(x)) if x.inside.is_none() => waiting += 1,
                (Place::Ground, _) => walking += 1,
            }
        }
        (walking, waiting, aboard)
    }

    /// People currently in the player's bus.
    pub fn riding(&self) -> usize {
        self.people
            .iter()
            .filter(|p| p.inside(BusId::Player))
            .count()
    }

    /// People walking the footpaths of the traffic network: (lane, distance along it). The
    /// traffic gives way to them at crossings and presses the pedestrian lights' buttons
    /// for them.
    /// Everybody on foot on the ground, for the traffic to stop for: position, velocity
    /// and whether they wait at a stop (a bus pulls up right beside those).
    pub fn on_foot(&self) -> Vec<(DVec3, DVec2, bool)> {
        self.people
            .iter()
            .filter(|p| p.place == Place::Ground)
            .map(|p| {
                let waiting = matches!(&p.state, State::Pax(x) if x.inside.is_none());
                (p.position, p.vel, waiting)
            })
            .collect()
    }

    pub fn strollers(&self) -> Vec<(usize, f32)> {
        self.people
            .iter()
            .filter_map(|p| match &p.state {
                State::Strolling(walk) => walk
                    .legs
                    .get(walk.leg)
                    .map(|leg| (leg.lane, leg.dist(walk.s))),
                _ => None,
            })
            .collect()
    }

    /// Report the waiting and alighting passengers to the bus script, the way OMSI does.
    pub fn write_pax_vars(&self, b: &mut VehicleInstance) {
        let doors = DoorWants {
            entry_req: self.entry_req.clone(),
            exit_req: self.exit_req.clone(),
            entry_busy: self.entry_busy.clone(),
            exit_busy: self.exit_busy.clone(),
            places: self.pax_places.get(&BusId::Player).cloned().unwrap_or_default(),
        };
        Self::write_door_requests(b, &doors);
    }

    /// Who wants a timetable bus to stop, as Omsi.exe asks before it lets one pull in
    /// (0x7da91f): the AI buses with somebody aboard on the way to a door to get off
    /// (task 5), and the stops where somebody is waiting for a bus or walking to one
    /// (tasks 1 to 3).
    pub fn stop_wishes(&self) -> (HashSet<u64>, HashSet<i64>) {
        let (mut alighting, mut waiting) = (HashSet::new(), HashSet::new());
        for p in &self.people {
            let State::Pax(x) = &p.state else { continue };
            match x.task {
                Task::InBusToExit => {
                    if let Some(BusId::Ai(id)) = x.inside {
                        alighting.insert(id);
                    }
                }
                Task::WaitingForBus | Task::ToBus | Task::WalkingToBus => {
                    if let Some(s) = x.stop {
                        waiting.insert(s);
                    }
                }
                // somebody late, still on the way up to the stop
                Task::WalkingToBusstop if x.late.is_some() && x.inside.is_none() => {
                    if let Some(s) = x.stop {
                        waiting.insert(s);
                    }
                }
                _ => {}
            }
        }
        (alighting, waiting)
    }

    /// How full each timetable bus is: passengers aboard over its places (seats and
    /// standing), for the departure displays. Only buses whose cabin is known.
    pub fn bus_loads(&self) -> HashMap<u64, f32> {
        let mut aboard: HashMap<u64, usize> = HashMap::new();
        for p in &self.people {
            if let State::Pax(x) = &p.state {
                if let Some(BusId::Ai(id)) = x.inside {
                    *aboard.entry(id).or_insert(0) += 1;
                }
            }
        }
        self.seats
            .iter()
            .filter_map(|(id, places)| match id {
                BusId::Ai(id) if !places.is_empty() => {
                    Some((*id, (aboard.get(id).copied().unwrap_or(0) as f32 / places.len() as f32).min(1.0)))
                }
                _ => None,
            })
            .collect()
    }

    /// Timetable buses to hold at their stop, for the traffic.
    pub fn take_holds(&mut self) -> Vec<(u64, Option<i64>, f32)> {
        std::mem::take(&mut self.holds)
    }

    /// The tickets sold at the cash desk since the last call: name and value. Lua plugins get
    /// them as the `ticket_sold` event.
    pub fn take_sales(&mut self) -> Vec<(String, f32)> {
        std::mem::take(&mut self.sales)
    }

    /// Door requests for the timetable buses, for the traffic to hand to their scripts.
    pub fn take_ai_requests(&mut self) -> Vec<(u64, DoorWants)> {
        std::mem::take(&mut self.ai_requests)
    }

    /// A line for the HUD about something that just happened.
    pub fn take_message(&mut self) -> Option<String> {
        self.message.take()
    }

    /// What the driver should do now, for the HUD: a passenger waiting at the cash desk
    /// for the ticket (only when the driver has to sell it).
    pub fn hint(&self) -> Option<String> {
        if !self.boarding.eq_ignore_ascii_case("pay") {
            return None;
        }
        self.people.iter().find(|p| matches!(&p.state, State::Pax(x) if x.inside == Some(BusId::Player) && x.ticket == TICKET_BUY && x.sub == 5))?;
        let (name, value) = self.request.clone()?;
        Some(format!("Passenger waiting for a ticket: {name} {value:.2} - press {}", self.ticket_key))
    }

    /// People sitting on each `[passpos]` of the player's bus, for `GetHumanCountOnSeat`
    /// (the BVG Citaro folds its tip-up seats down when somebody sits on them).
    pub fn seat_counts(&self) -> Vec<u32> {
        let Some(cabin) = self.player_cabin.as_ref() else {
            return Vec::new();
        };
        let sitting = self.people.iter().filter_map(|p| match &p.state {
            State::Pax(x) if x.inside == Some(BusId::Player) && x.task == Task::SittingInBus => x.seat,
            _ => None,
        });
        seat_numbers(&cabin.seats, sitting)
    }

    /// How many people stand on each `paths.cfg` link inside the player's bus, for the
    /// scripts' `GetHumanCountOnPathLink` (the NL/NG uses it for the fare gate).
    pub fn path_link_counts(&self) -> Vec<u32> {
        let Some(cabin) = self.player_cabin.as_ref() else {
            return Vec::new();
        };
        // (the link each walker is on, +0x698)
        let mut out = vec![0u32; cabin.links.len()];
        for p in &self.people {
            if let State::Pax(x) = &p.state {
                if x.inside == Some(BusId::Player) && (x.st == 1 || x.st == 5 || x.st == 9) {
                    if let Some(c) = x.link.and_then(|l| out.get_mut(l)) {
                        *c += 1;
                    }
                }
            }
        }
        out
    }

    /// Where everybody is, for logs.
    pub fn positions(&self) -> Vec<(String, DVec3)> {
        self.people
            .iter()
            .map(|p| (p.state.name().to_string(), p.position))
            .collect()
    }

    /// Count of people per state, for logs (omsi-app's `Humans::summary` adds the posing).
    pub fn summary(&self) -> String {
        let mut counts: std::collections::BTreeMap<&'static str, usize> = Default::default();
        for p in &self.people {
            *counts.entry(p.state.name()).or_default() += 1;
        }
        let mut out = counts
            .iter()
            .map(|(k, v)| format!("{v} {k}"))
            .collect::<Vec<_>>()
            .join(", ");
        let (n, total, worst) = self.tick_stats;
        if n > 0 {
            out.push_str(&format!(
                "; {:.2} ms a frame, longest {worst:.1} ms",
                total / n as f64
            ));
        }
        out
    }
}
