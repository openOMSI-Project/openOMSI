//! The bus step's display font: the font the chosen bus's destination displays are drawn in -
//! "As the bus" (its own fonts), an OMSI `.oft` font (every `Fonts` folder of the content
//! roots: the OMSI installation's and openOMSI's content folder's) or a TrueType/OpenType font
//! (those added to the content folder and the system's), each offered with the duty's
//! destination written in it as a small LED sign the way the game draws it on this bus: in the
//! grid of dots of the bus's own font, a line of its height (`omsi_content::dotfont`,
//! `omsi_sim::texttex::display_font_atlas` - the same atlas the game draws with). A chosen font
//! has the settings that matter on a display: how many rows of dots its letters take, bold,
//! the spacing. "Add font…" copies a `.oft` with its bitmaps, or a `.ttf`/`.otf`/`.ttc`, into
//! the content folder's `Fonts` (never the installation's). The choice is kept per bus file
//! (`omsi_launcher_lib::busfonts`) and goes to the game as `--display-font`.
//!
//! What a font changes on the bus is said as it is ([`Plan`]): its text displays a font draws
//! (the destination, the line number, a side or rear sign), the matrices its scripts write
//! with fonts they ask the game for by name (the Krüger matrices: the font takes those fonts'
//! places), and when the bus draws its destination as pictures of its own - then a font changes
//! only what else it has, or nothing, and the row says so instead of offering fonts that would
//! change nothing.

use super::theme::*;
use super::ui::{picture_option, ButtonKind, Ui};
use glam::Vec2;
use omsi_content::dotfont::{DisplayFontSpec, DotFont, DotGrid, VectorFace};
use omsi_content::font::{resample_into, Font, FontAtlas};
use omsi_launcher_lib::busfonts::{self, BusFonts};
use omsi_model::TextTexture;
use omsi_sim::texttex::{DisplayRole, ScriptSign};
use omsi_ui::paint::Align;
use omsi_ui::{tr, Color, Rect, Weight};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// How many rows of pixels a preview sign has about (a pixel font is magnified by whole
/// times up to it, a taller one shrunk to it).
const PREVIEW_ROWS: u32 = 44;
/// The widest a preview is (its text cut off there).
const PREVIEW_MAX_W: u32 = 560;
/// The sign's panel, and an unlit dot on it.
const PANEL_RGB: [u8; 3] = [10, 9, 8];
const DOT_RGB: [u8; 3] = [26, 22, 16];
/// The colour of a sign whose display gives none (black text on black).
const AMBER: [f32; 3] = [255.0, 170.0, 0.0];
/// The share of a destination's letters a font must have to be offered for it.
const ENOUGH_LETTERS: f32 = 0.6;

/// What the previews write when no duty gives a destination.
pub const SAMPLE: &str = "12 Hauptbahnhof";

// --- a bus's signs ----------------------------------------------------------------------------

/// A bus's destination display as the previews draw it: the widest of its destination
/// displays (the terminus, where a line number has its own beside it) and its own font - or,
/// on a bus whose script draws its matrix, that matrix's largest letter font.
#[derive(Clone)]
pub struct Sign {
    pub def: TextTexture,
    pub own: Option<Arc<FontAtlas>>,
    /// A line of the display's own font: what a chosen font is fitted to.
    pub line_h: i32,
    /// How many destination displays the bus has (the main model's).
    pub displays: usize,
    /// What the display shows: a line number's preview writes the line alone.
    pub role: DisplayRole,
}

impl Sign {
    /// The grid of dots the display's own font draws in (see `DotGrid::of_atlas`).
    pub fn grid(&self) -> DotGrid {
        self.own.as_deref().map(DotGrid::of_atlas).unwrap_or_else(|| DotGrid::plain(self.line_h.max(1) as u32))
    }

    /// What its preview writes of the destination `text` ("12 Hauptbahnhof"): a line number's
    /// display the line alone.
    pub fn text_of<'a>(&self, text: &'a str) -> &'a str {
        match self.role {
            DisplayRole::LineNumber => text.split_whitespace().next().unwrap_or(text),
            _ => text,
        }
    }
}

/// What a display font changes on a bus.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    /// Its text displays a font draws, by what they show.
    pub texts: Vec<DisplayRole>,
    /// The matrices its scripts draw with fonts they ask for by name (the font takes their
    /// places there).
    pub scripts: Vec<ScriptSign>,
    /// It shows pictures its scripts draw on its outside (a matrix) that no font draws: its
    /// destination drawn by itself, pixel by pixel or from bitmaps.
    pub pictures: bool,
    /// It has text displays of the cab or the saloon (IBIS, printer, the passengers'
    /// display inside), which keep their own fonts.
    pub devices: bool,
}

/// What a display font changes on the bus of `model` whose scripts are `scripts` (see `Plan`).
pub fn plan_of(model: &omsi_model::Model, scripts: &[PathBuf]) -> Plan {
    let displays = omsi_sim::texttex::destination_displays(model);
    let texts: Vec<DisplayRole> = displays.iter().map(|&i| DisplayRole::of(&model.text_textures[i])).collect();
    let signs = omsi_sim::texttex::script_signs(scripts);
    // (a mesh seen from outside showing a script's picture, as its texture or its transparency
    // map `\S:n`, that is no device of the cab or the saloon)
    let devices_words = ["ibis", "cockpit", "dash", "drucker", "printer", "ticket", "innen", "interior", "monitor", "tacho", "radio"];
    let pictures = !model.script_textures.is_empty()
        && model.meshes.iter().any(|m| {
            let outside = m.viewpoint == 0 || m.viewpoint & 1 != 0;
            let file = m.file.to_lowercase();
            outside && !devices_words.iter().any(|w| file.contains(w)) && m.materials.iter().any(|t| t.use_script_texture.is_some() || t.transmap.as_deref().is_some_and(|tm| tm.trim().to_ascii_lowercase().starts_with("\\s:")))
        });
    let device_words = ["ibis", "drucker", "printer", "ticket", "innen", "interior", "cockpit", "dash", "almex"];
    let devices = model.text_textures.iter().enumerate().any(|(i, t)| !displays.contains(&i) && device_words.iter().any(|w| t.variable.to_lowercase().contains(w) || t.font.to_lowercase().contains(w)));
    Plan { texts, scripts: signs, pictures, devices }
}

/// What and how much of the bus a chosen font changes, in words (see `Plan`).
pub fn plan_text(plan: &Plan) -> String {
    let mut roles: Vec<DisplayRole> = plan.texts.clone();
    roles.sort();
    roles.dedup();
    let what = roles
        .iter()
        .map(|r| match r {
            DisplayRole::Destination => tr("the destination"),
            DisplayRole::LineNumber => tr("the line number"),
            DisplayRole::Side => tr("the side sign"),
            DisplayRole::Rear => tr("the rear sign"),
        })
        .collect::<Vec<_>>()
        .join(", ");
    let mut parts: Vec<String> = Vec::new();
    if let Some(s) = plan.scripts.first() {
        let script = s.script.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        parts.push(tr("This bus's matrix script (%{script}) writes its destination and line number itself, in fonts it asks the game for by name: a chosen font takes their place there, each fitted to the height of the one it replaces.").replace("%{script}", &script));
        if !what.is_empty() {
            parts.push(tr("It also changes %{what}.").replace("%{what}", &what));
        }
        if plan.scripts.iter().any(|s| !s.kept.is_empty()) {
            parts.push(tr("The matrix's pictograms and clock keep their own.").into_owned());
        }
        if plan.scripts.iter().any(|s| s.pictures) {
            parts.push(tr("Destinations its depot file gives as pictures stay pictures.").into_owned());
        }
    } else if plan.pictures && !what.is_empty() && !roles.contains(&DisplayRole::Destination) {
        parts.push(tr("This bus draws its destination itself: a font changes only %{what}.").replace("%{what}", &what));
    } else if !what.is_empty() {
        parts.push(tr("A chosen font changes %{what}.").replace("%{what}", &what));
    } else {
        parts.push(tr("This bus draws its destination displays as pictures of its own: a display font does not change them.").into_owned());
    }
    if plan.devices && (!plan.scripts.is_empty() || !what.is_empty()) {
        parts.push(tr("The cab's and the saloon's displays (IBIS, printer, inside) keep their own fonts.").into_owned());
    }
    parts.join(" ")
}

/// A bus's signs: what a font changes, and the sign its previews draw (none: no font draws
/// any of its displays).
#[derive(Clone)]
pub struct BusSigns {
    pub plan: Plan,
    pub sign: Option<Sign>,
}

/// Read `bus`'s destination displays and what a font changes on it.
pub fn read_signs(root: &Path, bus: &str) -> anyhow::Result<BusSigns> {
    use anyhow::Context;
    let path = crate::spawn::player_bus_path(root, bus)?;
    let def = omsi_vehicle::Vehicle::load(&path).with_context(|| format!("loading {}", path.display()))?;
    let model_rel = def.model.clone().context("the bus has no [model]")?;
    let model_path = omsi_cfg::resolve_path(def.dir(), &model_rel);
    let model = omsi_model::Model::load(&model_path).with_context(|| format!("loading {}", model_path.display()))?;
    let plan = plan_of(&model, &def.scripts.scripts);
    let mut lib = omsi_sim::texttex::FontLibrary::new(root);
    let displays = omsi_sim::texttex::destination_displays(&model);
    // the terminus (a destination before a line number), the widest of them
    let main = displays.iter().copied().max_by_key(|&i| (DisplayRole::of(&model.text_textures[i]) != DisplayRole::LineNumber, model.text_textures[i].width, std::cmp::Reverse(i)));
    if let Some(main) = main {
        let t = model.text_textures[main].clone();
        let own = lib.load(&t.font);
        let line_h = own.as_ref().map(|a| a.font.height).filter(|h| *h > 0).unwrap_or(t.height).max(1);
        let role = DisplayRole::of(&t);
        return Ok(BusSigns { plan, sign: Some(Sign { def: t, own, line_h, displays: displays.len(), role }) });
    }
    // a matrix a script draws: its tallest font of small letters too, of the most letters (the
    // one line of a destination, not the line number's)
    for s in &plan.scripts {
        let tallest = s.fonts.iter().filter_map(|f| lib.load(f)).filter(|a| omsi_sim::texttex::is_letter_font(&a.font) && ('a'..='z').filter(|&c| a.font.has_glyph(c)).count() >= 13).max_by_key(|a| (a.font.height, a.font.chars.len()));
        if let Some(own) = tallest {
            let stem = s.script.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let def = TextTexture { variable: stem, font: own.font.name.trim().to_string(), width: 0, height: own.font.height, full_color: false, color: [0.0; 3], orientation: 0, grid: 1 };
            let line_h = own.font.height.max(1);
            return Ok(BusSigns { plan: plan.clone(), sign: Some(Sign { def, own: Some(own), line_h, displays: 0, role: DisplayRole::Destination }) });
        }
    }
    Ok(BusSigns { plan, sign: None })
}

/// Read `bus`'s destination displays (None: it has none a font draws).
#[allow(dead_code)]
pub fn read_sign(root: &Path, bus: &str) -> anyhow::Result<Option<Sign>> {
    Ok(read_signs(root, bus)?.sign)
}

/// A preview's pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

/// `text` on a small LED sign: as `sign`'s display draws it in `atlas` - a chosen font
/// (`fitted`: made for the display's line, see `atlas_for`), or the bus's own - the line cut
/// out of the texture, as wide as its letters, on a dark panel. A pixel font is magnified by
/// whole times to about `PREVIEW_ROWS` rows with the dots of a matrix sign between its pixels;
/// a taller font is shrunk to them.
pub fn sign_picture(sign: &Sign, atlas: Option<Arc<FontAtlas>>, fitted: bool, text: &str) -> Picture {
    let mut def = sign.def.clone();
    // (black letters say the colour comes from the mesh: amber, as most signs are)
    if def.color.iter().sum::<f32>() < 90.0 {
        def.color = AMBER;
    }
    def.full_color = false;
    let line_h = sign.line_h.max(1) as u32;
    // (the texture as wide as the text needs: a preview shows the font, not the clipping)
    if let Some(a) = atlas.as_ref() {
        let s = if fitted { omsi_content::font::fit_scale(line_h as f32, a.font.height.max(1) as f32) } else { 1.0 };
        def.width = def.width.max((a.text_width(text) as f32 * s) as i32 + 8);
    }
    def.height = def.height.max(line_h as i32);
    let (w, h) = (def.width.max(1) as u32, def.height.max(1) as u32);
    let mut state = omsi_sim::texttex::TextTextureState::new(def.clone(), atlas);
    state.fit = fitted.then_some(line_h);
    let img = state.image(text);
    // the line's rows (where the display's own font has its line), the letters' columns
    let top = ((h as i32 - line_h as i32) / 2).max(0) as u32;
    let rows = line_h.min(h - top.min(h)).max(1);
    let lit = |x: u32, y: u32| img[((y * w + x) * 4 + 3) as usize] > 0;
    let cols: Vec<u32> = (0..w).filter(|&x| (top..top + rows).any(|y| lit(x, y))).collect();
    let (x0, x1) = match (cols.first(), cols.last()) {
        (Some(&a), Some(&b)) => (a.saturating_sub(2), (b + 3).min(w)),
        _ => (0, w.min(48)),
    };
    let cw = (x1 - x0).max(1);
    let mut cut = vec![0u8; (cw * rows * 4) as usize];
    for y in 0..rows {
        let from = (((top + y) * w + x0) * 4) as usize;
        cut[(y * cw * 4) as usize..((y + 1) * cw * 4) as usize].copy_from_slice(&img[from..from + (cw * 4) as usize]);
    }
    led(&cut, cw, rows)
}

/// The cut-out line `src` (`w` x `h`, its colours laid over nothing) on the sign's panel.
fn led(src: &[u8], w: u32, h: u32) -> Picture {
    let k = (PREVIEW_ROWS / h.max(1)).clamp(1, 6);
    if h > PREVIEW_ROWS {
        // a large font: shrunk to the preview's rows, no dots of its own
        let dh = PREVIEW_ROWS;
        let dw = ((w as f32 * dh as f32 / h as f32).round() as u32).clamp(1, PREVIEW_MAX_W);
        let mut over = vec![0u8; (dw * dh * 4) as usize];
        resample_into(src, w, h, &mut over, dw, dh, 0, 0, (w as f32 * dh as f32 / h as f32).round() as i32, dh as i32);
        return on_panel(&over, dw, dh, |_, _| PANEL_RGB);
    }
    let (pw, ph) = ((w * k).min(PREVIEW_MAX_W), h * k);
    let mut big = vec![0u8; (pw * ph * 4) as usize];
    for y in 0..ph {
        for x in 0..pw {
            // (the matrix's dark seam between the dots: the last row and column of each cell)
            if k >= 3 && (x % k == k - 1 || y % k == k - 1) {
                continue;
            }
            let si = (((y / k) * w + x / k) * 4) as usize;
            let di = ((y * pw + x) * 4) as usize;
            big[di..di + 4].copy_from_slice(&src[si..si + 4]);
        }
    }
    on_panel(&big, pw, ph, |x, y| if k >= 3 && x % k != k - 1 && y % k != k - 1 { DOT_RGB } else { PANEL_RGB })
}

/// `over` (its colours laid over nothing) over the panel, whose colour at a pixel `under`
/// gives: an opaque picture.
fn on_panel(over: &[u8], w: u32, h: u32, under: impl Fn(u32, u32) -> [u8; 3]) -> Picture {
    let mut rgba = vec![255u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let a = over[i + 3] as f32 / 255.0;
            let bg = under(x, y);
            for c in 0..3 {
                rgba[i + c] = (over[i + c] as f32 + bg[c] as f32 * (1.0 - a)).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    Picture { w, h, rgba }
}

/// The share of `text`'s letters (not its spaces) `font` has a glyph for.
pub fn coverage(font: &Font, text: &str) -> f32 {
    let letters: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
    if letters.is_empty() {
        return 1.0;
    }
    letters.iter().filter(|&&c| font.glyph(c).is_some()).count() as f32 / letters.len() as f32
}

/// The fonts a player is offered for `text`: those that can write most of it (no plate's or
/// clock's digits alone), each name once, by name; `keep` among them whatever it can write.
pub fn offered(fonts: &[Font], text: &str, keep: Option<&str>) -> Vec<Font> {
    let keep = keep.map(|k| k.trim().to_ascii_lowercase());
    let mut out: Vec<Font> = fonts.iter().filter(|f| coverage(f, text) >= ENOUGH_LETTERS || keep.as_deref() == Some(f.name.trim().to_ascii_lowercase().as_str())).cloned().collect();
    out.sort_by(|a, b| super::buspick::name_cmp(a.name.trim(), b.name.trim()));
    out.dedup_by(|a, b| a.name.trim().eq_ignore_ascii_case(b.name.trim()));
    out
}

/// The size of `chosen`'s font file that the game draws on a line `line_h` high
/// (`FontLibrary::display_atlas`), among `fonts`.
pub fn size_for<'a>(fonts: &'a [Font], chosen: &str, line_h: i32) -> Option<&'a Font> {
    let first = fonts.iter().find(|f| f.name.trim().eq_ignore_ascii_case(chosen.trim()))?;
    let family: Vec<&Font> = fonts.iter().filter(|f| f.path == first.path).collect();
    let me = family.iter().position(|f| f.name.trim().eq_ignore_ascii_case(chosen.trim())).unwrap_or(0);
    let heights: Vec<i32> = family.iter().map(|f| f.height).collect();
    Some(family[omsi_sim::texttex::pick_size(&heights, me, line_h)])
}

/// The letters a preview of `text` needs from a vector font: its own and their capitals.
fn letters_of(text: &str) -> Vec<char> {
    let mut out: Vec<char> = text.chars().flat_map(|c| std::iter::once(c).chain(c.to_uppercase())).filter(|c| !c.is_whitespace()).collect();
    out.sort();
    out.dedup();
    out
}

/// The display font `spec` as `sign`'s display draws it (what the game draws with: see
/// `FontLibrary::display_font`), for the letters of `text`; and the share of them it has. An
/// `.oft` font from `fonts` (its bitmaps read with `decode`), a vector font from its file.
pub fn atlas_for(sign: &Sign, spec: &DisplayFontSpec, fonts: &[Font], text: &str, decode: &dyn Fn(&Path) -> Option<(u32, u32, Vec<u8>)>) -> Option<(FontAtlas, f32)> {
    let grid = sign.grid();
    let dots: DotFont = if spec.is_vector() {
        let data = spec.file.as_deref().and_then(|f| std::fs::read(f).ok())?;
        DotFont::from_vector(&data, spec.face, omsi_sim::texttex::vector_rows(spec, &grid), &letters_of(text)).ok()?
    } else {
        let font = size_for(fonts, &spec.name, sign.line_h)?;
        let atlas = omsi_sim::texttex::atlas_of(font.clone(), Path::new(""), decode)?;
        omsi_sim::texttex::oft_dots(&atlas)
    };
    let share = dots.coverage(text);
    Some((omsi_sim::texttex::display_font_atlas(&dots, spec, &grid, sign.line_h.max(1) as u32), share))
}

// --- the launcher's side ----------------------------------------------------------------------

enum Slot<T> {
    Reading,
    Ready(T),
    Failed,
}

/// The previews being drawn for one bus and text: what a preview's key is made of.
fn preview_key(bus: &str, text: &str, font: &str) -> String {
    format!("{}|{text}|{}", omsi_launcher_lib::busoptions::bus_key(bus), font.trim().to_ascii_lowercase())
}

/// Where the fonts offered come from: OMSI's `.oft` fonts, or TrueType/OpenType ones.
const SOURCES: [&str; 2] = ["OMSI fonts (.oft)", "TrueType / OpenType"];

/// One font of the list: the font (as it is), its name, and for an `.oft` font itself.
#[derive(Clone)]
struct Entry {
    spec: DisplayFontSpec,
    label: String,
}

/// The choices (kept in their file), the installed fonts and each bus's signs (read on
/// workers the first time they are asked for), and the previews (drawn on a worker, sent to
/// the GPU in `upload`).
pub struct DisplayFonts {
    choices: BusFonts,
    /// Where the choices are kept (none: not kept, a test's).
    file: Option<PathBuf>,
    fonts: Arc<Mutex<HashMap<String, Slot<Arc<Vec<Font>>>>>>,
    faces: Arc<Mutex<HashMap<String, Slot<Arc<Vec<VectorFace>>>>>>,
    signs: Arc<Mutex<HashMap<String, Slot<BusSigns>>>>,
    /// The fonts offered for a text (worked out again when the list or the text changes).
    offered: Option<(String, usize, Arc<Vec<Font>>)>,
    /// Vector fonts found unable to write a text (their preview key).
    unfit: Arc<Mutex<HashSet<String>>>,
    /// The list shown: `SOURCES` (none chosen: the kind of the bus's font).
    source: Option<usize>,
    /// The previews drawn, waiting for the GPU; on it (texture, size) by their key; what they
    /// are drawn for, and the worker drawing them (its number: a newer one stops it).
    pending: Arc<Mutex<Vec<(String, Picture)>>>,
    textures: HashMap<String, (usize, u32, u32)>,
    drawing: Option<String>,
    worker: Arc<AtomicU64>,
    /// Textures of previews no longer shown, to free on the GPU (`freeing`: next frame).
    stale: Vec<usize>,
    freeing: Vec<usize>,
    /// What "Add font…" did, and whether it went wrong.
    pub note: Option<(String, bool)>,
}

/// A text on a bus's destination display (`DisplayFonts::preview`): its picture on the GPU
/// (texture, size; None while it is drawn), whether it is wider than the display, the font - or
/// that the bus draws its displays as pictures of its own (`none`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Preview {
    pub picture: Option<(usize, u32, u32)>,
    pub too_wide: bool,
    pub font: String,
    pub none: bool,
}

/// What a click in the row asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    /// This font for the bus, as it is (an `.oft` font's name or a `DisplayFontSpec`
    /// argument); None: as the bus.
    #[allow(dead_code)]
    Pick(Option<String>),
    /// This font for the bus with the settings chosen so far; None: as the bus.
    Choose(Option<DisplayFontSpec>),
    /// The chosen font's settings: its rows of dots (none: the display's), bold, the dots
    /// between letters (none: the font's own).
    Style { rows: Option<u32>, bold: bool, spacing: Option<u32> },
    /// Show the fonts of `SOURCES[n]`.
    Source(usize),
    /// Add a font file to the content folder.
    Add,
}

impl DisplayFonts {
    /// The choices kept in `~/.openomsi`.
    pub fn load() -> DisplayFonts {
        let file = BusFonts::path();
        Self::with(BusFonts::read(&file), Some(file))
    }

    fn with(choices: BusFonts, file: Option<PathBuf>) -> DisplayFonts {
        DisplayFonts {
            choices,
            file,
            fonts: Default::default(),
            faces: Default::default(),
            signs: Default::default(),
            offered: None,
            unfit: Default::default(),
            source: None,
            pending: Default::default(),
            textures: HashMap::new(),
            drawing: None,
            worker: Arc::new(AtomicU64::new(0)),
            stale: Vec::new(),
            freeing: Vec::new(),
            note: None,
        }
    }

    /// Choices kept nowhere (tests).
    #[cfg(test)]
    pub fn in_memory() -> DisplayFonts {
        Self::with(BusFonts::default(), None)
    }

    /// The font `bus`'s destination displays are drawn in, with its settings (None: as the
    /// bus) - what the game gets as `--display-font`.
    pub fn font_for(&self, bus: &str) -> Option<String> {
        self.choices.font_for(bus)
    }

    /// The installed `.oft` fonts under `root` (None while they are read: asked for now, the
    /// first time).
    pub fn fonts(&self, root: &str) -> Option<Arc<Vec<Font>>> {
        read_once(&self.fonts, root, |root| omsi_sim::texttex::FontLibrary::new(Path::new(&root)).installed())
    }

    /// The TrueType/OpenType faces installed (the content folders' first, then the
    /// system's), each name once (None while they are read).
    pub fn faces(&self, root: &str) -> Option<Arc<Vec<VectorFace>>> {
        read_once(&self.faces, root, |root| {
            let mut faces = omsi_sim::texttex::FontLibrary::new(Path::new(&root)).vector_faces().to_vec();
            let mut seen = HashSet::new();
            faces.retain(|f| seen.insert(f.name.trim().to_lowercase()));
            // (those added to the content folder first, then the system's, each by name)
            let system = omsi_content::dotfont::system_font_dirs();
            faces.sort_by(|a, b| {
                let sys = |f: &VectorFace| system.iter().any(|d| f.path.starts_with(d));
                sys(a).cmp(&sys(b)).then_with(|| super::buspick::name_cmp(a.name.trim(), b.name.trim()))
            });
            faces
        })
    }

    /// `bus`'s sign: None while it is read (or when it could not be), Some(None) for a bus
    /// without a destination display a font draws.
    #[allow(dead_code)]
    pub fn sign(&self, root: &str, bus: &str) -> Option<Option<Sign>> {
        self.signs(root, bus).map(|s| s.sign)
    }

    /// `bus`'s signs and what a font changes on it (None while they are read).
    pub fn signs(&self, root: &str, bus: &str) -> Option<BusSigns> {
        if bus.trim().is_empty() {
            return None;
        }
        let key = format!("{root}|{}", omsi_launcher_lib::busoptions::bus_key(bus));
        let mut slots = self.signs.lock().ok()?;
        match slots.get(&key) {
            Some(Slot::Ready(s)) => return Some(s.clone()),
            Some(Slot::Failed) => return Some(BusSigns { plan: Plan::default(), sign: None }),
            Some(Slot::Reading) => return None,
            None => {}
        }
        slots.insert(key.clone(), Slot::Reading);
        let (map, root, bus) = (self.signs.clone(), PathBuf::from(root), bus.to_string());
        std::thread::spawn(move || {
            let slot = match std::panic::catch_unwind(|| read_signs(&root, &bus)) {
                Ok(Ok(s)) => Slot::Ready(s),
                Ok(Err(e)) => {
                    log::info!("display font: the displays of {bus}: {e:#}");
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

    /// The `.oft` fonts offered for `text` among `fonts` (see `offered`), `keep` among them.
    fn offered_for(&mut self, fonts: &Arc<Vec<Font>>, text: &str, keep: Option<&str>) -> Arc<Vec<Font>> {
        let key = format!("{text}|{}", keep.unwrap_or_default().to_ascii_lowercase());
        let id = Arc::as_ptr(fonts) as usize;
        if let Some((k, i, list)) = self.offered.as_ref() {
            if *k == key && *i == id {
                return list.clone();
            }
        }
        let list = Arc::new(offered(fonts, text, keep));
        self.offered = Some((key, id, list.clone()));
        list
    }

    /// `text` on `bus`'s destination display as a drive shows it - in the bus's own font, or
    /// the display font chosen for it - for the depot editor: its picture once it is drawn and
    /// on the GPU (asked for now), and whether it is wider than the display (in its own font).
    /// None while the display is read; `none` for a bus whose displays a font does not draw.
    pub fn preview(&mut self, root: &str, bus: &str, text: &str) -> Option<Preview> {
        let Some(sign) = self.sign(root, bus)? else { return Some(Preview { none: true, ..Default::default() }) };
        // (the font chosen with its settings, as the game gets it; an `.oft` one needs the
        // installed fonts read, a vector one its file)
        let chosen = self.choices.spec_for(bus);
        let fonts = match chosen.as_ref().filter(|c| !c.is_vector()) {
            Some(_) => self.fonts(root)?,
            None => Arc::new(Vec::new()),
        };
        let key = chosen.as_ref().map(|c| c.key()).unwrap_or_default();
        if self.picture(bus, text, &key).is_none() {
            self.draw(bus, text, &sign, fonts, Vec::new(), chosen.clone());
        }
        let too_wide = chosen.is_none() && sign.own.as_ref().is_some_and(|a| a.text_width(text) > sign.def.width);
        Some(Preview { picture: self.picture(bus, text, &key), too_wide, font: chosen.map(|c| c.name).unwrap_or_else(|| sign.def.font.trim().to_string()), none: false })
    }

    /// The preview of `font` ("" as the bus) on `bus`'s sign with `text`, once it is on the GPU.
    /// The list shown for `bus` (`SOURCES`): the one picked, else the kind of its font.
    fn source_for(&self, bus: &str) -> usize {
        self.source.unwrap_or_else(|| usize::from(self.choices.spec_for(bus).is_some_and(|s| s.is_vector())))
    }

    /// The preview of `font` (a `DisplayFontSpec::key`, "" as the bus) on `bus`'s sign with
    /// `text`, once it is on the GPU.
    fn picture(&self, bus: &str, text: &str, font: &str) -> Option<(usize, u32, u32)> {
        self.textures.get(&preview_key(bus, text, font)).copied()
    }

    /// Draw the previews of `bus`'s sign with `text` - as the bus, the chosen font with its
    /// settings, then every font of the list - on a worker, unless they are being drawn
    /// already. The previews of another bus or text, and of settings no longer chosen, are
    /// let go.
    #[allow(clippy::too_many_arguments)]
    fn draw(&mut self, bus: &str, text: &str, sign: &Sign, fonts: Arc<Vec<Font>>, list: Vec<DisplayFontSpec>, chosen: Option<DisplayFontSpec>) {
        let what = format!("{}|{}|{}", preview_key(bus, text, ""), chosen.as_ref().map(|c| c.key()).unwrap_or_default(), list.len());
        if self.drawing.as_deref() == Some(what.as_str()) {
            return;
        }
        self.drawing = Some(what);
        let prefix = format!("{}|", preview_key(bus, text, "").trim_end_matches('|'));
        // (what is no longer shown: off the GPU)
        let wanted: HashSet<String> = std::iter::once(preview_key(bus, text, "")).chain(chosen.iter().map(|c| preview_key(bus, text, &c.key()))).chain(list.iter().map(|s| preview_key(bus, text, &s.key()))).collect();
        let old: Vec<String> = self.textures.keys().filter(|k| !k.starts_with(&prefix)).cloned().collect();
        for k in old {
            if let Some((t, _, _)) = self.textures.remove(&k) {
                self.stale.push(t);
            }
        }
        // (a chosen font's settings no longer chosen: their pictures too)
        if let Some(c) = chosen.as_ref() {
            let styled = format!("{}|", preview_key(bus, text, &c.plain().key()));
            let other: Vec<String> = self.textures.keys().filter(|k| k.starts_with(&styled) && !wanted.contains(*k)).cloned().collect();
            for k in other {
                if let Some((t, _, _)) = self.textures.remove(&k) {
                    self.stale.push(t);
                }
            }
        }
        if let Ok(mut p) = self.pending.lock() {
            p.retain(|(k, _)| k.starts_with(&prefix));
        }
        let me = self.worker.fetch_add(1, Ordering::SeqCst) + 1;
        let (worker, pending, unfit, sign, bus, text) = (self.worker.clone(), self.pending.clone(), self.unfit.clone(), sign.clone(), bus.to_string(), text.to_string());
        let done: Vec<String> = self.textures.keys().cloned().collect();
        std::thread::spawn(move || {
            let decode = |p: &Path| omsi_texture::decode_file(p).ok().map(|i| (i.width, i.height, i.rgba));
            let shown = sign.text_of(&text).to_string();
            let put = |key: &str, pic: Picture| {
                if let Ok(mut p) = pending.lock() {
                    p.push((preview_key(&bus, &text, key), pic));
                }
            };
            if !done.contains(&preview_key(&bus, &text, "")) {
                put("", sign_picture(&sign, sign.own.clone(), false, &shown));
            }
            let order = chosen.iter().cloned().chain(list.into_iter().filter(|s| chosen.as_ref().is_none_or(|c| c.key() != s.key())));
            for spec in order {
                if worker.load(Ordering::SeqCst) != me {
                    return;
                }
                let key = spec.key();
                if done.contains(&preview_key(&bus, &text, &key)) || unfit.lock().is_ok_and(|u| u.contains(&preview_key(&bus, &text, &key))) {
                    continue;
                }
                let made = std::panic::catch_unwind(|| atlas_for(&sign, &spec, &fonts, &shown, &decode)).ok().flatten();
                match made {
                    // (a vector font that cannot write the destination: not offered for it)
                    Some((_, share)) if spec.is_vector() && share < ENOUGH_LETTERS && chosen.as_ref().is_none_or(|c| !c.same_font(&spec)) => {
                        if let Ok(mut u) = unfit.lock() {
                            u.insert(preview_key(&bus, &text, &key));
                        }
                    }
                    Some((a, _)) => put(&key, sign_picture(&sign, Some(Arc::new(a)), true, &shown)),
                    None if spec.is_vector() => {
                        if let Ok(mut u) = unfit.lock() {
                            u.insert(preview_key(&bus, &text, &key));
                        }
                    }
                    None => {}
                }
            }
        });
    }

    /// The previews drawn since the last frame go to the GPU, those let go off it (`mod.rs`,
    /// where the device is).
    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, gpu: &mut omsi_ui::Gpu) {
        // (a frame after they were let go: this frame's drawing may still name them)
        for t in std::mem::take(&mut self.freeing) {
            gpu.free(t);
        }
        self.freeing = std::mem::take(&mut self.stale);
        let new: Vec<(String, Picture)> = self.pending.lock().map(|mut p| std::mem::take(&mut *p)).unwrap_or_default();
        for (key, pic) in new {
            if self.textures.contains_key(&key) {
                continue;
            }
            let id = gpu.add_image(device, queue, pic.w, pic.h, &pic.rgba);
            self.textures.insert(key, (id, pic.w, pic.h));
        }
    }

    /// Everything on the GPU went with it: the previews are drawn again when next shown.
    pub fn drop_gpu(&mut self) {
        self.textures.clear();
        self.stale.clear();
        self.freeing.clear();
        self.drawing = None;
    }

    /// Do what the row asked for `bus` (under the OMSI folder `root`).
    pub fn edit(&mut self, root: &str, bus: &str, e: Edit) {
        match e {
            Edit::Pick(font) => {
                self.choices.set(bus, font.as_deref());
                self.note = None;
                self.save();
            }
            Edit::Choose(font) => {
                // (the settings chosen so far stay with the next font)
                let style = self.choices.spec_for(bus);
                let font = font.map(|mut f| {
                    if let Some(s) = style.as_ref() {
                        (f.rows, f.bold, f.spacing) = (s.rows, s.bold, s.spacing);
                    }
                    f
                });
                self.choices.set_spec(bus, font.as_ref());
                self.note = None;
                self.save();
            }
            Edit::Style { rows, bold, spacing } => {
                if let Some(mut f) = self.choices.spec_for(bus) {
                    (f.rows, f.bold, f.spacing) = (rows, bold, spacing);
                    self.choices.set_spec(bus, Some(&f));
                    self.save();
                }
            }
            Edit::Source(n) => self.source = Some(n.min(SOURCES.len() - 1)),
            Edit::Add => self.add(root, bus),
        }
    }

    fn save(&self) {
        if let Some(f) = &self.file {
            if let Err(err) = self.choices.write(f) {
                log::warn!("display fonts: {} not written: {err}", f.display());
            }
        }
    }

    /// "Add font…": a font file chosen in a file dialog into the content folder's `Fonts`, and
    /// its first font chosen for `bus`.
    fn add(&mut self, root: &str, bus: &str) {
        let Some(file) = omsi_launcher_lib::pick_file_of(&tr("Add a display font"), &tr("Fonts (.oft, .ttf, .otf, .ttc)"), busfonts::FONT_EXTENSIONS) else { return };
        let Some(dir) = busfonts::content_fonts_dir() else {
            self.note = Some((tr("There is no content folder to add fonts to: set the game under Setup.").into_owned(), true));
            return;
        };
        match busfonts::add_font(&file, &dir) {
            Ok(added) => {
                let mut text = tr("Added: %{fonts}").replace("%{fonts}", &added.names.join(", "));
                if !added.missing.is_empty() {
                    text.push_str(&format!(" · {}", tr("missing beside it: %{files}").replace("%{files}", &added.missing.join(", "))));
                }
                crate::mt::protect([text.as_str()]);
                // (the fonts read again, with the new one; it is chosen for the bus)
                if let Ok(mut f) = self.fonts.lock() {
                    f.remove(root);
                }
                if let Ok(mut f) = self.faces.lock() {
                    f.remove(root);
                }
                self.offered = None;
                self.drawing = None;
                let first = added.names.first().map(|n| match added.file.as_deref() {
                    Some(f) => DisplayFontSpec::vector(n, f, 0),
                    None => DisplayFontSpec::named(n),
                });
                if let Some(first) = first {
                    self.source = Some(usize::from(first.is_vector()));
                    self.edit(root, bus, Edit::Choose(Some(first)));
                }
                self.note = Some((text, !added.missing.is_empty()));
            }
            Err(e) => self.note = Some((format!("{e:#}"), true)),
        }
    }
}

/// The list kept in `map` under `root`, read by `read` on a worker the first time it is asked
/// for (None meanwhile).
fn read_once<T: Send + Sync + 'static>(map: &Arc<Mutex<HashMap<String, Slot<Arc<Vec<T>>>>>>, root: &str, read: fn(String) -> Vec<T>) -> Option<Arc<Vec<T>>> {
    let mut slots = map.lock().ok()?;
    match slots.get(root) {
        Some(Slot::Ready(f)) => return Some(f.clone()),
        Some(_) => return None,
        None => {}
    }
    slots.insert(root.to_string(), Slot::Reading);
    let (map, key) = (map.clone(), root.to_string());
    std::thread::spawn(move || {
        let list = std::panic::catch_unwind(|| read(key.clone()));
        if let Ok(mut m) = map.lock() {
            m.insert(key, list.map(|l| Slot::Ready(Arc::new(l))).unwrap_or(Slot::Failed));
        }
    });
    None
}

// --- the row in the bus sheet -----------------------------------------------------------------

/// The display font row at (`x`, `y`), `w` wide, for `bus` with the destination `text`: its
/// heading with "Add font…", what a font changes on the bus, the choice (each font with its
/// sign; OMSI's fonts or TrueType/OpenType ones), the chosen one's sign large under it and its
/// settings. Returns the height it takes and what was clicked.
#[allow(clippy::too_many_arguments)]
pub(super) fn section(ui: &mut Ui, x: f32, y: f32, w: f32, df: &mut DisplayFonts, root: &str, bus: &str, text: &str) -> (f32, Option<Edit>) {
    let touch = ui.input.touch || super::mobile::mobile();
    let top = y;
    let mut edit = None;
    ui.heading(Rect::new(x, y, w, 26.0), "Display font", None);
    // (a phone has no file dialog to add one with)
    if !cfg!(target_os = "android") && !super::mobile::mobile() {
        let label = tr("Add font…");
        let bw = ui.width(&label, 13.0, Weight::Medium) + 44.0;
        let r = Rect::new(x + w - bw, y - 2.0, bw, 26.0);
        if ui.button("display-font-add", r, "Add font…", Some("add"), ButtonKind::Ghost) {
            edit = Some(Edit::Add);
        }
        ui.tooltip(r, "Copy a font into openOMSI's content folder: an OMSI .oft with its bitmaps, or a TrueType / OpenType font (.ttf, .otf, .ttc)");
    }
    let mut y = y + 32.0;
    let note = |ui: &mut Ui, y: &mut f32, words: &str, c: Color| {
        let h = ui.paragraph_height(words, w - 10.0, 12.0, Weight::Regular);
        if ui.rect_visible(Rect::new(x, *y, w, h)) {
            ui.paragraph(words, Vec2::new(x + 2.0, *y), w - 10.0, 12.0, Weight::Regular, c);
        }
        *y += h + 6.0;
    };
    let signs = df.signs(root, bus);
    let fonts = df.fonts(root);
    match (signs, fonts) {
        (Some(BusSigns { plan, sign: None }), _) => {
            let says = plan_text(&plan);
            crate::mt::protect([says.as_str()]);
            note(ui, &mut y, &says, TEXT_DIM);
        }
        (Some(BusSigns { plan, sign: Some(sign) }), Some(fonts)) => {
            // what a font changes on this bus
            let says = plan_text(&plan);
            crate::mt::protect([says.as_str()]);
            note(ui, &mut y, &says, TEXT_DIM);
            let chosen = df.choices.spec_for(bus);
            // OMSI's fonts or TrueType/OpenType ones
            let mut source = df.source_for(bus);
            let seg = Rect::new(x, y, w.min(420.0), if touch { 40.0 } else { 30.0 });
            if ui.rect_visible(seg) && ui.segmented("display-font-source", seg, &mut source, &SOURCES) {
                edit = Some(Edit::Source(source));
            }
            y += seg.h + 8.0;
            let mut entries: Vec<Entry> = if source == 0 {
                let keep = chosen.as_ref().filter(|c| !c.is_vector()).map(|c| c.name.clone());
                df.offered_for(&fonts, text, keep.as_deref()).iter().map(|f| Entry { spec: DisplayFontSpec::named(f.name.trim()), label: f.name.trim().to_string() }).collect()
            } else {
                df.faces(root).map(|faces| faces.iter().map(|f| Entry { spec: f.spec(), label: f.name.clone() }).collect()).unwrap_or_default()
            };
            df.draw(bus, text, &sign, fonts.clone(), entries.iter().map(|e| e.spec.clone()).collect(), chosen.clone());
            // (a vector font found unable to write the destination is not offered for it)
            if source == 1 {
                let unfit = df.unfit.lock().map(|u| u.clone()).unwrap_or_default();
                entries.retain(|e| chosen.as_ref().is_some_and(|c| c.same_font(&e.spec)) || !unfit.contains(&preview_key(bus, text, &e.spec.key())));
            }
            let mut options: Vec<String> = Vec::with_capacity(entries.len() + 2);
            options.push(picture_option(df.picture(bus, text, ""), &tr("As the bus")));
            for e in &entries {
                options.push(picture_option(df.picture(bus, text, &e.spec.key()), &e.label));
            }
            // (a font chosen that is not in this list - the other kind, or no longer installed:
            // still there, so that it can be seen)
            let mut sel = match chosen.as_ref() {
                None => 0,
                Some(c) => match entries.iter().position(|e| e.spec.same_font(c)) {
                    Some(i) => i + 1,
                    None => {
                        let installed = if c.is_vector() { c.file.as_deref().is_some_and(Path::is_file) } else { fonts.iter().any(|f| f.name.trim().eq_ignore_ascii_case(c.name.trim())) };
                        let label = if installed { c.name.clone() } else { tr("%{font} (not installed)").replace("%{font}", &c.name) };
                        options.push(picture_option(df.picture(bus, text, &c.key()), &label));
                        options.len() - 1
                    }
                },
            };
            crate::mt::protect(options.iter().map(String::as_str));
            let field = Rect::new(x, y, w, if touch { 44.0 } else { ROW + 4.0 });
            if source == 1 && df.faces(root).is_none() {
                if ui.rect_visible(field) {
                    ui.text_in("Reading the fonts…", field, 12.0, Weight::Regular, TEXT_FAINT, Align::Left);
                }
            } else if ui.rect_visible(field) && ui.select("display-font", field, &mut sel, &options) {
                let pick = sel.checked_sub(1).and_then(|i| entries.get(i)).map(|e| e.spec.clone());
                // (the one not in the list picked again: kept)
                if sel == 0 || pick.is_some() {
                    edit = Some(Edit::Choose(pick));
                }
            }
            y += field.h + 8.0;
            // the chosen one large: the sign as the bus will show it
            let big = Rect::new(x, y, w, 58.0);
            if ui.rect_visible(big) {
                ui.p().rounded(big, 8.0, Color::rgba(PANEL_RGB[0], PANEL_RGB[1], PANEL_RGB[2], 1.0));
                ui.p().rounded_border(big, 8.0, 1.0, EDGE);
                let shown = df.picture(bus, text, &chosen.as_ref().map(|c| c.key()).unwrap_or_default());
                match shown {
                    Some((tex, pw, ph)) => {
                        let room = big.inset(7.0);
                        let k = (room.h / ph as f32).min(room.w / pw as f32).min(1.0);
                        let (iw, ih) = (pw as f32 * k, ph as f32 * k);
                        ui.image(Rect::new(room.center().x - iw * 0.5, room.center().y - ih * 0.5, iw, ih), tex, 0.0);
                    }
                    None => {
                        ui.text_in("Drawing the sign…", big, 12.0, Weight::Regular, TEXT_FAINT, Align::Center);
                    }
                }
            }
            y += big.h + 6.0;
            let grid = sign.grid();
            let says = match chosen.as_ref() {
                None => tr("As the bus: its own font (%{font}).").replace("%{font}", sign.def.font.trim()),
                Some(_) if sign.displays > 1 => tr("Drawn on its %{n} displays in their own grid of %{rows} rows of dots, in place of %{font}.").replace("%{n}", &sign.displays.to_string()).replace("%{rows}", &grid.rows.to_string()).replace("%{font}", sign.def.font.trim()),
                Some(_) => tr("Drawn in the display's own grid of %{rows} rows of dots, in place of %{font}.").replace("%{rows}", &grid.rows.to_string()).replace("%{font}", sign.def.font.trim()),
            };
            crate::mt::protect([says.as_str()]);
            note(ui, &mut y, &says, TEXT_FAINT);
            // its settings: rows of dots, spacing, bold
            if let Some(c) = chosen.as_ref() {
                let row_h = if touch { 40.0 } else { 30.0 };
                let max_rows = grid.rows.max(3) as f32;
                let mut rows = c.rows.map(|r| r.clamp(3, grid.rows.max(3)) as f32).unwrap_or(2.0);
                let auto = tr("Auto").into_owned();
                let fmt_rows = |v: f32| if v < 2.5 { auto.clone() } else { format!("{}", v as u32) };
                let r = Rect::new(x, y, w, row_h);
                let mut changed = None;
                if ui.rect_visible(r) && ui.slider_on_release("display-font-rows", r, &mut rows, 2.0, max_rows, 1.0, "Rows of dots", &fmt_rows) {
                    changed = Some((if rows < 2.5 { None } else { Some(rows as u32) }, c.bold, c.spacing));
                }
                y += row_h + 4.0;
                let mut spacing = c.spacing.map(|s| s as f32).unwrap_or(-1.0);
                let own = tr("As the font").into_owned();
                let fmt_spacing = |v: f32| if v < -0.5 { own.clone() } else { format!("{}", v as i32) };
                let r = Rect::new(x, y, w, row_h);
                if ui.rect_visible(r) && ui.slider_on_release("display-font-spacing", r, &mut spacing, -1.0, 6.0, 1.0, "Dots between letters", &fmt_spacing) {
                    changed = Some((c.rows, c.bold, if spacing < -0.5 { None } else { Some(spacing as u32) }));
                }
                y += row_h + 4.0;
                let mut bold = c.bold;
                let r = Rect::new(x, y, w, row_h);
                if ui.rect_visible(r) && ui.toggle("display-font-bold", r, &mut bold, "Bold (every stroke a dot wider)") {
                    changed = Some((c.rows, bold, c.spacing));
                }
                y += row_h + 6.0;
                crate::mt::protect([auto.as_str(), own.as_str()]);
                if let Some((rows, bold, spacing)) = changed {
                    edit = Some(Edit::Style { rows, bold, spacing });
                }
            }
        }
        _ => {
            if ui.rect_visible(Rect::new(x, y, w, 20.0)) {
                ui.text_in("Reading the bus's displays…", Rect::new(x + 2.0, y, w, 20.0), 12.0, Weight::Regular, TEXT_FAINT, Align::Left);
            }
            y += 26.0;
        }
    }
    if let Some((words, bad)) = df.note.clone() {
        note(ui, &mut y, &words, if bad { WARN } else { OK });
    }
    (y - top, edit)
}

/// The destination the previews write: the duty's first trip's line and terminus, else `SAMPLE`.
pub fn sample_of(state: &super::state::State) -> String {
    let trip = state.first_trip().and_then(|k| state.tour()?.trips.get(k));
    match trip {
        Some(t) if !t.terminus.trim().is_empty() => format!("{} {}", t.line.trim(), t.terminus.trim()).trim().to_string(),
        _ => SAMPLE.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omsi_content::font::FontChar;

    fn pixel_font(name: &str, h: i32, chars: &str) -> (Font, FontAtlas) {
        let mut x = 0;
        let glyphs: Vec<FontChar> = chars
            .chars()
            .map(|ch| {
                let g = FontChar { ch, x0: x, x1: x + 4, y: 0 };
                x += 4;
                g
            })
            .collect();
        let font = Font { name: name.into(), path: PathBuf::from(format!("{name}.oft")), height: h, gap: 1, chars: glyphs, ..Default::default() };
        let (aw, ah) = (x.max(1) as u32, h as u32);
        let alpha: Vec<u8> = (0..aw * ah).flat_map(|i| if (i % aw) % 4 != 3 { [255u8; 4] } else { [0u8; 4] }).collect();
        (font.clone(), FontAtlas::new(font, aw, ah, alpha.clone(), alpha))
    }

    fn sign(line_h: i32) -> Sign {
        let def = TextTexture { variable: "Matrix_Terminus".into(), font: "Own".into(), width: 128, height: 32, full_color: false, color: [0.0, 0.0, 0.0], orientation: 0, grid: 1 };
        Sign { def, own: None, line_h, displays: 2, role: DisplayRole::Destination }
    }

    #[test]
    fn a_preview_is_the_line_as_the_bus_draws_it_on_a_dark_sign() {
        let (_, a) = pixel_font("Pix 7", 7, "HBF12 ");
        let p = sign_picture(&sign(14), Some(Arc::new(a)), true, "12 HBF");
        // fitted twice (14 rows), magnified three times more to the preview: dots with seams
        assert_eq!(p.h, 14 * 3);
        assert_eq!(p.rgba.len(), (p.w * p.h * 4) as usize);
        assert!(p.rgba.chunks(4).all(|c| c[3] == 255), "opaque");
        let amber = p.rgba.chunks(4).filter(|c| c[0] > 200 && c[1] > 120 && c[2] < 40).count();
        assert!(amber > 0, "black letters are written amber");
        // (a seam between two dots is dark)
        let seam = (2 * p.w * 4) as usize;
        assert!(p.rgba[seam..seam + (p.w * 4) as usize].chunks(4).all(|c| c[0] < 40));
        // a tall font: shrunk to the preview's rows
        let (_, tall) = pixel_font("Big 64", 64, "HBF12 ");
        let mut high = sign(64);
        high.def.height = 80;
        let p = sign_picture(&high, Some(Arc::new(tall)), true, "12 HBF");
        assert_eq!(p.h, PREVIEW_ROWS);
        assert!(p.w <= PREVIEW_MAX_W);
        // no font: an empty sign, not nothing
        let p = sign_picture(&sign(14), None, false, "12 HBF");
        assert!(p.w > 0 && p.h > 0);
    }

    #[test]
    fn the_fonts_offered_can_write_the_destination() {
        let (letters, _) = pixel_font("Letters", 7, "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 ");
        let (digits, _) = pixel_font("Digits", 7, "0123456789");
        let (also, _) = pixel_font("Also letters", 9, "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789");
        let all = vec![letters.clone(), digits.clone(), also.clone(), letters.clone()];
        let names = |v: Vec<Font>| v.into_iter().map(|f| f.name).collect::<Vec<_>>();
        assert_eq!(names(offered(&all, "12 Hauptbahnhof", None)), ["Also letters", "Letters"], "small letters as their capitals; each once");
        assert_eq!(names(offered(&all, "12 Hauptbahnhof", Some("digits"))), ["Also letters", "Digits", "Letters"], "the chosen one stays");
        assert!(coverage(&digits, "12 Hauptbahnhof") < 0.2);
        assert_eq!(coverage(&digits, "  "), 1.0);
    }

    #[test]
    fn the_size_the_game_takes_of_a_font_file() {
        let size = |name: &str, h: i32| Font { name: name.into(), path: PathBuf::from("Krueger.oft"), height: h, ..Default::default() };
        let other = Font { name: "Other".into(), path: PathBuf::from("Other.oft"), height: 5, ..Default::default() };
        let fonts = vec![size("Krueger 7x4", 7), size("Krueger 16x9", 16), other];
        assert_eq!(size_for(&fonts, "Krueger 16x9", 16).map(|f| f.name.as_str()), Some("Krueger 16x9"));
        assert_eq!(size_for(&fonts, "krueger 16x9", 8).map(|f| f.name.as_str()), Some("Krueger 7x4"), "too tall: the size of its file that fits");
        assert_eq!(size_for(&fonts, "Krueger 7x4", 32).map(|f| f.name.as_str()), Some("Krueger 7x4"));
        assert_eq!(size_for(&fonts, "Missing", 16), None);
    }

    /// A vector font's preview is drawn with the atlas the game draws with: the same dots in
    /// the display's grid, as wide as its letters.
    #[test]
    fn a_vector_font_is_previewed_as_the_game_draws_it() {
        let dir = std::env::temp_dir().join(format!("omsi_preview_vector_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("Hanken.ttf");
        std::fs::write(&file, include_bytes!("../../../../assets/fonts/HankenGrotesk/HankenGrotesk-latin-700.ttf")).unwrap();
        // a matrix whose pixels are its LEDs, 16 rows (a script's churafont)
        let s = sign(16);
        let mut spec = DisplayFontSpec::vector("Hanken Grotesk Bold", &file, 0);
        let (a, share) = atlas_for(&s, &spec, &[], "12 Hauptbahnhof", &|_| None).unwrap();
        assert_eq!(share, 1.0);
        assert_eq!(a.font.height, 16);
        let mut game = omsi_sim::texttex::FontLibrary::new(&dir);
        let from_game = game.display_font_for(&spec, None, 16).unwrap();
        let glyph = |a: &FontAtlas, c: char| {
            let g = a.font.chars.iter().find(|g| g.ch == c).unwrap().clone();
            (0..16).flat_map(|y| (g.x0..g.x1).map(move |x| (x, y))).map(|(x, y)| a.alpha[(((g.y + y) as u32 * a.width + x as u32) * 4) as usize]).collect::<Vec<u8>>()
        };
        assert_eq!(glyph(&a, 'H'), glyph(&from_game, 'H'), "the same dots as the game's");
        assert!(glyph(&a, 'H').iter().all(|v| *v == 0 || *v == 255), "on or off, as a matrix");
        // bold: wider
        spec.bold = true;
        let (b, _) = atlas_for(&s, &spec, &[], "H", &|_| None).unwrap();
        assert!(glyph(&b, 'H').len() > glyph(&a, 'H').len());
        let p = sign_picture(&s, Some(Arc::new(a)), true, "12 Hauptbahnhof");
        assert!(p.w > 40 && p.h >= 16);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// What a font changes, said as it is: the Lion's City's line number is the passengers'
    /// display inside and its destination matrix a script's (`VMatrix.osc`, fonts by name).
    #[test]
    fn what_a_font_changes_on_the_bus() {
        use omsi_model::{MaterialDef, MeshDef, Model};
        let dir = std::env::temp_dir().join(format!("omsi_font_plan_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("churaKrueger")).unwrap();
        let matrix = dir.join("churaKrueger").join("VMatrix.osc");
        std::fs::write(&matrix, "{macro:Matrix_frame}\n\"churafont++ 32x16 Bold\" (M.V.GetFontIndex) (S.L.Font_A)\n\"churafont++ Numeric 32x24\" (M.V.GetFontIndex) (S.L.Font_B)\n\"churafont++ Pictogram 32\" (M.V.GetFontIndex) (S.L.Font_Icon)\n\"churafont++ Auxiliary 1\" (M.V.GetFontIndex) (S.L.Font_Aux)\n{end}\n{macro:Matrix_Write}\n0 0 0 (L.L.Font_A) 1 0 \"X\" (M.V.STTextOut)\n\"a.bmp\" 0 (M.V.STLoadTex)\n{end}\n").unwrap();
        let ibis = dir.join("IBIS.osc");
        std::fs::write(&ibis, "{macro:x}\n\"IBIS 5x7\" (M.V.GetFontIndex) (S.L.f)\n0 0 0 (L.L.f) 1 0 \"X\" (M.V.STTextOut)\n{end}\n").unwrap();
        let tt = |v: &str, f: &str| TextTexture { variable: v.into(), font: f.into(), width: 200, height: 50, color: [0.0; 3], grid: 1, ..Default::default() };
        let mesh = |file: &str, text: Option<i32>, transmap: Option<&str>| MeshDef { file: file.into(), materials: vec![MaterialDef { use_text_texture: text, transmap: transmap.map(str::to_string), ..Default::default() }], ..Default::default() };
        // the Lion's City: the line number inside, the matrix outside a script's
        let lions = Model {
            text_textures: vec![tt("IBIS", "IBIS-2_5x7"), tt("Matrix_Liniennummerstring", "DIN Narrow")],
            script_textures: vec![(256, 64), (1024, 256)],
            meshes: vec![mesh("IBIS\\ibis_display.o3d", Some(0), None), mesh("Drucker\\Innenanzeige\\Innenanzeige_Linie.o3d", Some(1), None), mesh("A20\\Aussenanzeige.o3d", None, Some("\\S:1"))],
            ..Default::default()
        };
        let plan = plan_of(&lions, &[matrix.clone(), ibis.clone()]);
        assert!(plan.texts.is_empty(), "the passengers' line number inside is no destination display: {plan:?}");
        assert_eq!(plan.scripts.len(), 1, "the matrix script, not the IBIS's");
        assert_eq!(plan.scripts[0].fonts, ["churafont++ 32x16 Bold", "churafont++ Numeric 32x24"]);
        assert_eq!(plan.scripts[0].kept, ["churafont++ Pictogram 32", "churafont++ Auxiliary 1"]);
        assert!(plan.scripts[0].pictures && plan.pictures && plan.devices);
        let says = plan_text(&plan);
        assert!(says.contains("VMatrix.osc") && says.contains("takes their place") && says.contains("pictograms") && says.contains("stay pictures") && says.contains("IBIS"), "{says}");
        // the same bus with its line number outside as a text display and no font script: the
        // destination its own, only the line number changes
        let mut line_outside = lions.clone();
        line_outside.meshes[1].file = "A20\\Linie_front.o3d".into();
        let plan = plan_of(&line_outside, &[ibis.clone()]);
        assert_eq!(plan.texts, [DisplayRole::LineNumber]);
        assert!(plan.scripts.is_empty() && plan.pictures);
        assert_eq!(plan_text(&plan), "This bus draws its destination itself: a font changes only the line number. The cab's and the saloon's displays (IBIS, printer, inside) keep their own fonts.");
        // a bus of text displays: what they show
        let sd200 = Model { text_textures: vec![tt("Matrix_Terminus", "Annax Small"), tt("Matrix_Nr", "Annax Large"), tt("Matrix_Seite", "Annax Small")], meshes: vec![mesh("matrix.o3d", Some(0), None), mesh("matrix.o3d", Some(1), None), mesh("matrix_side.o3d", Some(2), None)], ..Default::default() };
        let plan = plan_of(&sd200, &[]);
        assert_eq!(plan.texts, [DisplayRole::Destination, DisplayRole::LineNumber, DisplayRole::Side]);
        assert_eq!(plan_text(&plan), "A chosen font changes the destination, the line number, the side sign.");
        // nothing a font draws
        let none = Model { script_textures: vec![(64, 32)], meshes: vec![mesh("front.o3d", None, Some("\\S:0"))], ..Default::default() };
        assert!(plan_text(&plan_of(&none, &[])).contains("does not change them"));
        // a line number's preview writes the line alone
        let mut s = sign(14);
        s.role = DisplayRole::LineNumber;
        assert_eq!(s.text_of("12 Hauptbahnhof"), "12");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Real buses and fonts of the installed OMSI 2 (`OMSI_ROOT`; skipped without it): each
    /// bus's destination display drawn as the game draws it - in its own font and in others -
    /// and the launcher's preview of each, written as PNGs to `OMSI_FONT_SHOTS` when that is
    /// set. No line runs higher than the bus's own font's.
    #[test]
    fn real_buses_destinations_in_other_fonts() {
        let Some(root) = omsi_cfg::env::var_os("OMSI_ROOT").map(PathBuf::from) else {
            eprintln!("skipped: no OMSI_ROOT");
            return;
        };
        let shots = std::env::var_os("OMSI_FONT_SHOTS").map(PathBuf::from);
        if let Some(d) = shots.as_ref() {
            std::fs::create_dir_all(d).unwrap();
        }
        let decode = |p: &Path| omsi_texture::decode_file(p).ok().map(|i| (i.width, i.height, i.rgba));
        let mut lib = omsi_sim::texttex::FontLibrary::new(&root);
        let save = |name: &str, w: u32, h: u32, rgba: &[u8], k: u32| {
            let Some(d) = shots.as_ref() else { return };
            // (laid over black, each pixel k x k: as it is, larger)
            let img = image::RgbaImage::from_fn(w * k, h * k, |x, y| {
                let i = (((y / k) * w + x / k) * 4) as usize;
                image::Rgba([rgba[i], rgba[i + 1], rgba[i + 2], 255])
            });
            img.save(d.join(format!("{name}.png"))).unwrap();
        };
        let windows_font = omsi_content::dotfont::system_font_dirs().into_iter().map(|d| d.join("arialbd.ttf")).find(|f| f.is_file());
        for (bus, text) in [("Vehicles/MAN_SD200/MAN_SD82.bus", "Hauptbahnhof"), ("Vehicles/HH20_EBus2021/HHEBus2021_main.bus", "Rathausmarkt"), ("Vehicles/Citybus 530 by Kajosoft/01a_o530_e2_2.bus", "Dworzec Glowny"), ("Vehicles/BHD_MAN_LionsCity/MAN_A20.bus", "U Feldstraße")] {
            if !root.join(bus).is_file() {
                eprintln!("skipped: no {bus}");
                continue;
            }
            let signs = read_signs(&root, bus).unwrap();
            eprintln!("{bus}: {}", plan_text(&signs.plan));
            let Some(sign) = signs.sign else { continue };
            let stem = Path::new(bus).file_stem().unwrap().to_string_lossy().to_string();
            let g = sign.grid();
            eprintln!("{stem}: {} ({}, {}x{}, line {} px, grid {}/{} px, {} rows), {} displays", sign.def.variable, sign.def.font, sign.def.width, sign.def.height, sign.line_h, g.pitch, g.dot, g.rows, sign.displays);
            let mut specs: Vec<DisplayFontSpec> = ["CRNL_NL2x3_LAWO_16x8", "krueger-font_klein", "Annax Small", "X10_Lawo_3"].iter().map(|n| DisplayFontSpec::named(n)).collect();
            if let Some(f) = windows_font.as_ref() {
                specs.push(DisplayFontSpec::vector("Arial Bold", f, 0));
            }
            let mut own = sign.def.clone();
            own.width = own.width.max(256);
            for spec in std::iter::once(None).chain(specs.iter().map(Some)) {
                let (atlas, fit) = match spec {
                    None => (sign.own.clone(), None),
                    Some(s) => (lib.display_font(s, sign.own.as_deref(), sign.line_h, &decode), Some(sign.line_h as u32)),
                };
                let name = spec.map(|s| s.name.clone()).unwrap_or_else(|| "own".into());
                let Some(atlas) = atlas else {
                    eprintln!("  {name}: not installed");
                    continue;
                };
                let mut s = omsi_sim::texttex::TextTextureState::new(own.clone(), Some(atlas.clone()));
                s.fit = fit;
                let img = s.image(text);
                let (w, h) = (own.width as u32, own.height as u32);
                let rows: Vec<u32> = (0..h).filter(|&y| (0..w).any(|x| img[((y * w + x) * 4 + 3) as usize] > 0)).collect();
                eprintln!("  {name}: rows {:?}..{:?}", rows.first(), rows.last());
                if fit.is_some() && !rows.is_empty() {
                    let top = (h as i32 - sign.line_h) / 2;
                    assert!(rows[0] as i32 >= top && (*rows.last().unwrap() as i32) < top + sign.line_h, "{name} on {bus} runs out of its line");
                }
                let tag = name.replace(' ', "_");
                save(&format!("{stem}_{tag}_texture"), w, h, &img, if w <= 256 { 2 } else { 1 });
                let p = sign_picture(&sign, Some(atlas), fit.is_some(), &format!("12 {text}"));
                save(&format!("{stem}_{tag}_preview"), p.w, p.h, &p.rgba, 1);
            }
        }
    }

    #[test]
    fn the_row_is_translated() {
        let keys = [
            "the destination",
            "the line number",
            "the side sign",
            "the rear sign",
            "This bus's matrix script (%{script}) writes its destination and line number itself, in fonts it asks the game for by name: a chosen font takes their place there, each fitted to the height of the one it replaces.",
            "It also changes %{what}.",
            "The matrix's pictograms and clock keep their own.",
            "Destinations its depot file gives as pictures stay pictures.",
            "This bus draws its destination itself: a font changes only %{what}.",
            "A chosen font changes %{what}.",
            "This bus draws its destination displays as pictures of its own: a display font does not change them.",
            "The cab's and the saloon's displays (IBIS, printer, inside) keep their own fonts.",
            SOURCES[0],
            SOURCES[1],
            "Fonts (.oft, .ttf, .otf, .ttc)",
            "Copy a font into openOMSI's content folder: an OMSI .oft with its bitmaps, or a TrueType / OpenType font (.ttf, .otf, .ttc)",
            "Reading the fonts…",
            "Drawn on its %{n} displays in their own grid of %{rows} rows of dots, in place of %{font}.",
            "Drawn in the display's own grid of %{rows} rows of dots, in place of %{font}.",
            "Auto",
            "As the font",
            "Rows of dots",
            "Dots between letters",
            "Bold (every stroke a dot wider)",
        ];
        for lang in ["nl", "de", "fr", "ru", "uk", "pl"] {
            for k in keys {
                assert!(crate::_rust_i18n_try_translate(lang, k).is_some(), "{lang}: {k}");
            }
        }
        assert_eq!(crate::_rust_i18n_try_translate("nl", "the line number").as_deref(), Some("het lijnnummer"));
    }

    #[test]
    fn a_choice_is_kept_for_the_bus_and_goes_to_the_game() {
        let mut df = DisplayFonts::in_memory();
        assert_eq!(df.font_for("Vehicles/MAN_SD200/SD200.bus"), None);
        df.edit("C:/OMSI", "Vehicles/MAN_SD200/SD200.bus", Edit::Pick(Some("Annax Small".into())));
        assert_eq!(df.font_for("vehicles/man_sd200/sd200.bus").as_deref(), Some("Annax Small"));
        df.edit("C:/OMSI", "Vehicles/MAN_SD200/SD200.bus", Edit::Pick(None));
        assert_eq!(df.font_for("Vehicles/MAN_SD200/SD200.bus"), None);
        assert_eq!(preview_key("Vehicles\\A.bus", "12 X", " Annax Small"), "vehicles/a.bus|12 X|annax small");
        // a vector font, its settings, and another font keeping them
        let bus = "Vehicles/BHD_MAN_LionsCity/MAN_A20.bus";
        df.edit("C:/OMSI", bus, Edit::Choose(Some(DisplayFontSpec::vector("Arial Bold", Path::new("C:/Windows/Fonts/arialbd.ttf"), 0))));
        df.edit("C:/OMSI", bus, Edit::Style { rows: Some(16), bold: true, spacing: Some(2) });
        assert_eq!(df.font_for(bus).as_deref(), Some("Arial Bold|file=C:/Windows/Fonts/arialbd.ttf|rows=16|bold|spacing=2"));
        assert_eq!(df.source_for(bus), 1, "the list of its kind");
        df.edit("C:/OMSI", bus, Edit::Choose(Some(DisplayFontSpec::named("X10_Lawo_3"))));
        assert_eq!(df.font_for(bus).as_deref(), Some("X10_Lawo_3|rows=16|bold|spacing=2"));
        assert_eq!(df.source_for(bus), 0);
        df.edit("C:/OMSI", bus, Edit::Source(1));
        assert_eq!(df.source_for(bus), 1);
        df.edit("C:/OMSI", bus, Edit::Choose(None));
        assert_eq!(df.font_for(bus), None);
    }
}
