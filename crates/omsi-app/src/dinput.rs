//! Game controllers on Windows through DirectInput 8, as Omsi.exe reads them: every device
//! Windows lists as a game controller (wheels with their makers' drivers, pedals, joysticks,
//! button boxes - the system's newer Windows.Gaming.Input misses many of them), the eight
//! axes in the slots `gamectrler.cfg` numbers (X, Y, Z, Rx, Ry, Rz, the two sliders), up to
//! 128 buttons, and force feedback: one constant force on the wheel's axis the game sets
//! every frame (its centring spring, the drag of the steering, the shaking of the bus).
//!
//! The list of devices is looked up on a thread of its own, when Windows says a HID device
//! (every game controller is one) was plugged in or out: with some drivers the lookup takes
//! a tenth of a second and stalls the reading of the devices even from another thread, so
//! the old look every 3 seconds made driving stutter.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};
use windows::core::{w, Interface, GUID};
use windows::Win32::Devices::HumanInterfaceDevice::*;
use windows::Win32::Foundation::{HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

/// What a device gives: the axes (-1..1) in their DirectInput slots and 128 buttons. Laid out
/// as the data format below says.
#[repr(C)]
#[derive(Clone, Copy)]
struct RawState {
    axes: [i32; 8],
    pov: [u32; 4],
    buttons: [u8; 128],
}

impl Default for RawState {
    fn default() -> Self {
        RawState { axes: [0; 8], pov: [u32::MAX; 4], buttons: [0; 128] }
    }
}

const RANGE: i32 = 10_000;

/// One device opened.
pub(crate) struct Device {
    pub name: String,
    guid: GUID,
    dev: IDirectInputDevice8W,
    /// The slots the device has (an axis it lacks is not in the list).
    has_axis: [bool; 8],
    /// Hardware capability, also needed by the launcher which does not create effects.
    ff_capable: bool,
    /// Offset of the axis that can receive forces in our data format.
    ff_axis: u32,
    state: RawState,
    ff: Option<IDirectInputEffect>,
    ff_error_logged: bool,
    pub buttons: usize,
}

/// Take the device again (after the window left the front). DirectInput resets a force
/// feedback wheel when it is taken: its own centring came back on, and a G29 pulled itself
/// to the middle after the pause until mouse steering was switched on and off.
fn reacquire(dev: &IDirectInputDevice8W, ff: bool) -> bool {
    unsafe {
        let _ = dev.Unacquire();
        if ff {
            let mut ac = DIPROPDWORD { diph: DIPROPHEADER { dwSize: std::mem::size_of::<DIPROPDWORD>() as u32, dwHeaderSize: std::mem::size_of::<DIPROPHEADER>() as u32, dwObj: 0, dwHow: DIPH_DEVICE }, dwData: DIPROPAUTOCENTER_OFF };
            let _ = dev.SetProperty(prop(9), &mut ac.diph);
        }
        dev.Acquire().is_ok()
    }
}

impl Device {
    /// The axes the device has: (slot, value -1..1).
    pub fn axes(&self) -> Vec<(usize, f32)> {
        (0..8).filter(|k| self.has_axis[*k]).map(|k| (k, self.state.axes[k] as f32 / RANGE as f32)).collect()
    }

    pub fn has_ff(&self) -> bool {
        self.ff.is_some()
    }

    pub fn ff_capable(&self) -> bool {
        self.ff_capable
    }
}

/// The devices, and the thread that finds them.
pub(crate) struct DirectInput {
    di: IDirectInput8W,
    hwnd: HWND,
    /// The game wants force feedback (the window is the game's, not the launcher's).
    ff: bool,
    /// A foreground FFB device must not be reacquired while the window is away.
    focused: bool,
    pub devices: Vec<Device>,
    found: Arc<Mutex<Option<Vec<(GUID, String)>>>>,
    scan: mpsc::Sender<()>,
    /// Button changes since the last `poll`: (device, button, down).
    pub events: Vec<(String, usize, bool)>,
    last_force: Instant,
}

unsafe extern "system" fn collect(inst: *mut DIDEVICEINSTANCEW, out: *mut core::ffi::c_void) -> windows::core::BOOL {
    let v = &mut *(out as *mut Vec<(GUID, String)>);
    let inst = &*inst;
    let end = inst.tszProductName.iter().position(|c| *c == 0).unwrap_or(inst.tszProductName.len());
    v.push((inst.guidInstance, String::from_utf16_lossy(&inst.tszProductName[..end]).trim().to_string()));
    windows::core::BOOL(DIENUM_CONTINUE as i32)
}

fn create() -> Option<IDirectInput8W> {
    unsafe {
        let hinst: HINSTANCE = GetModuleHandleW(None).ok()?.into();
        let mut p: *mut core::ffi::c_void = std::ptr::null_mut();
        DirectInput8Create(hinst, DIRECTINPUT_VERSION, &IDirectInput8W::IID, &mut p, None).ok()?;
        (!p.is_null()).then(|| IDirectInput8W::from_raw(p))
    }
}

fn list(di: &IDirectInput8W) -> Vec<(GUID, String)> {
    let mut v: Vec<(GUID, String)> = Vec::new();
    unsafe {
        let _ = di.EnumDevices(DI8DEVCLASS_GAMECTRL, Some(collect), &mut v as *mut _ as *mut core::ffi::c_void, DIEDFL_ATTACHEDONLY);
    }
    v
}

/// How often Windows said a HID device came or went (see `notification_window`).
static HID_CHANGES: AtomicU64 = AtomicU64::new(0);

unsafe extern "system" fn notify_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_DEVICECHANGE && matches!(wp.0 as u32, DBT_DEVICEARRIVAL | DBT_DEVICEREMOVECOMPLETE) {
        HID_CHANGES.fetch_add(1, Ordering::Relaxed);
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

/// A message-only window of the calling thread that Windows tells when a HID device is
/// plugged in or out (the way SDL finds new controllers).
fn notification_window() -> Option<HWND> {
    unsafe {
        let hinst: HINSTANCE = GetModuleHandleW(None).ok()?.into();
        let class = w!("openOMSI game controllers");
        let wc = WNDCLASSW { lpfnWndProc: Some(notify_proc), hInstance: hinst, lpszClassName: class, ..Default::default() };
        // (0 when the class is there already - a second window of the launcher's)
        let _ = RegisterClassW(&wc);
        let hwnd = CreateWindowExW(WINDOW_EX_STYLE(0), class, w!(""), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, Some(hinst), None).ok()?;
        let filter = DEV_BROADCAST_DEVICEINTERFACE_W {
            dbcc_size: std::mem::size_of::<DEV_BROADCAST_DEVICEINTERFACE_W>() as u32,
            dbcc_devicetype: DBT_DEVTYP_DEVICEINTERFACE.0,
            dbcc_classguid: GUID_DEVINTERFACE_HID,
            ..Default::default()
        };
        if RegisterDeviceNotificationW(HANDLE(hwnd.0), &filter as *const _ as *const core::ffi::c_void, DEVICE_NOTIFY_WINDOW_HANDLE).is_err() {
            let _ = DestroyWindow(hwnd);
            return None;
        }
        Some(hwnd)
    }
}

#[derive(Clone, Copy)]
struct InputObject {
    guid: GUID,
    ty: u32,
    flags: u32,
}

unsafe extern "system" fn collect_object(inst: *mut DIDEVICEOBJECTINSTANCEW, out: *mut core::ffi::c_void) -> windows::core::BOOL {
    let objects = &mut *(out as *mut Vec<InputObject>);
    let inst = &*inst;
    objects.push(InputObject { guid: inst.guidType, ty: inst.dwType, flags: inst.dwFlags });
    windows::core::BOOL(DIENUM_CONTINUE as i32)
}

fn axis_slot(guid: GUID, has_axis: &[bool; 8]) -> Option<usize> {
    if guid == GUID_XAxis {
        Some(0)
    } else if guid == GUID_YAxis {
        Some(1)
    } else if guid == GUID_ZAxis {
        Some(2)
    } else if guid == GUID_RxAxis {
        Some(3)
    } else if guid == GUID_RyAxis {
        Some(4)
    } else if guid == GUID_RzAxis {
        Some(5)
    } else if guid == GUID_Slider {
        (6..8).find(|k| !has_axis[*k])
    } else {
        None
    }
}

/// Build the DirectInput data format from the controls the device actually exposes.
/// Some button boxes have no axes or POVs and reject a generic joystick format even when
/// its missing entries are marked optional.
fn format_objects(objects: &[InputObject]) -> (Vec<DIOBJECTDATAFORMAT>, [bool; 8], Option<u32>) {
    let mut objs = Vec::new();
    let mut has_axis = [false; 8];
    let mut ff_axis = None;
    let mut pov = 0;
    let mut button = 0;
    for object in objects {
        let (offset, flags) = if object.ty & DIDFT_AXIS != 0 {
            let Some(slot) = axis_slot(object.guid, &has_axis) else { continue };
            if has_axis[slot] {
                continue;
            }
            has_axis[slot] = true;
            let offset = (slot * 4) as u32;
            if object.flags & DIDOI_FFACTUATOR != 0 {
                ff_axis.get_or_insert(offset);
            }
            (offset, DIDOI_ASPECTPOSITION)
        } else if object.ty & DIDFT_POV != 0 && pov < 4 {
            let offset = (32 + pov * 4) as u32;
            pov += 1;
            (offset, 0)
        } else if object.ty & DIDFT_BUTTON != 0 && button < 128 {
            let offset = (48 + button) as u32;
            button += 1;
            (offset, 0)
        } else {
            continue;
        };
        // The exact instance number is already part of dwType, so the GUID is unnecessary
        // here and we do not keep pointers into the temporary enumeration buffer.
        objs.push(DIOBJECTDATAFORMAT { pguid: std::ptr::null(), dwOfs: offset, dwType: object.ty, dwFlags: flags });
    }
    (objs, has_axis, ff_axis)
}

fn data_format(dev: &IDirectInputDevice8W) -> Option<(Vec<DIOBJECTDATAFORMAT>, DIDATAFORMAT, [bool; 8], Option<u32>)> {
    let mut objects = Vec::new();
    unsafe {
        dev.EnumObjects(Some(collect_object), &mut objects as *mut _ as *mut core::ffi::c_void, DIDFT_ALL).ok()?;
    }
    let (objs, has_axis, ff_axis) = format_objects(&objects);
    let f = DIDATAFORMAT {
        dwSize: std::mem::size_of::<DIDATAFORMAT>() as u32,
        dwObjSize: std::mem::size_of::<DIOBJECTDATAFORMAT>() as u32,
        dwFlags: DIDF_ABSAXIS,
        dwDataSize: std::mem::size_of::<RawState>() as u32,
        dwNumObjs: objs.len() as u32,
        rgodf: std::ptr::null_mut(),
    };
    Some((objs, f, has_axis, ff_axis))
}

/// `MAKEDIPROP(n)`: DirectInput's own properties are numbers passed where a GUID's address
/// goes.
fn prop(n: usize) -> *const GUID {
    n as *const GUID
}

impl DirectInput {
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    pub fn force_axis(&self, name: &str) -> Option<usize> {
        self.devices.iter().find(|d| d.name == name && d.ff.is_some()).map(|d| d.ff_axis as usize / 4)
    }

    /// `hwnd`: the window the devices belong to; `ff`: take the devices for force feedback
    /// (the game's window: they then answer only while it is in front, as in OMSI).
    pub fn new(hwnd: isize, ff: bool) -> Option<DirectInput> {
        let di = create()?;
        let first = list(&di);
        log::info!("game controllers (DirectInput): {}", if first.is_empty() { "none".to_string() } else { first.iter().map(|d| d.1.as_str()).collect::<Vec<_>>().join(", ") });
        let found = Arc::new(Mutex::new(Some(first)));
        let f2 = found.clone();
        let (scan, requests) = mpsc::channel::<()>();
        let _ = std::thread::Builder::new().name("game controllers".into()).spawn(move || {
            let Some(di) = create() else { return };
            let window = notification_window();
            if window.is_none() {
                log::info!("game controllers: Windows gives no device notifications - looking for new ones every 3 s");
            }
            let mut seen = HID_CHANGES.load(Ordering::Relaxed);
            let mut last = Instant::now();
            // (a device plugged in raises several notifications: look once they settle)
            let mut pending: Option<Instant> = None;
            loop {
                loop {
                    match requests.try_recv() {
                        Ok(()) => pending = Some(Instant::now()),
                        Err(mpsc::TryRecvError::Empty) => break,
                        // (the game let the devices go)
                        Err(mpsc::TryRecvError::Disconnected) => {
                            if let Some(w) = window {
                                unsafe {
                                    let _ = DestroyWindow(w);
                                }
                            }
                            return;
                        }
                    }
                }
                unsafe {
                    let _ = MsgWaitForMultipleObjects(None, false, 250, QS_ALLINPUT);
                    let mut msg = MSG::default();
                    while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                        DispatchMessageW(&msg);
                    }
                }
                let n = HID_CHANGES.load(Ordering::Relaxed);
                if n != seen {
                    seen = n;
                    pending = Some(Instant::now());
                }
                if window.is_none() && last.elapsed() > Duration::from_secs(3) {
                    pending = Some(Instant::now() - Duration::from_secs(1));
                }
                if pending.is_some_and(|t| t.elapsed() > Duration::from_millis(300)) {
                    pending = None;
                    last = Instant::now();
                    let v = list(&di);
                    log::info!("game controllers found: {}", v.iter().map(|d| d.1.as_str()).collect::<Vec<_>>().join(", "));
                    *f2.lock().unwrap() = Some(v);
                }
            }
        });
        Some(DirectInput { di, hwnd: HWND(hwnd as *mut _), ff, focused: true, devices: Vec::new(), found, scan, events: Vec::new(), last_force: Instant::now() })
    }

    /// Let go of foreground effects when the game loses focus, then take them once on return.
    pub fn set_focus(&mut self, focused: bool) {
        if self.focused == focused {
            return;
        }
        self.focused = focused;
        log::info!("game controllers: window {} focus; {} DirectInput device(s)", if focused { "regained" } else { "lost" }, self.devices.len());
        for d in &mut self.devices {
            unsafe {
                if focused {
                    if !reacquire(&d.dev, d.ff.is_some()) {
                        log::warn!("{}: DirectInput could not reacquire the device after focus returned", d.name);
                    }
                    d.ff_error_logged = false;
                } else {
                    if let Some(e) = d.ff.as_ref() {
                        let _ = e.Stop();
                    }
                    let _ = d.dev.Unacquire();
                }
            }
        }
    }

    /// Look for devices again (the window heard of one plugged in or out).
    pub fn refresh(&self) {
        let _ = self.scan.send(());
    }

    fn open(&self, guid: &GUID, name: &str) -> Option<Device> {
        unsafe {
            let mut dev: Option<IDirectInputDevice8W> = None;
            self.di.CreateDevice(guid, &mut dev, None).ok()?;
            let dev = dev?;
            let Some((mut objs, mut fmt, has_axis, ff_axis)) = data_format(&dev) else {
                log::warn!("{name}: DirectInput could not list the device's controls");
                return None;
            };
            fmt.rgodf = objs.as_mut_ptr();
            if let Err(e) = dev.SetDataFormat(&mut fmt) {
                log::warn!("{name}: DirectInput rejected the device's data format ({e})");
                return None;
            }
            let mut caps = DIDEVCAPS { dwSize: std::mem::size_of::<DIDEVCAPS>() as u32, ..Default::default() };
            let _ = dev.GetCapabilities(&mut caps);
            let ff_capable = caps.dwFlags & DIDC_FORCEFEEDBACK != 0;
            let mut wants_ff = self.ff && ff_capable;
            let level = if wants_ff { DISCL_EXCLUSIVE | DISCL_FOREGROUND } else { DISCL_NONEXCLUSIVE | DISCL_BACKGROUND };
            if let Err(e) = dev.SetCooperativeLevel(self.hwnd, level) {
                if wants_ff {
                    // (forces need the device to themselves: another program - the wheel's
                    // own control software - may be holding it)
                    log::warn!("{name}: force feedback needs the device to itself, which Windows refused ({e}): no forces");
                    wants_ff = false;
                }
                dev.SetCooperativeLevel(self.hwnd, DISCL_NONEXCLUSIVE | DISCL_BACKGROUND).ok()?;
            }
            // every axis from -RANGE to RANGE
            let mut range = DIPROPRANGE { diph: DIPROPHEADER { dwSize: std::mem::size_of::<DIPROPRANGE>() as u32, dwHeaderSize: std::mem::size_of::<DIPROPHEADER>() as u32, dwObj: 0, dwHow: DIPH_DEVICE }, lMin: -RANGE, lMax: RANGE };
            let _ = dev.SetProperty(prop(4), &mut range.diph);
            // no dead zone and no saturation of the driver's own (DIPROP_DEADZONE 5,
            // DIPROP_SATURATION 6): some wheels' drivers set one - a PXN V99 lost 30 % of
            // its turn round the middle - and OMSI clears them as well; the settings'
            // dead zone is the only one
            for (p_id, v) in [(5usize, 0u32), (6, 10_000)] {
                let mut d = DIPROPDWORD { diph: DIPROPHEADER { dwSize: std::mem::size_of::<DIPROPDWORD>() as u32, dwHeaderSize: std::mem::size_of::<DIPROPHEADER>() as u32, dwObj: 0, dwHow: DIPH_DEVICE }, dwData: v };
                let _ = dev.SetProperty(prop(p_id), &mut d.diph);
            }
            let ff_axis = ff_axis.unwrap_or(0);
            let mut ff = None;
            if wants_ff {
                // the wheel's own centring off: the game's forces take its place
                let mut ac = DIPROPDWORD { diph: DIPROPHEADER { dwSize: std::mem::size_of::<DIPROPDWORD>() as u32, dwHeaderSize: std::mem::size_of::<DIPROPHEADER>() as u32, dwObj: 0, dwHow: DIPH_DEVICE }, dwData: DIPROPAUTOCENTER_OFF };
                if let Err(err) = dev.SetProperty(prop(9), &mut ac.diph) {
                    log::warn!("{name}: autocenter could not be disabled ({err})");
                }
            }
            let _ = dev.Acquire();
            if wants_ff {
                let mut axes = [ff_axis; 1];
                let mut dirs = [0i32; 1];
                let mut cf = DICONSTANTFORCE { lMagnitude: 0 };
                let mut eff = DIEFFECT {
                    dwSize: std::mem::size_of::<DIEFFECT>() as u32,
                    dwFlags: DIEFF_CARTESIAN | DIEFF_OBJECTOFFSETS,
                    dwDuration: u32::MAX, // INFINITE
                    dwGain: DI_FFNOMINALMAX,
                    dwTriggerButton: DIEB_NOTRIGGER,
                    cAxes: 1,
                    rgdwAxes: axes.as_mut_ptr(),
                    rglDirection: dirs.as_mut_ptr(),
                    cbTypeSpecificParams: std::mem::size_of::<DICONSTANTFORCE>() as u32,
                    lpvTypeSpecificParams: &mut cf as *mut _ as *mut core::ffi::c_void,
                    ..Default::default()
                };
                let mut e: Option<IDirectInputEffect> = None;
                match dev.CreateEffect(&GUID_ConstantForce, &mut eff, &mut e, None) {
                    Ok(()) => {
                        if let Some(e) = e.as_ref() {
                            if let Err(err) = e.Start(1, 0) {
                                log::warn!("{name}: force feedback effect could not start ({err})");
                            }
                        }
                        ff = e;
                    }
                    Err(err) => log::warn!("{name}: says it has force feedback, but its constant force could not be made ({err}): no forces"),
                }
            }
            log::info!("game controller (DirectInput): {name}, {} axes, {} buttons{}", has_axis.iter().filter(|a| **a).count(), caps.dwButtons, if ff.is_some() { format!(", force feedback on axis {}", ff_axis / 4) } else if ff_capable && self.ff { ", force feedback capable (effect unavailable)".into() } else if ff_capable { ", force feedback capable".into() } else { String::new() });
            Some(Device { name: name.to_string(), guid: *guid, dev, has_axis, ff_capable, ff_axis, state: RawState::default(), ff, ff_error_logged: false, buttons: caps.dwButtons as usize })
        }
    }

    /// Read every device; devices plugged in or out since the last list are opened or let go.
    pub fn poll(&mut self) {
        if !self.focused {
            self.events.clear();
            return;
        }
        if let Some(list) = self.found.lock().unwrap().take() {
            self.devices.retain(|d| list.iter().any(|(g, _)| *g == d.guid));
            for (g, name) in list {
                if !self.devices.iter().any(|d| d.guid == g) {
                    match self.open(&g, &name) {
                        Some(d) => self.devices.push(d),
                        // (said once per device list, so that a log tells why a device the
                        // system lists is missing)
                        None => log::warn!("game controller {name}: listed by Windows, but DirectInput could not open it"),
                    }
                }
            }
        }
        for d in &mut self.devices {
            let mut s = RawState { axes: d.state.axes, ..Default::default() };
            let read = |s: &mut RawState| unsafe {
                let _ = d.dev.Poll();
                d.dev.GetDeviceState(std::mem::size_of::<RawState>() as u32, s as *mut _ as *mut core::ffi::c_void)
            };
            let ok = match read(&mut s) {
                Ok(()) => true,
                Err(_) => reacquire(&d.dev, d.ff.is_some()) && read(&mut s).is_ok(),
            };
            if !ok {
                continue;
            }
            for b in 0..128 {
                let (was, now) = (d.state.buttons[b] & 0x80 != 0, s.buttons[b] & 0x80 != 0);
                if was != now {
                    self.events.push((d.name.clone(), b, now));
                }
            }
            // the hat switches as buttons after the 128 (up, right, down, left of each): the
            // D-pad of a wheel rim - Moza's among others - is a hat, and could not be given a key
            for k in 0..4 {
                let dirs = |pov: u32| -> [bool; 4] {
                    if pov == u32::MAX || pov & 0xFFFF == 0xFFFF {
                        return [false; 4];
                    }
                    let a = (pov % 36000) as i32;
                    let near = |c: i32| {
                        let d = (a - c).rem_euclid(36000);
                        d.min(36000 - d) < 6750
                    };
                    [near(0), near(9000), near(18000), near(27000)]
                };
                let (was, now) = (dirs(d.state.pov[k]), dirs(s.pov[k]));
                for dir in 0..4 {
                    if was[dir] != now[dir] {
                        self.events.push((d.name.clone(), crate::controllers::HAT_BUTTONS + k * 4 + dir, now[dir]));
                    }
                }
            }
            d.state = s;
        }
    }

    /// The force on the wheel of device `name`: -1 (full to the left) .. 1. Set at most 100
    /// times a second (each is a message to the device).
    /// Returns whether a force-feedback effect with this exact device name exists.
    pub fn set_force(&mut self, name: &str, f: f32) -> bool {
        let found = self.devices.iter().any(|d| d.name == name && d.ff.is_some());
        if !found {
            return false;
        }
        if !self.focused {
            return true;
        }
        if self.last_force.elapsed() < Duration::from_millis(10) {
            return true;
        }
        self.last_force = Instant::now();
        for d in self.devices.iter_mut().filter(|d| d.name == name) {
            let Some(e) = d.ff.as_ref() else { continue };
            let mut axes = [d.ff_axis; 1];
            let mut dirs = [0i32; 1];
            let mut cf = DICONSTANTFORCE { lMagnitude: (f.clamp(-1.0, 1.0) * DI_FFNOMINALMAX as f32) as i32 };
            let mut eff = DIEFFECT {
                dwSize: std::mem::size_of::<DIEFFECT>() as u32,
                dwFlags: DIEFF_CARTESIAN | DIEFF_OBJECTOFFSETS,
                cAxes: 1,
                rgdwAxes: axes.as_mut_ptr(),
                rglDirection: dirs.as_mut_ptr(),
                cbTypeSpecificParams: std::mem::size_of::<DICONSTANTFORCE>() as u32,
                lpvTypeSpecificParams: &mut cf as *mut _ as *mut core::ffi::c_void,
                ..Default::default()
            };
            unsafe {
                // (a device taken away - the window left the front - is taken again)
                let result = e.SetParameters(&mut eff, DIEP_TYPESPECIFICPARAMS | DIEP_START);
                let result = if result.is_err() {
                    reacquire(&d.dev, true);
                    e.SetParameters(&mut eff, DIEP_TYPESPECIFICPARAMS | DIEP_START)
                } else {
                    result
                };
                match result {
                    Ok(_) => d.ff_error_logged = false,
                    Err(err) if !d.ff_error_logged => {
                        log::warn!("{name}: force feedback effect could not be updated ({err})");
                        d.ff_error_logged = true;
                    }
                    Err(_) => {}
                }
            }
        }
        true
    }
}

impl Drop for DirectInput {
    fn drop(&mut self) {
        for d in &self.devices {
            unsafe {
                if let Some(e) = d.ff.as_ref() {
                    let _ = e.Stop();
                }
                let _ = d.dev.Unacquire();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The window that hears of devices plugged in or out opens (it is a thread's own).
    #[test]
    fn notification_window_opens() {
        let w = std::thread::spawn(|| notification_window().map(|w| unsafe { DestroyWindow(w).is_ok() })).join().unwrap();
        assert_eq!(w, Some(true));
    }
}
