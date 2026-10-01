//! Maps: `global.cfg`, `tile_x_y.map`, terrain, calendar, AI lists, chrono events.
//!
//! A tile is 300 m × 300 m; the terrain grid has 61 × 61 samples (5 m spacing).

pub mod ailists;
pub mod calendar;
pub mod global;
pub mod terrain;
pub mod tile;

pub use ailists::{active_chrono_dirs, chrono_deactivated_lines, date_code, typgroup_entry_valid, AiGroup, AiLists, DepotEntry};
pub use calendar::{Calendar, Holiday, HolidayRange, TimeZone};
pub use global::{EntryPoint, GlobalCfg, GroundTex, MapTileRef};
pub use terrain::Terrain;
pub use tile::{MapObject, MapRule, MapSpline, SplineAttachment, Tile};

/// Tile edge length in metres.
/// Tile edge of plain maps (m).
pub const TILE_SIZE: f64 = 300.0;
/// Tile edge of `[worldcoordinates]` maps: 1/300 degree, 371.9 m on both axes (measured on
/// 271 cross-tile spline links of Berlin-Spandau, spread 0.14 m).
pub const WORLD_TILE_SIZE: f64 = 371.9;

static TILE_SIZE_BITS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0x4072_C000_0000_0000); // 300.0

/// Tile edge of the loaded map (set once by the loader from `[worldcoordinates]`).
pub fn tile_size() -> f64 {
    f64::from_bits(TILE_SIZE_BITS.load(std::sync::atomic::Ordering::Relaxed))
}

pub fn set_tile_size(size: f64) {
    TILE_SIZE_BITS.store(size.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

static WORLD_COORDS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `[worldcoordinates]`: a tile is a Web Mercator tile of zoom 16 (the original,
/// the original), its row `ty` counted north from the equator. The latitude of
/// its lower edge is `2 atan(exp(2 pi ty / 2^16)) - 90` degrees, and the tile is as wide as
/// the earth's circumference times the cosine of that latitude over 2^16: the lower edge
/// `world_row_width(ty)`, the upper one `world_row_width(ty + 1)`, as high as the upper
/// edge is wide. Berlin-Spandau's rows are 371.6..372.2 m.
pub fn world_row_width(ty: i32) -> f64 {
    let lat = 2.0 * (std::f64::consts::TAU * ty as f64 / 65536.0).exp().atan() - std::f64::consts::FRAC_PI_2;
    40_075_016.69 * lat.cos() / 65536.0
}

/// The tile size a `[worldcoordinates]` map is laid out with: its median row's width. The
/// original keeps each tile's own size and places the tiles around the camera one by one;
/// a single grid needs one size, and each tile's contents are scaled onto it (see
/// [`world_tile_scale`]) - before, every such map took Spandau's 371.9 m, and its roads
/// broke apart at the tile borders (a third of a metre on Spandau's outer rows, tens of
/// metres on a map at another latitude).
pub fn world_tile_size(rows: impl Iterator<Item = i32>) -> f64 {
    let mut rows: Vec<i32> = rows.collect();
    if rows.is_empty() {
        return WORLD_TILE_SIZE;
    }
    // (the median: a long corridor of a few tiles must not pull the grid off the town)
    rows.sort_unstable();
    let r = rows[rows.len() / 2];
    (world_row_width(r) + world_row_width(r + 1)) / 2.0
}

/// Whether the loaded map has `[worldcoordinates]` (set with its tile size).
pub fn set_world_coordinates(on: bool) {
    WORLD_COORDS.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn world_coordinates() -> bool {
    WORLD_COORDS.load(std::sync::atomic::Ordering::Relaxed)
}

/// How much a tile of row `ty` is scaled onto the map's grid across (x) and along (y): its
/// width at mid-height and its height against the grid's tile size (1 for a plain map).
pub fn world_tile_scale(ty: i32) -> (f64, f64) {
    if !world_coordinates() {
        return (1.0, 1.0);
    }
    let s = tile_size();
    let (w0, w1) = (world_row_width(ty), world_row_width(ty + 1));
    (s / ((w0 + w1) / 2.0), s / w1)
}

/// Set the tile grid (size and `[worldcoordinates]`) for `global`. Everything that turns a
/// tile index and a position in the tile into world metres needs it first - the loader of a
/// situation too, which runs before the world (with 300 m a Spandau situation put the bus
/// kilometres off the map). `OMSI_OLD_WORLD_GRID`: the one size of 371.9 m for comparison.
pub fn configure_grid(global: &GlobalCfg) {
    let old_grid = std::env::var_os("OMSI_OLD_WORLD_GRID").is_some();
    set_tile_size(if global.world_coordinates && !old_grid {
        world_tile_size(global.tiles.iter().map(|t| t.y))
    } else if global.world_coordinates {
        WORLD_TILE_SIZE
    } else {
        TILE_SIZE
    });
    set_world_coordinates(global.world_coordinates && !old_grid);
}

/// World metres (x east, y north) of a point given in tile (tx, ty)'s own frame, as the
/// tile's contents are placed (scaled onto the grid, see [`world_tile_scale`]).
pub fn tile_local_to_world(tx: i32, ty: i32, local_x: f64, local_y: f64) -> (f64, f64) {
    let ts = tile_size();
    let (kx, ky) = world_tile_scale(ty);
    (tx as f64 * ts + local_x * kx, ty as f64 * ts + local_y * ky)
}

/// The tile a world point lies in and its position in that tile's own frame (the inverse
/// of [`tile_local_to_world`]).
pub fn world_to_tile_local(x: f64, y: f64) -> ((i32, i32), (f64, f64)) {
    let ts = tile_size();
    let (tx, ty) = ((x / ts).floor() as i32, (y / ts).floor() as i32);
    let (kx, ky) = world_tile_scale(ty);
    ((tx, ty), ((x - tx as f64 * ts) / kx, (y - ty as f64 * ts) / ky))
}

/// Latitude and longitude (degrees) of a point given in tile (tx, ty)'s own frame on a
/// `[worldcoordinates]` map: the tile is the Web Mercator tile of zoom 16 in column
/// `tx + 32768` and row `ty` counted north from the equator (see [`world_row_width`]), its
/// local metres run across at the tile's mid-height width and up at its upper edge's.
pub fn tile_local_to_lat_lon(tx: i32, ty: i32, local_x: f64, local_y: f64) -> (f64, f64) {
    let (w0, w1) = (world_row_width(ty), world_row_width(ty + 1));
    let lon = ((tx + 32768) as f64 + local_x / ((w0 + w1) / 2.0)) / 65536.0 * 360.0 - 180.0;
    let row = ty as f64 + local_y / w1;
    let lat = 2.0 * (std::f64::consts::TAU * row / 65536.0).exp().atan() - std::f64::consts::FRAC_PI_2;
    (lat.to_degrees(), lon)
}

/// Latitude and longitude (degrees) of a world point of the loaded map, for a web map of
/// the server's players; `None` on a map without `[worldcoordinates]`, which lies nowhere
/// on the earth.
pub fn world_to_lat_lon(x: f64, y: f64) -> Option<(f64, f64)> {
    if !world_coordinates() {
        return None;
    }
    let ((tx, ty), (lx, ly)) = world_to_tile_local(x, y);
    Some(tile_local_to_lat_lon(tx, ty, lx, ly))
}

/// The tile index in a `tile_<x>_<y>.map` file name.
pub fn tile_index_of(name: &str) -> Option<(i32, i32)> {
    let stem = name.trim().to_ascii_lowercase();
    let stem = stem.strip_prefix("tile_")?;
    let stem = stem.split('.').next()?;
    let (x, y) = stem.split_once('_')?;
    Some((x.parse().ok()?, y.parse().ok()?))
}

#[cfg(test)]
mod world_tests {
    use super::*;
    #[test]
    fn spandau_rows() {
        // the size openOMSI had measured on Spandau's cross-tile links, and the change
        // of 2.8 cm a row the joints show
        assert!((world_row_width(11281) - 371.9).abs() < 0.05);
        assert!(((world_row_width(11281) - world_row_width(11282)) - 0.0283).abs() < 0.002);
        assert_eq!(tile_index_of("tile_2405_11280.map"), Some((2405, 11280)));
        assert_eq!(tile_index_of("Tile_-2_3.map"), Some((-2, 3)));
    }

    #[test]
    fn spandau_on_the_earth() {
        // the bus stop "U Rathaus Spandau" of tile_2402_11279.map (object 67032): on the
        // Carl-Schurz-Strasse, a few metres from where OpenStreetMap has the stop
        let (lat, lon) = tile_local_to_lat_lon(2402, 11279, 342.786665653207, 276.13607466369);
        assert!((lat - 52.53541).abs() < 1e-4, "{lat}");
        assert!((lon - 13.19964).abs() < 1e-4, "{lon}");
    }
}
/// Terrain samples per tile edge.
pub const TERRAIN_SAMPLES: usize = 61;
