//! What the riders make of the driver: the ride, the greeting, the cash desk.

use super::*;

impl PeopleSim {
    /// The riders of the player's bus feel how it is driven (0x7d6964 - 0x7d6b7f): every
    /// jolt (`RideComfort::step`) takes the toll of the ride `(1 - x) * k` up for everybody
    /// walking or sitting in it, and whoever reaches a threshold says so (TooBad_A, _B, _C
    /// of the ticket pack's voices) - the third time getting off at the next stop. The
    /// toll eases off by 0.2 a kilometre (`pax_tick`). OMSI's passengers did this; here
    /// they never said a word about the driving (#862, #873).
    pub fn ride_comfort(&mut self, dt: f32, bus: Option<&VehicleInstance>, buses: &[BusNow], bus_ix: &HashMap<BusId, usize>, world: &dyn World) {
        let Some(v) = bus else { return };
        if dt <= 0.0 || self.avatar_only {
            return;
        }
        let a = v.physics.a_trans;
        let k = self.comfort.step(dt, self.time * 1000.0, v.physics.speed, a.x, a.y);
        if k <= 0.0 {
            return;
        }
        for i in 0..self.people.len() {
            if self.people[i].remote || self.people[i].puppet.is_some() {
                continue;
            }
            let Some(p) = self.pax(i) else { continue };
            if p.bus != Some(BusId::Player) || p.inside != Some(BusId::Player) || !matches!(p.task, Task::InBusToPlace | Task::InBusToExit | Task::SittingInBus) {
                continue;
            }
            if p.bad_at[2] <= 0.0 {
                let r = [self.rand_f() as f32, self.rand_f() as f32, self.rand_f() as f32];
                self.pax_mut(i).unwrap().bad_at = bad_ride_thresholds(r);
            }
            let p = self.pax_mut(i).unwrap();
            p.discomfort += (1.0 - p.discomfort) * k;
            let Some(c) = bad_ride_complaint(p.discomfort, p.complaint, p.bad_at) else { continue };
            p.complaint = c;
            if debug_pax() {
                log::info!("t={:.1} pax {} complains about the driving ({c}, toll {:.2})", self.time, self.people[i].label(), self.pax(i).unwrap().discomfort);
            }
            match c {
                1 => self.say_ex(i, "TooBad_A", true),
                2 => self.say_ex(i, "TooBad_B", true),
                _ => {
                    self.say_ex(i, "TooBad_C", true);
                    self.set_task(i, Task::InBusToExit, buses, bus_ix, world);
                }
            }
        }
    }

    /// The greeting or complaint stepping into the player's bus (0x62bf2d - 0x62c43c).
    pub fn greet_or_complain(&mut self, i: usize, bn: &BusNow) {
        let Some((whinge, chat)) = self.tickets.as_ref().map(|t| (t.whinge_prop, t.chattiness)) else { return };
        let air = bn.air;
        let mut complaint_seen = false;
        let mut code = 0u8;
        // too dark: the saloon light under half and dusk outside
        if bn.interior < 0.5 {
            let r = self.rand_f() as f32;
            if air.brightness < 0.2 + 0.3 * r {
                complaint_seen = true;
                if (self.rand_f() as f32) < whinge {
                    code = 1;
                }
            }
        }
        if let Some(t) = air.temp {
            let out = air.outside;
            let r = (self.rand() % 10) as f32 + 25.0;
            let hot = if t <= r {
                false
            } else {
                let r5 = (self.rand() % 5) as f32 + 3.0;
                t > r5 + out
            };
            let hot = hot || {
                let r = (self.rand() % 10) as f32;
                out * 0.5 + r + 20.0 < t && t < 25.0
            };
            if hot {
                complaint_seen = true;
                if code == 0 && (self.rand_f() as f32) < whinge {
                    code = if air.rel_hum <= 0.9 + 0.1 * self.rand_f() as f32 { 3 } else { 5 };
                }
            }
            let r = (self.rand() % 10) as f32 + 8.0;
            let cold = if t >= r {
                let r = (self.rand() % 10) as f32;
                t < (out - r) - 10.0
            } else {
                let r5 = (self.rand() % 5) as f32;
                t < out + 5.0 + r5 || {
                    let r = (self.rand() % 10) as f32;
                    t < (out - r) - 10.0
                }
            };
            if cold {
                complaint_seen = true;
                if code == 0 && (self.rand_f() as f32) < whinge {
                    code = 4;
                }
            }
        }
        if self.delay > 300.0 {
            complaint_seen = true;
            if code == 0 && (self.rand_f() as f32) < whinge {
                code = 2;
            }
        }
        let k = 1 + self.rand() % 2;
        match code {
            1 => self.say_ex(i, &format!("TooDark_{k}"), true),
            2 => self.say_ex(i, &format!("TooLate_{k}"), true),
            3 => self.say_ex(i, &format!("TooHot_{k}"), true),
            4 => self.say_ex(i, &format!("TooCold_{k}"), true),
            5 => self.say_ex(i, "TooWet_1", true),
            _ => {
                if (self.rand_f() as f32) < chat {
                    let h = (self.time_of_day.rem_euclid(86_400.0) / 3600.0).floor() as i32;
                    let daypart = if (3..=10).contains(&h) { 1 } else if (18..=23).contains(&h) { 2 } else { 0 };
                    let k = if daypart == 0 { self.rand() % 2 } else { self.rand() % 3 };
                    if k < 2 {
                        self.say_ex(i, &format!("Hello_{}", k + 1), false);
                    } else if daypart == 1 {
                        self.say_ex(i, "GoodMorning_1", false);
                    } else {
                        self.say_ex(i, "GoodEvening_1", false);
                    }
                }
            }
        }
        // OMSI's rating: people who stepped in, and those content
        self.stepped_in += 1;
        if !complaint_seen {
            self.content += 1;
        }
    }

    /// The ticket sale at the player's cash desk (case 4 with +0x61c = 3, 0x62c780 -
    /// 0x62d104): the ticket asked for, the money on the desk, the ticket taken, the change
    /// counted. The passenger asks again every 5 s (3 s after the second time) until the
    /// driver gets it right; the counter of those requests (`pardons`, the original's
    /// global at 0x859bc4) is shared by everybody at the desk.
    #[allow(clippy::too_many_arguments)]
    pub fn desk_sale(
        &mut self,
        i: usize,
        dt: f32,
        bn: &BusNow,
        player_bus: Option<&VehicleInstance>,
        taken_ticket: &mut bool,
    ) {
        let _ = dt;
        let p = self.pax(i).unwrap().clone();
        // the game plays the driver in `auto` boarding (a setting; OMSI has no such mode)
        let auto = self.boarding.eq_ignore_ascii_case("auto");
        let id = p.ticket_id as usize;
        let (name, value) = self
            .tickets
            .as_ref()
            .and_then(|t| t.tickets.get(id.saturating_sub(1)))
            .map(|t| (t.name.clone(), t.value))
            .unwrap_or_default();
        let tol = self.money.as_ref().map(|m| m.smallest_value()).unwrap_or(0.01) / 2.0;
        let owed = p.paid - p.price;
        // the change on the tray (sub_7e8900): enough of it, too much, too many coins
        let change = |h: &mut Self| -> (bool, bool, bool) {
            if auto {
                return (true, false, false);
            }
            let given = h.money.as_ref().map(|m| m.change_value()).unwrap_or(0.0);
            let too_much = tol < given - owed;
            let enough = owed - given <= tol;
            let needed = h.money.as_mut().map(|m| m.exact_coins_for(owed.max(0.0)).len()).unwrap_or(0) as f32;
            let count = h.money.as_ref().map(|m| m.change_count()).unwrap_or(0) as f32;
            let r = h.rand_f() as f32;
            let many = needed * (r + 1.5) <= count && count > 0.0;
            (enough && !too_much, too_much, many)
        };
        // the ticket given (sub_7e8f14): right, or a wrong one
        let ticket = |h: &Self| -> (bool, bool) {
            if auto || h.give_ticket {
                return (true, false);
            }
            let given = player_bus.and_then(|b| b.var("GivenTicket")).unwrap_or(-1.0);
            if given < 0.0 {
                return (false, false);
            }
            let ok = (given - (id as f32 - 1.0)).abs() < 0.5;
            (ok, !ok)
        };
        let (ch_ok, too_much, many) = if p.sub == 7 { change(self) } else { (false, false, false) };
        let (tk_ok, wrong) = if p.sub == 5 { ticket(self) } else { (false, false) };
        if p.sub == 3 {
            // the desk free (sub_7d1fec, +0x7a8): "Einmal ..., bitte"
            if self.desk_busy.is_some_and(|d| d != self.people[i].id) {
                return;
            }
            let k = 1 + self.rand() % 2;
            self.say_ex(i, &format!("Ticket_{id}_{k}"), false);
            self.ticket_requests += 1;
            self.request = Some((name, value));
            self.desk_busy = Some(self.people[i].id);
            self.pardons = 0;
            self.pardon_max = 0;
            let pp = self.pax_mut(i).unwrap();
            pp.talking = true;
            pp.timer = 0.5;
            pp.reach = true;
            pp.sub = 4;
            pp.paid = 0.0;
        } else if p.sub == 4 {
            if p.timer > 0.0 {
                return;
            }
            // the money on the desk (sub_7e8254)
            let point = bn.cabin.money_var.clone();
            let mut paid = value;
            if let Some(m) = self.money.as_mut() {
                let coins = if self.exact_fare || auto { m.exact_coins_for(value) } else { m.omsi_coins_for(value) };
                paid = m.value_of(&coins);
                if let Some((pos, var, parent)) = point {
                    // (put there when the view catches up: `show_bodies`)
                    self.bodies.ops.push(BodyOp::Coins { coins, point: pos, var, change: false, parent });
                }
            }
            self.paid = Some((paid, value));
            let pp = self.pax_mut(i).unwrap();
            pp.paid = paid;
            pp.sub = 5;
            pp.reach = false;
            pp.talking = false;
            pp.look_driver = false;
            pp.timer = 10.0;
        } else if p.sub == 5 && tk_ok {
            // the ticket: the hand to where it comes out
            let pp = self.pax_mut(i).unwrap();
            pp.sub = 6;
            pp.timer = 0.5;
            pp.reach = true;
            if let Some((_, t)) = bn.cabin.sale {
                pp.reach_at = t;
            }
            self.pardons = 0;
        } else if p.sub == 6 && p.timer <= 0.0 {
            self.pax_mut(i).unwrap().reach = false;
            self.tickets_sold += 1;
            self.ticket_cash += value;
            self.sales.push((name, value));
            *taken_ticket = true;
            if let Some(m) = self.money.as_mut() {
                m.clear(false);
            }
            self.paid = None;
            let pp = self.pax_mut(i).unwrap();
            if (p.price - p.paid).abs() > tol && !auto {
                pp.sub = 7;
                pp.timer = 5.0;
                self.change_due = Some(owed);
            } else {
                pp.sub = 8;
                self.change_due = Some(0.0);
            }
            let pp = self.pax_mut(i).unwrap();
            pp.talking = false;
            pp.look_driver = false;
        } else if p.sub == 7 && (ch_ok || (self.pardons > 1 && too_much)) {
            let pp = self.pax_mut(i).unwrap();
            pp.sub = 8;
            pp.timer = 0.5;
            pp.reach = true;
            pp.bad_change = many;
            if let Some(c) = bn.cabin.change_point {
                pp.reach_at = c;
            }
            if many {
                self.say_ex(i, "BadChange_1", true);
                self.ticket_points += 1;
            } else {
                if !too_much {
                    self.ticket_points += 2;
                }
                let r = self.rand_f() as f32;
                if !too_much && self.tickets.as_ref().is_some_and(|t| r < t.chattiness) {
                    self.say_ex(i, "Thanks_1", false);
                }
            }
        } else if p.sub == 8 && p.timer <= 0.0 {
            if let Some(m) = self.money.as_mut() {
                m.clear(true);
            }
            self.change_due = None;
            self.request = None;
            self.desk_busy = None;
            self.boarded += 1;
            self.served += 1;
            self.route_to_place(i, bn);
            let pp = self.pax_mut(i).unwrap();
            pp.reach = false;
            pp.talking = false;
            pp.look_driver = false;
            pp.sub = 0;
            pp.ticket = TICKET_NONE;
            pp.pt = bn.cabin.sale.and_then(|s| s.0);
        } else if (p.sub == 5 || p.sub == 7) && (p.timer <= 0.0 || (too_much && self.pardons == 0)) {
            // asking again (0x62ce80)
            let n = self.pardons as u64;
            let r = self.rand() % (n + 2);
            self.pax_mut(i).unwrap().timer = if n < 2 { 5.0 } else { 3.0 };
            let line = if (n == 0 || r == 0) && p.sub == 5 {
                if wrong { "BadTicket_A".to_string() } else { "PardonTicket_1".to_string() }
            } else if wrong && n == 1 && p.sub == 5 {
                "BadTicket_B".to_string()
            } else if !too_much && n == 0 && p.sub == 7 {
                "TooFew_A".to_string()
            } else if !too_much && n == 1 && p.sub == 7 {
                "TooFew_B".to_string()
            } else if too_much && n == 0 && p.sub == 7 {
                "TooMuch_A".to_string()
            } else if too_much && n == 1 && p.sub == 7 {
                self.pardons = 2;
                "TooMuch_B".to_string()
            } else {
                format!("Pardon_{}", r.min(3))
            };
            self.say_ex(i, &line, true);
            self.pax_mut(i).unwrap().talking = true;
            let skip = self.pardons != 0 && self.rand_f() >= 0.7;
            if !skip {
                self.pardons = self.pardons.saturating_add(1);
            }
        }
        self.pardon_max = self.pardon_max.max(self.pardons);
    }
}
