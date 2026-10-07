//! The livery studio's document: a project with its layers, as Omsi-Hub's Lakstudio keeps it
//! (`shared/lak.ts`: the same JSON, its Dutch field names, its o3d axes), and the pure rules
//! around it - undo and redo, the stripe templates and where they lie, mirroring a decal to the
//! other side, the quick livery, the name a livery may have in OMSI, the `.cti` it is written
//! as, and the size of the texture.
//!
//! Positions are kept in the bus's own frame as the game has it (x right, y forward, z up, in
//! metres) and written to the file in the o3d frame Omsi-Hub uses (x right, y up, z forward).

use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};

// --- the project -----------------------------------------------------------------------------

/// A livery being made, as `project.json` keeps it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Project {
    #[serde(rename = "versie", default = "one")]
    pub version: u32,
    pub id: String,
    /// The name the livery gets in the game (kept once it was saved there).
    #[serde(rename = "naam", default)]
    pub name: String,
    /// The bus file, relative to the OMSI folder.
    pub bus: String,
    #[serde(default = "quick_start")]
    pub start: String,
    /// The livery it was begun from (the base under the layers); empty: the model's own.
    #[serde(rename = "startKleurstelling", default, skip_serializing_if = "Option::is_none")]
    pub start_paint: Option<String>,
    #[serde(rename = "lagen", default)]
    pub layers: Vec<Layer>,
    #[serde(rename = "opties", default)]
    pub options: std::collections::BTreeMap<String, f32>,
    #[serde(rename = "spiegel", default)]
    pub mirror: Mirror,
    #[serde(rename = "gemaakt", default)]
    pub created: String,
    #[serde(rename = "bewaard", default)]
    pub saved: String,
    /// Once it is in the game: under which name and number, and which version of the files.
    #[serde(rename = "geplaatst", default, skip_serializing_if = "Option::is_none")]
    pub placed: Option<Placed>,
    #[serde(rename = "snel", default, skip_serializing_if = "Option::is_none")]
    pub quick: Option<Quick>,
    /// The buses of its family (the same paint texture, a `[CTC]` folder of their own) it is
    /// saved for too.
    #[serde(rename = "ookOp", default, skip_serializing_if = "Vec::is_empty")]
    pub family: Vec<String>,
}

fn one() -> u32 {
    1
}
fn quick_start() -> String {
    "snel".into()
}

/// The ways a livery begins (Omsi-Hub's starts): the quick livery over the livery begun from,
/// that livery as it is, its colours plain on the model's own paint (its logos and lettering
/// gone), or the model's own paint plain.
pub const STARTS: [(&str, &str); 4] = [("snel", "Quick livery"), ("precies", "This livery as it is"), ("effenKleuren", "Its colours, plain"), ("effen", "Plain")];

impl Project {
    /// Whether the livery begins on the model's own paint (a plain start), not the livery's.
    pub fn plain_start(&self) -> bool {
        matches!(self.start.as_str(), "effen" | "effenKleuren")
    }
}
fn yes() -> bool {
    true
}
fn is_true(b: &bool) -> bool {
    *b
}
fn full() -> f32 {
    1.0
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Mirror {
    #[serde(rename = "aan", default = "yes")]
    pub on: bool,
    /// The mirror plane across the bus (x, metres); None: the bus's middle.
    #[serde(rename = "vlakX", default, skip_serializing_if = "Option::is_none")]
    pub plane_x: Option<f32>,
}

impl Default for Mirror {
    fn default() -> Mirror {
        Mirror { on: true, plane_x: None }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Placed {
    #[serde(rename = "naam")]
    pub name: String,
    pub nnnn: u32,
    #[serde(rename = "versie")]
    pub version: u32,
    /// The files written for it (relative to the content folder), to be replaced next time.
    #[serde(rename = "bestanden", default)]
    pub files: Vec<String>,
}

impl Project {
    pub fn new(id: String, bus: String, start_paint: Option<String>, now: String) -> Project {
        Project { version: 1, id, name: String::new(), bus, start: quick_start(), start_paint, layers: Vec::new(), options: Default::default(), mirror: Mirror::default(), created: now.clone(), saved: now, placed: None, quick: None, family: Vec::new() }
    }

    pub fn layer(&self, id: &str) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == id)
    }

    pub fn layer_mut(&mut self, id: &str) -> Option<&mut Layer> {
        self.layers.iter_mut().find(|l| l.id == id)
    }

    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.layers.iter().position(|l| l.id == id)
    }

    /// What undo and redo take back and bring again.
    pub fn doc(&self) -> Doc {
        Doc { layers: self.layers.clone(), mirror: self.mirror.clone(), quick: self.quick.clone() }
    }

    pub fn set_doc(&mut self, d: Doc) {
        self.layers = d.layers;
        self.mirror = d.mirror;
        self.quick = d.quick;
    }
}

/// The part of a project that undo and redo step through.
#[derive(Clone, Debug, PartialEq)]
pub struct Doc {
    pub layers: Vec<Layer>,
    pub mirror: Mirror,
    pub quick: Option<Quick>,
}

// --- layers ----------------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Layer {
    pub id: String,
    #[serde(rename = "naam")]
    pub name: String,
    #[serde(rename = "zichtbaar", default = "yes")]
    pub visible: bool,
    /// 0..1
    #[serde(rename = "dekking", default = "full")]
    pub opacity: f32,
    #[serde(rename = "vergrendeld", default, skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
    /// "Keep details": how much of the base's seams, dirt and shading shows through, 0..1.
    #[serde(default = "full")]
    pub detail: f32,
    /// Paints over rubbers, lamps and grilles too (else only the paint zones take it).
    #[serde(rename = "ookOverRubbers", default, skip_serializing_if = "std::ops::Not::not")]
    pub over_trim: bool,
    /// Paints over the windows too (a print on the glass, window advertising); else the glass
    /// keeps its own.
    #[serde(rename = "ookOverRamen", default, skip_serializing_if = "std::ops::Not::not")]
    pub over_glass: bool,
    #[serde(rename = "uitRecept", default, skip_serializing_if = "std::ops::Not::not")]
    pub from_recipe: bool,
    #[serde(flatten)]
    pub kind: Kind,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "soort")]
pub enum Kind {
    /// The paint of one colour zone of the base, of a paint group, or (radius 1000) all of it.
    #[serde(rename = "zone")]
    Fill {
        /// The zone's colour in CIELAB.
        #[serde(rename = "centrum")]
        centre: [f32; 3],
        /// How far (ΔE) from it a texel still belongs to it.
        #[serde(rename = "straal")]
        radius: f32,
        /// A paint group of the bus instead (`Zones::group`: 0 the body's colour, 1 the second).
        #[serde(rename = "lakgroep", default, skip_serializing_if = "Option::is_none")]
        group: Option<u8>,
        #[serde(rename = "kleur")]
        colour: String,
        #[serde(rename = "verloop", default, skip_serializing_if = "Option::is_none")]
        gradient: Option<Gradient>,
    },
    #[serde(rename = "strook")]
    Stripe {
        #[serde(rename = "sjabloon")]
        template: StripeTemplate,
        /// Lower and upper edge, metres above the bottom of the bus's box.
        h1: f32,
        h2: f32,
        /// Degrees (slanted).
        #[serde(rename = "hoek", default)]
        angle: f32,
        /// Amplitude in metres (wave).
        #[serde(rename = "golf", default)]
        wave: f32,
        #[serde(rename = "zijden", default)]
        sides: Sides,
        #[serde(rename = "kleur")]
        colour: String,
        #[serde(rename = "verloop", default, skip_serializing_if = "Option::is_none")]
        gradient: Option<Gradient>,
    },
    #[serde(rename = "tekst")]
    Text {
        #[serde(rename = "tekst")]
        text: String,
        #[serde(rename = "lettertype", default)]
        font: String,
        /// The letters' height (cm), 2..150.
        #[serde(rename = "hoogteCm")]
        height_cm: f32,
        #[serde(rename = "kleur")]
        colour: String,
        #[serde(rename = "omlijning", default, skip_serializing_if = "Option::is_none")]
        outline: Option<Outline>,
        /// Percent of the letter height between the letters.
        #[serde(rename = "letterafstand", default)]
        spacing: f32,
        #[serde(rename = "verloop", default, skip_serializing_if = "Option::is_none")]
        gradient: Option<Gradient>,
        #[serde(rename = "plaats")]
        place: Place,
    },
    #[serde(rename = "afbeelding")]
    Image {
        /// The picture's hash (its file in the project's `beelden` folder).
        #[serde(rename = "beeld")]
        image: String,
        #[serde(rename = "witDoorzichtig", default)]
        white_clear: bool,
        /// Height over width of the picture.
        #[serde(rename = "verhouding", default, skip_serializing_if = "Option::is_none")]
        aspect: Option<f32>,
        #[serde(rename = "plaats")]
        place: Place,
    },
    #[serde(rename = "vorm")]
    Shape {
        #[serde(rename = "vorm")]
        shape: String,
        #[serde(rename = "kleur")]
        colour: String,
        #[serde(rename = "omlijning", default, skip_serializing_if = "Option::is_none")]
        outline: Option<Outline>,
        #[serde(rename = "verloop", default, skip_serializing_if = "Option::is_none")]
        gradient: Option<Gradient>,
        #[serde(rename = "plaats")]
        place: Place,
    },
    /// A shape of the player's own, drawn with the pen: corners and curves in the decal's box
    /// (0..1 each way, y down), filled even-odd.
    #[serde(rename = "pad")]
    Path {
        #[serde(rename = "punten")]
        nodes: Vec<PathNode>,
        #[serde(rename = "kleur")]
        colour: String,
        #[serde(rename = "omlijning", default, skip_serializing_if = "Option::is_none")]
        outline: Option<Outline>,
        #[serde(rename = "verloop", default, skip_serializing_if = "Option::is_none")]
        gradient: Option<Gradient>,
        #[serde(rename = "plaats")]
        place: Place,
    },
    /// Strokes of the brush and the eraser, as vectors on the bus (Omsi-Hub's `penseel`): what
    /// the brush painted in the layer's colour, the eraser taking it away again.
    #[serde(rename = "penseel")]
    Brush {
        #[serde(rename = "kleur")]
        colour: String,
        #[serde(rename = "streken", default)]
        strokes: Vec<Stroke>,
    },
}

/// One stroke of the brush or the eraser: the points it went through on the bus (its frame,
/// metres) with the surface's normal there, its radius, hardness (1: a sharp edge) and cover.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Stroke {
    #[serde(rename = "punten")]
    pub points: Vec<[f32; 3]>,
    #[serde(rename = "normalen", default)]
    pub normals: Vec<[f32; 3]>,
    #[serde(rename = "straalCm")]
    pub radius_cm: f32,
    #[serde(rename = "hardheid", default = "half")]
    pub hardness: f32,
    #[serde(rename = "dekking", default = "full")]
    pub opacity: f32,
    #[serde(rename = "gum", default)]
    pub erase: bool,
}

fn half() -> f32 {
    0.5
}

impl Kind {
    pub fn place(&self) -> Option<&Place> {
        match self {
            Kind::Text { place, .. } | Kind::Image { place, .. } | Kind::Shape { place, .. } | Kind::Path { place, .. } => Some(place),
            _ => None,
        }
    }
    pub fn place_mut(&mut self) -> Option<&mut Place> {
        match self {
            Kind::Text { place, .. } | Kind::Image { place, .. } | Kind::Shape { place, .. } | Kind::Path { place, .. } => Some(place),
            _ => None,
        }
    }
    /// The layer's main colour (an image has none).
    pub fn colour(&self) -> Option<&str> {
        match self {
            Kind::Fill { colour, .. } | Kind::Stripe { colour, .. } | Kind::Text { colour, .. } | Kind::Shape { colour, .. } | Kind::Path { colour, .. } | Kind::Brush { colour, .. } => Some(colour),
            Kind::Image { .. } => None,
        }
    }
    pub fn set_colour(&mut self, c: String) {
        match self {
            Kind::Fill { colour, .. } | Kind::Stripe { colour, .. } | Kind::Text { colour, .. } | Kind::Shape { colour, .. } | Kind::Path { colour, .. } | Kind::Brush { colour, .. } => *colour = c,
            Kind::Image { .. } => {}
        }
    }
    pub fn gradient_mut(&mut self) -> Option<&mut Option<Gradient>> {
        match self {
            Kind::Fill { gradient, .. } | Kind::Stripe { gradient, .. } | Kind::Text { gradient, .. } | Kind::Shape { gradient, .. } | Kind::Path { gradient, .. } => Some(gradient),
            Kind::Image { .. } | Kind::Brush { .. } => None,
        }
    }
    pub fn gradient(&self) -> Option<&Gradient> {
        match self {
            Kind::Fill { gradient, .. } | Kind::Stripe { gradient, .. } | Kind::Text { gradient, .. } | Kind::Shape { gradient, .. } | Kind::Path { gradient, .. } => gradient.as_ref(),
            Kind::Image { .. } | Kind::Brush { .. } => None,
        }
    }
    /// Whether the copy of this decal on the other side is a mirror image: as the player chose,
    /// else a shape is (an arrow points forward on both sides) and a text or a picture is not
    /// (its letters stay readable). A project from before the choice: Omsi-Hub's "same way
    /// round" made it readable.
    pub fn mirror_image(&self) -> bool {
        let Some(p) = self.place() else { return false };
        p.mirror_image_or(matches!(self, Kind::Shape { .. } | Kind::Path { .. }))
    }
    pub fn outline_mut(&mut self) -> Option<&mut Option<Outline>> {
        match self {
            Kind::Text { outline, .. } | Kind::Shape { outline, .. } | Kind::Path { outline, .. } => Some(outline),
            _ => None,
        }
    }
    /// The kind's name in the interface.
    pub fn label(&self) -> &'static str {
        match self {
            Kind::Fill { group: Some(0), .. } => "Body colour",
            Kind::Fill { group: Some(_), .. } => "Second colour",
            Kind::Fill { radius, .. } if *radius >= 999.0 => "Base colour",
            Kind::Fill { .. } => "Fill",
            Kind::Stripe { .. } => "Stripe",
            Kind::Text { .. } => "Text",
            Kind::Image { .. } => "Image",
            Kind::Shape { .. } => "Shape",
            Kind::Path { .. } => "Own shape",
            Kind::Brush { .. } => "Brush",
        }
    }
    pub fn icon(&self) -> &'static str {
        match self {
            Kind::Fill { .. } => "livery_fill",
            Kind::Stripe { .. } => "livery_stripe",
            Kind::Text { .. } => "livery_text",
            Kind::Image { .. } => "livery_image",
            Kind::Shape { .. } => "livery_shape",
            Kind::Path { .. } => "livery_pen",
            Kind::Brush { .. } => "livery_brush",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum StripeTemplate {
    #[serde(rename = "onderband")]
    Skirt,
    #[serde(rename = "raamband")]
    WindowBand,
    #[serde(rename = "dakband")]
    RoofBand,
    #[serde(rename = "schuin")]
    Slanted,
    #[serde(rename = "golf")]
    Wave,
    #[serde(rename = "tweekleurig")]
    TwoTone,
    #[serde(rename = "frontvlak")]
    Front,
    #[serde(rename = "achtervlak")]
    Rear,
}

impl StripeTemplate {
    pub const ALL: [StripeTemplate; 8] = [StripeTemplate::Skirt, StripeTemplate::WindowBand, StripeTemplate::RoofBand, StripeTemplate::Slanted, StripeTemplate::Wave, StripeTemplate::TwoTone, StripeTemplate::Front, StripeTemplate::Rear];
    pub fn label(self) -> &'static str {
        match self {
            StripeTemplate::Skirt => "Skirt band",
            StripeTemplate::WindowBand => "Window band",
            StripeTemplate::RoofBand => "Roof band",
            StripeTemplate::Slanted => "Slanted",
            StripeTemplate::Wave => "Wave",
            StripeTemplate::TwoTone => "Two-tone",
            StripeTemplate::Front => "Front face",
            StripeTemplate::Rear => "Rear face",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum Sides {
    #[default]
    #[serde(rename = "rondom")]
    All,
    #[serde(rename = "zijden")]
    Sides,
    #[serde(rename = "voor")]
    Front,
    #[serde(rename = "achter")]
    Rear,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Outline {
    #[serde(rename = "kleur")]
    pub colour: String,
    #[serde(rename = "breedteCm")]
    pub width_cm: f32,
}

/// A colour running into a second one, across the layer's box.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Gradient {
    #[serde(rename = "soort", default)]
    pub kind: GradientKind,
    #[serde(rename = "kleur2")]
    pub colour2: String,
    /// Degrees: 0 from left to right (from the rear to the front on the bus), 90 upwards.
    #[serde(rename = "hoek", default)]
    pub angle: f32,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum GradientKind {
    #[default]
    #[serde(rename = "lineair")]
    Linear,
    #[serde(rename = "radiaal")]
    Radial,
}

impl Gradient {
    /// Where (0..1) a point of the box lies along the gradient: `at` is the point in the box,
    /// 0..1 each way with y up.
    pub fn t(&self, at: Vec2) -> f32 {
        let d = at - Vec2::splat(0.5);
        match self.kind {
            GradientKind::Linear => {
                let (s, c) = self.angle.to_radians().sin_cos();
                let reach = 0.5 * (c.abs() + s.abs()).max(1e-3);
                (0.5 + (d.x * c + d.y * s) / reach * 0.5).clamp(0.0, 1.0)
            }
            GradientKind::Radial => (d.length() * 2.0).clamp(0.0, 1.0),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct PathNode {
    pub p: [f32; 2],
    /// The curve's handles into and out of this corner (absent: a sharp corner).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub c1: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub c2: Option<[f32; 2]>,
}

// --- placing a decal ---------------------------------------------------------------------------

/// The side of the bus a decal is projected onto.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Side {
    L,
    R,
    /// The front (voor).
    V,
    /// The rear (achter).
    A,
    /// The roof (dak).
    D,
}

impl Side {
    /// The decal's axes on this side, in the bus frame: `u` across the decal (to the right as
    /// it is seen from outside), `v` up it, `n` out of the bus (the way it is projected from).
    pub fn axes(self) -> (Vec3, Vec3, Vec3) {
        match self {
            Side::L => (Vec3::NEG_Y, Vec3::Z, Vec3::NEG_X),
            Side::R => (Vec3::Y, Vec3::Z, Vec3::X),
            Side::V => (Vec3::NEG_X, Vec3::Z, Vec3::Y),
            Side::A => (Vec3::X, Vec3::Z, Vec3::NEG_Y),
            Side::D => (Vec3::X, Vec3::Y, Vec3::Z),
        }
    }

    /// The side a surface with normal `n` (bus frame) faces most.
    pub fn of_normal(n: Vec3) -> Side {
        let a = n.abs();
        if a.z >= a.x && a.z >= a.y && n.z > 0.0 {
            Side::D
        } else if a.x >= a.y {
            if n.x >= 0.0 { Side::R } else { Side::L }
        } else if n.y >= 0.0 {
            Side::V
        } else {
            Side::A
        }
    }

}

/// Where a text, picture or shape stands on the bus.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Place {
    #[serde(rename = "zijde")]
    pub side: Side,
    /// Its middle in the bus frame (written in o3d axes).
    #[serde(rename = "midden", with = "o3d")]
    pub centre: Vec3,
    #[serde(rename = "breedteM")]
    pub width_m: f32,
    /// Its own height (metres), a shape's or a stretched picture's; None: a shape as wide as
    /// high, a picture in its own proportions (a text's comes from its letters).
    #[serde(rename = "hoogteM", default, skip_serializing_if = "Option::is_none")]
    pub height_m: Option<f32>,
    /// Width and height change together (the size fields); off: the one stretches without the
    /// other. A project from before keeps its proportions.
    #[serde(rename = "verhoudingVast", default = "yes", skip_serializing_if = "is_true")]
    pub keep_ratio: bool,
    /// Degrees, in the side's plane.
    #[serde(rename = "draai", default)]
    pub rotation: f32,
    #[serde(rename = "spiegel", default)]
    pub mirror: Coupling,
    /// Omsi-Hub's flag: the copy on the other side the same way round (a logo with letters)
    /// instead of a mirror image of it. Read for the projects made before `mirror_image`, and
    /// written along with it.
    #[serde(rename = "zelfdeRichting", default, skip_serializing_if = "std::ops::Not::not")]
    pub same_direction: bool,
    /// Whether the copy on the other side is a mirror image of this one (true) or reads the same
    /// way (false); None: as the kind has it (`Kind::mirror_image`).
    #[serde(rename = "spiegelbeeld", default, skip_serializing_if = "Option::is_none")]
    pub mirror_image: Option<bool>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum Coupling {
    #[default]
    #[serde(rename = "gekoppeld")]
    Coupled,
    #[serde(rename = "los")]
    Single,
}

/// `[x, y, z]` of the bus frame as Omsi-Hub's `[x, up, forward]`.
mod o3d {
    use glam::Vec3;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    pub fn serialize<S: Serializer>(v: &Vec3, s: S) -> Result<S::Ok, S::Error> {
        [v.x, v.z, v.y].serialize(s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec3, D::Error> {
        let [x, up, fwd] = <[f32; 3]>::deserialize(d)?;
        Ok(Vec3::new(x, fwd, up))
    }
}

impl Place {
    pub fn new(side: Side, centre: Vec3, width_m: f32) -> Place {
        Place { side, centre, width_m, height_m: None, keep_ratio: true, rotation: 0.0, mirror: Coupling::Coupled, same_direction: false, mirror_image: None }
    }

    /// The width or the height (metres) set in the size fields, the other kept in proportion
    /// while `keep_ratio` holds (else it stays as it was: the decal stretches). `aspect` is a
    /// picture's own height over width, which its height follows until it is stretched (None: a
    /// shape, as high as wide until it has a height of its own).
    pub fn resize(&mut self, width: Option<f32>, height: Option<f32>, aspect: Option<f32>) {
        let lim = |v: f32| v.clamp(0.02, 20.0);
        let w0 = self.width_m.max(1e-3);
        let h0 = self.height_m.unwrap_or(w0 * aspect.unwrap_or(1.0)).max(1e-3);
        if let Some(w) = width {
            let w = lim(w);
            self.height_m = match (self.keep_ratio, self.height_m) {
                (true, Some(_)) => Some(lim(h0 * w / w0)),
                (true, None) => None,
                (false, _) => Some(h0),
            };
            self.width_m = w;
        }
        if let Some(h) = height {
            let h = lim(h);
            if self.keep_ratio {
                self.width_m = lim(self.width_m * h / h0);
                if self.height_m.is_some() {
                    self.height_m = Some(h);
                }
            } else {
                self.height_m = Some(h);
            }
        }
    }

    /// A handle pulled (see [`grip_size`]): the decal gets that size around its middle. A
    /// corner keeps its proportions (a picture or a square shape keeps following its width);
    /// a side handle and a corner pulled freely stretch it, and the size fields stop keeping
    /// the proportions it had.
    pub fn pulled(&mut self, start: &Place, grip: Grip, size: (f32, f32), from: Vec2, now: Vec2, free: bool) {
        let free = free && grip == Grip::Corner;
        let (w, h) = grip_size(grip, size, from, now, free);
        self.width_m = w;
        if grip != Grip::Corner || free {
            self.height_m = Some(h);
            self.keep_ratio = false;
        } else {
            self.height_m = start.height_m.map(|_| h);
        }
    }

    /// Choose whether the copy on the other side is a mirror image (Omsi-Hub's flag kept in
    /// step, so that it reads the project the same).
    /// Whether its copy is a mirror image: as chosen, else `shape` (a shape's is, a text's and a
    /// picture's are not) unless Omsi-Hub's "same way round" says it is not.
    pub fn mirror_image_or(&self, shape: bool) -> bool {
        self.mirror_image.unwrap_or(!self.same_direction && shape)
    }

    pub fn set_mirror_image(&mut self, on: bool) {
        self.mirror_image = Some(on);
        self.same_direction = !on;
    }

    /// The four corners of a decal `w` x `h` metres: lower left, lower right, upper right,
    /// upper left (as Omsi-Hub's `decalHoeken`).
    pub fn corners(&self, w: f32, h: f32) -> [Vec3; 4] {
        let (u, v, _) = self.side.axes();
        let (s, c) = self.rotation.to_radians().sin_cos();
        [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)].map(|(a, b): (f32, f32)| {
            let (a, b) = (a * w, b * h);
            self.centre + u * (a * c - b * s) + v * (a * s + b * c)
        })
    }

    /// Where `p` (bus frame) lies in the decal's own box: x across, y up, metres from its
    /// middle (the inverse of `corners`).
    pub fn local(&self, p: Vec3) -> Vec2 {
        let (u, v, _) = self.side.axes();
        let d = p - self.centre;
        let (du, dv) = (d.dot(u), d.dot(v));
        let (s, c) = self.rotation.to_radians().sin_cos();
        Vec2::new(du * c + dv * s, -du * s + dv * c)
    }
}

/// A handle on a decal's outline: a corner, or the middle of a side (left and right: the
/// width, top and bottom: the height).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grip {
    Corner,
    Width,
    Height,
}

impl Grip {
    /// The handles on the outline of corners `c` (as `Place::corners`): the four corners, then
    /// the middles of the bottom, the right, the top and the left side.
    pub fn handles(c: &[Vec3; 4]) -> [(Grip, Vec3); 8] {
        let mid = |a: usize, b: usize| (c[a] + c[b]) * 0.5;
        [
            (Grip::Corner, c[0]),
            (Grip::Corner, c[1]),
            (Grip::Corner, c[2]),
            (Grip::Corner, c[3]),
            (Grip::Height, mid(0, 1)),
            (Grip::Width, mid(1, 2)),
            (Grip::Height, mid(2, 3)),
            (Grip::Width, mid(3, 0)),
        ]
    }
}

/// The size (metres) a decal of `size` gets when its handle `grip`, grabbed at `from` in the
/// decal's own box (metres from its middle, as `Place::local` has it), is pulled to `now`; its
/// middle stays. A corner scales both ways by the same factor (`free`: each way to the mouse);
/// a side handle stretches its own way only.
pub fn grip_size(grip: Grip, size: (f32, f32), from: Vec2, now: Vec2, free: bool) -> (f32, f32) {
    let lim = |v: f32| v.clamp(0.02, 20.0);
    match grip {
        Grip::Corner if free => (lim(now.x.abs() * 2.0), lim(now.y.abs() * 2.0)),
        Grip::Corner => {
            let k = (now.length() / from.length().max(1e-3)).clamp(0.05, 20.0);
            (lim(size.0 * k), lim(size.1 * k))
        }
        Grip::Width => (lim(now.x.abs() * 2.0), size.1),
        Grip::Height => (size.0, lim(now.y.abs() * 2.0)),
    }
}

/// The copy of a coupled decal on the other side: its place mirrored in the plane `plane_x` -
/// the side turned round, its middle across the plane, its turn the other way (seen from
/// outside, so that a slanted text rises to the front on both sides) - and whether its content
/// is a mirror image (`mirror_image`, flipped back in its own box: else it reads the same way
/// as the original). None for a decal that is not on a side or not coupled.
pub fn mirror_place(p: &Place, plane_x: f32, mirror_image: bool) -> Option<(Place, bool)> {
    if p.mirror != Coupling::Coupled || !matches!(p.side, Side::L | Side::R) {
        return None;
    }
    let side = if p.side == Side::L { Side::R } else { Side::L };
    let centre = Vec3::new(2.0 * plane_x - p.centre.x, p.centre.y, p.centre.z);
    Some((Place { side, centre, rotation: -p.rotation, ..p.clone() }, mirror_image))
}

// --- the bus's measures, the stripes ---------------------------------------------------------

/// The bus's box (bus frame) and its window line (the height, z, of the windows' lower edge).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BusDims {
    pub min: Vec3,
    pub max: Vec3,
    pub window: Option<f32>,
}

impl BusDims {
    pub fn height(&self) -> f32 {
        (self.max.z - self.min.z).max(0.5)
    }
    pub fn length(&self) -> f32 {
        (self.max.y - self.min.y).max(0.5)
    }
    /// The window line above the box's bottom.
    pub fn window_height(&self) -> f32 {
        match self.window {
            Some(w) if w > self.min.z + 0.2 && w < self.max.z => w - self.min.z,
            _ => self.height() * 0.45,
        }
    }
    pub fn middle_x(&self) -> f32 {
        (self.min.x + self.max.x) * 0.5
    }
}

/// A stripe after its template (Omsi-Hub's `strook`): the heights from the box and the window
/// line.
pub fn stripe(template: StripeTemplate, colour: &str, dims: &BusDims) -> Kind {
    let h = dims.height();
    let win = dims.window_height();
    // (a band from the bottom starts under the box: the bounding box often begins above the
    // body's lower edge, and a line of the base was left under the band)
    let under = -0.5;
    let (h1, h2, angle, wave, sides) = match template {
        StripeTemplate::Skirt => (under, (win * 0.35).max(0.2), 0.0, 0.0, Sides::All),
        StripeTemplate::WindowBand => (win - 0.12, win + 0.02, 0.0, 0.0, Sides::All),
        StripeTemplate::RoofBand => (h - 0.35, h + 0.1, 0.0, 0.0, Sides::All),
        StripeTemplate::Slanted => (win * 0.2, win * 0.55, 0.12f32.atan().to_degrees(), 0.0, Sides::Sides),
        StripeTemplate::Wave => (win * 0.3, win * 0.55, 0.0, 0.12, Sides::Sides),
        StripeTemplate::TwoTone => (under, win, 0.0, 0.0, Sides::All),
        StripeTemplate::Front => (under, h + 0.1, 0.0, 0.0, Sides::Front),
        StripeTemplate::Rear => (under, h + 0.1, 0.0, 0.0, Sides::Rear),
    };
    Kind::Stripe { template, h1, h2, angle, wave, sides, colour: colour.to_string(), gradient: None }
}

/// The length of a wave stripe's wave (metres).
pub const WAVE_LENGTH: f32 = 4.0;

/// A stripe's lower and upper edge (metres above the box's bottom) at `y` along the bus.
pub fn stripe_edges(h1: f32, h2: f32, angle: f32, wave: f32, y: f32, dims: &BusDims) -> (f32, f32) {
    let mid = (dims.min.y + dims.max.y) * 0.5;
    let rise = angle.to_radians().tan() * (y - mid) + wave * (std::f32::consts::TAU * (y - dims.min.y) / WAVE_LENGTH).sin();
    (h1 + rise, h2 + rise)
}

/// Whether a surface with normal `n` belongs to a stripe on `sides`.
pub fn stripe_side_weight(sides: Sides, n: Vec3) -> f32 {
    let ramp = |x: f32, a: f32, b: f32| ((x - a) / (b - a)).clamp(0.0, 1.0);
    match sides {
        Sides::All => 1.0,
        // the side walls: facing across more than along or up
        Sides::Sides => ramp(n.x.abs(), 0.45, 0.6),
        Sides::Front => ramp(n.y, 0.45, 0.6),
        Sides::Rear => ramp(-n.y, 0.45, 0.6),
    }
}

/// Snap a stripe's edge to the window line when it comes within 3 cm of it.
pub fn snap_to_window(h: f32, dims: &BusDims) -> f32 {
    let w = dims.window_height();
    if (h - w).abs() < 0.03 { w } else { h }
}

// --- a new layer -----------------------------------------------------------------------------

static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// A fresh layer id (`l` + the time and a counter in base 36, as Omsi-Hub makes them).
pub fn new_id(prefix: &str) -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) as u64;
    format!("{prefix}{}{}", base36(t), base36(n))
}

fn base36(mut v: u64) -> String {
    let digits = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut s = Vec::new();
    loop {
        s.push(digits[(v % 36) as usize]);
        v /= 36;
        if v == 0 {
            break;
        }
    }
    s.reverse();
    String::from_utf8(s).unwrap_or_default()
}

pub fn layer(name: &str, kind: Kind) -> Layer {
    Layer { id: new_id("l"), name: name.to_string(), visible: true, opacity: 1.0, locked: false, detail: 1.0, over_trim: false, over_glass: false, from_recipe: false, kind }
}

/// A new decal placed by a click on the bus: over the windows when the click was on a window
/// (a print on the glass), else not.
pub fn placed_layer(name: &str, kind: Kind, on_glass: bool) -> Layer {
    Layer { over_glass: on_glass, ..layer(name, kind) }
}

/// A base colour: a fill over all the paint (the protection keeps the rest).
pub fn base_colour(colour: &str) -> Kind {
    Kind::Fill { centre: [50.0, 0.0, 0.0], radius: 1000.0, group: None, colour: colour.to_string(), gradient: None }
}

/// A colour for one paint group of the bus (0 its body's colour, 1 the second).
pub fn group_colour(group: u8, colour: &str) -> Kind {
    Kind::Fill { centre: [50.0, 0.0, 0.0], radius: 1000.0, group: Some(group), colour: colour.to_string(), gradient: None }
}

/// A copy of layer `i` right above it, under a new id.
pub fn duplicate(layers: &mut Vec<Layer>, i: usize) -> Option<String> {
    let mut l = layers.get(i)?.clone();
    l.id = new_id("l");
    l.name = format!("{} (2)", l.name);
    l.locked = false;
    // (moved a little, so that the copy is seen)
    if let Some(p) = l.kind.place_mut() {
        let (u, v, _) = p.side.axes();
        p.centre += (u - v) * 0.1;
    }
    let id = l.id.clone();
    layers.insert(i + 1, l);
    Some(id)
}

/// Move layer `from` to `to` (both indices into the list as it is).
pub fn reorder(layers: &mut Vec<Layer>, from: usize, to: usize) {
    if from >= layers.len() {
        return;
    }
    let l = layers.remove(from);
    layers.insert(to.min(layers.len()), l);
}

// --- undo and redo ---------------------------------------------------------------------------

/// Steps back and forward through the document. A change is committed with the state it
/// started from (a drag commits once, on release, with the state from before it began).
pub struct History<T> {
    past: Vec<T>,
    future: Vec<T>,
}

impl<T> Default for History<T> {
    fn default() -> History<T> {
        History { past: Vec::new(), future: Vec::new() }
    }
}

pub const HISTORY_STEPS: usize = 500;

impl<T: Clone + PartialEq> History<T> {
    pub fn commit(&mut self, before: T, now: &T) {
        if &before == now {
            return;
        }
        self.past.push(before);
        if self.past.len() > HISTORY_STEPS {
            self.past.remove(0);
        }
        self.future.clear();
    }
    pub fn undo(&mut self, now: &mut T) -> bool {
        match self.past.pop() {
            Some(p) => {
                self.future.push(std::mem::replace(now, p));
                true
            }
            None => false,
        }
    }
    pub fn redo(&mut self, now: &mut T) -> bool {
        match self.future.pop() {
            Some(f) => {
                self.past.push(std::mem::replace(now, f));
                true
            }
            None => false,
        }
    }
    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }
}

// --- quick livery ----------------------------------------------------------------------------

/// The quick livery's choices (Omsi-Hub's `SnelleLakStand`): up to three colours, a stripe,
/// a name on the bus and a logo. Every choice becomes an ordinary layer with a fixed id.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Quick {
    #[serde(rename = "kleuren")]
    pub colours: [Option<String>; 3],
    #[serde(rename = "strook")]
    pub stripe: StripeTemplate,
    #[serde(rename = "strookGekozen", default)]
    pub stripe_chosen: bool,
    #[serde(rename = "naam", default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,
    #[serde(rename = "logoVerhouding", default, skip_serializing_if = "Option::is_none")]
    pub logo_aspect: Option<f32>,
    #[serde(rename = "lettertype", default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
}

impl Default for Quick {
    fn default() -> Quick {
        Quick { colours: [None, None, None], stripe: StripeTemplate::Skirt, stripe_chosen: false, name: String::new(), logo: None, logo_aspect: None, font: None }
    }
}

pub const RECIPE_BASE: &str = "r-grond";
pub const RECIPE_STRIPE: &str = "r-strook";
pub const RECIPE_SECOND: &str = "r-tweede";
pub const RECIPE_TEXT: &str = "r-tekst";
pub const RECIPE_LOGO: &str = "r-logo";

/// The box a text takes is this many letter heights high (room for descenders and outline).
pub const TEXT_BOX: f32 = 1.35;

/// Where the quick livery's name (or logo, `h` metres high) goes, above the box's bottom: between
/// the stripe's top and the window line, or on the stripe itself when there is no room above it.
/// Also whether it stands on the stripe.
pub fn name_spot(layers: &[Layer], dims: &BusDims, h: f32) -> (f32, bool) {
    let win = dims.window_height();
    let band = layers.iter().find(|l| l.id == RECIPE_STRIPE).and_then(|l| match &l.kind {
        Kind::Stripe { h1, h2, sides, .. } if !matches!(sides, Sides::Front | Sides::Rear) => Some((*h1, *h2)),
        _ => None,
    });
    let under = match band {
        Some((_, h2)) if h2 < win => h2,
        _ => win * 0.35,
    };
    let need = h + 0.1;
    if let Some((h1, h2)) = band {
        if win - under < need && h2 - h1.max(0.0) >= need {
            return ((h1.max(0.0) + h2.min(win)) * 0.5, true);
        }
    }
    ((h * 0.5 + 0.05).max(((under + win) * 0.5).min(win - h * 0.5 - 0.08)), false)
}

/// The quick livery applied to the layers (Omsi-Hub's `pasSnelleLak`): base colour and stripe at
/// the bottom, the name and the logo on top; a layer already there keeps what the player did to
/// it and takes only what the panel chose. `text_width` measures a text one metre high.
pub fn apply_quick(layers: &[Layer], q: &Quick, dims: &BusDims, text_width: &dyn Fn(&str, &str) -> f32) -> Vec<Layer> {
    let mut out: Vec<Layer> = layers.to_vec();
    let find = |out: &Vec<Layer>, id: &str| out.iter().position(|l| l.id == id);
    let set = |out: &mut Vec<Layer>, id: &str, layer: Option<Layer>, at: Option<usize>| {
        let i = out.iter().position(|l| l.id == id);
        match (layer, i) {
            (None, Some(i)) => {
                out.remove(i);
            }
            (None, None) => {}
            (Some(l), Some(i)) => out[i] = l,
            (Some(l), None) => match at {
                Some(a) => out.insert(a.min(out.len()), l),
                None => out.push(l),
            },
        }
    };
    let recipe = |id: &str, name: &str, kind: Kind| Layer { id: id.to_string(), from_recipe: true, ..layer(name, kind) };
    // the base colour: over all the paint, or (with a second colour) over the body's colour
    let group = q.colours[1].is_some().then_some(0u8);
    let base = q.colours[0].as_ref().map(|c| match find(&out, RECIPE_BASE).map(|i| out[i].clone()) {
        Some(mut l) if matches!(l.kind, Kind::Fill { .. }) => {
            l.kind.set_colour(c.clone());
            if let Kind::Fill { group: g, radius, .. } = &mut l.kind {
                if *radius >= 999.0 {
                    *g = group;
                }
            }
            l
        }
        _ => recipe(RECIPE_BASE, "Base colour", if group.is_some() { group_colour(0, c) } else { base_colour(c) }),
    });
    set(&mut out, RECIPE_BASE, base, Some(0));
    // the second colour over the livery's second paint group (a two-tone bus's roof or skirt)
    let second = q.colours[1].as_ref().map(|c| match find(&out, RECIPE_SECOND).map(|i| out[i].clone()) {
        Some(mut l) if matches!(l.kind, Kind::Fill { .. }) => {
            l.kind.set_colour(c.clone());
            l
        }
        _ => recipe(RECIPE_SECOND, "Second colour", group_colour(1, c)),
    });
    let at = find(&out, RECIPE_BASE).map(|i| i + 1).unwrap_or(0);
    set(&mut out, RECIPE_SECOND, second, Some(at));
    // the stripe, once colour 2 or a stripe is chosen
    let stripe_colour = q.colours[1].clone().unwrap_or_else(|| "#ffffff".into());
    let want = q.colours[1].is_some() || q.stripe_chosen;
    let stripe_layer = want.then(|| {
        let mut l = match find(&out, RECIPE_STRIPE).map(|i| out[i].clone()) {
            Some(l) if matches!(l.kind, Kind::Stripe { template, .. } if template == q.stripe) => l,
            _ => recipe(RECIPE_STRIPE, q.stripe.label(), stripe(q.stripe, &stripe_colour, dims)),
        };
        l.kind.set_colour(stripe_colour.clone());
        l.name = q.stripe.label().to_string();
        l
    });
    let at = find(&out, RECIPE_SECOND).or_else(|| find(&out, RECIPE_BASE)).map(|i| i + 1).unwrap_or(0);
    set(&mut out, RECIPE_STRIPE, stripe_layer, Some(at));
    // the name on the bus: above the stripe, 25 cm, readable on both sides
    let text = (!q.name.trim().is_empty()).then(|| {
        let font = q.font.clone().unwrap_or_else(|| super::shapes::DEFAULT_FONT.to_string());
        let old = find(&out, RECIPE_TEXT).map(|i| out[i].clone());
        let height_cm = match &old {
            Some(Layer { kind: Kind::Text { height_cm, .. }, .. }) => *height_cm,
            _ => 25.0,
        };
        let width = (text_width(&q.name, &font) * height_cm / 100.0).max(0.01);
        let (y, on_band) = name_spot(&out, dims, height_cm / 100.0 * TEXT_BOX);
        let colour = q.colours[2].clone().unwrap_or_else(|| if on_band { q.colours[0].clone().unwrap_or_else(|| "#1d3f8f".into()) } else { "#ffffff".into() });
        match old {
            Some(mut l) if matches!(l.kind, Kind::Text { .. }) => {
                if let Kind::Text { text, font: f, colour: c, place, .. } = &mut l.kind {
                    *text = q.name.clone();
                    *f = font;
                    *c = colour;
                    place.width_m = width;
                }
                l
            }
            _ => {
                let place = place_on(Side::R, 0.45, y, width, dims);
                recipe(RECIPE_TEXT, "Name", Kind::Text { text: q.name.clone(), font, height_cm, colour, outline: None, spacing: 0.0, gradient: None, place })
            }
        }
    });
    set(&mut out, RECIPE_TEXT, text, None);
    // the logo: at the front on both sides, 60 cm wide, the same way round on both
    let logo = q.logo.as_ref().map(|img| match find(&out, RECIPE_LOGO).map(|i| out[i].clone()) {
        Some(mut l) if matches!(l.kind, Kind::Image { .. }) => {
            if let Kind::Image { image, aspect, .. } = &mut l.kind {
                *image = img.clone();
                *aspect = q.logo_aspect.or(*aspect);
            }
            l
        }
        _ => {
            let h = 0.6 * q.logo_aspect.unwrap_or(0.5);
            let (y, _) = name_spot(&out, dims, h);
            let mut place = place_on(Side::R, 0.8, y, 0.6, dims);
            place.same_direction = true;
            recipe(RECIPE_LOGO, "Logo", Kind::Image { image: img.clone(), white_clear: true, aspect: q.logo_aspect, place })
        }
    });
    set(&mut out, RECIPE_LOGO, logo, None);
    out
}

/// A place on a side: `along` the share of the length from the rear (0) to the front (1), `up`
/// metres above the box's bottom.
pub fn place_on(side: Side, along: f32, up: f32, width_m: f32, dims: &BusDims) -> Place {
    let y = dims.min.y + dims.length() * along;
    let x = if side == Side::R { dims.max.x } else { dims.min.x };
    Place::new(side, Vec3::new(x, y, dims.min.z + up), width_m)
}

// --- the name in OMSI ------------------------------------------------------------------------

pub const NAME_MAX: usize = 48;

/// Why a name cannot be a new livery's (Omsi-Hub's `naamFout`), or None.
#[derive(Clone, Debug, PartialEq)]
pub enum NameError {
    Empty,
    TooLong,
    Character(char),
    Edge,
    Bracket,
    Reserved,
    Taken(String),
}

impl NameError {
    pub fn message(&self) -> String {
        match self {
            NameError::Empty => omsi_ui::tr("Give the livery a name.").into_owned(),
            NameError::TooLong => omsi_ui::tr("The name is too long (48 characters at most).").into_owned(),
            NameError::Character(c) => omsi_ui::tr("OMSI cannot read the character '%{c}' in a name.").replace("%{c}", &c.to_string()),
            NameError::Edge => omsi_ui::tr("The name cannot begin or end with a space.").into_owned(),
            NameError::Bracket => omsi_ui::tr("The name cannot begin with '['.").into_owned(),
            NameError::Reserved => omsi_ui::tr("That name is reserved; choose another.").into_owned(),
            NameError::Taken(n) => omsi_ui::tr("This bus already has a livery called '%{name}'.").replace("%{name}", n),
        }
    }
}

/// OMSI's own upper case (only a-z, as Omsi.exe compares names).
pub fn omsi_upper(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_lowercase() { c.to_ascii_uppercase() } else { c }).collect()
}

pub fn name_error<'a>(name: &str, existing: impl IntoIterator<Item = &'a str>) -> Option<NameError> {
    if name.is_empty() {
        return Some(NameError::Empty);
    }
    if name.chars().count() > NAME_MAX {
        return Some(NameError::TooLong);
    }
    for c in name.chars() {
        let u = c as u32;
        if u < 0x20 || u == 0x7f || cp1252_byte(c).is_none() || u == 0xa0 || u == 0xad {
            return Some(NameError::Character(c));
        }
    }
    if name.starts_with([' ', '\t']) || name.ends_with([' ', '\t']) {
        return Some(NameError::Edge);
    }
    if name.starts_with('[') {
        return Some(NameError::Bracket);
    }
    if ["standaard", "standard", "default"].iter().any(|r| name.eq_ignore_ascii_case(r)) {
        return Some(NameError::Reserved);
    }
    let key = omsi_upper(name);
    existing.into_iter().find(|e| omsi_upper(e) == key).map(|e| NameError::Taken(e.to_string()))
}

const CP1252_80: [u16; 32] = [
    0x20ac, 0, 0x201a, 0x0192, 0x201e, 0x2026, 0x2020, 0x2021, 0x02c6, 0x2030, 0x0160, 0x2039, 0x0152, 0, 0x017d, 0, 0, 0x2018, 0x2019, 0x201c, 0x201d, 0x2022, 0x2013, 0x2014, 0x02dc, 0x2122, 0x0161, 0x203a, 0x0153, 0, 0x017e, 0x0178,
];

/// The character's byte in Windows-1252, or None.
pub fn cp1252_byte(c: char) -> Option<u8> {
    let u = c as u32;
    if u < 0x80 || (0xa0..=0xff).contains(&u) {
        return Some(u as u8);
    }
    CP1252_80.iter().position(|&x| x != 0 && x as u32 == u).map(|i| 0x80 + i as u8)
}

/// Text as Windows-1252 bytes (what the name check let through always converts).
pub fn to_cp1252(s: &str) -> Vec<u8> {
    s.chars().map(|c| cp1252_byte(c).unwrap_or(b'?')).collect()
}

/// The name's slug for file names: ASCII `[a-z0-9-]`, 32 at most; `livery` without a letter.
pub fn slug(name: &str) -> String {
    let mut s = String::new();
    for c in name.chars() {
        let c = fold(c);
        for c in c.chars() {
            if c.is_ascii_alphanumeric() {
                s.push(c.to_ascii_lowercase());
            } else if !s.ends_with('-') {
                s.push('-');
            }
        }
    }
    let s: String = s.trim_matches('-').chars().take(32).collect();
    let s = s.trim_end_matches('-').to_string();
    if s.is_empty() { "livery".into() } else { s }
}

/// A letter without its accent (enough for the slug).
fn fold(c: char) -> String {
    let base = match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ą' => "a",
        'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' | 'Ā' | 'Ą' => "A",
        'ç' | 'ć' | 'č' => "c",
        'Ç' | 'Ć' | 'Č' => "C",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ę' | 'ě' => "e",
        'È' | 'É' | 'Ê' | 'Ë' | 'Ē' | 'Ę' | 'Ě' => "E",
        'ì' | 'í' | 'î' | 'ï' => "i",
        'Ì' | 'Í' | 'Î' | 'Ï' => "I",
        'ñ' | 'ń' | 'ň' => "n",
        'Ñ' | 'Ń' | 'Ň' => "N",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ő' => "o",
        'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø' | 'Ő' => "O",
        'ù' | 'ú' | 'û' | 'ü' | 'ů' | 'ű' => "u",
        'Ù' | 'Ú' | 'Û' | 'Ü' | 'Ů' | 'Ű' => "U",
        'ý' | 'ÿ' => "y",
        'š' | 'ś' => "s",
        'Š' | 'Ś' => "S",
        'ž' | 'ź' | 'ż' => "z",
        'Ž' | 'Ź' | 'Ż' => "Z",
        'ł' => "l",
        'Ł' => "L",
        'ß' => "ss",
        _ => return c.to_string(),
    };
    base.to_string()
}

/// The prefix of everything the studio writes beside a bus.
pub const PREFIX: &str = "LiveryStudio";

/// `~LiveryStudio_0007_slug.cti` (the `~` sorts it after the bus's own, so their numbers keep).
pub fn cti_name(nnnn: u32, slug: &str) -> String {
    format!("~{PREFIX}_{nnnn:04}_{slug}.cti")
}

/// `LiveryStudio\0007_slug`: the textures' folder under the `[CTC]` folder.
pub fn texture_folder(nnnn: u32, slug: &str) -> String {
    format!("{PREFIX}\\{nnnn:04}_{slug}")
}

/// A number as `StrToFloat` reads it: a point, no exponent.
pub fn cti_number(v: f32) -> String {
    if v == 0.0 || !v.is_finite() {
        return "0".into();
    }
    if v.fract() == 0.0 && v.abs() < 1e9 {
        return format!("{}", v as i64);
    }
    let s = format!("{:.6}", v);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The `.cti`'s text (Omsi-Hub's `ctiTekst`): a header of `*` lines OMSI skips, an `[item]` for
/// every slot (`(slot, path relative to the CTC folder)`), then the variables; CRLF after every
/// line. As bytes: `to_cp1252`.
pub fn cti_text(name: &str, nnnn: u32, date: &str, items: &[(String, String)], setvars: &[(String, f32)]) -> Result<String, String> {
    if let Some(e) = name_error(name, []) {
        return Err(format!("{e:?}"));
    }
    let clean = |s: &str, what: &str| -> Result<String, String> {
        if s.is_empty() || s.contains(['\r', '\n']) || s.starts_with('[') {
            return Err(format!("not a {what} for a .cti: {s:?}"));
        }
        Ok(s.to_string())
    };
    let mut lines: Vec<String> = vec![
        "*".repeat(64),
        format!(" Own livery from openOMSI's livery studio, no. {nnnn:04}"),
        format!(" Made {date}. Do not edit by hand."),
        "*".repeat(64),
        String::new(),
    ];
    for (slot, path) in items {
        let path = path.replace('/', "\\");
        if path.split('\\').any(|p| p == "..") || path.starts_with('\\') || path.get(1..2) == Some(":") {
            return Err(format!("path outside the CTC folder: {path}"));
        }
        lines.extend(["[item]".to_string(), name.to_string(), clean(slot, "slot")?, clean(&path, "path")?, String::new()]);
    }
    for (v, x) in setvars {
        lines.extend(["[setvar]".to_string(), clean(v, "variable")?, cti_number(*x), String::new()]);
    }
    Ok(lines.join("\r\n") + "\r\n")
}

/// The date in a `.cti`'s header: dd-mm-yyyy.
pub fn cti_date(days_since_epoch: i64) -> String {
    // (the civil date of a day count, Howard Hinnant's algorithm)
    let z = days_since_epoch + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{d:02}-{m:02}-{y:04}")
}

// --- the texture's size ----------------------------------------------------------------------

/// The size of the texture written (Omsi-Hub's `uitvoerMaat`): the base's, down to a multiple of
/// four, and a larger one than 4096 (beyond a tenth) brought back to 4096 on its long side.
pub fn output_size(w: u32, h: u32) -> (u32, u32) {
    let cap = 4096.0;
    let (mut b, mut hh) = (w as f32, h as f32);
    let long = b.max(hh);
    if long > cap * 1.1 {
        b = (b * cap / long).round();
        hh = (hh * cap / long).round();
    }
    let (b, hh) = (b as u32, hh as u32);
    ((b - b % 4).max(4), (hh - hh % 4).max(4))
}

/// The size the studio paints at while editing: the output's, halved while it is larger than
/// 2048 on its long side (at least 256).
pub fn edit_size(w: u32, h: u32) -> (u32, u32) {
    let (mut w, mut h) = (w.max(1), h.max(1));
    while w.max(h) > 2048 && w.min(h) >= 512 {
        w /= 2;
        h /= 2;
    }
    (w, h)
}

// --- colours ---------------------------------------------------------------------------------

/// `#rrggbb` (or `rrggbb`, `#rgb`) as bytes.
pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    let v = |a: &str| u8::from_str_radix(a, 16).ok();
    match s.len() {
        6 => Some([v(&s[0..2])?, v(&s[2..4])?, v(&s[4..6])?]),
        3 => {
            let d = |i: usize| v(&s[i..i + 1]).map(|x| x * 17);
            Some([d(0)?, d(1)?, d(2)?])
        }
        _ => None,
    }
}

pub fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dims() -> BusDims {
        BusDims { min: Vec3::new(-1.25, -6.0, 0.3), max: Vec3::new(1.25, 6.0, 3.3), window: Some(1.3) }
    }

    #[test]
    fn a_project_reads_and_writes_omsi_hub_s_json() {
        let json = r##"{"versie":1,"id":"abc","naam":"Stadtwerke","bus":"Vehicles/MAN/SD200.bus","start":"snel",
            "lagen":[{"id":"r-grond","naam":"Grondkleur","zichtbaar":true,"dekking":1,"detail":1,"uitRecept":true,"soort":"zone","centrum":[50,0,0],"straal":1000,"kleur":"#1d3f8f"},
                     {"id":"t1","naam":"Tekst","zichtbaar":true,"dekking":0.5,"detail":1,"soort":"tekst","tekst":"Lucstad","lettertype":"Hanken Grotesk","hoogteCm":25,"kleur":"#ffffff","plaats":{"zijde":"R","midden":[1.2,1.5,3.0],"breedteM":1.4,"draai":0,"spiegel":"gekoppeld"}}],
            "opties":{},"spiegel":{"aan":true},"gemaakt":"x","bewaard":"y"}"##;
        let p: Project = serde_json::from_str(json).unwrap();
        assert_eq!(p.layers.len(), 2);
        assert!(matches!(p.layers[0].kind, Kind::Fill { radius, .. } if radius == 1000.0));
        let Kind::Text { place, height_cm, .. } = &p.layers[1].kind else { panic!() };
        // o3d [x, up, forward] into the bus frame
        assert_eq!(place.centre, Vec3::new(1.2, 3.0, 1.5));
        assert_eq!(*height_cm, 25.0);
        assert_eq!(p.layers[1].opacity, 0.5);
        // and back the same way
        let again: serde_json::Value = serde_json::to_value(&p).unwrap();
        let m: [f32; 3] = serde_json::from_value(again["lagen"][1]["plaats"]["midden"].clone()).unwrap();
        assert_eq!(m, [1.2, 1.5, 3.0]);
        assert_eq!(again["lagen"][1]["soort"], "tekst");
        assert_eq!(again["lagen"][0]["soort"], "zone");
        let back: Project = serde_json::from_value(again).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn the_decal_s_corners_and_its_own_box_agree() {
        for side in [Side::L, Side::R, Side::V, Side::A, Side::D] {
            let mut p = Place::new(side, Vec3::new(0.3, 1.0, 1.5), 1.2);
            p.rotation = 30.0;
            let c = p.corners(1.2, 0.4);
            let l = p.local(c[2]);
            assert!((l - Vec2::new(0.6, 0.2)).length() < 1e-4, "{side:?}: {l}");
            let l = p.local(c[0]);
            assert!((l - Vec2::new(-0.6, -0.2)).length() < 1e-4, "{side:?}: {l}");
        }
        // seen from outside, u runs to the right: on the right side from the rear to the front
        assert_eq!(Side::R.axes().0, Vec3::Y);
        assert_eq!(Side::L.axes().0, Vec3::NEG_Y);
        assert_eq!(Side::of_normal(Vec3::new(-0.9, 0.1, 0.2)), Side::L);
        assert_eq!(Side::of_normal(Vec3::new(0.1, 0.95, 0.2)), Side::V);
        assert_eq!(Side::of_normal(Vec3::new(0.1, 0.2, 0.95)), Side::D);
    }

    #[test]
    fn a_coupled_decal_is_mirrored_to_the_other_side() {
        let mut p = Place::new(Side::R, Vec3::new(1.25, 2.0, 1.4), 0.6);
        p.rotation = 20.0;
        let (m, image) = mirror_place(&p, 0.0, true).unwrap();
        assert_eq!(m.side, Side::L);
        assert_eq!(m.centre, Vec3::new(-1.25, 2.0, 1.4));
        assert!(image);
        let (m2, image) = mirror_place(&p, 0.0, false).unwrap();
        assert!(!image);
        // either way the copy lies where the original's mirror image lies: its corners are the
        // original's across the plane (a text slanted up to the front rises to the front there too)
        for m in [&m, &m2] {
            assert_eq!(m.rotation, -20.0);
            let (a, b) = (p.corners(0.6, 0.2), m.corners(0.6, 0.2));
            for (k, c) in a.iter().enumerate() {
                let across = Vec3::new(-c.x, c.y, c.z);
                assert!(b.iter().any(|d| d.distance(across) < 1e-4), "corner {k}: {c} not mirrored in {b:?}");
            }
        }
        // the plane moved: x mirrored in it
        assert!((mirror_place(&p, 0.1, false).unwrap().0.centre.x + 1.05).abs() < 1e-5);
        let mut single = p.clone();
        single.mirror = Coupling::Single;
        assert!(mirror_place(&single, 0.0, false).is_none());
        assert!(mirror_place(&Place::new(Side::V, Vec3::ZERO, 1.0), 0.0, false).is_none());
    }

    #[test]
    fn texts_and_pictures_stay_readable_shapes_are_mirrored_unless_chosen() {
        let place = Place::new(Side::R, Vec3::new(1.25, 2.0, 1.4), 0.6);
        let text = Kind::Text { text: "Bus".into(), font: String::new(), height_cm: 25.0, colour: "#fff".into(), outline: None, spacing: 0.0, gradient: None, place: place.clone() };
        let image = Kind::Image { image: "h".into(), white_clear: false, aspect: None, place: place.clone() };
        let shape = Kind::Shape { shape: "pijl".into(), colour: "#fff".into(), outline: None, gradient: None, place: place.clone() };
        let path = Kind::Path { nodes: Vec::new(), colour: "#fff".into(), outline: None, gradient: None, place: place.clone() };
        assert!(!text.mirror_image() && !image.mirror_image(), "letters stay readable on the other side");
        assert!(shape.mirror_image() && path.mirror_image(), "an arrow points forward on both sides");
        assert!(!model_fill().mirror_image());
        // the player's choice, kept with Omsi-Hub's flag in step
        let mut s = shape.clone();
        s.place_mut().unwrap().set_mirror_image(false);
        assert!(!s.mirror_image() && s.place().unwrap().same_direction);
        let mut t = text.clone();
        t.place_mut().unwrap().set_mirror_image(true);
        assert!(t.mirror_image() && !t.place().unwrap().same_direction);
        // projects from before the choice: "the same way round" stays readable, a shape without
        // it stays a mirror image, a picture without it now reads the right way
        let old = |kind: &str, extra: &str| -> Kind {
            let json = format!(r##"{{"id":"x","naam":"x","soort":"{kind}",{extra}"plaats":{{"zijde":"R","midden":[1.2,1.5,3.0],"breedteM":1.4}}}}"##);
            serde_json::from_str::<Layer>(&json).unwrap().kind
        };
        assert!(!old("afbeelding", r#""beeld":"h","#).mirror_image());
        let mut same = old("vorm", r##""vorm":"pijl","kleur":"#fff","##);
        assert!(same.mirror_image());
        same.place_mut().unwrap().same_direction = true;
        assert!(!same.mirror_image());
        let json = serde_json::to_value(&t).unwrap();
        assert_eq!(json["plaats"]["spiegelbeeld"], true);
        let back: Kind = serde_json::from_value(json).unwrap();
        assert_eq!(back, t);
    }

    fn model_fill() -> Kind {
        base_colour("#123456")
    }

    #[test]
    fn a_decal_clicked_onto_a_window_goes_over_the_windows() {
        let place = Place::new(Side::R, Vec3::new(1.25, 2.0, 1.4), 0.6);
        let k = Kind::Shape { shape: "ster".into(), colour: "#fff".into(), outline: None, gradient: None, place };
        assert!(placed_layer("Star", k.clone(), true).over_glass);
        assert!(!placed_layer("Star", k, false).over_glass);
        assert!(!layer("Base", base_colour("#fff")).over_glass, "a base colour leaves the windows");
        // kept in the project (and absent when off)
        let mut l = layer("Band", base_colour("#fff"));
        l.over_glass = true;
        let v = serde_json::to_value(&l).unwrap();
        assert_eq!(v["ookOverRamen"], true);
        let back: Layer = serde_json::from_value(v).unwrap();
        assert!(back.over_glass);
        l.over_glass = false;
        assert!(serde_json::to_value(&l).unwrap().get("ookOverRamen").is_none());
    }

    #[test]
    fn stripes_come_from_the_box_and_the_window_line() {
        let d = dims();
        assert!((d.window_height() - 1.0).abs() < 1e-5);
        let Kind::Stripe { h1, h2, .. } = stripe(StripeTemplate::WindowBand, "#fff", &d) else { panic!() };
        assert!((h1 - 0.88).abs() < 1e-5 && (h2 - 1.02).abs() < 1e-5);
        let Kind::Stripe { h1, h2, sides, .. } = stripe(StripeTemplate::Front, "#fff", &d) else { panic!() };
        assert!(h1 < 0.0 && h2 > d.height() && sides == Sides::Front);
        let Kind::Stripe { h1, h2, .. } = stripe(StripeTemplate::Skirt, "#fff", &d) else { panic!() };
        assert!(h1 < 0.0 && (h2 - 0.35).abs() < 1e-5);
        // slanted: rising to the front; a wave: up and down along the bus
        let Kind::Stripe { h1, h2, angle, wave, .. } = stripe(StripeTemplate::Slanted, "#fff", &d) else { panic!() };
        let (a, _) = stripe_edges(h1, h2, angle, wave, -6.0, &d);
        let (b, _) = stripe_edges(h1, h2, angle, wave, 6.0, &d);
        assert!((b - a - 12.0 * 0.12).abs() < 1e-4);
        let Kind::Stripe { h1, h2, angle, wave, .. } = stripe(StripeTemplate::Wave, "#fff", &d) else { panic!() };
        let lift = |y: f32| stripe_edges(h1, h2, angle, wave, y, &d).0 - h1;
        assert!(lift(-6.0 + WAVE_LENGTH / 4.0) > 0.11 && lift(-6.0 + WAVE_LENGTH * 0.75) < -0.11);
        // the sides they paint
        assert_eq!(stripe_side_weight(Sides::Sides, Vec3::X), 1.0);
        assert_eq!(stripe_side_weight(Sides::Sides, Vec3::Y), 0.0);
        assert_eq!(stripe_side_weight(Sides::Front, Vec3::Y), 1.0);
        assert_eq!(stripe_side_weight(Sides::Rear, Vec3::Y), 0.0);
        assert!((snap_to_window(1.02, &d) - 1.0).abs() < 1e-5);
        assert_eq!(snap_to_window(1.1, &d), 1.1);
    }

    #[test]
    fn undo_and_redo_step_through_the_document() {
        let mut h: History<Vec<i32>> = History::default();
        let mut doc = vec![1];
        let before = doc.clone();
        doc.push(2);
        h.commit(before, &doc);
        let before = doc.clone();
        doc.push(3);
        h.commit(before, &doc);
        // a drag that came back where it began is no step
        h.commit(doc.clone(), &doc);
        assert!(h.undo(&mut doc));
        assert_eq!(doc, vec![1, 2]);
        assert!(h.undo(&mut doc));
        assert_eq!(doc, vec![1]);
        assert!(!h.undo(&mut doc));
        assert!(h.redo(&mut doc));
        assert_eq!(doc, vec![1, 2]);
        // something new: the future is gone
        let before = doc.clone();
        doc.push(9);
        h.commit(before, &doc);
        assert!(!h.redo(&mut doc));
        assert!(h.can_undo());
        for k in 0..HISTORY_STEPS + 20 {
            let before = doc.clone();
            doc.push(k as i32);
            h.commit(before, &doc);
        }
        let mut n = 0;
        while h.undo(&mut doc) {
            n += 1;
        }
        assert_eq!(n, HISTORY_STEPS);
    }

    #[test]
    fn layers_are_duplicated_and_reordered() {
        let mut ls = vec![layer("a", base_colour("#000000")), layer("b", Kind::Shape { shape: "ster".into(), colour: "#fff".into(), outline: None, gradient: None, place: Place::new(Side::R, Vec3::ZERO, 1.0) })];
        let id = duplicate(&mut ls, 1).unwrap();
        assert_eq!(ls.len(), 3);
        assert_eq!(ls[2].id, id);
        assert_ne!(ls[2].id, ls[1].id);
        assert_eq!(ls[2].name, "b (2)");
        reorder(&mut ls, 2, 0);
        assert_eq!(ls[0].id, id);
        reorder(&mut ls, 0, 99);
        assert_eq!(ls[2].id, id);
    }

    #[test]
    fn the_quick_livery_makes_ordinary_layers_and_keeps_what_was_done_to_them() {
        let d = dims();
        let w = |t: &str, _: &str| t.chars().count() as f32 * 0.6;
        let mut q = Quick { colours: [Some("#1d3f8f".into()), None, None], ..Quick::default() };
        let ls = apply_quick(&[], &q, &d, &w);
        assert_eq!(ls.len(), 1);
        assert_eq!(ls[0].id, RECIPE_BASE);
        q.colours[1] = Some("#ffffff".into());
        q.name = "Lucstad".into();
        q.logo = Some("abc".into());
        q.logo_aspect = Some(0.5);
        let mut ls = apply_quick(&ls, &q, &d, &w);
        let ids: Vec<&str> = ls.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, [RECIPE_BASE, RECIPE_SECOND, RECIPE_STRIPE, RECIPE_TEXT, RECIPE_LOGO]);
        // with a second colour the base takes the body's colour, the second the second group
        assert!(matches!(&ls[0].kind, Kind::Fill { group: Some(0), colour, .. } if colour == "#1d3f8f"));
        assert!(matches!(&ls[1].kind, Kind::Fill { group: Some(1), colour, .. } if colour == "#ffffff"));
        let Kind::Text { place, colour, height_cm, .. } = &ls[3].kind else { panic!() };
        assert_eq!(colour, "#ffffff");
        assert!((place.width_m - 7.0 * 0.6 * 0.25).abs() < 1e-4);
        assert!(place.centre.z > d.min.z + 0.35 && place.centre.z + height_cm / 200.0 * TEXT_BOX < d.min.z + d.window_height(), "between the band and the windows");
        // the player raised the band; another colour keeps it raised
        if let Kind::Stripe { h2, .. } = &mut ls[2].kind {
            *h2 = 0.6;
        }
        q.colours[1] = Some("#ff0000".into());
        let ls = apply_quick(&ls, &q, &d, &w);
        assert!(matches!(&ls[2].kind, Kind::Stripe { h2, colour, .. } if *h2 == 0.6 && colour == "#ff0000"));
        // another template starts the band anew; no name: no text
        q.stripe = StripeTemplate::RoofBand;
        q.name.clear();
        let ls = apply_quick(&ls, &q, &d, &w);
        assert!(matches!(&ls[2].kind, Kind::Stripe { template: StripeTemplate::RoofBand, .. }));
        assert!(ls.iter().all(|l| l.id != RECIPE_TEXT));
        let logo = ls.iter().find(|l| l.id == RECIPE_LOGO).unwrap();
        assert!(logo.kind.place().unwrap().same_direction);
    }

    #[test]
    fn names_are_checked_as_omsi_reads_them() {
        assert_eq!(name_error("", []), Some(NameError::Empty));
        assert_eq!(name_error(&"x".repeat(49), []), Some(NameError::TooLong));
        assert_eq!(name_error("Łódź", []), Some(NameError::Character('Ł')));
        assert_eq!(name_error("a\u{a0}b", []), Some(NameError::Character('\u{a0}')));
        assert_eq!(name_error(" Lucstad", []), Some(NameError::Edge));
        assert_eq!(name_error("[x", []), Some(NameError::Bracket));
        assert_eq!(name_error("Default", []), Some(NameError::Reserved));
        assert_eq!(name_error("braungold", ["Braungold"]), Some(NameError::Taken("Braungold".into())));
        // OMSI's upper case knows only a-z: ä and Ä are two names
        assert_eq!(name_error("Ärger", ["ärger"]), None);
        assert_eq!(name_error("Stadtwerke Lucstad €", ["BVG"]), None);
        assert_eq!(to_cp1252("Ä€"), vec![0xc4, 0x80]);
        assert_eq!(slug("Stadtwerke Lucstad"), "stadtwerke-lucstad");
        assert_eq!(slug("Łódź – Bus!"), "lodz-bus");
        assert_eq!(slug("!!!"), "livery");
        assert_eq!(slug(&"ab ".repeat(30)).len(), 32);
        assert_eq!(cti_name(7, "x"), "~LiveryStudio_0007_x.cti");
    }

    #[test]
    fn the_cti_is_written_as_omsi_reads_it() {
        let t = cti_text("Lucstad", 7, "04-10-2026", &[("farbschema_tex1".into(), "LiveryStudio/0007_lucstad/SD80_01_3fa9c1d2.dds".into())], &[("vis_spiegel".into(), 1.0), ("x".into(), 0.5)]).unwrap();
        assert!(t.ends_with("\r\n") && t.starts_with("****"));
        let lines: Vec<&str> = t.split("\r\n").collect();
        let i = lines.iter().position(|l| *l == "[item]").unwrap();
        assert_eq!(&lines[i..i + 4], ["[item]", "Lucstad", "farbschema_tex1", "LiveryStudio\\0007_lucstad\\SD80_01_3fa9c1d2.dds"]);
        let s = lines.iter().position(|l| *l == "[setvar]").unwrap();
        assert!(s > i);
        assert_eq!(&lines[s..s + 3], ["[setvar]", "vis_spiegel", "1"]);
        assert!(t.contains("[setvar]\r\nx\r\n0.5\r\n"));
        assert!(!t.contains('\n') || t.matches('\n').count() == t.matches("\r\n").count(), "CRLF only");
        assert!(cti_text("Lucstad", 1, "d", &[("s".into(), "..\\x.dds".into())], &[]).is_err());
        assert!(cti_text("[x", 1, "d", &[], &[]).is_err());
        assert_eq!(cti_date(0), "01-01-1970");
        assert_eq!(cti_date(20730), "04-10-2026");
    }

    #[test]
    fn the_output_is_a_multiple_of_four_and_4096_at_most() {
        assert_eq!(output_size(1024, 1024), (1024, 1024));
        assert_eq!(output_size(1023, 514), (1020, 512));
        assert_eq!(output_size(4170, 2600), (4168, 2600));
        assert_eq!(output_size(8192, 4096), (4096, 2048));
        assert_eq!(output_size(2, 2), (4, 4));
        assert_eq!(edit_size(4096, 2048), (2048, 1024));
        assert_eq!(edit_size(1024, 1024), (1024, 1024));
        assert_eq!(edit_size(4096, 4096), (2048, 2048));
    }

    #[test]
    fn colours_and_gradients() {
        assert_eq!(parse_hex("#1d3f8f"), Some([0x1d, 0x3f, 0x8f]));
        assert_eq!(parse_hex("fff"), Some([255, 255, 255]));
        assert_eq!(parse_hex("#12"), None);
        assert_eq!(hex([0x1d, 0x3f, 0x8f]), "#1d3f8f");
        let g = Gradient { kind: GradientKind::Linear, colour2: "#000".into(), angle: 0.0 };
        assert_eq!(g.t(Vec2::new(0.0, 0.3)), 0.0);
        assert_eq!(g.t(Vec2::new(1.0, 0.3)), 1.0);
        let up = Gradient { angle: 90.0, ..g.clone() };
        assert!((up.t(Vec2::new(0.2, 0.75)) - 0.75).abs() < 1e-4);
        let r = Gradient { kind: GradientKind::Radial, ..g };
        assert_eq!(r.t(Vec2::splat(0.5)), 0.0);
        assert_eq!(r.t(Vec2::new(1.0, 0.5)), 1.0);
    }

    #[test]
    fn the_handles_scale_at_the_corners_and_stretch_at_the_sides() {
        let close = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 1e-5 && (a.1 - b.1).abs() < 1e-5;
        let size = (2.0, 1.0);
        let corner = Vec2::new(1.0, 0.5);
        // a corner: both ways by the same factor, wherever the mouse goes
        assert!(close(grip_size(Grip::Corner, size, corner, Vec2::new(2.0, 1.0), false), (4.0, 2.0)));
        assert!(close(grip_size(Grip::Corner, size, corner, Vec2::new(0.5, 0.25), false), (1.0, 0.5)));
        // with Shift each way to the mouse
        assert!(close(grip_size(Grip::Corner, size, corner, Vec2::new(1.5, 0.25), true), (3.0, 0.5)));
        // a side: its own way only, the other way where the mouse is up or down makes no change
        assert!(close(grip_size(Grip::Width, size, Vec2::new(1.0, 0.0), Vec2::new(-1.5, 9.0), false), (3.0, 1.0)));
        assert!(close(grip_size(Grip::Height, size, Vec2::new(0.0, 0.5), Vec2::new(9.0, 1.0), true), (2.0, 2.0)));
        // never to nothing
        assert!(close(grip_size(Grip::Width, size, Vec2::new(1.0, 0.0), Vec2::ZERO, false), (0.02, 1.0)));
        // the handles: the corners, then the middles of the bottom, the right, the top, the left
        let p = Place::new(Side::R, Vec3::new(1.25, 0.0, 1.5), 2.0);
        let c = p.corners(2.0, 1.0);
        let h = Grip::handles(&c);
        assert!(h[..4].iter().all(|(g, _)| *g == Grip::Corner));
        assert_eq!(h[4..].iter().map(|(g, _)| *g).collect::<Vec<_>>(), vec![Grip::Height, Grip::Width, Grip::Height, Grip::Width]);
        for (g, q) in &h[4..] {
            let l = p.local(*q);
            match g {
                Grip::Width => assert!((l.x.abs() - 1.0).abs() < 1e-4 && l.y.abs() < 1e-4, "{l}"),
                _ => assert!((l.y.abs() - 0.5).abs() < 1e-4 && l.x.abs() < 1e-4, "{l}"),
            }
        }
    }

    #[test]
    fn a_picture_stretches_with_its_side_handles_and_its_copy_alike() {
        let pictures = std::collections::HashMap::new();
        let start = Place::new(Side::R, Vec3::new(1.25, 0.0, 1.5), 2.0);
        let image = |place: Place| Kind::Image { image: "h".into(), white_clear: false, aspect: Some(0.5), place };
        let size_of = |k: &Kind| super::super::paint::decal_size(k, &pictures).unwrap();
        assert_eq!(size_of(&image(start.clone())), (2.0, 1.0), "its own proportions");
        // a corner: it keeps them, its height still the picture's
        let mut p = start.clone();
        p.pulled(&start, Grip::Corner, (2.0, 1.0), Vec2::new(1.0, 0.5), Vec2::new(1.5, 0.75), false);
        assert!((p.width_m - 3.0).abs() < 1e-4 && p.height_m.is_none() && p.keep_ratio, "{p:?}");
        let (w, h) = size_of(&image(p));
        assert!((h - w * 0.5).abs() < 1e-5);
        // the right side's handle: wider, as high as it was, the size fields stop keeping the ratio
        let mut p = start.clone();
        p.pulled(&start, Grip::Width, (2.0, 1.0), Vec2::new(1.0, 0.0), Vec2::new(2.0, 0.3), false);
        assert_eq!((p.width_m, p.height_m, p.keep_ratio), (4.0, Some(1.0), false));
        assert_eq!(size_of(&image(p.clone())), (4.0, 1.0));
        // the copy on the other side the same size
        let (m, _) = mirror_place(&p, 0.0, false).unwrap();
        assert_eq!(size_of(&image(m)), (4.0, 1.0));
        // kept in the project; a project from before keeps its proportions
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["verhoudingVast"], false);
        assert_eq!(serde_json::from_value::<Place>(json).unwrap(), p);
        let old: Place = serde_json::from_str(r#"{"zijde":"R","midden":[1.2,1.5,3.0],"breedteM":1.4}"#).unwrap();
        assert!(old.keep_ratio && old.height_m.is_none());
        assert!(serde_json::to_value(&old).unwrap().get("verhoudingVast").is_none(), "nothing new written for it");
    }

    #[test]
    fn the_size_fields_keep_the_proportions_until_the_lock_is_off() {
        // a picture (half as high as wide) in its own proportions
        let mut p = Place::new(Side::R, Vec3::ZERO, 2.0);
        p.resize(Some(3.0), None, Some(0.5));
        assert_eq!((p.width_m, p.height_m), (3.0, None));
        p.resize(None, Some(0.75), Some(0.5));
        assert_eq!((p.width_m, p.height_m), (1.5, None));
        // unlocked: the height alone, then the width alone
        p.keep_ratio = false;
        p.resize(None, Some(2.0), Some(0.5));
        assert_eq!((p.width_m, p.height_m), (1.5, Some(2.0)));
        p.resize(Some(3.0), None, Some(0.5));
        assert_eq!((p.width_m, p.height_m), (3.0, Some(2.0)));
        // locked again: the stretched proportions kept
        p.keep_ratio = true;
        p.resize(Some(1.5), None, Some(0.5));
        assert_eq!((p.width_m, p.height_m), (1.5, Some(1.0)));
        // a square shape stays square while locked
        let mut q = Place::new(Side::L, Vec3::ZERO, 1.0);
        q.resize(None, Some(0.4), None);
        assert_eq!((q.width_m, q.height_m), (0.4, None));
    }
}
