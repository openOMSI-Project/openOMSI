//! The game driven by fingers: a steering wheel (or the phone's tilt), the pedals, the
//! gearbox, the doors, the brakes, the indicators and the horn on the screen, a panel with
//! the rest of the cab, and the cameras. What no control takes is the cockpit's and the
//! camera's: a tap works the switch under the finger as a click does, a drag on a switch
//! turns it, a drag elsewhere looks round (or turns the outside camera), two fingers zoom.
//! While the game menu, a list or the city map is open every finger is a mouse.
//!
//! The controls are drawn with `omsi-ui` over the game's picture (Material Symbols icons),
//! in physical pixels scaled by the screen's density. The pedals and the wheel feed the
//! same analog inputs a game controller does (`Player::analog`), the buttons fire the
//! actions of `Inputs/keyboard.cfg` (or the keys that stand for them), so every bus - stock
//! or mod - is driven by them as it is by the keyboard.

use super::*;
use omsi_ui::paint::Align;
use omsi_ui::{Atlas, Color, Draw, Fonts, Gpu, Layer, Painter, Rect, Weight};
use winit::event::{Touch as Finger, TouchPhase};

/// What a control does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Btn {
    Menu,
    Pause,
    Camera,
    LookReset,
    Map,
    Timetable,
    Panel,
    Hide,
    /// The n-th door, front to back (`Shift + n`).
    Door(usize),
    StopBrake,
    ParkingBrake,
    /// A gear action (`automatic_D`, `kw_s_plus` ...) and its letter.
    Gear(&'static str, &'static str),
    BlinkLeft,
    BlinkRight,
    Hazard,
    Horn,
    // the cab panel
    Battery,
    Engine,
    AutoStart,
    Headlights,
    HighBeam,
    Wipers,
    SaloonLights,
    Ticket,
    Tilt,
    Info,
    Navigator,
    InteriorCam,
    Screenshot,
    CloseMenu,
}

impl Btn {
    /// Held down while the finger stays (a push button), rather than a tap.
    fn held(self) -> bool {
        matches!(self, Btn::Door(_) | Btn::Horn | Btn::Engine)
    }
}

/// A control on the screen this frame.
#[derive(Clone)]
struct Button {
    btn: Btn,
    rect: Rect,
    icon: &'static str,
    label: String,
    /// Lit (a switch that is on, the chosen gear).
    on: bool,
    round: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Role {
    /// A button: where it was in the list when the finger came down, and which.
    Button(usize, Btn),
    /// The wheel: the finger's angle round the wheel's centre when it last moved (rad,
    /// screen y down: clockwise positive), and its distance from the centre then.
    Wheel(f32, f32),
    Throttle,
    Brake,
    Clutch,
    /// Walking or flying the free camera (W A S D from the stick).
    Stick,
    /// A switch of the cockpit held by the finger.
    Cockpit,
    /// Looking round (or, not moved far, a tap on the picture).
    Look,
    /// The mouse, while the city map has the screen.
    Mouse,
    /// The game menu's (or the chooser's) list: a drag scrolls it, a tap picks a line.
    Menu,
}

struct Touched {
    id: u64,
    start: Vec2,
    pos: Vec2,
    role: Role,
    moved: bool,
    since: Instant,
}

use glam::Vec2;

/// The touch controls' state (a field of `App`).
pub struct Touch {
    pub enabled: bool,
    fingers: Vec<Touched>,
    /// The wheel's turn (-1 .. 1), the pedals (0 .. 1).
    steer: f32,
    throttle: f32,
    brake: f32,
    /// A finger has the wheel (it stays where it is put; let go, it comes back).
    steering: bool,
    tilt: bool,
    panel: bool,
    hidden: bool,
    /// The two fingers of a pinch: their last distance.
    pinch: Option<f32>,
    /// Keys the stick holds (free camera, on foot).
    stick_keys: Vec<KeyCode>,
    stick_at: Option<(Vec2, Vec2)>,
    buttons: Vec<Button>,
    wheel_c: Vec2,
    wheel_r: f32,
    throttle_r: Rect,
    brake_r: Rect,
    /// A manual gearbox's clutch pedal (zero-sized when the settings work the clutch).
    clutch_r: Rect,
    clutch: f32,
    stick_c: Vec2,
    stick_r: f32,
    /// Physical pixels per point of the layout.
    u: f32,
    size: (f32, f32),
    gpu: Option<(Gpu, wgpu::TextureFormat)>,
    shot_gpu: Option<Gpu>,
    fonts: Option<Fonts>,
    atlas: Atlas,
    painter: Painter,
    /// The gear button pressed last (the gearbox's variables differ from bus to bus).
    gear: Option<&'static str>,
    /// A word about what a button did (bottom middle), and how long it stays.
    note: Option<(String, f32)>,
}

impl Touch {
    pub fn new() -> Touch {
        Touch {
            enabled: crate::platform::touch_controls(),
            fingers: Vec::new(),
            steer: 0.0,
            throttle: 0.0,
            brake: 0.0,
            steering: false,
            tilt: false,
            panel: false,
            hidden: false,
            pinch: None,
            stick_keys: Vec::new(),
            stick_at: None,
            buttons: Vec::new(),
            wheel_c: Vec2::ZERO,
            wheel_r: 1.0,
            throttle_r: Rect::new(0.0, 0.0, 0.0, 0.0),
            brake_r: Rect::new(0.0, 0.0, 0.0, 0.0),
            clutch_r: Rect::new(0.0, 0.0, 0.0, 0.0),
            clutch: 0.0,
            stick_c: Vec2::ZERO,
            stick_r: 1.0,
            u: 1.0,
            size: (1.0, 1.0),
            gpu: None,
            shot_gpu: None,
            fonts: None,
            atlas: Atlas::new(1024),
            painter: Painter::new(),
            note: None,
            gear: None,
        }
    }

    fn button_at(&self, p: Vec2) -> Option<usize> {
        // (a round button takes a finger a little outside its circle)
        self.buttons.iter().position(|b| {
            if b.round {
                p.distance(b.rect.center()) <= b.rect.w * 0.5 + 6.0 * self.u
            } else {
                b.rect.pad(4.0 * self.u, 4.0 * self.u).contains(p)
            }
        })
    }

    /// The window's graphics went away (a phone put the app in the background).
    pub fn drop_gpu(&mut self) {
        self.gpu = None;
        self.shot_gpu = None;
        self.atlas = Atlas::new(1024);
    }
}

const PANEL_BG: Color = Color::rgba(14, 16, 20, 0.62);
const PANEL_ON: Color = Color::rgba(232, 160, 48, 0.92);
const TEXT: Color = Color::rgba(240, 240, 240, 1.0);
const DIM: Color = Color::rgba(240, 240, 240, 0.55);

impl App {
    /// Where the controls are, for a picture of `w` x `h` physical pixels, and what they
    /// show now.
    fn touch_layout(&mut self, w: f32, h: f32) {
        let dpi = self.window.as_ref().map(|w| w.scale_factor() as f32).unwrap_or(1.0).max(0.5);
        // (a point is the screen's density, a little larger on a big tablet and smaller
        // on a small phone)
        let short = w.min(h) / dpi;
        let u = dpi * (short / 400.0).clamp(0.85, 1.35);
        let pad = 14.0 * u;
        let t = &mut self.touch;
        t.u = u;
        t.size = (w, h);
        let mut b: Vec<Button> = Vec::new();
        let rb = |x: f32, y: f32, r: f32| Rect::new(x - r, y - r, r * 2.0, r * 2.0);
        let push = |b: &mut Vec<Button>, btn: Btn, rect: Rect, icon: &'static str, label: &str, on: bool, round: bool| {
            b.push(Button { btn, rect, icon, label: label.to_string(), on, round });
        };
        let menu_mode = self.game_menu.is_some() || self.chooser.is_some() || self.navigator.as_ref().is_some_and(|n| n.map_open());
        if menu_mode {
            // only the way out: the menu is worked with the fingers as a mouse
            let r = 24.0 * u;
            if self.game_menu.is_some() || self.chooser.is_some() {
                push(&mut b, Btn::CloseMenu, rb(w - pad - r, pad + r, r), "close", "", false, true);
            } else {
                push(&mut b, Btn::Map, rb(w - pad - r, pad + r, r), "close", "", false, true);
            }
            t.buttons = b;
            return;
        }
        // --- the top bar
        let r = 21.0 * u;
        let y = pad + r;
        let step = r * 2.0 + 10.0 * u;
        let mut x = pad + r;
        for (btn, icon) in [(Btn::Menu, "menu"), (Btn::Pause, if self.paused { "play_arrow" } else { "pause" }), (Btn::Camera, "videocam"), (Btn::LookReset, "360")] {
            push(&mut b, btn, rb(x, y, r), icon, "", btn == Btn::Pause && self.paused, true);
            x += step;
        }
        let mut x = w - pad - r;
        let hidden = t.hidden;
        push(&mut b, Btn::Hide, rb(x, y, r), if hidden { "visibility" } else { "visibility" }, "", hidden, true);
        if hidden {
            t.buttons = b;
            return;
        }
        let driving = self.player.is_some() && matches!(self.view.as_str(), "driver" | "outside" | "pax");
        x -= step;
        push(&mut b, Btn::Panel, rb(x, y, r), "tune", "", t.panel, true);
        x -= step;
        push(&mut b, Btn::Map, rb(x, y, r), "map", "", false, true);
        if self.duty.is_some() {
            x -= step;
            push(&mut b, Btn::Timetable, rb(x, y, r), "departure_board", "", self.timetable, true);
        }
        x -= step;
        push(&mut b, Btn::Screenshot, rb(x, y, r), "photo_camera", "", false, true);
        if !driving {
            // on foot or the free camera: a stick to walk or fly
            t.stick_r = 62.0 * u;
            t.stick_c = Vec2::new(pad + t.stick_r + 10.0 * u, h - pad - t.stick_r - 10.0 * u);
        } else {
            t.stick_r = 0.0;
        }
        if driving {
            let p = self.player.as_ref().unwrap();
            let has = |name: &str| p.vehicle.ty.program.trigger(name).is_some() || p.bound_actions().iter().any(|a| a.eq_ignore_ascii_case(name));
            // --- the wheel, bottom left
            t.wheel_r = 78.0 * u;
            t.wheel_c = Vec2::new(pad + t.wheel_r + 6.0 * u, h - pad - t.wheel_r);
            // --- the pedals, bottom right
            let th = 150.0 * u;
            t.throttle_r = Rect::new(w - pad - 64.0 * u, h - pad - th, 64.0 * u, th);
            let bh = 112.0 * u;
            t.brake_r = Rect::new(t.throttle_r.x - 14.0 * u - 80.0 * u, h - pad - bh, 80.0 * u, bh);
            // the gearbox above the pedals: an automatic's R N D, a sequential lever's - N +,
            // else a manual's whole gate - R, N and every gear its script has (`kw_s_1` ...),
            // in two rows as the H of the lever (R 1 3 5 over N 2 4 6)
            const MANUAL: [(&str, &str); 10] = [("kw_s_R", "R"), ("kw_s_N", "N"), ("kw_s_1", "1"), ("kw_s_2", "2"), ("kw_s_3", "3"), ("kw_s_4", "4"), ("kw_s_5", "5"), ("kw_s_6", "6"), ("kw_s_7", "7"), ("kw_s_8", "8")];
            // (the kind of gearbox by what the bus's own scripts answer to: every key of
            // the keyboard layout is bound whatever the bus, the automatic's D included)
            let scripted = |name: &str| p.vehicle.ty.program.trigger(name).is_some();
            // (a manual's scripts answer to the gear keys; the LiAZ's KPP has - and + too, and
            // triggers up to 10 whatever its box has: `antrieb_number_gears` says how many)
            // (a dashboard script answers to the automatic's keys for its own display on a
            // manual bus as well - the Sprinter W906 MT showed R N D, #279: a gearbox
            // script that reads the clutch pedal is a manual one)
            let program = &p.vehicle.ty.program;
            let reads_clutch = program.var("Clutch").is_some_and(|v| program.reads(v));
            let manual = scripted("kw_s_1") && scripted("kw_s_2") && (!scripted("automatic_D") || reads_clutch);
            let count = p.vehicle.ty.program.constant("antrieb_number_gears").map(|n| n.round() as usize).filter(|n| (1..=8).contains(n));
            let gears: Vec<(&'static str, &'static str)> = if manual {
                MANUAL.iter().copied().enumerate().filter(|(k, (a, _))| {
                    matches!(*a, "kw_s_N") || (scripted(a) && count.is_none_or(|n| *k < n + 2))
                }).map(|(_, g)| g).collect()
            } else if has("automatic_D") {
                vec![("automatic_R", "R"), ("automatic_N", "N"), ("automatic_D", "D")]
            } else if has("kw_s_plus") {
                vec![("kw_s_minus", "−"), ("kw_s_N", "N"), ("kw_s_plus", "+")]
            } else {
                vec![("kw_s_R", "R"), ("kw_s_N", "N"), ("kw_s_1", "1")]
            };
            // (the top of the gearbox: the doors go above it)
            let gy;
            if manual && gears.len() > 3 {
                // the gear engaged, as the lever's script has it
                let engaged = p.vehicle.var("antrieb_getr_aktugang").or_else(|| p.vehicle.var("antrieb_getr_gang")).map(|g| g.round() as i32);
                let label_of = |g: i32| match g {
                    -1 => "R",
                    0 => "N",
                    g => MANUAL.get(g as usize + 1).map(|x| x.1).unwrap_or(""),
                };
                let cols = gears.len().div_ceil(2);
                let gw = (t.throttle_r.right() - t.brake_r.x) / cols as f32;
                let gh = 36.0 * u;
                let top = t.throttle_r.y - 10.0 * u - 2.0 * gh - 6.0 * u;
                gy = top;
                for (k, (action, letter)) in gears.iter().enumerate() {
                    let (col, row) = (k / 2, k % 2);
                    let on = match engaged {
                        Some(g) => label_of(g) == *letter,
                        None => t.gear == Some(*letter),
                    };
                    push(&mut b, Btn::Gear(action, letter), Rect::new(t.brake_r.x + gw * col as f32 + 3.0 * u, top + row as f32 * (gh + 6.0 * u), gw - 6.0 * u, gh), "", letter, on, false);
                }
            } else {
                let gw = (t.throttle_r.right() - t.brake_r.x) / 3.0;
                gy = t.throttle_r.y - 10.0 * u - 40.0 * u;
                for (k, (action, letter)) in gears.iter().enumerate() {
                    let on = t.gear == Some(*letter) && matches!(*letter, "R" | "N" | "D");
                    push(&mut b, Btn::Gear(action, letter), Rect::new(t.brake_r.x + gw * k as f32 + 3.0 * u, gy, gw - 6.0 * u, 40.0 * u), "", letter, on, false);
                }
            }
            // a manual without the automatic clutch of the settings: its clutch pedal, left of
            // the brake's buttons
            t.clutch_r = if manual && !self.settings.auto_clutch {
                let ch = 112.0 * u;
                Rect::new(t.brake_r.x - 14.0 * u - 50.0 * u - 14.0 * u - 64.0 * u, h - pad - ch, 64.0 * u, ch)
            } else {
                Rect::new(0.0, 0.0, 0.0, 0.0)
            };
            // the brakes left of the brake pedal
            let br = 25.0 * u;
            let bx = t.brake_r.x - 14.0 * u - br;
            let pb_on = p.vehicle.var("parkingbrake").or_else(|| p.vehicle.var("Handbremse")).or_else(|| p.vehicle.var("parking_brake")).is_some_and(|v| v > 0.5);
            push(&mut b, Btn::ParkingBrake, rb(bx, h - pad - br, br), "local_parking", "", pb_on, true);
            let sb_on = p.vehicle.var("bremse_halte_sw").or_else(|| p.vehicle.var("haltestellenbremse")).is_some_and(|v| v > 0.5);
            push(&mut b, Btn::StopBrake, rb(bx, h - pad - br * 3.0 - 10.0 * u, br), "back_hand", "", sb_on, true);
            // the doors: one button for each door, front to back, above the gearbox
            let doors = crate::player::door_keys(&p.vehicle.ty).len().clamp(1, 4);
            let dr = 23.0 * u;
            let dy = gy - 12.0 * u - dr;
            for k in 0..doors {
                let dx = t.throttle_r.right() - dr - (doors - 1 - k) as f32 * (dr * 2.0 + 10.0 * u);
                push(&mut b, Btn::Door(k + 1), rb(dx, dy, dr), "door_sliding", &format!("{}", k + 1), false, true);
            }
            // the indicators and the horn beside the wheel
            let ir = 23.0 * u;
            let iy = t.wheel_c.y - t.wheel_r - 14.0 * u - ir;
            let blink = p.vehicle.var("lights_sw_blinker").map(|v| v.round() as i32).unwrap_or(0);
            let warn = p.vehicle.var("lights_sw_warnblinker").is_some_and(|v| v > 0.5);
            push(&mut b, Btn::BlinkLeft, rb(t.wheel_c.x - t.wheel_r + ir, iy, ir), "turn_left", "", blink == 1, true);
            push(&mut b, Btn::Hazard, rb(t.wheel_c.x, iy, ir), "warning", "", warn, true);
            push(&mut b, Btn::BlinkRight, rb(t.wheel_c.x + t.wheel_r - ir, iy, ir), "turn_right", "", blink == 2, true);
            let hr = 26.0 * u;
            push(&mut b, Btn::Horn, rb(t.wheel_c.x + t.wheel_r + 18.0 * u + hr, h - pad - hr, hr), "campaign", "", false, true);
            // --- the cab panel: the rest of the switches, over the middle
            if t.panel {
                let items: Vec<(Btn, &'static str, &str, bool)> = vec![
                    (Btn::Battery, "power_settings_new", "Battery", p.vehicle.var("elec_busbar_main").or_else(|| p.vehicle.var("bat_switch")).is_some_and(|v| v > 0.5)),
                    (Btn::Engine, "key", "Engine start (hold)", p.vehicle.var("engine_on").is_some_and(|v| v > 0.5)),
                    (Btn::AutoStart, "autorenew", "Start the bus by itself", false),
                    (Btn::Headlights, "light", "Headlights", p.vehicle.var("lights_fern").or_else(|| p.vehicle.var("lights_sw_licht")).is_some_and(|v| v > 0.5)),
                    (Btn::HighBeam, "flashlight_on", "High beam", false),
                    (Btn::Wipers, "water_drop", "Wipers", p.vehicle.var("wiper_sw").is_some_and(|v| v > 0.5)),
                    (Btn::SaloonLights, "highlight", "Saloon lights", false),
                    (Btn::Ticket, "confirmation_number", "Sell ticket", false),
                    (Btn::InteriorCam, "airline_seat_recline_normal", "Next seat view", false),
                    (Btn::Navigator, "navigation", "Navigator", self.navigator.as_ref().is_some_and(|n| n.enabled)),
                    (Btn::Info, "info", "Information bar", self.info_bar),
                    (Btn::Tilt, "screen_rotation", "Tilt steering", t.tilt),
                ];
                let cols = 4;
                let cw = 150.0 * u;
                let ch = 64.0 * u;
                let gap = 8.0 * u;
                let rows = items.len().div_ceil(cols);
                let pw = cols as f32 * cw + (cols - 1) as f32 * gap;
                let ph = rows as f32 * ch + (rows - 1) as f32 * gap;
                let x0 = (w - pw) * 0.5;
                let y0 = (pad + r * 2.0 + 16.0 * u).max((h - ph) * 0.42);
                for (k, (btn, icon, label, on)) in items.into_iter().enumerate() {
                    let rect = Rect::new(x0 + (k % cols) as f32 * (cw + gap), y0 + (k / cols) as f32 * (ch + gap), cw, ch);
                    push(&mut b, btn, rect, icon, label, on, false);
                }
            }
        }
        t.buttons = b;
    }

    /// A finger on the screen.
    pub(crate) fn on_touch(&mut self, event_loop: &ActiveEventLoop, f: Finger) {
        if !self.touch.enabled {
            return;
        }
        let p = Vec2::new(f.location.x as f32, f.location.y as f32);
        match f.phase {
            TouchPhase::Started => self.finger_down(event_loop, f.id, p),
            TouchPhase::Moved => self.finger_move(f.id, p),
            TouchPhase::Ended => self.finger_up(event_loop, f.id, p, false),
            TouchPhase::Cancelled => self.finger_up(event_loop, f.id, p, true),
        }
    }

    pub(crate) fn finger_down(&mut self, event_loop: &ActiveEventLoop, id: u64, p: Vec2) {
        self.touch.fingers.retain(|f| f.id != id);
        let menu_mode = self.game_menu.is_some() || self.chooser.is_some() || self.navigator.as_ref().is_some_and(|n| n.map_open());
        let t = &self.touch;
        let role = if let Some(k) = t.button_at(p) {
            Role::Button(k, t.buttons[k].btn)
        } else if self.game_menu.is_some() || self.chooser.is_some() {
            Role::Menu
        } else if menu_mode {
            Role::Mouse
        } else if t.stick_r > 0.0 && p.distance(t.stick_c) <= t.stick_r * 1.3 {
            Role::Stick
        } else if self.player.is_some() && t.stick_r == 0.0 && !t.hidden && t.throttle_r.pad(6.0 * t.u, 6.0 * t.u).contains(p) {
            Role::Throttle
        } else if self.player.is_some() && t.stick_r == 0.0 && !t.hidden && t.brake_r.pad(6.0 * t.u, 6.0 * t.u).contains(p) {
            Role::Brake
        } else if self.player.is_some() && t.stick_r == 0.0 && !t.hidden && t.clutch_r.w > 0.0 && t.clutch_r.pad(6.0 * t.u, 6.0 * t.u).contains(p) {
            Role::Clutch
        } else if self.player.is_some() && t.stick_r == 0.0 && !t.hidden && !t.tilt && p.distance(t.wheel_c) <= t.wheel_r * 1.15 {
            {
                let d = p - t.wheel_c;
                Role::Wheel(d.y.atan2(d.x), d.length())
            }
        } else {
            // the cockpit's switch under the finger, else the camera's
            self.on_cursor(p.x, p.y);
            if self.hover.is_some() && self.view == "driver" && self.touch.fingers.iter().all(|f| f.role != Role::Cockpit) {
                self.left_button(event_loop, true);
                Role::Cockpit
            } else {
                Role::Look
            }
        };
        match role {
            Role::Button(_, b) => {
                if b.held() {
                    self.touch_button(event_loop, b, true);
                }
            }
            Role::Mouse => {
                self.on_cursor(p.x, p.y);
                self.left_button(event_loop, true);
            }
            // (the line under the finger lights up; it is picked when the finger comes up
            // without having moved - a finger that came down to scroll used to pick the
            // line it landed on)
            Role::Menu => self.on_cursor(p.x, p.y),
            Role::Wheel(..) => self.touch.steering = true,
            _ => {}
        }
        self.touch.fingers.push(Touched { id, start: p, pos: p, role, moved: false, since: Instant::now() });
        self.touch_pedals(p);
        // two fingers on the picture: a pinch
        let looks: Vec<Vec2> = self.touch.fingers.iter().filter(|f| matches!(f.role, Role::Look | Role::Mouse)).map(|f| f.pos).collect();
        if looks.len() == 2 {
            self.touch.pinch = Some(looks[0].distance(looks[1]).max(1.0));
        }
    }

    pub(crate) fn finger_move(&mut self, id: u64, p: Vec2) {
        let u = self.touch.u;
        let Some(k) = self.touch.fingers.iter().position(|f| f.id == id) else { return };
        let last = self.touch.fingers[k].pos;
        {
            let f = &mut self.touch.fingers[k];
            f.pos = p;
            if !f.moved && p.distance(f.start) > 10.0 * u {
                f.moved = true;
            }
        }
        let role = self.touch.fingers[k].role;
        match role {
            Role::Wheel(a0, _) => {
                // The wheel turns as far round as the finger goes round its centre - the
                // drawn rim stays under the finger, and the full lock is 120 degrees of it
                // (`touch_paint`). It used to follow the finger's way across instead, a drag
                // of one and a half diameters for the full lock: from its place in the corner
                // the finger ran off the screen at about half a lock to the left, the system
                // took the touch away and the wheel sprang back to the middle.
                // (close to the centre the angle means nothing: only followed)
                let d = p - self.touch.wheel_c;
                let a = d.y.atan2(d.x);
                if d.length() > self.touch.wheel_r * 0.2 {
                    let mut da = a - a0;
                    if da > std::f32::consts::PI {
                        da -= std::f32::consts::TAU;
                    } else if da < -std::f32::consts::PI {
                        da += std::f32::consts::TAU;
                    }
                    self.touch.steer = (self.touch.steer + da / WHEEL_LOCK_ANGLE).clamp(-1.0, 1.0);
                }
                self.touch.fingers[k].role = Role::Wheel(a, d.length());
            }
            Role::Throttle | Role::Brake => self.touch_pedals(p),
            Role::Stick => {
                let d = p - self.touch.stick_c;
                self.touch.stick_at = Some((self.touch.stick_c, d.clamp_length_max(self.touch.stick_r)));
                self.stick_keys(d / self.touch.stick_r);
            }
            Role::Cockpit | Role::Mouse => {
                if let (Role::Mouse, Some(_)) = (role, self.touch.pinch) {
                    return self.touch_pinch();
                }
                self.on_cursor(p.x, p.y);
            }
            // the list follows the finger, line for line
            Role::Menu => {
                if self.touch.fingers[k].moved {
                    // (no line lit under a finger that scrolls)
                    self.on_cursor(-1e4, -1e4);
                    let (start, row_h) = self.ui.as_ref().map(|u| (u.menu_start as f32, u.menu_row_h.max(1.0))).unwrap_or((0.0, 1.0));
                    let top = self.menu_top.unwrap_or(start) - (p.y - last.y) / row_h;
                    let (n, rows) = (self.menu_len() as f32, self.ui.as_ref().map(|u| u.menu_rows).unwrap_or(0) as f32);
                    self.menu_top = Some(top.clamp(0.0, (n - rows).max(0.0)));
                } else {
                    self.on_cursor(p.x, p.y);
                }
            }
            Role::Look => {
                if self.touch.pinch.is_some() {
                    return self.touch_pinch();
                }
                if self.touch.fingers[k].moved {
                    // (degrees for a point dragged: a full turn is a few swipes)
                    let k = 0.28 / u;
                    // (the view turns the way the finger moves: taken the other way round,
                    // as grabbing the world, every direction felt inverted)
                    self.look_by((p.x - last.x) * k, (p.y - last.y) * k);
                }
            }
            _ => {}
        }
    }

    fn touch_pinch(&mut self) {
        let looks: Vec<Vec2> = self.touch.fingers.iter().filter(|f| matches!(f.role, Role::Look | Role::Mouse)).map(|f| f.pos).collect();
        if looks.len() < 2 {
            return;
        }
        let d = looks[0].distance(looks[1]).max(1.0);
        if let Some(d0) = self.touch.pinch {
            let amount = (d - d0) / (28.0 * self.touch.u);
            if amount.abs() > 0.05 {
                self.wheel(amount);
                self.touch.pinch = Some(d);
            }
        }
    }

    pub(crate) fn finger_up(&mut self, event_loop: &ActiveEventLoop, id: u64, p: Vec2, cancelled: bool) {
        let Some(k) = self.touch.fingers.iter().position(|f| f.id == id) else { return };
        let f = self.touch.fingers.remove(k);
        if self.touch.fingers.iter().filter(|f| matches!(f.role, Role::Look | Role::Mouse)).count() < 2 {
            self.touch.pinch = None;
        }
        match f.role {
            Role::Button(_, b) => {
                if b.held() {
                    self.touch_button(event_loop, b, false);
                } else if !cancelled && self.touch.button_at(p).and_then(|k| self.touch.buttons.get(k)).map(|x| x.btn) == Some(b) {
                    self.touch_button(event_loop, b, true);
                }
            }
            Role::Wheel(..) => self.touch.steering = self.touch.fingers.iter().any(|f| matches!(f.role, Role::Wheel(..))),
            Role::Throttle => self.touch.throttle = 0.0,
            Role::Brake => self.touch.brake = 0.0,
            Role::Clutch => self.touch.clutch = 0.0,
            Role::Stick => {
                self.touch.stick_at = None;
                self.stick_keys(Vec2::ZERO);
            }
            Role::Cockpit | Role::Mouse => {
                self.on_cursor(p.x, p.y);
                self.left_button(event_loop, false);
            }
            Role::Menu => {
                if !f.moved && !cancelled {
                    self.on_cursor(p.x, p.y);
                    self.left_button(event_loop, true);
                    self.left_button(event_loop, false);
                } else {
                    // (the fraction of a line left over is rounded, as the menu shows it)
                    self.menu_top = self.menu_top.map(f32::round);
                }
            }
            Role::Look => {
                // a tap on the picture: a click there (a switch the finger missed by a hair
                // is still found: the cursor's pick is generous)
                if !f.moved && !cancelled && f.since.elapsed().as_secs_f32() < 0.45 && self.touch.fingers.is_empty() {
                    self.on_cursor(p.x, p.y);
                    self.left_button(event_loop, true);
                    self.left_button(event_loop, false);
                }
            }
        }
    }

    /// The pedals from where the fingers on them are: the higher up the pedal, the harder
    /// it is pressed (a finger at its foot presses a little).
    fn touch_pedals(&mut self, _p: Vec2) {
        let t = &mut self.touch;
        let depth = |r: Rect, y: f32| (0.2 + 0.8 * ((r.bottom() - y) / r.h)).clamp(0.2, 1.0);
        for f in &t.fingers {
            match f.role {
                Role::Throttle => t.throttle = depth(t.throttle_r, f.pos.y),
                Role::Brake => t.brake = depth(t.brake_r, f.pos.y),
                // (the clutch pushed in most of the way is in: the gearboxes want it at 1
                // to take a gear, see `pedal_ends`, and a thumb seldom sits at the top edge)
                Role::Clutch => t.clutch = if depth(t.clutch_r, f.pos.y) >= 0.75 { 1.0 } else { depth(t.clutch_r, f.pos.y) },
                _ => {}
            }
        }
    }

    /// The stick as W A S D (free camera, on foot); pushed far, Shift runs.
    fn stick_keys(&mut self, d: Vec2) {
        let mut want = Vec::new();
        if d.y < -0.35 {
            want.push(KeyCode::KeyW);
        }
        if d.y > 0.35 {
            want.push(KeyCode::KeyS);
        }
        if d.x < -0.35 {
            want.push(KeyCode::KeyA);
        }
        if d.x > 0.35 {
            want.push(KeyCode::KeyD);
        }
        if d.length() > 0.95 {
            want.push(KeyCode::ShiftLeft);
        }
        for k in self.touch.stick_keys.clone() {
            if !want.contains(&k) {
                self.keys.remove(&k);
            }
        }
        for k in &want {
            self.keys.insert(*k);
        }
        self.touch.stick_keys = want;
    }

    /// A key tapped (with Shift held when `shift`), through the keyboard's own handler.
    fn tap_key(&mut self, event_loop: &ActiveEventLoop, code: KeyCode, shift: bool, pressed: bool) {
        if shift {
            self.keys.insert(KeyCode::ShiftLeft);
        }
        self.on_key(event_loop, code, pressed, false);
        if shift {
            self.keys.remove(&KeyCode::ShiftLeft);
        }
    }

    /// A vehicle action of `Inputs/keyboard.cfg` (the axes' own, or the scripts').
    fn vehicle_action(&mut self, name: &str, down: bool) {
        if let Some(p) = self.player.as_mut() {
            if let Some(a) = omsi_sim::engine_action(name) {
                p.axes.set(a, down);
            } else {
                p.action(name, down);
            }
        }
    }

    fn touch_note(&mut self, text: &str) {
        self.touch.note = Some((text.to_string(), 1.6));
    }

    fn touch_button(&mut self, event_loop: &ActiveEventLoop, b: Btn, down: bool) {
        if down {
            crate::platform::buzz(12);
        }
        match b {
            Btn::Menu => {
                self.touch.panel = false;
                self.open_game_menu();
            }
            Btn::CloseMenu => {
                if self.chooser.is_some() {
                    self.tap_key(event_loop, KeyCode::Escape, false, true);
                } else {
                    self.close_game_menu();
                }
            }
            Btn::Pause => self.toggle_pause(),
            Btn::Camera => {
                // driver → outside → passenger → driver (on foot: back to the bus)
                self.view = match self.view.as_str() {
                    "driver" => "outside",
                    "outside" if self.player.as_ref().is_some_and(|p| p.pax_camera_count() > 0) => "pax",
                    _ => "driver",
                }
                .into();
                let v = match self.view.as_str() {
                    "driver" => "Driver's view",
                    "outside" => "Outside view",
                    _ => "Passenger's view",
                };
                self.touch_note(v);
            }
            Btn::LookReset => {
                self.game_action("view_reset_direction");
                self.orbit = ORBIT_DEFAULT;
            }
            Btn::Map => {
                if let Some(n) = self.navigator.as_mut() {
                    n.enabled = true;
                    n.toggle_map();
                }
            }
            Btn::Timetable => self.timetable = !self.timetable,
            Btn::Panel => self.touch.panel = !self.touch.panel,
            Btn::Hide => {
                self.touch.hidden = !self.touch.hidden;
                self.touch.panel = false;
            }
            Btn::Screenshot => self.take_screenshot(),
            Btn::Door(n) => {
                let code = [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4][(n - 1).min(3)];
                self.tap_key(event_loop, code, down, down);
            }
            Btn::StopBrake => {
                self.vehicle_action("bus_dooraft", true);
                self.vehicle_action("bus_dooraft", false);
            }
            Btn::ParkingBrake => {
                self.vehicle_action("parking_brake_toggle", true);
                self.vehicle_action("parking_brake_toggle", false);
            }
            Btn::Gear(action, letter) => {
                self.touch.gear = Some(letter);
                self.vehicle_action(action, true);
                self.vehicle_action(action, false);
            }
            // (the lever itself, not the Z / C / X keys: those are the simple layouts' only,
            // and with OMSI's keys chosen the phone's indicator buttons did nothing)
            Btn::BlinkLeft => self.blinker(1),
            Btn::BlinkRight => self.blinker(2),
            Btn::Hazard => self.blinker(3),
            Btn::Horn => self.vehicle_action("horn", down),
            Btn::Engine => self.vehicle_action("kw_m_enginestart", down),
            Btn::Battery => {
                self.vehicle_action("cp_batterietrennschalter_toggle", true);
                self.vehicle_action("cp_batterietrennschalter_toggle", false);
            }
            Btn::AutoStart => {
                self.tap_key(event_loop, KeyCode::KeyU, true, true);
                self.tap_key(event_loop, KeyCode::KeyU, true, false);
                self.touch.panel = false;
            }
            Btn::Headlights => {
                self.vehicle_action("kw_scheinwerfer_toggle", true);
                self.vehicle_action("kw_scheinwerfer_toggle", false);
            }
            Btn::HighBeam => {
                self.vehicle_action("kw_fernlicht_toggle", true);
                self.vehicle_action("kw_fernlicht_toggle", false);
            }
            Btn::Wipers => {
                self.vehicle_action("kw_wipermode_up", true);
                self.vehicle_action("kw_wipermode_up", false);
            }
            Btn::SaloonLights => {
                self.tap_key(event_loop, KeyCode::KeyI, false, true);
                self.tap_key(event_loop, KeyCode::KeyI, false, false);
            }
            Btn::Ticket => {
                self.vehicle_action("ticket_give", true);
                self.vehicle_action("ticket_give", false);
            }
            Btn::InteriorCam => {
                self.game_action("view_interiorcam_plus");
            }
            Btn::Navigator => {
                if let Some(n) = self.navigator.as_mut() {
                    n.enabled = !n.enabled;
                }
            }
            Btn::Info => self.info_bar = !self.info_bar,
            Btn::Tilt => {
                self.touch.tilt = !self.touch.tilt;
                crate::platform::set_tilt(self.touch.tilt);
                let t = if self.touch.tilt { "Tilt steering on: hold the phone like a wheel" } else { "Tilt steering off" };
                self.touch_note(t);
            }
        }
    }

    /// Once a frame before the bus moves: the wheel and the pedals as the controller's axes
    /// (the stronger of the two for the pedals), and the wheel coming back when let go.
    pub(crate) fn touch_frame(&mut self, dt: f32) {
        if !self.touch.enabled {
            return;
        }
        let t = &mut self.touch;
        if t.tilt {
            if let Some(s) = crate::platform::tilt_steering() {
                t.steer = s;
            }
        } else if !t.steering {
            let back = (1.2 + 1.5 * t.steer.abs()) * dt;
            t.steer = if t.steer.abs() <= back { 0.0 } else { t.steer - back * t.steer.signum() };
        }
        if let Some((_, left)) = t.note.as_mut() {
            *left -= dt;
            if *left <= 0.0 {
                t.note = None;
            }
        }
        let (steer, thr, brk, active) = (steer_curve(t.steer), t.throttle, t.brake, t.steering || t.tilt || t.steer != 0.0);
        let clu = t.clutch;
        if let Some(p) = self.player.as_mut() {
            let a = &mut p.analog;
            if active {
                a.steering = Some(steer);
            }
            if thr > 0.0 {
                a.throttle = Some(a.throttle.unwrap_or(0.0).max(thr));
            }
            if brk > 0.0 {
                a.brake = Some(a.brake.unwrap_or(0.0).max(brk));
            }
            if clu > 0.0 {
                a.clutch = Some(a.clutch.unwrap_or(0.0).max(clu));
            }
        }
    }

    /// Paint the controls into `painter` (physical pixels).
    fn touch_paint(&mut self) {
        let speed = self.player.as_ref().map(|p| p.vehicle.physics.velocity_kmh().abs());
        let (w, h) = self.touch.size;
        let t = &mut self.touch;
        let u = t.u;
        let fonts = t.fonts.get_or_insert_with(Fonts::new);
        let pt = &mut t.painter;
        pt.clear();
        t.atlas.begin_frame();
        let atlas = &mut t.atlas;
        // the wheel, the stick and the pedals
        let driving = !t.buttons.is_empty() && t.stick_r == 0.0 && !t.hidden && t.throttle_r.w > 0.0 && t.buttons.iter().any(|b| matches!(b.btn, Btn::Gear(..)));
        if driving {
            if !t.tilt {
                let c = t.wheel_c;
                let r = t.wheel_r;
                // every part is drawn once, side by side, never one see-through shape over
                // another (that showed as darker patches where they crossed)
                use std::f32::consts::{FRAC_PI_2, PI, TAU};
                let (rim_in, hub) = (r - 12.0 * u, 16.0 * u);
                let part = Color::rgba(230, 230, 230, if t.steering { 0.95 } else { 0.8 });
                pt.circle(c, rim_in, PANEL_BG);
                // the spokes turn with the wheel (one and a half turns to the lock)
                let a0 = t.steer * WHEEL_LOCK_ANGLE;
                let half = 3.5 * u;
                for k in 0..3 {
                    let a = a0 + FRAC_PI_2 + k as f32 * TAU / 3.0 + PI;
                    let d = Vec2::new(a.cos(), a.sin());
                    let n = Vec2::new(-d.y, d.x) * half;
                    // from the hub's edge to the rim's inner edge
                    let (p0, p1) = (c + d * hub, c + d * (rim_in * rim_in - half * half).max(0.0).sqrt());
                    pt.convex(&[p0 + n, p1 + n, p1 - n, p0 - n], part);
                }
                pt.circle(c, hub, part);
                // the rim, with the top mark in its gap
                let top = a0 - FRAC_PI_2;
                pt.arc(c, rim_in, r, top + 0.12, top + TAU - 0.12, part);
                pt.arc(c, rim_in, r, top - 0.12, top + 0.12, PANEL_ON);
            } else {
                pt.text(atlas, fonts, "TILT", 14.0 * u, Weight::Bold, t.wheel_c, Align::Center, DIM);
            }
            for (r, v, name) in [(t.clutch_r, t.clutch, "CLUTCH"), (t.brake_r, t.brake, "BRAKE"), (t.throttle_r, t.throttle, "GAS")] {
                if r.w <= 0.0 {
                    continue;
                }
                pt.rounded(r, 12.0 * u, PANEL_BG);
                if v > 0.0 {
                    let fill = Rect::new(r.x, r.bottom() - r.h * v, r.w, r.h * v);
                    pt.rounded(fill, 12.0 * u, match name { "GAS" => Color::rgba(104, 190, 118, 0.75), "CLUTCH" => Color::rgba(90, 150, 230, 0.75), _ => Color::rgba(222, 78, 68, 0.75) });
                }
                pt.rounded_border(r, 12.0 * u, 2.0 * u, Color::rgba(230, 230, 230, 0.35));
                // the pedal's ribs
                for k in 1..5 {
                    let y = r.y + r.h * k as f32 / 5.0;
                    pt.line(Vec2::new(r.x + 12.0 * u, y), Vec2::new(r.right() - 12.0 * u, y), 2.0 * u, Color::rgba(230, 230, 230, 0.18));
                }
                pt.text(atlas, fonts, name, 11.0 * u, Weight::Bold, Vec2::new(r.center().x, r.bottom() - 10.0 * u), Align::Center, DIM);
            }
            if let Some(v) = speed {
                let at = Vec2::new(w * 0.5, h - 22.0 * u);
                let s = format!("{v:.0} km/h");
                let tw = fonts.width(&s, 20.0 * u, Weight::Bold);
                pt.rounded(Rect::new(at.x - tw * 0.5 - 12.0 * u, at.y - 24.0 * u, tw + 24.0 * u, 34.0 * u), 10.0 * u, PANEL_BG);
                pt.text(atlas, fonts, &s, 20.0 * u, Weight::Bold, at, Align::Center, TEXT);
            }
        }
        if t.stick_r > 0.0 && !t.hidden {
            pt.circle(t.stick_c, t.stick_r, PANEL_BG);
            pt.arc(t.stick_c, t.stick_r - 3.0 * u, t.stick_r, 0.0, std::f32::consts::TAU, Color::rgba(230, 230, 230, 0.35));
            let knob = t.stick_at.map(|(c, d)| c + d).unwrap_or(t.stick_c);
            pt.circle(knob, 24.0 * u, Color::rgba(230, 230, 230, 0.8));
            pt.icon(atlas, "open_with", knob, 26.0 * u, Color::rgba(20, 20, 20, 0.9));
        }
        if t.panel && !t.hidden {
            let rs: Vec<Rect> = t.buttons.iter().filter(|b| !b.round && !matches!(b.btn, Btn::Gear(..))).map(|b| b.rect).collect();
            if let (Some(a), Some(b)) = (rs.first(), rs.last()) {
                let all = Rect::new(a.x - 10.0 * u, a.y - 10.0 * u, b.right() - a.x + 20.0 * u, b.bottom() - a.y + 20.0 * u);
                pt.rounded(all, 16.0 * u, Color::rgba(10, 12, 16, 0.78));
            }
        }
        // the buttons
        let held: Vec<Btn> = t.fingers.iter().filter_map(|f| if let Role::Button(_, b) = f.role { Some(b) } else { None }).collect();
        for b in t.buttons.iter() {
            let pressed = held.contains(&b.btn);
            let bg = if b.on { PANEL_ON } else if pressed { Color::rgba(90, 94, 100, 0.85) } else { PANEL_BG };
            let fg = if b.on { Color::rgba(20, 20, 20, 1.0) } else { TEXT };
            if b.round {
                let c = b.rect.center();
                let r = b.rect.w * 0.5 * if pressed { 0.94 } else { 1.0 };
                pt.circle(c, r, bg);
                pt.arc(c, r - 1.5 * u, r, 0.0, std::f32::consts::TAU, Color::rgba(255, 255, 255, 0.16));
                pt.icon(atlas, b.icon, c, r * 1.15, fg);
                if !b.label.is_empty() {
                    // (the door's number)
                    let at = c + Vec2::new(r * 0.62, r * 0.62);
                    pt.circle(at, 9.0 * u, Color::rgba(20, 20, 20, 0.9));
                    pt.text(atlas, fonts, &b.label, 11.0 * u, Weight::Bold, at + Vec2::new(0.0, 4.0 * u), Align::Center, TEXT);
                }
            } else {
                pt.rounded(b.rect, 10.0 * u, bg);
                if b.icon.is_empty() {
                    pt.text_in(atlas, fonts, &b.label, 17.0 * u, Weight::Bold, b.rect, Align::Center, fg);
                } else {
                    pt.icon(atlas, b.icon, Vec2::new(b.rect.x + 24.0 * u, b.rect.center().y), 24.0 * u, fg);
                    let tr = Rect::new(b.rect.x + 44.0 * u, b.rect.y, b.rect.w - 50.0 * u, b.rect.h);
                    let text = fonts.fit(&omsi_ui::tr(&b.label), 12.5 * u, Weight::Medium, tr.w);
                    pt.text_in(atlas, fonts, &text, 12.5 * u, Weight::Medium, tr, Align::Left, fg);
                }
            }
        }
        if let Some((note, left)) = t.note.clone() {
            let a = left.clamp(0.0, 0.4) / 0.4;
            let s = omsi_ui::tr(&note).to_string();
            let tw = fonts.width(&s, 15.0 * u, Weight::Medium);
            let at = Vec2::new(w * 0.5, h * 0.42);
            pt.rounded(Rect::new(at.x - tw * 0.5 - 16.0 * u, at.y - 24.0 * u, tw + 32.0 * u, 36.0 * u), 12.0 * u, Color::rgba(10, 12, 16, 0.8 * a));
            pt.text(atlas, fonts, &s, 15.0 * u, Weight::Medium, at, Align::Center, TEXT.alpha(a));
        }
    }

    /// Lay the controls out for a frame of `w` x `h` and paint them (drawn by
    /// `Touch::render` once the game's picture is in the frame).
    pub(crate) fn touch_prepare(&mut self, w: u32, h: u32) {
        if !self.touch.enabled {
            return;
        }
        self.touch_layout(w as f32, h as f32);
        self.touch_paint();
    }

    /// `touch down|move|up x,y [id]` of an input script: a finger, through the same path.
    pub(crate) fn script_touch(&mut self, event_loop: &ActiveEventLoop, verb: &str, x: f32, y: f32, id: u64) {
        let p = Vec2::new(x, y);
        match verb {
            "down" => self.finger_down(event_loop, id, p),
            "move" => self.finger_move(id, p),
            _ => self.finger_up(event_loop, id, p, false),
        }
    }
}

impl Touch {
    /// Draw the controls painted last over the frame (`view`, `w` x `h`).
    pub(crate) fn render(&mut self, r: &Renderer, view: &wgpu::TextureView, w: u32, h: u32) {
        if !self.enabled || self.painter.is_empty() {
            return;
        }
        let format = r.format();
        if self.gpu.as_ref().map(|g| g.1 != format).unwrap_or(true) {
            self.gpu = Some((Gpu::new(&r.device, format, 1, self.atlas.size), format));
            self.atlas.mark_all_dirty();
        }
        let Some((gpu, _)) = self.gpu.as_mut() else { return };
        gpu.upload(&r.device, &r.queue, 0, &self.painter.verts);
        gpu.upload_atlas(&r.queue, &mut self.atlas);
        let layers = [Layer::flat([0.0, 0.0, w as f32, h as f32], 0.0, 1.0)];
        let draws = [Draw { buffer: 0, range: 0..self.painter.len(), layer: 0, texture: 0 }];
        let mut enc = r.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("touch controls") });
        gpu.render(&r.device, &r.queue, &mut enc, view, (w, h), None, &layers, &draws);
        r.queue.submit([enc.finish()]);
    }

    /// The controls painted last as an RGBA picture (premultiplied) of `w` x `h`, for the
    /// `shot` pictures of an input script.
    pub(crate) fn picture(&mut self, r: &Renderer, w: u32, h: u32) -> Option<Vec<u8>> {
        if !self.enabled || self.painter.is_empty() {
            return None;
        }
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let size = self.atlas.size;
        let gpu = self.shot_gpu.get_or_insert_with(|| Gpu::new(&r.device, format, 1, size));
        gpu.upload(&r.device, &r.queue, 0, &self.painter.verts);
        self.atlas.mark_all_dirty();
        gpu.upload_atlas(&r.queue, &mut self.atlas);
        // (the window's pipeline has the atlas sent again next frame)
        self.atlas.mark_all_dirty();
        let tex = r.device.create_texture(&wgpu::TextureDescriptor { label: Some("touch shot"), size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC, view_formats: &[] });
        let view = tex.create_view(&Default::default());
        let layers = [Layer::flat([0.0, 0.0, w as f32, h as f32], 0.0, 1.0)];
        let draws = [Draw { buffer: 0, range: 0..self.painter.len(), layer: 0, texture: 0 }];
        let mut enc = r.device.create_command_encoder(&Default::default());
        gpu.render(&r.device, &r.queue, &mut enc, &view, (w, h), Some(wgpu::Color::TRANSPARENT), &layers, &draws);
        let stride = (w * 4).div_ceil(256) * 256;
        let buf = r.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (stride * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        enc.copy_texture_to_buffer(tex.as_image_copy(), wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(stride), rows_per_image: None } }, wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 });
        r.queue.submit([enc.finish()]);
        buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        r.device.poll(wgpu::PollType::wait_indefinitely()).ok();
        let data = buf.slice(..).get_mapped_range();
        let mut out = vec![0u8; (w * h * 4) as usize];
        for y in 0..h as usize {
            let row = &data[y * stride as usize..y * stride as usize + w as usize * 4];
            out[y * w as usize * 4..(y + 1) * w as usize * 4].copy_from_slice(row);
        }
        Some(out)
    }
}

/// `over` (premultiplied RGBA) laid over `base` (RGBA), for a picture with the controls.
pub(crate) fn composite(base: &mut [u8], over: &[u8]) {
    for (b, o) in base.chunks_exact_mut(4).zip(over.chunks_exact(4)) {
        let a = o[3] as f32 / 255.0;
        for c in 0..3 {
            b[c] = (o[c] as f32 + b[c] as f32 * (1.0 - a)).round().min(255.0) as u8;
        }
    }
}

/// How far the wheel turns at the full lock (rad): one and a half turns, as a bus's wheel
/// and the phone bus games have it - the finger goes round and round to the lock, and the
/// wheel comes back by itself when let go. (120 degrees was a lock in a flick.)
const WHEEL_LOCK_ANGLE: f32 = 3.0 * std::f32::consts::PI;

/// The wheel's turn as the bus gets it: one to one, the drawn wheel and the bus's wheel
/// turn alike (a curve that was gentle round the middle made the bus turn faster and
/// faster as the finger went on round).
fn steer_curve(s: f32) -> f32 {
    s
}
