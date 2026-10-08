//! `omsi.ui`: on-screen panels and notifications of the Lua plugins (see docs/PLUGINS.md).
//!
//! A plugin describes a panel as a Lua table once and replaces it when its content changes;
//! the game lays it out and draws it every frame. This module is the part every game shares:
//! the tables read into [`Panel`]s (checked and held to the limits here, so a plugin learns
//! at once what is wrong), which plugin owns what, the notifications, whether the panels
//! have the mouse, and the clicks waiting for their plugin. Drawing and hit-testing are the
//! game's (`omsi-app`'s `plugin_ui`).

use mlua::{Table, Value};
use std::cell::RefCell;
use std::rc::Rc;

/// `omsi.ui.version`: raised when something is added a plugin may want to test for.
pub const VERSION: i64 = 1;
/// Most panels of one plugin.
pub const MAX_PANELS: usize = 16;
/// Most elements of one panel, those in rows included.
pub const MAX_ELEMENTS: usize = 200;
/// Longest text, in characters.
pub const MAX_TEXT: usize = 500;
/// Most notifications of one plugin on the screen at once.
pub const MAX_TOASTS: usize = 8;
/// Longest panel or element id, in characters.
pub const MAX_ID: usize = 64;
/// Rows in rows in rows...: deeper is an error (a table that holds itself would never end).
const MAX_DEPTH: usize = 8;
/// Clicks kept for plugins that do not run (the game paused): the oldest go.
const MAX_CLICKS: usize = 32;

/// The panels of every Lua plugin, shared by the plugins and the game.
pub type SharedUi = Rc<RefCell<UiState>>;

/// An sRGB colour with straight alpha, as `"#RRGGBB"` or `"#RRGGBBAA"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba(pub [u8; 4]);

impl Rgba {
    pub fn parse(s: &str) -> Option<Rgba> {
        let hex = s.trim().strip_prefix('#')?;
        if !(hex.len() == 6 || hex.len() == 8) || !hex.is_ascii() {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        let a = if hex.len() == 8 { byte(6)? } else { 255 };
        Some(Rgba([byte(0)?, byte(2)?, byte(4)?, a]))
    }
}

/// Where on the screen a panel is placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Anchor {
    #[default]
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Anchor {
    fn parse(s: &str) -> Option<Anchor> {
        Some(match s {
            "top_left" => Anchor::TopLeft,
            "top" => Anchor::Top,
            "top_right" => Anchor::TopRight,
            "left" => Anchor::Left,
            "center" => Anchor::Center,
            "right" => Anchor::Right,
            "bottom_left" => Anchor::BottomLeft,
            "bottom" => Anchor::Bottom,
            "bottom_right" => Anchor::BottomRight,
            _ => return None,
        })
    }

    /// Where the anchor is across and down the screen: 0, a half or 1 of each.
    pub fn factors(self) -> (f32, f32) {
        match self {
            Anchor::TopLeft => (0.0, 0.0),
            Anchor::Top => (0.5, 0.0),
            Anchor::TopRight => (1.0, 0.0),
            Anchor::Left => (0.0, 0.5),
            Anchor::Center => (0.5, 0.5),
            Anchor::Right => (1.0, 0.5),
            Anchor::BottomLeft => (0.0, 1.0),
            Anchor::Bottom => (0.5, 1.0),
            Anchor::BottomRight => (1.0, 1.0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Weight {
    #[default]
    Regular,
    Medium,
    Bold,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

/// How a row's children share a width they do not fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RowAlign {
    #[default]
    Start,
    Center,
    End,
    /// The room left between the children.
    Between,
}

/// A panel: a card of elements stacked top to bottom. Sizes are logical pixels (the game
/// scales them with its interface).
#[derive(Debug, Clone, PartialEq)]
pub struct Panel {
    pub anchor: Anchor,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub padding: f32,
    pub gap: f32,
    /// None: the game's own card colour.
    pub background: Option<Rgba>,
    pub radius: f32,
    /// A stripe along the left edge.
    pub accent: Option<Rgba>,
    pub visible: bool,
    /// A click anywhere on it is a `ui_click` with no element.
    pub clickable: bool,
    pub children: Vec<Element>,
}

/// One element of a panel or a row (those with `visible = false` are left out when read).
#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    pub id: Option<String>,
    pub color: Option<Rgba>,
    /// In a row: takes the width the others leave.
    pub grow: bool,
    pub clickable: bool,
    pub kind: Kind,
}

impl Element {
    /// Whether a click on it is a `ui_click`: a button's always, others' with an id and
    /// `clickable = true`.
    pub fn takes_clicks(&self) -> bool {
        matches!(self.kind, Kind::Button { .. }) || (self.clickable && self.id.is_some())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Text {
        text: String,
        size: f32,
        weight: Weight,
        align: Align,
        wrap: bool,
    },
    /// A Material Symbols name of the game's icons; one it has not draws nothing.
    Icon {
        name: String,
        size: f32,
    },
    Row {
        children: Vec<Element>,
        gap: f32,
        align: RowAlign,
    },
    Bar {
        value: f32,
        height: f32,
        background: Option<Rgba>,
    },
    Badge {
        text: String,
        text_color: Option<Rgba>,
    },
    Divider,
    Space {
        size: f32,
    },
    Button {
        text: String,
        icon: Option<String>,
    },
}

/// A notification card of `omsi.ui.toast`.
#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    pub owner: u64,
    /// Tells the game's drawings of toasts apart (none is ever used twice).
    pub serial: u64,
    pub text: String,
    pub title: Option<String>,
    pub icon: Option<String>,
    pub color: Option<Rgba>,
    pub seconds: f32,
    /// Seconds it has been shown.
    pub age: f32,
}

/// A panel and who made it.
#[derive(Debug, Clone, PartialEq)]
pub struct PanelEntry {
    pub owner: u64,
    pub id: String,
    pub panel: Panel,
    /// Changes whenever the panel does: the game lays it out again only then.
    pub revision: u64,
}

/// A click on a panel, for its plugin.
#[derive(Debug, Clone, PartialEq)]
pub struct Click {
    pub owner: u64,
    pub panel: String,
    pub element: Option<String>,
}

/// Everything the plugins show, and what the game tells them back.
#[derive(Debug)]
pub struct UiState {
    panels: Vec<PanelEntry>,
    toasts: Vec<Toast>,
    clicks: Vec<Click>,
    /// The plugin that gave the panels the mouse, while they have it.
    focus: Option<u64>,
    /// Whether the panels are on the screen (the game hides them under its menus).
    shown: bool,
    /// Logical width, height, and physical pixels per logical one.
    screen: [f32; 3],
    serial: u64,
}

impl Default for UiState {
    fn default() -> Self {
        // (until the game has drawn a frame: a 1080p screen)
        UiState {
            panels: Vec::new(),
            toasts: Vec::new(),
            clicks: Vec::new(),
            focus: None,
            shown: true,
            screen: [1920.0, 1080.0, 1.0],
            serial: 0,
        }
    }
}

impl UiState {
    fn next_serial(&mut self) -> u64 {
        self.serial += 1;
        self.serial
    }

    /// Create or replace a plugin's panel. The same panel again changes nothing (a plugin
    /// that sets its panels on a timer redraws nothing while they stay the same).
    pub fn set(&mut self, owner: u64, id: &str, panel: Panel) -> Result<(), String> {
        check_id("the panel id", id)?;
        if let Some(i) = self
            .panels
            .iter()
            .position(|p| p.owner == owner && p.id == id)
        {
            if self.panels[i].panel != panel {
                let revision = self.next_serial();
                let e = &mut self.panels[i];
                e.panel = panel;
                e.revision = revision;
            }
            return Ok(());
        }
        if self.panels.iter().filter(|p| p.owner == owner).count() >= MAX_PANELS {
            return Err(format!("a plugin has at most {MAX_PANELS} panels"));
        }
        let revision = self.next_serial();
        self.panels.push(PanelEntry {
            owner,
            id: id.to_string(),
            panel,
            revision,
        });
        Ok(())
    }

    /// Remove a plugin's panel; false when it had none of that id.
    pub fn remove(&mut self, owner: u64, id: &str) -> bool {
        let n = self.panels.len();
        self.panels.retain(|p| !(p.owner == owner && p.id == id));
        self.clicks.retain(|c| !(c.owner == owner && c.panel == id));
        self.panels.len() != n
    }

    /// Remove every panel of a plugin.
    pub fn clear(&mut self, owner: u64) {
        self.panels.retain(|p| p.owner != owner);
        self.clicks.retain(|c| c.owner != owner);
    }

    /// What goes when a plugin stops (or is loaded again, or switched off): its panels and
    /// clicks, and the mouse when it was the one that asked for it (the cursor stayed on
    /// panels nobody would answer any more). Its notifications run their time: they say
    /// something that happened, and go by themselves.
    pub fn remove_owner(&mut self, owner: u64) {
        self.clear(owner);
        if self.focus == Some(owner) {
            self.focus = None;
        }
    }

    /// Show a notification; a plugin's oldest makes room when it has `MAX_TOASTS` already
    /// (a plugin that sends one per event is not told off for a busy minute).
    pub fn toast(&mut self, owner: u64, spec: ToastSpec) {
        if let Some(oldest) = self.toasts.iter().position(|t| t.owner == owner) {
            if self.toasts.iter().filter(|t| t.owner == owner).count() >= MAX_TOASTS {
                self.toasts.remove(oldest);
            }
        }
        let serial = self.next_serial();
        let ToastSpec {
            text,
            title,
            icon,
            color,
            seconds,
        } = spec;
        self.toasts.push(Toast {
            owner,
            serial,
            text,
            title,
            icon,
            color,
            seconds,
            age: 0.0,
        });
    }

    /// The panels have the mouse (`on`, asked by plugin `owner`) or let it go. They cannot
    /// have it while the game hides them. Returns the new state.
    pub fn set_focus(&mut self, owner: u64, on: bool) -> bool {
        self.focus = (on && self.shown).then_some(owner);
        self.focus.is_some()
    }

    pub fn focused(&self) -> bool {
        self.focus.is_some()
    }

    /// The game takes the mouse back (Esc).
    pub fn release_focus(&mut self) {
        self.focus = None;
    }

    /// The game shows the panels or hides them (its menus open); hidden, they let the mouse go.
    pub fn set_shown(&mut self, shown: bool) {
        self.shown = shown;
        if !shown {
            self.focus = None;
        }
    }

    pub fn set_screen(&mut self, width: f32, height: f32, scale: f32) {
        self.screen = [width, height, scale];
    }

    /// `omsi.ui.screen()`: logical width, height, and the interface's scale.
    pub fn screen(&self) -> [f32; 3] {
        self.screen
    }

    pub fn panels(&self) -> &[PanelEntry] {
        &self.panels
    }

    /// The notifications on the screen, oldest first.
    pub fn toasts(&self) -> &[Toast] {
        &self.toasts
    }

    /// The game's time passes for the notifications: those over go.
    pub fn tick(&mut self, dt: f32) {
        for t in self.toasts.iter_mut() {
            t.age += dt.max(0.0);
        }
        self.toasts.retain(|t| t.age < t.seconds);
    }

    /// A click on a panel (`element`: the element's id, None for the panel itself), sent to
    /// its plugin in the plugin's next frame.
    pub fn click(&mut self, owner: u64, panel: &str, element: Option<&str>) {
        if self.clicks.len() >= MAX_CLICKS {
            self.clicks.remove(0);
        }
        self.clicks.push(Click {
            owner,
            panel: panel.to_string(),
            element: element.map(str::to_string),
        });
    }

    /// The clicks waiting for a plugin.
    pub fn take_clicks(&mut self, owner: u64) -> Vec<Click> {
        let (mine, rest) = std::mem::take(&mut self.clicks)
            .into_iter()
            .partition(|c| c.owner == owner);
        self.clicks = rest;
        mine
    }
}

fn check_id(what: &str, id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err(format!("{what} is empty"));
    }
    if id.chars().count() > MAX_ID {
        return Err(format!("{what} is longer than {MAX_ID} characters"));
    }
    Ok(())
}

// --- the Lua tables ----------------------------------------------------------------------

/// Read a panel table. Unknown keys are left alone and elements of an unknown type skipped,
/// so a plugin written for a later game still shows here; a key of the wrong kind is an
/// error that says where it is.
pub fn parse_panel(t: &Table) -> Result<Panel, String> {
    let r = Reader {
        t,
        path: String::new(),
    };
    let mut count = 0;
    Ok(Panel {
        anchor: match r.string("anchor")? {
            None => Anchor::TopLeft,
            Some(a) => Anchor::parse(&a).ok_or_else(|| format!("anchor: \"{a}\" is no anchor (top_left, top, top_right, left, center, right, bottom_left, bottom, bottom_right)"))?,
        },
        x: r.number("x", 0.0, -10000.0, 10000.0)?,
        y: r.number("y", 0.0, -10000.0, 10000.0)?,
        width: r.number("width", 300.0, 40.0, 4000.0)?,
        padding: r.number("padding", 12.0, 0.0, 64.0)?,
        gap: r.number("gap", 6.0, 0.0, 64.0)?,
        background: r.color("background")?,
        radius: r.number("radius", 12.0, 0.0, 64.0)?,
        accent: r.color("accent")?,
        visible: r.boolean("visible", true)?,
        clickable: r.boolean("clickable", false)?,
        children: r.children("children", 0, &mut count)?,
    })
}

/// What `omsi.ui.toast(text, opts)` asks for.
#[derive(Debug, Clone, PartialEq)]
pub struct ToastSpec {
    pub text: String,
    pub title: Option<String>,
    pub icon: Option<String>,
    pub color: Option<Rgba>,
    pub seconds: f32,
}

/// Read `omsi.ui.toast`'s text and options (`seconds`, `title`, `icon`, `color`) with the
/// same limits and messages as a panel's.
pub fn parse_toast(text: &Value, opts: Option<&Table>) -> Result<ToastSpec, String> {
    let text = match text {
        Value::String(s) => s
            .to_str()
            .map_err(|_| "the text is no UTF-8".to_string())?
            .to_string(),
        Value::Integer(i) => i.to_string(),
        Value::Number(n) => Value::Number(*n).to_string().unwrap_or_default(),
        _ => return Err("the text of a notification is a text".to_string()),
    };
    if text.chars().count() > MAX_TEXT {
        return Err(format!("the text is longer than {MAX_TEXT} characters"));
    }
    let mut spec = ToastSpec {
        text,
        title: None,
        icon: None,
        color: None,
        seconds: 5.0,
    };
    if let Some(t) = opts {
        let r = Reader {
            t,
            path: "opts".into(),
        };
        spec.title = r.string("title")?;
        spec.icon = r.string("icon")?;
        spec.color = r.color("color")?;
        spec.seconds = r.number("seconds", 5.0, 1.0, 60.0)?;
    }
    Ok(spec)
}

/// A panel written as a plugin writes it - a Lua table constructor, `{ width = 300, ... }` -
/// read in a Lua state of its own: for the game's tests and pictures of panels.
pub fn panel_from_source(src: &str) -> Result<Panel, String> {
    let lua = mlua::Lua::new();
    let t: Table = lua
        .load(format!("return {src}"))
        .eval()
        .map_err(|e| e.to_string())?;
    parse_panel(&t)
}

/// Reads the keys of one table; `path` says where it is for the error messages.
struct Reader<'a> {
    t: &'a Table,
    path: String,
}

impl Reader<'_> {
    fn get(&self, key: &str) -> Result<Value, String> {
        // (raw: a metatable's code has no say in what the game reads)
        self.t
            .raw_get::<Value>(key)
            .map_err(|e| format!("{}: {e}", self.at(key)))
    }

    fn at(&self, key: &str) -> String {
        if self.path.is_empty() {
            key.to_string()
        } else {
            format!("{}.{key}", self.path)
        }
    }

    fn number(&self, key: &str, default: f32, min: f32, max: f32) -> Result<f32, String> {
        let v = match self.get(key)? {
            Value::Nil => return Ok(default),
            Value::Integer(i) => i as f64,
            Value::Number(n) if n.is_finite() => n,
            _ => return Err(format!("{}: a number is expected", self.at(key))),
        };
        Ok((v as f32).clamp(min, max))
    }

    fn boolean(&self, key: &str, default: bool) -> Result<bool, String> {
        match self.get(key)? {
            Value::Nil => Ok(default),
            Value::Boolean(b) => Ok(b),
            _ => Err(format!("{}: true or false is expected", self.at(key))),
        }
    }

    /// A text (numbers are written as Lua writes them), at most `MAX_TEXT` characters.
    fn string(&self, key: &str) -> Result<Option<String>, String> {
        let s = match self.get(key)? {
            Value::Nil => return Ok(None),
            Value::String(s) => s
                .to_str()
                .map_err(|_| format!("{}: the text is no UTF-8", self.at(key)))?
                .to_string(),
            Value::Integer(i) => i.to_string(),
            Value::Number(n) => mlua::Value::Number(n).to_string().unwrap_or_default(),
            _ => return Err(format!("{}: a text is expected", self.at(key))),
        };
        if s.chars().count() > MAX_TEXT {
            return Err(format!(
                "{}: longer than {MAX_TEXT} characters",
                self.at(key)
            ));
        }
        Ok(Some(s))
    }

    fn color(&self, key: &str) -> Result<Option<Rgba>, String> {
        match self.string(key)? {
            None => Ok(None),
            Some(s) => Rgba::parse(&s).map(Some).ok_or_else(|| {
                format!(
                    "{}: \"{s}\" is no colour (\"#RRGGBB\" or \"#RRGGBBAA\")",
                    self.at(key)
                )
            }),
        }
    }

    fn children(&self, key: &str, depth: usize, count: &mut usize) -> Result<Vec<Element>, String> {
        let list = match self.get(key)? {
            Value::Nil => return Ok(Vec::new()),
            Value::Table(t) => t,
            _ => return Err(format!("{}: a list of elements is expected", self.at(key))),
        };
        if depth >= MAX_DEPTH {
            return Err(format!(
                "{}: rows go at most {MAX_DEPTH} deep",
                self.at(key)
            ));
        }
        let mut out = Vec::new();
        for i in 1..=list.raw_len() {
            let path = format!("{}[{i}]", self.at(key));
            let t = match list
                .raw_get::<Value>(i)
                .map_err(|e| format!("{path}: {e}"))?
            {
                Value::Table(t) => t,
                _ => return Err(format!("{path}: an element is a table")),
            };
            *count += 1;
            if *count > MAX_ELEMENTS {
                return Err(format!("a panel has at most {MAX_ELEMENTS} elements"));
            }
            if let Some(e) = (Reader { t: &t, path }).element(depth, count)? {
                out.push(e);
            }
        }
        Ok(out)
    }

    /// One element; None when it is hidden or of a type this game does not know.
    fn element(&self, depth: usize, count: &mut usize) -> Result<Option<Element>, String> {
        let ty = self
            .string("type")?
            .ok_or_else(|| format!("{}: no type", self.path))?;
        let id = self.string("id")?;
        if let Some(id) = id.as_deref() {
            check_id(&self.at("id"), id)?;
        }
        let text = || self.string("text").map(Option::unwrap_or_default);
        let kind = match ty.as_str() {
            "text" => Kind::Text {
                text: text()?,
                size: self.number("size", 14.0, 6.0, 96.0)?,
                weight: match self.string("weight")?.as_deref() {
                    None | Some("regular") => Weight::Regular,
                    Some("medium") => Weight::Medium,
                    Some("bold") => Weight::Bold,
                    Some(w) => {
                        return Err(format!(
                            "{}: \"{w}\" is no weight (regular, medium, bold)",
                            self.at("weight")
                        ))
                    }
                },
                align: match self.string("align")?.as_deref() {
                    None | Some("left") => Align::Left,
                    Some("center") => Align::Center,
                    Some("right") => Align::Right,
                    Some(a) => {
                        return Err(format!(
                            "{}: \"{a}\" is no alignment (left, center, right)",
                            self.at("align")
                        ))
                    }
                },
                wrap: self.boolean("wrap", true)?,
            },
            "icon" => Kind::Icon {
                name: self.string("name")?.unwrap_or_default(),
                size: self.number("size", 20.0, 6.0, 128.0)?,
            },
            "row" => Kind::Row {
                children: self.children("children", depth + 1, count)?,
                gap: self.number("gap", 8.0, 0.0, 64.0)?,
                align: match self.string("align")?.as_deref() {
                    None | Some("start") => RowAlign::Start,
                    Some("center") => RowAlign::Center,
                    Some("end") => RowAlign::End,
                    Some("between") => RowAlign::Between,
                    Some(a) => {
                        return Err(format!(
                            "{}: \"{a}\" is no alignment (start, center, end, between)",
                            self.at("align")
                        ))
                    }
                },
            },
            "bar" => Kind::Bar {
                value: self.number("value", 0.0, 0.0, 1.0)?,
                height: self.number("height", 6.0, 1.0, 64.0)?,
                background: self.color("background")?,
            },
            "badge" => Kind::Badge {
                text: text()?,
                text_color: self.color("text_color")?,
            },
            "divider" => Kind::Divider,
            "space" => Kind::Space {
                size: self.number("size", 8.0, 0.0, 1000.0)?,
            },
            "button" => {
                if id.is_none() {
                    return Err(format!("{}: a button needs an id", self.path));
                }
                Kind::Button {
                    text: text()?,
                    icon: self.string("icon")?,
                }
            }
            // (an element of a later version of the API: left out)
            _ => return Ok(None),
        };
        let e = Element {
            id,
            color: self.color("color")?,
            grow: self.boolean("grow", false)?,
            clickable: self.boolean("clickable", false)?,
            kind,
        };
        Ok(self.boolean("visible", true)?.then_some(e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::Lua;

    fn panel(lua: &Lua, src: &str) -> Result<Panel, String> {
        let t: Table = lua.load(src).eval().unwrap();
        parse_panel(&t)
    }

    #[test]
    fn colours() {
        assert_eq!(Rgba::parse("#F47F30"), Some(Rgba([0xF4, 0x7F, 0x30, 255])));
        assert_eq!(
            Rgba::parse("#14161acc"),
            Some(Rgba([0x14, 0x16, 0x1A, 0xCC]))
        );
        for bad in ["F47F30", "#F47F3", "#GGGGGG", "#F47F30C", "#ÄÄÄ"] {
            assert_eq!(Rgba::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_panel_is_read_with_its_defaults() {
        let lua = Lua::new();
        let p = panel(
            &lua,
            r##"{ anchor = "bottom_right", x = 16, width = 340, accent = "#F47F30", children = {
                { type = "text", text = "Linie 42", size = 18, weight = "bold" },
                { type = "row", align = "between", children = {
                    { type = "icon", name = "schedule" },
                    { type = "text", text = 12.5, grow = true },
                    { type = "badge", text = "+2", color = "#2E7D32" },
                } },
                { type = "bar", value = 1.7 },
                { type = "text", text = "hidden", visible = false },
                { type = "sparkles", text = "from a later version" },
                { type = "button", id = "pause", text = "Pause", icon = "pause" },
            } }"##,
        )
        .unwrap();
        assert_eq!(p.anchor, Anchor::BottomRight);
        assert_eq!(
            (p.x, p.y, p.width, p.padding, p.gap, p.radius),
            (16.0, 0.0, 340.0, 12.0, 6.0, 12.0)
        );
        assert_eq!(p.accent, Some(Rgba([0xF4, 0x7F, 0x30, 255])));
        assert!(p.visible && !p.clickable && p.background.is_none());
        // (the hidden text and the unknown type are left out)
        assert_eq!(p.children.len(), 4);
        assert_eq!(
            p.children[0].kind,
            Kind::Text {
                text: "Linie 42".into(),
                size: 18.0,
                weight: Weight::Bold,
                align: Align::Left,
                wrap: true
            }
        );
        let Kind::Row {
            children,
            gap,
            align,
        } = &p.children[1].kind
        else {
            panic!("{:?}", p.children[1])
        };
        assert_eq!((*gap, *align, children.len()), (8.0, RowAlign::Between, 3));
        assert!(children[1].grow);
        assert_eq!(
            children[1].kind,
            Kind::Text {
                text: "12.5".into(),
                size: 14.0,
                weight: Weight::Regular,
                align: Align::Left,
                wrap: true
            }
        );
        // (held to its range)
        assert_eq!(
            p.children[2].kind,
            Kind::Bar {
                value: 1.0,
                height: 6.0,
                background: None
            }
        );
        assert!(p.children[3].takes_clicks() && !p.children[0].takes_clicks());
    }

    #[test]
    fn mistakes_say_where_they_are() {
        let lua = Lua::new();
        let cases = [
            (r#"{ width = "wide" }"#, "width: a number is expected"),
            (
                r#"{ anchor = "middle" }"#,
                "anchor: \"middle\" is no anchor",
            ),
            (
                r#"{ background = "red" }"#,
                "background: \"red\" is no colour",
            ),
            (
                r#"{ children = { { type = "button", text = "Go" } } }"#,
                "children[1]: a button needs an id",
            ),
            (
                r#"{ children = { { type = "row", children = { { type = "text", size = {} } } } } }"#,
                "children[1].children[1].size: a number is expected",
            ),
            (
                r#"{ children = { { text = "no type" } } }"#,
                "children[1]: no type",
            ),
            (
                r#"{ children = { "text" } }"#,
                "children[1]: an element is a table",
            ),
            (
                r#"{ children = { { type = "text", text = string.rep("x", 501) } } }"#,
                "children[1].text: longer than 500 characters",
            ),
            (
                r#"{ children = { { type = "text", weight = "heavy" } } }"#,
                "is no weight",
            ),
        ];
        for (src, want) in cases {
            let e = panel(&lua, src).unwrap_err();
            assert!(e.contains(want), "{src}: {e}");
        }
        // too many elements, rows counted with what is in them
        let e = panel(&lua, "local c = {} for i = 1, 101 do c[i] = { type = 'row', children = { { type = 'space' } } } end return { children = c }").unwrap_err();
        assert!(e.contains("at most 200 elements"), "{e}");
        // a row holding itself ends at the depth limit
        let e = panel(
            &lua,
            "local r = { type = 'row' } r.children = { r } return { children = { r } }",
        )
        .unwrap_err();
        assert!(e.contains("deep"), "{e}");
    }

    #[test]
    fn panels_belong_to_their_plugin() {
        let lua = Lua::new();
        let p = panel(&lua, "{}").unwrap();
        let mut ui = UiState::default();
        for i in 0..MAX_PANELS {
            ui.set(1, &format!("p{i}"), p.clone()).unwrap();
        }
        assert!(ui
            .set(1, "one too many", p.clone())
            .unwrap_err()
            .contains("at most 16 panels"));
        // replacing one is no new panel; another plugin has its own sixteen
        ui.set(1, "p0", p.clone()).unwrap();
        ui.set(2, "p0", p.clone()).unwrap();
        assert!(ui.set(1, "", p.clone()).is_err());
        assert_eq!(ui.panels().len(), MAX_PANELS + 1);
        assert!(ui.remove(1, "p3") && !ui.remove(1, "p3"));
        ui.clear(1);
        assert_eq!(ui.panels().iter().map(|p| p.owner).collect::<Vec<_>>(), [2]);
    }

    #[test]
    fn the_same_panel_again_is_no_change() {
        let lua = Lua::new();
        let mut ui = UiState::default();
        ui.set(
            1,
            "trip",
            panel(&lua, r#"{ children = { { type = "text", text = "a" } } }"#).unwrap(),
        )
        .unwrap();
        let first = ui.panels()[0].revision;
        ui.set(
            1,
            "trip",
            panel(&lua, r#"{ children = { { type = "text", text = "a" } } }"#).unwrap(),
        )
        .unwrap();
        assert_eq!(ui.panels()[0].revision, first);
        ui.set(
            1,
            "trip",
            panel(&lua, r#"{ children = { { type = "text", text = "b" } } }"#).unwrap(),
        )
        .unwrap();
        assert_ne!(ui.panels()[0].revision, first);
    }

    #[test]
    fn toasts_go_by_themselves_and_make_room() {
        let lua = Lua::new();
        let mut ui = UiState::default();
        for i in 0..MAX_TOASTS + 2 {
            ui.toast(
                1,
                parse_toast(
                    &Value::String(lua.create_string(format!("t{i}")).unwrap()),
                    None,
                )
                .unwrap(),
            );
        }
        let opts: Table = lua
            .load(r##"{ seconds = 1, title = "Pay", icon = "payments", color = "#2E7D32" }"##)
            .eval()
            .unwrap();
        let spec = parse_toast(&Value::Integer(42), Some(&opts)).unwrap();
        assert_eq!(
            spec,
            ToastSpec {
                text: "42".into(),
                title: Some("Pay".into()),
                icon: Some("payments".into()),
                color: Some(Rgba([0x2E, 0x7D, 0x32, 255])),
                seconds: 1.0
            }
        );
        ui.toast(2, spec);
        let bad: Table = lua.load(r#"{ seconds = "long" }"#).eval().unwrap();
        assert_eq!(
            parse_toast(&Value::Integer(1), Some(&bad)).unwrap_err(),
            "opts.seconds: a number is expected"
        );
        assert_eq!(ui.toasts().len(), MAX_TOASTS + 1);
        assert_eq!(ui.toasts()[0].text, "t2");
        ui.tick(2.0);
        assert!(ui.toasts().iter().all(|t| t.owner == 1));
        ui.tick(3.5);
        assert!(ui.toasts().is_empty());
    }

    #[test]
    fn focus_clicks_and_owners() {
        let mut ui = UiState::default();
        ui.set(
            7,
            "menu",
            Panel {
                anchor: Anchor::Center,
                x: 0.0,
                y: 0.0,
                width: 200.0,
                padding: 12.0,
                gap: 6.0,
                background: None,
                radius: 12.0,
                accent: None,
                visible: true,
                clickable: true,
                children: Vec::new(),
            },
        )
        .unwrap();
        assert!(ui.set_focus(7, true) && ui.focused());
        ui.release_focus();
        assert!(!ui.focused());
        // hidden under the game's menu, the panels let the mouse go and cannot have it
        assert!(ui.set_focus(7, true));
        ui.set_shown(false);
        assert!(!ui.focused() && !ui.set_focus(7, true));
        ui.set_shown(true);
        assert!(ui.set_focus(7, true));
        ui.click(7, "menu", Some("ok"));
        ui.click(8, "x", None);
        assert_eq!(
            ui.take_clicks(7),
            [Click {
                owner: 7,
                panel: "menu".into(),
                element: Some("ok".into())
            }]
        );
        assert!(ui.take_clicks(7).is_empty());
        // another plugin stops: the mouse stays; the one that asked for it stops: it goes
        ui.remove_owner(8);
        assert!(ui.focused());
        ui.click(8, "x", None);
        ui.remove_owner(7);
        assert!(ui.panels().is_empty() && !ui.focused());
        assert_eq!(ui.take_clicks(8).len(), 1);
    }
}
