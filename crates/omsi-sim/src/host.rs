//! Script host for road vehicles: system variables, callbacks, sound triggers.

use crate::scripttex::{FontTable, ScriptTexture};
use crate::texttex::FontLibrary;
use crate::SimClock;
use omsi_script::{Host, NameId, Stacks, State, SysVar};
use omsi_vehicle::hof::Hof;
use parking_lot::Mutex;
use std::sync::Arc;

/// How many stops a page's departures are kept for at the same time (`omsi.getDepartures`).
pub const MAX_HTML_DEPARTURE_STOPS: usize = 8;

/// `wearlifespan` of a vehicle that does not wear (OMSI: every AI vehicle, and the
/// player's with the maintenance option "infinite").
pub const AI_WEAR_LIFESPAN: f32 = 1.5e6;

#[derive(Default)]
pub struct VehicleHost {
    /// The paint scheme the vehicle is made with (`Some(None)`: the model's own textures),
    /// set before it is made: `Colorscheme` and the scheme's `[setvar]`s are there for its
    /// `{init}`, as Omsi.exe sets them when it makes the vehicle (0x70a174), before the
    /// scripts start. None: not known yet (`apply_paint_vars` later).
    pub paint_scheme: Option<Option<usize>>,
    /// Fleet number and registration chosen by the vehicle dialog. They are copied to the
    /// script's `number` / `ident` strings before `{init}`, like Omsi.exe does.
    pub initial_number: Option<String>,
    pub initial_ident: Option<String>,
    /// A time of day a script wrote to `(S.S.Time)` this frame: the game's clock takes it.
    pub time_written: Option<f64>,
    pub clock: SimClock,
    pub mouse: (f32, f32),
    pub precip_type: f32,
    pub precip_rate: f32,
    /// World-space weather air velocity (m/s), used by water on the glass.
    pub wind: glam::Vec3,
    /// `StreetCond`: how the road under the vehicle is - 0 dry, 1 wet, 2 covered in snow,
    /// and everything in between. The engine feeds it like `Dirt_Norm` (no varlist declares
    /// it); the stock sound configurations fade `Sounds\WetLane_1.wav` in over 0 … 1 and
    /// `WetLane_2.wav` over 1 … 2, and `spray.osc` raises the wheel spray while it is
    /// between 1 and 2.
    pub street_cond: f32,
    /// People aboard (`humans_count`), set by whoever carries them.
    pub humans_count: f32,
    /// 1 while the vehicle drives a timetable duty (`schedule_active`).
    pub schedule_active: f32,
    /// The odometer's reading when the session began (km).
    pub km_base: f64,
    pub temperature: f32,
    pub abs_humidity: f32,
    pub sun_alt: f32,
    /// Last collision: position (vehicle frame, m) and energy (kJ), for the coll_* sysvars.
    /// The energy is this frame's crash: the vehicle clears it once its `collision` block
    /// has run.
    pub coll_pos: [f32; 3],
    pub coll_energy: f32,
    /// `wearlifespan`: how long wearing parts last, 1 = normal. It scales every random
    /// lifetime the scripts draw (rear-door automatic, bulbs, the matrix display). At 0 the
    /// SD200's rear door was "worn out" on its first closing and reopened by itself with the
    /// opening sound from then on.
    pub wear_lifespan: f32,
    /// Ground height (m) under a point given in the vehicle frame, minus the vehicle's own
    /// height: filled every frame by `VehicleInstance::update` for `GetHeightAbovePoint`.
    pub ground_probe: Option<Arc<dyn Fn(f32, f32, f32) -> f32 + Send + Sync>>,
    /// People standing on each `paths.cfg` link inside the cabin (`GetHumanCountOnPathLink`).
    pub humans_on_path_link: Vec<u32>,
    /// People sitting on each `[passpos]` (`GetHumanCountOnSeat`).
    pub humans_on_seat: Vec<u32>,
    /// Callbacks a script asked for that this host does not provide (reported once each).
    unknown_callbacks: Vec<String>,
    pub auto_clutch: f32,
    pub no_sound: f32,
    pub fired_triggers: Vec<String>,
    /// Triggers (lower case) whose sounds read variables in their volume curves: when one
    /// fires, the variables of that moment are kept in `fired_trigger_vars` (the door's
    /// hit sound reads `doorSpeed_<n>`, which the script turns round right after it).
    pub snapshot_triggers: hashbrown::HashSet<String>,
    pub fired_trigger_vars: Vec<(String, Vec<f32>)>,
    /// `(T.F.name)` triggers of this frame: (trigger, sound file relative to the sound folder).
    pub fired_file_triggers: Vec<(String, String)>,
    pub messages: Vec<String>,
    /// Fonts registered by `GetFontIndex` (index = position), loaded through `font_lib`.
    pub fonts: FontTable,
    pub font_lib: Option<Arc<Mutex<FontLibrary>>>,
    /// `[scripttexture]` images drawn by the `ST*` callbacks.
    pub script_textures: Vec<ScriptTexture>,
    /// Folder for `STLoadTex` paths (the vehicle directory).
    pub content_dir: std::path::PathBuf,
    /// Depot file (termini, bus stop strings, IBIS trips) behind the `Get*` callbacks.
    pub hof: Option<Arc<Hof>>,
    /// The map's ticket pack for `GetTicketName/Value`.
    pub tickets: Option<Arc<omsi_content::tickets::TicketPack>>,
    /// Coins the driver handed out with `GiveChangeCoin` (currency coin indices), taken by
    /// the passenger system.
    pub change_coins: Vec<usize>,
    /// The script's `number` string variable (the fleet number): `NrSpecRandom` is
    /// specific to it.
    pub number_var: Option<u32>,
    /// Timetable information for the `GetTT*` callbacks (line, delay in s, stops).
    pub tt_line: String,
    pub tt_delay: f32,
    pub tt_stops: Vec<(String, f32, f32)>,
    /// The map objects of `tt_stops` (0: not known).
    pub tt_stop_ids: Vec<i64>,
    pub tt_terminus_index: i32,
    pub tt_busstop_index: i32,
    /// The next buses due at the bus stop a scenery object belongs to (its `[varparent]`),
    /// soonest first, for the `GetArrBus*` callbacks of the stop's departure displays.
    pub arrivals: Vec<Arrival>,
    /// Route, line and destination requests of the vehicle's HTML pages, taken by the game
    /// (`VehicleInstance::take_html_requests`).
    pub html_requests: Vec<crate::htmltex::HtmlRequest>,
    /// The departures the game made for the stops the pages asked for (`omsi.getDepartures`),
    /// by key (trimmed, lower case): (line, destination, timestamp), soonest first.
    pub html_departures: std::collections::HashMap<String, Vec<crate::vehicle_api::Departure>>,
    /// Which board generation of the game `html_departures` is from.
    pub html_departures_gen: u64,
    /// The stops the pages asked departures for, taken by the game: the `MAX_HTML_DEPARTURE_STOPS`
    /// asked most recently, the oldest first (see [`VehicleHost::want_departures`]).
    pub html_departure_wants: Vec<String>,
}

/// A bus due at a stop (`GetArrBusLine`, `GetArrBusTerminus`, `GetArrBusTimeDiff`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Arrival {
    pub line: String,
    pub terminus: String,
    /// Seconds until it arrives: its timetable time plus its delay, less the time of day.
    /// Negative once it is due and not there yet (the stock display shows 0 and blinks).
    pub due: f32,
}

/// `STLoadTex`: a destination display loads its bitmap every time it changes, in every bus
/// of the fleet - read from the disk (or an archive) in the frame each time. The pictures
/// are kept (up to 64 MB of them), the misses too.
fn st_load(path: &std::path::Path) -> Result<Arc<omsi_texture::Image>, String> {
    type Cache = Mutex<(std::collections::HashMap<std::path::PathBuf, Result<Arc<omsi_texture::Image>, String>>, usize)>;
    static CACHE: std::sync::OnceLock<Cache> = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(hit) = cache.lock().0.get(path) {
        return hit.clone();
    }
    let got = omsi_texture::decode_file(path).map(Arc::new).map_err(|e| e.to_string());
    let mut c = cache.lock();
    let bytes = got.as_ref().map(|i| i.rgba.len()).unwrap_or(0);
    if c.1 + bytes > 64 << 20 {
        c.0.clear();
        c.1 = 0;
    }
    c.1 += bytes;
    c.0.insert(path.to_path_buf(), got.clone());
    got
}

/// The weather's air temperature and humidity (bits of two f32), known before any vehicle
/// is made: the `{init}` of a bus reads `Weather_Temperature` for its engine and cabin (the
/// PAZ and LAZ carburettor engines, every heater script). Set by the game whenever the
/// weather is loaded or changes; NaN until then.
static AMBIENT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(u64::MAX);

/// Tell vehicles made from now on what the weather is (see [`VehicleHost::new`]).
pub fn set_ambient_weather(temperature: f32, abs_humidity: f32) {
    let bits = ((temperature.to_bits() as u64) << 32) | abs_humidity.to_bits() as u64;
    AMBIENT.store(bits, std::sync::atomic::Ordering::Relaxed);
}

fn ambient_weather() -> Option<(f32, f32)> {
    let bits = AMBIENT.load(std::sync::atomic::Ordering::Relaxed);
    (bits != u64::MAX).then(|| (f32::from_bits((bits >> 32) as u32), f32::from_bits(bits as u32)))
}

impl VehicleHost {
    /// A page asked for the departures of stop `key` (trimmed, lower case). The game keeps the
    /// stops asked most recently: one asked again moves to the back, a new one past the limit
    /// pushes out the one asked longest ago. (A fixed first-come list stayed full for good, and
    /// every later stop - the bus drives on to new ones - got no departures at all.)
    pub fn want_departures(&mut self, key: String) {
        if let Some(i) = self.html_departure_wants.iter().position(|k| *k == key) {
            let k = self.html_departure_wants.remove(i);
            self.html_departure_wants.push(k);
            return;
        }
        if self.html_departure_wants.len() >= MAX_HTML_DEPARTURE_STOPS {
            self.html_departure_wants.remove(0);
        }
        self.html_departure_wants.push(key);
    }

    pub fn new(clock: SimClock) -> Self {
        // the weather is there before {init} runs: made at 0 °C (the value before the first
        // weather update) every engine was cold, and the PAZ's carburettor engine, which
        // wants its choke below 0 °C, caught and died again on the key
        let (temperature, abs_humidity) = ambient_weather().unwrap_or((0.0, 0.0));
        // `AutoClutch` is 1 unless OMSI's options say `[no_automaticClutch]` (the exe sets
        // 1.0 before reading them): the manual gearboxes of the Sprinters, the PAZ and the
        // LAZ work their clutch themselves then, and at 0 every gear a driver without a
        // clutch pedal put in was thrown out again at once
        Self { clock, wear_lifespan: AI_WEAR_LIFESPAN, auto_clutch: 1.0, temperature, abs_humidity, ..Default::default() }
    }

    /// A host for trying the scripts out on a copy of the vehicle's state (the IBIS typist
    /// plans its key presses that way): the clock, weather, depot, tickets and timetable,
    /// but no fonts, script textures or sounds.
    pub fn scratch(&self) -> VehicleHost {
        VehicleHost {
            clock: self.clock.clone(),
            precip_type: self.precip_type,
            precip_rate: self.precip_rate,
            wind: self.wind,
            street_cond: self.street_cond,
            temperature: self.temperature,
            abs_humidity: self.abs_humidity,
            sun_alt: self.sun_alt,
            wear_lifespan: self.wear_lifespan,
            humans_on_path_link: self.humans_on_path_link.clone(),
            humans_on_seat: self.humans_on_seat.clone(),
            unknown_callbacks: self.unknown_callbacks.clone(),
            auto_clutch: self.auto_clutch,
            no_sound: self.no_sound,
            content_dir: self.content_dir.clone(),
            hof: self.hof.clone(),
            tickets: self.tickets.clone(),
            tt_line: self.tt_line.clone(),
            tt_delay: self.tt_delay,
            tt_stops: self.tt_stops.clone(),
            tt_stop_ids: self.tt_stop_ids.clone(),
            tt_terminus_index: self.tt_terminus_index,
            tt_busstop_index: self.tt_busstop_index,
            ..Default::default()
        }
    }

    /// Callbacks the scripts called that this host does not provide (lower case).
    pub fn unknown_callbacks(&self) -> &[String] {
        &self.unknown_callbacks
    }

    /// The font behind a `GetFontIndex` result; -1 (a missing font) draws nothing.
    fn font_atlas(&self, index: i32) -> Option<Arc<omsi_content::font::FontAtlas>> {
        usize::try_from(index).ok().and_then(|i| self.fonts.entries.get(i)).and_then(|f| f.1.clone())
    }

    fn terminus(&self, index: i32) -> Option<&omsi_vehicle::hof::Terminus> {
        self.hof.as_ref().and_then(|h| h.termini.get(usize::try_from(index).ok()?))
    }

    fn arrival(&self, index: i32) -> Option<&Arrival> {
        self.arrivals.get(usize::try_from(index).ok()?)
    }

    fn bus_stop(&self, index: i32) -> Option<&omsi_vehicle::hof::BusStop> {
        self.hof.as_ref().and_then(|h| h.bus_stops.get(usize::try_from(index).ok()?))
    }
}

/// The `(M.V.…)` callbacks this host answers (lower case); anything else reads 0.
pub const PROVIDED_CALLBACKS: &[&str] = &[
    "getfontindex", "textlength", "stnewtex", "stlock", "stunlock", "stfilter", "stsetcolor", "stdrawpixel", "stdrawrect", "sttextout", "streadpixel", "stgetr", "stgetg", "stgetb", "stgeta", "stcopycolor", "stloadtex",
    "getrouteindex", "getrouteterminusindex", "getterminuscode", "getterminusindex", "getterminusstring", "getbusstopcount", "getroutebusstopident", "getbusstopindex", "getbusstopstring", "getdepotstringglobal",
    "getttlinestring", "getttdelay", "getttbusstopcount", "getttbusstopindex", "gettterminusindex", "getttterminusindex", "getttbusstopname", "getttbusstopdep", "getttbusstoparr",
    "getheightabovepoint", "gethumancountonpathlink", "gethumancountonseat", "givechangecoin", "nrspecrandom", "getticketname", "gettticketname", "getticketvalue",
    "getarrbusline", "getarrbusterminus", "getarrbustimediff",
];

/// A number a script hands a callback, as Omsi.exe takes it: rounded to the nearest
/// integer, a half to the even one (`fistp` under Delphi's control word; TRoadVehicleInst
/// 0x7d28c4 and TComplMapObjInst 0x7bb10c convert every argument with sub_404c7c), not cut
/// off - 1100.9999 out of float arithmetic is 1101.
fn arg_i32(v: f32) -> i32 {
    v.round_ties_even() as i32
}

/// The same as an index (a negative one names nothing).
fn arg_idx(v: f32) -> usize {
    usize::try_from(arg_i32(v)).unwrap_or(usize::MAX)
}

impl Host for VehicleHost {
    fn sys_var(&mut self, v: SysVar) -> f32 {
        match v {
            SysVar::Timegap => self.clock.timegap,
            SysVar::GetTime => self.clock.run_time as f32,
            SysVar::NoSound => self.no_sound,
            SysVar::Pause => if self.clock.paused { 1.0 } else { 0.0 },
            // seconds since midnight ("Time in seconds", says the stock clock object): every
            // script divides it by 3600 for the hour (IBIS and ticket printer clocks) or
            // compares it with 28800/72000 (the Citaro's door chime by day). Handing out hours
            // made every clock read 00:00 and the day-time checks think it was night.
            SysVar::Time => self.clock.time as f32,
            SysVar::Day => self.clock.day_month().0 as f32,
            SysVar::Month => self.clock.day_month().1 as f32,
            SysVar::Year => self.clock.year as f32,
            SysVar::DayOfYear => self.clock.day_of_year as f32,
            SysVar::MouseX => self.mouse.0,
            SysVar::MouseY => self.mouse.1,
            SysVar::PrecipType => self.precip_type,
            SysVar::PrecipRate => self.precip_rate,
            SysVar::CollPosX => self.coll_pos[0],
            SysVar::CollPosY => self.coll_pos[1],
            SysVar::CollPosZ => self.coll_pos[2],
            // Read as often as the block likes: the stock collision block adds it to the
            // general damage and then again to the engine's, and a value gone after the
            // first read left the engine and the drivetrain whole after any crash.
            SysVar::CollEnergy => self.coll_energy,
            SysVar::WeatherTemperature => self.temperature,
            SysVar::WeatherAbsHum => self.abs_humidity,
            SysVar::WearLifespan => self.wear_lifespan,
            SysVar::AutoClutch => self.auto_clutch,
            SysVar::SunAlt => self.sun_alt,
        }
    }

    /// `(S.S.x)`: OMSI writes through the system variable's pointer into the engine's
    /// own value (`TXPC_calcblock_var`). What the engine sets anew every frame (the frame
    /// time, the mouse, the weather, the sun) takes the write for the rest of this frame
    /// only; the clock keeps it - `time_written` hands a new time of day to the game - and so
    /// do the collision values and the switches.
    fn set_sys_var(&mut self, v: SysVar, value: f32) {
        if !value.is_finite() {
            return;
        }
        match v {
            SysVar::Timegap => self.clock.timegap = value,
            SysVar::GetTime => self.clock.run_time = value as f64,
            SysVar::NoSound => self.no_sound = value,
            SysVar::Pause => self.clock.paused = value > 0.5,
            SysVar::Time => {
                self.clock.time = (value as f64).rem_euclid(86400.0);
                self.time_written = Some(self.clock.time);
            }
            SysVar::Day | SysVar::Month | SysVar::Year => {
                let (d, m) = self.clock.day_month();
                let (y, m, d) = match v {
                    SysVar::Day => (self.clock.year, m, value as i32),
                    SysVar::Month => (self.clock.year, value as i32, d),
                    _ => (value as i32, m, d),
                };
                self.clock.set_date(y, m, d);
            }
            SysVar::DayOfYear => self.clock.day_of_year = (value as i32).clamp(1, crate::clock::days_in_year(self.clock.year)),
            SysVar::MouseX => self.mouse.0 = value,
            SysVar::MouseY => self.mouse.1 = value,
            SysVar::PrecipType => self.precip_type = value,
            SysVar::PrecipRate => self.precip_rate = value,
            SysVar::CollPosX => self.coll_pos[0] = value,
            SysVar::CollPosY => self.coll_pos[1] = value,
            SysVar::CollPosZ => self.coll_pos[2] = value,
            SysVar::CollEnergy => self.coll_energy = value,
            SysVar::WeatherTemperature => self.temperature = value,
            SysVar::WeatherAbsHum => self.abs_humidity = value,
            SysVar::WearLifespan => self.wear_lifespan = value,
            SysVar::AutoClutch => self.auto_clutch = value,
            SysVar::SunAlt => self.sun_alt = value,
        }
    }

    fn callback(&mut self, name: &str, _id: NameId, stacks: &mut Stacks, state: &mut State) {
        let mut buf = [0u8; 64];
        let owned;
        let lname: &str = match buf.get_mut(..name.len()) {
            Some(b) => {
                b.copy_from_slice(name.as_bytes());
                b.make_ascii_lowercase();
                std::str::from_utf8(b).unwrap_or(name)
            }
            None => {
                owned = name.to_ascii_lowercase();
                &owned
            }
        };
        if lname.starts_with("st") && debug_text() {
            log::info!("callback {name} stack {:?} strings {:?}", &stacks.st[..stacks.st.len().min(8)], stacks.sst.last());
        }
        match lname {
            "getfontindex" => {
                let font = stacks.pop_str();
                let idx = match self.fonts.entries.iter().position(|f| f.0.eq_ignore_ascii_case(&font)) {
                    Some(i) => i,
                    None => {
                        // an empty name or a depot string that is no font (the Krüger
                        // matrix asks for the custom fonts of termini 14991.. this way) is
                        // looked up once and remembered as missing
                        let atlas = if font.trim().is_empty() { None } else { self.font_lib.as_ref().and_then(|l| l.lock().load(&font)) };
                        self.fonts.entries.push((font, atlas));
                        self.fonts.entries.len() - 1
                    }
                };
                // OMSI answers -1 for a font it does not have, and scripts rely on it: the
                // Krüger matrix falls back from a missing custom font to the one before with
                // `1 + 31416 * l1 1 + max 31415 % 1 -`, which only works for -1. An index for
                // a font that cannot draw left the destination display blank.
                let found = self.fonts.entries[idx].1.is_some();
                stacks.push(if found { idx as f32 } else { -1.0 });
            }
            "textlength" => {
                let font = arg_i32(stacks.pop());
                let text = stacks.pop_str();
                // as Omsi.exe measures it (0x5d6c00): the glyphs and the gaps between them
                let w = self.font_atlas(font).map(|a| a.font.text_width(&text) as f32).unwrap_or(0.0);
                if debug_text() {
                    log::info!("TextLength(font {font} = {:?}, {text:?}) = {w}", self.font_atlas(font).map(|a| a.font.name.clone()));
                }
                stacks.push(w);
            }
            // --- script textures (matrix displays)
            "stnewtex" => {
                let i = arg_idx(stacks.pop());
                if let Some(t) = self.script_textures.get_mut(i) {
                    t.renew();
                }
            }
            "stlock" => {
                let i = arg_idx(stacks.pop());
                if let Some(t) = self.script_textures.get_mut(i) {
                    t.locked = true;
                }
            }
            "stunlock" => {
                let i = arg_idx(stacks.pop());
                if let Some(t) = self.script_textures.get_mut(i) {
                    t.unlock();
                }
            }
            "stfilter" => {
                // OMSI creates the mip chain only when the script asks it to, normally
                // immediately after STUnlock. Remember that request for the renderer; it
                // must also refresh the chain when this matrix changes again.
                let i = arg_idx(stacks.pop());
                if let Some(t) = self.script_textures.get_mut(i) {
                    if !t.mipmaps {
                        t.mipmaps = true;
                        t.dirty = true;
                    }
                }
            }
            "stsetcolor" => {
                let b = stacks.pop();
                let g = stacks.pop();
                let r = stacks.pop();
                let a = stacks.pop();
                let i = arg_idx(stacks.pop());
                if let Some(t) = self.script_textures.get_mut(i) {
                    t.color = [r, g, b, a].map(|c| arg_i32(c).clamp(0, 255) as u8);
                }
            }
            "stdrawpixel" => {
                let y = arg_i32(stacks.pop());
                let x = arg_i32(stacks.pop());
                let i = arg_idx(stacks.pop());
                if let Some(t) = self.script_textures.get_mut(i) {
                    let c = t.color;
                    t.put(x, y, c);
                }
            }
            "stdrawrect" => {
                let y2 = arg_i32(stacks.pop());
                let x2 = arg_i32(stacks.pop());
                let y1 = arg_i32(stacks.pop());
                let x1 = arg_i32(stacks.pop());
                let i = arg_idx(stacks.pop());
                if let Some(t) = self.script_textures.get_mut(i) {
                    t.rect(x1, y1, x2, y2);
                }
            }
            "sttextout" => {
                let text = stacks.pop_str();
                let spacing = arg_i32(stacks.pop());
                let mode = stacks.pop();
                let font = arg_i32(stacks.pop());
                let y = arg_i32(stacks.pop());
                let x = arg_i32(stacks.pop());
                let i = arg_idx(stacks.pop());
                let atlas = self.font_atlas(font);
                if debug_text() {
                    log::info!("STTextOut(tex {i}, x {x}, y {y}, font {font}, spacing {spacing}, {text:?}) atlas {:?} from {:?} bitmap {:?} {}x{} E {:?}", atlas.as_ref().map(|a| a.font.name.clone()), atlas.as_ref().map(|a| a.font.path.clone()), atlas.as_ref().map(|a| a.font.alpha.clone()), atlas.as_ref().map(|a| a.width).unwrap_or(0), atlas.as_ref().map(|a| a.height).unwrap_or(0), atlas.as_ref().and_then(|a| a.font.glyph('E').map(|g| (g.x0, g.x1, g.y))));
                }
                if let (Some(t), Some(a)) = (self.script_textures.get_mut(i), atlas) {
                    t.text_out(&a, x, y, spacing, arg_i32(mode) as u8, &text);
                }
            }
            "streadpixel" => {
                let y = arg_i32(stacks.pop());
                let x = arg_i32(stacks.pop());
                let i = arg_idx(stacks.pop());
                // `STReadPixel` makes the selected texture's current ST colour the
                // pixel it read.  RHLib then asks `STGet*` of either that source texture
                // or a different target texture: the latter must retain its own draw
                // colour while the source keeps changing beneath the scaler.
                if let Some(t) = self.script_textures.get_mut(i) {
                    let color = t.get(x, y);
                    t.color = color;
                }
            }
            "stgetr" | "stgetg" | "stgetb" | "stgeta" => {
                let i = arg_idx(stacks.pop());
                let k = match lname {
                    "stgetr" => 0,
                    "stgetg" => 1,
                    "stgetb" => 2,
                    _ => 3,
                };
                stacks.push(self.script_textures.get(i).map(|t| t.color[k] as f32).unwrap_or(0.0));
            }
            "stcopycolor" => {
                // colour of texture a → texture b
                let b = arg_idx(stacks.pop());
                let a = arg_idx(stacks.pop());
                if let Some(c) = self.script_textures.get(a).map(|t| t.color) {
                    if let Some(t) = self.script_textures.get_mut(b) {
                        t.color = c;
                    }
                }
            }
            "stloadtex" => {
                let i = arg_idx(stacks.pop());
                let path = stacks.pop_str();
                // relative to the vehicle's texture folder, like every texture name of the
                // vehicle: the Krüger matrix loads `..\..\Anzeigen\Krueger\<bitmap>`,
                // which is `Vehicles\Anzeigen\...` seen from `Vehicles\<bus>\Texture`
                // (and found in whichever content root has it)
                let mut full = omsi_cfg::resolve_path(&omsi_cfg::resolve_path(&self.content_dir, "Texture"), &path);
                // (then the global `Texture` folder, as OMSI looks)
                if !omsi_cfg::vfs::is_file(&full) {
                    if let Some((_, p)) = omsi_cfg::find_in_roots(&format!("Texture/{}", path.replace('\\', "/"))) {
                        full = p;
                    }
                }
                // (OMSI pushes nothing: a file it cannot find goes to its log only)
                match st_load(&full) {
                    Ok(img) => {
                        if let Some(t) = self.script_textures.get_mut(i) {
                            t.load(img.width, img.height, &img.rgba);
                        }
                    }
                    Err(e) => log::warn!("STLoadTex: did not find texture file {}: {e}", full.display()),
                }
            }
            // --- depot (HOF) callbacks of the IBIS: arguments come off the float stack, the
            // last pushed first
            "getrouteindex" => {
                let code = stacks.pop();
                let code_i = arg_i32(code);
                let idx = self
                    .hof
                    .as_ref()
                    .and_then(|h| {
                        h.info_trips.iter().position(|t| {
                            let c = t.code.trim();
                            omsi_cfg::parse_i32(c) == code_i
                                || (code - omsi_cfg::parse_f32(c)).abs() < 1e-3
                        })
                    })
                    .map(|i| i as f32)
                    .unwrap_or(-1.0);
                stacks.push(idx);
            }
            // An index of -1 (what the lookups answer for an unknown code) names no entry:
            // the strings are empty and the codes -1. Reading entry 0 instead put the first
            // terminus of the depot ("LEERFELD") where a mod's display looks for a custom
            // font or an optional sign.
            "getrouteterminusindex" => {
                let route = arg_i32(stacks.pop());
                let idx = self.hof.as_ref().and_then(|h| {
                    let trip = h.info_trips.get(usize::try_from(route).ok()?)?;
                    let code = omsi_cfg::parse_i32(&trip.route);
                    h.termini.iter().position(|t| t.code == code)
                });
                stacks.push(idx.map(|i| i as f32).unwrap_or(-1.0));
            }
            "getterminuscode" => {
                let idx = arg_i32(stacks.pop());
                let code = self.terminus(idx).map(|t| t.code as f32).unwrap_or(-1.0);
                stacks.push(code);
            }
            "getterminusindex" => {
                let code = arg_i32(stacks.pop());
                let idx = self.hof.as_ref().and_then(|h| h.termini.iter().position(|t| t.code == code)).map(|i| i as f32).unwrap_or(-1.0);
                stacks.push(idx);
            }
            "getterminusstring" => {
                let n = arg_i32(stacks.pop());
                let idx = arg_i32(stacks.pop());
                let s = self.terminus(idx).and_then(|t| t.strings.get(usize::try_from(n).ok()?)).cloned().unwrap_or_default();
                stacks.push_str(s);
            }
            "getbusstopcount" => {
                let route = arg_i32(stacks.pop());
                let n = self.hof.as_ref().and_then(|h| h.info_busstop_lists.get(usize::try_from(route).ok()?)).map(|l| l.len() as f32).unwrap_or(0.0);
                stacks.push(n);
            }
            "getroutebusstopident" => {
                let stop = arg_i32(stacks.pop());
                let route = arg_i32(stacks.pop());
                let s = self.hof.as_ref().and_then(|h| h.info_busstop_lists.get(usize::try_from(route).ok()?)).and_then(|l| l.get(usize::try_from(stop).ok()?)).cloned().unwrap_or_default();
                stacks.push_str(s);
            }
            "getbusstopindex" => {
                let ident = stacks.pop_str();
                let idx = self.hof.as_ref().and_then(|h| h.bus_stops.iter().position(|b| b.ident.trim().eq_ignore_ascii_case(ident.trim()))).map(|i| i as f32).unwrap_or(-1.0);
                stacks.push(idx);
            }
            "getbusstopstring" => {
                let n = arg_i32(stacks.pop());
                let idx = arg_i32(stacks.pop());
                let s = self.bus_stop(idx).and_then(|b| b.strings.get(usize::try_from(n).ok()?)).cloned().unwrap_or_default();
                stacks.push_str(s);
            }
            "getdepotstringglobal" => {
                let n = arg_i32(stacks.pop());
                let s = self.hof.as_ref().and_then(|h| h.global_strings.get(usize::try_from(n).ok()?)).cloned().unwrap_or_default();
                stacks.push_str(s);
            }
            // --- timetable callbacks
            "getttlinestring" => stacks.push_str(self.tt_line.clone()),
            "getttdelay" => stacks.push(self.tt_delay),
            "getttbusstopcount" => stacks.push(self.tt_stops.len() as f32),
            // Without a timetable there is no current stop. Returning the first
            // stop (0) makes the Atron repeatedly detect an arrival and clear its
            // sales text after the holding brake has been on for three seconds.
            "getttbusstopindex" => stacks.push(if self.tt_stops.is_empty() {
                -1.0
            } else {
                self.tt_busstop_index as f32
            }),
            "gettterminusindex" | "getttterminusindex" => stacks.push(if self.tt_stops.is_empty() {
                -1.0
            } else {
                self.tt_terminus_index as f32
            }),
            // how high a point of the vehicle stands over the ground (the NL/NG ramp
            // measures the kerb this way before extending)
            "getheightabovepoint" => {
                let z = stacks.pop();
                let y = stacks.pop();
                let x = stacks.pop();
                let h = self.ground_probe.as_ref().map(|f| f(x, y, z)).unwrap_or(0.0);
                stacks.push(h);
            }
            // passengers standing on one link of the cabin path network
            "gethumancountonpathlink" => {
                let i = arg_i32(stacks.pop());
                stacks.push(self.humans_on_path_link.get(usize::try_from(i).unwrap_or(usize::MAX)).copied().unwrap_or(0) as f32);
            }
            "getttbusstopname" => {
                let i = arg_i32(stacks.pop());
                stacks.push_str(self.tt_stops.get(usize::try_from(i).unwrap_or(usize::MAX)).map(|s| s.0.clone()).unwrap_or_default());
            }
            "getttbusstopdep" => {
                let i = arg_i32(stacks.pop());
                stacks.push(self.tt_stops.get(usize::try_from(i).unwrap_or(usize::MAX)).map(|s| s.2).unwrap_or(0.0));
            }
            "getttbusstoparr" => {
                let i = arg_i32(stacks.pop());
                stacks.push(self.tt_stops.get(usize::try_from(i).unwrap_or(usize::MAX)).map(|s| s.1).unwrap_or(0.0));
            }
            "givechangecoin" => {
                let i = arg_i32(stacks.pop());
                if i >= 0 {
                    self.change_coins.push(i as usize);
                }
            }
            // OMSI's number-specific random value: the same for a fleet
            // number and characteristic every time (broken matrix pixels, wear effects)
            "nrspecrandom" => {
                let n = arg_i32(stacks.pop());
                let number = self.number_var.and_then(|i| state.str_vars.get(i as usize)).cloned().unwrap_or_default();
                stacks.push(omsi_vehicle::sound::spec_random(&number, n));
            }
            "getticketname" | "gettticketname" => {
                let i = arg_i32(stacks.pop());
                stacks.push_str(self.tickets.as_ref().and_then(|t| t.tickets.get(usize::try_from(i).unwrap_or(usize::MAX))).map(|t| t.name.clone()).unwrap_or_default());
            }
            "getticketvalue" => {
                let i = arg_i32(stacks.pop());
                stacks.push(self.tickets.as_ref().and_then(|t| t.tickets.get(usize::try_from(i).unwrap_or(usize::MAX))).map(|t| t.value).unwrap_or(0.0));
            }
            "gethumancountonseat" => {
                let i = arg_i32(stacks.pop());
                stacks.push(self.humans_on_seat.get(usize::try_from(i).unwrap_or(usize::MAX)).copied().unwrap_or(0) as f32);
            }
            // --- scenery objects (`program/callbacklist_scenobj.txt`): the n-th bus due at
            // the object's stop. Past the last one the line and terminus are empty (the stock
            // display leaves its line dark on an empty terminus) and the time is 0.
            "getarrbusline" => {
                let i = arg_i32(stacks.pop());
                stacks.push_str(self.arrival(i).map(|a| a.line.clone()).unwrap_or_default());
            }
            "getarrbusterminus" => {
                let i = arg_i32(stacks.pop());
                stacks.push_str(self.arrival(i).map(|a| a.terminus.clone()).unwrap_or_default());
            }
            "getarrbustimediff" => {
                let i = arg_i32(stacks.pop());
                stacks.push(self.arrival(i).map(|a| a.due).unwrap_or(0.0));
            }
            _ => {
                // numeric callbacks default to 0; say once which one the host lacks
                if !self.unknown_callbacks.iter().any(|n| n == lname) {
                    log::warn!("script callback (M.V.{name}) is not provided; it reads 0");
                    self.unknown_callbacks.push(lname.to_string());
                }
                stacks.push(0.0);
            }
        }
    }

    fn sound_trigger_file(&mut self, name: &str, file: &str) {
        // `" " (T.F.ev_nbs_on)` is how the MB_C2 gearbox script says "no sound" (every
        // frame in neutral, 80 times a minute per bus at Ahlheim): nothing to play
        if file.trim().is_empty() {
            return;
        }
        self.fired_file_triggers.push((name.to_string(), file.to_string()));
    }
    fn sound_trigger(&mut self, name: &str, _id: NameId) {
        self.fired_triggers.push(name.to_string());
    }
    fn sound_trigger_vars(&mut self, name: &str, id: NameId, vars: &[f32]) {
        self.sound_trigger(name, id);
        if !self.snapshot_triggers.is_empty() {
            let key = name.to_ascii_lowercase();
            if self.snapshot_triggers.contains(&key) {
                self.fired_trigger_vars.push((key, vars.to_vec()));
            }
        }
    }

    /// `$msg`: kept as the last few (OMSI shows the latest on its debug line; every
    /// AI bus says one per stop, and the list grew all session).
    fn message(&mut self, text: &str) {
        const KEEP: usize = 16;
        log::debug!("script message: {text}");
        if self.messages.len() >= KEEP {
            self.messages.remove(0);
        }
        self.messages.push(text.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scripttex::ScriptTexture;
    use omsi_script::{compile, CompileInput, Vm};

    #[test]
    fn departure_wants_keep_the_stops_asked_most_recently() {
        let mut host = VehicleHost::new(SimClock::default());
        for i in 0..MAX_HTML_DEPARTURE_STOPS + 3 {
            host.want_departures(format!("stop {i}"));
        }
        assert_eq!(host.html_departure_wants.len(), MAX_HTML_DEPARTURE_STOPS);
        assert_eq!(host.html_departure_wants.first().map(String::as_str), Some("stop 3"));
        assert_eq!(host.html_departure_wants.last().map(String::as_str), Some("stop 10"));
        // asked again: moves to the back and nothing is pushed out
        host.want_departures("stop 3".to_string());
        assert_eq!(host.html_departure_wants.len(), MAX_HTML_DEPARTURE_STOPS);
        assert_eq!(host.html_departure_wants.last().map(String::as_str), Some("stop 3"));
        assert_eq!(host.html_departure_wants.first().map(String::as_str), Some("stop 4"));
    }

    /// A callback's argument is rounded, as Omsi.exe's `fistp` does: 1100.9999 out of the
    /// script's arithmetic names terminus 1101, not 1100.
    #[test]
    fn callback_arguments_are_rounded() {
        let p = compile(&CompileInput::default());
        let mut state = State::new(&p);
        let mut host = VehicleHost::new(SimClock::default());
        let terminus = |code: i32, name: &str| omsi_vehicle::hof::Terminus { code, strings: vec![name.to_string()], ..Default::default() };
        host.hof = Some(Arc::new(Hof { termini: vec![terminus(1100, "A"), terminus(1101, "B")], ..Default::default() }));
        let mut stacks = Stacks::default();
        stacks.push(1100.9999);
        host.callback("GetTerminusIndex", 0, &mut stacks, &mut state);
        assert_eq!(stacks.pop(), 1.0);
        // index 0.9999, string 0: the second terminus' first string
        stacks.push(0.9999);
        stacks.push(0.0);
        host.callback("GetTerminusString", 0, &mut stacks, &mut state);
        assert_eq!(stacks.pop_str(), "B");
        // a half goes to the even number, as under Delphi's control word
        assert_eq!((arg_i32(0.5), arg_i32(1.5), arg_i32(2.5), arg_i32(-0.5)), (0, 2, 2, 0));
        assert_eq!(arg_idx(-1.0), usize::MAX);
    }

    #[test]
    fn script_texture_colour_is_kept_per_texture() {
        let p = compile(&CompileInput::default());
        let mut state = State::new(&p);
        let mut host = VehicleHost::new(SimClock::default());
        host.script_textures.push(ScriptTexture::new(2, 1));
        host.script_textures.push(ScriptTexture::new(2, 1));
        host.script_textures[0].put(1, 0, [12, 34, 56, 78]);
        host.script_textures[1].color = [1, 2, 3, 255];
        let mut stacks = Stacks::default();

        // RHLib reads a source pixel, then asks its target texture whether its
        // previously selected drawing colour is transparent.
        stacks.push(0.0);
        stacks.push(1.0);
        stacks.push(0.0);
        host.callback("STReadPixel", 0, &mut stacks, &mut state);

        stacks.push(0.0);
        host.callback("STGetA", 0, &mut stacks, &mut state);
        assert_eq!(stacks.pop(), 78.0);
        stacks.push(1.0);
        host.callback("STGetA", 0, &mut stacks, &mut state);
        assert_eq!(stacks.pop(), 255.0);
    }

    #[test]
    fn stfilter_marks_a_script_texture_for_mipmaps() {
        let dir = std::env::temp_dir().join(format!("omsi_host_stfilter_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("matrix.osc");
        std::fs::write(
            &script,
            "{trigger:matrix_refresh}\n0 (M.V.STLock)\n0 (M.V.STUnlock)\n0 (M.V.STFilter)\n{end}\n",
        )
            .unwrap();
        let p = compile(&CompileInput {
            scripts: vec![script],
            ..Default::default()
        });
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        let mut host = VehicleHost::new(SimClock::default());
        host.script_textures.push(ScriptTexture::new(32, 16));
        host.script_textures[0].dirty = false;
        let mut st = State::new(&p);
        let mut vm = Vm::new();

        assert!(vm.run_trigger(&p, "matrix_refresh", &mut st, &mut host));
        assert!(!host.script_textures[0].locked);
        assert!(host.script_textures[0].mipmaps);
        assert!(host.script_textures[0].dirty);
    }

    #[test]
    fn atron_unlock_filter_relock_publishes_the_released_image() {
        let dir = std::env::temp_dir().join(format!(
            "omsi_host_atron_test_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("atron.osc");
        std::fs::write(
            &script,
            "{trigger:draw}\n0 (M.V.STLock)\n0 255 255 255 255 (M.V.STSetColor)\n0 1 1 (M.V.STDrawPixel)\n0 (M.V.STUnlock)\n0 (M.V.STFilter)\n0 (M.V.STLock)\n0 2 1 (M.V.STDrawPixel)\n{end}\n{trigger:publish}\n0 (M.V.STUnlock)\n0 (M.V.STFilter)\n0 (M.V.STLock)\n{end}\n",
        ).unwrap();
        let p = compile(&CompileInput {
            scripts: vec![script],
            ..Default::default()
        });
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        let mut host = VehicleHost::new(SimClock::default());
        host.script_textures.push(ScriptTexture::new(4, 2));
        let mut state = State::new(&p);
        let mut vm = Vm::new();
        assert!(vm.run_trigger(&p, "draw", &mut state, &mut host));
        let t = &mut host.script_textures[0];
        assert!(t.locked);
        assert!(t.mipmaps);
        let first = t.take_upload().expect("STUnlock must survive STLock in the same frame");
        assert_eq!(&first[20..24], &[255; 4]);
        assert_eq!(
            &first[24..28],
            &[0; 4],
            "edits after relocking wait for the next unlock"
        );
        assert!(t.take_upload().is_none());
        assert!(vm.run_trigger(&p, "publish", &mut state, &mut host));
        let second = host.script_textures[0].take_upload().unwrap();
        assert_eq!(&second[24..28], &[255; 4]);
        assert!(vm.run_trigger(&p, "publish", &mut state, &mut host));
        assert!(
            host.script_textures[0].take_upload().is_none(),
            "filtering unchanged pixels needs no new upload"
        );
    }

    #[test]
    fn atron_arrival_check_does_not_clear_sales_text_without_a_timetable() {
        let dir = std::env::temp_dir().join(format!(
            "omsi_host_atron_arrival_test_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("arrival.osc");
        std::fs::write(&script, "{trigger:check}\n(M.V.GetTTBusstopIndex) 1 - -1 >=\n{if}\n0 (M.V.STNewTex)\n{endif}\n{end}\n").unwrap();
        let p = compile(&CompileInput {
            scripts: vec![script],
            ..Default::default()
        });
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        let mut host = VehicleHost::new(SimClock::default());
        host.script_textures.push(ScriptTexture::new(1, 1));
        host.script_textures[0].put(0, 0, [255; 4]);
        let mut state = State::new(&p);
        let mut vm = Vm::new();
        assert!(vm.run_trigger(&p, "check", &mut state, &mut host));
        assert_eq!(host.script_textures[0].rgba, vec![255; 4]);
        // The first stop of an actual timetable still reports an arrival.
        host.tt_stops.push(("First stop".into(), 0.0, 0.0));
        assert!(vm.run_trigger(&p, "check", &mut state, &mut host));
        assert_eq!(host.script_textures[0].rgba, vec![0; 4]);
        host.script_textures[0].put(0, 0, [255; 4]);
        host.tt_stops.clear();
        assert!(vm.run_trigger(&p, "check", &mut state, &mut host));
        assert_eq!(host.script_textures[0].rgba, vec![255; 4]);
    }

    /// The engine part of the stock SD200/SD202/NL202 collision block: a rear hit low down
    /// damages the engine, one at the front only the general account.
    #[test]
    fn a_collision_block_reads_the_energy_twice() {
        let dir = std::env::temp_dir().join(format!("omsi_host_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("collision.osc");
        std::fs::write(
            &script,
            "{trigger:collision}\n(L.L.collision_energy) (L.S.coll_energy) + (S.L.collision_energy)\n(L.S.coll_pos_y) -4.70 <\n(L.S.coll_pos_z) 1.10 < &&\n{if}\n(L.L.collision_energy_eng) (L.S.coll_energy) + (S.L.collision_energy_eng)\n{endif}\n{end}\n",
        )
            .unwrap();
        let vars = dir.join("vars.txt");
        std::fs::write(&vars, "collision_energy\ncollision_energy_eng\n").unwrap();
        let p = compile(&CompileInput { varlists: vec![vars], scripts: vec![script], ..Default::default() });
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        let mut host = VehicleHost::new(SimClock::default());
        let mut st = State::new(&p);
        let mut vm = Vm::new();
        host.coll_pos = [-1.23, -5.73, 0.90];
        host.coll_energy = 115.5;
        assert!(vm.run_trigger(&p, "collision", &mut st, &mut host));
        host.coll_pos = [1.23, 5.62, 0.88];
        host.coll_energy = 20.0;
        vm.run_trigger(&p, "collision", &mut st, &mut host);
        assert_eq!(st.get(p.var("collision_energy").unwrap()), 135.5);
        assert_eq!(st.get(p.var("collision_energy_eng").unwrap()), 115.5);
    }
}

fn debug_text() -> bool {
    omsi_cfg::flags::OMSI_DEBUG_TEXT.is_set()
}
