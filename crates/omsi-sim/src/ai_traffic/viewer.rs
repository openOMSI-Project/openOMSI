//! Where the player looks from: what the population may not be seen doing.

use super::*;

/// Where the player looks from, for putting cars on the road and taking them off only
/// where nobody sees it happen.
#[derive(Debug, Clone, Copy)]
pub struct Viewer {
    pub pos: DVec3,
    pub forward: DVec3,
    /// Tangents of half the horizontal and vertical field of view.
    pub tan_x: f64,
    pub tan_y: f64,
    /// Beyond this distance nothing shows (fog, or a car smaller than a pixel) (m).
    pub range: f64,
    /// What the renderer leaves out (see `omsi_render::RenderOptions`): objects smaller on
    /// the screen than `min_size` (the original's measure), and farther than `max_dist`
    /// (0 = no limit); `fov` is the vertical field of view (radians).
    pub min_size: f64,
    pub max_dist: f64,
    pub fov: f64,
}

/// A car further away than this is below two pixels on a 900-line screen (m).
pub const VISIBLE_RANGE: f64 = 900.0;

/// Within this distance of the camera no car appears or vanishes, seen or not: the mirrors
/// and a turn of the head see what is near (see [`Traffic::may_appear`]).
pub const NEVER_VANISH_WITHIN: f64 = 150.0;
/// Within this distance a vehicle appears or vanishes only behind something, wherever the
/// player looks (see `Traffic::hidden`).
pub const NEAR_HIDE: f64 = 350.0;

/// Within this distance of the camera an AI vehicle is animated and drawn even out of the
/// view (m): the mirrors look behind, and a car beside the view throws its shadow into it.
pub const UNSEEN_NEAR: f64 = 80.0;

/// How far above where a LAN player's vehicle stands their eye is taken to be (m): a
/// driver's in a bus.
pub const LAN_EYE_HEIGHT: f64 = 2.5;

impl Viewer {
    /// A LAN player's view as the host can tell it (host): from the driver's seat of their
    /// vehicle at `pos`, facing its `heading` (degrees clockwise from north), as wide as the
    /// view the population takes when it has no camera. Where they really look the host
    /// does not know; within `NEAR_HIDE` that does not matter (see [`Viewer::hides`]).
    pub fn lan_player(pos: DVec3, heading: f64) -> Viewer {
        let h = heading.to_radians();
        Viewer {
            pos: pos + DVec3::Z * LAN_EYE_HEIGHT,
            forward: DVec3::new(h.sin(), h.cos(), 0.0),
            tan_x: 1.2,
            tan_y: 0.6,
            range: VISIBLE_RANGE,
            min_size: 0.0,
            max_dist: 0.0,
            fov: 1.0,
        }
    }

    /// Could nobody looking from here see a vehicle of radius `r` at `p` appear or vanish?
    /// Never close by; within `NEAR_HIDE` only behind something (`occluded`: buildings or
    /// the ground hide it from here), wherever the camera looks; further off beyond what is
    /// drawn, out of the picture, or behind something.
    pub fn hides(&self, p: DVec3, r: f64, occluded: impl FnOnce() -> bool) -> bool {
        let d = (p - self.pos).length();
        if d < NEVER_VANISH_WITHIN {
            return false;
        }
        if !self.draws(d, r) {
            return true;
        }
        if d < NEAR_HIDE {
            return occluded();
        }
        !self.frames(p, r) || occluded()
    }

    /// The view of a camera at `position` looking along `forward`, with a vertical field of
    /// view of `fov_deg` and its far plane at `far`, a picture `aspect` wide to high, in fog
    /// that hides everything beyond `fog_range`.
    pub fn from_camera(position: DVec3, forward: DVec3, fov_deg: f32, far: f32, aspect: f64, fog_range: f64) -> Viewer {
        let tan_y = (fov_deg as f64 * 0.5).to_radians().tan();
        Viewer {
            pos: position,
            forward: forward.normalize_or_zero(),
            tan_x: tan_y * aspect.max(0.2),
            tan_y,
            range: fog_range.min(VISIBLE_RANGE).min(far as f64),
            min_size: 0.0,
            max_dist: 0.0,
            fov: (fov_deg as f64).to_radians(),
        }
    }

    /// A wider picture than the camera's own (a triple screen's side panels): the tangents
    /// of its half-angles, horizontal and vertical. The size limit stays the camera's.
    pub fn with_extent(mut self, extent: Option<(f64, f64)>) -> Viewer {
        if let Some((tan_x, tan_y)) = extent {
            self.tan_x = self.tan_x.max(tan_x);
            self.tan_y = self.tan_y.max(tan_y);
        }
        self
    }

    /// The renderer's culling as well (`RenderOptions::min_obj_size`, `max_obj_dist`).
    pub fn with_culling(mut self, min_size: f32, max_dist: f32) -> Viewer {
        self.min_size = min_size.max(0.0) as f64;
        self.max_dist = max_dist.max(0.0) as f64;
        self
    }

    /// Would the renderer draw an object of radius `r` this far away at all? Beyond that a
    /// car can come and go in plain view without anybody seeing it happen.
    pub fn draws(&self, dist: f64, r: f64) -> bool {
        // (the renderer measures a vehicle by a sphere about its origin, which may stand
        // well off its middle: half as much again, and a metre, to be sure)
        let r = r * 1.5 + 1.0;
        if self.max_dist > 0.0 && dist > self.max_dist + r {
            return false;
        }
        self.min_size <= 0.0 || 2.0 * r / (dist.max(0.01) * self.fov.max(1e-3)) >= self.min_size
    }

    /// Does a sphere of radius `r` at `p` lie within the view frustum and range?
    pub fn frames(&self, p: DVec3, r: f64) -> bool {
        let rel = p - self.pos;
        let dist = rel.length();
        if dist <= r {
            return true;
        }
        if dist - r > self.range {
            return false;
        }
        let f = self.forward;
        let mut right = f.cross(DVec3::Z);
        if right.length() < 1e-3 {
            right = DVec3::X;
        }
        let right = right.normalize();
        let up = right.cross(f);
        let z = rel.dot(f);
        if z < -r {
            return false;
        }
        let (x, y) = (rel.dot(right), rel.dot(up));
        x.abs() <= z * self.tan_x + r * (1.0 + self.tan_x * self.tan_x).sqrt()
            && y.abs() <= z * self.tan_y + r * (1.0 + self.tan_y * self.tan_y).sqrt()
    }
}

impl TrafficSim {
    /// Could neither the player nor any LAN player (`lan_eyes`, host) see a vehicle of
    /// radius `r` at `p` appear or vanish? `occluded(v)`: buildings or the ground hide it
    /// from `v`. Asked of the host's own camera alone, a dedicated server's (where the map
    /// starts, kilometres from the players) saw nothing near them: cars came into being and
    /// vanished in plain view of every player - beside a bus at a junction in Gladbeck seven
    /// a minute appeared and five vanished within 150 m, the queue that had waited a minute
    /// at the lights all at once.
    pub fn unseen(&self, p: DVec3, r: f64, mut occluded: impl FnMut(&Viewer) -> bool) -> bool {
        self.viewer
            .map(|v| v.hides(p, r, || occluded(&v)))
            .unwrap_or(true)
            && self.lan_eyes.iter().all(|v| v.hides(p, r, || occluded(v)))
    }

    /// How far the nearest one looking (the player's camera or a LAN player's eye) is from
    /// `p`, None when nobody looks.
    pub fn nearest_eye(&self, p: DVec3) -> Option<f64> {
        self.viewer
            .iter()
            .chain(self.lan_eyes.iter())
            .map(|v| (p - v.pos).length())
            .reduce(f64::min)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera(pos: DVec3, forward: DVec3) -> Viewer {
        Viewer {
            pos,
            forward,
            tan_x: 1.2,
            tan_y: 0.6,
            range: VISIBLE_RANGE,
            min_size: 0.0,
            max_dist: 0.0,
            fov: 1.0,
        }
    }

    #[test]
    fn a_lan_player_looks_ahead_from_the_drivers_seat() {
        let v = Viewer::lan_player(DVec3::new(100.0, 200.0, 30.0), 90.0);
        assert!((v.pos - DVec3::new(100.0, 200.0, 30.0 + LAN_EYE_HEIGHT)).length() < 1e-9);
        assert!(
            (v.forward - DVec3::X).length() < 1e-9,
            "east: {:?}",
            v.forward
        );
        let north = Viewer::lan_player(DVec3::ZERO, 0.0);
        assert!((north.forward - DVec3::Y).length() < 1e-9);
    }

    #[test]
    fn a_lan_players_view_hides_only_what_they_cannot_see() {
        let v = Viewer::lan_player(DVec3::ZERO, 0.0);
        let open = || false;
        let behind_a_house = || true;
        // close by: never, behind something or not
        assert!(!v.hides(DVec3::new(0.0, 100.0, 0.0), 2.5, behind_a_house));
        assert!(!v.hides(DVec3::new(0.0, -100.0, 0.0), 2.5, behind_a_house));
        // within NEAR_HIDE, wherever they look: only behind something
        assert!(!v.hides(DVec3::new(0.0, -250.0, 0.0), 2.5, open));
        assert!(v.hides(DVec3::new(0.0, -250.0, 0.0), 2.5, behind_a_house));
        // further off: in the picture ahead only behind something, out of it or beyond the
        // range always
        assert!(!v.hides(DVec3::new(0.0, 500.0, 0.0), 2.5, open));
        assert!(v.hides(DVec3::new(0.0, 500.0, 0.0), 2.5, behind_a_house));
        assert!(v.hides(DVec3::new(0.0, -500.0, 0.0), 2.5, open));
        assert!(v.hides(DVec3::new(0.0, VISIBLE_RANGE + 50.0, 0.0), 2.5, open));
    }

    #[test]
    fn what_a_lan_player_sees_is_not_hidden_by_the_hosts_camera_far_away() {
        let random = super::super::setup::RandomTypes {
            types: Vec::new(),
            groups: Vec::new(),
            group_curves: false,
            group_uvg: Vec::new(),
            uvg_defaults: Vec::new(),
        };
        let mut t = TrafficSim::assemble(
            Path::new("."),
            Network::default(),
            random,
            Vec::new(),
            HashMap::new(),
            (Vec::new(), Vec::new()),
            Vec::new(),
            (1.0, 0),
            0,
        );
        // a dedicated server's camera, where the map starts, looking away
        t.viewer = Some(camera(
            DVec3::new(-2000.0, 0.0, 2.0),
            DVec3::new(-1.0, 0.0, 0.0),
        ));
        let at_the_lights = DVec3::new(0.0, 80.0, 0.0);
        let down_the_road = DVec3::new(0.0, 250.0, 0.0);
        let open = |_: &Viewer| false;
        // nobody else: the host's camera alone decides
        assert!(t.unseen(at_the_lights, 2.5, open));
        assert_eq!(
            t.nearest_eye(at_the_lights).map(|d| d.round()),
            Some(2002.0)
        );
        // a player's bus waits at the lights: what they see stays, behind a house it may go
        t.lan_eyes = vec![Viewer::lan_player(DVec3::ZERO, 0.0)];
        assert!(!t.unseen(at_the_lights, 2.5, open));
        assert!(!t.unseen(down_the_road, 2.5, open));
        let behind_a_house_from_the_bus = |v: &Viewer| v.pos.truncate().length() < 1.0;
        assert!(t.unseen(down_the_road, 2.5, behind_a_house_from_the_bus));
        assert_eq!(t.nearest_eye(at_the_lights).map(|d| d.round()), Some(80.0));
        // and the host's own camera still counts where it looks
        t.viewer = Some(camera(DVec3::new(0.0, -500.0, 2.0), DVec3::Y));
        assert!(!t.unseen(down_the_road, 2.5, behind_a_house_from_the_bus));
    }
}
