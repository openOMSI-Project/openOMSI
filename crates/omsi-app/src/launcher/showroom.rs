//! The launcher's bus preview: the chosen bus drawn by the game's own renderer - its
//! model, paint and materials - on the plain (Vanilla+) path without the costly passes,
//! under the light of the chosen time and weather, into a picture the launcher shows
//! in a card. It is drawn again only when something changed (another bus, paint, light,
//! the preview turned by the mouse), never every frame.
//!
//! A bus is read on a worker (its type, its scripts run to their resting state, its
//! textures and meshes put on the GPU ahead) and then placed into a scene of its own with
//! a fresh `World` (whose caches belong to one scene); the old scene goes when the new one
//! is ready, so the picture never goes blank while switching.
//!
//! A second showroom, the studio, takes the bus picker's photos (see `busphoto`): the same
//! bus in the same light, from a corner that never moves, read back into a picture. The
//! photos are kept here because they go with the graphics device, as the showroom does.

use super::super::*;
use glam::{DVec3, Vec2, Vec3};
use omsi_render::{Camera, Lighting, Renderer, Scene};
use std::sync::mpsc::{channel, Receiver};

/// What the showroom shows.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Look {
    pub root: PathBuf,
    pub map: String,
    pub bus: String,
    pub paint: String,
    pub weather: String,
    /// Minutes of the day, and the date.
    pub time: i32,
    pub date: String,
}

struct Ready {
    look: Look,
    world: Arc<scene::World>,
    vt: Arc<omsi_sim::VehicleType>,
    vehicle: omsi_sim::VehicleInstance,
    scheme: Option<usize>,
    /// The bus options it was made with.
    options: Vec<(String, f32)>,
}

struct Shown {
    look: Look,
    scene: Scene,
    world: Option<Arc<scene::World>>,
    vehicle: Option<omsi_sim::VehicleInstance>,
    /// Its livery, and the bus options it wears.
    scheme: Option<usize>,
    options: Vec<(String, f32)>,
    /// The display font its destination displays are drawn in (None: its own).
    letters: Option<String>,
    render: Option<scene::VehicleRender>,
    trailers: Vec<scene::VehicleRender>,
    /// The bus's type (the livery studio reads its meshes and paint slots).
    vt: Option<Arc<omsi_sim::VehicleType>>,
    /// Centre and size of the bus (its bounding box), as the studio frames its photos.
    centre: glam::Vec3,
    length: f32,
    /// The whole bus's box (the rear section of an articulated one in it), as the showroom
    /// frames it.
    bus: BusBox,
    weather: omsi_content::weather::Weather,
    lighting: Lighting,
}

pub struct Showroom {
    shown: Option<Shown>,
    wanted: Option<Look>,
    /// The last look that could not be shown (not read again until something changes).
    failed: Option<Look>,
    /// The bus shown is read again even when the look stays (its files changed: a livery
    /// saved from the studio).
    reread: bool,
    loading: Option<(Look, Receiver<Result<Ready, String>>)>,
    /// The bus options to put on the bus (see `dress`), and the bus being made anew with
    /// them (the options it gets, the bus when it is made).
    options: Vec<(String, f32)>,
    dressing: Option<(Vec<(String, f32)>, Receiver<omsi_sim::VehicleInstance>)>,
    /// The display font to draw the bus's destination displays in (see `letter`).
    letters: Option<String>,
    pub error: Option<String>,
    /// Orbit: yaw and pitch (degrees) and distance factor, eased towards the targets.
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
    yaw_to: f32,
    pitch_to: f32,
    zoom_to: f32,
    pub auto_turn: bool,
    idle: f32,
    /// Where the bus stands in the picture (see `frame`): the part asked for, the part it is
    /// framed in now (eased towards it), and the seconds since it was last asked for.
    stage_to: Stage,
    stage_now: Stage,
    stage_idle: f32,
    /// The bus picker's photographer: the bus framed the old way, so that the photos taken
    /// before stay like the new ones.
    studio: bool,
    pub busy: bool,
    /// The picture: its texture and size, and whether it must be drawn again.
    target: Option<(wgpu::Texture, wgpu::TextureView, u32, u32)>,
    dirty: bool,
    /// Bumped whenever `target` was made anew (the interface binds it again).
    pub generation: u64,
    /// Where the camera aims: ahead of the bus's middle by this share of its length.
    aim: f32,
    /// The bus picker's photos, on the GPU with this showroom (see `busphoto`).
    pub photos: super::busphoto::Photos,
    /// A camera of the caller's own in place of the orbit (the livery studio's).
    camera: Option<Camera>,
    /// A second picture with a camera of its own, drawn with the first (the livery studio's
    /// other side while it mirrors), and its generation.
    second: Option<(wgpu::Texture, wgpu::TextureView, u32, u32)>,
    second_camera: Option<Camera>,
    pub second_generation: u64,
    /// The second picture's size, to be made at the next `preview`.
    pending_second: Option<(u32, u32)>,
}

/// The bus shown, opened up for the livery studio: its scene (whose texture slots it paints),
/// its type, the vehicle and its parts' renders.
pub struct Parts<'a> {
    pub scene: &'a mut Scene,
    pub vt: &'a Arc<omsi_sim::VehicleType>,
    pub vehicle: &'a omsi_sim::VehicleInstance,
    pub render: &'a scene::VehicleRender,
    pub trailers: &'a [scene::VehicleRender],
}

/// The studio's corner: in front of the bus on its door side, a little above it. The camera
/// aims ahead of the middle: seen from a front corner the near front looms larger than the
/// far rear, and aimed at the middle the bus's front ran off the photo's edge.
const PHOTO_YAW: f32 = 218.0;
const PHOTO_PITCH: f32 = 11.0;
const PHOTO_ZOOM: f32 = 0.8;
const PHOTO_AIM: f32 = 0.16;

/// Names the bus whose preview is being read and placed, until it is in the picture.
fn placing_mark() -> PathBuf {
    omsi_launcher_lib::data_dir().join("showroom-placing.txt")
}

fn args_for(look: &Look) -> Args {
    let mut v = vec!["omsi".to_string(), "--root".into(), look.root.to_string_lossy().to_string()];
    if !look.map.is_empty() {
        v.extend(["--map".into(), look.map.clone()]);
    }
    if !look.bus.is_empty() {
        v.extend(["--bus".into(), look.bus.clone()]);
    }
    if !look.paint.is_empty() {
        v.extend(["--paint".into(), look.paint.clone()]);
    }
    // (the current weather is fetched by the game, not by the preview)
    if !look.weather.is_empty() && !look.weather.starts_with("metar:") {
        v.extend(["--weather".into(), look.weather.clone()]);
    }
    v.extend(["--time".into(), format!("{:02}:{:02}", look.time / 60, look.time % 60)]);
    if !look.date.is_empty() {
        v.extend(["--date".into(), look.date.clone()]);
    }
    Args::try_parse_from(v).unwrap_or_else(|_| Args::parse_from(["omsi"]))
}

// (a launcher closed while a preview loads did not hang: the mark goes with it)
impl Drop for Showroom {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(placing_mark());
    }
}

impl Showroom {
    pub fn new() -> Showroom {
        Showroom {
            shown: None,
            wanted: None,
            failed: None,
            loading: None,
            options: Vec::new(),
            dressing: None,
            letters: None,
            error: None,
            yaw: 215.0,
            pitch: 8.0,
            zoom: 1.0,
            yaw_to: 215.0,
            pitch_to: 8.0,
            zoom_to: 1.0,
            auto_turn: false,
            idle: 0.0,
            stage_to: WHOLE,
            stage_now: WHOLE,
            stage_idle: f32::INFINITY,
            studio: false,
            busy: false,
            target: None,
            dirty: true,
            generation: 0,
            aim: 0.0,
            photos: Default::default(),
            reread: false,
            camera: None,
            second: None,
            second_camera: None,
            second_generation: 0,
            pending_second: None,
        }
    }

    /// The bus shown, its scene and type, for the livery studio (None while none is placed).
    pub fn parts(&mut self) -> Option<Parts<'_>> {
        let s = self.shown.as_mut()?;
        Some(Parts { scene: &mut s.scene, vt: s.vt.as_ref()?, vehicle: s.vehicle.as_ref()?, render: s.render.as_ref()?, trailers: &s.trailers })
    }

    /// Draw the bus with `cam` instead of the orbit (None: the orbit again); the picture is drawn
    /// again when it changed.
    pub fn set_camera(&mut self, cam: Option<Camera>) {
        let differs = match (&self.camera, &cam) {
            (Some(a), Some(b)) => a.position != b.position || a.yaw != b.yaw || a.pitch != b.pitch,
            (None, None) => false,
            _ => true,
        };
        if differs {
            self.camera = cam;
            self.dirty = true;
        }
    }

    /// A second picture of the bus with `cam` (None: none), `w` x `h` pixels, drawn whenever the
    /// first is: its view, made with the first's `preview`.
    pub fn set_second(&mut self, cam: Option<(Camera, u32, u32)>) {
        let Some((cam, w, h)) = cam else {
            self.second = None;
            self.second_camera = None;
            return;
        };
        let differs = self.second_camera.is_none_or(|a| a.position != cam.position || a.yaw != cam.yaw || a.pitch != cam.pitch);
        let (w, h) = (w.max(16), h.max(16));
        if self.second.as_ref().is_none_or(|t| (t.2, t.3) != (w, h)) {
            self.second = None;
            self.second_camera = None;
            self.pending_second = Some((w, h));
        }
        if differs {
            self.dirty = true;
        }
        self.second_camera = Some(cam);
    }

    /// The second picture (see `set_second`).
    pub fn second_view(&self) -> Option<(wgpu::TextureView, u32, u32)> {
        self.second.as_ref().map(|t| (t.1.clone(), t.2, t.3))
    }

    /// Draw the picture again (a texture of the bus changed).
    pub fn touch(&mut self) {
        self.dirty = true;
    }

    /// A showroom that takes the bus picker's photos: the bus in the middle of the picture,
    /// from the same corner every time, nothing eased or turned.
    pub fn studio() -> Showroom {
        let mut s = Showroom::new();
        (s.yaw, s.yaw_to, s.pitch, s.pitch_to, s.zoom, s.zoom_to) = (PHOTO_YAW, PHOTO_YAW, PHOTO_PITCH, PHOTO_PITCH, PHOTO_ZOOM, PHOTO_ZOOM);
        (s.studio, s.aim) = (true, PHOTO_AIM);
        s
    }

    /// The part of the picture the bus is to stand in (shares of it; see `stage_in`), asked
    /// for every frame the showroom is on screen. Away for a moment (the tiles were shown),
    /// the bus is put there at once; while shown it glides there (the window resized).
    pub fn set_stage(&mut self, stage: Stage) {
        if self.stage_idle > 0.3 && self.stage_now != stage {
            self.stage_now = stage;
            self.dirty = true;
        }
        self.stage_to = stage;
        self.stage_idle = 0.0;
    }

    /// The stage eased towards the one asked for; the picture is drawn again while it moves.
    fn ease_stage(&mut self, dt: f32) {
        self.stage_idle += dt;
        let k = 1.0 - (-dt / 0.35).exp();
        for (now, to) in self.stage_now.iter_mut().zip(self.stage_to) {
            if (to - *now).abs() > 1e-4 {
                *now += (to - *now) * k;
                self.dirty = true;
            } else {
                *now = to;
            }
        }
    }

    /// `look` is in place, ready to be drawn.
    pub fn shows(&self, look: &Look) -> bool {
        self.shown.as_ref().is_some_and(|s| &s.look == look && s.vehicle.is_some())
    }

    /// `look` will not be shown: the bus did not load, or the map did not open.
    pub fn gave_up(&self, look: &Look) -> bool {
        self.failed.as_ref() == Some(look) || (self.wanted.is_none() && self.loading.is_none() && !self.shows(look))
    }

    /// The files of bus `bus` changed (a livery saved into the game): when it is the bus
    /// shown or asked for, it is read again with whatever paint is chosen; the picture before
    /// stays until then.
    pub fn reread(&mut self, bus: &str) {
        let of_bus = |l: &Look| l.bus == bus;
        if self.wanted.as_ref().is_some_and(of_bus) || self.shown.as_ref().is_some_and(|s| of_bus(&s.look)) {
            self.reread = true;
            self.failed = None;
        }
    }

    /// Let go of the bus shown and its scene (the studio between two photos holds nothing).
    pub fn forget(&mut self) {
        self.shown = None;
        self.wanted = None;
        self.loading = None;
        self.target = None;
    }

    /// The bus shown, drawn at `w` x `h` pixels into a texture of its own and read back.
    pub fn photograph(&mut self, renderer: &mut Renderer, w: u32, h: u32) -> Option<image::RgbaImage> {
        self.shown.as_ref()?;
        let tex = renderer.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("bus photo"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: renderer.format(),
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = tex.create_view(&Default::default());
        self.render(renderer, &view, w, h, None);
        let stride = (w * 4).div_ceil(256) * 256;
        let buf = renderer.device.create_buffer(&wgpu::BufferDescriptor { label: Some("bus photo"), size: (stride * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let mut enc = renderer.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(tex.as_image_copy(), wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(stride), rows_per_image: None } }, wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 });
        renderer.queue.submit([enc.finish()]);
        buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        renderer.device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
        let data = buf.slice(..).get_mapped_range();
        let bgra = matches!(renderer.format(), wgpu::TextureFormat::Bgra8UnormSrgb | wgpu::TextureFormat::Bgra8Unorm);
        let mut img = image::RgbaImage::new(w, h);
        for (y, row) in img.rows_mut().enumerate() {
            let line = &data[y * stride as usize..];
            for (x, px) in row.enumerate() {
                let i = x * 4;
                *px = if bgra { image::Rgba([line[i + 2], line[i + 1], line[i], 255]) } else { image::Rgba([line[i], line[i + 1], line[i + 2], 255]) };
            }
        }
        Some(img)
    }

    /// Show this (a bus, its paint, the time and weather to light it with).
    pub fn want(&mut self, look: Look) {
        if self.wanted.as_ref() != Some(&look) {
            self.wanted = Some(look);
        }
    }

    /// Put these bus options (variable, value) on the bus, over its livery's: the bus shown is
    /// made anew with them, as the game makes it (see `made`) - its picture stays meanwhile.
    pub fn dress(&mut self, options: Vec<(String, f32)>) {
        self.options = options;
    }

    /// Draw the bus's destination displays in the display font `font` (None: its own), as the
    /// game will (`--display-font`): the bus shown takes it at once.
    pub fn letter(&mut self, font: Option<String>) {
        self.letters = font;
    }

    /// The bus options the bus shown wears (None: no bus shown).
    pub fn dressed(&self) -> Option<&[(String, f32)]> {
        self.shown.as_ref().map(|s| s.options.as_slice())
    }

    /// The mouse dragged over the empty part of the window (degrees), or turned the wheel.
    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw_to += dx * 0.35;
        self.pitch_to = (self.pitch_to + dy * 0.25).clamp(-4.0, 55.0);
        self.idle = 0.0;
    }
    pub fn zoom_by(&mut self, k: f32) {
        self.zoom_to = (self.zoom_to * k).clamp(0.55, 2.2);
        self.idle = 0.0;
    }

    /// Per frame: start loading what is wanted, take over what finished loading, move the
    /// camera. Returns true when a new scene was put in place.
    pub fn update(&mut self, renderer: &Renderer, dt: f32) -> bool {
        let mut swapped = false;
        // what finished loading
        if let Some((look, rx)) = self.loading.as_ref() {
            match rx.try_recv() {
                Ok(Ok(ready)) => {
                    self.loading = None;
                    self.shown = Some(self.place(renderer, ready));
                    self.error = None;
                    self.dirty = true;
                    swapped = true;
                }
                Ok(Err(e)) => {
                    let _ = std::fs::remove_file(placing_mark());
                    log::warn!("showroom {}: {e}", look.bus);
                    self.error = Some(e);
                    // (the bus before stayed in the picture as if it were the one chosen, and
                    // the failed one was read again every frame)
                    self.failed = Some(look.clone());
                    self.shown = None;
                    self.loading = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(_) => {
                    let _ = std::fs::remove_file(placing_mark());
                    self.failed = Some(look.clone());
                    self.loading = None;
                }
            }
        }
        // what is wanted and not shown or loading
        if let Some(w) = self.wanted.clone() {
            let shown = self.shown.as_ref().map(|s| &s.look);
            let loading = self.loading.as_ref().map(|l| &l.0);
            if (shown != Some(&w) || self.reread) && loading != Some(&w) && self.loading.is_none() && self.failed.as_ref() != Some(&w) {
                // only the light changed: no need to read the bus again
                let same_bus = !self.reread && shown.map(|s| s.bus == w.bus && s.paint == w.paint && s.map == w.map && s.root == w.root).unwrap_or(false);
                if same_bus {
                    if let Some(s) = self.shown.as_mut() {
                        s.look = w.clone();
                        let args = args_for(&w);
                        s.weather = load_weather(&args);
                        setup_sky(&args, renderer, &mut s.scene, omsi_content::Envir::load(&args.root.join("envir.cfg")).ok().as_ref(), Some(&s.weather));
                        s.lighting = lighting_for(&args, &s.weather);
                    }
                    self.dirty = true;
                } else {
                    self.start_loading(renderer, w);
                }
            }
        }
        // the bus options changed: the bus shown made anew with them (a bus or livery being
        // read is made with them from the start)
        if let Some((_, rx)) = self.dressing.as_ref() {
            match rx.try_recv() {
                Ok(vehicle) => {
                    if let Some((options, _)) = self.dressing.take() {
                        self.put_on(vehicle, options);
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(_) => {
                    // (it could not be made: not tried again for these options)
                    if let (Some((options, _)), Some(s)) = (self.dressing.take(), self.shown.as_mut()) {
                        s.options = options;
                    }
                }
            }
        }
        if self.dressing.is_none() && self.loading.is_none() {
            if let Some((s, v)) = self.shown.as_ref().filter(|s| s.options != self.options).and_then(|s| Some((s, s.vehicle.as_ref()?))) {
                let parts: Vec<(Arc<omsi_sim::VehicleType>, bool)> = v.trailers.iter().map(|t| (t.ty.clone(), t.reversed)).collect();
                let (vt, scheme, root, clock) = (v.ty.clone(), s.scheme, s.look.root.clone(), start_clock(&args_for(&s.look)));
                let options = self.options.clone();
                let wear = options.clone();
                let (tx, rx) = channel();
                std::thread::spawn(move || {
                    let _ = tx.send(made(&root, &vt, Some(&parts), scheme, clock, &wear));
                });
                self.dressing = Some((options, rx));
            }
        }
        // another display font: the bus shown writes its displays in it
        if self.dressing.is_none() {
            if let Some(s) = self.shown.as_mut().filter(|s| s.letters != self.letters) {
                if let (Some(w), Some(v)) = (s.world.as_ref(), s.vehicle.as_mut()) {
                    fonts_on(w, v, self.letters.as_deref());
                    self.dirty = true;
                }
                s.letters = self.letters.clone();
            }
        }
        self.busy = self.loading.is_some();
        // camera
        self.idle += dt;
        if self.auto_turn && self.idle > 4.0 {
            self.yaw_to += dt * 6.0;
        }
        let k = 1.0 - (-dt / 0.18).exp();
        // still turning: the picture must follow
        if (self.yaw_to - self.yaw).abs() > 0.05 || (self.pitch_to - self.pitch).abs() > 0.05 || (self.zoom_to - self.zoom).abs() > 0.001 {
            self.dirty = true;
        }
        self.yaw += (self.yaw_to - self.yaw) * k;
        self.pitch += (self.pitch_to - self.pitch) * k;
        self.zoom += (self.zoom_to - self.zoom) * k;
        // (the stage moving moves the bus: drawn again too, which it was not while it eased)
        self.ease_stage(dt);
        // the bus's own state, whenever the picture is drawn again
        if let (true, Some(s)) = (self.dirty, self.shown.as_mut()) {
            if let (Some(v), Some(r)) = (s.vehicle.as_mut(), s.render.as_mut()) {
                player::sync_vehicle_transforms(renderer, &mut s.scene, v, r, &mut s.trailers, false);
            }
        }
        swapped
    }

    fn start_loading(&mut self, renderer: &Renderer, look: Look) {
        self.reread = false;
        // A preview that never came (the launcher hung while placing it, and had to be
        // ended) is not tried again for that bus: the bus is remembered and the launcher hung
        // at every start, whatever version (#1478, #1635).
        // (`showroom-skip.txt` keeps such buses, a line each; delete it to try them again)
        let skip = omsi_launcher_lib::data_dir().join("showroom-skip.txt");
        if std::fs::read_to_string(placing_mark()).is_ok_and(|b| b.trim() == look.bus.trim()) {
            log::warn!("showroom: the preview of {} hung the launcher when it was last tried; it is left out from now on ({})", look.bus, skip.display());
            let _ = std::fs::remove_file(placing_mark());
            let mut list = std::fs::read_to_string(&skip).unwrap_or_default();
            list.push_str(look.bus.trim());
            list.push('\n');
            let _ = std::fs::write(&skip, list);
        }
        if std::fs::read_to_string(&skip).is_ok_and(|t| t.lines().any(|b| b.trim() == look.bus.trim())) {
            self.error = Some("The 3D preview of this bus stopped the launcher once, so it is left out".into());
            self.failed = Some(look);
            return;
        }
        let _ = std::fs::write(placing_mark(), &look.bus);
        let args = args_for(&look);
        let root = look.root.clone();
        let map_cfg = omsi_cfg::resolve_path(&root, &look.map);
        let date = start_clock(&args).date_code();
        let t0 = std::time::Instant::now();
        let world = match scene::World::open(&root, &map_cfg, date) {
            Ok(w) => {
                log::info!("showroom: {} opened in {:.2} s", map_cfg.display(), t0.elapsed().as_secs_f64());
                Arc::new(w)
            }
            Err(e) => {
                let _ = std::fs::remove_file(placing_mark());
                self.error = Some(format!("{e:#}"));
                self.wanted = None;
                return;
            }
        };
        let prefetch = world.vehicle_prefetch(renderer);
        let (tx, rx) = channel();
        let l2 = look.clone();
        let options = self.options.clone();
        std::thread::spawn(move || {
            let r = (|| -> Result<Ready> {
                if l2.bus.is_empty() {
                    return Err(anyhow!("no bus"));
                }
                let path = player_bus_path(&root, &l2.bus)?;
                let vt = Arc::new(omsi_sim::VehicleType::load(&root, &path)?);
                let scheme = paint_scheme(&vt, Some(l2.paint.as_str()).filter(|p| !p.is_empty()));
                let vehicle = made(&root, &vt, None, scheme, start_clock(&args), &options);
                prefetch.prefetch(&vt, scheme);
                for t in &vehicle.trailers {
                    prefetch.prefetch(&t.ty, crate::spawn::part_scheme(&vt, scheme, &t.ty));
                }
                Ok(Ready { look: l2, world, vt, vehicle, scheme, options })
            })();
            let _ = tx.send(r.map_err(|e| format!("{e:#}")));
        });
        self.loading = Some((look, rx));
    }

    fn place(&mut self, renderer: &Renderer, r: Ready) -> Shown {
        let t0 = std::time::Instant::now();
        let mut scene = renderer.new_scene();
        let args = args_for(&r.look);
        let weather = load_weather(&args);
        let envir = omsi_content::Envir::load(&args.root.join("envir.cfg")).ok();
        setup_sky(&args, renderer, &mut scene, envir.as_ref(), Some(&weather));
        add_floor(renderer, &mut scene);
        let world = r.world;
        let mut vehicle = r.vehicle;
        vehicle.position = DVec3::ZERO;
        vehicle.heading = 0.0;
        let render = world.add_vehicle(renderer, &mut scene, &r.vt, r.scheme);
        let trailers: Vec<scene::VehicleRender> = vehicle.trailers.iter().map(|t| world.add_vehicle_part(renderer, &mut scene, &t.ty, crate::spawn::part_scheme(&r.vt, r.scheme, &t.ty), &render)).collect();
        fonts_on(&world, &mut vehicle, self.letters.as_deref());
        vehicle.update(1.0 / 30.0);
        // the bus's size from its bounding box (with the rear section behind it)
        let bb = r.vt.def.bounding_box.unwrap_or([2.5, 12.0, 3.0, 0.0, 0.0, 1.5]);
        let mut length = bb[1];
        let mut centre = glam::Vec3::new(bb[3], bb[4], bb[5]);
        let mut bus = bus_box(r.vt.def.bounding_box, r.vt.model_box(), bb);
        for t in &vehicle.trailers {
            let tb = t.ty.def.bounding_box.unwrap_or([2.5, 8.0, 3.0, 0.0, 0.0, 1.5]);
            let behind = (t.position - vehicle.position).truncate().length() as f32;
            let back = behind + tb[1] * 0.5;
            let total = bb[1] * 0.5 + back;
            centre.y = bb[4] + bb[1] * 0.5 - total * 0.5;
            length = total;
            // (the section stands straight behind at rest)
            let mut rear = bus_box(t.ty.def.bounding_box, t.ty.model_box(), tb);
            rear.centre += Vec3::new(0.0, -behind, (t.position.z - vehicle.position.z) as f32);
            bus = bus.with(rear);
        }
        let lighting = lighting_for(&args, &weather);
        log::info!("showroom: {} ({} meshes, {:.1} m long) placed in {:.2} s", r.look.bus, render.instances.len(), length, t0.elapsed().as_secs_f64());
        let _ = std::fs::remove_file(placing_mark());
        Shown { look: r.look, scene, world: Some(world), vehicle: Some(vehicle), scheme: r.scheme, options: r.options, letters: self.letters.clone(), render: Some(render), trailers, vt: Some(r.vt.clone()), centre, length, bus, weather, lighting }
    }

    /// The bus made anew with the bus options `options` takes the place of the one shown (the
    /// same type: its meshes on the GPU stay; one made for a bus no longer shown is let go).
    fn put_on(&mut self, mut vehicle: omsi_sim::VehicleInstance, options: Vec<(String, f32)>) {
        let Some(s) = self.shown.as_mut() else { return };
        if !s.vehicle.as_ref().is_some_and(|v| Arc::ptr_eq(&v.ty, &vehicle.ty) && v.trailers.len() == vehicle.trailers.len()) {
            return;
        }
        vehicle.position = DVec3::ZERO;
        vehicle.heading = 0.0;
        if let Some(w) = s.world.as_ref() {
            fonts_on(w, &mut vehicle, s.letters.as_deref());
        }
        vehicle.update(1.0 / 30.0);
        log::info!("showroom: {} made anew with {} bus option(s)", s.look.bus, options.len());
        s.vehicle = Some(vehicle);
        s.options = options;
        self.dirty = true;
    }

    /// The picture of the bus at `w` x `h` pixels, drawn again when something changed.
    pub fn preview(&mut self, renderer: &mut Renderer, w: u32, h: u32) -> Option<wgpu::TextureView> {
        self.shown.as_ref()?;
        let (w, h) = (w.max(16), h.max(16));
        if self.target.as_ref().map(|t| (t.2, t.3) != (w, h)).unwrap_or(true) {
            let tex = renderer.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("bus preview"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: renderer.format(),
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = tex.create_view(&Default::default());
            self.target = Some((tex, view, w, h));
            self.generation += 1;
            self.dirty = true;
        }
        if let Some((sw, sh)) = self.pending_second.take() {
            let tex = renderer.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("bus preview, second"),
                size: wgpu::Extent3d { width: sw, height: sh, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: renderer.format(),
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = tex.create_view(&Default::default());
            self.second = Some((tex, view, sw, sh));
            self.second_generation += 1;
            self.dirty = true;
        }
        if self.dirty {
            self.dirty = false;
            let view = self.target.as_ref().unwrap().1.clone();
            self.render(renderer, &view, w, h, None);
            if let (Some((_, v2, sw, sh)), Some(cam)) = (self.second.as_ref().map(|t| (0, t.1.clone(), t.2, t.3)), self.second_camera) {
                self.render(renderer, &v2, sw, sh, Some(cam));
            }
        }
        self.target.as_ref().map(|t| t.1.clone())
    }

    fn render(&mut self, renderer: &mut Renderer, target: &wgpu::TextureView, w: u32, h: u32, with: Option<Camera>) {
        let (yaw, pitch, zoom, stage, aim, studio) = (self.yaw, self.pitch, self.zoom, self.stage_now, self.aim, self.studio);
        let Some(s) = self.shown.as_mut() else { return };
        let aspect = w as f32 / h.max(1) as f32;
        let cam = match with.or(self.camera) {
            Some(c) => c,
            None if studio => studio_camera(s.centre, s.length, yaw, pitch, zoom, aim, aspect),
            None => frame(&s.bus, yaw, pitch, zoom, aspect, stage),
        };
        s.scene.overlays.clear();
        let _ = &s.weather;
        renderer.render(&mut s.scene, target, w, h, &cam, &s.lighting);
    }

    /// A bus is there to show.
    pub fn has_picture(&self) -> bool {
        self.shown.as_ref().map(|s| s.vehicle.is_some()).unwrap_or(false) && self.target.is_some()
    }
}

/// The bus of type `vt` as the game puts it down (`spawn_player`): its livery's variables and
/// the bus options `options` there for its scripts' `{init}`, both set again after it,
/// its coupled parts on (`parts`, else read as the game reads them), three steps of the
/// scripts, then the options once more and a step - as `--setvar` comes - so that a script
/// taking them over in its first frames (the NLC's `setvar.osc`) sees them too.
fn made(root: &Path, vt: &Arc<omsi_sim::VehicleType>, parts: Option<&[(Arc<omsi_sim::VehicleType>, bool)]>, scheme: Option<usize>, clock: omsi_sim::SimClock, options: &[(String, f32)]) -> omsi_sim::VehicleInstance {
    let mut host = omsi_sim::VehicleHost::new(clock);
    host.paint_scheme = Some(scheme);
    host.start_vars = options.to_vec();
    let mut vehicle = omsi_sim::VehicleInstance::new(vt.clone(), host);
    vehicle.apply_paint_vars(scheme);
    for (var, v) in options {
        vehicle.set_var(var, *v);
    }
    match parts {
        Some(parts) => {
            for (t, reversed) in parts {
                vehicle.attach_trailer_ex(t.clone(), *reversed);
            }
        }
        None => {
            load_coupled_parts(root, &mut vehicle);
        }
    }
    for _ in 0..3 {
        vehicle.update(1.0 / 30.0);
    }
    if !options.is_empty() {
        for (var, v) in options {
            vehicle.set_var(var, *v);
        }
        vehicle.update(1.0 / 30.0);
    }
    vehicle
}

/// The fonts of the bus's text displays (its rear sections' too), its destination displays in
/// the display font `letters` the player chose (None: their own).
fn fonts_on(world: &scene::World, vehicle: &mut omsi_sim::VehicleInstance, letters: Option<&str>) {
    vehicle.init_text_textures(&mut world.fonts.lock(), &|p| omsi_texture::decode_file(p).ok().map(|i| (i.width, i.height, i.rgba)));
    for t in vehicle.trailers.iter_mut() {
        t.init_text_textures(&mut world.fonts.lock(), &|p| omsi_texture::decode_file(p).ok().map(|i| (i.width, i.height, i.rgba)));
    }
    if let Some(font) = letters.map(str::trim).filter(|f| !f.is_empty()) {
        vehicle.apply_display_font(font, &mut world.fonts.lock(), &|p| omsi_texture::decode_file(p).ok().map(|i| (i.width, i.height, i.rgba)));
    }
}

// --- framing the bus ------------------------------------------------------------------------
//
// The picture covers the whole window, under the sheet and the bar, but the bus is to stand in
// the part they leave free (the stage), in its middle and as large as it fits. The renderer's
// lens looks straight through the picture's middle, so the camera is turned past the bus
// instead, by as much as the stage's middle is off the picture's; and it stands as far off as
// the bus needs to fit the stage seen from its longest side, so that turning it round keeps
// its size.

/// The part of the picture the bus is framed in, as shares of the picture: left, top, right,
/// bottom (from the top down).
pub type Stage = [f32; 4];

/// The whole picture.
pub const WHOLE: Stage = [0.0, 0.0, 1.0, 1.0];

/// `stage`, a part of the window, as shares of `picture` (the part of the window the picture
/// covers): clipped to it, and never thinner than a twentieth of it.
pub fn stage_in(picture: omsi_ui::Rect, stage: omsi_ui::Rect) -> Stage {
    let (w, h) = (picture.w.max(1.0), picture.h.max(1.0));
    let x0 = ((stage.x - picture.x) / w).clamp(0.0, 0.95);
    let y0 = ((stage.y - picture.y) / h).clamp(0.0, 0.95);
    let x1 = ((stage.right() - picture.x) / w).clamp(x0 + 0.05, 1.0);
    let y1 = ((stage.bottom() - picture.y) / h).clamp(y0 + 0.05, 1.0);
    [x0, y0, x1, y1]
}

/// The bus's box in the showroom: its middle and half its size each way.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BusBox {
    pub centre: Vec3,
    pub half: Vec3,
}

impl BusBox {
    /// From OMSI's `[boundingbox]`: the sizes across, along and up, then the middle.
    pub fn of(bb: [f32; 6]) -> BusBox {
        BusBox { centre: Vec3::new(bb[3], bb[4], bb[5]), half: Vec3::new(bb[0], bb[1], bb[2]).abs() * 0.5 }
    }

    /// This box and `other` in one (an articulated bus: its front and its rear section).
    pub fn with(self, other: BusBox) -> BusBox {
        let lo = (self.centre - self.half).min(other.centre - other.half);
        let hi = (self.centre + self.half).max(other.centre + other.half);
        BusBox { centre: (lo + hi) * 0.5, half: (hi - lo) * 0.5 }
    }

    fn corners(&self) -> [Vec3; 8] {
        let s = |k: usize, bit: usize| if k & bit == 0 { -1.0 } else { 1.0 };
        std::array::from_fn(|k| self.centre + self.half * Vec3::new(s(k, 1), s(k, 2), s(k, 4)))
    }
}

/// How far past its `[boundingbox]` a bus's model may reach across and along and still be
/// framed: a bumper, not its mirrors (an O560's hang 0.55 m ahead of it, and framed with them
/// the body stood off the middle: they poke into the margin instead) nor the headlights' light
/// some models lay on the road ahead.
const REACH: Vec3 = Vec3::new(0.2, 0.3, 0.0);

/// The box a bus (or its rear section) takes in the showroom, from its `[boundingbox]` `bb` and
/// the box its model's vertices take (`VehicleType::model_box`): across and along the bounding
/// box's, or a little more where the model reaches past it, and up what the model reaches,
/// from just under the floor. The bounding box's height and middle are no guide to what is
/// seen - an O560's stood 0.6 m above the floor, an A20's ended 0.5 m under its roof.
/// `fallback` without either.
fn bus_box(bb: Option<[f32; 6]>, model: Option<(Vec3, Vec3)>, fallback: [f32; 6]) -> BusBox {
    let b = BusBox::of(bb.unwrap_or(fallback));
    let Some((mlo, mhi)) = model else { return b };
    let (blo, bhi) = (b.centre - b.half, b.centre + b.half);
    let (lo, hi) = if bb.is_some() { (mlo.min(blo).max(blo - REACH), mhi.max(bhi).min(bhi + REACH)) } else { (mlo, mhi) };
    let z0 = mlo.z.clamp(-0.3, 1.0);
    let z1 = mhi.z.clamp(z0 + 1.0, 6.0);
    let (lo, hi) = (Vec3::new(lo.x, lo.y, z0), Vec3::new(hi.x, hi.y, z1));
    BusBox { centre: (lo + hi) * 0.5, half: (hi - lo) * 0.5 }
}

/// The showroom's lens, its height in degrees, and its near plane.
const FOV: f32 = 30.0;
const NEAR: f32 = 0.2;
/// The share of the stage the bus takes each way, seen from where it takes the most (its long
/// side): the rest is room round it, for its shadow and the eye.
const FILL: f32 = 0.88;
/// The camera never sinks lower than this above the floor (metres).
const EYE_LOW: f32 = 0.6;

/// Half the lens' height at a distance of one.
fn half_v() -> f32 {
    (FOV.to_radians() * 0.5).tan()
}

/// Where the camera stands and where it looks: heading and pitch in degrees, as `Camera`'s.
#[derive(Clone, Copy, Debug)]
struct View {
    eye: Vec3,
    yaw: f32,
    pitch: f32,
}

impl View {
    /// Where `p` lands in a picture `aspect` wide (NDC: -1 to 1, y up); None behind the lens.
    fn project(&self, p: Vec3, aspect: f32) -> Option<Vec2> {
        let half_v = half_v();
        let (sy, cy) = self.yaw.to_radians().sin_cos();
        let (sp, cp) = self.pitch.to_radians().sin_cos();
        // ahead, right and up, as `Camera::view_proj` makes them
        let (f, s, u) = (Vec3::new(sy * cp, cy * cp, sp), Vec3::new(cy, -sy, 0.0), Vec3::new(-sy * sp, -cy * sp, cp));
        let q = p - self.eye;
        let z = q.dot(f);
        (z > NEAR).then(|| Vec2::new(q.dot(s) / (z * half_v * aspect), q.dot(u) / (z * half_v)))
    }

    /// The smallest rectangle (NDC) round the bus's corners; None when one is behind the lens.
    fn bounds(&self, bus: &BusBox, aspect: f32) -> Option<(Vec2, Vec2)> {
        let (mut lo, mut hi) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
        for c in bus.corners() {
            let p = self.project(c, aspect)?;
            (lo, hi) = (lo.min(p), hi.max(p));
        }
        Some((lo, hi))
    }
}

/// The camera's heading and pitch (degrees) for which a point straight along `w` (a unit
/// vector from the camera) lands at `at` (NDC) in a picture `aspect` wide: the camera turned
/// past the point by as much as `at` is off the middle, its horizon kept level.
fn aim(w: Vec3, at: Vec2, aspect: f32) -> (f32, f32) {
    let half_v = half_v();
    // the point's direction as the camera sees it: right, up and ahead
    let v = Vec3::new(at.x * half_v * aspect, at.y * half_v, 1.0).normalize();
    // its height in the world is that of the camera's up and ahead together:
    // w.z = v.y cos(pitch) + v.z sin(pitch)
    let r = (v.y * v.y + v.z * v.z).sqrt().max(1e-6);
    let pitch = (w.z / r).clamp(-1.0, 1.0).asin() - v.y.atan2(v.z);
    // and its heading the camera's, turned by its angle off the camera's level ahead
    let ahead = v.z * pitch.cos() - v.y * pitch.sin();
    let yaw = w.x.atan2(w.y) - v.x.atan2(ahead);
    (yaw.to_degrees(), pitch.to_degrees())
}

/// The camera's place orbiting the bus: `dist` metres from its middle, at heading `yaw` round
/// it, looking down by `pitch` degrees, and never under the floor.
fn eye(bus: &BusBox, yaw: f32, pitch: f32, dist: f32) -> Vec3 {
    let (sy, cy) = yaw.to_radians().sin_cos();
    let (sp, cp) = pitch.to_radians().sin_cos();
    let mut e = bus.centre - Vec3::new(sy * cp, cy * cp, -sp) * dist;
    e.z = e.z.max(EYE_LOW);
    e
}

/// The view from `eye` with the bus's middle at `at` (NDC).
fn looking(bus: &BusBox, eye: Vec3, at: Vec2, aspect: f32) -> View {
    let (yaw, pitch) = aim((bus.centre - eye).normalize_or(Vec3::Y), at, aspect);
    View { eye, yaw, pitch }
}

/// How far from its middle the camera must stand, looking down by `pitch`, for the bus's
/// outline to fit `room` (NDC) with its middle aimed at `centre`: the farthest of the headings
/// all round it, so that it keeps its distance - and its size - while it is turned.
fn fit_distance(bus: &BusBox, pitch: f32, aspect: f32, centre: Vec2, room: Vec2) -> f32 {
    let fits = |yaw: f32, d: f32| looking(bus, eye(bus, yaw, pitch, d), centre, aspect).bounds(bus, aspect).is_some_and(|(lo, hi)| hi.x - lo.x <= room.x && hi.y - lo.y <= room.y);
    let mut need = bus.half.length() + 1.0;
    for k in 0..36 {
        let yaw = k as f32 * 10.0;
        if fits(yaw, need) {
            continue;
        }
        let (mut lo, mut hi) = (need, need * 2.0);
        while !fits(yaw, hi) && hi < 20000.0 {
            (lo, hi) = (hi, hi * 2.0);
        }
        for _ in 0..24 {
            let mid = (lo + hi) * 0.5;
            if fits(yaw, mid) {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        need = hi;
    }
    need
}

/// The showroom's view: orbiting the bus at heading `yaw` round it and looking down by `pitch`
/// (degrees), `zoom` times as far off as it must be to fit `stage` whichever way it is turned,
/// and aimed so that the bus stands in the middle of the stage - its outline, not its middle
/// point, which the near end looming larger pushes off to the far side.
fn frame_view(bus: &BusBox, yaw: f32, pitch: f32, zoom: f32, aspect: f32, stage: Stage) -> View {
    let centre = Vec2::new(stage[0] + stage[2] - 1.0, 1.0 - stage[1] - stage[3]);
    let room = Vec2::new(stage[2] - stage[0], stage[3] - stage[1]).max(Vec2::splat(0.02)) * 2.0 * FILL;
    let dist = (fit_distance(bus, pitch, aspect, centre, room) * zoom).max(bus.half.length() + NEAR);
    let e = eye(bus, yaw, pitch, dist);
    let mut at = centre;
    let mut view = looking(bus, e, at, aspect);
    for _ in 0..4 {
        let Some((lo, hi)) = view.bounds(bus, aspect) else { break };
        let off = centre - (lo + hi) * 0.5;
        if off.length() < 1e-4 {
            break;
        }
        at = (at + off).clamp(Vec2::splat(-0.9), Vec2::splat(0.9));
        view = looking(bus, e, at, aspect);
    }
    view
}

/// The showroom's camera (see `frame_view`).
pub fn frame(bus: &BusBox, yaw: f32, pitch: f32, zoom: f32, aspect: f32, stage: Stage) -> Camera {
    let v = frame_view(bus, yaw, pitch, zoom, aspect, stage);
    Camera { position: v.eye.as_dvec3(), yaw: v.yaw, pitch: v.pitch, roll: 0.0, fov_deg: FOV, near: NEAR, far: 6000.0 }
}

/// The studio's camera for the bus picker's photos, as the showroom framed the bus before it
/// knew a stage (the photos taken then are kept): the whole picture, the bus's length alone
/// fitted to its width, aimed `aim` of its length ahead of its middle.
fn studio_camera(centre: Vec3, length: f32, yaw: f32, pitch: f32, zoom: f32, aim: f32, aspect: f32) -> Camera {
    let fit = (length * 0.5 + 1.0) / (half_v() * aspect * 0.92).max(0.1);
    let dist = (fit * zoom).max(8.0);
    let (sy, cy) = yaw.to_radians().sin_cos();
    let (sp, cp) = pitch.to_radians().sin_cos();
    let target = DVec3::new(centre.x as f64, (centre.y + aim * length) as f64, (centre.z * 0.75) as f64);
    // from the camera towards the bus: forward along the yaw, down by the pitch
    let dir = DVec3::new((sy * cp) as f64, (cy * cp) as f64, -sp as f64);
    let mut pos = target - dir * dist as f64;
    pos.z = pos.z.max(0.6);
    Camera { position: pos, yaw, pitch: -pitch, roll: 0.0, fov_deg: FOV, near: NEAR, far: 6000.0 }
}

/// The light of the look's time and weather, with the sun's shadow under the bus. Always
/// the plain renderer, whatever the game's graphics setting: a preview is to be quick and
/// clear, not the game's picture (no enhanced exposure and glow, no weather effects).
fn lighting_for(args: &Args, weather: &omsi_content::weather::Weather) -> Lighting {
    let clock = start_clock(args);
    let envir = omsi_content::Envir::load(&args.root.join("envir.cfg")).ok();
    let daylight = omsi_sim::Daylight::compute(&clock, envir.as_ref());
    let mut l = weather_lighting(&daylight, weather, crate::weather_setup::cloud_drift_at(weather, clock.time), 0.0, true);
    l.shadows = daylight.altitude_deg > 2.0;
    l.enhanced = false;
    l.classic = false;
    l.detail = false;
    l.wetness = 0.0;
    l.snow = 0.0;
    l.fog_density = 0.0;
    l
}

/// A round showroom floor under the bus: dark, matt, catching its shadow.
fn add_floor(renderer: &Renderer, scene: &mut Scene) {
    let n = 96;
    let r = 400.0f32;
    let mut positions = vec![glam::Vec3::ZERO];
    let mut normals = vec![glam::Vec3::Z];
    let mut uvs = vec![glam::Vec2::ZERO];
    for k in 0..n {
        let a = std::f32::consts::TAU * k as f32 / n as f32;
        positions.push(glam::Vec3::new(a.cos() * r, a.sin() * r, 0.0));
        normals.push(glam::Vec3::Z);
        uvs.push(glam::Vec2::new(a.cos(), a.sin()));
    }
    let mut indices = Vec::new();
    for k in 0..n {
        indices.extend([0u32, 1 + ((k + 1) % n) as u32, 1 + k as u32]);
    }
    let data = omsi_geometry::MeshData { positions, normals, uvs, ranges: vec![(0, indices.len() as u32, 0)], indices, one_sided: false };
    let mesh = renderer.add_mesh(scene, &data);
    let mat = renderer.add_material(scene, None, omsi_render::AlphaMode::Opaque, [0.12, 0.125, 0.135, 1.0], false);
    renderer.add_instance(scene, mesh, DVec3::new(0.0, 0.0, -0.005), glam::Mat4::IDENTITY, vec![mat]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use omsi_ui::Rect;

    /// A solo bus, an articulated one (front and rear section), a midibus and a double-decker.
    fn buses() -> Vec<(&'static str, BusBox)> {
        let solo = BusBox::of([2.55, 12.0, 3.1, 0.0, -1.2, 1.55]);
        let front = BusBox::of([2.55, 11.2, 3.1, 0.0, -1.0, 1.55]);
        let rear = BusBox::of([2.55, 7.6, 3.1, 0.0, -1.0 - 9.4, 1.55]);
        let midi = BusBox::of([2.3, 8.5, 2.9, 0.0, -0.8, 1.45]);
        let decker = BusBox::of([2.55, 11.2, 4.3, 0.0, -1.0, 2.15]);
        vec![("solo", solo), ("articulated", front.with(rear)), ("midi", midi), ("double-decker", decker)]
    }

    /// The window's pixel `p` (a point of the world) lands on, seen by `cam` in a window `size`
    /// big: the renderer's own projection, not the framing's.
    fn on_screen(cam: &Camera, size: Vec2, p: Vec3) -> Vec2 {
        let m = cam.view_proj(size.x / size.y, cam.position);
        let n = m.project_point3((p.as_dvec3() - cam.position).as_vec3());
        Vec2::new((n.x + 1.0) * 0.5 * size.x, (1.0 - n.y) * 0.5 * size.y)
    }

    /// The bus's outline in the window, in pixels: left top and right bottom.
    fn outline(cam: &Camera, size: Vec2, bus: &BusBox) -> (Vec2, Vec2) {
        let pts: Vec<Vec2> = bus.corners().iter().map(|c| on_screen(cam, size, *c)).collect();
        (pts.iter().fold(Vec2::splat(f32::INFINITY), |a, p| a.min(*p)), pts.iter().fold(Vec2::splat(f32::NEG_INFINITY), |a, p| a.max(*p)))
    }

    /// The bus step's stage in a window `size` big, as the launcher lays it out.
    fn stage_of(size: Vec2) -> Rect {
        super::super::buspick::stage(size, super::super::flow::sheet_rect(size))
    }

    #[test]
    fn the_camera_turns_past_a_point_to_put_it_where_asked() {
        for (yaw, pitch) in [(0.0f32, 0.0f32), (215.0, 8.0), (90.0, -4.0), (300.0, 55.0)] {
            let (sy, cy) = yaw.to_radians().sin_cos();
            let (sp, cp) = pitch.to_radians().sin_cos();
            let w = Vec3::new(sy * cp, cy * cp, -sp);
            for at in [Vec2::ZERO, Vec2::new(0.3, 0.0), Vec2::new(0.25, 0.08), Vec2::new(-0.6, 0.4), Vec2::new(0.8, -0.5)] {
                let (cyaw, cpitch) = aim(w, at, 1.8);
                let cam = Camera { position: DVec3::ZERO, yaw: cyaw, pitch: cpitch, roll: 0.0, fov_deg: FOV, near: NEAR, far: 6000.0 };
                let n = cam.view_proj(1.8, DVec3::ZERO).project_point3(w * 20.0);
                assert!((n.x - at.x).abs() < 1e-3 && (n.y - at.y).abs() < 1e-3, "{yaw}/{pitch} to {at:?}: {n:?}");
                // and the framing's own projection agrees with the renderer's
                let p = View { eye: Vec3::ZERO, yaw: cyaw, pitch: cpitch }.project(w * 20.0, 1.8).unwrap();
                assert!((p - at).length() < 1e-3, "{p:?}");
            }
        }
    }

    #[test]
    fn the_bus_stands_in_the_middle_of_the_free_part_and_fits_it() {
        for size in [Vec2::new(1440.0, 900.0), Vec2::new(2560.0, 1347.0), Vec2::new(1080.0, 680.0), Vec2::new(1920.0, 1080.0), Vec2::new(1280.0, 1024.0)] {
            let window = Rect::new(0.0, 0.0, size.x, size.y);
            let stage = stage_of(size);
            let shares = stage_in(window, stage);
            for (name, bus) in buses() {
                for yaw in [215.0f32, 90.0, 270.0, 0.0, 180.0, 135.0, 32.0] {
                    for pitch in [8.0f32, -4.0, 25.0, 55.0] {
                        let cam = frame(&bus, yaw, pitch, 1.0, size.x / size.y, shares);
                        let (lo, hi) = outline(&cam, size, &bus);
                        let what = format!("{name} at {yaw}/{pitch} in {size}: {lo} - {hi}, stage {stage:?}");
                        // all of it in the stage, nothing under the sheet or the actions
                        assert!(lo.x >= stage.x - 0.5 && hi.x <= stage.right() + 0.5 && lo.y >= stage.y - 0.5 && hi.y <= stage.bottom() + 0.5, "{what}");
                        // in the stage's middle
                        let mid = (lo + hi) * 0.5;
                        assert!((mid - stage.center()).length() < 2.0, "{what}: middle {mid}, the stage's {}", stage.center());
                    }
                }
                // seen from its long side it takes the stage's width or height, but for the margin
                let cam = frame(&bus, 270.0, 8.0, 1.0, size.x / size.y, shares);
                let (lo, hi) = outline(&cam, size, &bus);
                let fill = ((hi.x - lo.x) / stage.w).max((hi.y - lo.y) / stage.h);
                assert!(fill > 0.8 && fill <= FILL + 0.01, "{name} in {size}: fills {fill}");
                // from the default corner it is not lost in it either
                let cam = frame(&bus, 215.0, 8.0, 1.0, size.x / size.y, shares);
                let (lo, hi) = outline(&cam, size, &bus);
                assert!((hi.x - lo.x) / stage.w > 0.5 || (hi.y - lo.y) / stage.h > 0.5, "{name} in {size}: small from the corner");
            }
        }
    }

    #[test]
    fn turning_keeps_the_distance_and_zooming_keeps_the_middle() {
        let size = Vec2::new(1440.0, 900.0);
        let stage = stage_of(size);
        let shares = stage_in(Rect::new(0.0, 0.0, size.x, size.y), stage);
        let (_, bus) = buses().remove(1);
        let dist = |yaw: f32, zoom: f32| frame_view(&bus, yaw, 8.0, zoom, size.x / size.y, shares).eye.distance(bus.centre);
        let d = dist(215.0, 1.0);
        for yaw in [0.0, 47.0, 90.0, 180.0, 333.0] {
            assert!((dist(yaw, 1.0) - d).abs() < 1e-3 * d, "the bus keeps its size while turned ({yaw})");
        }
        assert!((dist(215.0, 0.7) - d * 0.7).abs() < 1e-3 * d);
        // closer in, larger, and still about the stage's middle
        let near = frame(&bus, 215.0, 8.0, 0.7, size.x / size.y, shares);
        let far = frame(&bus, 215.0, 8.0, 1.0, size.x / size.y, shares);
        let ((a, b), (c, e)) = (outline(&near, size, &bus), outline(&far, size, &bus));
        assert!(b.x - a.x > (e.x - c.x) * 1.2);
        assert!((((a + b) * 0.5) - stage.center()).length() < 2.0);
    }

    #[test]
    fn the_stage_is_what_the_sheet_and_the_actions_leave() {
        use super::super::flow::{ACTION_BOTTOM, ACTION_H, SHEET_TOP};
        for size in [Vec2::new(1440.0, 900.0), Vec2::new(2560.0, 1347.0), Vec2::new(1080.0, 680.0)] {
            let sheet = super::super::flow::sheet_rect(size);
            let s = stage_of(size);
            assert!(s.x > sheet.right() && s.right() < size.x && s.y >= SHEET_TOP, "{size}: {s:?}");
            // above the duty's line over the main action
            assert!(s.bottom() < size.y - ACTION_BOTTOM - ACTION_H - 32.0, "{size}: {s:?}");
            assert!(s.w > 400.0 && s.h > 300.0, "{size}: {s:?}");
            let shares = stage_in(Rect::new(0.0, 0.0, size.x, size.y), s);
            assert!((shares[0] * size.x - s.x).abs() < 1e-3 && (shares[3] * size.y - s.bottom()).abs() < 1e-3);
        }
        // (a stage off the picture is clipped to it, and kept a sliver at the least)
        assert_eq!(stage_in(Rect::new(0.0, 0.0, 100.0, 100.0), Rect::new(-20.0, 50.0, 300.0, 0.0)), [0.0, 0.5, 1.0, 0.55]);
    }

    #[test]
    fn an_articulated_bus_is_framed_with_its_rear_section() {
        let front = BusBox::of([2.55, 11.0, 3.1, 0.0, 0.0, 1.55]);
        let rear = BusBox::of([2.55, 7.0, 3.1, 0.0, -9.5, 1.55]);
        let both = front.with(rear);
        assert!((both.half.y * 2.0 - 18.5).abs() < 1e-4 && (both.centre.y - (5.5 - 9.25)).abs() < 1e-4, "{both:?}");
        assert_eq!(both.half.x, 2.55 * 0.5);
    }

    #[test]
    fn the_stage_is_taken_at_once_after_a_while_away_and_glided_to_while_shown() {
        let mut s = Showroom::new();
        let (a, b) = ([0.3, 0.1, 0.98, 0.8], [0.25, 0.1, 0.98, 0.85]);
        s.set_stage(a);
        assert_eq!(s.stage_now, a, "the first time: at once");
        s.dirty = false;
        s.ease_stage(1.0 / 60.0);
        assert!(!s.dirty, "standing still: nothing to draw again");
        s.set_stage(b);
        assert_eq!(s.stage_now, a, "shown: it glides");
        s.ease_stage(1.0 / 60.0);
        assert!(s.dirty && s.stage_now[0] < a[0] && s.stage_now[0] > b[0]);
        for _ in 0..240 {
            s.set_stage(b);
            s.ease_stage(1.0 / 60.0);
        }
        assert_eq!(s.stage_now, b);
        // the tiles were shown a while: back in the showroom, the bus stands where it should
        s.ease_stage(0.5);
        s.set_stage(a);
        assert_eq!(s.stage_now, a);
    }

    #[test]
    fn the_box_is_the_model_s_within_reason() {
        // (an O560 coach, its numbers as installed: the bounding box 0.6 m above the floor, the
        // mirrors past its front and sides, the rear bumper a little past its back)
        let bb = [2.46, 12.08, 3.3, 0.0, 0.22, 2.28];
        let b = bus_box(Some(bb), Some((Vec3::new(-1.48, -5.85, 0.0026), Vec3::new(1.46, 6.80, 3.44))), bb);
        let (lo, hi) = (b.centre - b.half, b.centre + b.half);
        assert!((lo.z - 0.0026).abs() < 1e-4 && (hi.z - 3.44).abs() < 1e-4, "on the floor, as high as the model: {b:?}");
        assert!((lo.y + 5.85).abs() < 1e-4, "the bumper in it: {b:?}");
        assert!((hi.y - (0.22 + 6.04 + 0.3)).abs() < 1e-4 && (lo.x + 1.23 + 0.2).abs() < 1e-4, "the mirrors not all: {b:?}");
        // a model with its headlights' light on the road ahead, and a helper far under it: a
        // little past the bounding box at the most, and from just under the floor
        let b = bus_box(Some(bb), Some((Vec3::new(-1.48, -5.85, -2.15), Vec3::new(1.46, 16.0, 3.44))), bb);
        let (lo, hi) = (b.centre - b.half, b.centre + b.half);
        assert!((hi.y - (0.22 + 6.04 + 0.3)).abs() < 1e-4 && (lo.z + 0.3).abs() < 1e-4, "{b:?}");
        // no model: the bounding box; no bounding box: the model
        assert_eq!(bus_box(Some(bb), None, bb), BusBox::of(bb));
        let b = bus_box(None, Some((Vec3::new(-1.2, -5.0, 0.0), Vec3::new(1.2, 6.0, 3.0))), [2.5, 12.0, 3.0, 0.0, 0.0, 1.5]);
        assert!((b.half.y - 5.5).abs() < 1e-4 && (b.centre.y - 0.5).abs() < 1e-4);
    }

    #[test]
    fn the_studio_frames_its_photos_as_before() {
        // (the formula the photos taken so far were framed with)
        let (centre, length, aspect) = (Vec3::new(0.0, -1.0, 1.5), 12.0f32, 560.0 / 315.0);
        let cam = studio_camera(centre, length, PHOTO_YAW, PHOTO_PITCH, PHOTO_ZOOM, PHOTO_AIM, aspect);
        let half_v = (30.0f32.to_radians() * 0.5).tan();
        let dist = ((length * 0.5 + 1.0) / (half_v * aspect * 0.92) * PHOTO_ZOOM).max(8.0);
        let (sy, cy) = PHOTO_YAW.to_radians().sin_cos();
        let (sp, cp) = PHOTO_PITCH.to_radians().sin_cos();
        let target = DVec3::new(0.0, (-1.0 + PHOTO_AIM * length) as f64, 1.5 * 0.75);
        let pos = target - DVec3::new((sy * cp) as f64, (cy * cp) as f64, -sp as f64) * dist as f64;
        assert!((cam.position - pos).length() < 1e-4 && cam.yaw == PHOTO_YAW && cam.pitch == -PHOTO_PITCH && cam.fov_deg == 30.0);
    }
}
