//! `map.*`: the map - its tiles, stops, objects, ground and lanes - and moving the player's
//! bus on it.

use super::{def, multi, nums, o, p, rec};
use crate::api::{ApiError, ApiFn, Value};

const NEW: &str = crate::api::VERSION;
/// Most objects `objects_near` gives.
const MAX_OBJECTS: usize = 2000;

fn place(p: [f64; 4]) -> Vec<(&'static str, Value)> {
    vec![("x", p[0].into()), ("y", p[1].into()), ("z", p[2].into()), ("heading", p[3].into())]
}

pub static FNS: &[ApiFn] = &[
    def!("map.name", "map", [], "string or nil", "The map's name (its folder's).", NEW, None, false, |c, a| c.io().map_info().map(|m| m.name)),
    def!("map.info", "map", [], "table or nil", "`{name, friendly_name, path, left_hand_traffic, tile_size}`: the map's names, its global.cfg and the side it drives on.", NEW, None, false, |c, a| {
        c.io().map_info().map(|m| rec(vec![("name", m.name.into()), ("friendly_name", m.friendly_name.into()), ("path", m.path.into()), ("left_hand_traffic", m.left_hand_traffic.into()), ("tile_size", m.tile_size.into())]))
    }),
    def!("map.tiles", "map", [], "list of tables", "The map's tiles: `{x, y, file, loaded}` (numbered as global.cfg's `[map]` list).", NEW, None, false, |c, a| {
        Value::List(c.io().map_tiles().into_iter().map(|t| rec(vec![("x", t.x.into()), ("y", t.y.into()), ("file", t.file.into()), ("loaded", t.loaded.into())])).collect())
    }),
    def!("map.tile_at", "map", [p("x", "number"), p("y", "number")], "tile_x, tile_y, local_x, local_y", "The tile of a map point and the metres in it (x east, y north).", NEW, None, true, |c, a| {
        let (x, y) = (a.num(0)?, a.num(1)?);
        Ok::<_, ApiError>(multi(c.io().map_tile_at(x, y).map(|((tx, ty), (lx, ly))| [Value::Int(tx as i64), Value::Int(ty as i64), Value::Num(lx), Value::Num(ly)])))
    }),
    def!("map.to_world", "map", [p("tile_x", "integer"), p("tile_y", "integer"), p("local_x", "number"), p("local_y", "number")], "x, y", "A place given by its tile and the metres in it, in map coordinates.", NEW, None, true, |c, a| {
        let (tx, ty, lx, ly) = (a.int(0)? as i32, a.int(1)? as i32, a.num(2)?, a.num(3)?);
        Ok::<_, ApiError>(multi(c.io().map_from_tile(tx, ty, lx, ly).map(|(x, y)| [Value::Num(x), Value::Num(y)])))
    }),
    def!("map.stops", "map", [], "list of tables", "The bus stops of the tiles loaded (around the camera): `{id, name, x, y, z, heading}`.", NEW, None, false, |c, a| {
        Value::List(c.io().map_stops().into_iter().map(|s| {
            let mut r = vec![("id", Value::Int(s.id)), ("name", s.name.trim().into())];
            r.extend(place(s.pos));
            rec(r)
        }).collect())
    }),
    def!("map.object", "map", [p("id", "integer")], "x, y, z, heading", "Where a map object is, by its id (a stop's id of the timetable, say); nothing when the game has not read its tile.", NEW, None, true, |c, a| {
        let id = a.int(0)?;
        Ok::<_, ApiError>(nums(c.io().map_object(id)))
    }),
    def!("map.objects_near", "map", [p("x", "number"), p("y", "number"), o("radius", "number")], "list of tables", "The map objects within `radius` m (default 50, at most 2000 of them, nearest first): `{id, x, y, z, heading, distance}`.", NEW, None, false, |c, a| {
        let (x, y, r) = (a.num(0)?, a.num(1)?, a.opt_num(2)?.unwrap_or(50.0).clamp(0.0, 5000.0));
        let mut list: Vec<(f64, i64, [f64; 4])> = c.io().map_objects_near(x, y, r).into_iter().map(|(id, p)| ((p[0] - x).hypot(p[1] - y), id, p)).collect();
        list.sort_by(|a, b| a.0.total_cmp(&b.0));
        list.truncate(MAX_OBJECTS);
        Ok::<_, ApiError>(Value::List(list.into_iter().map(|(d, id, p)| {
            let mut r = vec![("id", Value::Int(id))];
            r.extend(place(p));
            r.push(("distance", d.into()));
            rec(r)
        }).collect()))
    }),
    def!("map.ground", "map", [p("x", "number"), p("y", "number")], "number or nil", "The height of the ground (the road where there is one, else the terrain) at a map point, where its tile is loaded.", NEW, None, false, |c, a| {
        let (x, y) = (a.num(0)?, a.num(1)?);
        Ok::<_, ApiError>(c.io().map_ground(x, y).0)
    }),
    def!("map.terrain", "map", [p("x", "number"), p("y", "number")], "number or nil", "The terrain's height alone at a map point.", NEW, None, false, |c, a| {
        let (x, y) = (a.num(0)?, a.num(1)?);
        Ok::<_, ApiError>(c.io().map_ground(x, y).1)
    }),
    def!("map.lane", "map", [p("x", "number"), p("y", "number")], "table or nil", "The traffic lane nearest to a map point: `{index, distance, speed_limit, name, traffic_light}`.", NEW, None, false, |c, a| {
        let (x, y) = (a.num(0)?, a.num(1)?);
        Ok::<_, ApiError>(c.io().map_lane(x, y).map(|l| rec(vec![("index", l.index.into()), ("distance", l.distance.into()), ("speed_limit", Value::from(l.speed_limit_kmh)), ("name", l.name.into()), ("traffic_light", l.has_light.into())])))
    }),
    def!("map.speed_limit", "map", [], "number or nil", "The speed limit (km/h) of the lane the player's bus drives on (`nil` off the lanes).", NEW, None, false, |c, a| {
        let io = c.io();
        io.position().and_then(|p| io.map_lane(p[0], p[1])).filter(|l| l.distance < 6.0).map(|l| l.speed_limit_kmh as f64)
    }),
    def!("map.entrypoints", "map", [], "list of tables", "The map's start points: `{index, name, x, y, z, heading}` (`index` from 1; the place where the game has read its tile).", NEW, None, false, |c, a| {
        Value::List(c.io().map_entrypoints().into_iter().map(|e| {
            let mut r = vec![("index", Value::from(e.index + 1)), ("name", e.name.into())];
            if let Some(p) = e.pos {
                r.extend(place(p));
            }
            rec(r)
        }).collect())
    }),
    def!("map.teleport", "map", [p("x", "number"), p("y", "number"), o("z", "number"), o("heading", "number")], "boolean", "Moves the player's bus to a map point (z none: onto the highest ground there) facing `heading` (default north), as the game menu's move does; the `service` event says `teleport`.", NEW, WorldWrite, false, |c, a| {
        let (x, y, z, h) = (a.num(0)?, a.num(1)?, a.opt_num(2)?.unwrap_or(0.0), a.opt_num(3)?.unwrap_or(0.0));
        Ok::<_, ApiError>(c.io().map_teleport([x, y, z], h))
    }),
    def!("map.teleport_to", "map", [p("index", "integer")], "boolean", "Moves the player's bus to start point `index` (from 1, as `map.entrypoints` lists them).", NEW, WorldWrite, false, |c, a| {
        let i = a.int(0)?;
        Ok::<_, ApiError>(i >= 1 && c.io().map_teleport_entry(i as usize - 1))
    }),
    def!("map.place_on_road", "map", [p("x", "number"), p("y", "number")], "boolean", "Puts the player's bus on the street nearest to a map point (within 300 m), along it.", NEW, WorldWrite, false, |c, a| {
        let (x, y) = (a.num(0)?, a.num(1)?);
        Ok::<_, ApiError>(c.io().map_place_on_road(x, y))
    }),
    def!("map.tile", "map", [], "tile_x, tile_y", "The tile the player's bus is on (nothing on foot).", NEW, None, true, |c, a| {
        let io = c.io();
        multi(io.info_value("tile_x").zip(io.info_value("tile_y")).map(|(x, y)| [Value::from(x), Value::from(y)]))
    }),
];
