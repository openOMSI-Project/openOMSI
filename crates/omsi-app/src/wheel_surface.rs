//! What each wheel of the player's bus rolls on, told to its scripts as Omsi.exe does
//! (`Axle_SurfaceID_<axle>_L/_R`: the `[surface]` id of the road's or the ground's texture
//! under the wheel, see `World::surface_under`).

/// Set the bus's `Axle_SurfaceID_` variables for this frame.
pub(crate) fn tell_scripts(w: &crate::scene::World, v: &mut omsi_sim::VehicleInstance) {
    let Some(rb) = v.rigid.as_ref() else { return };
    let rot = v.body_rotation();
    let found: Vec<(String, u8)> = rb
        .wheels
        .iter()
        .enumerate()
        .map(|(k, wh)| {
            let hub = v.position + rot.transform_vector3(wh.attach).as_dvec3();
            let contact = glam::DVec3::new(hub.x, hub.y, if wh.ground_seen { wh.ground_z } else { hub.z - wh.radius as f64 });
            let axle = rb.wheel_axle.get(k).copied().unwrap_or(0);
            let side = if wh.attach.x < 0.0 { "L" } else { "R" };
            (format!("Axle_SurfaceID_{axle}_{side}"), w.surface_under(contact).unwrap_or(0))
        })
        .collect();
    for (name, id) in found {
        v.set_var(&name, id as f32);
    }
}
