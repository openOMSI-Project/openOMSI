// Snow on the carriageways where the device has no texture unit left for its field (see
// `sixteen_texture_units`): the weather's on/off "snow on road", as before.

fn road_snow_dynamic() -> bool {
    return false;
}

fn vehicle_snow(code: f32, local: vec3<f32>) -> f32 {
    return 0.0;
}

fn road_snow(world: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(0.0);
}
