//! The plugin itself: its name and files, messages to the other plugins, and its settings
//! with the panel the game makes for them.

use super::{def, o, p};
use crate::api::runtime::{self, Mail};
use crate::api::{paths, ApiError, ApiFn, CallError, Ctx, Perm, PluginState, Value};
use crate::ui::{self, Element, Kind, Panel, Rgba};

const NEW: &str = crate::api::VERSION;
/// The settings panel's id (a plugin's own ids cannot be told from it, so they are kept off
/// it: see `ui.set`... a plugin naming a panel so replaces it, which does no harm).
pub const SETTINGS_PANEL: &str = "__settings";
/// The settings' file in the data folder.
const SETTINGS_FILE: &str = "settings.json";
/// Most settings of a plugin.
const MAX_SETTINGS: usize = 40;

/// One setting.
#[derive(Debug, Clone, PartialEq)]
pub struct Setting {
    pub key: String,
    pub label: String,
    pub kind: SettingKind,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SettingKind {
    Bool,
    Number { min: f32, max: f32, step: f32 },
    Text,
    Choice(Vec<String>),
}

/// The settings a plugin declared (`plugin.settings`).
#[derive(Debug, Default)]
pub struct Settings {
    pub list: Vec<Setting>,
    pub title: String,
}

impl Settings {
    fn get(&self, key: &str) -> Option<&Setting> {
        self.list.iter().find(|s| s.key == key)
    }

    fn values(&self) -> Value {
        Value::Map(self.list.iter().map(|s| (s.key.clone(), s.value.clone())).collect())
    }
}

/// A value held to what the setting takes (None: not one of it).
fn fit(kind: &SettingKind, v: &Value) -> Option<Value> {
    match kind {
        SettingKind::Bool => match v {
            Value::Bool(b) => Some(Value::Bool(*b)),
            _ => None,
        },
        SettingKind::Number { min, max, step } => v.as_f64().map(|n| {
            let mut n = n.clamp(*min as f64, *max as f64);
            if *step > 0.0 {
                n = *min as f64 + ((n - *min as f64) / *step as f64).round() * *step as f64;
            }
            Value::Num(n)
        }),
        SettingKind::Text => v.to_text().map(|t| Value::Str(t.chars().take(ui::MAX_TEXT).collect())),
        SettingKind::Choice(c) => v.to_text().filter(|t| c.contains(t)).map(Value::Str),
    }
}

/// Read `plugin.settings`'s list.
fn parse(spec: &Value) -> Result<Vec<Setting>, String> {
    let list = spec.items();
    if list.len() > MAX_SETTINGS {
        return Err(format!("at most {MAX_SETTINGS} settings"));
    }
    let mut out: Vec<Setting> = Vec::new();
    for (i, s) in list.iter().enumerate() {
        let at = |k: &str| format!("settings[{}].{k}", i + 1);
        let text = |k: &str| s.get(k).and_then(Value::to_text);
        let key = text("key").ok_or_else(|| at("key") + ": a text is needed")?;
        if out.iter().any(|o| o.key == key) {
            return Err(format!("{}: \"{key}\" is there twice", at("key")));
        }
        let num = |k: &str, d: f32| s.get(k).and_then(Value::as_f64).map_or(d, |n| n as f32);
        let kind = match text("type").as_deref() {
            Some("bool") => SettingKind::Bool,
            Some("number") => SettingKind::Number { min: num("min", 0.0), max: num("max", 100.0), step: num("step", 0.0) },
            Some("text") => SettingKind::Text,
            Some("choice") => {
                let c: Vec<String> = s.get("choices").map(|c| c.items().iter().filter_map(Value::to_text).collect()).unwrap_or_default();
                if c.is_empty() {
                    return Err(format!("{}: a choice needs its choices", at("choices")));
                }
                SettingKind::Choice(c)
            }
            other => return Err(format!("{}: {other:?} is no setting type (bool, number, text, choice)", at("type"))),
        };
        let default = s.get("default").cloned().unwrap_or(Value::Nil);
        let value = fit(&kind, &default).unwrap_or(match &kind {
            SettingKind::Bool => Value::Bool(false),
            SettingKind::Number { min, .. } => Value::Num(*min as f64),
            SettingKind::Text => Value::Str(String::new()),
            SettingKind::Choice(c) => Value::Str(c[0].clone()),
        });
        let label = text("label").unwrap_or_else(|| key.clone());
        out.push(Setting { key, label, kind, value });
    }
    Ok(out)
}

fn load_saved(s: &PluginState) -> Vec<(String, Value)> {
    match std::fs::read_to_string(s.data_dir.join(SETTINGS_FILE)).ok().map(|t| crate::api::json::decode(&t, false)) {
        Some(Ok(Value::Map(m))) => m,
        _ => Vec::new(),
    }
}

fn save(s: &PluginState) {
    let text = crate::api::json::encode_pretty(&s.settings.values());
    if let Err(e) = std::fs::create_dir_all(&s.data_dir).and_then(|_| std::fs::write(s.data_dir.join(SETTINGS_FILE), text)) {
        log::warn!("{} settings not saved: {e}", s.tag);
    }
}

/// The settings panel: one control per setting, and a button that closes it.
fn panel(s: &Settings) -> Panel {
    let el = |id: Option<String>, kind: Kind| Element { id, color: None, grow: false, clickable: false, kind };
    let text = |t: &str, size: f32, weight: ui::Weight| Kind::Text { text: t.to_string(), size, weight, align: ui::Align::Left, wrap: true };
    let mut children = vec![el(None, Kind::Row {
        children: vec![
            Element { grow: true, ..el(None, text(&s.title, 16.0, ui::Weight::Bold)) },
            el(Some("__close".into()), Kind::Button { text: String::new(), icon: Some("close".into()) }),
        ],
        gap: 8.0,
        align: ui::RowAlign::Start,
    })];
    for st in &s.list {
        let id = Some(st.key.clone());
        match &st.kind {
            SettingKind::Bool => children.push(el(id, Kind::Checkbox { text: st.label.clone(), checked: st.value.truthy() })),
            SettingKind::Number { min, max, step } => {
                let v = st.value.as_f64().unwrap_or(*min as f64);
                children.push(el(None, Kind::Row {
                    children: vec![Element { grow: true, ..el(None, text(&st.label, 13.0, ui::Weight::Regular)) }, el(None, Kind::Badge { text: crate::api::value::lua_number(v).trim_end_matches(".0").to_string(), text_color: None })],
                    gap: 8.0,
                    align: ui::RowAlign::Start,
                }));
                children.push(el(id, Kind::Slider { value: v as f32, min: *min, max: *max, step: *step }));
            }
            SettingKind::Text => {
                children.push(el(None, text(&st.label, 13.0, ui::Weight::Regular)));
                children.push(el(id, Kind::Input { text: st.value.to_text().unwrap_or_default(), placeholder: String::new(), max: 200 }));
            }
            SettingKind::Choice(c) => {
                children.push(el(None, text(&st.label, 13.0, ui::Weight::Regular)));
                let sel = c.iter().position(|x| Some(x.as_str()) == st.value.as_str()).unwrap_or(0);
                children.push(el(id, Kind::Tabs { tabs: c.clone(), selected: sel }));
            }
        }
    }
    Panel { anchor: ui::Anchor::Center, width: 360.0, accent: Some(Rgba([232, 160, 48, 255])), draggable: true, children, ..Panel::default() }
}

fn show(s: &mut PluginState, on: bool) {
    let owner = s.owner;
    if on {
        let p = panel(&s.settings);
        let _ = s.ui().set(owner, SETTINGS_PANEL, p);
    } else {
        s.ui().remove(owner, SETTINGS_PANEL);
    }
}

/// A control of the settings panel changed (from the runtime's frame).
pub fn setting_changed(ctx: &mut Ctx<'_>, key: &str, value: Value) -> Result<(), CallError> {
    let s = ctx.state();
    let Some(st) = s.settings.list.iter_mut().find(|x| x.key == key) else { return Ok(()) };
    let value = match &st.kind {
        SettingKind::Choice(c) => value.as_i64().and_then(|i| c.get((i - 1).max(0) as usize)).map(|t| Value::Str(t.clone())),
        k => fit(k, &value),
    };
    let Some(value) = value.filter(|v| *v != st.value) else { return Ok(()) };
    st.value = value.clone();
    let number = matches!(st.kind, SettingKind::Number { .. });
    save(s);
    // (the number beside a slider follows it)
    if number {
        show(s, true);
    }
    runtime::dispatch(ctx, "setting", vec![Value::Str(key.to_string()), value])
}

/// A click on the settings panel: its close button.
pub fn settings_click(ctx: &mut Ctx<'_>, element: Option<&str>) {
    if element == Some("__close") {
        show(ctx.state(), false);
    }
}

fn mail(c: &mut Ctx<'_>, to: Option<String>, topic: String, data: Value) -> Result<(), ApiError> {
    if matches!(data, Value::Callback(_)) {
        return Err(ApiError("a function cannot be sent".into()));
    }
    let s = c.state();
    s.hub.post(Mail { from: s.name.clone(), to, topic, data });
    Ok(())
}

pub static FNS: &[ApiFn] = &[
    def!("plugin.name", "plugin", [], "string", "The plugin's name: its file's, or its folder's for a `main.lua`.", NEW, None, false, |c, a| c.state().name.clone()),
    def!("plugin.list", "plugin", [], "list of strings", "The names of the plugins loaded, this one too.", NEW, None, false, |c, a| c.state().hub.names.borrow().clone()),
    def!("plugin.send", "plugin", [p("to", "string"), p("topic", "string"), o("data", "any")], "nil", "Sends a message to another plugin (by its name): it hears `message(from, topic, data)` in its next frame. The data is numbers, texts, booleans or tables of them.", NEW, None, false, |c, a| {
        let (to, topic) = (a.str(0)?, a.str(1)?);
        let data = a.take(2);
        mail(c, Some(to), topic, data)
    }),
    def!("plugin.broadcast", "plugin", [p("topic", "string"), o("data", "any")], "nil", "Sends a message to every other plugin loaded.", NEW, None, false, |c, a| {
        let topic = a.str(0)?;
        let data = a.take(1);
        mail(c, None, topic, data)
    }),
    def!("plugin.read", "plugin", [p("path", "string")], "text, or nil and the reason", "A file of the plugin's own folder (a table of stops, a translation): read only, relative to the folder.", NEW, None, true, |c, a| {
        let rel = a.str(0)?;
        let r = c.state().folder.clone().ok_or_else(|| "the plugin has no folder of files".to_string()).and_then(|f| paths::inside(&f, &rel)).and_then(|p| std::fs::read_to_string(p).map_err(|e| e.to_string()));
        Ok::<_, ApiError>(match r {
            Ok(t) => Value::List(vec![Value::Str(t)]),
            Err(e) => Value::List(vec![Value::Nil, Value::Str(e)]),
        })
    }),
    def!("plugin.files", "plugin", [o("dir", "string")], "list of tables", "The files of the plugin's own folder (or a folder in it): `{name, dir, size}`.", NEW, None, false, |c, a| {
        let rel = a.opt_str(0)?.unwrap_or_default();
        let r = c.state().folder.clone().ok_or_else(|| "no folder".to_string()).and_then(|f| paths::inside(&f, &rel)).and_then(|p| super::storage::list_dir(&p));
        Ok::<_, ApiError>(r.unwrap_or(Value::List(Vec::new())))
    }),
    def!("plugin.permissions", "plugin", [], "list of strings", "The permissions the plugin has (a plain `.lua` file has them all).", NEW, None, false, |c, a| {
        let s = c.state();
        Perm::ALL.iter().filter(|p| s.allows(**p)).map(|p| p.as_str().to_string()).collect::<Vec<_>>()
    }),
    def!("plugin.errors", "plugin", [], "integer", "How many errors the plugin had (at 10 it is switched off).", NEW, None, false, |c, a| c.state().errors as i64),
    def!("plugin.disable", "plugin", [o("reason", "string")], "nil", "Switches the plugin off until its file changes or the game starts again (its `stop` comes).", NEW, None, false, |c, a| {
        let why = a.opt_str(0)?.unwrap_or_else(|| "it asked to".into());
        let s = c.state();
        log::info!("{} switched off: {why}", s.tag);
        s.disabled = true;
        let owner = s.owner;
        s.ui().remove_owner(owner);
        Ok::<_, ApiError>(())
    }),
    def!("plugin.settings", "plugin", [p("settings", "table"), o("title", "string")], "table", "Declares the plugin's settings: a list of `{key, type, label, default}` with `type` `\"bool\"`, `\"number\"` (`min`, `max`, `step`), `\"text\"` or `\"choice\"` (`choices`, a list of texts). The values the player chose before come back (as a table key -> value); the game makes a settings panel of them (`plugin.show_settings`), saves them in the data folder and sends `setting(key, value)` when one changes.", NEW, None, false, |c, a| {
        let list = parse(a.table(0)?).map_err(ApiError)?;
        let title = a.opt_str(1)?;
        let s = c.state();
        let saved = load_saved(s);
        s.settings.title = title.unwrap_or_else(|| s.name.clone());
        s.settings.list = list;
        for st in s.settings.list.iter_mut() {
            if let Some(v) = saved.iter().find(|(k, _)| *k == st.key).and_then(|(_, v)| fit(&st.kind, v)) {
                st.value = v;
            }
        }
        Ok::<_, ApiError>(s.settings.values())
    }),
    def!("plugin.setting", "plugin", [p("key", "string")], "any", "A setting's value (`nil`: no such setting).", NEW, None, false, |c, a| {
        let k = a.str(0)?;
        Ok::<_, ApiError>(c.state().settings.get(&k).map(|s| s.value.clone()))
    }),
    def!("plugin.set_setting", "plugin", [p("key", "string"), p("value", "any")], "boolean", "Changes a setting (held to its range; saved); `true` when it took the value.", NEW, None, false, |c, a| {
        let k = a.str(0)?;
        let s = c.state();
        let Some(st) = s.settings.list.iter_mut().find(|x| x.key == k) else { return Ok(false.into()) };
        let Some(v) = fit(&st.kind, a.get(1)) else { return Ok(false.into()) };
        st.value = v;
        save(s);
        if s.ui().get(s.owner, SETTINGS_PANEL).is_some() {
            show(s, true);
        }
        Ok::<_, ApiError>(true)
    }),
    def!("plugin.show_settings", "plugin", [o("on", "bool")], "boolean", "Shows the plugin's settings panel (or hides it with `false`); it can be dragged and closed while the panels have the mouse (`ui.focus`). `false` when the plugin declared no settings.", NEW, Ui, false, |c, a| {
        let on = a.flag(0, true);
        let s = c.state();
        if s.settings.list.is_empty() {
            return Ok(false.into());
        }
        show(s, on);
        Ok::<_, ApiError>(true)
    }),
];
