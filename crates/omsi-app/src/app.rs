//! The window's game: `App`, its state and its per-frame work.

use super::*;

mod groups;
pub(crate) use groups::*;

const SLOW_UPLOAD_MB_S: f64 = 300.0;

pub(crate) struct App {
    pub(crate) args: Args,
    /// What draws the picture besides the renderer and the scene (see `GfxState`).
    pub(crate) gfx: GfxState,
    pub(crate) window: Option<Arc<Window>>,
    pub(crate) renderer: Option<Renderer>,
    /// The VR headset and what goes with it (see `VrState`).
    pub(crate) xr: VrState,
    pub(crate) scene: Option<Scene>,
    pub(crate) camera: Option<Camera>,
    pub(crate) player: Option<Player>,
    /// The simulated world around the player's bus (see `SessionState`).
    pub(crate) session: SessionState,
    /// The menus, lists, editor and on-screen panels (see `MenuState`).
    pub(crate) menus: MenuState,
    pub(crate) world: Option<Arc<World>>,
    /// Where the camera looks from and to, per view (see `ViewState`).
    pub(crate) cam: ViewState,
    /// Chat, mouse-over names and name tags (Roboto).
    pub(crate) ui: Option<ui::Ui>,
    /// Frame timing, profiling and the test hooks (see `PerfState`).
    pub(crate) perf: PerfState,
    /// What the game sounds out: the engine, the world around, the radio, the voice chat.
    pub(crate) sound: SoundState,
    pub(crate) clock: omsi_sim::SimClock,
    pub(crate) started: Instant,
    pub(crate) view: String,
    /// The keyboard, the mouse, controllers, head tracking and touch (see `InputState`).
    pub(crate) input: InputState,
    pub(crate) last: Instant,
    /// The simulation stands still (OMSI's `sim_pause`, P, or the menu): nothing moves,
    /// the clock stops, the picture and the camera go on.
    pub(crate) paused: bool,
    /// Plugins and the services outside the game (see `Integrations`).
    pub(crate) integrations: Integrations,
    /// The multiplayer session and the other players (see `NetState`).
    pub(crate) net: NetState,
    /// Last workshop / fuel pump / wash message, and how long it still shows.
    pub(crate) service_msg: Option<(String, f32)>,
    pub(crate) settings: settings::Settings,
    /// The exit is under way.
    pub(crate) exiting: bool,
}

impl App {
    #[cfg(windows)]
    pub(crate) fn vr_active(&self) -> bool { self.xr.vr.is_some() }

    #[cfg(not(windows))]
    pub(crate) fn vr_active(&self) -> bool { false }

    pub(crate) fn resumed_impl(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(window) = self.window.clone() {
            // back from the background (a phone): the window's surface is made again
            if self.gfx.surface.is_none() {
                if let Some(r) = self.renderer.as_ref() {
                    let size = window.inner_size();
                    let vsync = self.settings.vsync && !self.vr_active();
                    self.gfx.surface = SurfaceState::new_with(&self.gfx.instance, window.clone(), r, size.width.max(1), size.height.max(1), vsync).ok();
                    self.last = Instant::now();
                }
            }
            return;
        }
        self.create_window(event_loop, None);
    }

    /// The game's window (or the launcher's, handed over on a phone), its surface and the
    /// renderer; then the menu or, when the session is given, the world.
    pub(crate) fn create_window(&mut self, event_loop: &ActiveEventLoop, given: Option<Arc<Window>>) {
        // --size sets the window's size in points as well (1600x900 unless given)
        let (lw, lh) = self
            .args
            .size
            .split_once('x')
            .map(|(a, b)| {
                (
                    a.parse::<u32>().unwrap_or(1600),
                    b.parse::<u32>().unwrap_or(900),
                )
            })
            .unwrap_or((1600, 900));
        let (fit, at) = crate::startup::fit_window(event_loop, lw as f64, lh as f64);
        let mut attrs = Window::default_attributes()
            .with_title("openOMSI")
            .with_inner_size(fit)
            .with_window_icon(crate::startup::window_icon());
        if let Some(at) = at {
            attrs = attrs.with_position(at);
        }
        // the window size of the settings (pixels, #904) unless --size names one
        let resolution = crate::settings::Settings::resolution().filter(|_| self.args.size == crate::cli::DEFAULT_SIZE);
        if let Some((w, h)) = resolution {
            attrs = attrs.with_inner_size(winit::dpi::PhysicalSize::new(w, h));
            if let Some(m) = crate::startup::home_monitor(event_loop) {
                let (sw, sh) = (m.size().width as i32, m.size().height as i32);
                attrs = attrs.with_position(winit::dpi::PhysicalPosition::new(m.position().x + ((sw - w as i32) / 2).max(0), m.position().y + ((sh - h as i32) * 2 / 5).max(0)));
            }
        }
        // (a Steam Deck's Gaming Mode: gamescope shows one window over the whole screen and
        // scales whatever size it has to it - a window fitted to 90 % of the screen came out
        // blurred and letterboxed. There the window is the screen's, unless a size is set)
        let gamescope = resolution.is_none() && crate::startup::under_gamescope();
        if gamescope {
            log::info!("gamescope (Steam Deck Gaming Mode): the window fills the screen");
        }
        if self.settings.fullscreen || gamescope {
            attrs = attrs.with_fullscreen(Some(winit::window::Fullscreen::Borderless(crate::startup::home_monitor(event_loop))));
        }
        if self.settings.triple.enabled
            && self.settings.triple_span
            && !self.settings.vr_requested()
        {
            let mut monitors: Vec<_> = event_loop.available_monitors().collect();
            monitors.sort_by_key(|m| m.position().x);
            let row = monitors.windows(3).find(|row| {
                let size = row[0].size();
                row.iter()
                    .all(|m| m.size() == size && m.position().y == row[0].position().y)
                    && row[1].position().x == row[0].position().x + size.width as i32
                    && row[2].position().x == row[1].position().x + size.width as i32
            });
            if let Some(row) = row {
                let size = row[0].size();
                if self.settings.fullscreen || gamescope || resolution.is_some() || self.args.size != crate::cli::DEFAULT_SIZE {
                    log::info!("triple screen: spanning three monitors instead of the fullscreen / window size settings");
                }
                self.gfx.spanned = true;
                attrs = attrs
                    .with_fullscreen(None)
                    .with_decorations(false)
                    .with_position(row[0].position())
                    .with_inner_size(winit::dpi::PhysicalSize::new(size.width * 3, size.height));
                log::info!("triple screen: spanning {}x{}", size.width * 3, size.height);
            } else {
                log::warn!("triple screen: no equal horizontal monitor row found; using configured window size (Surround/Eyefinity can expose one wide display)");
            }
        }
        // OMSI_BACKGROUND=1: a test window that does not take the keyboard from whoever is
        // working at the screen (OMSI_INPUT drives the handlers directly, it needs no focus)
        if omsi_cfg::flags::OMSI_BACKGROUND.is_set() || omsi_cfg::flags::OMSI_HIDDEN_WINDOW.is_set() {
            attrs = attrs.with_active(false);
        }
        // OMSI_HIDDEN_WINDOW=1: the window is never shown at all, so a benchmark of the
        // window's own frame (its steps, OMSI_PROFILE's stages) can run beside whoever works
        // at the screen; macOS then reports it occluded and the frames are drawn into a
        // texture of its size (see `frame_acquire`)
        if omsi_cfg::flags::OMSI_HIDDEN_WINDOW.is_set() {
            attrs = attrs.with_visible(false);
        }
        let window = match given {
            Some(w) => w,
            // (no display to open it on, a compositor that refuses it: said so, not a panic
            // report about "window")
            None => match event_loop.create_window(attrs) {
                Ok(w) => Arc::new(w),
                Err(e) => {
                    fatal_message(&format!("The game cannot open its window: {e}"));
                    crate::platform::exit(event_loop);
                    return;
                }
            },
        };
        let mut renderer = match window_renderer(&mut self.gfx.instance, &window, self.settings.render_options()) {
            Ok(r) => r,
            Err(e) => {
                fatal_message(&format!("The game cannot draw on this computer: {e:#}"));
                crate::platform::exit(event_loop);
                return;
            }
        };
        #[cfg(windows)]
        if self.settings.vr_requested() {
            match crate::openxr::Vr::new(&renderer, self.settings.vr_scale, self.settings.vr_desktop_mirror) {
                Ok(vr) => self.xr.vr = Some(vr),
                Err(e) => log::error!("OpenXR could not start: {e:#}"),
            }
        }
        let upload = renderer.upload_speed_mb_s();
        log::info!("graphics: {upload:.0} MB/s copied towards the card");
        if upload < SLOW_UPLOAD_MB_S {
            log::error!("graphics: the driver copies only {upload:.0} MB/s towards the card (thousands are usual); every texture and buffer the game sends waits on it, down to a few frames a second - restarting the computer usually brings it back");
            self.service_msg = Some((format!("Graphics driver is slow ({upload:.0} MB/s): the game will stutter. Restarting the computer usually fixes it."), 30.0));
        }
        crate::lights::load_smoke_texture(&mut renderer, &self.args.root);
        crate::lights::set_corona_root(&self.args.root);
        let size = window.inner_size();
        let surface = match SurfaceState::new_with(
            &self.gfx.instance,
            window.clone(),
            &renderer,
            size.width,
            size.height,
            self.settings.vsync && !self.vr_active(),
        ) {
            Ok(s) => s,
            Err(e) => {
                fatal_message(&format!("The game's window cannot be drawn into: {e:#}"));
                crate::platform::exit(event_loop);
                return;
            }
        };
        let (sw, sh) = renderer.scene_size(size.width, size.height);
        log::info!(
            "window: {}x{} pixels (scale factor {:.2}), 3D picture {sw}x{sh}, present mode {:?}",
            size.width,
            size.height,
            window.scale_factor(),
            surface.config.present_mode
        );
        let scene = renderer.new_scene();
        self.window = Some(window);
        self.gfx.surface = Some(surface);
        self.renderer = Some(renderer);
        self.scene = Some(scene);
        // fully specified runs skip the menu
        if self.args.bus.is_some() || self.args.cam.is_some() || self.args.no_menu {
            self.load_world_now(event_loop);
        } else {
            let mut fonts = omsi_sim::texttex::FontLibrary::new(&self.args.root);
            self.menus.hud = Some(hud::Hud::new(&mut fonts));
            self.menus.menu = Some(menu::Menu::new(&self.args.root, &self.args.map));
        }
    }

    /// Load the map, vehicle and traffic according to `args`.
    pub(crate) fn load_world_now(&mut self, event_loop: &ActiveEventLoop) {
        // a joining player loads the host's date, time, weather and season (after the menu)
        if let Some(l) = self.net.lan.as_mut() {
            lan::adopt_host_world(&mut self.args, l, &mut self.net.remotes);
        }
        let renderer = self.renderer.take().expect("renderer");
        let mut scene = renderer.new_scene();
        self.session.envir = omsi_content::Envir::load(&self.args.root.join("envir.cfg")).ok();
        // the weather cycle: a first weather that suits the month, the others after it
        if crate::weather_cycle::is_cycle(self.args.weather.as_deref()) {
            let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(7);
            let mut c = crate::weather_cycle::Cycle::new(seed);
            let month = start_clock(&self.args).day_month().1;
            let all = crate::weather_cycle::installed();
            let clear = omsi_content::weather::Weather { fog: (50000.0, 1.0), ..Default::default() };
            let r = c.rand();
            self.args.weather = crate::weather_cycle::pick(&all, &clear, "", month, r);
            log::info!("weather cycle: starting with {:?}", self.args.weather);
            self.session.weather_cycle = Some(c);
        }
        self.session.weather = Some(load_weather(&self.args));
        // the roads start in the state this weather has already left them in, as they do
        // offscreen: a session begun in the rain used to open on a bone-dry street
        self.session.wetness = self.session.weather.as_ref().map(initial_wetness).unwrap_or(0.0);
        self.clock = start_clock(&self.args);
        setup_sky(&self.args, &renderer, &mut scene, self.session.envir.as_ref(), self.session.weather.as_ref());
        // the window streams the tiles around the camera unless a fixed area was asked for
        if !self.args.all && self.args.radius.is_none() {
            match open_world(&self.args) {
                Ok((w, cam, _)) => {
                    let w = Arc::new(w);
                    let distance = self
                        .args
                        .view_distance
                        .or_else(settings::view_distance)
                        .unwrap_or(1200.0)
                        .max(omsi_map::tile_size());
                    w.set_fast_texture_loads(true);
                    w.set_texture_budget(texture_budget(&self.settings));
                    log::info!(
                        "texture budget: {:.0} MB",
                        texture_budget(&self.settings) as f64 / 1e6
                    );
                    self.gfx.streamer = Some(tiles::Streamer::new(
                        w.clone(),
                        &start_centers(&self.args, &cam, Some(&w)),
                        distance,
                        700.0,
                    ));
                    self.world = Some(w);
                    self.cam.starting = Some(cam);
                }
                Err(e) => {
                    log::error!("{e:#}");
                    crate::platform::exit(event_loop);
                }
            }
            self.renderer = Some(renderer);
            self.scene = Some(scene);
            self.last = Instant::now();
            return;
        }
        match load_world(&self.args, &renderer, &mut scene) {
            Ok((w, cam)) => self.start_world(Arc::new(w), cam, &renderer, &mut scene),
            Err(e) => {
                log::error!("{e:#}");
                crate::platform::exit(event_loop);
            }
        }
        self.renderer = Some(renderer);
        self.scene = Some(scene);
        self.last = Instant::now();
    }

    /// Everything that needs the map to stand: the player's bus, the passengers, the traffic
    /// and the timetable.
    pub(crate) fn start_world(
        &mut self,
        w: Arc<World>,
        cam: Camera,
        renderer: &Renderer,
        mut scene: &mut Scene,
    ) {
        report_missing_content(&w, &mut self.service_msg);
        {
            {
                // (once more when it fails: a file read while the start was still reading
                // others; a failure is said on the screen - the game went on without a bus
                // and the player found himself on foot, with no word why)
                let first = spawn_player(&self.args, &w, &renderer, &mut scene);
                let spawned = match first {
                    Err(e) if self.args.bus.is_some() => {
                        log::warn!("the bus could not be put down ({e:#}); trying again");
                        spawn_player(&self.args, &w, &renderer, &mut scene).map_err(|e2| {
                            self.service_msg = Some((format!("The bus could not be loaded: {}", format!("{e2:#}").lines().next().unwrap_or_default()), 15.0));
                            e2
                        })
                    }
                    other => other,
                };
                match spawned {
                    Ok(mut p) => {
                        let audio = omsi_audio::AudioEngine::new();
                        if let Some(p) = p.as_mut() {
                            p.vehicle.host.auto_clutch = if self.settings.auto_clutch { 1.0 } else { 0.0 };
                            p.load_sounds(&audio);
                            p.ibis_background = true;
                            // --autostart applies in the window too, not only offscreen
                            if self.args.autostart && !self.args.is_resuming() {
                                let msg = p.start_up();
                                self.service_msg = Some((msg, 6.0));
                            }
                            // a pack this bus borrows parts from is not installed: say so
                            // once, it explains dark displays and missing devices
                            if !p.vehicle.ty.missing_packs.is_empty() {
                                let packs: Vec<String> = p.vehicle.ty.missing_packs.iter().map(|(n, _)| n.clone()).collect();
                                let msg = format!(
                                    "This bus takes parts from vehicle pack(s) that are not installed: {} (install them for its displays and devices)",
                                    packs.join(", ")
                                );
                                self.service_msg = Some(match self.service_msg.take() {
                                    Some((m, _)) => (format!("{m}   |   {msg}"), 12.0),
                                    None => (msg, 12.0),
                                });
                            }
                        }
                        self.sound.ambience = Some(ambience::Ambience::load(&audio, &self.args.root));
                        self.sound.audio = Some(audio);
                        if let Some(p) = &p {
                            if self.args.cam.is_none() && self.args.view != "free" {
                                self.camera = Some(p.camera(&self.args.view, &cam));
                            }
                        }
                        self.player = p;
                    }
                    Err(e) => log::error!("{e:#}"),
                }
                // a situation's further vehicles, each as it was saved
                for o in self.args.situation_others.clone() {
                    let one = Args {
                        bus: Some(o.bus.clone()),
                        spawn: Some(o.spawn.clone()),
                        hof: o.hof.clone(),
                        paint: o.paint.clone(),
                        situation_vars: o.vars.clone(),
                        situation_strvars: o.strvars.clone(),
                        situation_odometer_km: o.odometer_km,
                        situation_others: Vec::new(),
                        line: None,
                        tour: None,
                        trip: None,
                        autostart: false,
                        situation_next_stop: None,
                        ..self.args.clone()
                    };
                    match spawn_player(&one, &w, &renderer, &mut scene) {
                        Ok(Some(q)) => {
                            log::info!("situation: {} placed at {}", o.bus, o.spawn);
                            self.session.placed.push(q);
                        }
                        Ok(None) => {}
                        Err(e) => log::warn!("situation vehicle {}: {e:#}", o.bus),
                    }
                }
                if self.camera.is_none() {
                    self.camera = Some(cam);
                }
                self.menus.navigator = Some(navigator::Navigator::new(
                    self.settings.navigator,
                    self.settings.ui_opacity,
                    &self.settings.navigator_corner,
                ));
                if let Some(n) = self.menus.navigator.as_mut() {
                    n.arrows = self.settings.nav_arrows;
                    n.show_ai = self.settings.nav_ai;
                }
                if let Some(d) = self.args.driver.as_deref() {
                    self.session.career = career::Career::load(&self.args.root, d);
                }
                // (and a player who joins another's game sees the host's people)
                if self.args.passengers || self.args.lan_join.is_some() {
                    let mut h = humans::Humans::new(&self.args.root, &mut self.gfx.sim_view.people);
                    if let Some(lan) = self.net.lan.as_ref() {
                        h.set_lan_seed(lan::population_seed(lan));
                    }
                    h.exact_fare = self.settings.exact_fare;
                    h.boarding = self.settings.boarding.clone();
                    h.voices = match self.settings.pax_voices.as_str() { "off" => 2, "tickets" => 1, _ => 0 };
                    if let Some(p) = self.player.as_mut() {
                        h.set_cabin(&mut p.vehicle);
                        h.ticket_key = ticket_key_name(&self.args.root, &p.bindings);
                        h.tickets = p.vehicle.host.tickets.clone();
                        if !w.global.money_system.trim().is_empty() {
                            h.money =
                                Some(money::Money::new(&self.args.root, &w.global.money_system));
                        }
                    }
                    if let Some(p) = self.player.as_ref() {
                        if self.args.riders > 0 {
                            let centre = p.vehicle.position;
                            h.populate(&mut self.gfx.sim_view.people, &w, &renderer, &mut scene, centre);
                            h.seed_riders(&mut self.gfx.sim_view.people, self.args.riders, &p.vehicle, &w, &renderer, &mut scene);
                        }
                    }
                    self.session.humans = Some(h);
                }
                // (a player who joins draws the host's traffic in it, whatever their own count
                // says: the host's cars had nowhere to go without it)
                let populated = self.args.traffic > 0 || self.args.schedule || crate::rail_drive::args_rail(&self.args) || self.args.lan_join.is_some();
                // Without traffic it still runs the light programs and switches the lamps:
                // they stood frozen with red, yellow and green all lit (#727).
                {
                    match traffic::Traffic::new(&self.args.root, &w, self.args.traffic) {
                        Ok(mut t) => {
                            t.lights_only = !populated;
                            t.no_timetable_buses = self.args.no_timetable_buses;
                            if let Some(lan) = self.net.lan.as_ref() {
                                t.set_lan_seed(lan::population_seed(lan));
                            }
                            if self.args.traffic > 0 {
                                t.precache_random(&w, &renderer, &mut scene);
                            }
                            t.day_time = parse_time(&self.args.time);
                            self.gfx.sim_view.traffic = Default::default();
                            self.session.traffic = Some(t);
                        }
                        Err(e) => log::error!("traffic: {e:#}"),
                    }
                    if self.args.schedule {
                        let mut sch =
                            schedule::Schedule::new(&self.args.root, &w, &start_clock(&self.args));
                        sch.precache(
                            &w,
                            &renderer,
                            &mut scene,
                            self.session.traffic.as_mut(),
                            parse_time(&self.args.time),
                        );
                        if let (Some(line), Some(p)) = (&self.args.line, self.player.as_mut()) {
                            self.session.duty = match sch.player_duty(
                                &w,
                                line,
                                self.args.tour.as_deref().unwrap_or(""),
                                parse_time(&self.args.time),
                                self.args.trip.as_deref(),
                                self.args.whole_tour,
                            ) {
                                Ok(mut d) => {
                                    if let Some(k) = self.args.duty_trip {
                                        d.start_at(k, self.args.duty_first_stop);
                                    }
                                    if self.args.is_resuming() {
                                        d.resume(&mut p.vehicle, parse_time(&self.args.time), self.args.situation_next_stop);
                                        // the IBIS keeps its saved trip; with --autostart
                                        // the duty's next trips are typed into it again
                                        p.duty_typed = self.args.autostart;
                                    }
                                    Some(d)
                                }
                                Err(e) => {
                                    log::warn!("no player duty: {e}");
                                    self.service_msg = Some((format!("No duty: {e}"), 20.0));
                                    None
                                }
                            };
                            // --autostart in the window puts the duty on the IBIS as well
                            // (it only ever did offscreen: the duty did not exist yet when
                            // the start-up began, and the displays stayed dark)
                            if let (true, Some(d)) = (
                                self.args.autostart && !self.args.is_resuming(),
                                self.session.duty.as_mut(),
                            ) {
                                d.update(&mut p.vehicle, parse_time(&self.args.time));
                                let (trip, stop) = d.trip_for_ibis();
                                p.set_duty_destination(trip, stop);
                            }
                            if let Some(d) = self.session.duty.as_ref() {
                                let mut fonts = w.fonts.lock();
                                if let Err(e) = crate::schedule_paper::update_vehicle(
                                    &mut p.vehicle,
                                    d,
                                    &mut fonts,
                                ) {
                                    log::warn!("driver timetable paper: {e:#}");
                                }
                            }
                        }
                        self.session.schedule = Some(sch);
                    }
                }
                if self.session.traffic.is_none() {
                    if let Some(n) = self.menus.navigator.as_mut() {
                        n.add_lanes(std::mem::take(&mut *w.lanes.lock()));
                    }
                }
                if let (Some(lan), Some(p)) = (self.net.lan.as_mut(), self.player.as_mut()) {
                    lan::settle_spawn(
                        lan,
                        &mut self.net.remotes,
                        p,
                        &self.args,
                        &w,
                        self.session.traffic.as_ref().map(|t| &t.net),
                        // (still loading: a moment's wait for the host's list puts the bus
                        // where it is free at once - without it the bus stood inside another
                        // player's for the first frames and then jumped out of it)
                        std::time::Duration::from_millis(1500),
                    );
                }
                self.world = Some(w);
            }
        }
        self.last = Instant::now();
    }

    /// The first area of a streamed map is loading: show how far it got, and start the rest
    /// of the world once it is there. Returns false while the loading screen is up.
    pub(crate) fn drive_start(&mut self, event_loop: &ActiveEventLoop) -> bool {
        let Some(cam) = self.cam.starting.take() else {
            return true;
        };
        let (Some(renderer), Some(mut scene)) = (self.renderer.take(), self.scene.take()) else {
            self.cam.starting = Some(cam);
            return true;
        };
        let centers = start_centers(&self.args, &cam, self.world.as_deref());
        let progress = match self.gfx.streamer.as_mut() {
            Some(streamer) => {
                streamer.update(
                    &renderer,
                    &mut scene,
                    &centers,
                    std::time::Duration::from_millis(30),
                    None,
                );
                streamer.initial_progress()
            }
            None => None,
        };
        let Some((done, total)) = progress else {
            let w = self.world.clone().expect("world");
            log::info!(
                "map ready after {:.2} s: {} tiles loaded",
                self.started.elapsed().as_secs_f64(),
                w.loaded_tiles().len()
            );
            self.start_world(w, cam, &renderer, &mut scene);
            self.renderer = Some(renderer);
            self.scene = Some(scene);
            return true;
        };
        let name = self
            .world
            .as_ref()
            .map(|w| {
                if w.global.friendly_name.trim().is_empty() {
                    w.global.name.clone()
                } else {
                    w.global.friendly_name.clone()
                }
            })
            .unwrap_or_default();
        let mut reconfigure = false;
        if let (Some(ui), Some(s), Some(win)) = (
            self.ui.as_mut(),
            self.gfx.surface.as_ref(),
            self.window.as_ref(),
        ) {
            scene.overlays.clear();
            let dpi = win.scale_factor() as f32;
            let scale = dpi * crate::ui::size_factor(s.config.height as f32, dpi, self.settings.ui_scale, self.settings.ui_scale_window);
            ui.loading(
                &renderer,
                &mut scene,
                s.config.width as f32,
                s.config.height as f32,
                scale,
                name.trim(),
                "",
                done as f32 / total.max(1) as f32,
            );
            let acquired = s.surface.get_current_texture();
            // a swapchain that no longer fits the window (Vulkan says so after the switch
            // to full screen, without a resize event) is made again, as the game's own
            // frames do: left as it was, every later frame of the loading screen failed
            // the same way and its picture stood still until the map was there (#776)
            reconfigure = matches!(acquired, wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost);
            if let wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) = acquired
            {
                let view = frame.texture.create_view(&Default::default());
                // the tiles loaded so far stay out of the picture: the camera looks at nothing
                let blank = Camera {
                    position: DVec3::new(0.0, 0.0, -1.0e6),
                    yaw: 0.0,
                    pitch: -89.0,
                    roll: 0.0,
                    fov_deg: 60.0,
                    near: 0.5,
                    far: 10.0,
                };
                let lighting = omsi_render::Lighting {
                    sky_color: glam::Vec3::new(0.08, 0.10, 0.14),
                    ..Default::default()
                };
                let mut renderer = renderer;
                renderer.render(
                    &mut scene,
                    &view,
                    s.config.width,
                    s.config.height,
                    &blank,
                    &lighting,
                );
                win.pre_present_notify();
                frame.present();
                self.renderer = Some(renderer);
            } else {
                self.renderer = Some(renderer);
            }
            win.request_redraw();
        } else {
            self.renderer = Some(renderer);
        }
        if reconfigure {
            if let (Some(s), Some(r), Some(win)) = (self.gfx.surface.as_mut(), self.renderer.as_ref(), self.window.as_ref()) {
                let size = win.inner_size();
                s.resize(r, size.width, size.height);
            }
        }
        self.scene = Some(scene);
        self.cam.starting = Some(cam);
        if let Some(limit) = self.args.exit_after {
            if self.started.elapsed().as_secs_f32() > limit {
                log::info!("exit after {limit} s while loading: {done} of {total} tiles");
                crate::platform::exit(event_loop);
            }
        }
        false
    }

    /// Stream the tiles around the camera and the player's bus (whose ground must stay when
    /// the free camera flies off); when the loaded set changed, hand the new data to whatever
    /// keeps its own copy (the bus's obstacles, the traffic network, the navigator, the
    /// street lamps).
    pub(crate) fn drive_streaming(&mut self) {
        let mut centers: Vec<DVec3> = self.camera.iter().map(|c| c.position).collect();
        centers.extend(self.player.iter().map(|p| p.vehicle.position));
        // a LAN host simulates the world around every player: the ground and the roads there
        if self.net.lan.as_ref().map(|l| l.role == omsi_net::Role::Host).unwrap_or(false) {
            centers.extend(self.net.remotes.remotes.values().map(|r| r.vehicle().position));
        }
        let (Some(streamer), Some(w), Some(r), Some(scene)) = (
            self.gfx.streamer.as_mut(),
            self.world.as_ref(),
            self.renderer.as_ref(),
            self.scene.as_mut(),
        ) else {
            return;
        };
        // textures uploaded as RGBA to spare a frame, compressed on the workers since, and the
        // texture budget
        w.apply_texture_upgrades(
            r,
            scene,
            Some(Instant::now() + std::time::Duration::from_millis(2)),
        );
        w.update_texture_budget(r, scene, &centers, false);
        if centers.is_empty() {
            return;
        }
        let changed = streamer.update(
            r,
            scene,
            &centers,
            std::time::Duration::from_millis(6),
            self.sound.audio.as_ref(),
        );
        // Uploads can temporarily exceed the texture budget before the next frame's
        // housekeeping pass. Recheck immediately after streaming so far textures are
        // reduced before the renderer allocates more frame resources.
        if !changed {
            return;
        }
        w.update_texture_budget(r, scene, &centers, true);
        if let Some(p) = self.player.as_mut() {
            // (OMSI's [no_collision]: no solid object stops the bus)
            p.vehicle.collision = self.settings.collision_objects.then(|| w.collision.lock().clone());
            p.vehicle.wheel_walls = self.settings.collision_objects;
        }
        match self.session.traffic.as_mut() {
            Some(t) => {
                t.add_tiles(w);
            }
            None => {
                // no traffic system: the navigator keeps the roads for its map
                let lanes = std::mem::take(&mut *w.lanes.lock());
                if let Some(n) = self.menus.navigator.as_mut() {
                    n.add_lanes(lanes);
                }
            }
        }
        if let Some(on) = self.session.lamps_on {
            w.set_lamps(r, scene, on);
        }
    }
}

/// Where a streamed map loads around while it starts: the vehicle's spawn point (a bus must
/// stand on ground when it appears, whatever `--cam` says), and the camera unless the view
/// follows the bus (every view but the free camera does, `--cam` or not).
pub(crate) fn start_centers(args: &Args, cam: &Camera, world: Option<&World>) -> Vec<DVec3> {
    let mut out: Vec<DVec3> = spawn_point(args, world).into_iter().collect();
    let view_in_bus = args.bus.is_some() && args.view != "free";
    if out.is_empty() || !view_in_bus {
        out.push(cam.position);
    }
    out
}

/// What the map needs and this installation lacks, said on the screen and written to
/// `~/.openomsi/missing_content.txt` by add-on folder: a map short of an add-on showed
/// holes, bare roads and white objects, and nobody could tell that from a fault of the game.
pub(crate) fn report_missing_content(w: &World, msg: &mut Option<(String, f32)>) {
    let (files, textures) = w.missing_content();
    if files.is_empty() && textures.is_empty() {
        return;
    }
    // the add-on a file comes with: the folder under Sceneryobjects / Splines / Vehicles
    let addon = |f: &str| f.split('/').take(2).collect::<Vec<_>>().join("/");
    let mut by_addon: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for (f, what) in &files {
        by_addon.entry(addon(f)).or_default().push(format!("{what}: {f}"));
    }
    let mut text = format!("openOMSI: content this map uses that is not installed\nmap: {}\n\n", w.map_dir.display());
    for (a, list) in &by_addon {
        text.push_str(&format!("{a} ({} files)\n", list.len()));
        for l in list {
            text.push_str(&format!("  {l}\n"));
        }
    }
    if !textures.is_empty() {
        text.push_str(&format!("\ntextures not found ({}):\n", textures.len()));
        for t in &textures {
            text.push_str(&format!("  {t}\n"));
        }
    }
    let Some(dir) = crate::lan::data_dir() else { return };
    let path = dir.join("missing_content.txt");
    let _ = std::fs::write(&path, text);
    let objects = files.iter().filter(|(_, w)| *w != "spline").count();
    let splines = files.len() - objects;
    let addons: Vec<&String> = by_addon.keys().take(4).collect();
    let more = if by_addon.len() > 4 { format!(" and {} more", by_addon.len() - 4) } else { String::new() };
    log::warn!("missing content: {objects} objects, {splines} splines, {} textures (list: {})", textures.len(), path.display());
    if !files.is_empty() {
        *msg = Some((
            format!(
                "This map uses {objects} objects and {splines} splines that are not installed (add-ons: {}{more}). The list is in {}",
                addons.iter().map(|a| a.as_str()).collect::<Vec<_>>().join(", "),
                path.display()
            ),
            15.0,
        ));
    }
}

/// How long the glide between two cockpit cameras takes (seconds). The eye, the turn of the
/// view and the field of view all follow the same curve over this time. 0 = hard cut.
pub(crate) const CAM_BLEND_SECS: f32 = 0.54;
/// The longest step of time one frame adds to the glide (seconds): a frame that hitches at
/// the start of a switch does not skip ahead in it.
pub(crate) const CAM_BLEND_MAX_DT: f32 = 1.0 / 30.0;

fn wrap_deg(a: f32) -> f32 {
    (a + 180.0).rem_euclid(360.0) - 180.0
}

/// `a` (k = 0) to `b` (k = 1), both cameras fixed in the bus's frame: the eye, the turn of the
/// view and the field of view on a straight way. The bus's own motion (its pitch, bank, the
/// head) is put on the result afterwards, so the glide is the same standing and driving.
pub(crate) fn blend_local(a: &omsi_vehicle::Camera, b: &omsi_vehicle::Camera, k: f32) -> omsi_vehicle::Camera {
    // (measured from `b`: at k = 1 every value is exactly `b`'s - no 360 degree residue of a
    // yaw that went the short way round, no rounding left over for the hand-over to the
    // plain camera to show)
    let k = k.clamp(0.0, 1.0);
    if k >= 1.0 {
        return b.clone();
    }
    let rest = 1.0 - k;
    let l = |x: f32, y: f32| y + (x - y) * rest;
    // The view direction turns along the great circle between the two (a slerp of the
    // directions), not yaw and pitch each on their own straight line: that swept the view
    // out in a bow - up and across at once - while the eye went straight, which looked like
    // a zigzag in the glide.
    let dir = |c: &omsi_vehicle::Camera| {
        let (sy, cy) = c.yaw.to_radians().sin_cos();
        let (sp, cp) = c.pitch.to_radians().sin_cos();
        glam::Vec3::new(sy * cp, cy * cp, sp)
    };
    let (fa, fb) = (dir(a), dir(b));
    let dot = fa.dot(fb).clamp(-1.0, 1.0);
    let (yaw, pitch) = if dot < -0.9995 {
        // (turned right round: no one great circle, so the plain way)
        (b.yaw - wrap_deg(b.yaw - a.yaw) * rest, l(a.pitch, b.pitch))
    } else {
        let f = if dot > 0.9995 {
            (fa * rest + fb * k).normalize_or(fb)
        } else {
            let theta = dot.acos();
            let s = theta.sin();
            ((fa * ((rest * theta).sin() / s)) + (fb * ((k * theta).sin() / s))).normalize_or(fb)
        };
        (f.x.atan2(f.y).to_degrees(), f.z.clamp(-1.0, 1.0).asin().to_degrees())
    };
    omsi_vehicle::Camera {
        pos: [l(a.pos[0], b.pos[0]), l(a.pos[1], b.pos[1]), l(a.pos[2], b.pos[2])],
        dist: l(a.dist, b.dist),
        fov: l(a.fov, b.fov),
        yaw,
        pitch,
        extra: b.extra,
        fixed: b.fixed,
    }
}

/// A bus whose mirrors are frozen, and the seconds since they were first drawn.
pub(crate) struct FrozenMirrors {
    pub(crate) bus: u64,
    pub(crate) since: f32,
}

#[derive(Default)]
pub(crate) struct CamBlend {
    /// (view, camera numbers) of the last frame: a change of the numbers inside the same
    /// view is a camera switch.
    pub key: Option<(String, (usize, usize))>,
    /// The camera the glide started from (in the bus's frame), while one is under way.
    pub from: Option<omsi_vehicle::Camera>,
    /// The cockpit camera as it was drawn last frame (in the bus's frame): where the next
    /// glide starts from.
    pub shown: Option<omsi_vehicle::Camera>,
    /// Just sat down at the wheel (from on foot): the next cockpit frame glides in from where
    /// the walker's eyes were.
    pub entering: bool,
    /// 0..1 progress of the glide.
    pub t: f32,
    /// What the glide's last picture was off from the plain camera by, let go of over a
    /// fraction of a second after the hand-over (so that nothing is left to jump).
    pub carry: Option<CamCarry>,
}

/// A small difference between two pictures of the camera (world position, angles in degrees,
/// field of view) that is eased out instead of being cut.
#[derive(Clone, Copy)]
pub(crate) struct CamCarry {
    pub pos: glam::DVec3,
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    pub fov: f32,
}

impl CamCarry {
    /// `a` minus `b`.
    pub fn between(a: &omsi_render::Camera, b: &omsi_render::Camera) -> Self {
        Self {
            pos: a.position - b.position,
            yaw: wrap_deg(a.yaw - b.yaw),
            pitch: a.pitch - b.pitch,
            roll: wrap_deg(a.roll - b.roll),
            fov: a.fov_deg - b.fov_deg,
        }
    }

    /// Put on a camera.
    pub fn apply(&self, c: &mut omsi_render::Camera) {
        c.position += self.pos;
        c.yaw += self.yaw;
        c.pitch = (c.pitch + self.pitch).clamp(-89.0, 89.0);
        c.roll += self.roll;
        c.fov_deg += self.fov;
    }

    /// Ease out by one frame; false once nothing is left.
    pub fn decay(&mut self, dt: f32) -> bool {
        let k = (-dt.clamp(0.0, 0.1) * 12.0).exp();
        self.pos *= k as f64;
        self.yaw *= k;
        self.pitch *= k;
        self.roll *= k;
        self.fov *= k;
        self.pos.length() > 1e-4 || self.yaw.abs() > 0.01 || self.pitch.abs() > 0.01 || self.roll.abs() > 0.01 || self.fov.abs() > 0.01
    }
}

impl CamBlend {
    /// How far along the way from the old camera to the new one: ease-out
    /// `s = 1-(1-t)^3` — fast off the mark, settling softly, so adjacent
    /// seats snap round without lagging behind the key.
    pub fn progress(&self) -> f32 {
        let t = self.t.clamp(0.0, 1.0);
        1.0 - (1.0 - t) * (1.0 - t) * (1.0 - t)
    }
}

#[cfg(test)]
mod cam_blend_tests {
    use super::CamBlend;

    fn blend(t: f32) -> f32 {
        CamBlend { key: None, from: None, shown: None, entering: false, t, carry: None }.progress()
    }

    #[test]
    fn glide_starts_fast_and_settles_softly() {
        assert_eq!(blend(0.0), 0.0);
        assert_eq!(blend(1.0), 1.0);
        assert!((blend(0.5) - 0.875).abs() < 1e-6);
        assert!(blend(0.2) > 0.4, "fast off the mark");
    }
}
