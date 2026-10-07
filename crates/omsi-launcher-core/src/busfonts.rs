//! The display fonts the player chose in the launcher's bus step: an OMSI `.oft` font (by its
//! `[newfont]` name) or a TrueType/OpenType font (a face of a `.ttf`, `.otf` or `.ttc` file -
//! one added to the content folder, or one of the system's), with the settings that matter on
//! a display (its rows of dots, bold, the spacing; see `omsi_content::dotfont`), that the bus's
//! destination displays are drawn in instead of the fonts the bus came with. They are kept per
//! bus file in `~/.openomsi/bus-fonts.json`, beside the bus options (a bus keeps its own
//! whatever is driven in between), with a default for every bus that has none of its own, and
//! go to the game as `--display-font` (see `duty_args`).
//!
//! A choice of an `.oft` font without settings is kept as its name alone, as the file had it
//! before vector fonts and the settings; the others as an object - and a file of either kind
//! loads.
//!
//! `BusFonts::font_for` is also what an AI bus of the same file would take - the company's
//! buses on the player's own lines, once the game is told which those are (not yet: only the
//! player's bus gets it).

use anyhow::{anyhow, Context, Result};
use omsi_content::dotfont::DisplayFontSpec;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::busoptions::bus_key;

/// One choice as the file keeps it: a font's name alone, or the font with its settings.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(untagged)]
enum Kept {
    Name(String),
    Font(KeptFont),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
struct KeptFont {
    font: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    file: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "is_zero")]
    face: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rows: Option<u32>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    bold: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    spacing: Option<u32>,
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}

impl Kept {
    fn of(spec: &DisplayFontSpec) -> Kept {
        if *spec == DisplayFontSpec::named(&spec.name) {
            return Kept::Name(spec.name.trim().to_string());
        }
        Kept::Font(KeptFont { font: spec.name.trim().to_string(), file: spec.file.clone(), face: spec.face, rows: spec.rows, bold: spec.bold, spacing: spec.spacing })
    }

    /// The font (None: "as the bus", kept to say no to a default).
    fn spec(&self) -> Option<DisplayFontSpec> {
        let spec = match self {
            Kept::Name(n) => DisplayFontSpec::named(n),
            Kept::Font(k) => DisplayFontSpec { name: k.font.trim().to_string(), file: k.file.clone(), face: k.face, rows: k.rows, bold: k.bold, spacing: k.spacing },
        };
        (!spec.name.is_empty()).then_some(spec)
    }
}

/// The choices: per bus file (see `bus_key`) the font with its settings ("" for the bus's own
/// whatever the default); and the default.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct BusFonts {
    /// The font of every bus without a choice of its own (none: each as the bus has it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default: Option<Kept>,
    #[serde(default)]
    buses: BTreeMap<String, Kept>,
}

impl BusFonts {
    /// Where they are kept.
    pub fn path() -> PathBuf {
        crate::data_dir().join("bus-fonts.json")
    }

    /// The choices kept (none when there is no file or it cannot be read).
    pub fn load() -> BusFonts {
        Self::read(&Self::path())
    }

    pub fn read(path: &Path) -> BusFonts {
        std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.write(&Self::path())
    }

    pub fn write(&self, path: &Path) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, text)
    }

    /// The font `bus`'s destination displays are drawn in, as the game takes it
    /// (`--display-font`, `DisplayFontSpec::to_arg`): its own choice, else the default; None
    /// as the bus has them.
    pub fn font_for(&self, bus: &str) -> Option<String> {
        self.spec_for(bus).map(|s| s.to_arg())
    }

    /// The font with its settings that `bus` is drawn in (see `font_for`).
    pub fn spec_for(&self, bus: &str) -> Option<DisplayFontSpec> {
        match self.buses.get(&bus_key(bus)) {
            Some(k) => k.spec(),
            None => self.default.as_ref().and_then(Kept::spec),
        }
    }

    /// The choice made for `bus` itself: None when there is none (the default holds),
    /// Some(None) when it is to be as the bus whatever the default.
    pub fn chosen(&self, bus: &str) -> Option<Option<DisplayFontSpec>> {
        self.buses.get(&bus_key(bus)).map(Kept::spec)
    }

    /// Choose the font `font` for `bus` (a font's name, or a `DisplayFontSpec` argument);
    /// None: as the bus (kept as "" while a default would say otherwise, else nothing kept).
    pub fn set(&mut self, bus: &str, font: Option<&str>) {
        self.set_spec(bus, font.and_then(DisplayFontSpec::parse).as_ref());
    }

    /// Choose `font` with its settings for `bus` (see `set`).
    pub fn set_spec(&mut self, bus: &str, font: Option<&DisplayFontSpec>) {
        let key = bus_key(bus);
        match font.filter(|f| !f.name.trim().is_empty()) {
            Some(f) => {
                self.buses.insert(key, Kept::of(f));
            }
            None if self.default.is_some() => {
                self.buses.insert(key, Kept::Name(String::new()));
            }
            None => {
                self.buses.remove(&key);
            }
        }
    }

    /// The font of every bus without a choice of its own (None: each as the bus).
    pub fn set_default(&mut self, font: Option<&str>) {
        self.default = font.and_then(DisplayFontSpec::parse).map(|s| Kept::of(&s));
        if self.default.is_none() {
            // ("as the bus" was only kept to say no to the default)
            self.buses.retain(|_, f| f.spec().is_some());
        }
    }
}

/// What `add_font` brought.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AddedFont {
    /// The fonts in the file (`[newfont]` names, a vector font's faces' names), in its order.
    pub names: Vec<String>,
    /// The bitmaps it names that were not beside it (the font is there, but draws nothing
    /// until they are).
    pub missing: Vec<String>,
    /// A TrueType/OpenType font: the file as it is in the `Fonts` folder now (its faces are
    /// `names`, in their order).
    pub file: Option<PathBuf>,
}

/// The kinds of font file `add_font` takes.
pub const FONT_EXTENSIONS: &[&str] = &["oft", "ttf", "otf", "ttc"];

/// Copy the font file `file` into `fonts_dir` - the `Fonts` folder of openOMSI's content
/// folder (`content_dir`), never the OMSI installation's - so that the game and the launcher
/// find it among the installed fonts: an OMSI `.oft` font with the bitmaps its `[newfont]`
/// blocks name (looked for beside it as the game looks for them, in folders below it too), or
/// a TrueType/OpenType font file.
pub fn add_font(file: &Path, fonts_dir: &Path) -> Result<AddedFont> {
    if omsi_content::dotfont::is_vector_font(file) {
        return add_vector_font(file, fonts_dir);
    }
    let oft = file;
    if !oft.extension().is_some_and(|x| x.eq_ignore_ascii_case("oft")) {
        return Err(anyhow!("{} is no font (.oft, .ttf, .otf, .ttc)", oft.display()));
    }
    let fonts = omsi_content::font::Font::load_all(oft).map_err(|e| anyhow!("{}: {e}", oft.display()))?;
    if fonts.is_empty() {
        return Err(anyhow!("{} defines no font", oft.display()));
    }
    let from = oft.parent().map(Path::to_path_buf).unwrap_or_default();
    std::fs::create_dir_all(fonts_dir).with_context(|| format!("creating {}", fonts_dir.display()))?;
    let name = oft.file_name().context("a font file without a name")?;
    let target = fonts_dir.join(name);
    if !same_file(oft, &target) {
        std::fs::copy(oft, &target).with_context(|| format!("copying {} to {}", oft.display(), target.display()))?;
    }
    let mut out = AddedFont { names: fonts.iter().map(|f| f.name.trim().to_string()).collect(), ..Default::default() };
    let mut done: Vec<String> = Vec::new();
    for rel in fonts.iter().flat_map(|f| [f.bitmap.trim(), f.alpha.trim()]) {
        let key = rel.replace('\\', "/").to_ascii_lowercase();
        if rel.is_empty() || done.contains(&key) {
            continue;
        }
        done.push(key);
        let src = omsi_cfg::resolve_path(&from, rel);
        if !src.is_file() {
            out.missing.push(rel.to_string());
            continue;
        }
        // (where the font file says, below the Fonts folder: no way out of it)
        let parts: Vec<&str> = rel.split(['/', '\\']).filter(|p| !p.is_empty() && *p != "." && *p != "..").collect();
        let dst = parts.iter().fold(fonts_dir.to_path_buf(), |p, s| p.join(s));
        if let Some(d) = dst.parent() {
            std::fs::create_dir_all(d).with_context(|| format!("creating {}", d.display()))?;
        }
        if !same_file(&src, &dst) {
            std::fs::copy(&src, &dst).with_context(|| format!("copying {} to {}", src.display(), dst.display()))?;
        }
    }
    omsi_cfg::content_changed();
    Ok(out)
}

/// Whether two paths are the same file (a font picked from where it already is: nothing to
/// copy).
fn same_file(a: &Path, b: &Path) -> bool {
    a.canonicalize().ok().zip(b.canonicalize().ok()).is_some_and(|(a, b)| a == b)
}

/// `add_font` of a TrueType/OpenType file: copied as it is, its faces' names read.
fn add_vector_font(file: &Path, fonts_dir: &Path) -> Result<AddedFont> {
    let faces = omsi_content::dotfont::vector_faces(file);
    if faces.is_empty() {
        return Err(anyhow!("{} is no TrueType or OpenType font", file.display()));
    }
    std::fs::create_dir_all(fonts_dir).with_context(|| format!("creating {}", fonts_dir.display()))?;
    let target = fonts_dir.join(file.file_name().context("a font file without a name")?);
    if !same_file(file, &target) {
        std::fs::copy(file, &target).with_context(|| format!("copying {} to {}", file.display(), target.display()))?;
    }
    omsi_cfg::content_changed();
    Ok(AddedFont { names: faces.iter().map(|f| f.name.clone()).collect(), missing: Vec::new(), file: Some(target) })
}

/// The content folder's `Fonts`, where `add_font` puts a font (none without a content folder).
pub fn content_fonts_dir() -> Option<PathBuf> {
    crate::content_dir().map(|c| c.join("Fonts"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_font_is_kept_per_bus_file_with_a_default_beside() {
        let mut f = BusFonts::default();
        assert_eq!(f.font_for("Vehicles/MAN_SD200/SD200.bus"), None, "nothing chosen: as the bus");
        f.set("Vehicles\\MAN_SD200\\SD200.bus", Some("Annax Small"));
        assert_eq!(f.font_for("vehicles/man_sd200/sd200.bus").as_deref(), Some("Annax Small"), "whatever the spelling");
        assert_eq!(f.chosen("Vehicles/MAN_SD200/SD200.bus"), Some(Some(DisplayFontSpec::named("Annax Small"))));
        // a default for the others; the SD200 keeps its own
        f.set_default(Some("Krueger 16x9"));
        assert_eq!(f.font_for("Vehicles/HH20_EBus2021/HHEBus2021_main.bus").as_deref(), Some("Krueger 16x9"));
        assert_eq!(f.font_for("Vehicles/MAN_SD200/SD200.bus").as_deref(), Some("Annax Small"));
        // "as the bus" against a default is kept as a choice of its own
        f.set("Vehicles/MAN_SD200/SD200.bus", None);
        assert_eq!(f.font_for("Vehicles/MAN_SD200/SD200.bus"), None);
        assert_eq!(f.chosen("Vehicles/MAN_SD200/SD200.bus"), Some(None));
        // without the default it is nothing kept at all
        f.set_default(None);
        assert_eq!(f.chosen("Vehicles/MAN_SD200/SD200.bus"), None);
        assert_eq!(f, BusFonts::default());
        // a blank name is no font
        f.set("Vehicles/x.bus", Some("  "));
        assert_eq!(f, BusFonts::default());
    }

    #[test]
    fn the_choices_survive_in_their_file() {
        let dir = std::env::temp_dir().join(format!("omsi_bus_fonts_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("bus-fonts.json");
        let mut f = BusFonts::default();
        f.set("Vehicles/MAN_NL_NG/NL202.bus", Some("Annax Medium D"));
        f.set_default(Some("Annax Small"));
        f.write(&file).unwrap();
        assert_eq!(BusFonts::read(&file), f);
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("\"vehicles/man_nl_ng/nl202.bus\": \"Annax Medium D\"") && text.contains("\"default\": \"Annax Small\""), "{text}");
        // a file of before the default, and one that is no file of this
        std::fs::write(&file, r#"{"buses":{"vehicles/x.bus":"A"}}"#).unwrap();
        assert_eq!(BusFonts::read(&file).font_for("Vehicles/X.bus").as_deref(), Some("A"));
        std::fs::write(&file, "not json").unwrap();
        assert_eq!(BusFonts::read(&file), BusFonts::default());
        // a vector font with its settings beside an .oft font as it is: each kept as it was
        // chosen, the plain one as its name (as a file of before has it)
        let mut f = BusFonts::default();
        let mut arial = DisplayFontSpec::vector("Arial Bold", Path::new("C:/Windows/Fonts/arialbd.ttf"), 0);
        arial.rows = Some(16);
        arial.bold = true;
        arial.spacing = Some(2);
        f.set_spec("Vehicles/BHD_MAN_LionsCity/MAN_A20.bus", Some(&arial));
        f.set("Vehicles/MAN_SD200/SD200.bus", Some("X10_Lawo_3"));
        let mut lawo = DisplayFontSpec::named("X10_Lawo_3");
        lawo.bold = true;
        f.set_spec("Vehicles/MAN_NL_NG/NL202.bus", Some(&lawo));
        f.write(&file).unwrap();
        let back = BusFonts::read(&file);
        assert_eq!(back, f);
        assert_eq!(back.spec_for("vehicles/bhd_man_lionscity/man_a20.bus"), Some(arial.clone()));
        assert_eq!(back.font_for("Vehicles/BHD_MAN_LionsCity/MAN_A20.bus").as_deref(), Some("Arial Bold|file=C:/Windows/Fonts/arialbd.ttf|rows=16|bold|spacing=2"));
        assert_eq!(back.spec_for("Vehicles/MAN_NL_NG/NL202.bus"), Some(lawo));
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("\"vehicles/man_sd200/sd200.bus\": \"X10_Lawo_3\""), "{text}");
        assert!(text.contains("\"font\": \"Arial Bold\"") && text.contains("\"rows\": 16") && !text.contains("\"face\""), "{text}");
        // Luc's own file as it was written before
        std::fs::write(&file, "{\n  \"buses\": {\n    \"vehicles/bhd_man_lionscity/man_a20.bus\": \"X10_Lawo_3\"\n  }\n}").unwrap();
        assert_eq!(BusFonts::read(&file).spec_for("Vehicles/BHD_MAN_LionsCity/MAN_A20.bus"), Some(DisplayFontSpec::named("X10_Lawo_3")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_font_comes_into_the_content_folder_with_its_bitmaps() {
        let base = std::env::temp_dir().join(format!("omsi_add_font_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let src = base.join("download");
        std::fs::create_dir_all(src.join("bmp")).unwrap();
        std::fs::write(
            src.join("MyLED.oft"),
            "[newfont]\nMyLED 7\nbmp\\myled.bmp\nbmp\\myled_a.bmp\n7\n1\n\n[char]\nA\n0\n5\n0\n\n[newfont]\nMyLED 16\nmyled16.bmp\nmissing_alpha.bmp\n16\n2\n",
        )
        .unwrap();
        std::fs::write(src.join("bmp").join("myled.bmp"), b"BM").unwrap();
        std::fs::write(src.join("bmp").join("myled_a.bmp"), b"BM").unwrap();
        std::fs::write(src.join("myled16.bmp"), b"BM").unwrap();
        let fonts = base.join("content").join("Fonts");
        let added = add_font(&src.join("MyLED.oft"), &fonts).unwrap();
        assert_eq!(added.names, ["MyLED 7", "MyLED 16"]);
        assert_eq!(added.missing, ["missing_alpha.bmp"]);
        assert!(fonts.join("MyLED.oft").is_file());
        assert!(fonts.join("bmp").join("myled.bmp").is_file() && fonts.join("bmp").join("myled_a.bmp").is_file() && fonts.join("myled16.bmp").is_file());
        // added again (the same font picked twice, or from its new place): the same
        assert_eq!(add_font(&fonts.join("MyLED.oft"), &fonts).unwrap().names, added.names);
        // not a font
        std::fs::write(src.join("notes.txt"), "x").unwrap();
        assert!(add_font(&src.join("notes.txt"), &fonts).is_err());
        std::fs::write(src.join("empty.oft"), "nothing here\n").unwrap();
        assert!(add_font(&src.join("empty.oft"), &fonts).is_err());
        // a TrueType font: copied as it is, its face's name read; a file of that name that is
        // no font is refused
        std::fs::write(src.join("Hanken.ttf"), include_bytes!("../../../assets/fonts/HankenGrotesk/HankenGrotesk-latin-700.ttf")).unwrap();
        let added = add_font(&src.join("Hanken.ttf"), &fonts).unwrap();
        assert_eq!(added.file.as_deref(), Some(fonts.join("Hanken.ttf").as_path()));
        assert!(fonts.join("Hanken.ttf").is_file());
        assert!(added.names.len() == 1 && added.names[0].starts_with("Hanken Grotesk"), "{:?}", added.names);
        std::fs::write(src.join("fake.otf"), "x").unwrap();
        assert!(add_font(&src.join("fake.otf"), &fonts).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }
}
