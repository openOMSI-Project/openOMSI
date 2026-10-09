//! What the world tells a vehicle's scripts of the ground: the surface under a tyre.

use super::*;

impl World {
    /// OMSI's `[surface]` id under a tyre whose contact is at `at` (see
    /// `TileSurface::surface_under`); `None` where no tile is loaded.
    pub fn surface_under(&self, at: DVec3) -> Option<u8> {
        let key = tile_key(at.x, at.y);
        let (lx, ly) = ((at.x - key.0 as f64 * tile_size()) as f32, (at.y - key.1 as f64 * tile_size()) as f32);
        let terrain = self.terrains.read().get(&key).map(|t| t.sample(lx, ly));
        let surfaces = self.surfaces.read();
        let s = surfaces.get(&key)?;
        Some(s.surface_under(lx, ly, at.z as f32, terrain))
    }
}
