//! Coupled vehicles: trailers, rear sections and the cars of a train.

use super::*;
use crate::scene::World;
use omsi_render::{Renderer, Scene};

impl Traffic {
    /// Attach explicitly listed cars (a `.zug` train): (type, reversed).
    pub fn attach_cars(
        &mut self,
        view: &mut TrafficView,
        world: &World,
        renderer: &Renderer,
        scene: &mut Scene,
        car: usize,
        cars: &[(Arc<VehicleType>, bool)],
    ) {
        let c = &mut self.sim.cars[car];
        for (t, rev) in cars {
            view.add_trailer(world, renderer, scene, c.id, t);
            c.vehicle.attach_trailer_ex(t.clone(), *rev);
        }
    }

    /// The cars of car `ci`'s train, front to back, each with whether it is turned round
    /// (the first, the one that drives, is not).
    pub(crate) fn consist(&self, ci: usize) -> Vec<(Arc<VehicleType>, bool)> {
        let v = &self.sim.cars[ci].vehicle;
        std::iter::once((v.ty.clone(), false)).chain(v.trailers.iter().map(|t| (t.ty.clone(), t.reversed))).collect()
    }

    /// Couple `cars` behind car `ci` instead of the ones it has (a train made up anew).
    pub(crate) fn set_trailers(&mut self, view: &mut TrafficView, world: &World, renderer: &Renderer, scene: &mut Scene, ci: usize, cars: &[(Arc<VehicleType>, bool)]) {
        let c = &mut self.sim.cars[ci];
        view.release_trailers(world, renderer, scene, c.id);
        c.vehicle.trailers.clear();
        self.attach_cars(view, world, renderer, scene, ci, cars);
    }

    /// Turn train `ci` round as Omsi.exe does for a trip whose `[trainreverse]` differs from
    /// how the train stands (0x613a98): the whole consist the other way, its last car
    /// leading - here the vehicle that drives is made anew as that car, standing where it
    /// stands (at `s` on `lane`, which runs the new way), and the others coupled behind it
    /// in the opposite order, each turned round. `behind`: the lanes the train has behind
    /// it now, nearest last, for the track its cars stand on. The car keeps its id and its
    /// service.
    pub(crate) fn turn_train(&mut self, view: &mut TrafficView, world: &World, renderer: &Renderer, scene: &mut Scene, ci: usize, lane: usize, s: f32, behind: &[usize], reversed: bool) {
        let cars: Vec<(Arc<VehicleType>, bool)> = self.consist(ci).into_iter().rev().map(|(t, r)| (t, !r)).collect();
        let Some((lead, lead_turned)) = cars.first().cloned() else { return };
        if lead_turned {
            // (a lead car turned round is drawn facing the way: none of the stock trains has one)
            log::debug!("train {}: its last car leads turned round", self.sim.cars[ci].id);
        }
        let (id, seed, scheme) = (self.sim.cars[ci].id, self.sim.cars[ci].seed, self.sim.cars[ci].scheme);
        // (where its cars stood, front to back: they stand there still, the other way round)
        let before: Vec<DVec3> = std::iter::once(self.sim.cars[ci].vehicle.position).chain(self.sim.cars[ci].vehicle.trailers.iter().map(|t| t.position)).collect();
        let center = self.sim.viewer.map(|v| v.pos).unwrap_or_default();
        let kind = self.sim.net.lanes[lane].kind;
        // (the new car takes the id: the old one's renders are let go once it is replaced)
        let old_render = view.take(id);
        self.create_car(view, world, renderer, scene, center, kind, lane, s, lead, seed, Some(scheme), Some(id), Some(0.0), None);
        let Some(mut new) = self.sim.cars.pop() else { return };
        let old = &mut self.sim.cars[ci];
        // its service goes with it (its line and destination are set for the trip it takes
        // on); the way it drives, from where it stands
        new.vehicle.host.hof = old.vehicle.host.hof.clone();
        new.bus = old.bus.take();
        new.state.max_speed_kmh = old.state.max_speed_kmh;
        new.state.length = old.state.length;
        new.state.accel = old.state.accel;
        new.state.decel = old.state.decel;
        new.state.lat_accel = old.state.lat_accel;
        new.state.min_gap = old.state.min_gap;
        new.state.speed = 0.0;
        new.consist_reversed = reversed;
        let old = std::mem::replace(&mut self.sim.cars[ci], new);
        self.drop_sounds(old.id);
        if let Some(r) = old_render {
            release_car_render(world, renderer, scene, r);
        }
        self.set_trailers(view, world, renderer, scene, ci, &cars[1..]);
        self.seed_rail_trail(ci, behind);
        let c = &mut self.sim.cars[ci];
        let trail = &c.rail_trail;
        let (state, net) = (&c.state, &self.sim.net);
        c.vehicle.retrail(0.0, &|d| Some(rail_behind(trail, state, net, d)));
        let after: Vec<DVec3> = std::iter::once(c.vehicle.position).chain(c.vehicle.trailers.iter().map(|t| t.position)).collect();
        let moved = before.iter().rev().zip(&after).map(|(a, b)| (*a - *b).truncate().length()).fold(0.0f64, f64::max);
        log::info!(
            "train {id} turned round: {} (its cars {:.1} m at most from where they stood)",
            std::iter::once(&c.vehicle.ty).chain(c.vehicle.trailers.iter().map(|t| &t.ty)).map(|t| t.def.path.file_stem().unwrap_or_default().to_string_lossy().to_string()).collect::<Vec<_>>().join(" + "),
            moved
        );
    }

    /// The track behind rail car `ci` as it has just been put on its lane: back along its
    /// lane and then `behind` (the lanes before it, nearest last), for its coupled cars.
    pub(super) fn seed_rail_trail(&mut self, ci: usize, behind: &[usize]) {
        let c = &mut self.sim.cars[ci];
        let net = &self.sim.net;
        let odo = c.state.odometer as f64;
        let mut pts: Vec<(f64, DVec3)> = Vec::new();
        let (mut lane, mut s) = (c.state.lane, c.state.s);
        let mut back = behind.iter().rev();
        let mut d = 0.0f64;
        while d <= RAIL_TRAIL {
            pts.push((odo - d, net.lanes[lane].at(s.max(0.0)).0));
            s -= 1.0;
            if s < 0.0 {
                match back.next() {
                    Some(&l) => {
                        s += net.lanes[l].length();
                        lane = l;
                    }
                    None => break,
                }
            }
            d += 1.0;
        }
        c.rail_trail = pts.into_iter().rev().collect();
    }

    /// The vehicles coupled behind `ty` (its rear sections, trailers, the cars of a unit),
    /// each with whether it is turned round, loaded (once per file). As Omsi.exe builds a
    /// consist (0x70a174): towards the back of the train a vehicle goes on with its
    /// `[couple_back]`, or with its `[couple_front]` when it is itself turned round; the
    /// coupled one is turned round when the coupling's flag says so, against the one it
    /// hangs on; and a coupling back to the file it came from that turns nothing round is
    /// not followed. (Following `[couple_back]` whatever the way, the Berlin A3's unit -
    /// the S car and its K car turned round behind it, whose own `[couple_back]` names the
    /// S car again - went on S, K, S, K, S, none of them turned.)
    pub(crate) fn trailer_chain(&mut self, ty: &Arc<VehicleType>) -> Vec<(Arc<VehicleType>, bool)> {
        self.coupled_chain(ty, false, true)
    }

    /// See [`Traffic::trailer_chain`]: from `ty` (turned round: `rev`) towards the back of
    /// the train, or towards its front, the nearest first.
    pub(crate) fn coupled_chain(&mut self, ty: &Arc<VehicleType>, rev: bool, toward_back: bool) -> Vec<(Arc<VehicleType>, bool)> {
        let mut out = Vec::new();
        let (mut lead, mut lead_rev) = (ty.clone(), rev);
        let mut seen = vec![lead.def.path.clone()];
        for _ in 0..crate::spawn::MAX_COUPLED_PARTS {
            let Some((path, r)) = crate::spawn::next_coupled(&lead.def, lead_rev, toward_back) else {
                break;
            };
            // a consist that comes round again ends here (see `spawn::chain_has_part`)
            if crate::spawn::chain_has_part(&seen, &path) {
                break;
            }
            let root = self.sim.root.clone();
            let t =
                self.sim.trailer_types.entry(path.clone()).or_insert_with(
                    || match VehicleType::load_ai(&root, &path) {
                        Ok(t) => Some(Arc::new(t)),
                        Err(e) => {
                            log::warn!("trailer {}: {e}", path.display());
                            None
                        }
                    },
                );
            let Some(t) = t.clone() else { break };
            out.push((t.clone(), r));
            seen.push(path);
            lead = t;
            lead_rev = r;
        }
        out
    }
}
