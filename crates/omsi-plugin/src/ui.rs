//! `omsi.ui`: on-screen panels and notifications of the plugins (see docs/PLUGINS.md).
//!
//! A plugin describes a panel as a table once and replaces it (or patches one element of it,
//! `ui.update`) when its content changes; the game lays it out and draws it every frame. This
//! module is the part every game shares: the tables read into [`Panel`]s (checked and held to
//! the limits here, so a plugin learns at once what is wrong), which plugin owns what, the
//! notifications, whether the panels have the mouse, the clicks and the changes of sliders,
//! boxes and fields waiting for their plugin, the panels dragged and the field being typed
//! into. Drawing and hit-testing are the game's (`omsi-app`'s `plugin_ui`).

use crate::api::Value;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// `omsi.ui.version`: raised when something is added a plugin may want to test for.
/// 2: images, checkboxes, sliders, text fields, tabs, charts, tables, draggable panels.
pub const VERSION: i64 = 2;
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
/// Most values of a chart, cells of a table.
pub const MAX_POINTS: usize = 512;
/// Rows in rows in rows...: deeper is an error (a table that holds itself would never end).
const MAX_DEPTH: usize = 8;
/// Clicks and changes kept for plugins that do not run (the game paused): the oldest go.
const MAX_CLICKS: usize = 32;

/// The panels of every plugin, shared by the plugins and the game.
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
    /// While the panels have the mouse it can be moved by dragging it (where it has nothing
    /// clickable); the place it is moved to stays when it is set again.
    pub draggable: bool,
    pub children: Vec<Element>,
}

impl Default for Panel {
    fn default() -> Panel {
        Panel { anchor: Anchor::TopLeft, x: 0.0, y: 0.0, width: 300.0, padding: 12.0, gap: 6.0, background: None, radius: 12.0, accent: None, visible: true, clickable: false, draggable: false, children: Vec::new() }
    }
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

    /// Whether the mouse changes its value (a checkbox, a slider, a field, tabs).
    pub fn is_control(&self) -> bool {
        self.id.is_some() && matches!(self.kind, Kind::Checkbox { .. } | Kind::Slider { .. } | Kind::Input { .. } | Kind::Tabs { .. })
    }

    /// Its value as `ui.value` and `ui_change` give it.
    pub fn value(&self) -> Value {
        match &self.kind {
            Kind::Checkbox { checked, .. } => Value::Bool(*checked),
            Kind::Slider { value, .. } => Value::Num(*value as f64),
            Kind::Input { text, .. } => Value::Str(text.clone()),
            Kind::Tabs { selected, .. } => Value::Int(*selected as i64 + 1),
            _ => Value::Nil,
        }
    }
}

/// How a chart draws its values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChartStyle {
    #[default]
    Line,
    Bars,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Text { text: String, size: f32, weight: Weight, align: Align, wrap: bool },
    /// A Material Symbols name of the game's icons; one it has not draws nothing.
    Icon { name: String, size: f32 },
    Row { children: Vec<Element>, gap: f32, align: RowAlign },
    Bar { value: f32, height: f32, background: Option<Rgba> },
    Badge { text: String, text_color: Option<Rgba> },
    Divider,
    Space { size: f32 },
    Button { text: String, icon: Option<String> },
    /// A picture of the plugin's folder (PNG, JPEG, BMP, TGA); `path` None: no such file.
    Image { path: Option<PathBuf>, width: f32, height: f32 },
    Checkbox { text: String, checked: bool },
    Slider { value: f32, min: f32, max: f32, step: f32 },
    /// A text field: clicked, it takes what is typed until Enter, Esc or a click elsewhere.
    Input { text: String, placeholder: String, max: usize },
    Tabs { tabs: Vec<String>, selected: usize },
    /// Values over time, as a line or as bars; `min`/`max` None: the values' own.
    Chart { values: Vec<f32>, min: Option<f32>, max: Option<f32>, height: f32, style: ChartStyle, fill: bool },
    /// Columns of texts: the header row bold; `widths` shares of the width (none: alike).
    Table { columns: Vec<String>, rows: Vec<Vec<String>>, widths: Vec<f32>, size: f32 },
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
    /// Where it was dragged to, from where its table puts it (logical pixels).
    pub moved: (f32, f32),
}

/// A click on a panel, for its plugin.
#[derive(Debug, Clone, PartialEq)]
pub struct Click {
    pub owner: u64,
    pub panel: String,
    pub element: Option<String>,
}

/// A new value of a checkbox, slider, field or tabs, for its plugin (`ui_change`).
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub owner: u64,
    pub panel: String,
    pub element: String,
    pub value: Value,
}

/// The field being typed into.
#[derive(Debug, Clone, PartialEq)]
pub struct Typing {
    pub owner: u64,
    pub panel: String,
    pub element: String,
}

/// Everything the plugins show, and what the game tells them back.
#[derive(Debug)]
pub struct UiState {
    panels: Vec<PanelEntry>,
    toasts: Vec<Toast>,
    clicks: Vec<Click>,
    changes: Vec<Change>,
    /// The plugin that gave the panels the mouse, while they have it.
    focus: Option<u64>,
    /// Whether the panels are on the screen (the game hides them under its menus).
    shown: bool,
    /// Logical width, height, and physical pixels per logical one.
    screen: [f32; 3],
    serial: u64,
    typing: Option<Typing>,
}

impl Default for UiState {
    fn default() -> Self {
        // (until the game has drawn a frame: a 1080p screen)
        UiState { panels: Vec::new(), toasts: Vec::new(), clicks: Vec::new(), changes: Vec::new(), focus: None, shown: true, screen: [1920.0, 1080.0, 1.0], serial: 0, typing: None }
    }
}

impl UiState {
    fn next_serial(&mut self) -> u64 {
        self.serial += 1;
        self.serial
    }

    fn find(&mut self, owner: u64, id: &str) -> Option<&mut PanelEntry> {
        self.panels.iter_mut().find(|p| p.owner == owner && p.id == id)
    }

    /// Create or replace a plugin's panel. The same panel again changes nothing (a plugin
    /// that sets its panels on a timer redraws nothing while they stay the same).
    pub fn set(&mut self, owner: u64, id: &str, panel: Panel) -> Result<(), String> {
        check_id("the panel id", id)?;
        if let Some(i) = self.panels.iter().position(|p| p.owner == owner && p.id == id) {
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
        self.panels.push(PanelEntry { owner, id: id.to_string(), panel, revision, moved: (0.0, 0.0) });
        Ok(())
    }

    /// Remove a plugin's panel; false when it had none of that id.
    pub fn remove(&mut self, owner: u64, id: &str) -> bool {
        let n = self.panels.len();
        self.panels.retain(|p| !(p.owner == owner && p.id == id));
        self.clicks.retain(|c| !(c.owner == owner && c.panel == id));
        self.changes.retain(|c| !(c.owner == owner && c.panel == id));
        if self.typing.as_ref().is_some_and(|t| t.owner == owner && t.panel == id) {
            self.typing = None;
        }
        self.panels.len() != n
    }

    /// Remove every panel of a plugin.
    pub fn clear(&mut self, owner: u64) {
        self.panels.retain(|p| p.owner != owner);
        self.clicks.retain(|c| c.owner != owner);
        self.changes.retain(|c| c.owner != owner);
        if self.typing.as_ref().is_some_and(|t| t.owner == owner) {
            self.typing = None;
        }
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

    /// The ids of a plugin's panels.
    pub fn ids(&self, owner: u64) -> Vec<String> {
        self.panels.iter().filter(|p| p.owner == owner).map(|p| p.id.clone()).collect()
    }

    /// A plugin's panel.
    pub fn get(&self, owner: u64, id: &str) -> Option<&PanelEntry> {
        self.panels.iter().find(|p| p.owner == owner && p.id == id)
    }

    /// Show or hide a panel; false when there is none.
    pub fn set_visible(&mut self, owner: u64, id: &str, on: bool) -> bool {
        let serial = self.next_serial();
        match self.find(owner, id) {
            Some(e) => {
                if e.panel.visible != on {
                    e.panel.visible = on;
                    e.revision = serial;
                }
                true
            }
            None => false,
        }
    }

    /// Change keys of one element of a panel (`ui.update`): what an element of its type
    /// takes, read as `set` reads them. Err when there is no such panel or element.
    pub fn update(&mut self, owner: u64, id: &str, element: &str, fields: &Value, folder: Option<&Path>) -> Result<(), String> {
        let serial = self.next_serial();
        let e = self.find(owner, id).ok_or_else(|| format!("no panel \"{id}\""))?;
        let el = find_element(&mut e.panel.children, element).ok_or_else(|| format!("no element \"{element}\" in panel \"{id}\""))?;
        let before = el.clone();
        patch(el, fields, folder)?;
        if *el != before {
            e.revision = serial;
        }
        Ok(())
    }

    /// The value of a checkbox, slider, field or tabs.
    pub fn value(&self, owner: u64, id: &str, element: &str) -> Option<Value> {
        let e = self.panels.iter().find(|p| p.owner == owner && p.id == id)?;
        find_in(&e.panel.children, element).map(Element::value)
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
        let ToastSpec { text, title, icon, color, seconds } = spec;
        self.toasts.push(Toast { owner, serial, text, title, icon, color, seconds, age: 0.0 });
    }

    /// The panels have the mouse (`on`, asked by plugin `owner`) or let it go. They cannot
    /// have it while the game hides them. Returns the new state.
    pub fn set_focus(&mut self, owner: u64, on: bool) -> bool {
        self.focus = (on && self.shown).then_some(owner);
        if self.focus.is_none() {
            self.typing = None;
        }
        self.focus.is_some()
    }

    pub fn focused(&self) -> bool {
        self.focus.is_some()
    }

    /// The game takes the mouse back (Esc).
    pub fn release_focus(&mut self) {
        self.focus = None;
        self.typing = None;
    }

    /// The game shows the panels or hides them (its menus open); hidden, they let the mouse go.
    pub fn set_shown(&mut self, shown: bool) {
        self.shown = shown;
        if !shown {
            self.focus = None;
            self.typing = None;
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
    /// its plugin in the plugin's next frame. A checkbox clicked is ticked or unticked, a tab
    /// chosen, a field starts taking the keys: each a change for the plugin.
    pub fn click(&mut self, owner: u64, panel: &str, element: Option<&str>) {
        if let Some(el) = element {
            if self.control_click(owner, panel, el) {
                return;
            }
        }
        if self.clicks.len() >= MAX_CLICKS {
            self.clicks.remove(0);
        }
        self.clicks.push(Click { owner, panel: panel.to_string(), element: element.map(str::to_string) });
    }

    /// A click on a control; false when `el` is none.
    fn control_click(&mut self, owner: u64, panel: &str, el: &str) -> bool {
        let serial = self.next_serial();
        let Some(e) = self.find(owner, panel) else { return false };
        let Some(c) = find_element(&mut e.panel.children, el) else { return false };
        let value = match &mut c.kind {
            Kind::Checkbox { checked, .. } => {
                *checked = !*checked;
                Value::Bool(*checked)
            }
            Kind::Input { .. } => {
                self.typing = Some(Typing { owner, panel: panel.to_string(), element: el.to_string() });
                return true;
            }
            _ => return false,
        };
        e.revision = serial;
        self.push_change(owner, panel, el, value);
        true
    }

    fn push_change(&mut self, owner: u64, panel: &str, element: &str, value: Value) {
        // (a slider dragged: only its latest value waits)
        if let Some(c) = self.changes.iter_mut().find(|c| c.owner == owner && c.panel == panel && c.element == element) {
            c.value = value;
            return;
        }
        if self.changes.len() >= MAX_CLICKS {
            self.changes.remove(0);
        }
        self.changes.push(Change { owner, panel: panel.to_string(), element: element.to_string(), value });
    }

    /// The game sets a control's value (a slider at `fraction` of its range, a tab chosen):
    /// changed, it is a change for the plugin.
    pub fn set_control(&mut self, owner: u64, panel: &str, element: &str, fraction: f32) {
        let serial = self.next_serial();
        let Some(e) = self.find(owner, panel) else { return };
        let Some(c) = find_element(&mut e.panel.children, element) else { return };
        let value = match &mut c.kind {
            Kind::Slider { value, min, max, step } => {
                let mut v = *min + fraction.clamp(0.0, 1.0) * (*max - *min);
                if *step > 0.0 {
                    v = (*min + ((v - *min) / *step).round() * *step).clamp(min.min(*max), max.max(*min));
                }
                if v == *value {
                    return;
                }
                *value = v;
                Value::Num(v as f64)
            }
            Kind::Tabs { tabs, selected } => {
                let n = tabs.len().max(1);
                let i = ((fraction.clamp(0.0, 0.9999) * n as f32) as usize).min(n - 1);
                if i == *selected {
                    return;
                }
                *selected = i;
                Value::Int(i as i64 + 1)
            }
            _ => return,
        };
        e.revision = serial;
        self.push_change(owner, panel, element, value);
    }

    /// The field being typed into, if any.
    pub fn typing(&self) -> Option<&Typing> {
        self.typing.as_ref()
    }

    /// Text typed into the field (`None`: one character rubbed out); a change for its plugin.
    pub fn type_text(&mut self, text: Option<&str>) {
        let Some(t) = self.typing.clone() else { return };
        let serial = self.next_serial();
        let Some(e) = self.find(t.owner, &t.panel) else { return };
        let Some(c) = find_element(&mut e.panel.children, &t.element) else { return };
        let Kind::Input { text: have, max, .. } = &mut c.kind else { return };
        match text {
            Some(s) => {
                for ch in s.chars().filter(|c| !c.is_control()) {
                    if have.chars().count() < *max {
                        have.push(ch);
                    }
                }
            }
            None => {
                have.pop();
            }
        }
        let value = Value::Str(have.clone());
        e.revision = serial;
        self.push_change(t.owner, &t.panel, &t.element, value);
    }

    /// Typing ends (Enter, Esc, a click elsewhere): `submit` sends `ui_submit` for Enter.
    pub fn stop_typing(&mut self, submit: bool) {
        let Some(t) = self.typing.take() else { return };
        if submit {
            if self.clicks.len() >= MAX_CLICKS {
                self.clicks.remove(0);
            }
            // (Enter in a field is a click on it: the plugin takes the text)
            self.clicks.push(Click { owner: t.owner, panel: t.panel, element: Some(t.element) });
        }
    }

    /// A panel dragged by `dx`, `dy` logical pixels.
    pub fn drag(&mut self, owner: u64, id: &str, dx: f32, dy: f32) {
        if let Some(e) = self.find(owner, id) {
            e.moved.0 += dx;
            e.moved.1 += dy;
        }
    }

    /// The clicks waiting for a plugin.
    pub fn take_clicks(&mut self, owner: u64) -> Vec<Click> {
        if self.clicks.is_empty() {
            return Vec::new();
        }
        let (mine, rest) = std::mem::take(&mut self.clicks).into_iter().partition(|c| c.owner == owner);
        self.clicks = rest;
        mine
    }

    /// The changes waiting for a plugin.
    pub fn take_changes(&mut self, owner: u64) -> Vec<Change> {
        if self.changes.is_empty() {
            return Vec::new();
        }
        let (mine, rest) = std::mem::take(&mut self.changes).into_iter().partition(|c| c.owner == owner);
        self.changes = rest;
        mine
    }
}

fn find_element<'a>(list: &'a mut [Element], id: &str) -> Option<&'a mut Element> {
    for e in list.iter_mut() {
        if e.id.as_deref() == Some(id) {
            return Some(e);
        }
        if let Kind::Row { children, .. } = &mut e.kind {
            if let Some(f) = find_element(children, id) {
                return Some(f);
            }
        }
    }
    None
}

fn find_in<'a>(list: &'a [Element], id: &str) -> Option<&'a Element> {
    for e in list {
        if e.id.as_deref() == Some(id) {
            return Some(e);
        }
        if let Kind::Row { children, .. } = &e.kind {
            if let Some(f) = find_in(children, id) {
                return Some(f);
            }
        }
    }
    None
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

// --- the tables --------------------------------------------------------------------------

/// Read a panel table. Unknown keys are left alone and elements of an unknown type skipped,
/// so a plugin written for a later game still shows here; a key of the wrong kind is an
/// error that says where it is. `folder`: the plugin's own, where its images are.
pub fn parse_panel(t: &Value, folder: Option<&Path>) -> Result<Panel, String> {
    let r = Reader { t, path: String::new(), folder };
    if !t.is_table() {
        return Err("the panel is a table".into());
    }
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
        draggable: r.boolean("draggable", false)?,
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
pub fn parse_toast(text: &Value, opts: Option<&Value>) -> Result<ToastSpec, String> {
    let text = match text {
        Value::Str(_) | Value::Int(_) | Value::Num(_) => text.to_text().unwrap_or_default(),
        _ => return Err("the text of a notification is a text".to_string()),
    };
    if text.chars().count() > MAX_TEXT {
        return Err(format!("the text is longer than {MAX_TEXT} characters"));
    }
    let mut spec = ToastSpec { text, title: None, icon: None, color: None, seconds: 5.0 };
    if let Some(t) = opts {
        let r = Reader { t, path: "opts".into(), folder: None };
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
    let t: mlua::Value = lua.load(format!("return {src}")).eval().map_err(|e| e.to_string())?;
    parse_panel(&crate::lua::to_value(&t)?, None)
}

/// Change the keys `fields` gives of an element (`ui.update`).
fn patch(e: &mut Element, fields: &Value, folder: Option<&Path>) -> Result<(), String> {
    let r = Reader { t: fields, path: e.id.clone().unwrap_or_default(), folder };
    if let Some(c) = r.color("color")? {
        e.color = Some(c);
    }
    let visible = r.boolean("visible", true)?;
    let has = |k: &str| fields.get(k).is_some_and(|v| !v.is_nil());
    match &mut e.kind {
        Kind::Text { text, .. } | Kind::Badge { text, .. } | Kind::Button { text, .. } | Kind::Checkbox { text, .. } if has("text") => {
            *text = r.string("text")?.unwrap_or_default();
        }
        Kind::Input { text, .. } if has("text") => *text = r.string("text")?.unwrap_or_default(),
        _ => {}
    }
    match &mut e.kind {
        Kind::Bar { value, .. } if has("value") => *value = r.number("value", 0.0, 0.0, 1.0)?,
        Kind::Slider { value, min, max, .. } if has("value") => *value = r.number("value", *min, min.min(*max), max.max(*min))?,
        Kind::Checkbox { checked, .. } if has("checked") => *checked = r.boolean("checked", false)?,
        Kind::Tabs { selected, tabs } if has("selected") => *selected = (r.number("selected", 1.0, 1.0, tabs.len().max(1) as f32)? as usize).saturating_sub(1),
        Kind::Icon { name, .. } if has("name") => *name = r.string("name")?.unwrap_or_default(),
        Kind::Chart { values, .. } if has("values") => *values = r.numbers("values")?,
        Kind::Table { rows, .. } if has("rows") => *rows = r.rows("rows")?,
        Kind::Image { path, .. } if has("src") => *path = r.image("src")?,
        _ => {}
    }
    // (an element hidden by an update stays in the panel, empty: `visible = true` brings it back)
    if !visible {
        e.kind = match &e.kind {
            Kind::Text { size, weight, align, wrap, .. } => Kind::Text { text: String::new(), size: *size, weight: *weight, align: *align, wrap: *wrap },
            k => k.clone(),
        };
    }
    Ok(())
}

/// Reads the keys of one table; `path` says where it is for the error messages.
struct Reader<'a> {
    t: &'a Value,
    path: String,
    folder: Option<&'a Path>,
}

impl Reader<'_> {
    fn get(&self, key: &str) -> &Value {
        self.t.get(key).unwrap_or(&Value::Nil)
    }

    fn at(&self, key: &str) -> String {
        if self.path.is_empty() {
            key.to_string()
        } else {
            format!("{}.{key}", self.path)
        }
    }

    fn number(&self, key: &str, default: f32, min: f32, max: f32) -> Result<f32, String> {
        let v = match self.get(key) {
            Value::Nil => return Ok(default),
            Value::Int(i) => *i as f64,
            Value::Num(n) if n.is_finite() => *n,
            _ => return Err(format!("{}: a number is expected", self.at(key))),
        };
        Ok((v as f32).clamp(min, max))
    }

    fn opt_number(&self, key: &str) -> Result<Option<f32>, String> {
        match self.get(key) {
            Value::Nil => Ok(None),
            _ => self.number(key, 0.0, f32::MIN, f32::MAX).map(Some),
        }
    }

    fn boolean(&self, key: &str, default: bool) -> Result<bool, String> {
        match self.get(key) {
            Value::Nil => Ok(default),
            Value::Bool(b) => Ok(*b),
            _ => Err(format!("{}: true or false is expected", self.at(key))),
        }
    }

    /// A text (numbers are written as Lua writes them), at most `MAX_TEXT` characters.
    fn string(&self, key: &str) -> Result<Option<String>, String> {
        let s = match self.get(key) {
            Value::Nil => return Ok(None),
            v @ (Value::Str(_) | Value::Int(_) | Value::Num(_)) => v.to_text().unwrap_or_default(),
            _ => return Err(format!("{}: a text is expected", self.at(key))),
        };
        if s.chars().count() > MAX_TEXT {
            return Err(format!("{}: longer than {MAX_TEXT} characters", self.at(key)));
        }
        Ok(Some(s))
    }

    fn color(&self, key: &str) -> Result<Option<Rgba>, String> {
        match self.string(key)? {
            None => Ok(None),
            Some(s) => Rgba::parse(&s).map(Some).ok_or_else(|| format!("{}: \"{s}\" is no colour (\"#RRGGBB\" or \"#RRGGBBAA\")", self.at(key))),
        }
    }

    fn list(&self, key: &str) -> Result<&[Value], String> {
        match self.get(key) {
            Value::Nil => Ok(&[]),
            Value::List(l) => Ok(l),
            Value::Map(m) if m.is_empty() => Ok(&[]),
            _ => Err(format!("{}: a list is expected", self.at(key))),
        }
    }

    fn texts(&self, key: &str) -> Result<Vec<String>, String> {
        let l = self.list(key)?;
        if l.len() > MAX_POINTS {
            return Err(format!("{}: at most {MAX_POINTS} entries", self.at(key)));
        }
        l.iter()
            .enumerate()
            .map(|(i, v)| {
                let s = v.to_text().ok_or_else(|| format!("{}[{}]: a text is expected", self.at(key), i + 1))?;
                Ok(s.chars().take(MAX_TEXT).collect())
            })
            .collect()
    }

    fn numbers(&self, key: &str) -> Result<Vec<f32>, String> {
        let l = self.list(key)?;
        if l.len() > MAX_POINTS {
            return Err(format!("{}: at most {MAX_POINTS} values", self.at(key)));
        }
        l.iter().enumerate().map(|(i, v)| v.as_f64().filter(|n| n.is_finite()).map(|n| n as f32).ok_or_else(|| format!("{}[{}]: a number is expected", self.at(key), i + 1))).collect()
    }

    fn rows(&self, key: &str) -> Result<Vec<Vec<String>>, String> {
        let l = self.list(key)?;
        let mut cells = 0;
        let mut out = Vec::new();
        for (i, row) in l.iter().enumerate() {
            let r = Reader { t: row, path: format!("{}[{}]", self.at(key), i + 1), folder: None };
            let cols: Vec<String> = match row {
                Value::List(c) => c.iter().map(|v| v.to_text().unwrap_or_default().chars().take(MAX_TEXT).collect()).collect(),
                _ => return Err(format!("{}: a row is a list of texts", r.path)),
            };
            cells += cols.len();
            if cells > MAX_POINTS {
                return Err(format!("{}: a table has at most {MAX_POINTS} cells", self.at(key)));
            }
            out.push(cols);
        }
        Ok(out)
    }

    /// An image of the plugin's folder: the file when it is there.
    fn image(&self, key: &str) -> Result<Option<PathBuf>, String> {
        let Some(src) = self.string(key)? else { return Ok(None) };
        let Some(folder) = self.folder else { return Ok(None) };
        let p = crate::api::paths::inside(folder, &src).map_err(|e| format!("{}: {e}", self.at(key)))?;
        Ok(p.is_file().then_some(p))
    }

    fn children(&self, key: &str, depth: usize, count: &mut usize) -> Result<Vec<Element>, String> {
        let list = match self.get(key) {
            Value::Nil => return Ok(Vec::new()),
            Value::List(l) => l.as_slice(),
            Value::Map(m) if m.is_empty() => &[],
            _ => return Err(format!("{}: a list of elements is expected", self.at(key))),
        };
        if depth >= MAX_DEPTH {
            return Err(format!("{}: rows go at most {MAX_DEPTH} deep", self.at(key)));
        }
        let mut out = Vec::new();
        for (i, t) in list.iter().enumerate() {
            let path = format!("{}[{}]", self.at(key), i + 1);
            if !t.is_table() {
                return Err(format!("{path}: an element is a table"));
            }
            *count += 1;
            if *count > MAX_ELEMENTS {
                return Err(format!("a panel has at most {MAX_ELEMENTS} elements"));
            }
            if let Some(e) = (Reader { t, path, folder: self.folder }).element(depth, count)? {
                out.push(e);
            }
        }
        Ok(out)
    }

    /// One element; None when it is hidden or of a type this game does not know.
    fn element(&self, depth: usize, count: &mut usize) -> Result<Option<Element>, String> {
        let ty = self.string("type")?.ok_or_else(|| format!("{}: no type", self.path))?;
        let id = self.string("id")?;
        if let Some(id) = id.as_deref() {
            check_id(&self.at("id"), id)?;
        }
        let text = || self.string("text").map(Option::unwrap_or_default);
        let needs_id = |what: &str| -> Result<(), String> {
            match id {
                Some(_) => Ok(()),
                None => Err(format!("{}: a {what} needs an id", self.path)),
            }
        };
        let kind = match ty.as_str() {
            "text" => Kind::Text {
                text: text()?,
                size: self.number("size", 14.0, 6.0, 96.0)?,
                weight: match self.string("weight")?.as_deref() {
                    None | Some("regular") => Weight::Regular,
                    Some("medium") => Weight::Medium,
                    Some("bold") => Weight::Bold,
                    Some(w) => return Err(format!("{}: \"{w}\" is no weight (regular, medium, bold)", self.at("weight"))),
                },
                align: match self.string("align")?.as_deref() {
                    None | Some("left") => Align::Left,
                    Some("center") => Align::Center,
                    Some("right") => Align::Right,
                    Some(a) => return Err(format!("{}: \"{a}\" is no alignment (left, center, right)", self.at("align"))),
                },
                wrap: self.boolean("wrap", true)?,
            },
            "icon" => Kind::Icon { name: self.string("name")?.unwrap_or_default(), size: self.number("size", 20.0, 6.0, 128.0)? },
            "row" => Kind::Row {
                children: self.children("children", depth + 1, count)?,
                gap: self.number("gap", 8.0, 0.0, 64.0)?,
                align: match self.string("align")?.as_deref() {
                    None | Some("start") => RowAlign::Start,
                    Some("center") => RowAlign::Center,
                    Some("end") => RowAlign::End,
                    Some("between") => RowAlign::Between,
                    Some(a) => return Err(format!("{}: \"{a}\" is no alignment (start, center, end, between)", self.at("align"))),
                },
            },
            "bar" => Kind::Bar { value: self.number("value", 0.0, 0.0, 1.0)?, height: self.number("height", 6.0, 1.0, 64.0)?, background: self.color("background")? },
            "badge" => Kind::Badge { text: text()?, text_color: self.color("text_color")? },
            "divider" => Kind::Divider,
            "space" => Kind::Space { size: self.number("size", 8.0, 0.0, 1000.0)? },
            "button" => {
                needs_id("button")?;
                Kind::Button { text: text()?, icon: self.string("icon")? }
            }
            "image" => Kind::Image { path: self.image("src")?, width: self.number("width", 64.0, 1.0, 4000.0)?, height: self.number("height", 64.0, 1.0, 4000.0)? },
            "checkbox" => {
                needs_id("checkbox")?;
                Kind::Checkbox { text: text()?, checked: self.boolean("checked", false)? }
            }
            "slider" => {
                needs_id("slider")?;
                let min = self.number("min", 0.0, -1e9, 1e9)?;
                let max = self.number("max", 1.0, -1e9, 1e9)?;
                Kind::Slider { value: self.number("value", min, min.min(max), max.max(min))?, min, max, step: self.number("step", 0.0, 0.0, 1e9)? }
            }
            "input" => {
                needs_id("text field")?;
                let max = self.number("max", 100.0, 1.0, MAX_TEXT as f32)? as usize;
                Kind::Input { text: text()?.chars().take(max).collect(), placeholder: self.string("placeholder")?.unwrap_or_default(), max }
            }
            "tabs" => {
                needs_id("tabs element")?;
                let tabs = self.texts("tabs")?;
                let n = tabs.len().max(1) as f32;
                Kind::Tabs { selected: (self.number("selected", 1.0, 1.0, n)? as usize).saturating_sub(1), tabs }
            }
            "chart" => Kind::Chart {
                values: self.numbers("values")?,
                min: self.opt_number("min")?,
                max: self.opt_number("max")?,
                height: self.number("height", 48.0, 8.0, 1000.0)?,
                style: match self.string("style")?.as_deref() {
                    None | Some("line") => ChartStyle::Line,
                    Some("bars") => ChartStyle::Bars,
                    Some(s) => return Err(format!("{}: \"{s}\" is no chart style (line, bars)", self.at("style"))),
                },
                fill: self.boolean("fill", false)?,
            },
            "table" => Kind::Table { columns: self.texts("columns")?, rows: self.rows("rows")?, widths: self.numbers("widths")?, size: self.number("size", 13.0, 6.0, 48.0)? },
            // (an element of a later version of the API: left out)
            _ => return Ok(None),
        };
        let e = Element { id, color: self.color("color")?, grow: self.boolean("grow", false)?, clickable: self.boolean("clickable", false)?, kind };
        Ok(self.boolean("visible", true)?.then_some(e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel(src: &str) -> Result<Panel, String> {
        panel_from_source(src)
    }

    #[test]
    fn colours() {
        assert_eq!(Rgba::parse("#F47F30"), Some(Rgba([0xF4, 0x7F, 0x30, 255])));
        assert_eq!(Rgba::parse("#14161acc"), Some(Rgba([0x14, 0x16, 0x1A, 0xCC])));
        for bad in ["F47F30", "#F47F3", "#GGGGGG", "#F47F30C", "#ÄÄÄ"] {
            assert_eq!(Rgba::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_panel_is_read_with_its_defaults() {
        let p = panel(
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
        assert_eq!((p.x, p.y, p.width, p.padding, p.gap, p.radius), (16.0, 0.0, 340.0, 12.0, 6.0, 12.0));
        assert_eq!(p.accent, Some(Rgba([0xF4, 0x7F, 0x30, 255])));
        assert!(p.visible && !p.clickable && p.background.is_none());
        // (the hidden text and the unknown type are left out)
        assert_eq!(p.children.len(), 4);
        assert_eq!(p.children[0].kind, Kind::Text { text: "Linie 42".into(), size: 18.0, weight: Weight::Bold, align: Align::Left, wrap: true });
        let Kind::Row { children, gap, align } = &p.children[1].kind else { panic!("{:?}", p.children[1]) };
        assert_eq!((*gap, *align, children.len()), (8.0, RowAlign::Between, 3));
        assert!(children[1].grow);
        assert_eq!(children[1].kind, Kind::Text { text: "12.5".into(), size: 14.0, weight: Weight::Regular, align: Align::Left, wrap: true });
        // (held to its range)
        assert_eq!(p.children[2].kind, Kind::Bar { value: 1.0, height: 6.0, background: None });
        assert!(p.children[3].takes_clicks() && !p.children[0].takes_clicks());
    }

    #[test]
    fn mistakes_say_where_they_are() {
        let cases = [
            (r#"{ width = "wide" }"#, "width: a number is expected"),
            (r#"{ anchor = "middle" }"#, "anchor: \"middle\" is no anchor"),
            (r#"{ background = "red" }"#, "background: \"red\" is no colour"),
            (r#"{ children = { { type = "button", text = "Go" } } }"#, "children[1]: a button needs an id"),
            (r#"{ children = { { type = "row", children = { { type = "text", size = {} } } } } }"#, "children[1].children[1].size: a number is expected"),
            (r#"{ children = { { text = "no type" } } }"#, "children[1]: no type"),
            (r#"{ children = { "text" } }"#, "children[1]: an element is a table"),
            (r#"{ children = { { type = "text", text = string.rep("x", 501) } } }"#, "children[1].text: longer than 500 characters"),
            (r#"{ children = { { type = "text", weight = "heavy" } } }"#, "is no weight"),
            (r#"{ children = { { type = "slider", value = 2 } } }"#, "a slider needs an id"),
            (r#"{ children = { { type = "chart", values = { 1, "x" } } } }"#, "children[1].values[2]: a number is expected"),
        ];
        for (src, want) in cases {
            let e = panel(src).unwrap_err();
            assert!(e.contains(want), "{src}: {e}");
        }
        // too many elements, rows counted with what is in them
        let e = panel("(function() local c = {} for i = 1, 101 do c[i] = { type = 'row', children = { { type = 'space' } } } end return { children = c } end)()").unwrap_err();
        assert!(e.contains("at most 200 elements"), "{e}");
        // a row holding itself ends at the depth limit
        let e = panel("(function() local r = { type = 'row' } r.children = { r } return { children = { r } } end)()").unwrap_err();
        assert!(e.contains("deep"), "{e}");
    }

    #[test]
    fn panels_belong_to_their_plugin() {
        let p = panel("{}").unwrap();
        let mut ui = UiState::default();
        for i in 0..MAX_PANELS {
            ui.set(1, &format!("p{i}"), p.clone()).unwrap();
        }
        assert!(ui.set(1, "one too many", p.clone()).unwrap_err().contains("at most 16 panels"));
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
        let mut ui = UiState::default();
        ui.set(1, "trip", panel(r#"{ children = { { type = "text", text = "a" } } }"#).unwrap()).unwrap();
        let first = ui.panels()[0].revision;
        ui.set(1, "trip", panel(r#"{ children = { { type = "text", text = "a" } } }"#).unwrap()).unwrap();
        assert_eq!(ui.panels()[0].revision, first);
        ui.set(1, "trip", panel(r#"{ children = { { type = "text", text = "b" } } }"#).unwrap()).unwrap();
        assert_ne!(ui.panels()[0].revision, first);
    }

    #[test]
    fn toasts_go_by_themselves_and_make_room() {
        let mut ui = UiState::default();
        for i in 0..MAX_TOASTS + 2 {
            ui.toast(1, parse_toast(&Value::Str(format!("t{i}")), None).unwrap());
        }
        let opts = Value::map([("seconds", 1i64.into()), ("title", "Pay".into()), ("icon", "payments".into()), ("color", "#2E7D32".into())]);
        let spec = parse_toast(&Value::Int(42), Some(&opts)).unwrap();
        assert_eq!(spec, ToastSpec { text: "42".into(), title: Some("Pay".into()), icon: Some("payments".into()), color: Some(Rgba([0x2E, 0x7D, 0x32, 255])), seconds: 1.0 });
        ui.toast(2, spec);
        let bad = Value::map([("seconds", "long".into())]);
        assert_eq!(parse_toast(&Value::Int(1), Some(&bad)).unwrap_err(), "opts.seconds: a number is expected");
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
        ui.set(7, "menu", Panel { anchor: Anchor::Center, width: 200.0, clickable: true, ..Panel::default() }).unwrap();
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
        assert_eq!(ui.take_clicks(7), [Click { owner: 7, panel: "menu".into(), element: Some("ok".into()) }]);
        assert!(ui.take_clicks(7).is_empty());
        // another plugin stops: the mouse stays; the one that asked for it stops: it goes
        ui.remove_owner(8);
        assert!(ui.focused());
        ui.click(8, "x", None);
        ui.remove_owner(7);
        assert!(ui.panels().is_empty() && !ui.focused());
        assert_eq!(ui.take_clicks(8).len(), 1);
    }

    #[test]
    fn controls_change_and_tell_their_plugin() {
        let mut ui = UiState::default();
        let p = panel(
            r#"{ children = {
                { type = "checkbox", id = "rain", text = "Rain" },
                { type = "row", children = { { type = "slider", id = "vol", min = 0, max = 10, step = 1, value = 3 } } },
                { type = "input", id = "name", max = 5 },
                { type = "tabs", id = "tab", tabs = { "A", "B", "C" } },
                { type = "chart", values = { 1, 2, 3 }, style = "bars" },
                { type = "table", columns = { "Stop", "Time" }, rows = { { "Zoo", "12:01" } } },
            } }"#,
        )
        .unwrap();
        ui.set(1, "p", p).unwrap();
        ui.click(1, "p", Some("rain"));
        ui.set_control(1, "p", "vol", 0.72);
        ui.set_control(1, "p", "tab", 0.5);
        ui.click(1, "p", Some("name"));
        assert_eq!(ui.typing().map(|t| t.element.as_str()), Some("name"));
        ui.type_text(Some("Hello world"));
        ui.type_text(None);
        ui.stop_typing(true);
        let changes: Vec<(String, Value)> = ui.take_changes(1).into_iter().map(|c| (c.element, c.value)).collect();
        assert_eq!(changes, [("rain".to_string(), Value::Bool(true)), ("vol".into(), Value::Num(7.0)), ("tab".into(), Value::Int(2)), ("name".into(), Value::Str("Hell".into()))]);
        // (Enter in the field is a click on it; the checkbox's click was its change)
        assert_eq!(ui.take_clicks(1).len(), 1);
        assert_eq!(ui.value(1, "p", "vol"), Some(Value::Num(7.0)));
        ui.update(1, "p", "rain", &Value::map([("checked", false.into())]), None).unwrap();
        assert_eq!(ui.value(1, "p", "rain"), Some(Value::Bool(false)));
        assert!(ui.update(1, "p", "nothing", &Value::map([]), None).is_err());
        ui.drag(1, "p", 5.0, -2.0);
        assert_eq!(ui.get(1, "p").unwrap().moved, (5.0, -2.0));
    }
}
