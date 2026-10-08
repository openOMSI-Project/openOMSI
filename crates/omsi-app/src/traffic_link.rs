//! What the traffic is told about the player and the LAN buses.

use super::*;

/// Position, heading, half extents and speed of the player's vehicle for the AI's
/// obstacle checks.
pub(crate) fn player_outline(p: &Player) -> traffic::PlayerBox {
    vehicle_outline(&p.vehicle, p.vehicle.physics.speed)
}

/// `player_outline` of any vehicle moving at `speed` (m/s).
pub(crate) fn vehicle_outline(v: &omsi_sim::VehicleInstance, speed: f32) -> traffic::PlayerBox {
    let bb =
        v.ty.def
            .bounding_box
            .unwrap_or([2.5, 11.0, 3.0, 0.0, 0.0, 1.5]);
    box_outline(v.position, v.heading, bb, speed)
}

pub(crate) fn trailer_outline(
    t: &omsi_sim::vehicle::TrailerPart,
    speed: f32,
) -> Option<traffic::PlayerBox> {
    t.ty.def
        .bounding_box
        .map(|bb| box_outline(t.position, t.heading, bb, speed))
}

fn box_outline(position: DVec3, heading: f64, bb: [f32; 6], speed: f32) -> traffic::PlayerBox {
    let h = heading.to_radians();
    let centre = position
        + DVec3::new(
            (bb[3] as f64) * h.cos() + (bb[4] as f64) * h.sin(),
            -(bb[3] as f64) * h.sin() + (bb[4] as f64) * h.cos(),
            0.0,
        );
    (centre, heading, bb[1] * 0.5, bb[0] * 0.5, speed)
}

/// Everything of the player's besides the bus's own box that the traffic has to keep out
/// of: the rear sections of an articulated bus or a coupled trailer (the traffic saw only
/// the front section and drove into the back of a turning GN92), and the vehicles placed
/// by hand from the vehicle list, with their trailers - under ids of their own beside the
/// LAN players'.
pub(crate) fn own_outlines(player: Option<&Player>, placed: &[Player]) -> Vec<(u32, traffic::PlayerBox)> {
    let mut out = Vec::new();
    let mut add = |v: &omsi_sim::VehicleInstance, base: u32, whole: bool| {
        let speed = v.physics.speed;
        if whole {
            out.push((base, vehicle_outline(v, speed)));
        }
        for (k, t) in v.trailers.iter().enumerate() {
            if let Some(outline) = trailer_outline(t, speed) {
                out.push((base + 1 + k as u32, outline));
            }
        }
    };
    if let Some(p) = player {
        add(&p.vehicle, 0xFFFF_0000, false);
    }
    for (i, q) in placed.iter().enumerate() {
        add(&q.vehicle, 0xFFFE_0000 - (i as u32) * 16, true);
    }
    out
}

/// The LAN players' buses as obstacles for the AI traffic (their speed as last sent).
pub(crate) fn lan_outlines(game: &lan::LanGame) -> Vec<(u32, traffic::PlayerBox)> {
    game.remotes
        .iter()
        .map(|(id, r)| (*id, vehicle_outline(r.vehicle(), r.last.speed_kmh / 3.6)))
        .collect()
}

/// Keep the state of a vehicle beside its outline: a scenery path marked as a turn
/// detects that direction, even if the path overlaps another road or its lamps are dark
/// during the blink cycle. Rear sections carry their towing vehicle's indicator state.
pub(crate) fn outline_indicators(
    game: &lan::LanGame,
    player: Option<&Player>,
    placed: &[Player],
) -> hashbrown::HashMap<u32, u8> {
    let mut out: hashbrown::HashMap<u32, u8> = game
        .remotes
        .iter()
        .map(|(id, r)| (*id, r.last.blinker))
        .collect();
    let mut add = |v: &omsi_sim::VehicleInstance, base: u32| {
        let indicator = lan::indicator(v);
        out.insert(base, indicator);
        for (k, _) in v.trailers.iter().enumerate() {
            out.insert(base + 1 + k as u32, indicator);
        }
    };
    if let Some(p) = player {
        add(&p.vehicle, 0xFFFF_0000);
    }
    for (i, q) in placed.iter().enumerate() {
        add(&q.vehicle, 0xFFFE_0000 - i as u32 * 16);
    }
    out
}

/// What the traffic needs to know every frame besides the time: where the player looks
/// from, the day of the week, who walks the footpaths, what hides what.
pub(crate) fn traffic_inputs(
    t: &mut traffic::Traffic,
    cam: Option<&Camera>,
    aspect: f64,
    extent: Option<(f64, f64)>,
    fog: f64,
    clock: &omsi_sim::SimClock,
    humans: Option<&humans::Humans>,
    player: Option<&Player>,
    render: &omsi_render::RenderOptions,
) {
    if let Some(c) = cam {
        t.viewer = Some(
            traffic::Viewer::new(c, aspect, fog)
                .with_extent(extent)
                .with_culling(render.min_obj_size, render.max_obj_dist),
        );
    }
    t.weekday = clock.weekday();
    t.walkers = humans.map(|h| h.strollers()).unwrap_or_default();
    t.people = humans.map(|h| h.on_foot()).unwrap_or_default();
    // the player's obstacle boxes follow the streamed tiles; without a player the world's
    // own are asked
    t.occluders = player.and_then(|p| p.vehicle.collision.clone());
}

/// Posts (`[crashmode_pole]`) the vehicle knocked over this frame: laid on the ground from
/// their foot in the direction they were hit.
pub(crate) fn lay_down_poles(
    world: &World,
    renderer: &Renderer,
    scene: &mut Scene,
    vehicle: &mut omsi_sim::VehicleInstance,
) {
    for (id, push) in std::mem::take(&mut vehicle.knocked_now) {
        if let Some(pos) = world.lay_down_pole(renderer, scene, id, push) {
            log::info!("knocked over post {id} at ({:.1}, {:.1})", pos.x, pos.y);
        }
    }
}
