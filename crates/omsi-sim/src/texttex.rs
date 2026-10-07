//! `[texttexture]`: textures generated from script string variables with `.oft` fonts.
//!
//! Every text texture is identified by its index in the model's `[texttexture]` list;
//! materials refer to it with `[useTextTexture] n`.

use hashbrown::HashMap;
use omsi_content::dotfont::{DisplayFontSpec, DotFont, DotGrid, VectorFace};
use omsi_content::font::{Font, FontAtlas};
use omsi_model::TextTexture;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct FontLibrary {
    atlases: HashMap<String, Option<Arc<FontAtlas>>>,
    root: std::path::PathBuf,
    /// Every `[newfont]` of every content root's `Fonts/*.oft`, in lookup order; read once.
    index: Option<Vec<Font>>,
    /// Every face of the TrueType/OpenType fonts of the content roots' `Fonts` and of the
    /// system; read once, when a display font's file is not where it was chosen.
    faces: Option<Vec<VectorFace>>,
    /// A display font's dots (by the font and the rows it is made in) and the atlases made of
    /// them for the displays it is drawn on (by the choice, the display's grid and line).
    dots: HashMap<(String, u32), Option<Arc<DotFont>>>,
    drawn: HashMap<(String, DotGrid, u32), Option<Arc<FontAtlas>>>,
}

/// Style words at the end of a font name ("churafont++ 32x8 Bold").
const STYLE_WORDS: &[&str] = &["bold", "heavy", "black", "light", "thin", "medium", "regular", "italic", "narrow", "condensed", "wide"];

/// A font name without its trailing style words, lower case: the family and size.
fn family_of(name: &str) -> String {
    let mut words: Vec<String> = name.split_whitespace().map(|w| w.to_ascii_lowercase()).collect();
    while words.len() > 1 && words.last().map(|w| STYLE_WORDS.contains(&w.as_str())).unwrap_or(false) {
        words.pop();
    }
    words.join(" ")
}

/// The cell size a font's name carries ("Krueger 16x9", "churafont++ Numeric 26x11 Bold"),
/// used to keep a substitute the same size as the font a script asked for.
fn size_of(name: &str) -> Option<(u32, u32)> {
    name.split_whitespace().find_map(|w| {
        let w = w.to_ascii_lowercase();
        let (a, b) = w.split_once('x')?;
        Some((a.parse().ok()?, b.parse().ok()?))
    })
}

impl FontLibrary {
    pub fn new(root: &Path) -> FontLibrary {
        FontLibrary { atlases: HashMap::new(), root: root.to_path_buf(), index: None, faces: None, dots: HashMap::new(), drawn: HashMap::new() }
    }

    /// Every face of the TrueType/OpenType fonts installed: those added to a content root's
    /// `Fonts` (the launcher's "Add font…") first, then the system's (see
    /// `omsi_content::dotfont::system_font_dirs`).
    pub fn vector_faces(&mut self) -> &[VectorFace] {
        if self.faces.is_none() {
            let mut dirs: Vec<PathBuf> = omsi_cfg::content_roots().into_iter().map(|r| r.join("Fonts")).collect();
            if dirs.is_empty() {
                dirs.push(self.root.join("Fonts"));
            }
            dirs.extend(omsi_content::dotfont::system_font_dirs());
            self.faces = Some(dirs.iter().flat_map(|d| omsi_content::dotfont::vector_font_files(d)).flat_map(|f| omsi_content::dotfont::vector_faces(&f)).collect());
        }
        self.faces.as_deref().unwrap_or(&[])
    }

    /// The display font `spec` as a display whose own font is `own` draws it on a line of
    /// `line_h` pixels (see [`display_font_atlas`]); None when the font is not installed.
    pub fn display_font(&mut self, spec: &DisplayFontSpec, own: Option<&FontAtlas>, line_h: i32, decode: &dyn Fn(&Path) -> Option<(u32, u32, Vec<u8>)>) -> Option<Arc<FontAtlas>> {
        let line_h = line_h.max(1);
        let grid = own.map(DotGrid::of_atlas).unwrap_or_else(|| DotGrid::plain(line_h as u32));
        let key = (spec.key(), grid, line_h as u32);
        if let Some(a) = self.drawn.get(&key) {
            return a.clone();
        }
        let dots = self.dots_of(spec, &grid, line_h, decode);
        let atlas = dots.map(|d| Arc::new(display_font_atlas(&d, spec, &grid, line_h as u32)));
        self.drawn.insert(key, atlas.clone());
        atlas
    }

    /// `display_font` with the built-in image decoder.
    pub fn display_font_for(&mut self, spec: &DisplayFontSpec, own: Option<&FontAtlas>, line_h: i32) -> Option<Arc<FontAtlas>> {
        self.display_font(spec, own, line_h, &|p| omsi_texture::decode_file(p).ok().map(|i| (i.width, i.height, i.rgba)))
    }

    /// The dots of the font `spec` names for a display of `grid`: a vector font rasterised
    /// at the rows it is drawn in there, an `.oft` font (the size of its file that fits the
    /// line) in its own grid.
    fn dots_of(&mut self, spec: &DisplayFontSpec, grid: &DotGrid, line_h: i32, decode: &dyn Fn(&Path) -> Option<(u32, u32, Vec<u8>)>) -> Option<Arc<DotFont>> {
        let rows = if spec.is_vector() { vector_rows(spec, grid) } else { 0 };
        let key = (spec.plain().key(), if spec.is_vector() { rows } else { line_h as u32 });
        if let Some(d) = self.dots.get(&key) {
            return d.clone();
        }
        let dots = if spec.is_vector() {
            let data = self.vector_data(spec);
            match data.map(|(d, face)| DotFont::from_vector(&d, face, rows, &omsi_content::dotfont::charset())) {
                Some(Ok(mut d)) => {
                    d.name = spec.name.clone();
                    Some(Arc::new(d))
                }
                Some(Err(e)) => {
                    log::warn!("display font \"{}\": {e}", spec.name);
                    None
                }
                None => None,
            }
        } else {
            self.display_atlas(&spec.name, line_h, decode).map(|a| Arc::new(oft_dots(&a)))
        };
        if dots.is_none() {
            log::warn!("display font \"{}\" is not installed", spec.name);
        }
        self.dots.insert(key, dots.clone());
        dots
    }

    /// The bytes and the face of a vector display font: its file, else (moved, or chosen on
    /// another computer) the installed face of the same name.
    fn vector_data(&mut self, spec: &DisplayFontSpec) -> Option<(Vec<u8>, u32)> {
        if let Some(d) = spec.file.as_deref().and_then(|f| std::fs::read(f).ok().or_else(|| omsi_cfg::vfs::read(f).ok())) {
            return Some((d, spec.face));
        }
        let wanted = spec.name.trim().to_lowercase();
        let face = self.vector_faces().iter().find(|f| f.name.trim().to_lowercase() == wanted)?.clone();
        log::info!("display font \"{}\": not at {:?}, taken from {}", spec.name, spec.file, face.path.display());
        let d = std::fs::read(&face.path).ok().or_else(|| omsi_cfg::vfs::read(&face.path).ok())?;
        Some((d, face.face))
    }

    fn index(&mut self) -> &[Font] {
        if self.index.is_none() {
            // the Fonts folder of every content root (installed mods and archives first, then
            // the installation): a mod's display fonts live in the content folder, and looking
            // only in the installation's `Fonts` left the O530's number plate, matrix and
            // dashboard displays empty. A file of the same name higher up replaces the stock one.
            let mut files = omsi_cfg::read_dir_merged("Fonts");
            if files.is_empty() {
                files = omsi_cfg::vfs::read_dir_paths(&self.root.join("Fonts"));
            }
            // each folder in root order, its files in name order (a listing comes in the file
            // system's order)
            let mut folders: Vec<std::path::PathBuf> = Vec::new();
            for p in &files {
                let d = p.parent().map(|d| d.to_path_buf()).unwrap_or_default();
                if !folders.contains(&d) {
                    folders.push(d);
                }
            }
            files.sort_by_cached_key(|p| (folders.iter().position(|d| Some(d.as_path()) == p.parent()).unwrap_or(usize::MAX), p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()));
            // Two files of one folder naming the same font: the one read later takes its place,
            // as in the original (the LiAZ's `ANX_S.oft` spells its "ц" as "=", and a
            // "ANX_S - копия.oft" lying beside it, read first, left the letter out). A
            // folder of higher priority keeps its fonts over the folders after it.
            let mut all: Vec<Font> = Vec::new();
            let mut folder_of: Vec<usize> = Vec::new();
            for p in files {
                if !p.extension().map(|x| x.eq_ignore_ascii_case("oft")).unwrap_or(false) {
                    continue;
                }
                let folder = folders.iter().position(|d| Some(d.as_path()) == p.parent()).unwrap_or(usize::MAX);
                // `Font::path` is the .oft (read through the VFS): its bitmaps lie next to it
                if let Ok(list) = Font::load_all(&p) {
                    for f in list {
                        let key = f.name.trim().to_ascii_lowercase();
                        match all.iter().position(|o| o.name.trim().to_ascii_lowercase() == key) {
                            Some(k) if folder_of[k] == folder => all[k] = f,
                            Some(_) => {}
                            None => {
                                all.push(f);
                                folder_of.push(folder);
                            }
                        }
                    }
                }
            }
            self.index = Some(all);
        }
        self.index.as_deref().unwrap_or(&[])
    }

    /// Whether a font of exactly this name is installed.
    pub fn has_exact(&mut self, name: &str) -> bool {
        let wanted = name.trim();
        self.index().iter().any(|f| f.name.trim().eq_ignore_ascii_case(wanted))
    }

    /// The font called `name`, or - when there is none - one of the same family and size in
    /// another weight: mods ask for weights their packs never shipped (the Citaro pack's
    /// Krüger matrix wants "churafont++ Numeric 26x11 Bold" and "churafont++ 32x8 Bold";
    /// the fonts are "churafont++ Numeric 26x11" and "churafont++ 32x8"), and without it
    /// the line number stayed off the destination display. A name of no known family (the
    /// depot strings the matrix tries as custom fonts) is still not found.
    fn find(&mut self, name: &str) -> Option<Font> {
        let wanted = name.trim();
        if wanted.is_empty() {
            return None;
        }
        let index = self.index();
        if let Some(f) = index.iter().find(|f| f.name.trim().eq_ignore_ascii_case(wanted)) {
            return Some(f.clone());
        }
        let family = family_of(wanted);
        // the plain weight first, then the others in file order - but only a font of the same
        // size. A dot-matrix display (the Krüger matrix asks for eight sizes by name, from
        // "Krueger 7x4" to "Krueger 16x9") draws its letters cell by cell: substituting
        // another size there does not make the text wider or narrower, it makes it a soup of
        // letter fragments. Without a font of that size the script must hear "no font" (-1)
        // and pick the next size itself, which is what the original does.
        let sibling = index
            .iter()
            .filter(|f| size_of(&f.name) == size_of(wanted))
            .find(|f| f.name.trim().eq_ignore_ascii_case(&family))
            .or_else(|| index.iter().filter(|f| size_of(&f.name) == size_of(wanted)).find(|f| family_of(&f.name) == family));
        let Some(sibling) = sibling else {
            // a display whose font is missing is worth a line in the log: it is the first
            // thing to look at when letters come out wrong on a matrix or a plate
            log::warn!("font \"{wanted}\" is in no Fonts folder of any content root");
            return None;
        };
        log::warn!("font \"{wanted}\" not found; drawing with \"{}\" (same family and size)", sibling.name.trim());
        Some(sibling.clone())
    }

    /// Every font installed - each `[newfont]` of every content root's `Fonts/*.oft` once, in
    /// lookup order (the launcher's list of display fonts).
    pub fn installed(&mut self) -> Vec<Font> {
        self.index().to_vec()
    }

    /// The display font `chosen` (a `[newfont]` name) for a display whose own font is
    /// `line_h` pixels high: the font itself, or - when it is taller than the line - the
    /// tallest size of the same `.oft` file that fits (the Krüger pack's "Krueger 16x9" chosen
    /// for a 7-pixel line number becomes its "Krueger 7x4", not 16x9 squeezed into a smear);
    /// none fitting, its smallest, which `FontAtlas::render_fitted` shrinks. None when no font
    /// of that name is installed.
    pub fn display_atlas(&mut self, chosen: &str, line_h: i32, decode: &dyn Fn(&Path) -> Option<(u32, u32, Vec<u8>)>) -> Option<Arc<FontAtlas>> {
        let wanted = chosen.trim();
        if wanted.is_empty() {
            return None;
        }
        let index = self.index();
        let first = index.iter().position(|f| f.name.trim().eq_ignore_ascii_case(wanted))?;
        let file = index[first].path.clone();
        let family: Vec<(String, i32)> = index.iter().filter(|f| f.path == file).map(|f| (f.name.clone(), f.height)).collect();
        let me = family.iter().position(|(n, _)| n.trim().eq_ignore_ascii_case(wanted)).unwrap_or(0);
        let heights: Vec<i32> = family.iter().map(|f| f.1).collect();
        let k = pick_size(&heights, me, line_h);
        self.get(&family[k].0, decode)
    }

    /// `get` with the built-in image decoder.
    pub fn load(&mut self, name: &str) -> Option<Arc<FontAtlas>> {
        self.get(name, &|p| omsi_texture::decode_file(p).ok().map(|i| (i.width, i.height, i.rgba)))
    }

    /// Fonts are looked up by their `[newfont]` name across all `Fonts/*.oft` files of every
    /// content root (installed mods first, then the installation): a mod's display fonts
    /// live in the content folder, and looking only in the installation's `Fonts` left the
    /// O530's number plate, matrix and dashboard displays empty.
    pub fn get(&mut self, name: &str, decode: &dyn Fn(&Path) -> Option<(u32, u32, Vec<u8>)>) -> Option<Arc<FontAtlas>> {
        let key = name.to_ascii_lowercase();
        if let Some(a) = self.atlases.get(&key) {
            return a.clone();
        }
        let found = self.find(name);
        let fonts_dir = self.root.join("Fonts");
        let atlas = found.and_then(|f| atlas_of(f, &fonts_dir, decode)).map(Arc::new);
        if atlas.is_none() && !name.trim().is_empty() {
            log::warn!("font \"{name}\" not found");
        }
        self.atlases.insert(key, atlas.clone());
        atlas
    }
}

/// A font's colour bitmap laid over its alpha bitmap pixel for pixel. Omsi.exe reads a
/// glyph's colour at the very pixel it reads its coverage at, in the colour bitmap's own
/// rows (0x5d67bc: the same scanline row and byte column in both), whatever size either
/// has - a colour bitmap need not be the alpha's size, and often is a small swatch of the
/// one colour (the stock `EFADfont.bmp` is 128 x 128 under a 128 x 200 alpha). Past its
/// edges, where Omsi.exe reads into other rows or fails, the swatch repeats. (Taken as no
/// colour bitmap at all, a full-colour text came out in the alpha's white, #829.)
fn color_at_alpha_pixels(color: &[u8], cw: u32, ch: u32, aw: u32, ah: u32) -> Vec<u8> {
    let mut out = vec![0u8; (aw * ah * 4) as usize];
    for y in 0..ah {
        for x in 0..aw {
            let s = (((y % ch) * cw + x % cw) * 4) as usize;
            let d = ((y * aw + x) * 4) as usize;
            out[d..d + 4].copy_from_slice(&color[s..s + 4]);
        }
    }
    out
}

/// `font` with its bitmaps, read with `decode` - not kept anywhere (the launcher's previews of
/// every installed font draw each once). The bitmaps sit beside the `.oft` that names them
/// (`fonts_dir` for a font read from nowhere).
pub fn atlas_of(font: Font, fonts_dir: &Path, decode: &dyn Fn(&Path) -> Option<(u32, u32, Vec<u8>)>) -> Option<FontAtlas> {
    let dir = font.path.parent().map(Path::to_path_buf).unwrap_or_else(|| fonts_dir.to_path_buf());
    let alpha_path = omsi_cfg::resolve_path(&dir, &font.alpha);
    let color_path = omsi_cfg::resolve_path(&dir, &font.bitmap);
    let (aw, ah, alpha) = decode(&alpha_path)?;
    // (a colour bitmap of another size is laid over the alpha pixel for pixel, as Omsi.exe
    // reads it - see `color_at_alpha_pixels`)
    let color = match (color_path != alpha_path).then(|| decode(&color_path)).flatten() {
        Some((cw, ch, c)) if (cw, ch) == (aw, ah) => c,
        Some((cw, ch, c)) if cw > 0 && ch > 0 && c.len() == (cw * ch * 4) as usize => color_at_alpha_pixels(&c, cw, ch, aw, ah),
        _ => alpha.clone(),
    };
    Some(FontAtlas::new(font, aw, ah, color, alpha))
}

/// Which size of a display font fits a line `line_h` high (see `FontLibrary::display_atlas`):
/// `heights` are the sizes of one font file, `chosen` the one the player picked. The chosen
/// one when it is no taller than the line (it is magnified by a whole number to fill it);
/// else the tallest that is; none fitting, the smallest.
pub fn pick_size(heights: &[i32], chosen: usize, line_h: i32) -> usize {
    let Some(&own) = heights.get(chosen) else { return 0 };
    if own <= line_h.max(1) {
        return chosen;
    }
    let fitting = heights.iter().enumerate().filter(|(_, &h)| h > 0 && h <= line_h).max_by_key(|(i, &h)| (h, std::cmp::Reverse(*i)));
    match fitting {
        Some((i, _)) => i,
        None => heights.iter().enumerate().filter(|(_, &h)| h > 0).min_by_key(|(i, &h)| (h, *i)).map(|(i, _)| i).unwrap_or(chosen),
    }
}

/// How many rows a vector display font is rasterised in for a display of `grid`: the rows
/// chosen, else the display's own - never more than the display has.
pub fn vector_rows(spec: &DisplayFontSpec, grid: &DotGrid) -> u32 {
    spec.rows.unwrap_or(grid.rows).clamp(3, grid.rows.max(3))
}

/// An `.oft` display font's dots: its letters read in its own grid.
pub fn oft_dots(atlas: &FontAtlas) -> DotFont {
    DotFont::from_atlas(atlas, &DotGrid::of_atlas(atlas))
}

/// The display font `dots` (with the settings of `spec`: its rows, bold, spacing) as a display
/// whose own font draws in `grid` shows it on a line of `line_h` pixels - what the game draws
/// with (a text texture, a script's matrix) and the launcher's previews show.
pub fn display_font_atlas(dots: &DotFont, spec: &DisplayFontSpec, grid: &DotGrid, line_h: u32) -> FontAtlas {
    let mut d = match spec.rows {
        Some(r) if dots.rows > r => dots.resized(r),
        _ => dots.clone(),
    };
    d = d.styled(spec.bold, spec.spacing);
    d.to_atlas(grid, line_h, spec.name.trim())
}

/// Whether `font` writes letters or a line number - not pictograms, a clock, or a script's
/// helper (the Krüger matrix measures hex digits with the widths of its "Auxiliary" fonts):
/// a display font takes its place on a matrix that a script draws.
pub fn is_letter_font(font: &Font) -> bool {
    let name = format!("{} ", font.name.to_lowercase());
    if KEPT_FONT_PARTS.iter().any(|p| name.contains(p)) {
        return false;
    }
    let letters = ('A'..='Z').filter(|&c| font.glyph(c).is_some_and(|g| g.x1 > g.x0)).count();
    let digits = ('0'..='9').filter(|&c| font.glyph(c).is_some_and(|g| g.x1 > g.x0)).count();
    letters >= 13 || digits == 10
}

/// Parts of a font's name a matrix script keeps its own of (see `is_letter_font`).
const KEPT_FONT_PARTS: &[&str] = &["pictogram", "piktogramm", "icon", "symbol", "auxiliary", "aux ", "chrono", "clock", "uhr", "logo", "arrow", "pfeil"];

/// Parts of a script file's name that make it the one drawing the bus's destination signs.
const SIGN_SCRIPT_PARTS: &[&str] = &["matrix", "matr", "ziel", "annax", "krueger", "krüger", "lawo", "mobitec", "aesys", "gorba", "buse", "brose", "flipdot", "rollband", "destination", "kierunk"];

/// ... and those that make it a device of the cab or the saloon, whatever else it says.
const DEVICE_SCRIPT_PARTS: &[&str] = &["ibis", "cockpit", "dash", "drucker", "printer", "ticket", "rbl", "afr", "innen", "interior", "almex", "cashdesk", "kasse", "tacho", "radio"];

/// A script of the vehicle that draws a destination sign into its script textures with fonts
/// it asks the game for by name (`GetFontIndex` then `STTextOut`): the Krüger matrices and
/// their kin. A display font chosen for the bus takes the place of those fonts there, each
/// fitted to the height of the one it replaces.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ScriptSign {
    pub script: PathBuf,
    /// The fonts it asks for that may write letters or the line number (whether a font does
    /// is known once it is loaded: `is_letter_font`).
    pub fonts: Vec<String>,
    /// The fonts it keeps its own: pictograms, clocks, its helpers.
    pub kept: Vec<String>,
    /// It also loads whole pictures of destinations (`STLoadTex`; a depot file's terminus
    /// with a bitmap): those stay pictures.
    pub pictures: bool,
}

/// The scripts among `scripts` (a vehicle's, see `omsi_vehicle::Vehicle::scripts`) that draw
/// destination signs with fonts (see `ScriptSign`), read from their code: the font names
/// handed to `GetFontIndex` as they are written in it.
pub fn script_signs(scripts: &[PathBuf]) -> Vec<ScriptSign> {
    let mut out = Vec::new();
    for path in scripts {
        let name = path.to_string_lossy().replace('\\', "/").to_lowercase();
        // (the file and the folder it is in: `churaKrueger/VMatrix.osc`)
        let tail: String = name.rsplit('/').take(2).collect::<Vec<_>>().join("/");
        if !SIGN_SCRIPT_PARTS.iter().any(|p| tail.contains(p)) || DEVICE_SCRIPT_PARTS.iter().any(|p| tail.contains(p)) {
            continue;
        }
        let program = omsi_script::compile(&omsi_script::CompileInput { scripts: vec![path.clone()], ..Default::default() });
        let used = program.callbacks_used();
        let calls = |c: &str| used.iter().any(|u| u.eq_ignore_ascii_case(c));
        if !calls("STTextOut") || !calls("GetFontIndex") {
            continue;
        }
        let (mut fonts, mut kept) = (Vec::new(), Vec::new());
        for f in program.literal_arguments("getfontindex") {
            let f = f.trim().to_string();
            if f.is_empty() || fonts.iter().chain(kept.iter()).any(|o: &String| o.eq_ignore_ascii_case(&f)) {
                continue;
            }
            let lower = format!("{} ", f.to_lowercase());
            if KEPT_FONT_PARTS.iter().any(|p| lower.contains(p)) {
                kept.push(f);
            } else {
                fonts.push(f);
            }
        }
        if fonts.is_empty() {
            continue;
        }
        out.push(ScriptSign { script: path.clone(), fonts, kept, pictures: calls("STLoadTex") });
    }
    out
}

/// Parts of a name that make a text texture one of the bus's destination displays: the
/// display itself (matrix, Ziel, terminus, line - the SD200's `Matrix_Terminus`, the Hamburg
/// buses' `LW_show_linie`) or a maker of destination signs whose name its font carries
/// (Annax, Lawo, Krüger, Mobitec - the Hamburg buses' `4_LW_Benefit_*`).
const DISPLAY_PARTS: &[&str] = &[
    "matrix", "ziel", "terminus", "linie", "destination", "kierunek", "annax", "lawo", "benefit", "krueger", "krüger", "mobitec", "aesys", "gorba", "hanover", "brose", "buse", "streetberlin", "flipdot", "rollband",
];

/// Whole words of a name that say the same (`17_LED_Klein`, a `front` or `heck` sign).
const DISPLAY_WORDS: &[&str] = &["led", "lw", "front", "side", "seite", "heck", "rear", "back", "line", "route", "dest"];

/// Words of displays that a display font may be meant for only on a mesh that is seen from
/// outside alone.
const WEAK_WORDS: &[&str] = &["lcd", "display", "anzeige", "sign"];

/// Parts of a name that make a text texture something else, whatever else it says: the cab's
/// and the saloon's devices (IBIS, ticket printers, the ALMEX, the dashboard, the passenger
/// information inside - the O530's `interior_display_monitor_kierunek`), plates and fleet
/// numbers, paper.
const NOT_DISPLAY: &[&str] = &[
    "ibis", "drucker", "ticket", "printer", "almex", "kasownik", "faremaster", "validator", "cockpit", "dash", "tacho", "odo", "innen", "interior", "inside", "monitor", "infotainment", "setvar", "radio", "kennz", "plate", "wagennummer", "vehno", "zettel", "handschrift", "schedule", "fahrplan", "menu", "tablet", "rg6000", "segment", "clock", "uhr",
];

/// Whole words of a name that say the same: `LK_Linie` and `LK_Route` are the Lion's City's
/// ticket printer's line and route, `CD_LineTerminus` its cab display, `ianz_*` the MAN SL's
/// saloon displays (Innenanzeige), `ZD_*` a dashboard's central display, `Hst*` the stops a
/// printer lists.
const NOT_DISPLAY_WORDS: &[&str] = &["lk", "cd", "zd", "ianz", "hst"];

/// The words of a name: apart at everything that is no letter or digit, in lower case.
fn name_words(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| w.to_lowercase()).collect()
}

/// Whether a text texture is one of the destination displays a display font is for, by its
/// variable's and its font's names: the sign's own words, and none of a device's. `outside`:
/// a mesh that shows it is seen from outside (see `seen_from_outside`); `outside_only`:
/// every mesh that shows it is seen only from outside (an LCD there is a sign, not a dashboard).
pub fn is_destination_display(tt: &TextTexture, outside: bool, outside_only: bool) -> bool {
    // a plate's and a fleet number's colours come from their font's bitmap
    if tt.full_color || !outside {
        return false;
    }
    let var = tt.variable.trim().to_lowercase();
    if var == "ident" || var == "number" || var.parse::<usize>().is_ok() {
        return false;
    }
    let names = [var, tt.font.trim().to_lowercase()];
    if names.iter().any(|n| NOT_DISPLAY.iter().any(|p| n.contains(p))) {
        return false;
    }
    let words: Vec<String> = names.iter().flat_map(|n| name_words(n)).collect();
    if words.iter().any(|w| NOT_DISPLAY_WORDS.contains(&w.as_str())) {
        return false;
    }
    if names.iter().any(|n| DISPLAY_PARTS.iter().any(|p| n.contains(p))) || words.iter().any(|w| DISPLAY_WORDS.contains(&w.as_str())) {
        return true;
    }
    outside_only && words.iter().any(|w| WEAK_WORDS.contains(&w.as_str()))
}

/// For every text texture of `model`: whether a mesh showing it is seen from outside
/// (`[viewpoint]` 0, all views, or one with the outside view, bit 1), and whether every mesh
/// showing it is seen only from there. One no mesh shows is seen from nowhere (the Lion's
/// City's `Matrix_Liniennummerstring`: its matrix is a script texture).
pub fn seen_from_outside(model: &omsi_model::Model) -> Vec<(bool, bool)> {
    let mut out: Vec<(bool, bool, bool)> = vec![(false, true, false); model.text_textures.len()];
    for m in &model.meshes {
        let outside = m.viewpoint == 0 || m.viewpoint & 1 != 0;
        let only = m.viewpoint == 1;
        for t in m.materials.iter().filter_map(|t| t.use_text_texture) {
            if let Some(o) = usize::try_from(t).ok().and_then(|t| out.get_mut(t)) {
                o.0 |= outside;
                o.1 &= only;
                o.2 = true;
            }
        }
    }
    out.into_iter().map(|(outside, only, used)| if used { (outside, only) } else { (false, false) }).collect()
}

/// For every text texture of `model`: whether every mesh showing it is one of the cab's or
/// the saloon's devices by its file's name (the Lion's City's `Matrix_Liniennummerstring` is
/// the line number of `Drucker\Innenanzeige\Innenanzeige_Linie.o3d`, the passengers' display
/// inside - its outside matrix is a script's). One no mesh shows is none.
pub fn shown_on_devices_only(model: &omsi_model::Model) -> Vec<bool> {
    let mut out: Vec<(bool, bool)> = vec![(false, true); model.text_textures.len()];
    for m in &model.meshes {
        let file = m.file.to_lowercase();
        let words = name_words(&file);
        let device = NOT_DISPLAY.iter().any(|p| file.contains(p)) || words.iter().any(|w| NOT_DISPLAY_WORDS.contains(&w.as_str()));
        for t in m.materials.iter().filter_map(|t| t.use_text_texture) {
            if let Some(o) = usize::try_from(t).ok().and_then(|t| out.get_mut(t)) {
                o.0 = true;
                o.1 &= device;
            }
        }
    }
    out.into_iter().map(|(used, device)| used && device).collect()
}

/// The text textures of `model` that are its destination displays (see
/// `is_destination_display`), by their place in its `[texttexture]` list.
pub fn destination_displays(model: &omsi_model::Model) -> Vec<usize> {
    let seen = seen_from_outside(model);
    let devices = shown_on_devices_only(model);
    model
        .text_textures
        .iter()
        .enumerate()
        .filter(|(i, t)| seen.get(*i).is_some_and(|&(o, only)| is_destination_display(t, o, only)) && !devices.get(*i).copied().unwrap_or(false))
        .map(|(i, _)| i)
        .collect()
}

/// What a destination display shows, by the words of its variable's and font's names: the
/// line number, a side or rear sign, else the destination (at the front).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DisplayRole {
    Destination,
    LineNumber,
    Side,
    Rear,
}

impl DisplayRole {
    pub fn of(tt: &TextTexture) -> DisplayRole {
        let names = format!("{} {}", tt.variable, tt.font).to_lowercase();
        let words = name_words(&names);
        let has = |ws: &[&str]| words.iter().any(|w| ws.contains(&w.as_str()));
        if has(&["heck", "rear", "back", "hinten", "tyl"]) || names.contains("heck") {
            DisplayRole::Rear
        } else if has(&["seite", "side", "bok"]) || names.contains("seite") {
            DisplayRole::Side
        } else if has(&["nr", "nummer", "linie", "line", "route", "number", "liniennummer"]) || names.contains("linie") || names.contains("liniennummer") {
            DisplayRole::LineNumber
        } else {
            DisplayRole::Destination
        }
    }
}

/// Draw the destination displays among `states` (the text textures of `model`, in its
/// order) in the display font `chosen` the player picked for the bus (a
/// `DisplayFontSpec::to_arg`: an `.oft` font's name, or a vector font with its settings), as
/// each display draws letters (`FontLibrary::display_font`: in the grid of dots of the
/// display's own font, a line of its height) - its size, colour and placement stay the bus's.
/// Returns how many it changed (none when the font is missing).
pub fn apply_display_font(states: &mut [TextTextureState], model: &omsi_model::Model, chosen: &str, lib: &mut FontLibrary, decode: &dyn Fn(&Path) -> Option<(u32, u32, Vec<u8>)>) -> usize {
    let Some(spec) = DisplayFontSpec::parse(chosen) else { return 0 };
    let mut changed = 0;
    for i in destination_displays(model) {
        let Some(s) = states.get_mut(i) else { continue };
        // the display's line: its own font's height (a font missing: the texture's)
        let line_h = s.atlas.as_ref().map(|a| a.font.height).filter(|h| *h > 0).unwrap_or(s.def.height).max(1);
        let Some(atlas) = lib.display_font(&spec, s.atlas.as_deref(), line_h, decode) else {
            log::warn!("display font \"{}\" is not installed: the bus's own is kept", spec.name);
            return changed;
        };
        log::info!("display font: {} ({}) drawn in \"{}\" for a line of {line_h} px", s.def.variable, s.def.font, spec.name);
        s.atlas = Some(atlas);
        s.fit = Some(line_h as u32);
        s.last_text = None;
        changed += 1;
    }
    changed
}

/// Runtime state of one `[texttexture]`.
#[derive(Clone)]
pub struct TextTextureState {
    pub def: TextTexture,
    pub atlas: Option<Arc<FontAtlas>>,
    pub last_text: Option<String>,
    /// Latest rendered RGBA image, present when it changed since the last upload.
    pub pending: Option<Vec<u8>>,
    /// Drawn in a display font the player chose instead of the bus's own: the height of a
    /// line of the bus's own font, which the chosen one is fitted to (`render_fitted`).
    pub fit: Option<u32>,
}

impl TextTextureState {
    pub fn new(def: TextTexture, atlas: Option<Arc<FontAtlas>>) -> Self {
        Self { def, atlas, last_text: None, pending: None, fit: None }
    }

    /// Re-render when the string variable changed. Returns true when a new image is pending.
    pub fn update(&mut self, text: &str) -> bool {
        if self.last_text.as_deref() == Some(text) {
            return false;
        }
        self.last_text = Some(text.to_string());
        self.pending = Some(self.image(text));
        true
    }

    /// The picture of `text` in this texture's font, size, colour and placement.
    pub fn image(&self, text: &str) -> Vec<u8> {
        let (w, h) = (self.def.width.max(1) as u32, self.def.height.max(1) as u32);
        let rgb = [self.def.color[0] as u8, self.def.color[1] as u8, self.def.color[2] as u8];
        let align = omsi_content::font::TextAlign { orientation: self.def.orientation, grid: self.def.grid };
        match (&self.atlas, self.fit) {
            (Some(a), Some(line_h)) => a.render_fitted(text, w, h, self.def.full_color, rgb, align, line_h),
            (Some(a), None) => a.render_aligned(text, w, h, self.def.full_color, rgb, align),
            (None, _) => vec![0u8; (w * h * 4) as usize],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A full-colour font whose colour bitmap is not the alpha's size takes each glyph
    /// pixel's colour from the same pixel of the colour bitmap, as Omsi.exe reads it, not
    /// the alpha's white (#829).
    #[test]
    fn a_colour_bitmap_of_another_size_still_colours_the_glyphs() {
        let dir = std::path::PathBuf::from("/fonts");
        let font = Font { path: dir.join("colour.oft"), name: "Colour".into(), bitmap: "colour.bmp".into(), alpha: "colour_alpha.bmp".into(), height: 4, gap: 0, chars: Vec::new() };
        let mut lib = FontLibrary::new(&dir);
        lib.index = Some(vec![font]);
        // a 4 x 4 white alpha under a 2 x 2 swatch: red, green / blue, yellow
        let swatch = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [255, 255, 0, 255]];
        let decode = |p: &Path| -> Option<(u32, u32, Vec<u8>)> {
            match p.file_name()?.to_str()? {
                "colour_alpha.bmp" => Some((4, 4, vec![255; 64])),
                "colour.bmp" => Some((2, 2, swatch.concat())),
                _ => None,
            }
        };
        let atlas = lib.get("Colour", &decode).expect("the font loads");
        assert_eq!((atlas.width, atlas.height), (4, 4));
        for (x, y) in [(0usize, 0usize), (1, 0), (0, 1), (1, 1), (3, 2)] {
            let i = (y * 4 + x) * 4;
            assert_eq!(atlas.color[i..i + 4], swatch[(y % 2) * 2 + x % 2], "pixel ({x}, {y})");
        }
    }

    #[test]
    fn font_families() {
        assert_eq!(family_of("churafont++ Numeric 26x11 Bold"), "churafont++ numeric 26x11");
        assert_eq!(family_of("churafont++ 32x10 Heavy"), "churafont++ 32x10");
        assert_eq!(family_of("churafont++ 14x10"), "churafont++ 14x10");
        assert_eq!(family_of("Bold"), "bold");
        assert_eq!(family_of("LEERFELD"), "leerfeld");
    }

    fn tt(variable: &str, font: &str) -> TextTexture {
        TextTexture { variable: variable.into(), font: font.into(), width: 512, height: 128, full_color: false, color: [255.0, 160.0, 0.0], orientation: 0, grid: 1 }
    }

    /// The installed buses' text textures (OMSI 2's `Vehicles`): their destination displays,
    /// and the cab's, the saloon's and the plates' that a display font must leave alone.
    #[test]
    fn destination_displays_by_their_names() {
        let sign = |v: &str, f: &str| is_destination_display(&tt(v, f), true, false);
        // MAN SD200, Kajosoft O530, Hamburg electric bus and city bus 2017
        assert!(sign("Matrix_Terminus", "Annax Small"));
        assert!(sign("Matrix_Nr", "Annax Large"));
        assert!(sign("Matrix_Nr", "StreetBerlin"));
        assert!(sign("LW_show_linie", "4_LW_Benefit_Linie"));
        assert!(sign("LW_show_zeile1", "4_LW_Benefit_Klein"));
        assert!(sign("LW_show_voll", "17_LED_Gross"));
        assert!(sign("stringoutput_zielG_A", "krueger-font_gross"));
        assert!(sign("Rollband_Dest1", "SG_Rlbnd_Dest"));
        // the cab, the saloon, the ticket machines, the dashboards
        for (v, f) in [
            ("IBIS", "IBIS_5x7"),
            ("ticketprinter_display", "IBIS-2_5x7"),
            ("cockpit_temperatur", "LCD_7-Segment"),
            ("odometer", "NLC_LCD_7-Segment"),
            ("innenanz_hst", "HH4_barlow"),
            ("interior_display_monitor_kierunek", "KJT3D_monitor"),
            ("almex_s_ziel", "HH20_HHAschedule_font"),
            ("Drucker_TerminusString", "DIN Narrow"),
            ("Faremaster_terminus_name", "O560_US_MSFont_Faremaster"),
            ("IBIS_cabindisplay", "LCD-Innenanzeige"),
            ("Innenanzeige_Haltestelle", "nlc_Innenanzeige_8px"),
            ("rg6_przystanki", "KJT3D_rg6000"),
            ("LK_Linie", "DIN Narrow"),
            ("CD_LineTerminus", "DIN Narrow"),
            ("ianz_Brose_LVA", "SG_Brose_LVA"),
            ("number", "21_vehno"),
            ("number", "Annax Large"),
        ] {
            assert!(!sign(v, f), "{v} / {f}");
        }
        // a plate (its colours from the font) is none, nor is a display seen from the cab only
        assert!(!is_destination_display(&tt("Kennzeichen_Ziel", "Kennz_DtAlt"), true, false));
        let mut coloured = tt("Matrix_Terminus", "Annax Small");
        coloured.full_color = true;
        assert!(!is_destination_display(&coloured, true, true));
        assert!(!is_destination_display(&tt("ident", "nlc_Kennzeichen"), true, true));
        assert!(!is_destination_display(&tt("Matrix_Terminus", "Annax Small"), false, false));
        // an LCD or a display of no other name: a sign only where it is seen from outside alone
        assert!(is_destination_display(&tt("Anzeige_aussen", "MyFont"), true, true));
        assert!(!is_destination_display(&tt("Anzeige_aussen", "MyFont"), true, false));
        assert!(is_destination_display(&tt("front_text", "MyFont"), true, false));
        // ("led" a word, not a part of one)
        assert!(!sign("filled_text", "Called"));
    }

    #[test]
    fn displays_seen_from_outside() {
        use omsi_model::{MaterialDef, MeshDef, Model};
        let mesh = |vp: i32, tex: &[i32]| MeshDef { viewpoint: vp, materials: tex.iter().map(|t| MaterialDef { use_text_texture: Some(*t), ..Default::default() }).collect(), ..Default::default() };
        let model = Model {
            text_textures: vec![tt("Matrix_Terminus", "Annax Small"), tt("Matrix_Nr", "Annax Large"), tt("Anzeige", "X"), tt("Matrix_script", "Y"), tt("Anzeige_heck", "Z")],
            // 0: all views and the cab; 1: the cab alone; 2: outside alone; 3: none; 4: outside + passengers
            meshes: vec![mesh(0, &[0]), mesh(2, &[0, 1]), mesh(1, &[2]), mesh(5, &[4])],
            ..Default::default()
        };
        assert_eq!(seen_from_outside(&model), vec![(true, false), (false, false), (true, true), (false, false), (true, false)]);
        assert_eq!(destination_displays(&model), vec![0, 2, 4]);
    }

    #[test]
    fn a_display_font_size_that_fits_the_line() {
        // (Krueger 7x4, 9x5, 16x9 of one file)
        let h = [7, 9, 16];
        assert_eq!(pick_size(&h, 0, 16), 0, "smaller: kept, magnified twice");
        assert_eq!(pick_size(&h, 2, 16), 2);
        assert_eq!(pick_size(&h, 2, 12), 1, "taller than the line: the tallest that fits");
        assert_eq!(pick_size(&h, 2, 8), 0);
        assert_eq!(pick_size(&h, 2, 5), 0, "none fits: the smallest, shrunk");
        assert_eq!(pick_size(&[32], 0, 16), 0);
        assert_eq!(pick_size(&[], 0, 16), 0);
    }

    /// The installed OMSI 2's buses (`OMSI_ROOT`; skipped without it): the destination
    /// displays each model has (printed), the SD200's matrix among them and none of a cab's
    /// IBIS or a plate.
    #[test]
    fn installed_buses_destination_displays() {
        let Some(root) = omsi_cfg::env::var_os("OMSI_ROOT").map(std::path::PathBuf::from) else {
            eprintln!("skipped: no OMSI_ROOT");
            return;
        };
        let mut found: Vec<(String, String)> = Vec::new();
        for bus in std::fs::read_dir(root.join("Vehicles")).into_iter().flatten().flatten() {
            let model_dir = bus.path().join("model");
            for cfg in std::fs::read_dir(&model_dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("cfg"))) {
                let Ok(model) = omsi_model::Model::load(&cfg) else { continue };
                let shown: Vec<String> = destination_displays(&model).iter().map(|&i| format!("{} ({})", model.text_textures[i].variable, model.text_textures[i].font)).collect();
                if !shown.is_empty() {
                    eprintln!("{}: {}", cfg.strip_prefix(&root).unwrap_or(&cfg).display(), shown.join(", "));
                }
                let dir = bus.file_name().to_string_lossy().to_string();
                found.extend(destination_displays(&model).iter().map(|&i| (dir.clone(), model.text_textures[i].variable.clone())));
            }
        }
        if found.iter().any(|(d, _)| d == "MAN_SD200") {
            assert!(found.iter().any(|(d, v)| d == "MAN_SD200" && v == "Matrix_Terminus"));
        }
        assert!(!found.iter().any(|(_, v)| v.eq_ignore_ascii_case("ibis") || v.eq_ignore_ascii_case("ident")), "{found:?}");
    }

    /// A text texture fitted to a display font draws it in the bus's own size and place.
    #[test]
    fn a_display_font_keeps_the_display_size_colour_and_place() {
        use omsi_content::font::{Font, FontChar};
        // a 5x7 pixel font: one letter, solid
        let font = Font { name: "Pix 7".into(), height: 7, gap: 1, chars: vec![FontChar { ch: 'A', x0: 0, x1: 5, y: 0 }], ..Default::default() };
        let alpha: Vec<u8> = (0..5 * 7).flat_map(|_| [255u8, 255, 255, 255]).collect();
        let atlas = Arc::new(FontAtlas::new(font, 5, 7, alpha.clone(), alpha));
        let mut def = tt("Matrix_Terminus", "Annax Small");
        (def.width, def.height) = (40, 16);
        let mut s = TextTextureState::new(def, Some(atlas));
        s.fit = Some(16);
        let img = s.image("A");
        let ink: Vec<(usize, usize)> = (0..16).flat_map(|y| (0..40).map(move |x| (x, y))).filter(|(x, y)| img[(y * 40 + x) * 4 + 3] > 0).collect();
        // twice the size (10 x 14), centred, the bus's colour, every pixel whole
        assert_eq!(ink.len(), 10 * 14);
        assert_eq!(ink.iter().map(|p| p.0).min(), Some(15));
        assert_eq!(ink.iter().map(|p| p.1).min(), Some(1));
        assert!(ink.iter().all(|(x, y)| img[(y * 40 + x) * 4..(y * 40 + x) * 4 + 4] == [255, 160, 0, 255]));
    }
}
