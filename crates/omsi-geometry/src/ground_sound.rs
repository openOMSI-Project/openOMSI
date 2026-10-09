//! What a tile sounds like to the ambience: which surface the bare ground is where (the
//! map's painted ground layers over its base texture), the trees standing on it, and how
//! built-up it is.

use crate::{TileSurface, tile_size};

/// The ground's surfaces and the trees of one tile.
#[derive(Debug, Clone, Default)]
pub struct GroundSound {
    /// Cells per tile edge, and per cell (row by row from the tile's south edge, as the
    /// terrain's y runs) OMSI's `[surface]` id of the ground layer that shows there.
    pub size: usize,
    pub ids: Vec<u8>,
    /// The trees: tile-local x, y and their height (m).
    pub trees: Vec<[f32; 3]>,
    /// The objects placed on the tile (houses, signs, lamps …): how built-up it is.
    pub objects: u32,
}

impl GroundSound {
    /// From the ground's painted layers: `base` is the first `[groundtex]`'s surface id, each
    /// layer a mask (alpha, `width` × `height`, first row south) with its surface id. The
    /// layer painted last over a cell shows (OMSI draws them in order).
    pub fn from_layers(size: usize, base: u8, layers: &[(u8, usize, usize, &[u8])]) -> GroundSound {
        let size = size.max(1);
        let mut ids = vec![base; size * size];
        for (id, w, h, rgba) in layers {
            if *w == 0 || *h == 0 || rgba.len() < w * h * 4 {
                continue;
            }
            for j in 0..size {
                for i in 0..size {
                    let x = ((i as f32 + 0.5) / size as f32 * *w as f32) as usize;
                    let y = ((j as f32 + 0.5) / size as f32 * *h as f32) as usize;
                    let a = rgba[(y.min(h - 1) * w + x.min(w - 1)) * 4 + 3];
                    if a >= 128 {
                        ids[j * size + i] = *id;
                    }
                }
            }
        }
        GroundSound { size, ids, trees: Vec::new(), objects: 0 }
    }

    /// The ground's surface id at tile-local (x, y); asphalt where nothing is known.
    pub fn ground_at(&self, x: f32, y: f32) -> u8 {
        if self.size == 0 || self.ids.len() < self.size * self.size {
            return 0;
        }
        let t = tile_size() as f32;
        let i = ((x / t * self.size as f32).floor().max(0.0) as usize).min(self.size - 1);
        let j = ((y / t * self.size as f32).floor().max(0.0) as usize).min(self.size - 1);
        self.ids[j * self.size + i]
    }
}

impl TileSurface {
    /// What a tyre at tile-local (x, y), with its contact at height `z`, rolls on: of the
    /// highest road, deck or surface object face a little above the contact or below it and
    /// the bare ground (height `terrain`), the one the contact is on - the nearer, a face
    /// winning a near tie. (The ground often runs a few centimetres over a road whose hole
    /// cuts it out of the picture, and the wheel stands on the road there.)
    pub fn surface_under(&self, x: f32, y: f32, z: f32, terrain: Option<f32>) -> u8 {
        let face = self.drive.surface_at(x, y, z + 0.3);
        match (face, terrain) {
            (Some((fz, id)), Some(tz)) if (z - fz).abs() <= (z - tz).abs() + 0.15 => id,
            (Some((_, id)), None) => id,
            (_, Some(_)) => self.sound.ground_at(x, y),
            (None, None) => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn the_last_painted_layer_shows() {
        // a 2x2 mask: layer 2 (cobbles) painted on the west half, layer 5 (gravel) on the
        // south-west cell over it
        let w = |a: [u8; 4]| a.iter().flat_map(|v| [0, 0, 0, *v]).collect::<Vec<u8>>();
        let cobbles = w([255, 0, 255, 0]);
        let gravel = w([255, 0, 0, 0]);
        let g = GroundSound::from_layers(2, 4, &[(2, 2, 2, &cobbles), (5, 2, 2, &gravel)]);
        let t = tile_size() as f32;
        assert_eq!(g.ground_at(0.1 * t, 0.1 * t), 5);
        assert_eq!(g.ground_at(0.1 * t, 0.9 * t), 2);
        assert_eq!(g.ground_at(0.9 * t, 0.9 * t), 4, "the base layer (grass)");
        assert_eq!(GroundSound::default().ground_at(1.0, 1.0), 0);
    }

    #[test]
    fn a_road_face_wins_over_the_ground_below_it() {
        let mut ts = TileSurface::new(32);
        ts.sound = GroundSound { size: 1, ids: vec![4], ..Default::default() };
        // a cobbled road plate 10 x 10 m at 0.2 m
        let a = Vec3::new(10.0, 10.0, 0.2);
        let (b, c, d) = (Vec3::new(20.0, 10.0, 0.2), Vec3::new(20.0, 20.0, 0.2), Vec3::new(10.0, 20.0, 0.2));
        for tri in [[a, b, c], [a, c, d]] {
            assert!(ts.drive.push_kind(tri, false));
            ts.drive.tag_last(2);
        }
        ts.finish();
        assert_eq!(ts.surface_under(15.0, 15.0, 0.2, Some(0.0)), 2, "on the road");
        assert_eq!(ts.surface_under(30.0, 30.0, 0.0, Some(0.0)), 4, "on the grass beside it");
        // the bare ground over the plate (a buried face): the ground's
        assert_eq!(ts.surface_under(15.0, 15.0, 1.0, Some(1.0)), 4);
        // the ground a little over the road it is cut away for: the road's
        assert_eq!(ts.surface_under(15.0, 15.0, 0.2, Some(0.3)), 2);
    }
}
