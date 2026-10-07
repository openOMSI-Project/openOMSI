//! The bus step's bus options: the `[setvar]` variables of the chosen bus's liveries - its
//! mirrors, rims, seats, door types, the displays a repaint brings - offered to choose, after
//! Omsi-Hub's paint studio ("busopties": `lakfamilie.ts` `lakOpties`, `busrust.ts`
//! `typischVan`). Each is offered with the values the liveries give it; "As the livery" leaves
//! it to the livery chosen, as the game has it without the option. A value chosen goes to the
//! game as `--setvar` after the paint, is shown on the bus in the showroom at once, and is
//! kept for the bus file (`omsi_launcher_lib::busoptions`).
//!
//! An option is "appearance" when no script of the bus reads it and a mesh's `[visible]` or a
//! material's `[matl_change]` does: it only changes what is seen. Every other one is
//! "technical" - a script works with it (the NLC's `setvar.osc` takes its `vis_CTI_*` over
//! into its own, the Kajosoft O530's sets its displays and validators up from them in
//! `{init}`) - and waits behind a fold, with a word that it changes how the bus works. A
//! script whose file name says "visual" (the SD200's `visual.osc` only draws its grille or
//! not) is no reason. A variable nothing reads counts as technical too: who knows.
//!
//! What a livery does not set, the bus has as its scripts make it; what most liveries give it
//! (a livery that does not set it counting as 0, as the game starts it) is said beside "As the
//! livery" - the bus's usual look, as Omsi-Hub shows it.

use super::theme::*;
use super::ui::{id_of, ButtonKind, Feel, Ui};
use glam::Vec2;
use omsi_launcher_lib::busoptions::{value_text, BusOptions};
use omsi_ui::paint::Align;
use omsi_ui::{tr, Rect, Weight};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

// --- what a bus offers ----------------------------------------------------------------------

/// A livery as the options see it: its name and what its `[setvar]` lines set, in order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Livery {
    pub name: String,
    pub vars: Vec<(String, f32)>,
}

impl Livery {
    /// The value it gives `var` (its last line for it: the game sets them in order).
    pub fn value(&self, var: &str) -> Option<f32> {
        self.vars.iter().rev().find(|(n, _)| n.trim().eq_ignore_ascii_case(var.trim())).map(|(_, v)| *v)
    }
}

/// What an option changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// Only what is seen: a mesh shown or not, a material swapped.
    Appearance,
    /// What a script of the bus works with.
    Technical,
}

/// A value an option takes, and the liveries that give it.
#[derive(Clone, Debug, PartialEq)]
pub struct Seen {
    pub value: f32,
    pub liveries: Vec<String>,
}

/// One bus option: a variable the liveries set.
#[derive(Clone, Debug, PartialEq)]
pub struct BusOption {
    /// As the first livery to set it spells it.
    pub var: String,
    pub label: String,
    pub kind: Kind,
    /// Every value the liveries give it, the smallest first.
    pub values: Vec<Seen>,
    /// What most liveries give it (see the module's head).
    pub usual: f32,
}

impl BusOption {
    /// Seen only as 0 and 1: off or on.
    pub fn is_switch(&self) -> bool {
        self.values.iter().all(|s| s.value == 0.0 || s.value == 1.0)
    }

    /// What `livery` sets it to (None: it does not set it, or the model's own textures).
    pub fn as_livery(&self, livery: Option<&Livery>) -> Option<f32> {
        livery.and_then(|l| l.value(&self.var))
    }
}

/// What a bus offers: its liveries (as the game lists them) and the options they make.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Catalogue {
    pub liveries: Vec<Livery>,
    pub options: Vec<BusOption>,
}

impl Catalogue {
    /// The livery of that name (the first of it, as the game takes it); none for the model's
    /// own textures ("").
    pub fn livery(&self, name: &str) -> Option<&Livery> {
        let n = name.trim();
        if n.is_empty() {
            return None;
        }
        self.liveries.iter().find(|l| l.name.trim().eq_ignore_ascii_case(n))
    }
}

/// What the bus's scripts and model do with variables.
#[derive(Clone, Debug, Default)]
pub struct Usage {
    /// The scripts: file name and text, in lower case.
    pub scripts: Vec<(String, String)>,
    /// The variables a mesh's `[visible]` or a material's `[matl_change]` reads (lower case).
    pub shown: HashSet<String>,
}

/// A variable's name as a person reads it: the prefixes that only say it is a setvar (`vis_`,
/// `CTI_`, `SV_`) off, the words apart where the underscores and the capitals put them, in
/// sentence case - an abbreviation in capitals stays so, as does a lone letter (the NLC's
/// `Rad_V`, front).
pub fn label_of(var: &str) -> String {
    let mut rest = var.trim();
    loop {
        let lower = rest.to_ascii_lowercase();
        match ["vis_", "cti_", "sv_", "setvar_"].iter().find(|p| lower.starts_with(**p) && rest.len() > p.len()) {
            Some(p) => rest = &rest[p.len()..],
            None => break,
        }
    }
    let words = split_words(rest);
    if words.is_empty() {
        return var.trim().to_string();
    }
    let mut out: Vec<String> = Vec::with_capacity(words.len());
    for (i, w) in words.iter().enumerate() {
        let letters = w.chars().filter(|c| c.is_alphabetic()).count();
        let digits = w.chars().filter(|c| c.is_ascii_digit()).count();
        let caps = letters > 0 && w.chars().filter(|c| c.is_alphabetic()).all(|c| c.is_uppercase());
        let word = if (caps && letters > 1) || (letters == 1 && w.chars().count() == 1) || (digits > 0 && letters > 0 && w.chars().count() <= 3) {
            // an abbreviation, a lone letter, a short code (H2, E5)
            w.to_uppercase()
        } else if i == 0 {
            let lower = w.to_lowercase();
            let mut c = lower.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
        } else {
            w.to_lowercase()
        };
        out.push(word);
    }
    out.join(" ")
}

/// The words of a name: apart at `_`, `-`, spaces and dots, before a capital after a small
/// letter, between letters and digits, and before the last capital of a run followed by a
/// small letter ("LEDColor": LED, Color).
fn split_words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    for piece in s.split(['_', '-', ' ', '.']).filter(|p| !p.is_empty()) {
        let cs: Vec<char> = piece.chars().collect();
        let mut cur = String::new();
        for (i, &c) in cs.iter().enumerate() {
            if i > 0 {
                let p = cs[i - 1];
                let next_small = cs.get(i + 1).is_some_and(|n| n.is_lowercase());
                let apart = (p.is_lowercase() && c.is_uppercase()) || (p.is_alphabetic() && c.is_ascii_digit() && cur.chars().count() > 1) || (p.is_ascii_digit() && c.is_alphabetic()) || (p.is_uppercase() && c.is_uppercase() && next_small);
                if apart && !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            cur.push(c);
        }
        if !cur.is_empty() {
            out.push(cur);
        }
    }
    out
}

/// Whether a script's text (lower case) works with `var` (lower case): `(L.L.var)` reads it,
/// `(S.L.var)` writes it.
fn mentioned(text: &str, var: &str) -> bool {
    let needle = format!(".l.{var}");
    let mut from = 0;
    while let Some(i) = text[from..].find(&needle) {
        let end = from + i + needle.len();
        if !text[end..].chars().next().is_some_and(|c| c.is_alphanumeric() || c == '_') {
            return true;
        }
        from = end;
    }
    false
}

/// Appearance or technical (see the module's head).
pub fn kind_of(var: &str, usage: &Usage) -> Kind {
    let v = var.trim().to_ascii_lowercase();
    let scripted = usage.scripts.iter().any(|(name, text)| !name.contains("visual") && mentioned(text, &v));
    if !scripted && usage.shown.contains(&v) {
        Kind::Appearance
    } else {
        Kind::Technical
    }
}

/// What most liveries give `var`, one that does not set it counting as 0; between as many,
/// the value of the first livery giving one of them (Omsi-Hub's `typischVan`).
pub fn usual_of(liveries: &[Livery], var: &str) -> f32 {
    let given: Vec<f32> = liveries.iter().map(|l| l.value(var).unwrap_or(0.0)).map(|v| if v == 0.0 { 0.0 } else { v }).collect();
    let mut count: HashMap<u32, usize> = HashMap::new();
    for v in &given {
        *count.entry(v.to_bits()).or_default() += 1;
    }
    let most = count.values().copied().max().unwrap_or(0);
    given.iter().copied().find(|v| count.get(&v.to_bits()) == Some(&most)).unwrap_or(0.0)
}

/// The options `liveries` make: every variable one of them sets (`Colorscheme` aside - the
/// game sets that from the livery itself), with its values and the liveries giving each. The
/// appearance ones first, each kind by its label.
pub fn gather(liveries: &[Livery], usage: &Usage) -> Vec<BusOption> {
    let mut order: Vec<String> = Vec::new();
    let mut seen: HashMap<String, Vec<Seen>> = HashMap::new();
    for l in liveries {
        // (what it sets each variable to in the end)
        let mut own: Vec<(String, f32)> = Vec::new();
        for (n, v) in &l.vars {
            let n = n.trim();
            if n.is_empty() || n.eq_ignore_ascii_case("Colorscheme") || !v.is_finite() {
                continue;
            }
            match own.iter_mut().find(|(o, _)| o.eq_ignore_ascii_case(n)) {
                Some(o) => o.1 = *v,
                None => own.push((n.to_string(), *v)),
            }
        }
        for (n, v) in own {
            let list = seen.entry(n.to_ascii_lowercase()).or_insert_with(|| {
                order.push(n.clone());
                Vec::new()
            });
            match list.iter_mut().find(|s| s.value == v) {
                Some(s) => {
                    if !s.liveries.iter().any(|x| x.eq_ignore_ascii_case(&l.name)) {
                        s.liveries.push(l.name.clone());
                    }
                }
                None => list.push(Seen { value: v, liveries: vec![l.name.clone()] }),
            }
        }
    }
    let mut out: Vec<BusOption> = order
        .into_iter()
        .map(|var| {
            let mut values = seen.remove(&var.to_ascii_lowercase()).unwrap_or_default();
            values.sort_by(|a, b| a.value.total_cmp(&b.value));
            BusOption { label: label_of(&var), kind: kind_of(&var, usage), usual: usual_of(liveries, &var), values, var }
        })
        .collect();
    out.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| super::buspick::name_cmp(&a.label, &b.label)).then_with(|| a.var.to_ascii_lowercase().cmp(&b.var.to_ascii_lowercase())));
    out
}

/// The value chosen for `var` (the spelling does not matter).
fn pick(picks: &BTreeMap<String, f32>, var: &str) -> Option<f32> {
    picks.iter().find(|(k, _)| k.eq_ignore_ascii_case(var)).map(|(_, v)| *v)
}

fn same(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

/// What the game gets for the bus (`--setvar`): every value chosen that the livery `livery`
/// does not set to the same itself. Before the bus was read (`cat` None) every value chosen.
/// A choice for a variable the bus no longer has is left out.
pub fn setvars(cat: Option<&Catalogue>, livery: &str, picks: &BTreeMap<String, f32>) -> Vec<(String, f32)> {
    let Some(c) = cat else {
        return picks.iter().map(|(k, v)| (k.clone(), *v)).collect();
    };
    let lv = c.livery(livery);
    c.options
        .iter()
        .filter_map(|o| {
            let v = pick(picks, &o.var)?;
            (!o.as_livery(lv).is_some_and(|l| same(l, v))).then(|| (o.var.clone(), v))
        })
        .collect()
}

/// A few livery names for a value's line: two, and how many more.
pub fn liveries_text(names: &[String]) -> String {
    match names.len() {
        0..=2 => names.join(", "),
        n => format!("{}, {} +{}", names[0], names[1], n - 2),
    }
}

/// What "As the livery" comes to for `o` under `livery`, in words: its value, or the usual one.
pub fn livery_words(o: &BusOption, livery: Option<&Livery>) -> String {
    let word = |v: f32| if o.is_switch() { tr(if v == 0.0 { "Off" } else { "On" }).into_owned() } else { value_text(v) };
    match o.as_livery(livery) {
        Some(v) => tr("As the livery (%{value})").replace("%{value}", &word(v)),
        None => tr("As the livery (usually %{value})").replace("%{value}", &word(o.usual)),
    }
}

/// The values to choose from for `o`: those the liveries give it, and a value chosen that none
/// gives (still offered, so that it can be seen and changed).
fn choices(o: &BusOption, chosen: Option<f32>) -> Vec<Seen> {
    let mut v = o.values.clone();
    if let Some(c) = chosen.filter(|c| !v.iter().any(|s| same(s.value, *c))) {
        v.push(Seen { value: c, liveries: Vec::new() });
        v.sort_by(|a, b| a.value.total_cmp(&b.value));
    }
    v
}

/// A selector's lines for `o`: "As the livery (…)", then each value with the liveries giving it.
pub fn entries(o: &BusOption, livery: Option<&Livery>, chosen: Option<f32>) -> Vec<String> {
    std::iter::once(livery_words(o, livery))
        .chain(choices(o, chosen).iter().map(|s| {
            if s.liveries.is_empty() {
                value_text(s.value)
            } else {
                tr("%{value} – as %{liveries}").replace("%{value}", &value_text(s.value)).replace("%{liveries}", &liveries_text(&s.liveries))
            }
        }))
        .collect()
}

/// Read what the bus `bus` (a file under `root`, as the launcher lists it) offers: its
/// liveries as the game makes them (the `.cti` files of its `[CTC]` folders, then the model's
/// own items), its scripts and its model's `[visible]` and `[matl_change]` variables.
pub fn read(root: &Path, bus: &str) -> anyhow::Result<Catalogue> {
    use anyhow::Context;
    let path = crate::spawn::player_bus_path(root, bus)?;
    let def = omsi_vehicle::Vehicle::load(&path).with_context(|| format!("loading {}", path.display()))?;
    let model_rel = def.model.clone().context("the bus has no [model]")?;
    let model_path = omsi_cfg::resolve_path(def.dir(), &model_rel);
    let model = omsi_model::Model::load(&model_path).with_context(|| format!("loading {}", model_path.display()))?;
    let liveries: Vec<Livery> = omsi_sim::vehicle::model_paint_schemes(def.dir(), &model).into_iter().map(|s| Livery { name: s.name.trim().to_string(), vars: s.set_vars }).collect();
    let scripts: Vec<(String, String)> = def
        .scripts
        .scripts
        .iter()
        .filter_map(|p| {
            let text = omsi_cfg::codepage::decode(&omsi_cfg::vfs::read(p).ok()?);
            Some((p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default(), text.to_lowercase()))
        })
        .collect();
    let shown: HashSet<String> = model
        .meshes
        .iter()
        .flat_map(|m| m.visible.iter().map(|v| v.0.trim().to_ascii_lowercase()).chain(m.materials.iter().filter_map(|t| t.change.as_ref().map(|c| c.2.trim().to_ascii_lowercase()))))
        .collect();
    let options = gather(&liveries, &Usage { scripts, shown });
    // (names of things, as their files spell them: never machine-translated)
    crate::mt::protect(options.iter().flat_map(|o| [o.label.as_str(), o.var.as_str()]));
    Ok(Catalogue { liveries, options })
}

// --- the choices, and what each bus offers ---------------------------------------------------

enum Slot {
    Reading,
    Ready(Arc<Catalogue>),
    Failed,
}

/// The launcher's side of the options: the choices per bus (kept in their file), what each bus
/// offers (read on a worker the first time it is asked for), and whether the technical ones
/// are unfolded.
pub struct Options {
    picks: BusOptions,
    /// Where the choices are kept (none: not kept, a test's).
    file: Option<PathBuf>,
    read: Arc<Mutex<HashMap<String, Slot>>>,
    pub technical_open: bool,
}

/// What a click in the section asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    /// This value for that variable; None: as the livery.
    Set(String, Option<f32>),
    /// Everything as the livery.
    Reset,
    /// Fold the technical options open or shut.
    Technical,
}

impl Options {
    /// The choices kept in `~/.openomsi`.
    pub fn load() -> Options {
        let file = BusOptions::path();
        Options { picks: BusOptions::read(&file), file: Some(file), read: Default::default(), technical_open: false }
    }

    /// Choices kept nowhere (tests).
    #[cfg(test)]
    pub fn in_memory() -> Options {
        Options { picks: BusOptions::default(), file: None, read: Default::default(), technical_open: false }
    }

    /// What `bus` offers: None while it is read (asked for now, the first time) or when it
    /// could not be.
    pub fn catalogue(&self, root: &str, bus: &str) -> Option<Arc<Catalogue>> {
        if bus.trim().is_empty() {
            return None;
        }
        let key = format!("{root}|{}", omsi_launcher_lib::busoptions::bus_key(bus));
        let mut slots = self.read.lock().ok()?;
        match slots.get(&key) {
            Some(Slot::Ready(c)) => return Some(c.clone()),
            Some(_) => return None,
            None => {}
        }
        slots.insert(key.clone(), Slot::Reading);
        let (map, root, bus) = (self.read.clone(), PathBuf::from(root), bus.to_string());
        std::thread::spawn(move || {
            let slot = match std::panic::catch_unwind(|| read(&root, &bus)) {
                Ok(Ok(c)) => Slot::Ready(Arc::new(c)),
                Ok(Err(e)) => {
                    log::info!("bus options of {bus}: {e:#}");
                    Slot::Failed
                }
                Err(_) => Slot::Failed,
            };
            if let Ok(mut m) = map.lock() {
                m.insert(key, slot);
            }
        });
        None
    }

    /// The buses are read again (the content changed): so is what they offer.
    pub fn forget(&self) {
        if let Ok(mut m) = self.read.lock() {
            m.retain(|_, s| matches!(s, Slot::Reading));
        }
    }

    /// The choices for `bus`.
    pub fn picks(&self, bus: &str) -> BTreeMap<String, f32> {
        self.picks.of(bus)
    }

    /// What the showroom puts on the bus: every choice (one the livery makes the same is the
    /// same picture).
    pub fn for_preview(&self, bus: &str) -> Vec<(String, f32)> {
        self.picks.of(bus).into_iter().collect()
    }

    /// What the game gets for `bus` in livery `paint` (see `setvars`).
    pub fn for_game(&self, root: &str, bus: &str, paint: &str) -> Vec<(String, f32)> {
        let picks = self.picks.of(bus);
        if picks.is_empty() {
            return Vec::new();
        }
        setvars(self.catalogue(root, bus).as_deref(), paint, &picks)
    }

    /// Do what the section asked for `bus`.
    pub fn edit(&mut self, bus: &str, e: Edit) {
        match e {
            Edit::Set(var, v) => self.picks.set(bus, &var, v),
            Edit::Reset => self.picks.reset(bus),
            Edit::Technical => {
                self.technical_open = !self.technical_open;
                return;
            }
        }
        if let Some(f) = &self.file {
            if let Err(err) = self.picks.write(f) {
                log::warn!("bus options: {} not written: {err}", f.display());
            }
        }
    }
}

// --- the section in a sheet -----------------------------------------------------------------

/// What the section shows.
pub struct Section<'a> {
    pub cat: &'a Catalogue,
    /// The livery chosen ("" the model's own textures).
    pub livery: &'a str,
    pub picks: &'a BTreeMap<String, f32>,
    pub technical_open: bool,
}

/// The bus options at (`x`, `y`), `w` wide - in a list that scrolls, the rows out of sight
/// are only measured. Returns the height they take (none for a bus without options) and what
/// was clicked.
pub(super) fn section(ui: &mut Ui, x: f32, y: f32, w: f32, s: &Section) -> (f32, Option<Edit>) {
    if s.cat.options.is_empty() {
        return (0.0, None);
    }
    let touch = ui.input.touch || super::mobile::mobile();
    let row_h = if touch { 54.0 } else { 46.0 };
    let livery = s.cat.livery(s.livery);
    let mut edit = None;
    let top = y;
    // the heading, and the way back to the liveries' own when something was chosen
    ui.heading(Rect::new(x, y, w, 26.0), "Bus options", None);
    let changed = s.cat.options.iter().filter(|o| pick(s.picks, &o.var).is_some()).count();
    if changed > 0 {
        let label = tr("Reset");
        let rw = ui.width(&label, 13.0, Weight::Medium) + 44.0;
        let r = Rect::new(x + w - rw, y - 2.0, rw, 26.0);
        if ui.button("bus-options-reset", r, "Reset", Some("restart_alt"), ButtonKind::Ghost) {
            edit = Some(Edit::Reset);
        }
        ui.tooltip(r, "Every option back as the livery has it");
    }
    let mut y = y + 30.0;
    let (looks, technical): (Vec<&BusOption>, Vec<&BusOption>) = s.cat.options.iter().partition(|o| o.kind == Kind::Appearance);
    for o in &looks {
        let r = Rect::new(x, y, w, row_h);
        if let Some(e) = option_row(ui, o, r, livery, pick(s.picks, &o.var), touch) {
            edit = Some(e);
        }
        y += row_h;
    }
    if !technical.is_empty() {
        y += if looks.is_empty() { 0.0 } else { 6.0 };
        let fold = Rect::new(x, y, w, 34.0);
        if ui.rect_visible(fold) {
            if ui.row("bus-options-technical", fold, false) {
                edit = Some(Edit::Technical);
            }
            ui.icon(if s.technical_open { "expand_less" } else { "expand_more" }, Vec2::new(fold.x + 12.0, fold.center().y), 18.0, TEXT_DIM);
            let tw = ui.width(&tr("Technical"), 12.5, Weight::Medium);
            ui.text_in("Technical", Rect::new(fold.x + 30.0, fold.y, tw + 4.0, fold.h), 12.5, Weight::Medium, TEXT_SOFT, Align::Left);
            let n = technical.iter().filter(|o| pick(s.picks, &o.var).is_some()).count();
            let count = if n > 0 { format!("{n} / {}", technical.len()) } else { technical.len().to_string() };
            ui.text_in(&count, Rect::new(fold.x + 38.0 + tw, fold.y, fold.w - tw - 40.0, fold.h), 12.0, Weight::Regular, if n > 0 { accent_2() } else { TEXT_FAINT }, Align::Left);
        }
        y += 38.0;
        if s.technical_open {
            let note = "These change how the bus works: its scripts read them.";
            let nh = ui.paragraph_height(note, w - 10.0, 11.5, Weight::Regular);
            if ui.rect_visible(Rect::new(x, y, w, nh)) {
                ui.paragraph(note, Vec2::new(x + 10.0, y), w - 10.0, 11.5, Weight::Regular, TEXT_DIM);
            }
            y += nh + 6.0;
            for o in &technical {
                let r = Rect::new(x, y, w, row_h);
                if let Some(e) = option_row(ui, o, r, livery, pick(s.picks, &o.var), touch) {
                    edit = Some(e);
                }
                y += row_h;
            }
        }
    }
    (y - top, edit)
}

/// One option: its name (the variable's own under it), and its choice on the right - Livery,
/// Off, On for a switch, else a list of the values.
fn option_row(ui: &mut Ui, o: &BusOption, r: Rect, livery: Option<&Livery>, chosen: Option<f32>, touch: bool) -> Option<Edit> {
    if !ui.rect_visible(r) {
        return None;
    }
    let ch = if touch { 36.0 } else { 30.0 };
    let cw = (r.w * 0.52).clamp(150.0, 200.0).min(r.w - 90.0);
    let ctl = Rect::new(r.right() - cw - 8.0, r.y + (r.h - ch) * 0.5, cw, ch);
    let lx = r.x + 10.0;
    let lw = (ctl.x - 10.0 - lx).max(20.0);
    if chosen.is_some() {
        ui.p().rounded(Rect::new(r.x, r.y + 10.0, 3.0, r.h - 20.0), 1.5, accent());
    }
    let mid = r.y + r.h * 0.5;
    ui.text_in(&o.label, Rect::new(lx, mid - 17.0, lw, 18.0), 13.0, Weight::Medium, if chosen.is_some() { TEXT } else { TEXT_SOFT }, Align::Left);
    ui.text_in(&o.var, Rect::new(lx, mid + 2.0, lw, 14.0), 10.5, Weight::Regular, TEXT_FAINT, Align::Left);
    let says = livery_words(o, livery);
    let tip = format!("{}\n{}\n{says}", o.label, o.var);
    crate::mt::protect([says.as_str(), tip.as_str()]);
    ui.tooltip(Rect::new(lx, r.y, lw, r.h), &tip);
    let name = format!("bus-option-{}", o.var.to_ascii_lowercase());
    if o.is_switch() {
        let mut sel = match chosen {
            None => 0,
            Some(v) if v == 0.0 => 1,
            Some(_) => 2,
        };
        // (the value the livery gives, marked under it; a fainter mark the usual one)
        let mark = match o.as_livery(livery) {
            Some(v) => (if v == 0.0 { 1 } else { 2 }, true),
            None => (if o.usual == 0.0 { 1 } else { 2 }, false),
        };
        if switch3(ui, &name, ctl, &mut sel, ["Livery", "Off", "On"], mark) {
            return Some(Edit::Set(o.var.clone(), [None, Some(0.0), Some(1.0)][sel]));
        }
        ui.tooltip(ctl, &says);
    } else {
        let list = entries(o, livery, chosen);
        crate::mt::protect(list.iter().map(String::as_str));
        let values = choices(o, chosen);
        let mut sel = chosen.and_then(|c| values.iter().position(|s| same(s.value, c))).map(|i| i + 1).unwrap_or(0);
        if ui.select(&name, ctl, &mut sel, &list) {
            return Some(Edit::Set(o.var.clone(), sel.checked_sub(1).map(|i| values[i].value)));
        }
    }
    None
}

/// Three choices side by side - the livery's, off, on - each as wide as its word: the chosen
/// one's pill quiet while it is the livery's, blue when the player chose. `mark` puts a dot
/// under the choice the livery gives (a fainter one: what most liveries give, this one
/// setting nothing).
fn switch3(ui: &mut Ui, name: &str, r: Rect, sel: &mut usize, labels: [&str; 3], mark: (usize, bool)) -> bool {
    let id = id_of(name);
    let px = 12.0;
    // (each as wide as its word in the interface's language)
    let natural: Vec<f32> = labels.iter().map(|l| ui.width(&tr(l), px, Weight::Bold) + 18.0).collect();
    let total: f32 = natural.iter().sum();
    let k = r.w / total.max(1.0);
    let mut cells = Vec::with_capacity(3);
    let mut cx = r.x;
    for w in &natural {
        cells.push(Rect::new(cx, r.y, w * k, r.h));
        cx += w * k;
    }
    ui.p().rounded(r, r.h * 0.5, FIELD);
    let mut changed = false;
    let mut hovered = [false; 3];
    for (i, cell) in cells.iter().enumerate() {
        let (h, _, clicked) = ui.interact(id ^ (i as u64 + 31), *cell);
        hovered[i] = h;
        if clicked && *sel != i {
            *sel = i;
            changed = true;
        }
    }
    let at = cells[(*sel).min(2)];
    let (x0, x1) = ui.slide_span(id ^ 0x5e17, at.x + 3.0, at.right() - 3.0);
    let chosen = ui.spring(id ^ 0xc01, if *sel == 0 { 0.0 } else { 1.0 }, Feel::SLIDE).clamp(0.0, 1.0);
    let pill = Rect::new(x0, r.y + 3.0, (x1 - x0).max(0.0), r.h - 6.0);
    ui.p().rounded(pill, pill.h * 0.5, HOVER.mix(accent(), chosen));
    for (i, (cell, word)) in cells.iter().zip(labels.iter()).enumerate() {
        let on = *sel == i;
        let c = if on && i > 0 { on_accent() } else if on || hovered[i] { TEXT } else { TEXT_DIM };
        ui.text_in(word, Rect::new(cell.x + 2.0, cell.y - 1.0, cell.w - 4.0, cell.h), px, if on { Weight::Bold } else { Weight::Medium }, c, Align::Center);
        if i == mark.0 {
            let dot = if on && i > 0 { on_accent() } else { accent_2() };
            ui.p().circle(Vec2::new(cell.center().x, cell.bottom() - 5.0), 2.0, dot.alpha(if mark.1 { 0.95 } else { 0.4 }));
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn livery(name: &str, vars: &[(&str, f32)]) -> Livery {
        Livery { name: name.into(), vars: vars.iter().map(|(n, v)| (n.to_string(), *v)).collect() }
    }

    #[test]
    fn names_read_as_words() {
        assert_eq!(label_of("vis_mirrors"), "Mirrors");
        assert_eq!(label_of("vis_CTI_spiegeltyp"), "Spiegeltyp");
        assert_eq!(label_of("vis_CTI_Rad_V"), "Rad V");
        assert_eq!(label_of("vis_CTI_Rad_H2"), "Rad H2");
        assert_eq!(label_of("vis_CTI_Matrix_LED_Color"), "Matrix LED color");
        assert_eq!(label_of("vis_CTI_Tuer45_piepen"), "Tuer 45 piepen");
        assert_eq!(label_of("vis_SV_123DNR"), "123 DNR");
        assert_eq!(label_of("vis_IBIS_Version_int"), "IBIS version int");
        assert_eq!(label_of("vis_grill_invisible"), "Grill invisible");
        assert_eq!(label_of("reg_number_window_sticker"), "Reg number window sticker");
        assert_eq!(label_of("vis_CTI_AdBluelampe"), "Ad bluelampe");
        assert_eq!(label_of("frontDoorLEDColor"), "Front door LED color");
        assert_eq!(label_of("door_2_type"), "Door 2 type");
        // a prefix alone is the name; no words at all: the name as it is
        assert_eq!(label_of("vis_"), "Vis");
        assert_eq!(label_of("__"), "__");
        assert_eq!(label_of("  SV_Tempomat "), "Tempomat");
    }

    #[test]
    fn a_script_working_with_a_variable_makes_it_technical() {
        let usage = Usage {
            scripts: vec![
                ("setvar.osc".into(), "{frame}\n(l.l.vis_cti_rad_v) (s.l.vis_sv_rad_v)\n(l.l.vis_cti_matrix_led) 1 =\n{end}".into()),
                ("visual.osc".into(), "(l.l.vis_grill_invisible) {if} {endif}".into()),
            ],
            shown: ["vis_cti_rad_v", "vis_mirrors", "vis_grill_invisible", "vis_cti_matrix"].into_iter().map(String::from).collect(),
        };
        assert_eq!(kind_of("vis_CTI_Rad_V", &usage), Kind::Technical, "a script takes it over");
        assert_eq!(kind_of("vis_mirrors", &usage), Kind::Appearance, "a [visible] alone");
        assert_eq!(kind_of("vis_grill_invisible", &usage), Kind::Appearance, "a visual script is no reason");
        // (vis_cti_matrix_led is another variable than vis_cti_matrix)
        assert_eq!(kind_of("vis_CTI_matrix", &usage), Kind::Appearance);
        assert_eq!(kind_of("vis_CTI_matrix_LED", &usage), Kind::Technical);
        assert_eq!(kind_of("nothing_reads_it", &usage), Kind::Technical, "unknown: careful");
        assert!(mentioned("(s.l.door_2_type)", "door_2_type") && !mentioned("(l.l.door_2_types)", "door_2_type"));
    }

    #[test]
    fn the_usual_value_is_what_most_liveries_give() {
        // (O560: three of four wheel the alternative rims, one sets nothing - 0 for the game)
        let ls = vec![livery("Haren", &[("vis_wheels", 1.0)]), livery("Postbus", &[]), livery("Leibfritz", &[("vis_wheels", 1.0)]), livery("Base", &[("vis_wheels", 1.0)])];
        assert_eq!(usual_of(&ls, "VIS_WHEELS"), 1.0);
        // as many each way: the first livery's
        let tie = vec![livery("A", &[("seats", 2.0)]), livery("B", &[("seats", 3.0)]), livery("C", &[("seats", 3.0)]), livery("D", &[("seats", 2.0)])];
        assert_eq!(usual_of(&tie, "seats"), 2.0);
        let few = vec![livery("A", &[("x", 2.0)]), livery("B", &[]), livery("C", &[])];
        assert_eq!(usual_of(&few, "x"), 0.0);
        assert_eq!(usual_of(&[], "x"), 0.0);
    }

    #[test]
    fn the_options_of_the_liveries() {
        let ls = vec![
            livery("Berlin", &[("vis_mirrors", 1.0), ("vis_CTI_matrix", 2.0), ("Colorscheme", 4.0)]),
            livery("Bremen", &[("VIS_MIRRORS", 0.0), ("vis_CTI_matrix", 2.0), ("vis_CTI_matrix", 1.0)]),
            livery("Hamburg", &[("vis_CTI_matrix", 2.0)]),
            livery("Berlin", &[("vis_mirrors", 1.0)]),
        ];
        let usage = Usage { scripts: vec![("main.osc".into(), "(l.l.vis_cti_matrix)".into())], shown: ["vis_mirrors".to_string()].into_iter().collect() };
        let o = gather(&ls, &usage);
        assert_eq!(o.iter().map(|o| o.var.as_str()).collect::<Vec<_>>(), ["vis_mirrors", "vis_CTI_matrix"], "the appearance first; Colorscheme is the game's");
        let mirrors = &o[0];
        assert_eq!((mirrors.label.as_str(), mirrors.kind, mirrors.is_switch()), ("Mirrors", Kind::Appearance, true));
        assert_eq!(mirrors.values, vec![Seen { value: 0.0, liveries: vec!["Bremen".into()] }, Seen { value: 1.0, liveries: vec!["Berlin".into()] }]);
        let matrix = &o[1];
        assert_eq!((matrix.kind, matrix.is_switch(), matrix.usual), (Kind::Technical, false, 2.0));
        // (Bremen's last line for it counts)
        assert_eq!(matrix.values, vec![Seen { value: 1.0, liveries: vec!["Bremen".into()] }, Seen { value: 2.0, liveries: vec!["Berlin".into(), "Hamburg".into()] }]);
        assert!(gather(&[livery("Plain", &[])], &usage).is_empty(), "a bus without setvars offers nothing");
    }

    fn catalogue() -> Catalogue {
        let liveries = vec![livery("Berlin", &[("vis_mirrors", 1.0), ("vis_matrix", 2.0)]), livery("Bremen", &[("vis_mirrors", 0.0), ("vis_matrix", 3.0)]), livery("Plain", &[])];
        let usage = Usage { scripts: Vec::new(), shown: ["vis_mirrors".to_string(), "vis_matrix".to_string()].into_iter().collect() };
        Catalogue { options: gather(&liveries, &usage), liveries }
    }

    #[test]
    fn what_the_game_gets() {
        let c = catalogue();
        let picks: BTreeMap<String, f32> = [("VIS_Mirrors".to_string(), 0.0), ("vis_matrix".to_string(), 2.0), ("gone".to_string(), 1.0)].into_iter().collect();
        // Berlin has mirrors and matrix 2: only the mirrors off go
        assert_eq!(setvars(Some(&c), "Berlin", &picks), vec![("vis_mirrors".to_string(), 0.0)]);
        // Bremen: its mirrors are off already, its matrix is 3
        assert_eq!(setvars(Some(&c), "bremen", &picks), vec![("vis_matrix".to_string(), 2.0)]);
        // a livery that sets neither, and the model's own textures: both go
        assert_eq!(setvars(Some(&c), "Plain", &picks).len(), 2);
        assert_eq!(setvars(Some(&c), "", &picks).len(), 2);
        // not read yet: every choice as it is
        assert_eq!(setvars(None, "Berlin", &picks).len(), 3);
        assert!(setvars(Some(&c), "Berlin", &BTreeMap::new()).is_empty());
    }

    #[test]
    fn the_choices_offered() {
        let c = catalogue();
        let mirrors = c.options.iter().find(|o| o.var == "vis_mirrors").unwrap();
        let matrix = c.options.iter().find(|o| o.var == "vis_matrix").unwrap();
        assert_eq!(livery_words(mirrors, c.livery("Berlin")), "As the livery (On)");
        assert_eq!(livery_words(mirrors, c.livery("Plain")), "As the livery (usually Off)");
        assert_eq!(entries(matrix, c.livery("Bremen"), None), ["As the livery (3)", "2 – as Berlin", "3 – as Bremen"]);
        // a value chosen that no livery gives is offered too; as many liveries give 2, 3 and
        // nothing (0): the first's is the usual one
        assert_eq!(matrix.usual, 2.0);
        assert_eq!(entries(matrix, None, Some(7.0)), ["As the livery (usually 2)", "2 – as Berlin", "3 – as Bremen", "7"]);
        let many: Vec<String> = ["A", "B", "C", "D", "E"].into_iter().map(String::from).collect();
        assert_eq!(liveries_text(&many), "A, B +3");
        assert_eq!(liveries_text(&many[..2]), "A, B");
    }

    #[test]
    fn choices_are_kept_and_put_back() {
        let mut o = Options::in_memory();
        o.edit("Vehicles/X/x.bus", Edit::Set("vis_mirrors".into(), Some(0.0)));
        o.edit("Vehicles/X/x.bus", Edit::Set("vis_matrix".into(), Some(3.0)));
        assert_eq!(o.for_preview("vehicles/x/X.bus"), vec![("vis_matrix".to_string(), 3.0), ("vis_mirrors".to_string(), 0.0)]);
        o.edit("Vehicles/X/x.bus", Edit::Set("vis_mirrors".into(), None));
        assert_eq!(o.picks("Vehicles/X/x.bus").len(), 1);
        o.edit("Vehicles/X/x.bus", Edit::Reset);
        assert!(o.for_preview("Vehicles/X/x.bus").is_empty());
        assert!(!o.technical_open);
        o.edit("Vehicles/X/x.bus", Edit::Technical);
        assert!(o.technical_open);
        // nothing chosen: nothing for the game, and no bus read for it
        assert!(o.for_game("", "Vehicles/X/x.bus", "Berlin").is_empty());
    }

    /// What the launcher passes, the game reads back.
    #[test]
    fn the_game_reads_the_options_the_launcher_passes() {
        let vars = vec![("vis_mirrors".to_string(), 0.0), ("vis_CTI_matrix".to_string(), 2.0), ("tilt".to_string(), 0.25)];
        let arg = omsi_launcher_lib::busoptions::setvar_arg(&vars).unwrap();
        assert_eq!(crate::spawn::setvars(Some(arg.as_str())), vars);
        assert_eq!(crate::spawn::setvars(Some(" a = 1 ,=2,b,c=3")), vec![("a".to_string(), 1.0), ("c".to_string(), 3.0)]);
        assert!(crate::spawn::setvars(None).is_empty());
    }

    /// A bus as it lies in the OMSI folder: its liveries' `.cti`, the model's own item after
    /// them, a script that takes one option over and a mesh that shows another.
    #[test]
    fn a_bus_is_read_from_its_files() {
        let root = std::env::temp_dir().join(format!("omsi_bus_options_read_{}", std::process::id()));
        let bus = root.join("Vehicles").join("Test");
        std::fs::create_dir_all(bus.join("model")).unwrap();
        std::fs::create_dir_all(bus.join("script")).unwrap();
        std::fs::create_dir_all(bus.join("texture").join("repaints")).unwrap();
        std::fs::write(bus.join("test.bus"), "[friendlyname]\r\nMaker\r\nTest\r\nBeige\r\n\r\n[model]\r\nmodel\\model.cfg\r\n\r\n[script]\r\n1\r\nscript\\main.osc\r\n").unwrap();
        std::fs::write(bus.join("script").join("main.osc"), "{init}\r\n(L.L.vis_seats) (S.L.in_seats)\r\n{end}\r\n").unwrap();
        std::fs::write(
            bus.join("model").join("model.cfg"),
            "[CTC]\r\nColorscheme\r\ntexture\\repaints\r\n2\r\n\r\n[CTCTexture]\r\nbody\r\nbody.dds\r\n\r\n[mesh]\r\nmirror.o3d\r\n\r\n[visible]\r\nvis_mirrors\r\n1\r\n\r\n[item]\r\nWerk\r\nbody\r\nwerk.dds\r\n\r\n[setvar]\r\nvis_mirrors\r\n0\r\n",
        )
        .unwrap();
        std::fs::write(bus.join("texture").join("repaints").join("liveries.cti"), "[item]\r\nBerlin\r\nbody\r\nberlin.dds\r\n\r\n[setvar]\r\nvis_mirrors\r\n1\r\n\r\n[setvar]\r\nvis_seats\r\n2\r\n\r\n[item]\r\nBremen\r\nbody\r\nbremen.dds\r\n\r\n[setvar]\r\nvis_seats\r\n3\r\n").unwrap();
        let c = read(&root, "Vehicles/Test/test.bus").unwrap();
        assert_eq!(c.liveries.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["Berlin", "Bremen", "Werk"]);
        let o: Vec<(&str, Kind, usize)> = c.options.iter().map(|o| (o.var.as_str(), o.kind, o.values.len())).collect();
        assert_eq!(o, [("vis_mirrors", Kind::Appearance, 2), ("vis_seats", Kind::Technical, 2)]);
        assert!(read(&root, "Vehicles/Test/none.bus").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
