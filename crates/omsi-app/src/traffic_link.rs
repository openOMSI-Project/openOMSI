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
    omsi_sim::ai_traffic::light_paths::box_outline(v.position, v.heading, bb, speed)
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
        out.extend(rear_outlines(v, speed).map(|(k, b)| (base + 1 + k as u32, b)));
    };
    if let Some(p) = player {
        add(&p.vehicle, 0xFFFF_0000, false);
    }
    for (i, q) in placed.iter().enumerate() {
        add(&q.vehicle, 0xFFFE_0000 - (i as u32) * 16, true);
    }
    out
}

/// The boxes of a vehicle's rear sections (an articulated bus's, a coupled trailer's), each
/// with its place in the train, moving at `speed` (m/s).
fn rear_outlines(
    v: &omsi_sim::VehicleInstance,
    speed: f32,
) -> impl Iterator<Item = (usize, traffic::PlayerBox)> + '_ {
    v.trailers.iter().enumerate().filter_map(move |(k, t)| {
        let bb = t.ty.def.bounding_box?;
        Some((
            k,
            omsi_sim::ai_traffic::light_paths::box_outline(t.position, t.heading, bb, speed),
        ))
    })
}

/// The LAN players' buses as obstacles for the AI traffic (their speed as last sent).
pub(crate) fn lan_outlines(game: &lan::LanGame) -> Vec<(u32, traffic::PlayerBox)> {
    game.remotes
        .iter()
        .flat_map(|(id, r)| remote_outlines(*id, r.vehicle(), r.last.speed_kmh / 3.6))
        .collect()
}

/// The first id of a LAN player's rear sections. The traffic knew only the front section of
/// another player's bus: a car following it kept its gap to the front and drove into the
/// rear section, and one turning or merging in behind the front took the back half of the
/// bus for free road - on a dedicated server, where every bus is a LAN player's, every
/// articulated bus was half invisible. The rear sections' ids start at 0xFFF0_0000, below
/// the own player's and the placed vehicles': the traffic puts only the ids under that onto
/// the lanes for the right of way (the front section stands for the bus there).
fn remote_rear_base(id: u32) -> u32 {
    0xFFF0_0000 + (id & 0x7FFF) * 16
}

/// One LAN player's bus for the traffic: its own box under the player's id, and the rear
/// sections of an articulated bus or a trailer (where the player's game has them) as the
/// own bus's are given (`own_outlines`).
fn remote_outlines(
    id: u32,
    v: &omsi_sim::VehicleInstance,
    speed: f32,
) -> Vec<(u32, traffic::PlayerBox)> {
    let rear_base = remote_rear_base(id);
    std::iter::once((id, vehicle_outline(v, speed)))
        .chain(
            rear_outlines(v, speed)
                .take(15)
                .map(|(k, b)| (rear_base + 1 + k as u32, b)),
        )
        .collect()
}

/// The indicators of the vehicles of `lan_outlines` and `own_outlines` by the same ids (a
/// rear section shows its towing vehicle's): a light path marked as a turn asks its light
/// for whoever stands on it indicating that way.
pub(crate) fn outline_indicators(game: &lan::LanGame, player: Option<&Player>, placed: &[Player]) -> hashbrown::HashMap<u32, u8> {
    let mut out: hashbrown::HashMap<u32, u8> = hashbrown::HashMap::new();
    for (id, r) in &game.remotes {
        out.insert(*id, r.last.blinker);
        // (their rear sections under the ids `remote_outlines` gives them)
        for k in 0..r.vehicle().trailers.len().min(15) {
            out.insert(remote_rear_base(*id) + 1 + k as u32, r.last.blinker);
        }
    }
    let mut add = |v: &omsi_sim::VehicleInstance, base: u32| {
        let indicator = lan::indicator(v);
        out.insert(base, indicator);
        for k in 0..v.trailers.len() {
            out.insert(base + 1 + k as u32, indicator);
        }
    };
    if let Some(p) = player {
        add(&p.vehicle, 0xFFFF_0000);
    }
    for (i, q) in placed.iter().enumerate() {
        add(&q.vehicle, 0xFFFE_0000 - (i as u32) * 16);
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
            traffic::Viewer::from_camera(c.position, c.forward().as_dvec3(), c.fov_deg, c.far, aspect, fog)
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
#[cfg(test)]
mod tests {
    use super::*;

    fn section(bounding_box: [f32; 6]) -> Arc<omsi_sim::VehicleType> {
        Arc::new(omsi_sim::VehicleType {
            def: omsi_vehicle::Vehicle {
                bounding_box: Some(bounding_box),
                ..Default::default()
            },
            model: Default::default(),
            model_dir: Default::default(),
            program: Default::default(),
            meshes: Vec::new(),
            paint_schemes: Vec::new(),
            texchanges: Vec::new(),
            wheel_meshes: Vec::new(),
            suspension_axles: Vec::new(),
            missing_packs: Vec::new(),
            mesh_bounds: Vec::new(),
            mesh_boxes: Vec::new(),
        })
    }

    #[test]
    fn a_lan_players_articulated_bus_is_whole_for_the_traffic() {
        // (Gladbeck's articulated bus: a 9.8 m front section, a 7.5 m rear section)
        let mut bus = omsi_sim::VehicleInstance::new(
            section([2.52, 9.774, 2.86, 0.0, 1.779, 1.806]),
            omsi_sim::VehicleHost::new(Default::default()),
        );
        bus.attach_trailer(section([2.52, 7.5, 2.84, 0.0, 0.276, 1.802]));
        // heading north at (100, 200), the rear section's origin 8 m behind, as the
        // player's game sends it
        bus.position = DVec3::new(100.0, 200.0, 0.0);
        bus.heading = 0.0;
        let coupling = DVec3::new(100.0, 196.085, 0.0);
        bus.trailers[0].set_remote_pose(DVec3::new(100.0, 192.0, 0.0), 0.0, coupling);
        let out = remote_outlines(7, &bus, 8.0);
        assert_eq!(out.len(), 2, "the front and the rear section");
        let (front_id, front) = out[0];
        assert_eq!(front_id, 7);
        assert!((front.0.y - 201.779).abs() < 1e-3 && (front.2 - 4.887).abs() < 1e-3);
        let (rear_id, rear) = out[1];
        // (not put onto the lanes for the right of way: the front stands for the bus there)
        assert!(rear_id >= 0xFFF0_0000, "{rear_id:#x}");
        assert!(
            (rear.0.y - 192.276).abs() < 1e-3 && (rear.0.x - 100.0).abs() < 1e-3,
            "{:?}",
            rear.0
        );
        assert!((rear.2 - 3.75).abs() < 1e-3 && rear.4 == 8.0);
        // another player's rear section has an id of its own
        let other = remote_outlines(8, &bus, 8.0);
        assert_ne!(other[1].0, rear_id);
    }
}
