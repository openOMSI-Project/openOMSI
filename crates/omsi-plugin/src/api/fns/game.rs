//! `game.*`, `lan.*` and the panels' further functions (`ui.*` besides the first ones): the
//! game's settings and state, screenshots and notifications, the LAN session, and changing
//! panels in place.

use super::{def, multi, o, p, rec};
use crate::api::{ApiError, ApiFn, Ctx, Value};

const NEW: &str = crate::api::VERSION;
/// Longest `lan.send` text: the session's command messages carry 160 characters, and the
/// plugin's name goes with it.
pub const LAN_MAX: usize = 120;

fn pairs(v: Vec<(&'static str, crate::InfoValue)>) -> Value {
    Value::Map(v.into_iter().map(|(k, v)| (k.to_string(), v.into())).collect())
}

fn panel_id(c: &mut Ctx<'_>, id: &str) -> Result<u64, ApiError> {
    let s = c.state();
    let owner = s.owner;
    if s.ui().get(owner, id).is_none() {
        return Err(ApiError(format!("no panel \"{id}\"")));
    }
    Ok(owner)
}

pub static FNS: &[ApiFn] = &[
    def!("game.version", "game", [], "string", "The game's version (`\"0.2.22\"`).", NEW, None, false, |c, a| {
        let io = c.io();
        io.info().into_iter().find(|(k, _)| *k == "version").and_then(|(_, v)| Value::from(v).to_text()).unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string())
    }),
    def!("game.api", "game", [], "integer", "The version of the plugin interface (`api_abi` of an `.oop`; raised only when something is taken away or changes).", NEW, None, false, |c, a| crate::api::ABI),
    def!("game.has", "game", [p("name", "string")], "boolean", "Whether this game has an API function (`\"weather.set\"`): for a plugin that should also run in an older openOMSI.", NEW, None, false, |c, a| {
        let n = a.str(0)?;
        Ok::<_, ApiError>(crate::api::find(n.strip_prefix("omsi.").unwrap_or(&n)).is_some())
    }),
    def!("game.platform", "game", [], "string", "The system the game runs on: `\"windows\"`, `\"macos\"`, `\"linux\"`, `\"android\"`.", NEW, None, false, |c, a| std::env::consts::OS),
    def!("game.settings", "game", [], "table", "The settings a plugin may read: `graphics` (`\"vanilla\"`, `\"vanilla_plus\"`, `\"enhanced\"`), `language` (the cockpit's, `\"ENG\"`), `ui_language`, `ui_scale`, `volume`, `fov`, `render_scale`, `max_fps`, `fullscreen`, `vsync`, `msaa`, `shadows`, `time_speed`, `units` (`\"metric\"`: speeds are km/h everywhere), ...", NEW, None, false, |c, a| pairs(c.io().settings())),
    def!("game.stats", "game", [], "table", "This session's counts, as the personnel file has them: `km`, `stops_served`, `stops_skipped`, `crashes`, `heavy_crashes`, `pedestrians`, `tickets`, `cash`, `passengers`, ...", NEW, None, false, |c, a| pairs(c.io().stats())),
    def!("game.fps", "game", [], "number or nil", "Frames a second now.", NEW, None, false, |c, a| c.io().fps().map(|f| f as f64)),
    def!("game.menu_open", "game", [], "boolean", "Whether the game menu is open.", NEW, None, false, |c, a| c.io().menu_open()),
    def!("game.screenshot", "game", [], "string or nil", "Takes a screenshot with the next frame, as the camera key does; the file it goes to (the `screenshot` event says when it is there).", NEW, Ui, false, |c, a| c.io().screenshot()),
    def!("game.notify", "game", [p("text", "string"), o("kind", "string"), o("seconds", "number")], "boolean", "A notification of the game's own (the cards the server's messages use): `kind` `\"info\"`, `\"warning\"` or `\"alert\"`.", NEW, Ui, false, |c, a| {
        let t = a.str(0)?;
        let kind = match a.opt_str(1)?.as_deref() {
            None | Some("info") => 0,
            Some("warning") => 1,
            Some("alert") => 2,
            Some(k) => return Err(ApiError(format!("omsi.game.notify: \"{k}\" is none of info, warning, alert"))),
        };
        let s = a.opt_num(2)?.unwrap_or(6.0).clamp(1.0, 60.0) as f32;
        Ok(c.io().notify(&t, kind, s))
    }),
    def!("game.action", "game", [p("name", "string")], "boolean", "A game action of keyboard.cfg's `[game]` (`sim_pause`, `view_set_map`, `view_toggle_informationdisplay`, `view_set_schedule`, ...), as its key does; `true` when the game knows it.", NEW, WorldWrite, false, |c, a| {
        let n = a.str(0)?;
        Ok::<_, ApiError>(c.io().game_action(&n))
    }),
    def!("lan.active", "lan", [], "boolean", "Whether this game is in a LAN session.", NEW, None, false, |c, a| c.io().lan().is_some()),
    def!("lan.me", "lan", [], "id, name, host", "This player in the session: its id (the host is 1), its name, whether it hosts.", NEW, None, true, |c, a| multi(c.io().lan().map(|(h, id, n)| [Value::from(id), Value::Str(n), Value::Bool(h)]))),
    def!("lan.players", "lan", [], "list of tables", "The other players of the session: `{id, name, host, bus, line, tour, x, y, z, heading, speed, on_foot, passengers}`.", NEW, None, false, |c, a| {
        Value::List(c.io().lan_players().into_iter().map(|p| rec(vec![
            ("id", p.id.into()),
            ("name", p.name.into()),
            ("host", p.host.into()),
            ("bus", p.bus.into()),
            ("line", p.line.into()),
            ("tour", p.tour.into()),
            ("x", p.pos[0].into()),
            ("y", p.pos[1].into()),
            ("z", p.pos[2].into()),
            ("heading", p.pos[3].into()),
            ("speed", Value::from(p.speed_kmh)),
            ("on_foot", p.on_foot.into()),
            ("passengers", p.passengers.into()),
        ])).collect())
    }),
    def!("lan.send", "lan", [p("to", "integer"), p("text", "string")], "true, or false and the reason", "Sends a short text (at most 120 characters, no `|`) to the same plugin on another player's game (`to` its id; 0: every other player); it hears `lan_message(from, text)`. A player sends at most about ten a second; a game without the plugin ignores them.", NEW, Lan, true, |c, a| {
        let (to, text) = (a.int(0)?, a.str(1)?);
        if text.chars().count() > LAN_MAX || text.contains(['|', '\n', '\r']) {
            return Ok(super::core::ok_or_reason(Err(format!("a LAN message is at most {LAN_MAX} characters, without | or line breaks"))));
        }
        let name = c.state().name.clone();
        let io = c.io();
        let r = if to == 0 {
            let ids: Vec<u32> = io.lan_players().into_iter().map(|p| p.id).collect();
            ids.into_iter().try_for_each(|id| io.lan_send(&name, id, &text))
        } else {
            io.lan_send(&name, to.clamp(0, u32::MAX as i64) as u32, &text)
        };
        Ok::<_, ApiError>(super::core::ok_or_reason(r))
    }),
    def!("lan.chat", "lan", [p("text", "string")], "true, or false and the reason", "Says a line in the session's chat, as the player would (every player sees it; at most one a second).", NEW, Lan, true, |c, a| {
        let t = a.str(0)?;
        Ok::<_, ApiError>(super::core::ok_or_reason(c.io().lan_chat(&t)))
    }),
    def!("ui.update", "ui", [p("panel", "string"), p("element", "string"), p("values", "table")], "true, or false and the reason", "Changes one element of a panel in place, by its id: `text`, `color`, `value` (a bar, a slider), `checked`, `selected`, `name` (an icon), `values` (a chart), `rows` (a table), `src` (an image) - cheaper than setting the whole panel again.", NEW, Ui, true, |c, a| {
        let (id, el) = (a.str(0)?, a.str(1)?);
        let fields = a.table(2)?.clone();
        let s = c.state();
        let (owner, folder) = (s.owner, s.folder.clone());
        let r = s.ui().update(owner, &id, &el, &fields, folder.as_deref());
        Ok::<_, ApiError>(super::core::ok_or_reason(r))
    }),
    def!("ui.value", "ui", [p("panel", "string"), p("element", "string")], "any", "The value of a checkbox, slider, text field or tabs element now (as `ui_change` gives it).", NEW, Ui, false, |c, a| {
        let (id, el) = (a.str(0)?, a.str(1)?);
        let s = c.state();
        let owner = s.owner;
        let v = s.ui().value(owner, &id, &el);
        Ok::<_, ApiError>(v)
    }),
    def!("ui.show", "ui", [p("panel", "string"), o("on", "bool")], "boolean", "Shows a panel (or hides it with `false`; it is kept); `false` when there is none.", NEW, Ui, false, |c, a| {
        let (id, on) = (a.str(0)?, a.flag(1, true));
        let s = c.state();
        let owner = s.owner;
        let r = s.ui().set_visible(owner, &id, on);
        Ok::<_, ApiError>(r)
    }),
    def!("ui.toggle", "ui", [p("panel", "string")], "boolean", "Shows a hidden panel or hides a shown one; whether it shows now.", NEW, Ui, false, |c, a| {
        let id = a.str(0)?;
        let owner = panel_id(c, &id)?;
        let s = c.state();
        let on = !s.ui().get(owner, &id).is_some_and(|e| e.panel.visible);
        s.ui().set_visible(owner, &id, on);
        Ok::<_, ApiError>(on)
    }),
    def!("ui.panels", "ui", [], "list of strings", "The ids of the plugin's panels.", NEW, None, false, |c, a| {
        let s = c.state();
        let owner = s.owner;
        let ids = s.ui().ids(owner);
        ids
    }),
    def!("ui.moved", "ui", [p("panel", "string")], "dx, dy", "How far the player dragged a panel from where its table puts it (pixels).", NEW, None, true, |c, a| {
        let id = a.str(0)?;
        let s = c.state();
        let owner = s.owner;
        let m = s.ui().get(owner, &id).map(|e| e.moved);
        Ok::<_, ApiError>(multi(m.map(|(x, y)| [Value::from(x), Value::from(y)])))
    }),
    def!("ui.typing", "ui", [], "boolean", "Whether the player types into a text field of a plugin now (the keys then go to the field, not to the bus).", NEW, None, false, |c, a| c.state().ui().typing().is_some()),
];
