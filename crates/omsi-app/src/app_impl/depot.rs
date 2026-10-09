//! The depot services: the fuel pump, the bus wash and the repair.

use super::*;

impl App {
    /// The running fuel pump or bus wash, one frame of it: its trigger with the frame's
    /// time; done once the tank (the dirt) has not changed for `SERVICE_SETTLE` seconds, and
    /// stopped by driving off. Either way the plugins hear of it (`service`).
    pub(crate) fn tick_service(&mut self, dt: f32) {
        const SERVICE_SETTLE: f32 = 1.5;
        let Some(pumping) = self.session.pumping else { return };
        let kind = pumping.kind;
        let Some(p) = self.player.as_mut() else {
            self.session.pumping = None;
            return;
        };
        let (name, var) = if kind == "refuel" { ("veh_tank", "engine_tank_content") } else { ("veh_wash", "Dirt_Wiped") };
        if p.vehicle.physics.velocity_kmh().abs() > 2.0 {
            let now = p.vehicle.var(var).unwrap_or(0.0);
            self.session.pumping = None;
            self.service_msg = Some((if kind == "refuel" { "Refuelling stopped" } else { "Washing stopped" }.into(), 4.0));
            self.service_event(kind, pumping.by, Some(if kind == "refuel" { now - pumping.from } else { now }));
            return;
        }
        let before = p.vehicle.var(var).unwrap_or(0.0);
        p.vehicle.service(name, dt);
        let now = p.vehicle.var(var).unwrap_or(0.0);
        let idle = if (now - before).abs() > 1e-4 { 0.0 } else { pumping.idle + dt };
        if idle > SERVICE_SETTLE {
            self.session.pumping = None;
            let line = if kind == "refuel" { format!("refuelled: {now:.0} l in the tank") } else { format!("washed: dirt {:.0}%", now * 100.0) };
            log::info!("{line}");
            self.service_msg = Some((line, 6.0));
            self.service_event(kind, pumping.by, Some(if kind == "refuel" { now - pumping.from } else { now }));
        } else {
            self.session.pumping = Some(Pumping { idle, ..pumping });
            if kind == "refuel" {
                self.service_msg = Some((format!("Refuelling: {now:.0} l"), 1.0));
            }
        }
    }

    /// One of the depot services of the game menu: "refuel", "wash" or "repair", asked for
    /// by `by` (the `service` event's).
    pub(crate) fn run_service(&mut self, kind: &str, by: &'static str) {
        let Some(w) = self.world.clone() else { return };
        let Some(p) = self.player.as_mut() else { return };
        // The pump and the wash run as OMSI's do, `veh_tank` / `veh_wash` every frame
        // while they are on, the tank filling litre by litre (`tick_service`); it had been
        // full the moment the menu was clicked (#1785). (Off a station: the message below.)
        if matches!(kind, "refuel" | "wash") && at_petrol_station(&w, &p.vehicle) {
            let name = if kind == "refuel" { "veh_tank" } else { "veh_wash" };
            if !p.vehicle.service(name, 0.0) {
                self.service_msg = Some((format!("this vehicle has no {} handling ({name})", if kind == "refuel" { "fuel pump" } else { "bus wash" }), 6.0));
                return;
            }
            if kind == "wash" {
                p.vehicle.dirt = 0.0;
                p.vehicle.set_engine_var("Dirt_Norm", 0.0);
            }
            let from = p.vehicle.var("engine_tank_content").unwrap_or(0.0);
            self.session.pumping = Some(Pumping { kind: if kind == "refuel" { "refuel" } else { "wash" }, idle: 0.0, from, by });
            self.service_msg = Some((if kind == "refuel" { "Refuelling... (drive off to stop)" } else { "Washing..." }.into(), 4.0));
            return;
        }
        let one = Args {
            refuel: kind == "refuel",
            wash: kind == "wash",
            repair: kind == "repair",
            ..self.args.clone()
        };
        let at_station = at_petrol_station(&w, &p.vehicle);
        let mut clock = self.clock.clone();
        let msg = run_services(&one, &mut p.vehicle, &mut clock, w.global.repair_time_min, at_station);
        // (the game minutes the repair took, the team's way there too)
        let repaired = (kind == "repair" && msg.iter().any(|m| m.starts_with("repaired"))).then(|| ((clock.time - self.clock.time) / 60.0) as f32);
        while clock.time >= 86400.0 {
            clock.time -= 86400.0;
            clock.day_of_year = clock.day_of_year % omsi_sim::clock::days_in_year(clock.year) + 1;
        }
        if !self.settings.time_sync || self.net.lan.as_ref().is_some_and(|l| l.role == omsi_net::Role::Client) {
            self.clock = clock;
        }
        p.vehicle.host.clock = self.clock.clone();
        for line in &msg {
            log::info!("{line}");
        }
        if let Some(line) = msg.into_iter().next() {
            self.service_msg = Some((line, 6.0));
        }
        if let Some(minutes) = repaired {
            self.service_event("repair", by, Some(minutes));
        }
    }

    /// Who runs a line of the game menu now: a plugin's `omsi.command`, or the player.
    pub(crate) fn menu_by(&self) -> &'static str {
        if self.integrations.plugin_command {
            "plugin"
        } else {
            "player"
        }
    }

    /// Tell the plugins the player's bus was refuelled, washed, repaired, put back on its
    /// wheels or moved (`service`: `kind`, who did it, and the amount of the kind).
    pub(crate) fn service_event(&mut self, kind: &'static str, by: &'static str, amount: Option<f32>) {
        if self.player.is_none() {
            return;
        }
        use omsi_plugin::InfoValue::{Num, Text};
        let mut args = vec![Text(kind.into()), Text(by.into())];
        args.extend(amount.map(|a| Num(a as f64)));
        crate::plugins::queue_event(&mut self.integrations.plugin_events, "service", args);
    }
}
