//! The groups of `App`'s state, one per part of the game (see `App`).

use super::*;

/// The game's sound: the audio engine and what plays through it.
pub(crate) struct SoundState {
    /// The player's bus radio as internet radio.
    pub(crate) radio: radio::Radio,
    pub(crate) audio: Option<omsi_audio::AudioEngine>,
    /// Sounds of the world around the camera (rain, footsteps).
    pub(crate) ambience: Option<ambience::Ambience>,
    /// Positional voice through GreenTeaSpeak in a session (`voice`).
    pub(crate) voice: Option<crate::voice::Voice>,
}

/// The VR headset: its session (Windows), the navigator shown in it, the cockpit pointer
/// and the picture zoom.
pub(crate) struct VrState {
    #[cfg(windows)]
    pub(crate) vr: Option<crate::openxr::Vr>,
    pub(crate) vr_nav_profiles: crate::vr_navigator::Profiles,
    pub(crate) vr_nav_edit: Option<crate::vr_navigator::Editing>,
    /// Last Windows mouse position used for the unbounded VR cockpit pointer.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub(crate) vr_cursor_physical: Option<(f32, f32)>,
    #[cfg_attr(not(windows), allow(dead_code))]
    pub(crate) vr_cursor_warp_pending: Option<(f32, f32)>,
    /// Right mouse button toggles the headset picture zoom.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub(crate) vr_zoom_active: bool,
}

/// The multiplayer session: the LAN or server connection and the other players seen in it.
pub(crate) struct NetState {
    /// Other players on foot whose avatars are drawn (their ids).
    pub(crate) remote_walkers: Vec<u32>,
    /// The player on foot is in this other player's bus (see `lan`: drawn from inside).
    pub(crate) inside_remote: Option<u32>,
    /// A dedicated server said we administer it (`admin`).
    pub(crate) is_admin: bool,
    /// LAN session, and the other players' buses (drawn and heard like AI vehicles) with the
    /// chat line.
    pub(crate) lan: Option<omsi_net::LanSession>,
    pub(crate) remotes: lan::LanGame,
}

/// What the game talks to besides itself: the OMSI and Lua plugins, Discord, Steam, the
/// website's "playing now" and the look for a newer release.
pub(crate) struct Integrations {
    /// Keys pressed (true) and let go since the Lua plugins' last frame.
    pub(crate) plugin_keys: Vec<(String, bool)>,
    /// What happened since the Lua plugins' last frame: crashes, people knocked down,
    /// stops skipped (see `plugins::queue_event`).
    pub(crate) plugin_events: Vec<omsi_plugin::GameEvent>,
    /// The Lua plugins' panels and notifications on the screen (`omsi.ui`).
    pub(crate) plugin_panels: crate::plugin_ui::PluginPanels,
    /// Discord's "Playing openOMSI" status, and when it was last brought up to date.
    pub(crate) discord: Option<crate::discord::Discord>,
    pub(crate) discord_t: f32,
    // Steamworks API layer and it's last updated time
    #[cfg(steam)]
    pub(crate) steam: Option<crate::steam::Steam>,
    /// The look for a newer release during the session (cards over the navigator).
    pub(crate) update_watch: crate::update_watch::UpdateWatch,
    /// "Playing now" on the website (None: not counted, setting `presence`).
    pub(crate) presence: Option<crate::presence::Presence>,
    /// The OMSI plugins (`plugins/*.opl`), loaded with the first frame.
    pub(crate) plugins: Option<omsi_plugin::Plugins>,
}

/// How the frames go and what is measured or scripted about them: the frame rate, the
/// profile, the stutters, the frame-rate governor, the `OMSI_INPUT` script, screenshots and
/// the log.
pub(crate) struct PerfState {
    pub(crate) fps: f32,
    /// Per-stage frame time accumulators (OMSI_PROFILE), seconds.
    pub(crate) profile: std::collections::BTreeMap<&'static str, f64>,
    /// `profile` as it was at the start of the last frame: what a slow frame spent where.
    pub(crate) profile_prev: std::collections::BTreeMap<&'static str, f64>,
    pub(crate) total_frames: u32,
    /// `OMSI_INPUT` script: (seconds after start, command), in order.
    pub(crate) input_script: Vec<(f32, String)>,
    /// A pending screenshot: its output path and whether touch controls are composited over it.
    /// Scripted `shot <file>` captures keep the controls for visual tests; player screenshots
    /// leave them out so the camera button produces a clean image.
    pub(crate) shot: Option<(PathBuf, bool)>,
    pub(crate) frames: u32,
    pub(crate) fps_t: Instant,
    /// What the log has said (see applog.rs).
    pub(crate) log_state: crate::applog::LogState,
    /// Frames longer than 50 ms (stutters) and the worst frame, for the exit summary.
    pub(crate) spikes: u32,
    pub(crate) worst_ms: f32,
    /// The frame-rate governor's two-second window.
    /// Window seconds, frames, and time waiting on presentation/GPU in that window.
    pub(crate) governor: (f32, u32, f32),
    /// Readings in a row at the smallest render scale still waiting for the card.
    pub(crate) governor_low: u32,
    /// Cumulative presentation wait at the previous frame, independent of OMSI_PROFILE.
    pub(crate) governor_wait_prev: f64,
    /// OMSI_PROFILE: process CPU seconds, time and frame count once the start-up is over,
    /// for the CPU time a frame costs (the wall time says little on a busy machine).
    pub(crate) cpu_mark: Option<(f64, Instant, u32)>,
    /// OMSI_PROFILE: the stages when `cpu_mark` was taken, and every frame's time since
    /// then (s), for the exit summary's percentiles (see `perf_report`).
    pub(crate) profile_mark: Option<crate::perf_report::ProfileMark>,
    pub(crate) frame_times: Vec<f32>,
}

/// The drawing around the renderer: the wgpu instance and surface, the tile streaming, the
/// bus mirrors, the window's visibility and what stands in for it, and the view sync's
/// state.
pub(crate) struct GfxState {
    pub(crate) instance: wgpu::Instance,
    pub(crate) surface: Option<SurfaceState<'static>>,
    /// Tile streaming around the camera (the window's default).
    pub(crate) streamer: Option<tiles::Streamer>,
    /// The window spans the triple screen's three monitors: fullscreen would shrink it to one.
    pub(crate) spanned: bool,
    /// Mirror pictures due (see `MIRROR_RATE`), and which mirror is next.
    pub(crate) mirror_budget: f32,
    pub(crate) mirrors_seen: usize,
    pub(crate) mirror_turn: usize,
    /// With no real-time reflections: the bus whose mirrors are frozen (see
    /// `MIRROR_FREEZE_REDRAW`).
    pub(crate) frozen_mirrors: Option<FrozenMirrors>,
    /// The mirror panels laid over the picture (see `mirror_hud`).
    pub(crate) mirror_hud: crate::mirror_hud::MirrorHud,
    /// The window is minimised or out of sight, as its events last said.
    pub(crate) window_hidden: bool,
    /// OMSI 2's route arrows over the road (the `nav_arrows` setting).
    pub(crate) route_arrows: crate::route_arrows::RouteArrows,
    /// Frames the window was hidden for (they are not drawn).
    pub(crate) hidden_frames: u32,
    /// Stand-in for the window's frame while the window is hidden (OMSI_RENDER_OCCLUDED).
    pub(crate) stand_in: Option<wgpu::Texture>,
    /// What the renderer shows of the AI traffic and the people (`view_sync`): their
    /// renders, kept apart from `session.traffic` and `session.humans`.
    pub(crate) sim_view: crate::view_sync::SimView,
}

/// The camera's state besides the camera itself: the head turned and zoomed per view, the
/// switch between cameras, the outside camera's distance, the free camera's speed and the
/// pedestrian view.
pub(crate) struct ViewState {
    /// The map is open but the first area is still loading: the view to start with.
    pub(crate) starting: Option<Camera>,
    pub(crate) speed: f32,
    /// The idle head sway waiting where it is while the cursor is on a control
    /// (see `head_idle::Hold`).
    pub(crate) head_idle_hold: crate::head_idle::Hold,
    /// OMSI's pedestrian ("ego") view: the free camera walking at eye height on whatever
    /// people stand on (`view_set_ego`, F11).
    pub(crate) ego: bool,
    /// The camera is in the own bus's cab this frame (see RedrawRequested).
    pub(crate) in_cab: bool,
    /// How far the player has turned the head (driver, passenger) or swung the outside
    /// camera around the bus, and how far that camera sits from it.
    pub(crate) look: (f32, f32),
    /// Where the view is drawn between that angle and the one of the frame before: the way
    /// the mouse (or the stick, or the keys) went is eased in, so the head glides to the
    /// angle asked for rather than jumping to it (`look_smoothing_ms`; 0 keeps it equal to
    /// `look`). Only the camera reads this - everything that turns the view writes `look`.
    pub(crate) look_smooth: (f32, f32),
    /// Each view keeps its own `look` (as OMSI's cameras do): turning the outside camera
    /// (F3) leaves the driver's head (F1) where it was. `look_view` is the view `look`
    /// belongs to now; see `App::sync_view_look`.
    pub(crate) view_looks: std::collections::HashMap<String, (f32, f32)>,
    pub(crate) look_view: String,
    /// Smooth switch between two cockpit cameras (arrow keys), see `CamBlend`.
    pub(crate) cam_blend: CamBlend,
    /// The zoom of the views inside the bus (driver, passenger): their field of view is
    /// the camera's times this (the mouse wheel, + and -, a pinch), per view.
    pub(crate) view_zoom: std::collections::HashMap<String, f32>,
    /// Eased Space return in flight (F1 only): ((look from), (zoom from), seconds in,
    /// look key it started from). A hand on the view cancels it; other views reset
    /// instantly. If the camera changes mid-glide, the originating camera is
    /// finalized straight ahead instead of keeping a partial angle.
    pub(crate) f1_reset: Option<((f32, f32), f32, f32, String)>,
    pub(crate) orbit: f32,
}

/// What the player's hands and head do: the keys and buttons held, the cursor, mouse and
/// controller driving, head tracking, the phone's touch controls and the switch being
/// dragged.
pub(crate) struct InputState {
    pub(crate) cursor: (f32, f32),
    /// Render-only cursor position; input and hit testing use `cursor` directly.
    pub(crate) cursor_display: Option<(f32, f32)>,
    pub(crate) window_focused: bool,
    /// The window lost the focus or was minimised or hidden: the keyboard and the mouse
    /// work nothing until it has the focus again (`App::input_lost` / `input_back`).
    pub(crate) input_away: bool,
    pub(crate) keys: hashbrown::HashSet<KeyCode>,
    /// Door trigger groups currently held by the Shift+number shortcut. Keeping the
    /// release until physical key-up prevents latched button states and door chatter.
    pub(crate) door_key_triggers: hashbrown::HashMap<KeyCode, Vec<String>>,
    pub(crate) mouse_look: bool,
    /// The left and right mouse buttons held.
    pub(crate) buttons_held: (bool, bool),
    /// The middle button held (looks round; the right button zooms).
    pub(crate) mmb_held: bool,
    /// The right button (or both) held: OMSI's mouse zoom (0x82c5f8) - moving the mouse up
    /// widens the view in the bus or takes the outside camera further away, by the value at
    /// the press over 500 pixels: (the cursor's height then, the zoom or distance then).
    pub(crate) both_drag: Option<(f32, f32)>,
    /// Seconds Ctrl+Shift+Page Up/Down has been held (the clock runs faster the longer).
    pub(crate) clock_hold: f32,
    /// A controller button held for looking left, right, up, down (`view_look_*`).
    pub(crate) pad_look: [bool; 4],
    /// A controller button held for the multiplayer bus radio (`voice_radio`).
    pub(crate) pad_voice_radio: bool,
    /// The arrow keys turned the head (a glance that comes back when they are let go).
    pub(crate) arrow_glance: bool,
    /// Head tracking (Settings → head tracking), started with the first frame that wants it.
    pub(crate) headtrack: Option<crate::headtrack::HeadTracker>,
    /// When head tracking last failed to start (tried again a few seconds later).
    pub(crate) headtrack_failed: Option<std::time::Instant>,
    /// Last TrackIR/OpenTrack output scales, used to keep the displayed camera position
    /// fixed while a sensitivity slider is changed.
    pub(crate) headtrack_scale_last: Option<[f32; 6]>,
    /// Per-axis compensation for a live sensitivity change.
    pub(crate) headtrack_scale_bias: [f32; 6],
    /// Last inversion state; inversion is a direction change, not a new camera origin.
    pub(crate) headtrack_invert_last: Option<[bool; 6]>,
    /// Steering wheels, pedals, joysticks and gamepads (`Inputs/gamectrler.cfg`).
    pub(crate) controllers: Option<crate::controllers::Controllers>,
    /// OMSI's mouse control (`toggel_mouse_ctrl`, O): the cursor's place steers (across) and
    /// works the pedals (up throttle, down brake).
    pub(crate) mouse_drive: bool,
    /// Mouse steering: the steering it gives (fraction of the full lock) and how long (s)
    /// it still eases in after being switched on (OMSI: a second, see app_events).
    pub(crate) mouse_steer: (f32, f32),
    /// Mouse steering's own point, past the window's edges too, and the cursor held while
    /// the mouse steers. OMSI divides the width by the speed, and at 30 km/h the edge of the
    /// screen was a third of the lock, with nowhere further to move (app_impl/mouse_grab.rs).
    pub(crate) mouse_grab: crate::app_impl::MouseGrab,
    /// Where the cursor steered when the right button began to look round: it goes back
    /// there when the button is let go, so the wheel does not jump to where looking left it.
    pub(crate) steer_cursor: Option<(f32, f32)>,
    /// The cursor is put in the middle of the window before the mouse steers for the first
    /// time (a game started with the mouse steering on: wherever the cursor was, the wheel
    /// turned and the bus drove off on full throttle).
    pub(crate) center_cursor: bool,
    /// The cursor hidden while a controller drives: where it stood.
    pub(crate) cursor_hidden: Option<(f32, f32)>,
    /// The wheel's place when it last counted as moved.
    pub(crate) last_ctl_steer: Option<f32>,
    /// The mouse's throttle and brake (eased in with the steering).
    pub(crate) mouse_pedals: (f32, f32),
    /// The speed mouse steering divides by, smoothed.
    pub(crate) mouse_kmh: f32,
    /// The speed a gamepad stick's steering divides by, smoothed (as `mouse_kmh`).
    pub(crate) pad_kmh: f32,
    /// Where a gamepad stick turns the wheel to, smoothed (`pad_steer_smooth`).
    pub(crate) pad_steer_target: f32,
    /// OMSI's global key actions from `Inputs/keyboard.cfg` ([game]).
    pub(crate) game_keys: Vec<omsi_content::KeyBinding>,
    /// Keys (DirectInput scan codes, no modifier) the player bound on the Controls page to
    /// something the original's keyboard.cfg does not have there: a driving preset (W A S D,
    /// the arrows) leaves them alone - D bound to the gearbox is the gearbox, not "steer right".
    pub(crate) own_keys: std::collections::HashSet<i32>,
    /// The same for keys held with Shift (a Shift+number of the player's own is not a door key).
    pub(crate) own_shift: std::collections::HashSet<i32>,
    /// The left button is held on a switch: mouse movement turns it.
    pub(crate) dragging: bool,
    /// The left button is held on a page of the bus (an `[htmltexture]`): its script texture
    /// index and the place on it the pointer was last seen.
    pub(crate) html_pressed: Option<(usize, f32, f32)>,
    /// The same for a page of a scenery object: its map id, script texture index and place.
    pub(crate) html_object_pressed: Option<(i64, usize, f32, f32)>,
    /// Cursor movement (logical pixels) while dragging a switch, not yet handed to the
    /// script: `<event>_drag` fires once a frame with it (see `Player::drag`).
    pub(crate) drag_delta: (f32, f32),
    /// The mouse cursor currently shows the hand (it is over a switch).
    pub(crate) cursor_kind: u8,
    /// The on-screen controls of a phone (see `touch.rs`).
    pub(crate) touch: crate::touch::Touch,
}

/// What is open over the picture and how it is being worked: the start and game menus with
/// their lists and drop-downs, the object editor, the vehicle placer, the tutorial, the
/// hover, the HUD, the navigator and the timetable.
pub(crate) struct MenuState {
    /// The game menu's vehicle chooser is open, with this vehicle chosen (index into
    /// `vehicle_list`), and the vehicles it offers (name, path).
    pub(crate) chooser: Option<usize>,
    /// The object editor, while it is on (`crate::editor`).
    pub(crate) editor: Option<crate::editor::Editor>,
    pub(crate) vehicle_list: Vec<(String, String)>,
    /// The drop-down open over a row of the settings window, if one is.
    pub(crate) dropdown: Option<crate::game_lists::Dropdown>,
    /// (manufacturer, type) of each vehicle of `vehicle_list`, by its path.
    pub(crate) vehicle_meta: std::collections::HashMap<String, (String, String)>,
    pub(crate) hud: Option<hud::Hud>,
    /// The route navigator (ETS2-style map in a corner).
    pub(crate) navigator: Option<navigator::Navigator>,
    pub(crate) menu: Option<menu::Menu>,
    /// Cursor and view the hover was last worked out for (see the redraw).
    pub(crate) hover_key: Option<(i32, i32, i32, i32)>,
    /// The cockpit switch the cursor is over, shown in the HUD.
    pub(crate) hover: Option<String>,
    /// The part under the cursor when it is not a switch, so the HUD can say so.
    pub(crate) hover_part: Option<String>,
    /// A `[mouseevent]` mesh is under the cursor (named in `hover` or not): the hand cursor.
    pub(crate) hover_hand: bool,
    /// The game menu (Escape, OMSI's `open_mainmenue`): the chosen line of it.
    pub(crate) game_menu: Option<usize>,
    /// The first line of the game menu (or chooser) shown, when a finger has scrolled it
    /// (in lines, fractional while dragged); `None`: the chosen line is kept in view.
    pub(crate) menu_top: Option<f32>,
    pub(crate) menu_scroll_drag: bool,
    /// The scroll bar of an open drop-down held with the mouse: where on its thumb it was
    /// taken (pixels from the thumb's top).
    pub(crate) dd_scroll_drag: Option<f32>,
    /// The same for the scroll bar of the timetable beside a line's tours.
    pub(crate) pane_scroll_drag: Option<f32>,
    /// The timetable beside the tours scrolled with the wheel: (the tour's line in the list,
    /// the first stop shown).
    pub(crate) pane_scroll: Option<(usize, usize)>,
    /// The digits of a time being typed in the world page of the game menu (None: not typing).
    pub(crate) menu_edit: Option<String>,
    pub(crate) menu_edit_icao: bool,
    /// `menu_edit` is the search of the open list being typed (see `game_lists::searchable`).
    pub(crate) menu_edit_search: bool,
    /// The search the open list is filtered by (empty: none).
    pub(crate) menu_search: String,
    /// The vehicle being chosen in "Place a vehicle" takes the place of the one driven
    /// (the game menu's "Swap for another vehicle", #728).
    pub(crate) swap_pending: bool,
    /// The vehicle chosen being read on a worker thread (see `App::place_vehicle`).
    pub(crate) pending_placement: Option<crate::spawn::PendingPlacement>,
    /// The line of the open list whose slider the mouse button holds (it follows the cursor).
    pub(crate) menu_drag: Option<usize>,
    /// The keyboard chose the line of the menu last (the mouse moved since: false), so the
    /// chosen line is shown lit; with the mouse only the line under it is.
    pub(crate) menu_kbd: bool,
    /// The next click on the city map puts the bus there (Esc → Move the bus on the map).
    pub(crate) teleport_pick: bool,
    /// The tutorial being run (`--tutorial`), loaded on the first frame.
    pub(crate) tutorial: Option<crate::tutorial::Tutorial>,
    /// The mouse wheel over the menu, notches not yet turned into lines.
    pub(crate) wheel_acc: f32,
    /// The object editor: an object dragged with the mouse; seconds to the next resend of
    /// all edits to the others (LAN host); the copies the host made, as this client shows them.
    pub(crate) editor_drag: bool,
    pub(crate) editor_sync_t: f32,
    pub(crate) remote_added: std::collections::HashMap<i64, crate::scene::TileGpu>,
    /// Placing a vehicle with the mouse (the spawner): see `placing`.
    pub(crate) placing: Option<crate::placing::Placing>,
    /// The chooser shows the administration's lines (label, action) instead of vehicles.
    pub(crate) admin_list: Option<Vec<(String, String)>>,
    /// Which of the game menu's lists `admin_list` holds (see `game_lists`).
    pub(crate) list_kind: Option<crate::game_lists::ListKind>,
    /// A binding chosen in the pause menu that is waiting for the next physical key:
    /// (true: [game], false: [vehicles], index in that section).
    pub(crate) key_capture: Option<(bool, usize)>,
    /// Whether the game stood paused before the menu opened (closing it goes back to that).
    pub(crate) menu_prev_pause: bool,
    /// OMSI's information bar (`view_toggle_informationdisplay`, Ctrl+Y): time, speed, the
    /// air and cabin temperatures, the passengers aboard, the trip and its next stop along
    /// the top of the picture.
    pub(crate) info_bar: bool,
    /// OMSI's timetable window (`view_set_schedule`, Insert).
    pub(crate) timetable: bool,
    /// The server's notifications on the screen (`notify`), oldest first.
    pub(crate) notices: Vec<crate::ui::Notice>,
}

/// The simulated session besides the map, the player's bus and the clock: the other
/// vehicles, the AI traffic and people, the duty and timetable, the weather and the wet
/// roads, the driver's career and the player on foot.
pub(crate) struct SessionState {
    /// A situation's further vehicles and those placed from the game menu, standing.
    pub(crate) placed: Vec<Player>,
    /// The sim date and the season's texture folder the loaded world shows (see
    /// `follow_date`).
    pub(crate) world_day: Option<(i32, Option<String>)>,
    pub(crate) traffic: Option<traffic::Traffic>,
    pub(crate) schedule: Option<schedule::Schedule>,
    pub(crate) humans: Option<humans::Humans>,
    pub(crate) duty: Option<schedule::PlayerDuty>,
    /// The duty was told the places of the stops beyond the loaded tiles.
    pub(crate) duty_places: bool,
    pub(crate) rain: rain::Rain,
    /// The player's bus's cabin air and the condensation on its glass.
    pub(crate) cabin_air: crate::condensation::CabinAir,
    /// What the tyres throw up from the water on the roads (see `puddles`).
    pub(crate) spray: puddles::Spray,
    pub(crate) lamps_on: Option<bool>,
    pub(crate) populate_t: f32,
    pub(crate) humans_populate_t: f32,
    pub(crate) first_populate: bool,
    pub(crate) envir: Option<omsi_content::Envir>,
    pub(crate) weather: Option<omsi_content::weather::Weather>,
    /// How far the clock was set since the timetable was last put out again (s; see
    /// `shift_clock`).
    pub(crate) clock_jump: f64,
    /// The bus whose seat (`settings::bus_seats`) `settings.seat` holds now.
    pub(crate) seat_bus: String,
    /// The player out of the seat, walking about (`on_foot`).
    pub(crate) on_foot: Option<crate::on_foot::OnFoot>,
    /// Where the bus last stood on the ground (and facing where): it is put back there when
    /// it falls through the world (see `admin::guard_fall`).
    pub(crate) safe_pose: Option<(glam::DVec3, f64)>,
    /// Seconds since `safe_pose` was taken.
    pub(crate) safe_age: f32,
    /// A time of day the bus's script wrote (`(S.S.Time)`), for the clock at the next frame.
    pub(crate) pending_time: Option<f64>,
    /// The play time (`clock.run_time`) the last situation was saved at.
    pub(crate) autosave_t: f64,
    /// The fuel pump or the bus wash running (`run_service`): which, and the seconds the
    /// tank or the dirt has not changed (it ends after `SERVICE_SETTLE`).
    pub(crate) pumping: Option<(&'static str, f32)>,
    /// The driver's personnel file and this session's statistics.
    pub(crate) career: career::Career,
    /// The duty's stops with their times as driven, kept in a file (`journey`).
    pub(crate) journey: Option<crate::journey::Journey>,
    /// How wet the roads are (0..1), built up by rain and dried by the sun.
    pub(crate) wetness: f32,
    /// How far the cloud cover has drifted with the wind (fractions of its tiling), summed
    /// up frame by frame so that a change of wind does not throw the sky around.
    pub(crate) cloud_drift: [f32; 2],
    /// A change of weather coming in (see `weather_cycle`).
    pub(crate) weather_blend: Option<crate::weather_cycle::Blend>,
    /// The weather cycle, when the weather chosen is `cycle`.
    pub(crate) weather_cycle: Option<crate::weather_cycle::Cycle>,
    /// The METAR sync's download under way (see `tick_metar`), and the seconds to the next one.
    pub(crate) metar_rx: Option<std::sync::mpsc::Receiver<Option<omsi_content::weather::Weather>>>,
    /// The current METAR receiver is a single manual fetch rather than the continuous sync.
    pub(crate) metar_once: bool,
    pub(crate) metar_next: f64,
}
