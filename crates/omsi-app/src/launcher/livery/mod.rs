//! The livery studio: the player paints a bus in 3D - colours, stripes, texts, pictures, shapes
//! of their own - and saves it as a livery the game offers (Omsi-Hub's Lakstudio, ported).
//!
//! The bus stands in a showroom of its own (`showroom`), drawn with the studio's camera. Its
//! paint textures are baked: every texel knows where it lies on the bus (`bake`), so that the
//! layers - placed on the bus, never on the flat texture - are painted into the texture per
//! texel (`paint`) on a worker, and the result replaces the texture in the scene's slot. Saving
//! writes the textures at their full size and a `.cti` into the content folder (`export`). The
//! project (`model`) is kept as JSON under `~/.openomsi/liveries/<id>/`, with the pictures the
//! player brought in, and saved by itself as it changes.
//!
//! Opened from the bus company (`open_for_company`: a bus of its fleet, or the dealer's) the
//! livery is the company's design: saving it costs the design (and, for a bus of the fleet, its
//! painting in the workshop), refused when the company's cash is short (`company::livery`);
//! the studio goes back to the company when it is left.

pub mod bake;
pub mod colour;
pub mod export;
#[cfg(test)]
mod headless;
pub mod model;
pub mod paint;
pub mod shapes;
mod studio;

use super::showroom::{Look, Showroom};
use super::{Launcher, Page};
use bake::{BusGeom, MeshIn, Outside, Role, SlotUse, Zones};
use glam::{Mat4, Vec2};
use model::{Doc, History, Project};
use omsi_render::{Renderer, TextureId};
use shapes::Raster;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::Instant;

/// Open the livery studio on `bus` (a bus file as the bus step names it) in `paint` (its
/// livery to begin from; None: the model's own). Without a bus the studio asks for one.
pub fn open(l: &mut Launcher, bus: Option<String>, paint: Option<String>) {
    l.livery.company = None;
    l.go(Page::Livery);
    match bus {
        Some(b) if !b.is_empty() => start(l, b, paint.filter(|p| !p.is_empty()), None),
        _ => l.livery.session = None,
    }
}

/// The bus company the studio paints for: its id, the bus of its fleet to paint once the
/// design is saved (None: a design for the dealer's bus).
#[derive(Clone, Debug, PartialEq)]
pub struct ForCompany {
    pub id: String,
    pub vehicle: Option<u32>,
}

/// Open the studio on `bus` for the bus company: the livery saved is its design, paid for.
pub fn open_for_company(l: &mut Launcher, bus: &str, paint: &str, company: &str, vehicle: Option<u32>) {
    open(l, Some(bus.to_string()), Some(paint.to_string()).filter(|p| !p.trim().is_empty()));
    l.livery.company = Some(ForCompany { id: company.to_string(), vehicle });
}

/// What saving the livery costs the company now (the design, and the bus's painting), and the
/// company's name and cash; None when the studio does not paint for one.
pub fn company_cost(l: &Launcher) -> Option<(String, omsi_launcher_lib::company::Cents, omsi_launcher_lib::company::Cents)> {
    use omsi_launcher_lib::company::livery;
    let fc = l.livery.company.as_ref()?;
    let c = l.company.company.as_ref().filter(|c| c.id == fc.id)?;
    let name = l.livery.session.as_ref().map(|s| s.ui.name.clone()).unwrap_or_default();
    let paint = fc.vehicle.and_then(|id| c.vehicle(id)).map(|v| livery::paint_cost(c, v)).unwrap_or(0);
    Some((c.name.clone(), livery::save_cost(c, &name) + paint, c.cash))
}

/// The studio's state in the launcher.
pub struct LiveryView {
    /// The bus being painted, in a showroom of its own.
    pub showroom: Showroom,
    view_tex: Option<usize>,
    view_gen: u64,
    /// Where the bus's picture lies this frame (physical size asked of the showroom).
    view_rect: Option<omsi_ui::Rect>,
    /// The other side's picture while mirroring: where it lies, its interface texture and its
    /// generation.
    pub other_rect: Option<omsi_ui::Rect>,
    other_tex: Option<usize>,
    other_gen: u64,
    /// O held (before and after), the middle button held (panning).
    pub hold_o: bool,
    pub middle: bool,
    session: Option<Session>,
    chooser: studio::Chooser,
    /// The built-in shapes' pictures in the interface pipeline.
    shape_tex: HashMap<&'static str, usize>,
    dropped: Vec<PathBuf>,
    /// Painting for the bus company (see `open_for_company`).
    pub company: Option<ForCompany>,
}

impl Default for LiveryView {
    fn default() -> LiveryView {
        LiveryView { showroom: Showroom::new(), view_tex: None, view_gen: 0, view_rect: None, other_rect: None, other_tex: None, other_gen: 0, hold_o: false, middle: false, session: None, chooser: Default::default(), shape_tex: HashMap::new(), dropped: Vec::new(), company: None }
    }
}

impl LiveryView {
    /// The graphics device went: everything made on it goes too (the bus is placed again).
    pub fn drop_gpu(&mut self) {
        self.showroom = Showroom::new();
        self.view_tex = None;
        self.view_gen = 0;
        self.other_tex = None;
        self.other_gen = 0;
        self.shape_tex.clear();
        if let Some(s) = self.session.as_mut() {
            s.ready = None;
            s.preparing = None;
            s.placed_look = None;
        }
    }
}

/// A painted texture of the bus.
#[derive(Clone)]
pub struct Target {
    pub index: u8,
    /// The model's file name (as `[CTCTexture]` names it) and the slots that take it, per part
    /// of the bus (0 the front, then its rear sections), its `_#low` copy's with them.
    pub default: String,
    pub slots: Vec<(usize, String)>,
    /// The texture slots in the showroom's scene that show it.
    pub ids: Vec<TextureId>,
    /// The base's file (the livery begun from, or the model's own).
    pub base: PathBuf,
    /// For a plain start in a livery's colours: that livery's file, whose colours the paint
    /// groups take.
    pub tint: Option<PathBuf>,
    /// The bus maker's template for it (MA, MU), if the game's Repaint-Tool has one.
    pub template: Option<[PathBuf; 2]>,
}

/// The maker's template of a texture (`file`, as the model names it): `<stem>.rpc` in a folder
/// of the game's `SDK\RepaintTool`, whose first five lines name its BS, AL, MA, AD and MU
/// pictures (the advertising and window variants, `_NL`, `_full`, are other files).
pub fn find_template(root: &Path, file: &str) -> Option<[PathBuf; 2]> {
    let stem = Path::new(file.trim()).file_stem()?.to_string_lossy().to_lowercase();
    let tool = omsi_cfg::resolve_path(root, "SDK/RepaintTool");
    for dir in std::fs::read_dir(&tool).ok()?.flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
        let Some(rpc) = std::fs::read_dir(&dir).ok()?.flatten().map(|e| e.path()).find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase() == format!("{stem}.rpc"))) else {
            continue;
        };
        let text = omsi_cfg::codepage::decode(&std::fs::read(&rpc).ok()?);
        let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).take(5).collect();
        if lines.len() < 5 {
            continue;
        }
        let path = |l: &str| Some(omsi_cfg::resolve_path(&dir, l)).filter(|p| p.is_file());
        if let (Some(ma), Some(mu)) = (path(lines[2]), path(lines[4])) {
            return Some([ma, mu]);
        }
    }
    None
}

/// A template's pictures at `w` x `h` (RGBA each).
pub fn template_pictures(t: &[PathBuf; 2], w: u32, h: u32) -> Option<[Vec<u8>; 2]> {
    let one = |p: &PathBuf| -> Option<Vec<u8>> {
        let img = omsi_texture::decode_file(p).ok()?;
        Some(if (img.width, img.height) != (w, h) { omsi_texture::bc::resize(&img.rgba, img.width, img.height, w, h) } else { img.rgba })
    };
    Some([one(&t[0])?, one(&t[1])?])
}

/// The bus ready to paint: its shape, the canvases on the painter, the textures replaced.
pub struct Ready {
    pub geom: Arc<BusGeom>,
    pub outside: Arc<Outside>,
    /// The colour zones of each texture (as `targets`).
    pub zones: Arc<Vec<Zones>>,
    pub targets: Vec<Target>,
    /// The bases at the editing size (before and after), and the last pictures painted.
    pub bases: Vec<omsi_texture::Image>,
    pub current: Vec<Vec<u8>>,
    /// Texels a metre (the first texture's) and the full size's.
    pub density: f32,
    pub full: Vec<(u32, u32)>,
    worker: paint::Worker,
    sent: u64,
    shown: u64,
    /// The layers last sent (a change sends them again).
    last: Option<(Vec<model::Layer>, model::Mirror)>,
    showing_base: bool,
    /// Whether the windows can carry the livery (their texture is one the livery changes); None:
    /// a bus without windows.
    pub windows: Option<bool>,
    /// The canvases the painter paints (for what their texels are: shared by both sides).
    pub canvases: Arc<Vec<paint::Canvas>>,
    /// The paint at the full size (`sharpen`): for which painting (`shown`), per texture (None:
    /// it is painted at its full size already); the painting on its way, the newest painting
    /// asked for (one for an older stops) and when the last came back.
    sharp: Option<(u64, Vec<Option<omsi_texture::Image>>)>,
    sharpening: Option<Receiver<(u64, Result<Vec<Option<omsi_texture::Image>>, String>)>>,
    wanted: Arc<std::sync::atomic::AtomicU64>,
    shown_at: Option<Instant>,
}

struct Prepared {
    geom: Arc<BusGeom>,
    outside: Arc<Outside>,
    zones: Arc<Vec<Zones>>,
    canvases: Vec<paint::Canvas>,
    bases: Vec<omsi_texture::Image>,
    full: Vec<(u32, u32)>,
    windows: Option<bool>,
    /// A plain start's colours: per paint group the livery's (its median there).
    start_colours: Vec<(u8, [u8; 3])>,
}

/// A bus of the livery's family: another bus file of its folder whose model takes the same
/// paint texture - the livery fits it as it is. Its `[CTC]` folder (the same as the bus's: the
/// livery comes on it by itself) and its slots per texture (file key, slot name).
#[derive(Clone, Debug)]
pub struct Kin {
    pub bus: String,
    pub name: String,
    pub ctc: Option<PathBuf>,
    pub slots: Vec<(String, String)>,
}

/// The family of `bus` among `buses` (file, name) of the same folder, for the textures `keys`.
fn family(root: &Path, bus: &str, buses: &[(String, String)], keys: &[String]) -> Vec<Kin> {
    let folder = |f: &str| f.replace('\\', "/").rsplit_once('/').map(|x| x.0.to_ascii_lowercase()).unwrap_or_default();
    let mine = folder(bus);
    let mut out = Vec::new();
    for (file, name) in buses.iter().filter(|(f, _)| !f.eq_ignore_ascii_case(bus) && folder(f) == mine) {
        let Ok(path) = crate::spawn::player_bus_path(root, file) else { continue };
        let Ok(def) = omsi_vehicle::Vehicle::load(&path) else { continue };
        let Some(model_rel) = def.model.clone() else { continue };
        let Ok(model) = omsi_model::Model::load(&omsi_cfg::resolve_path(def.dir(), &model_rel)) else { continue };
        let slots: Vec<(String, String)> = model.ctc_textures.iter().map(|(slot, f)| (file_key(f), slot.clone())).filter(|(k, _)| keys.contains(k)).collect();
        if slots.is_empty() {
            continue;
        }
        let ctc = model.ctc.first().map(|c| omsi_cfg::resolve_path(def.dir(), &c.path));
        out.push(Kin { bus: file.clone(), name: name.clone(), ctc, slots });
    }
    out
}

/// One livery being made.
pub struct Session {
    pub project: Project,
    pub dir: PathBuf,
    pub look: Look,
    pub history: History<Doc>,
    /// The document before the change going on (committed when it ends).
    pending: Option<Doc>,
    pub selected: Option<String>,
    pub tool: studio::Tool,
    pub picker: studio::Picker,
    pub recent: Vec<[u8; 3]>,
    pub cam: studio::Cam,
    pub status: Option<(String, bool, Instant)>,
    autosave: Option<Instant>,
    /// A change not in the temporary file yet, and when that was last written
    /// ([`TEMP_FILE`]).
    temp_due: bool,
    temp_at: Option<Instant>,
    pub pictures: Arc<HashMap<String, Arc<Raster>>>,
    pub ready: Option<Ready>,
    preparing: Option<(Receiver<Result<(Prepared, Vec<Target>), String>>, Vec<Target>)>,
    /// The look the textures were taken from (placed again: taken again), and the bus options
    /// the bus wore then (changed: its shape is read again, a mesh may come or go).
    placed_look: Option<Look>,
    placed_options: Vec<(String, f32)>,
    pub failed: Option<String>,
    pub ui: studio::State,
    export: Option<Receiver<export::Msg>>,
    pub progress: Option<(String, f32)>,
    /// Save asked while a game runs: done once it is closed (the game reads the liveries as it
    /// loads a bus; a texture half written then would be read half).
    pub queued: bool,
    /// The buses of its family (read on a worker once the bus is there).
    pub family: Vec<Kin>,
    family_rx: Option<Receiver<Vec<Kin>>>,
    /// The names this bus's liveries already have.
    pub taken: Vec<String>,
    /// Per part of the bus (the front, then its rear sections): its `[CTC]` folder and the start
    /// livery's variables (written into its `.cti`).
    parts: Vec<(Option<PathBuf>, Vec<(String, f32)>)>,
}

impl Session {
    pub fn say(&mut self, text: impl Into<String>, err: bool) {
        self.status = Some((text.into(), err, Instant::now()));
    }

    /// The document changed this frame by the interface (`before` it was so): one undo step once
    /// the change is over.
    pub fn changed_from(&mut self, before: Doc) {
        if self.pending.is_none() {
            self.pending = Some(before);
        }
        self.touched();
    }

    /// The project changed: into the temporary file at once, into `project.json` a little
    /// later.
    fn touched(&mut self) {
        self.autosave = Some(Instant::now());
        self.temp_due = true;
    }

    /// The project as it is now into its temporary file (on a worker: a long stroke does not
    /// wait for the disk).
    fn keep_temp(&mut self) {
        self.temp_due = false;
        self.temp_at = Some(Instant::now());
        match serde_json::to_vec(&self.project) {
            Ok(bytes) => {
                let _ = temp_writer().send(TempMsg::Write(self.dir.clone(), bytes));
            }
            Err(e) => log::warn!("livery studio: the temporary file: {e}"),
        }
    }

    /// Close the change going on (the mouse let go, the text field left).
    pub fn settle(&mut self) {
        if let Some(b) = self.pending.take() {
            let now = self.project.doc();
            self.history.commit(b, &now);
        }
    }

    pub fn undo(&mut self) {
        self.settle();
        let mut d = self.project.doc();
        if self.history.undo(&mut d) {
            self.project.set_doc(d);
            self.touched();
        }
    }

    pub fn redo(&mut self) {
        self.settle();
        let mut d = self.project.doc();
        if self.history.redo(&mut d) {
            self.project.set_doc(d);
            self.touched();
        }
    }

    pub fn dims(&self) -> Option<model::BusDims> {
        self.ready.as_ref().map(|r| r.geom.dims)
    }

    /// The bus's own `[CTC]` folder (its front part's).
    pub fn parts_ctc(&self) -> Option<PathBuf> {
        self.parts.first().and_then(|p| p.0.clone())
    }

    /// The camera on the other side while mirroring: the studio's, mirrored in the plane.
    pub fn other_camera(&self) -> Option<omsi_render::Camera> {
        let plane = self.mirror_plane()?;
        self.ready.as_ref()?;
        let mut c = self.cam.camera();
        c.position.x = 2.0 * plane as f64 - c.position.x;
        c.yaw = -c.yaw;
        Some(c)
    }

    pub fn mirror_plane(&self) -> Option<f32> {
        let d = self.dims()?;
        self.project.mirror.on.then(|| self.project.mirror.plane_x.unwrap_or(d.middle_x()))
    }

    fn save_project(&mut self) {
        self.project.saved = now_text();
        match std::fs::create_dir_all(&self.dir).and_then(|_| std::fs::write(self.dir.join("project.json"), serde_json::to_vec_pretty(&self.project).unwrap_or_default())) {
            // (the temporary file has nothing newer then: it goes, after any write still on
            // its way)
            Ok(()) if !self.temp_due => {
                let _ = temp_writer().send(TempMsg::Remove(self.dir.clone()));
            }
            Ok(()) => {}
            Err(e) => log::warn!("livery studio: {}: {e}", self.dir.display()),
        }
    }

    /// Bring a picture in: its bytes kept with the project, its hash its name.
    pub fn import_picture(&mut self, path: &Path) -> Result<(String, Arc<Raster>), String> {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        if bytes.len() > 20 << 20 {
            return Err(omsi_ui::tr("The picture is larger than 20 MB.").into_owned());
        }
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let pic = shapes::decode_image(&bytes, &name).map_err(|e| omsi_ui::tr("This picture cannot be read: %{e}").replace("%{e}", &e))?;
        use sha2::Digest;
        let h = sha2::Sha256::digest(&bytes);
        let hash: String = h.iter().take(20).map(|b| format!("{b:02x}")).collect();
        let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_else(|| "png".into());
        let dir = self.dir.join("beelden");
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join(format!("{hash}.{ext}")), &bytes);
        let pic = Arc::new(pic);
        let mut map = (*self.pictures).clone();
        map.insert(hash.clone(), pic.clone());
        self.pictures = Arc::new(map);
        Ok((hash, pic))
    }
}

/// Where the projects are kept.
pub fn projects_dir() -> PathBuf {
    omsi_launcher_lib::data_dir().join("liveries")
}

/// Beside a project's `project.json`: the project as it was at its last change, written at once
/// with every change (a stroke as it goes, at most five times a second) and gone again once
/// `project.json` has caught up. Found newer than `project.json` - the studio ended without
/// saving, a crash, the power - it is the project.
pub const TEMP_FILE: &str = "project.autosave.json";

enum TempMsg {
    /// The project's folder and the project.
    Write(PathBuf, Vec<u8>),
    Remove(PathBuf),
}

/// The worker that writes the temporary files, in the order asked (of a run of writes to one
/// folder only the last).
fn temp_writer() -> &'static std::sync::mpsc::Sender<TempMsg> {
    static W: std::sync::OnceLock<std::sync::mpsc::Sender<TempMsg>> = std::sync::OnceLock::new();
    W.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<TempMsg>();
        let _ = std::thread::Builder::new().name("livery autosave".into()).spawn(move || {
            while let Ok(first) = rx.recv() {
                let mut run = vec![first];
                run.extend(rx.try_iter());
                for (k, m) in run.iter().enumerate() {
                    match m {
                        TempMsg::Write(dir, bytes) => {
                            if run[k + 1..].iter().any(|n| matches!(n, TempMsg::Write(d, _) | TempMsg::Remove(d) if d == dir)) {
                                continue;
                            }
                            let part = dir.join(format!("{TEMP_FILE}.part"));
                            if let Err(e) = std::fs::create_dir_all(dir).and_then(|_| std::fs::write(&part, bytes)).and_then(|_| std::fs::rename(&part, dir.join(TEMP_FILE))) {
                                log::warn!("livery studio: {}: {e}", dir.join(TEMP_FILE).display());
                            }
                        }
                        TempMsg::Remove(dir) => {
                            let _ = std::fs::remove_file(dir.join(TEMP_FILE));
                        }
                    }
                }
            }
        });
        tx
    })
}

/// The project in `dir`: its temporary file's when that is newer than `project.json` (or
/// there is no `project.json` yet), and whether it was.
fn load_project(dir: &Path) -> Option<(Project, bool)> {
    let saved = dir.join("project.json");
    let temp = dir.join(TEMP_FILE);
    let when = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let read = |p: &Path| std::fs::read(p).ok().and_then(|b| serde_json::from_slice::<Project>(&b).ok());
    if let Some(t) = when(&temp) {
        if when(&saved).is_none_or(|s| t > s) {
            if let Some(p) = read(&temp) {
                return Some((p, true));
            }
        }
    }
    read(&saved).map(|p| (p, false))
}

/// The projects there are, newest first.
pub fn projects() -> Vec<Project> {
    let mut v: Vec<Project> = std::fs::read_dir(projects_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| load_project(&e.path()).map(|p| p.0))
        .collect();
    v.sort_by(|a, b| b.saved.cmp(&a.saved));
    v
}

fn now_text() -> String {
    let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("{s}")
}

fn today() -> String {
    let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    model::cti_date(s.div_euclid(86400))
}

/// Begin painting `bus` (a new project, or `project` again).
/// Begin a new livery on `bus` from `paint` in one of the ways of `model::STARTS`.
pub fn start_new(l: &mut Launcher, bus: String, paint: Option<String>, how: &str) {
    let mut p = Project::new(model::new_id("p"), bus.clone(), paint.clone(), now_text());
    p.start = how.to_string();
    start(l, bus, paint, Some(p));
}

pub fn start(l: &mut Launcher, bus: String, paint: Option<String>, project: Option<Project>) {
    let project = project.unwrap_or_else(|| Project::new(model::new_id("p"), bus.clone(), paint.clone(), now_text()));
    let dir = projects_dir().join(&project.id);
    // (changes the studio did not get to save: from the temporary file)
    let (project, restored) = match load_project(&dir) {
        Some((p, true)) if p.id == project.id => (p, true),
        _ => (project, false),
    };
    // the pictures it has
    let mut pictures = HashMap::new();
    for e in std::fs::read_dir(dir.join("beelden")).into_iter().flatten().flatten() {
        let p = e.path();
        let (Some(stem), Ok(bytes)) = (p.file_stem().map(|s| s.to_string_lossy().into_owned()), std::fs::read(&p)) else { continue };
        if let Ok(r) = shapes::decode_image(&bytes, &p.to_string_lossy()) {
            pictures.insert(stem, Arc::new(r));
        }
    }
    let map = if !l.state.choice.map.is_empty() { l.state.choice.map.clone() } else { l.state.maps.first().map(|m| m.file.clone()).unwrap_or_default() };
    let look = Look { root: PathBuf::from(&l.state.config.root), map, bus: project.bus.clone(), paint: project.start_paint.clone().unwrap_or_default(), weather: String::new(), time: 13 * 60, date: String::new() };
    let name = project.name.clone();
    log::info!("livery studio: {} on {}{}", project.id, project.bus, project.start_paint.as_deref().map(|p| format!(" from '{p}'")).unwrap_or_default());
    l.livery.session = Some(Session {
        project,
        dir,
        look,
        history: History::default(),
        pending: None,
        selected: None,
        tool: studio::Tool::Select,
        picker: studio::Picker::default(),
        recent: Vec::new(),
        cam: studio::Cam::default(),
        status: None,
        autosave: None,
        temp_due: false,
        temp_at: None,
        pictures: Arc::new(pictures),
        ready: None,
        preparing: None,
        placed_look: None,
        placed_options: Vec::new(),
        queued: false,
        family: Vec::new(),
        family_rx: None,
        failed: None,
        ui: studio::State::named(name),
        export: None,
        progress: None,
        taken: Vec::new(),
        parts: Vec::new(),
    });
    if restored {
        if let Some(s) = l.livery.session.as_mut() {
            log::info!("livery studio: {} brought back from {TEMP_FILE}", s.project.id);
            s.say(omsi_ui::tr("Your last changes were brought back from the temporary file"), false);
            // (`project.json` catches up)
            s.autosave = Some(Instant::now());
        }
    }
}

/// A file dropped onto the window while the studio is open: a picture to place.
pub fn dropped(l: &mut Launcher, path: PathBuf) {
    l.livery.dropped.push(path);
}

/// A key went down or up (the studio holds O for before and after).
pub fn key(l: &mut Launcher, code: winit::keyboard::KeyCode, down: bool) {
    if code == winit::keyboard::KeyCode::KeyO && l.page == Page::Livery {
        l.livery.hold_o = down && l.ui.focus.is_none();
    }
}

/// Per frame before the interface: the bus loaded and prepared, the paint sent and taken back.
pub fn update(l: &mut Launcher, dt: f32) {
    if l.page != Page::Livery {
        return;
    }
    let Some(renderer) = l.renderer.as_ref() else { return };
    let v = &mut l.livery;
    let Some(s) = v.session.as_mut() else { return };
    // (the maps arrive after the launcher opened)
    if s.look.map.is_empty() {
        s.look.map = if !l.state.choice.map.is_empty() { l.state.choice.map.clone() } else { l.state.maps.first().map(|m| m.file.clone()).unwrap_or_default() };
    }
    // the bus wears the livery's bus options
    let options: Vec<(String, f32)> = s.project.options.iter().map(|(k, v)| (k.clone(), *v)).collect();
    v.showroom.dress(options.clone());
    if !s.look.map.is_empty() && s.failed.is_none() {
        v.showroom.want(s.look.clone());
    }
    v.showroom.update(renderer, dt);
    if s.failed.is_none() && v.showroom.gave_up(&s.look) && !s.look.map.is_empty() && v.showroom.error.is_some() {
        s.failed = v.showroom.error.clone();
    }
    // the bus is there: its shape, its textures, onto the worker
    let dressed = v.showroom.dressed().is_some_and(|d| d == options.as_slice());
    if v.showroom.shows(&s.look) && dressed && (s.placed_look.as_ref() != Some(&s.look) || s.placed_options != options) && s.preparing.is_none() {
        s.placed_look = Some(s.look.clone());
        s.placed_options = options;
        s.ready = None;
        let buses: Vec<(String, String)> = l.state.vehicles.iter().map(|v| (v.file.clone(), v.name.clone())).collect();
        match prepare(&mut v.showroom, s, &l.state.config.root, &buses) {
            Ok(()) => {}
            Err(e) => s.failed = Some(e),
        }
    }
    if let Some((rx, _)) = s.preparing.as_ref() {
        match rx.try_recv() {
            Ok(Ok((p, targets))) => {
                s.preparing = None;
                if let Some(parts) = v.showroom.parts() {
                    // the slots get pictures of their own, which the painter fills
                    for (t, base) in targets.iter().zip(&p.bases) {
                        for &id in &t.ids {
                            renderer.replace_texture(parts.scene, id, &omsi_texture::TextureData::from_image(base.clone()));
                        }
                        renderer.rebind_textures(parts.scene, &t.ids);
                    }
                }
                // a plain start in a livery's colours: a layer for each paint group, once
                if s.project.layers.is_empty() && s.project.start == "effenKleuren" && !p.start_colours.is_empty() {
                    for (g, c) in &p.start_colours {
                        let name = if *g == 0 { "Body colour".to_string() } else { format!("{} {}", omsi_ui::tr("Colour"), g + 1) };
                        s.project.layers.push(model::layer(&name, model::group_colour(*g, &model::hex(*c))));
                    }
                    s.touched();
                }
                let density = p.canvases.first().map(|c| c.bake.density).unwrap_or(100.0);
                let dims = p.geom.dims;
                let current = p.bases.iter().map(|b| b.rgba.clone()).collect();
                let canvases = Arc::new(p.canvases);
                let worker = paint::Worker::start(canvases.clone(), dims);
                if s.cam.dist == 0.0 {
                    let aspect = s.ui.view().map(|r| r.w / r.h.max(1.0)).unwrap_or(1.7);
                    s.cam = studio::Cam::fitting(&dims, 6, aspect);
                }
                s.ready = Some(Ready { geom: p.geom, outside: p.outside, zones: p.zones, targets, bases: p.bases, current, density, full: p.full, worker, sent: 0, shown: 0, last: None, showing_base: false, windows: p.windows, canvases, sharp: None, sharpening: None, wanted: Arc::new(std::sync::atomic::AtomicU64::new(0)), shown_at: None });
                v.showroom.touch();
            }
            Ok(Err(e)) => {
                s.preparing = None;
                s.failed = Some(e);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(_) => s.preparing = None,
        }
    }
    // the paint: sent when the layers changed, taken back when painted
    let moving = l.ui.input.down && s.pending.is_some();
    let pressed = l.ui.input.down;
    let selected = s.selected.clone();
    let mirror = s.mirror_plane();
    if let Some(r) = s.ready.as_mut() {
        let now = (s.project.layers.clone(), s.project.mirror.clone());
        if r.last.as_ref() != Some(&now) {
            let hint = selected.as_ref().filter(|_| moving).and_then(|id| s.project.index_of(id)).map(|k| {
                let key = super::ui::id_of(&format!("{}|{:?}", serde_json::to_string(&s.project.layers[..k]).unwrap_or_default(), s.project.mirror));
                (k, key)
            });
            r.sent += 1;
            let cx = paint::Context { pictures: s.pictures.clone(), mirror, density: r.density, max_px: 2048 };
            r.worker.send(paint::Job { seq: r.sent, layers: now.0.clone(), cx, moving: hint });
            r.wanted.store(r.sent, std::sync::atomic::Ordering::Relaxed);
            r.last = Some(now);
        }
        let mut done = None;
        while let Ok(d) = r.worker.rx.try_recv() {
            done = Some(d);
        }
        let base = v.hold_o || s.ui.before_after;
        if let Some(d) = done {
            r.shown = d.seq;
            r.current = d.pictures;
            r.shown_at = Some(Instant::now());
            r.sharp = None;
            if !base {
                upload(&mut v.showroom, renderer, &r.targets, &r.bases, Some(&r.current));
            }
        }
        if base != r.showing_base {
            r.showing_base = base;
            upload(&mut v.showroom, renderer, &r.targets, &r.bases, (!base).then_some(&r.current));
            if let Some((_, pics)) = r.sharp.as_ref().filter(|x| !base && x.0 == r.shown) {
                upload_full(&mut v.showroom, renderer, &r.targets, pics);
            }
        }
        sharpen(r, &s.pictures, mirror, !pressed && !moving);
        if let Some(rx) = r.sharpening.as_ref() {
            match rx.try_recv() {
                Ok((seq, res)) => {
                    r.sharpening = None;
                    match res {
                        Ok(pics) if seq == r.shown => {
                            if !r.showing_base {
                                upload_full(&mut v.showroom, renderer, &r.targets, &pics);
                            }
                            r.sharp = Some((seq, pics));
                        }
                        Ok(_) => {}
                        Err(e) => {
                            log::warn!("livery studio: the paint at its full size: {e}");
                            r.sharp = Some((seq, Vec::new()));
                        }
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => r.sharpening = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
    }
    if let Some(f) = s.family_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
        s.family_rx = None;
        log::info!("livery studio: its family: {}", f.iter().map(|k| k.name.as_str()).collect::<Vec<_>>().join(", "));
        s.family = f;
    }
    // a picture dropped onto the window
    for p in std::mem::take(&mut v.dropped) {
        studio::picture_chosen(s, &p);
    }
    // the export's progress
    if let Some(rx) = s.export.as_ref() {
        let mut end = None;
        while let Ok(m) = rx.try_recv() {
            match m {
                export::Msg::Progress(t, f) => s.progress = Some((t, f)),
                export::Msg::Done(files) => end = Some(Ok(files)),
                export::Msg::Failed(e) => end = Some(Err(e)),
            }
        }
        if let Some(end) = end {
            s.export = None;
            s.progress = None;
            match end {
                Ok(files) => saved(l, files),
                Err(e) => {
                    if let Some(s) = l.livery.session.as_mut() {
                        s.say(omsi_ui::tr("The livery was not saved: %{e}").replace("%{e}", &e), true);
                    }
                }
            }
        }
    }
    // a save asked while the game ran, now that it is closed
    if l.livery.session.as_ref().is_some_and(|s| s.queued && s.export.is_none()) && !l.state.in_game() {
        save(l);
    }
    // every change into the temporary file at once (while the mouse paints, five times a
    // second), and `project.json` two seconds after the last
    if let Some(s) = l.livery.session.as_mut() {
        if s.temp_due && s.temp_at.is_none_or(|t| t.elapsed().as_secs_f32() >= 0.2) {
            s.keep_temp();
        }
        if s.autosave.is_some_and(|t| t.elapsed().as_secs_f32() > 2.0) && !l.ui.input.down {
            s.autosave = None;
            s.save_project();
        }
    }
}

fn upload(showroom: &mut Showroom, renderer: &Renderer, targets: &[Target], bases: &[omsi_texture::Image], pictures: Option<&Vec<Vec<u8>>>) {
    let Some(parts) = showroom.parts() else { return };
    for (k, t) in targets.iter().enumerate() {
        let Some(base) = bases.get(k) else { continue };
        let img = match pictures.and_then(|p| p.get(k)) {
            Some(px) if px.len() == base.rgba.len() => omsi_texture::Image { width: base.width, height: base.height, rgba: px.clone(), has_alpha: base.has_alpha },
            _ => base.clone(),
        };
        let mut again = false;
        for &id in &t.ids {
            again |= renderer.update_texture_mips(parts.scene, id, &img);
        }
        if again {
            renderer.rebind_textures(parts.scene, &t.ids);
        }
    }
    showroom.touch();
}

/// The paint at rest at its full size: the studio paints the textures smaller while the paint
/// changes (`model::edit_size`, `EDIT_TEXELS`), and small details came out in blocks; once
/// the last painting is shown and nothing has changed for a moment, those textures are painted
/// again at the size the save writes, on a thread of their own, and take the smaller ones'
/// place. A change stops it between two bands of rows.
fn sharpen(r: &mut Ready, pictures: &Arc<HashMap<String, Arc<Raster>>>, mirror: Option<f32>, at_rest: bool) {
    let rests = at_rest && r.shown == r.sent && r.shown > 0 && r.shown_at.is_some_and(|t| t.elapsed().as_secs_f32() > 0.6);
    if !rests || r.sharpening.is_some() || r.sharp.as_ref().is_some_and(|x| x.0 == r.shown) {
        return;
    }
    let todo: Vec<(usize, u8, PathBuf, Option<[PathBuf; 2]>)> = r
        .targets
        .iter()
        .enumerate()
        .filter(|(k, _)| matches!((r.full.get(*k), r.bases.get(*k)), (Some(f), Some(b)) if *f != (b.width, b.height)))
        .map(|(k, t)| (k, t.index, t.base.clone(), t.template.clone()))
        .collect();
    if todo.is_empty() {
        r.sharp = Some((r.shown, Vec::new()));
        return;
    }
    let seq = r.shown;
    let (geom, outside, zones, wanted, n) = (r.geom.clone(), r.outside.clone(), r.zones.clone(), r.wanted.clone(), r.targets.len());
    let layers = r.last.as_ref().map(|x| x.0.clone()).unwrap_or_default();
    let pictures = pictures.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new().name("livery full size".into()).spawn(move || {
        let stop = || wanted.load(std::sync::atomic::Ordering::Relaxed) != seq;
        let from = export::Painting { geom: &geom, outside: &outside, zones: &zones, layers: &layers, pictures: &pictures, mirror };
        let t0 = Instant::now();
        let mut out: Vec<Option<omsi_texture::Image>> = vec![None; n];
        for (k, index, base, template) in &todo {
            match export::paint_full(&from, *k, *index, base, template.as_ref(), &stop, &|_| {}) {
                Ok(Some((rgba, width, height, has_alpha))) => out[*k] = Some(omsi_texture::Image { width, height, rgba, has_alpha }),
                // (a change came: this paint is not wanted any more)
                Ok(None) => return,
                Err(e) => {
                    let _ = tx.send((seq, Err(e)));
                    return;
                }
            }
        }
        log::info!("livery studio: {} texture(s) at their full size in {:.1} s", todo.len(), t0.elapsed().as_secs_f32());
        let _ = tx.send((seq, Ok(out)));
    });
    if spawned.is_ok() {
        r.sharpening = Some(rx);
    }
}

/// The pictures painted at the full size in place of the editing size's (a texture without
/// one keeps what it shows).
fn upload_full(showroom: &mut Showroom, renderer: &Renderer, targets: &[Target], pics: &[Option<omsi_texture::Image>]) {
    let Some(parts) = showroom.parts() else { return };
    for (t, img) in targets.iter().zip(pics) {
        let Some(img) = img else { continue };
        let mut again = false;
        for &id in &t.ids {
            again |= renderer.update_texture_mips(parts.scene, id, img);
        }
        if again {
            renderer.rebind_textures(parts.scene, &t.ids);
        }
    }
    showroom.touch();
}

/// The file name of a texture as a material names it, lower case and without its folder.
fn file_key(name: &str) -> String {
    name.trim().rsplit(['\\', '/']).next().unwrap_or("").to_ascii_lowercase()
}

/// One texture the livery paints: its file as the model names it (lower case, `key`), and the
/// `[CTCTexture]` slots of the bus's parts that take it.
#[derive(Clone, Debug, PartialEq)]
pub struct PaintTexture {
    pub key: String,
    pub file: String,
    /// (part, slot): 0 the front, then the rear sections.
    pub slots: Vec<(usize, String)>,
    /// The slots of its `<name>_#low` copy (the far LOD), which take the same picture.
    pub low: Vec<(usize, String)>,
}

/// The textures a livery paints on a bus of `parts` - per part its `[CTCTexture]` list and the
/// files of it a display shows (which keep theirs): every file once, with the slots that take
/// it in every part. An articulated bus's rear section has its own model, its own slots and
/// usually its own texture; a file both parts show is one texture with the slots of both.
pub fn paint_textures(parts: &[(&[(String, String)], &[String])]) -> Vec<PaintTexture> {
    let stem = |key: &str| key.rsplit_once('.').map(|(s, _)| s.to_string()).unwrap_or_else(|| key.to_string());
    let low_of = |key: &str| stem(key).strip_suffix("_#low").map(str::to_string);
    let mut out: Vec<PaintTexture> = Vec::new();
    for (p, (ctc, displays)) in parts.iter().enumerate() {
        for (slot, file) in ctc.iter() {
            let key = file_key(file);
            if key.is_empty() || displays.contains(&key) || low_of(&key).is_some() {
                continue;
            }
            match out.iter_mut().find(|t| t.key == key) {
                Some(t) => {
                    if !t.slots.iter().any(|s| s.0 == p && s.1.eq_ignore_ascii_case(slot)) {
                        t.slots.push((p, slot.clone()));
                    }
                }
                None => out.push(PaintTexture { key, file: file.trim().to_string(), slots: vec![(p, slot.clone())], low: Vec::new() }),
            }
        }
    }
    for (p, (ctc, _)) in parts.iter().enumerate() {
        for (slot, file) in ctc.iter() {
            let Some(of) = low_of(&file_key(file)) else { continue };
            if let Some(t) = out.iter_mut().find(|t| stem(&t.key) == of) {
                t.low.push((p, slot.clone()));
            }
        }
    }
    out
}

/// A part of the bus as the studio reads it: its type, the livery it shows (`part_scheme` for a
/// rear section), which of its meshes are shown and where each lies in the bus's frame.
pub struct PartIn<'a> {
    pub ty: &'a omsi_sim::VehicleType,
    pub scheme: Option<usize>,
    pub visible: Vec<bool>,
    pub xf: Vec<Mat4>,
}

/// The textures a livery paints on the bus of `parts` and its meshes as the bake takes them -
/// each with its part and its index there, every slot with its texture, and whether it is glass
/// or kept from the paint (`Role`) - and the meshes kept from the paint, by name (for the log).
pub fn bus_meshes<'a>(parts: &[PartIn<'a>]) -> (Vec<PaintTexture>, Vec<(usize, usize, MeshIn<'a>)>, Vec<String>) {
    // the painted textures: the parts' [CTCTexture]s, not a display's
    let displays: Vec<Vec<String>> = parts
        .iter()
        .map(|p| {
            p.ty.model
                .ctc_textures
                .iter()
                .map(|(_, f)| file_key(f))
                .filter(|key| p.ty.model.meshes.iter().flat_map(|m| m.materials.iter()).any(|o| file_key(&o.texture) == *key && (o.transmap.as_deref().is_some_and(|t| t.trim().starts_with("\\S:")) || o.use_script_texture.is_some() || o.use_text_texture.is_some())))
                .collect()
        })
        .collect();
    let lists: Vec<(&[(String, String)], &[String])> = parts.iter().zip(&displays).map(|(p, d)| (p.ty.model.ctc_textures.as_slice(), d.as_slice())).collect();
    let textures = paint_textures(&lists);
    let keys: Vec<&str> = textures.iter().map(|d| d.key.as_str()).collect();
    let mut meshes = Vec::new();
    let mut kept: Vec<String> = Vec::new();
    for (pi, p) in parts.iter().enumerate() {
        for (i, vm) in p.ty.meshes.iter().enumerate() {
            if !p.visible.get(i).copied().unwrap_or(false) || vm.data.indices.is_empty() {
                continue;
            }
            let def = &p.ty.model.meshes[vm.def_index];
            let role = Role::of_mesh(&def.file, turns_as_wheel(def), def.viewpoint == 0 || def.viewpoint & 1 != 0);
            let slots: Vec<(Option<u8>, bool, bool)> = vm
                .materials
                .iter()
                .enumerate()
                .map(|(slot, m)| {
                    let target = keys.iter().position(|k| *k == file_key(&m.texture)).map(|k| k as u8);
                    let blended = def.materials.iter().any(|d| omsi_sim::vehicle::override_slot(&vm.materials, d) == Some(slot) && d.alpha == 2) || m.diffuse[3] < 0.99;
                    match role.slot(blended) {
                        SlotUse::Paint => (target, false, false),
                        SlotUse::Glass => (target, true, false),
                        SlotUse::Keep => (target, blended, true),
                        SlotUse::Skip => (None, blended, false),
                    }
                })
                .collect();
            if !matches!(role, Role::Body | Role::Decal | Role::Glass) && vm.materials.iter().any(|m| keys.contains(&file_key(&m.texture).as_str())) {
                kept.push(format!("{} ({role:?})", def.file));
            }
            meshes.push((pi, i, MeshIn { positions: &vm.data.positions, normals: &vm.data.normals, uvs: &vm.data.uvs, indices: &vm.data.indices, ranges: &vm.data.ranges, slots, transform: p.xf[i] }));
        }
    }
    (textures, meshes, kept)
}

/// Whether the model turns mesh `def` as a wheel (`Wheel_Rotation_<n>_*`).
fn turns_as_wheel(def: &omsi_model::MeshDef) -> bool {
    def.animations.iter().any(|a| a.kind == Some(omsi_model::AnimKind::Rot) && a.variable.to_ascii_lowercase().starts_with("wheel_rotation_"))
}

/// Texels the studio paints at while editing, over all the textures: the largest ones first at
/// their editing size, the rest halved as long as they would go beyond it.
const EDIT_TEXELS: u64 = 10 << 20;

/// Read the bus's shape and paint slots from the showroom and start the preparation.
fn prepare(showroom: &mut Showroom, s: &mut Session, root: &str, family_buses: &[(String, String)]) -> Result<(), String> {
    let root = PathBuf::from(root);
    let Some(parts) = showroom.parts() else { return Err("no bus".into()) };
    let vt = parts.vt.clone();
    let vehicle = parts.vehicle;
    // the bus's parts: the front, and the rear sections behind it with the livery of the same name
    let scheme = crate::spawn::paint_scheme(&vt, s.project.start_paint.as_deref());
    let mut list: Vec<PartIn> = vec![PartIn { ty: &vt, scheme, visible: vehicle.mesh_props.iter().map(|p| p.visible).collect(), xf: (0..vt.meshes.len()).map(|i| vehicle.mesh_local_transform(i)).collect() }];
    let mut renders = vec![parts.render];
    for (t, r) in vehicle.trailers.iter().zip(parts.trailers.iter()) {
        let off = Mat4::from_translation((t.position - vehicle.position).as_vec3());
        list.push(PartIn { ty: &t.ty, scheme: crate::spawn::part_scheme(&vt, scheme, &t.ty), visible: t.mesh_props.iter().map(|p| p.visible).collect(), xf: (0..t.ty.meshes.len()).map(|i| off * t.mesh_local_transform(i)).collect() });
        renders.push(r);
    }
    let (textures, found, kept) = bus_meshes(&list);
    if textures.is_empty() {
        return Err(omsi_ui::tr("This bus has no paint the game lets change ([CTC]).").into_owned());
    }
    // the texture slots in the scene that show each texture
    let mut ids: Vec<Vec<TextureId>> = vec![Vec::new(); textures.len()];
    let mut area = vec![0.0f32; textures.len()];
    let mut meshes: Vec<MeshIn> = Vec::with_capacity(found.len());
    for (p, i, m) in found {
        for (slot, (target, ..)) in m.slots.iter().enumerate() {
            let Some(t) = target else { continue };
            if let Some(inst) = renders[p].instances.get(i).and_then(|k| parts.scene.instances.get(*k)) {
                if let Some(tex) = inst.materials.get(slot).and_then(|m| parts.scene.materials.get(*m)).and_then(|m| m.texture) {
                    if !ids[*t as usize].contains(&tex) {
                        ids[*t as usize].push(tex);
                    }
                }
            }
        }
        meshes.push(m);
    }
    let geom = BusGeom::build(&meshes);
    // (the bus framed at once: while the paint is prepared the camera stood in the bus)
    if s.cam.dist == 0.0 {
        let aspect = s.ui.view().map(|r| r.w / r.h.max(1.0)).unwrap_or(1.7);
        s.cam = studio::Cam::fitting(&geom.dims, 6, aspect);
    }
    for t in &geom.tris {
        if let Some(k) = t.target {
            area[k as usize] += (t.p[1] - t.p[0]).cross(t.p[2] - t.p[0]).length() * 0.5;
        }
    }
    // the bases: the start livery's textures (each part's own), or the model's own (a plain
    // start: in the livery's colours, which it is read for)
    let plain = s.project.plain_start();
    let tinted = s.project.start == "effenKleuren";
    let mut targets = Vec::new();
    for (k, t) in textures.iter().enumerate() {
        if ids[k].is_empty() || area[k] < 0.5 {
            continue;
        }
        let PartIn { ty, scheme: sch, .. } = &list[t.slots.first().map(|s| s.0).unwrap_or(0)];
        let find = |sch: Option<usize>| {
            let (subs, sdir) = sch.map(|i| ty.scheme_substitutions(i)).unwrap_or_default();
            let mut dirs: Vec<PathBuf> = sdir.iter().cloned().collect();
            dirs.extend(ty.texture_dirs(&root));
            let dir_refs: Vec<&Path> = dirs.iter().map(|d| d.as_path()).collect();
            let wanted = subs.get(&t.key).cloned().unwrap_or_else(|| t.file.clone());
            let found = sdir.as_ref().map(|d| omsi_cfg::resolve_path(d, &wanted)).filter(|p| p.is_file()).or_else(|| omsi_texture::find_texture(&wanted, &dir_refs)).or_else(|| omsi_texture::find_texture(&t.file, &dir_refs));
            (found, wanted)
        };
        let (base, wanted) = find(if plain { None } else { *sch });
        let Some(base) = base else {
            log::warn!("livery studio: the texture {wanted} was not found");
            continue;
        };
        let tint = if tinted && sch.is_some() { find(*sch).0.filter(|p| *p != base) } else { None };
        let slots = t.slots.iter().chain(&t.low).cloned().collect();
        let template = find_template(&root, &t.file);
        if template.is_some() {
            log::info!("livery studio: {} has its maker's template", t.file);
        }
        targets.push((area[k], Target { index: k as u8, default: t.file.clone(), slots, ids: ids[k].clone(), base, tint, template }));
    }
    targets.sort_by(|a, b| b.0.total_cmp(&a.0));
    let targets: Vec<Target> = targets.into_iter().take(8).map(|t| t.1).collect();
    if targets.is_empty() {
        return Err(omsi_ui::tr("The paint textures of this bus were not found.").into_owned());
    }
    // can the windows carry the livery: their texture is one of these
    let windows = geom.tris.iter().any(|t| t.glass).then(|| geom.tris.iter().any(|t| t.glass && t.target.is_some_and(|k| targets.iter().any(|x| x.index == k))));
    // the names the game knows for this bus, and where each part's .cti files lie
    let own = s.project.placed.as_ref().map(|p| p.name.clone());
    let mut taken: Vec<String> = Vec::new();
    for PartIn { ty, .. } in &list {
        for p in &ty.paint_schemes {
            if own.as_ref().is_none_or(|o| !o.eq_ignore_ascii_case(&p.name)) && !taken.iter().any(|n| n.eq_ignore_ascii_case(&p.name)) {
                taken.push(p.name.clone());
            }
        }
    }
    s.taken = taken;
    // its family, once (read while it is shown on itself)
    if s.family.is_empty() && s.family_rx.is_none() && s.look.bus == s.project.bus {
        let keys: Vec<String> = targets.iter().map(|t| file_key(&t.default)).collect();
        let (bus, root2) = (s.project.bus.clone(), root.clone());
        let buses: Vec<(String, String)> = family_buses.to_vec();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new().name("livery family".into()).spawn(move || {
            let _ = tx.send(family(&root2, &bus, &buses, &keys));
        }).ok();
        s.family_rx = Some(rx);
    }
    s.parts = list.iter().map(|PartIn { ty, scheme: sch, .. }| (ty.model.ctc.first().map(|c| omsi_cfg::resolve_path(ty.def.dir(), &c.path)), sch.map(|i| ty.paint_schemes[i].set_vars.clone()).unwrap_or_default())).collect();
    log::info!(
        "livery studio: {} part(s), {} meshes, {} triangles, painting {}; the box {:.2?} - {:.2?}, the window line {:?}, windows {}",
        list.len(),
        meshes.len(),
        geom.tris.len(),
        targets.iter().map(|t| format!("{} ({})", t.default, t.slots.iter().map(|s| format!("{}:{}", s.0, s.1)).collect::<Vec<_>>().join(" "))).collect::<Vec<_>>().join(", "),
        geom.dims.min,
        geom.dims.max,
        geom.dims.window,
        match windows {
            Some(true) => "paintable",
            Some(false) => "not paintable",
            None => "none",
        }
    );
    log::info!("livery studio: kept from the paint: {}", if kept.is_empty() { "nothing".into() } else { kept.join(", ") });
    let geom = Arc::new(geom);
    let (tx, rx) = std::sync::mpsc::channel();
    let work = targets.clone();
    std::thread::Builder::new()
        .name("livery bake".into())
        .spawn(move || {
            let t0 = Instant::now();
            let r = (|| -> Result<(Prepared, Vec<Target>), String> {
                let outside = Arc::new(Outside::build(&geom.tris));
                let mut bakes = Vec::new();
                let mut bases = Vec::new();
                let mut full = Vec::new();
                let mut kept = Vec::new();
                let mut texels = 0u64;
                for t in work {
                    let img = omsi_texture::decode_file(&t.base).map_err(|e| format!("{}: {e}", t.base.display()))?;
                    let (ow, oh) = model::output_size(img.width, img.height);
                    let (mut w, mut h) = model::edit_size(ow, oh);
                    while texels + (w as u64 * h as u64) > EDIT_TEXELS && w.min(h) >= 512 {
                        w /= 2;
                        h /= 2;
                    }
                    let rgba = if (w, h) != (img.width, img.height) { omsi_texture::bc::resize(&img.rgba, img.width, img.height, w, h) } else { img.rgba };
                    let bake = bake::Bake::build(&geom.tris, t.index, w, h, 0, h, &outside);
                    // (a texture nothing of the outside shows, an interior's, is not painted)
                    if !bake.first.iter().any(|s| s.flags & bake::OUTSIDE != 0) {
                        log::info!("livery studio: {} shows nothing outside", t.default);
                        continue;
                    }
                    texels += w as u64 * h as u64;
                    let has_alpha = rgba.chunks_exact(4).any(|p| p[3] < 255);
                    bakes.push(bake);
                    bases.push(omsi_texture::Image { width: w, height: h, rgba, has_alpha });
                    full.push((ow, oh));
                    kept.push(t);
                }
                if kept.is_empty() {
                    return Err(omsi_ui::tr("The paint textures of this bus were not found.").into_owned());
                }
                let colours: Vec<Vec<[u8; 3]>> = bakes.iter().zip(&bases).map(|(b, base)| paint::outside_colours(b, &base.rgba)).collect();
                let zones = Arc::new(Zones::of_targets(&colours));
                let canvases: Vec<paint::Canvas> = bakes
                    .into_iter()
                    .zip(&bases)
                    .zip(zones.iter())
                    .zip(&kept)
                    .map(|(((b, base), z), t)| {
                        let tp = t.template.as_ref().and_then(|tp| template_pictures(tp, base.width, base.height)).map(|[ma, mu]| paint::Template::rows(&ma, &mu, base.width, 0, base.height));
                        paint::Canvas::with(b, &base.rgba, z).with_template(tp)
                    })
                    .collect();
                // a plain start in a livery's colours: per paint group the livery's median there
                let mut by_group: std::collections::BTreeMap<u8, Vec<[u8; 3]>> = Default::default();
                for ((t, cv), base) in kept.iter().zip(&canvases).zip(&bases) {
                    let Some(tint) = t.tint.as_ref() else { continue };
                    let Ok(img) = omsi_texture::decode_file(tint) else { continue };
                    let rgba = if (img.width, img.height) != (base.width, base.height) { omsi_texture::bc::resize(&img.rgba, img.width, img.height, base.width, base.height) } else { img.rgba };
                    for (i, smp) in cv.bake.first.iter().enumerate() {
                        if smp.flags & bake::OUTSIDE == 0 || smp.flags & (bake::KEEP | bake::GLASS) != 0 || i % 3 != 0 {
                            continue;
                        }
                        let (a, b, t) = cv.mix[i];
                        let t = t as f32 / 50_000.0;
                        let z = if t >= 0.8 { a } else if t <= 0.2 { b } else { continue };
                        if z == bake::BLACK || !cv.zones.is_paint(z as usize) {
                            continue;
                        }
                        by_group.entry(cv.zones.group[z as usize]).or_default().push([rgba[i * 4], rgba[i * 4 + 1], rgba[i * 4 + 2]]);
                    }
                }
                let start_colours = by_group
                    .into_iter()
                    .filter(|(_, v)| v.len() >= 16)
                    .map(|(g, mut v)| {
                        let mut med = [0u8; 3];
                        for (k, m) in med.iter_mut().enumerate() {
                            v.sort_by_key(|c| c[k]);
                            *m = v[v.len() / 2][k];
                        }
                        (g, med)
                    })
                    .collect();
                log::info!(
                    "livery studio: prepared in {:.2} s; zones {}",
                    t0.elapsed().as_secs_f32(),
                    kept.iter().zip(zones.iter()).map(|(t, z)| format!("{}: {}", t.default, (0..z.centres.len()).map(|i| format!("L{:.0} a{:.0} b{:.0} {:.0}%{}", z.centres[i][0], z.centres[i][1], z.centres[i][2], z.share[i] * 100.0, if z.group[i] == bake::NO_GROUP { String::new() } else { format!(" paint {}", z.group[i]) })).collect::<Vec<_>>().join(", "))).collect::<Vec<_>>().join("; ")
                );
                Ok((Prepared { geom, outside, zones, canvases, bases, full, windows, start_colours }, kept))
            })();
            let _ = tx.send(r);
        })
        .map_err(|e| e.to_string())?;
    s.preparing = Some((rx, targets));
    Ok(())
}

/// Save the livery into the game (on a worker; `saved` follows).
pub fn save(l: &mut Launcher) {
    // (for the bus company: not when it cannot pay for it)
    if let Some((company, cost, cash)) = company_cost(l).filter(|x| x.2 < x.1) {
        if let Some(s) = l.livery.session.as_mut() {
            let text = omsi_ui::tr("%{company} has %{cash}: not enough for the livery (%{amount}). A loan on the company's Finances page helps.").replace("%{company}", &company).replace("%{cash}", &eur(cash)).replace("%{amount}", &eur(cost));
            s.say(text, true);
        }
        return;
    }
    let Some(s) = l.livery.session.as_mut() else { return };
    if s.ready.is_none() || s.export.is_some() {
        return;
    }
    s.settle();
    if s.look.bus != s.project.bus {
        s.say(omsi_ui::tr("Go back to your own bus to save the livery."), true);
        return;
    }
    let name = s.ui.name.clone();
    if let Some(e) = model::name_error(&name, s.taken.iter().map(|n| n.as_str())) {
        s.say(e.message(), true);
        return;
    }
    if l.state.in_game() {
        s.queued = true;
        s.say(omsi_ui::tr("The game is running: the livery is saved into it as soon as the game is closed."), false);
        return;
    }
    s.queued = false;
    let Some(content) = omsi_launcher_lib::content_dir() else {
        s.say(omsi_ui::tr("The content folder was not found."), true);
        return;
    };
    // every part's [CTC] folder (an articulated bus's rear section has one too), as the content
    // folder has it
    let root = PathBuf::from(&l.state.config.root);
    let mut parts = Vec::new();
    for (ctc, mut setvars) in s.parts.clone() {
        let Some(ctc) = ctc else {
            parts.push(None);
            continue;
        };
        // (the bus options chosen in the studio over the start livery's)
        for (var, v) in &s.project.options {
            match setvars.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(var)) {
                Some(x) => x.1 = *v,
                None => setvars.push((var.clone(), *v)),
            }
        }
        let ctc_dir = if ctc.starts_with(&content) {
            ctc
        } else if let Ok(rel) = ctc.strip_prefix(&root) {
            content.join(rel)
        } else {
            s.say(omsi_ui::tr("This bus lies outside the game's folders; its livery cannot be saved."), true);
            return;
        };
        if !export::inside(&ctc_dir, &content) {
            return;
        }
        if let Err(e) = std::fs::create_dir_all(&ctc_dir) {
            s.say(format!("{}: {e}", ctc_dir.display()), true);
            return;
        }
        parts.push(Some(export::Part { ctc_dir, setvars }));
    }
    if parts.first().is_none_or(|p| p.is_none()) {
        s.say(omsi_ui::tr("This bus has no paint the game lets change ([CTC])."), true);
        return;
    }
    // the buses of its family it is saved for too, with a [CTC] folder of their own: their
    // folder a part more, their slots for the textures
    let mut extra: Vec<(usize, Vec<(String, String)>)> = Vec::new();
    for kin in s.family.iter().filter(|k| s.project.family.contains(&k.bus)) {
        let Some(ctc) = kin.ctc.clone() else { continue };
        let ctc_dir = if ctc.starts_with(&content) {
            ctc
        } else if let Ok(rel) = ctc.strip_prefix(&root) {
            content.join(rel)
        } else {
            continue;
        };
        if parts.iter().flatten().any(|p| p.ctc_dir == ctc_dir) || !export::inside(&ctc_dir, &content) || std::fs::create_dir_all(&ctc_dir).is_err() {
            continue;
        }
        parts.push(Some(export::Part { ctc_dir, setvars: s.project.options.iter().map(|(k, v)| (k.clone(), *v)).collect() }));
        extra.push((parts.len() - 1, kin.slots.clone()));
    }
    // (one number in every folder, free in each)
    let nnnn = s.project.placed.as_ref().map(|p| p.nnnn).unwrap_or_else(|| parts.iter().flatten().map(|p| export::next_number(&p.ctc_dir)).max().unwrap_or(1));
    let Some(r) = s.ready.as_ref() else { return };
    let job = export::Job {
        name: name.clone(),
        nnnn,
        date: today(),
        content: content.clone(),
        parts,
        targets: r
            .targets
            .iter()
            .map(|t| {
                let mut slots = t.slots.clone();
                for (k, slot) in &extra {
                    for (key, name) in slot {
                        if *key == file_key(&t.default) && !slots.contains(&(*k, name.clone())) {
                            slots.push((*k, name.clone()));
                        }
                    }
                }
                export::Target { index: t.index, default: t.default.clone(), slots, base: t.base.clone(), template: t.template.clone() }
            })
            .collect(),
        geom: r.geom.clone(),
        outside: r.outside.clone(),
        zones: r.zones.clone(),
        layers: s.project.layers.clone(),
        pictures: s.pictures.clone(),
        mirror: s.mirror_plane(),
        old: s.project.placed.as_ref().map(|p| p.files.clone()).unwrap_or_default(),
    };
    s.project.name = name;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new().name("livery export".into()).spawn(move || export::run(job, tx)).ok();
    s.export = Some(rx);
    s.progress = Some((omsi_ui::tr("Painting the textures…").into_owned(), 0.0));
}

/// Take the livery out of the game again: the files its last save wrote - only the studio's own,
/// only in the content folder, the `.cti` files first so that the game forgets it before its
/// textures go - and their folders once empty. The project keeps its layers; Save puts it back.
pub fn remove_from_game(l: &mut Launcher) {
    let root = l.state.config.root.clone();
    let Some(s) = l.livery.session.as_mut() else { return };
    if s.export.is_some() {
        return;
    }
    let Some(placed) = s.project.placed.clone() else { return };
    if l.state.in_game() {
        s.say(omsi_ui::tr("Close the game first: it may be using the livery."), true);
        return;
    }
    let Some(content) = omsi_launcher_lib::content_dir() else {
        s.say(omsi_ui::tr("The content folder was not found."), true);
        return;
    };
    let mut files = placed.files.clone();
    files.sort_by_key(|f| !f.to_ascii_lowercase().ends_with(".cti"));
    let mut left = Vec::new();
    for f in files {
        let p = content.join(&f);
        if !f.contains(model::PREFIX) || !export::inside(&p, &content) {
            continue;
        }
        if p.is_file() && std::fs::remove_file(&p).is_err() {
            left.push(f);
            continue;
        }
        // (the texture's folder `LiveryStudio\0007_slug`, and `LiveryStudio` itself, once empty)
        let mut dir = p.parent();
        while let Some(d) = dir.filter(|d| d.starts_with(&content) && d.to_string_lossy().contains(model::PREFIX)) {
            if std::fs::remove_dir(d).is_err() {
                break;
            }
            dir = d.parent();
        }
    }
    if !left.is_empty() {
        log::warn!("livery studio: could not remove {left:?}");
        s.project.placed = Some(model::Placed { files: left, ..placed });
        s.save_project();
        s.say(omsi_ui::tr("Some files of the livery could not be removed (is the game using them?)."), true);
        return;
    }
    omsi_cfg::content_changed();
    log::info!("livery studio: '{}' removed from the game", placed.name);
    s.project.placed = None;
    s.save_project();
    s.say(omsi_ui::tr("'%{name}' is no longer in the game.").replace("%{name}", &placed.name), false);
    let bus = s.project.bus.clone();
    if let Some(v) = l.state.vehicles.iter_mut().find(|v| v.file == bus) {
        v.paints.retain(|p| !p.eq_ignore_ascii_case(&placed.name));
    }
    l.showroom.photos.forget(&root, &bus, &placed.name);
    if l.state.choice.bus == bus && l.state.choice.paint.eq_ignore_ascii_case(&placed.name) {
        l.state.choice.paint = String::new();
        l.state.touched();
    }
}

/// What the project keeps of a save into the game that wrote `files`.
fn placed_after(project: &Project, files: Vec<String>) -> model::Placed {
    let nnnn = files.iter().find_map(|f| f.rsplit('/').next().and_then(|n| n.strip_prefix(&format!("~{}_", model::PREFIX))).and_then(|r| r.get(..4)).and_then(|n| n.parse().ok())).unwrap_or(1);
    let version = project.placed.as_ref().map(|p| p.version + 1).unwrap_or(1);
    model::Placed { name: project.name.clone(), nnnn, version, files }
}

/// Leave the studio (Back), safe at any moment: the project saved first. What still works goes on
/// by itself with what it holds and touches nothing of the studio's: the bake and the painter
/// end with nobody to tell (their channels closed), a save into the game finishes and its files
/// are kept in the project. The bus's GPU objects go with the showroom (the device keeps what a
/// frame in flight still uses).
pub fn leave(l: &mut Launcher) {
    if let Some(mut s) = l.livery.session.take() {
        s.settle();
        s.save_project();
        if let Some(rx) = s.export.take() {
            let (mut project, dir) = (s.project.clone(), s.dir.clone());
            let _ = std::thread::Builder::new().name("livery export (studio left)".into()).spawn(move || {
                while let Ok(m) = rx.recv() {
                    match m {
                        export::Msg::Done(files) => {
                            project.placed = Some(placed_after(&project, files));
                            project.saved = now_text();
                            let _ = std::fs::write(dir.join("project.json"), serde_json::to_vec_pretty(&project).unwrap_or_default());
                            log::info!("livery studio: '{}' saved into the game after the studio was left", project.name);
                            break;
                        }
                        export::Msg::Failed(e) => {
                            log::warn!("livery studio: '{}' was not saved: {e}", project.name);
                            break;
                        }
                        export::Msg::Progress(..) => {}
                    }
                }
            });
        }
        log::info!("livery studio: left {}", s.project.id);
    }
    l.livery.view_rect = None;
    l.livery.hold_o = false;
    l.livery.showroom.forget();
    // (painting for the bus company: back to it)
    let back = if l.livery.company.take().is_some() { Page::Company } else { Page::Drive };
    l.go(back);
}

/// Whole euros as the company's pages write them.
fn eur(c: omsi_launcher_lib::company::Cents) -> String {
    super::company::eur(c)
}

/// A livery saved for the bus company: its design booked, and the bus it was painted for sent
/// to the workshop to be painted in it.
fn saved_for_company(l: &mut Launcher, name: &str, bus: &str) {
    use omsi_launcher_lib::company::livery;
    let Some(fc) = l.livery.company.clone() else { return };
    if l.company.company.as_ref().is_none_or(|c| c.id != fc.id) {
        return;
    }
    let (name, bus) = (name.to_string(), bus.to_string());
    let Some(cost) = super::company::act(l, |c| livery::save_design(c, &name, &bus)) else { return };
    let mut text = omsi_ui::tr("'%{name}' is the company's livery: %{amount} for its design.").replace("%{name}", &name).replace("%{amount}", &eur(cost));
    if let Some(id) = fc.vehicle {
        if super::company::act(l, |c| livery::paint(c, id, &name)).is_some() {
            let number = l.company.company.as_ref().and_then(|c| c.vehicle(id)).map(|v| v.number.clone()).unwrap_or_default();
            text = format!("{text} {}", omsi_ui::tr("Bus %{n} is painted in it in the workshop.").replace("%{n}", &number));
        }
    }
    if let Some(s) = l.livery.session.as_mut() {
        s.say(text, false);
    }
}

/// The livery is in the game: kept in the project, the bus's list and photo told.
fn saved(l: &mut Launcher, files: Vec<String>) {
    let root = l.state.config.root.clone();
    let Some(s) = l.livery.session.as_mut() else { return };
    let name = s.project.name.clone();
    let old_name = s.project.placed.as_ref().map(|p| p.name.clone());
    s.project.placed = Some(placed_after(&s.project, files));
    s.save_project();
    s.say(omsi_ui::tr("'%{name}' is in the game: choose it on the bus step.").replace("%{name}", &name), false);
    let bus = s.project.bus.clone();
    crate::mt::protect([name.as_str()]);
    // the bus step's list and photo
    if let Some(v) = l.state.vehicles.iter_mut().find(|v| v.file == bus) {
        if let Some(old) = old_name.filter(|o| !o.eq_ignore_ascii_case(&name)) {
            v.paints.retain(|p| !p.eq_ignore_ascii_case(&old));
        }
        if !v.paints.iter().any(|p| p.eq_ignore_ascii_case(&name)) {
            v.paints.push(name.clone());
        }
    }
    l.showroom.photos.forget(&root, &bus, &name);
    // (the bus step's bus read again: a save under a name it already shows has new textures)
    l.showroom.reread(&bus);
    if l.state.choice.bus == bus && l.livery.company.is_none() {
        l.state.choice.paint = name.clone();
        l.state.touched();
    }
    saved_for_company(l, &name, &bus);
}

/// The studio's page.
pub fn draw(l: &mut Launcher) {
    studio::draw(l);
}

/// On a phone: the studio needs a mouse and a larger screen.
pub fn draw_phone(l: &mut Launcher, r: omsi_ui::Rect) {
    l.ui.paragraph("The livery studio needs a computer: open the launcher on your PC to paint a bus.", Vec2::new(r.x + 8.0, r.y + 8.0), r.w - 16.0, 14.0, omsi_ui::Weight::Regular, super::theme::TEXT_DIM);
}

/// The studio's picture and the shapes' pictures onto the interface's GPU (after the interface
/// was laid out: its texture is drawn from the next frame).
pub fn gpu(v: &mut LiveryView, renderer: &mut Renderer, gpu: &mut omsi_ui::Gpu, scale: f32) {
    if let Some(r) = v.view_rect.take() {
        let (w, h) = ((r.w * scale) as u32, (r.h * scale) as u32);
        // the other side, drawn with the bus (its camera the studio's mirrored)
        let other = v.other_rect.and_then(|o| v.session.as_ref().and_then(|s| s.other_camera()).map(|c| (c, (o.w * scale) as u32, (o.h * scale) as u32)));
        v.showroom.set_second(other);
        if let Some(view) = v.showroom.preview(renderer, w.max(16), h.max(16)) {
            if v.view_gen != v.showroom.generation || v.view_tex.is_none() {
                v.view_gen = v.showroom.generation;
                match v.view_tex {
                    Some(id) => gpu.set_view(&renderer.device, id, &view, (w.max(16), h.max(16))),
                    None => v.view_tex = Some(gpu.add_view(&renderer.device, &view, (w.max(16), h.max(16)))),
                }
            }
        }
        if let Some((view, sw, sh)) = v.showroom.second_view() {
            if v.other_gen != v.showroom.second_generation || v.other_tex.is_none() {
                v.other_gen = v.showroom.second_generation;
                match v.other_tex {
                    Some(id) => gpu.set_view(&renderer.device, id, &view, (sw, sh)),
                    None => v.other_tex = Some(gpu.add_view(&renderer.device, &view, (sw, sh))),
                }
            }
        }
        if v.shape_tex.is_empty() {
            for (key, _) in shapes::SHAPES {
                let (m, _) = shapes::shape_raster(key, 96, 96, 0.0);
                let rgba: Vec<u8> = m.data.iter().flat_map(|a| [255, 255, 255, *a]).collect();
                let size = wgpu::Extent3d { width: 96, height: 96, depth_or_array_layers: 1 };
                let tex = renderer.device.create_texture(&wgpu::TextureDescriptor { label: Some("livery shape"), size, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba8UnormSrgb, usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[] });
                renderer.queue.write_texture(tex.as_image_copy(), &rgba, wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(96 * 4), rows_per_image: Some(96) }, size);
                let view = tex.create_view(&Default::default());
                v.shape_tex.insert(key, gpu.add_view(&renderer.device, &view, (96, 96)));
            }
        }
    }
}

/// `OMSI_LAUNCHER_LIVERY=<bus file>[|<livery>]` or `=project:<id>`: the launcher opens on the studio
/// (for looking at it).
pub fn from_env(l: &mut Launcher) {
    if let Ok(v) = omsi_cfg::env::var("OMSI_LAUNCHER_LIVERY") {
        // (`project:<id>`: a livery begun before)
        if let Some(id) = v.strip_prefix("project:") {
            if let Some(p) = projects().into_iter().find(|p| p.id == id) {
                l.go(Page::Livery);
                let (bus, paint) = (p.bus.clone(), p.start_paint.clone());
                start(l, bus, paint, Some(p));
            }
            return;
        }
        let (bus, paint) = match v.split_once('|') {
            Some((b, p)) => (b.to_string(), Some(p.to_string())),
            None => (v.clone(), None),
        };
        open(l, Some(bus).filter(|b| !b.is_empty()), paint);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn an_articulated_bus_paints_both_parts_with_their_own_slots() {
        // the HH20 E-bus's front and rear section (model_21_main.cfg, model_21_T.cfg)
        let s = |v: &[(&str, &str)]| v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect::<Vec<_>>();
        let main = s(&[("farbschema_tex1", "newC2EG.tga"), ("farbschema_tex2", "21_decals.dds"), ("farbschema_tex3", "21_innen_1.bmp"), ("farbschema_tex4", "17_trans.dds"), ("farbschema_tex7", "newC2EG_#low.tga")]);
        let trail = s(&[("farbschema_tex5", "newC2EG_T.tga"), ("farbschema_tex2", "21_decals.dds"), ("farbschema_tex3", "21_innen_1.bmp"), ("farbschema_tex6", "21_T_trans.dds"), ("farbschema_tex8", "newC2EG_T_#low.tga"), ("anzeige", "matrix.bmp")]);
        let none: Vec<String> = Vec::new();
        let display = vec!["matrix.bmp".to_string()];
        let t = super::paint_textures(&[(main.as_slice(), none.as_slice()), (trail.as_slice(), display.as_slice())]);
        let find = |k: &str| t.iter().find(|x| x.key == k).unwrap_or_else(|| panic!("{k}: {t:?}"));
        let front = find("newc2eg.tga");
        assert_eq!(front.slots, vec![(0, "farbschema_tex1".to_string())]);
        assert_eq!(front.low, vec![(0, "farbschema_tex7".to_string())], "the far LOD takes the same picture");
        let rear = find("newc2eg_t.tga");
        assert_eq!(rear.slots, vec![(1, "farbschema_tex5".to_string())], "the rear section's own texture and slot");
        assert_eq!(rear.low, vec![(1, "farbschema_tex8".to_string())]);
        assert_eq!(find("21_decals.dds").slots, vec![(0, "farbschema_tex2".to_string()), (1, "farbschema_tex2".to_string())], "one texture both parts show");
        assert_eq!(find("21_t_trans.dds").slots, vec![(1, "farbschema_tex6".to_string())], "the rear windows");
        assert!(t.iter().all(|x| x.key != "matrix.bmp" && !x.key.contains("#low")), "{t:?}");
        assert_eq!(t.len(), 6);
    }

    use super::{load_project, temp_writer, Project, TempMsg, TEMP_FILE};

    /// The temporary file is the project when it is newer than `project.json`, or alone; the
    /// worker writes it whole and takes it away again in the order asked.
    #[test]
    fn the_temporary_file_brings_back_what_was_not_saved() {
        let dir = std::env::temp_dir().join(format!("livery-temp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut p = Project::new("p1".into(), "Vehicles/x.bus".into(), None, "1".into());
        p.name = "saved".into();
        // alone: it is the project
        std::fs::write(dir.join(TEMP_FILE), serde_json::to_vec(&p).unwrap()).unwrap();
        assert_eq!(load_project(&dir).map(|x| (x.0.name, x.1)), Some(("saved".into(), true)));
        // older than project.json: project.json
        std::thread::sleep(std::time::Duration::from_millis(30));
        std::fs::write(dir.join("project.json"), serde_json::to_vec(&p).unwrap()).unwrap();
        assert_eq!(load_project(&dir).map(|x| x.1), Some(false));
        // (gone once project.json caught up, as saving takes it away)
        std::fs::remove_file(dir.join(TEMP_FILE)).unwrap();
        // newer: the temporary file's
        std::thread::sleep(std::time::Duration::from_millis(30));
        p.name = "painted on".into();
        temp_writer().send(TempMsg::Write(dir.clone(), serde_json::to_vec(&p).unwrap())).unwrap();
        let wait = |want: bool| {
            for _ in 0..200 {
                if dir.join(TEMP_FILE).exists() == want {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            panic!("the temporary file is still {}", if want { "missing" } else { "there" });
        };
        wait(true);
        assert_eq!(load_project(&dir).map(|x| (x.0.name, x.1)), Some(("painted on".into(), true)));
        assert!(!dir.join(format!("{TEMP_FILE}.part")).exists());
        temp_writer().send(TempMsg::Remove(dir.clone())).unwrap();
        wait(false);
        assert_eq!(load_project(&dir).map(|x| (x.0.name, x.1)), Some(("saved".into(), false)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn texture_names_are_compared_by_their_file() {
        assert_eq!(super::file_key(" Texture\\SD80_01.TGA "), "sd80_01.tga");
        assert_eq!(super::file_key("sd80_01.tga"), "sd80_01.tga");
    }
}
