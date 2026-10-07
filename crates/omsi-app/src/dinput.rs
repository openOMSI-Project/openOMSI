//! Game controllers on Windows through DirectInput 8, as Omsi.exe reads them: every device
//! Windows lists as a game controller (wheels with their makers' drivers, pedals, joysticks,
//! button boxes - the system's newer Windows.Gaming.Input misses many of them), the eight
//! axes in the slots `gamectrler.cfg` numbers (X, Y, Z, Rx, Ry, Rz, the two sliders), up to
//! 128 buttons, and force feedback: one constant force on the wheel's axis the game sets
//! every frame (its centring spring, the drag of the steering), and a sine on the same axis
//! for the scripts' shaking (`FF_Vib_Amp`, `FF_Vib_Period`), which the wheel itself plays:
//! a rattle of a few milliseconds cannot be drawn into a force set once a frame.
//!
//! The list of devices is looked up on a thread of its own, when Windows says a HID device
//! (every game controller is one) was plugged in or out: with some drivers the lookup takes
//! a tenth of a second and stalls the reading of the devices even from another thread, so
//! the old look every 3 seconds made driving stutter.

use std::cell::{Cell, RefCell};
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
        RawState {
            axes: [0; 8],
            pov: [u32::MAX; 4],
            buttons: [0; 128],
        }
    }
}

const RANGE: i32 = 10_000;
const VJOY_HARDWARE_ID: (u16, u16) = (0x1234, 0xBEAD);
/// A failed read after reacquiring can be transient, but several in a row mean the
/// DirectInput object itself is stale. Reopen it instead of keeping its last state forever.
const READ_FAILURE_LIMIT: u8 = 3;

/// One device opened.
pub(crate) struct Device {
    pub name: String,
    guid: GUID,
    /// USB vendor and product from DirectInput's product GUID, when available.
    pub hardware_id: Option<(u16, u16)>,
    dev: IDirectInputDevice8W,
    path: Option<String>,
    retry_at: Option<Instant>,
    /// The slots the device has (an axis it lacks is not in the list).
    has_axis: [bool; 8],
    /// Hardware capability, also needed by the launcher which does not create effects.
    ff_capable: bool,
    /// Offset of the axis that can receive forces in our data format.
    ff_axis: u32,
    state: RawState,
    /// Whether `state` came from a successful read. Invalid state must never drive a bus.
    valid: bool,
    /// Consecutive Poll/GetDeviceState failures after the usual reacquire attempt.
    read_failures: u8,
    ff: Option<IDirectInputEffect>,
    ff_error_logged: bool,
    /// The periodic effect of the shaking, the magnitude and period last given it, and when
    /// (set at most 100 times a second, as the constant force).
    vib: Option<IDirectInputEffect>,
    vib_last: (u32, u32),
    vib_at: Option<Instant>,
    pub buttons: usize,
}

/// Take the device again (after the window left the front). DirectInput resets a force
/// feedback wheel when it is taken: its own centring came back on, and a G29 pulled itself
/// to the middle after the pause until mouse steering was switched on and off.
fn reacquire(dev: &IDirectInputDevice8W, ff: bool) -> bool {
    unsafe {
        let _ = dev.Unacquire();
        if ff {
            let mut ac = DIPROPDWORD {
                diph: DIPROPHEADER {
                    dwSize: std::mem::size_of::<DIPROPDWORD>() as u32,
                    dwHeaderSize: std::mem::size_of::<DIPROPHEADER>() as u32,
                    dwObj: 0,
                    dwHow: DIPH_DEVICE,
                },
                dwData: DIPROPAUTOCENTER_OFF,
            };
            let _ = dev.SetProperty(prop(9), &mut ac.diph);
        }
        dev.Acquire().is_ok()
    }
}

fn pov_dirs(pov: u32) -> [bool; 4] {
    if pov == u32::MAX || pov & 0xFFFF == 0xFFFF {
        return [false; 4];
    }
    let a = (pov % 36000) as i32;
    let near = |c: i32| {
        let d = (a - c).rem_euclid(36000);
        d.min(36000 - d) < 6750
    };
    [near(0), near(9000), near(18000), near(27000)]
}

/// Releases for everything DirectInput last reported as held. A disappearing device does
/// not send button-up events, otherwise a shifter, door button or parking brake can stay
/// pressed in the vehicle script after the hardware is gone.
fn state_release_events(name: &str, state: &RawState) -> Vec<(String, usize, bool)> {
    let mut out = Vec::new();
    for (b, value) in state.buttons.iter().enumerate() {
        if value & 0x80 != 0 {
            out.push((name.to_string(), b, false));
        }
    }
    for (hat, pov) in state.pov.iter().copied().enumerate() {
        for (dir, down) in pov_dirs(pov).into_iter().enumerate() {
            if down {
                out.push((
                    name.to_string(),
                    crate::controllers::HAT_BUTTONS + hat * 4 + dir,
                    false,
                ));
            }
        }
    }
    out
}

/// State snapshots have no chronological ordering between buttons. Release old gears
/// before selecting new ones, even when the new gear has the lower button number.
fn order_button_events(events: &mut [(String, usize, bool)]) {
    events.sort_by_key(|(_, _, down)| *down);
}

fn invalidate_state(
    name: &str,
    state: &mut RawState,
    valid: &mut bool,
    events: &mut Vec<(String, usize, bool)>,
) {
    events.extend(state_release_events(name, state));
    *state = RawState::default();
    *valid = false;
}

fn state_axes(state: &RawState, valid: bool, has_axis: &[bool; 8]) -> Vec<(usize, f32)> {
    if !valid {
        return Vec::new();
    }
    (0..8)
        .filter(|k| has_axis[*k])
        .map(|k| (k, state.axes[k] as f32 / RANGE as f32))
        .collect()
}

impl Device {
    fn invalidate(&mut self, events: &mut Vec<(String, usize, bool)>) {
        invalidate_state(&self.name, &mut self.state, &mut self.valid, events);
        self.vib_last = (0, 0);
        self.vib_at = None;
        unsafe {
            for effect in [self.ff.as_ref(), self.vib.as_ref()].into_iter().flatten() {
                let _ = effect.Stop();
            }
            let _ = self.dev.Unacquire();
        }
    }

    /// The axes the device has: (slot, value -1..1).
    pub fn axes(&self) -> Vec<(usize, f32)> {
        state_axes(&self.state, self.valid, &self.has_axis)
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
    found: Arc<Mutex<Option<ScanResult>>>,
    scan: mpsc::Sender<()>,
    /// Retry only failed opens; do not enumerate every healthy wheel on a timer.
    pending_open: Vec<(GUID, String, Instant, u32)>,
    /// Button changes since the last `poll`: (device, button, down).
    pub events: Vec<(String, usize, bool)>,
    last_force: Instant,
}

fn is_controller_device(dev_type: u32, usage_page: u16, usage: u16) -> bool {
    let primary = dev_type & 0xFF;
    if matches!(
        primary,
        DI8DEVTYPE_JOYSTICK
            | DI8DEVTYPE_GAMEPAD
            | DI8DEVTYPE_DRIVING
            | DI8DEVTYPE_FLIGHT
            | DI8DEVTYPE_1STPERSON
            | DI8DEVTYPE_DEVICECTRL
            | DI8DEVTYPE_SUPPLEMENTAL
    ) {
        return true;
    }
    if primary == DI8DEVTYPE_DEVICE {
        return match usage_page {
            // Generic Desktop: Joystick (0x04), Gamepad (0x05), Multi-axis controller (0x08)
            0x01 => matches!(usage, 0x04 | 0x05 | 0x08),
            // Simulation Controls (steering wheels, pedals, cockpits, flight controls)
            0x02 => true,
            // Sport Controls (0x04) or Game Controls (0x05)
            0x04 | 0x05 => true,
            _ => false,
        };
    }
    false
}

unsafe extern "system" fn collect(
    inst: *mut DIDEVICEINSTANCEW,
    out: *mut core::ffi::c_void,
) -> windows::core::BOOL {
    let v = &mut *(out as *mut Vec<(GUID, String)>);
    let inst = &*inst;
    if !is_controller_device(inst.dwDevType, inst.wUsagePage, inst.wUsage) {
        return windows::core::BOOL(DIENUM_CONTINUE as i32);
    }
    let end = inst
        .tszProductName
        .iter()
        .position(|c| *c == 0)
        .unwrap_or(inst.tszProductName.len());
    let name = String::from_utf16_lossy(&inst.tszProductName[..end])
        .trim()
        .to_string();
    if !v.iter().any(|(g, _)| *g == inst.guidInstance) {
        v.push((inst.guidInstance, name));
    }
    windows::core::BOOL(DIENUM_CONTINUE as i32)
}

fn create() -> Option<IDirectInput8W> {
    unsafe {
        let hinst: HINSTANCE = GetModuleHandleW(None).ok()?.into();
        let mut p: *mut core::ffi::c_void = std::ptr::null_mut();
        DirectInput8Create(
            hinst,
            DIRECTINPUT_VERSION,
            &IDirectInput8W::IID,
            &mut p,
            None,
        )
        .ok()?;
        (!p.is_null()).then(|| IDirectInput8W::from_raw(p))
    }
}

fn list(di: &IDirectInput8W) -> Vec<(GUID, String)> {
    let mut v: Vec<(GUID, String)> = Vec::new();
    unsafe {
        let _ = di.EnumDevices(
            DI8DEVCLASS_ALL,
            Some(collect),
            &mut v as *mut _ as *mut core::ffi::c_void,
            DIEDFL_ATTACHEDONLY,
        );
    }
    v
}

#[derive(Default)]
struct ScanResult {
    devices: Vec<(GUID, String)>,
    removed_paths: Vec<String>,
}

// Each DirectInput worker owns its notification window. Keep removals on that thread,
// so the launcher and game cannot consume each other's disconnect notifications.
thread_local! {
    static HID_REMOVALS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn removed_path(path: Option<&str>, removed: &[String]) -> bool {
    path.is_some_and(|p| removed.iter().any(|r| r.eq_ignore_ascii_case(p)))
}

/// The wait before the next attempt at a device that would not open: 1 s, doubling, at most 30 s.
fn retry_delay(tries: u32) -> Duration {
    Duration::from_secs((1u64 << tries.min(5)).min(30))
}

/// Poll must succeed before reading state, including on the single retry after Acquire.
/// The closures keep the lifecycle decision testable without manufacturing a COM handle.
fn read_with_recovery<T, E>(
    mut read: impl FnMut() -> Result<T, E>,
    mut acquire: impl FnMut() -> bool,
) -> Result<T, E> {
    match read() {
        Ok(state) => Ok(state),
        Err(error) => {
            if acquire() {
                read()
            } else {
                Err(error)
            }
        }
    }
}

/// How often Windows said a HID device came or went (see `notification_window`).
static HID_CHANGES: AtomicU64 = AtomicU64::new(0);

unsafe extern "system" fn notify_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_DEVICECHANGE && matches!(wp.0 as u32, DBT_DEVICEARRIVAL | DBT_DEVICEREMOVECOMPLETE)
    {
        if wp.0 as u32 == DBT_DEVICEREMOVECOMPLETE && lp.0 != 0 {
            let header = &*(lp.0 as *const DEV_BROADCAST_HDR);
            if header.dbch_devicetype == DBT_DEVTYP_DEVICEINTERFACE {
                let interface = lp.0 as *const DEV_BROADCAST_DEVICEINTERFACE_W;
                let offset = std::mem::offset_of!(DEV_BROADCAST_DEVICEINTERFACE_W, dbcc_name);
                let size = header.dbch_size as usize;
                if size > offset {
                    let chars = std::slice::from_raw_parts(
                        std::ptr::addr_of!((*interface).dbcc_name).cast::<u16>(),
                        (size - offset) / 2,
                    );
                    let end = chars.iter().position(|c| *c == 0).unwrap_or(chars.len());
                    let path = String::from_utf16_lossy(&chars[..end]);
                    HID_REMOVALS.with(|paths| paths.borrow_mut().push(path));
                }
            }
        }
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
        let wc = WNDCLASSW {
            lpfnWndProc: Some(notify_proc),
            hInstance: hinst,
            lpszClassName: class,
            ..Default::default()
        };
        // (0 when the class is there already - a second window of the launcher's)
        let _ = RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!(""),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(hinst),
            None,
        )
        .ok()?;
        let filter = DEV_BROADCAST_DEVICEINTERFACE_W {
            dbcc_size: std::mem::size_of::<DEV_BROADCAST_DEVICEINTERFACE_W>() as u32,
            dbcc_devicetype: DBT_DEVTYP_DEVICEINTERFACE.0,
            dbcc_classguid: GUID_DEVINTERFACE_HID,
            ..Default::default()
        };
        if RegisterDeviceNotificationW(
            HANDLE(hwnd.0),
            &filter as *const _ as *const core::ffi::c_void,
            DEVICE_NOTIFY_WINDOW_HANDLE,
        )
        .is_err()
        {
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

unsafe extern "system" fn collect_object(
    inst: *mut DIDEVICEOBJECTINSTANCEW,
    out: *mut core::ffi::c_void,
) -> windows::core::BOOL {
    let objects = &mut *(out as *mut Vec<InputObject>);
    let inst = &*inst;
    objects.push(InputObject {
        guid: inst.guidType,
        ty: inst.dwType,
        flags: inst.dwFlags,
    });
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
fn format_objects(objects: &[InputObject]) -> (Vec<DIOBJECTDATAFORMAT>, [bool; 8], Vec<u32>) {
    let mut objs = Vec::new();
    let mut has_axis = [false; 8];
    let mut ff_axes = Vec::new();
    let mut pov = 0;
    let mut buttons = [false; 128];
    for object in objects {
        let (offset, flags) = if object.ty & DIDFT_AXIS != 0 {
            let Some(slot) = axis_slot(object.guid, &has_axis) else {
                continue;
            };
            if has_axis[slot] {
                continue;
            }
            has_axis[slot] = true;
            let offset = (slot * 4) as u32;
            if object.flags & DIDOI_FFACTUATOR != 0 {
                ff_axes.push(offset);
            }
            (offset, DIDOI_ASPECTPOSITION)
        } else if object.ty & DIDFT_POV != 0 && pov < 4 {
            let offset = (32 + pov * 4) as u32;
            pov += 1;
            (offset, 0)
        } else if object.ty & DIDFT_BUTTON != 0 {
            let button = ((object.ty >> 8) & 0xFFFF) as usize;
            if button >= buttons.len() || buttons[button] {
                continue;
            }
            buttons[button] = true;
            let offset = (48 + button) as u32;
            (offset, 0)
        } else {
            continue;
        };
        // The exact instance number is already part of dwType, so the GUID is unnecessary
        // here and we do not keep pointers into the temporary enumeration buffer.
        objs.push(DIOBJECTDATAFORMAT {
            pguid: std::ptr::null(),
            dwOfs: offset,
            dwType: object.ty,
            dwFlags: flags,
        });
    }
    (objs, has_axis, ff_axes)
}

/// The axes (data offsets) a wheel's constant force is tried on, in order: the X axis first -
/// a wheel steers with it, as OMSI's forces assume - whether or not the driver marks it as a
/// force actuator, then the axes the driver marks. (The MOZA R5 reports its Y axis as its
/// first actuator: the forces went to an axis without a motor and the wheel stayed slack.)
fn force_axes(has_axis: &[bool; 8], actuators: &[u32]) -> Vec<u32> {
    let mut out = Vec::new();
    if has_axis[0] {
        out.push(0);
    }
    for &a in actuators {
        if !out.contains(&a) {
            out.push(a);
        }
    }
    if out.is_empty() {
        out.push(0);
    }
    out
}

fn data_format(
    dev: &IDirectInputDevice8W,
) -> Option<(
    Vec<DIOBJECTDATAFORMAT>,
    DIDATAFORMAT,
    [bool; 8],
    Vec<u32>,
)> {
    let mut objects = Vec::new();
    unsafe {
        dev.EnumObjects(
            Some(collect_object),
            &mut objects as *mut _ as *mut core::ffi::c_void,
            DIDFT_ALL,
        )
        .ok()?;
    }
    let (objs, has_axis, ff_axes) = format_objects(&objects);
    if objs.is_empty() {
        return None;
    }
    let dw_flags = if has_axis.iter().any(|&a| a) {
        DIDF_ABSAXIS
    } else {
        0
    };
    let f = DIDATAFORMAT {
        dwSize: std::mem::size_of::<DIDATAFORMAT>() as u32,
        dwObjSize: std::mem::size_of::<DIOBJECTDATAFORMAT>() as u32,
        dwFlags: dw_flags,
        dwDataSize: std::mem::size_of::<RawState>() as u32,
        dwNumObjs: objs.len() as u32,
        rgodf: std::ptr::null_mut(),
    };
    Some((objs, f, has_axis, ff_axes))
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
        self.devices
            .iter()
            .find(|d| d.name == name && d.ff.is_some())
            .map(|d| d.ff_axis as usize / 4)
    }

    /// `hwnd`: the window the devices belong to; `ff`: take the devices for force feedback
    /// (the game's window: they then answer only while it is in front, as in OMSI).
    pub fn new(hwnd: isize, ff: bool) -> Option<DirectInput> {
        let di = create()?;
        let first = list(&di);
        let initially_empty = first.is_empty();
        log::info!(
            "game controllers (DirectInput): {}",
            if first.is_empty() {
                "none".to_string()
            } else {
                first
                    .iter()
                    .map(|d| d.1.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        );
        let found = Arc::new(Mutex::new(Some(ScanResult {
            devices: first,
            removed_paths: Vec::new(),
        })));
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
            let mut empty = initially_empty;
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
                } else if empty && last.elapsed() > Duration::from_secs(5) {
                    // Some older wheel drivers appear after the first enumeration
                    // without sending a HID arrival notification.
                    pending = Some(Instant::now() - Duration::from_secs(1));
                }
                if pending.is_some_and(|t| t.elapsed() > Duration::from_millis(300)) {
                    pending = None;
                    last = Instant::now();
                    let v = list(&di);
                    empty = v.is_empty();
                    log::info!("game controllers found: {}", v.iter().map(|d| d.1.as_str()).collect::<Vec<_>>().join(", "));
                    let mut found = f2.lock().unwrap();
                    // A second enumeration must not overwrite a removal before poll consumes it.
                    let mut removed_paths = found.take().map(|r| r.removed_paths).unwrap_or_default();
                    HID_REMOVALS.with(|paths| removed_paths.append(&mut paths.borrow_mut()));
                    *found = Some(ScanResult { devices: v, removed_paths });
                }
            }
        });
        Some(DirectInput {
            di,
            hwnd: HWND(hwnd as *mut _),
            ff,
            focused: true,
            devices: Vec::new(),
            found,
            scan,
            pending_open: Vec::new(),
            events: Vec::new(),
            last_force: Instant::now(),
        })
    }

    /// Let go of foreground effects when the game loses focus, then take them once on return.
    pub fn set_focus(&mut self, focused: bool) {
        if self.focused == focused {
            return;
        }
        self.focused = focused;
        log::info!(
            "game controllers: window {} focus; {} DirectInput device(s)",
            if focused { "regained" } else { "lost" },
            self.devices.len()
        );
        let mut reopen = Vec::new();
        for d in &mut self.devices {
            unsafe {
                if focused {
                    d.valid = false;
                    d.read_failures = 0;
                    d.retry_at = None;
                    if !reacquire(&d.dev, d.ff.is_some()) {
                        log::warn!("{}: DirectInput could not reacquire the device after focus returned; reopening it", d.name);
                        reopen.push((d.guid, d.name.clone()));
                    }
                    d.ff_error_logged = false;
                } else {
                    self.events.extend(state_release_events(&d.name, &d.state));
                    d.state = RawState::default();
                    d.valid = false;
                    d.read_failures = 0;
                    if let Some(e) = d.ff.as_ref() {
                        let _ = e.Stop();
                    }
                    // (the shaking is started again once the focus is back)
                    if let Some(e) = d.vib.as_ref() {
                        let _ = e.Stop();
                    }
                    d.vib_last = (0, 0);
                    let _ = d.dev.Unacquire();
                }
            }
        }
        if focused {
            self.reopen_devices(reopen);
        }
    }

    /// Throw away stale DirectInput objects and create them again. Reusing a GUID is not
    /// enough to prove the old COM object survived a USB/driver reset.
    fn reopen_devices(&mut self, reopen: Vec<(GUID, String)>) {
        if reopen.is_empty() {
            return;
        }
        for d in self
            .devices
            .iter_mut()
            .filter(|d| reopen.iter().any(|(guid, _)| *guid == d.guid))
        {
            d.invalidate(&mut self.events);
            log::warn!(
                "{}: DirectInput stale device; discarding and reopening",
                d.name
            );
        }
        self.devices
            .retain(|d| !reopen.iter().any(|(guid, _)| *guid == d.guid));
        let mut rescan = false;
        for (guid, name) in reopen {
            match self.open(&guid, &name, false) {
                Some(d) => {
                    log::info!("{name}: DirectInput device reopened");
                    self.devices.push(d);
                }
                None => {
                    log::warn!("{name}: DirectInput device could not be reopened; asking Windows to enumerate controllers again");
                    if !self.pending_open.iter().any(|(g, ..)| *g == guid) {
                        self.pending_open.push((guid, name, Instant::now() + retry_delay(0), 1));
                    }
                    rescan = true;
                }
            }
        }
        if rescan {
            let _ = self.scan.send(());
        }
    }

    /// Look for devices again (the window heard of one plugged in or out).
    pub fn refresh(&self) {
        let _ = self.scan.send(());
    }

    /// `retry`: a delayed attempt after a failed one, whose failure was already reported (its
    /// messages go to the debug log).
    fn open(&self, guid: &GUID, name: &str, retry: bool) -> Option<Device> {
        let warn = if retry { log::Level::Debug } else { log::Level::Warn };
        unsafe {
            let mut dev: Option<IDirectInputDevice8W> = None;
            self.di.CreateDevice(guid, &mut dev, None).ok()?;
            let dev = dev?;
            let mut info = DIDEVICEINSTANCEW {
                dwSize: std::mem::size_of::<DIDEVICEINSTANCEW>() as u32,
                ..Default::default()
            };
            let hardware_id = dev.GetDeviceInfo(&mut info).ok().and_then(|_| {
                let vid = info.guidProduct.data1 as u16;
                let pid = (info.guidProduct.data1 >> 16) as u16;
                (vid != 0 && pid != 0).then_some((vid, pid))
            });
            let mut identity = DIPROPGUIDANDPATH {
                diph: DIPROPHEADER {
                    dwSize: std::mem::size_of::<DIPROPGUIDANDPATH>() as u32,
                    dwHeaderSize: std::mem::size_of::<DIPROPHEADER>() as u32,
                    dwObj: 0,
                    dwHow: DIPH_DEVICE,
                },
                ..Default::default()
            };
            let path = dev
                .GetProperty(prop(12), &mut identity.diph)
                .ok()
                .and_then(|_| {
                    let end = identity
                        .wszPath
                        .iter()
                        .position(|c| *c == 0)
                        .unwrap_or(identity.wszPath.len());
                    (end != 0).then(|| String::from_utf16_lossy(&identity.wszPath[..end]))
                });
            let id = hardware_id
                .map(|(vid, pid)| format!("{vid:04X}/{pid:04X}"))
                .unwrap_or_else(|| "unknown".into());
            log::log!(
                if retry { log::Level::Debug } else { log::Level::Info },
                "{name}: DirectInput identity VID/PID {id}, instance {guid:?}, HID path {path:?}"
            );
            let Some((mut objs, mut fmt, has_axis, ff_axes)) = data_format(&dev) else {
                log::log!(warn, "{name}: DirectInput could not list the device's controls");
                return None;
            };
            fmt.rgodf = objs.as_mut_ptr();
            if let Err(e) = dev.SetDataFormat(&mut fmt) {
                log::log!(warn, "{name}: DirectInput rejected the device's data format ({e})");
                return None;
            }
            let mut caps = DIDEVCAPS {
                dwSize: std::mem::size_of::<DIDEVCAPS>() as u32,
                ..Default::default()
            };
            let _ = dev.GetCapabilities(&mut caps);
            let ff_capable = caps.dwFlags & DIDC_FORCEFEEDBACK != 0;
            let mut wants_ff = self.ff && ff_capable;
            let level = if wants_ff {
                DISCL_EXCLUSIVE | DISCL_FOREGROUND
            } else {
                DISCL_NONEXCLUSIVE | DISCL_BACKGROUND
            };
            if let Err(e) = dev.SetCooperativeLevel(self.hwnd, level) {
                if wants_ff {
                    // (forces need the device to themselves: another program - the wheel's
                    // own control software - may be holding it)
                    log::warn!("{name}: force feedback needs the device to itself, which Windows refused ({e}): no forces");
                    wants_ff = false;
                }
                dev.SetCooperativeLevel(self.hwnd, DISCL_NONEXCLUSIVE | DISCL_BACKGROUND)
                    .ok()?;
            }
            // every axis from -RANGE to RANGE
            let mut range = DIPROPRANGE {
                diph: DIPROPHEADER {
                    dwSize: std::mem::size_of::<DIPROPRANGE>() as u32,
                    dwHeaderSize: std::mem::size_of::<DIPROPHEADER>() as u32,
                    dwObj: 0,
                    dwHow: DIPH_DEVICE,
                },
                lMin: -RANGE,
                lMax: RANGE,
            };
            let _ = dev.SetProperty(prop(4), &mut range.diph);
            // no dead zone and no saturation of the driver's own (DIPROP_DEADZONE 5,
            // DIPROP_SATURATION 6): some wheels' drivers set one - a PXN V99 lost 30 % of
            // its turn round the middle - and OMSI clears them as well; the settings'
            // dead zone is the only one
            for (p_id, v) in [(5usize, 0u32), (6, 10_000)] {
                let mut d = DIPROPDWORD {
                    diph: DIPROPHEADER {
                        dwSize: std::mem::size_of::<DIPROPDWORD>() as u32,
                        dwHeaderSize: std::mem::size_of::<DIPROPHEADER>() as u32,
                        dwObj: 0,
                        dwHow: DIPH_DEVICE,
                    },
                    dwData: v,
                };
                let _ = dev.SetProperty(prop(p_id), &mut d.diph);
            }
            let candidates = force_axes(&has_axis, &ff_axes);
            let mut ff_axis = candidates[0];
            let mut ff = None;
            let mut vib = None;
            if wants_ff {
                // the wheel's own centring off: the game's forces take its place
                let mut ac = DIPROPDWORD {
                    diph: DIPROPHEADER {
                        dwSize: std::mem::size_of::<DIPROPDWORD>() as u32,
                        dwHeaderSize: std::mem::size_of::<DIPROPHEADER>() as u32,
                        dwObj: 0,
                        dwHow: DIPH_DEVICE,
                    },
                    dwData: DIPROPAUTOCENTER_OFF,
                };
                if let Err(err) = dev.SetProperty(prop(9), &mut ac.diph) {
                    log::warn!("{name}: autocenter could not be disabled ({err})");
                }
            }
            let _ = dev.Acquire();
            if wants_ff {
                let mut dirs = [0i32; 1];
                let mut cf = DICONSTANTFORCE { lMagnitude: 0 };
                let mut refused = None;
                for &axis in &candidates {
                    let mut axes = [axis; 1];
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
                        Ok(()) if e.is_some() => {
                            if let Some(e) = e.as_ref() {
                                if let Err(err) = e.Start(1, 0) {
                                    log::warn!("{name}: force feedback effect could not start ({err})");
                                }
                            }
                            ff = e;
                            ff_axis = axis;
                            break;
                        }
                        Ok(()) => {}
                        Err(err) => refused = Some(err),
                    }
                }
                if ff.is_none() {
                    match refused {
                        Some(err) => log::warn!("{name}: says it has force feedback, but its constant force could not be made ({err}): no forces"),
                        None => log::warn!("{name}: says it has force feedback, but its constant force could not be made: no forces"),
                    }
                }
                let mut axes = [ff_axis; 1];
                let mut eff = DIEFFECT {
                    dwSize: std::mem::size_of::<DIEFFECT>() as u32,
                    dwFlags: DIEFF_CARTESIAN | DIEFF_OBJECTOFFSETS,
                    dwDuration: u32::MAX, // INFINITE
                    dwGain: DI_FFNOMINALMAX,
                    dwTriggerButton: DIEB_NOTRIGGER,
                    cAxes: 1,
                    rgdwAxes: axes.as_mut_ptr(),
                    rglDirection: dirs.as_mut_ptr(),
                    ..Default::default()
                };
                if ff.is_some() && hardware_id != Some(VJOY_HARDWARE_ID) {
                    let mut pf = DIPERIODIC {
                        dwMagnitude: 0,
                        lOffset: 0,
                        dwPhase: 0,
                        dwPeriod: 100_000,
                    };
                    eff.cbTypeSpecificParams = std::mem::size_of::<DIPERIODIC>() as u32;
                    eff.lpvTypeSpecificParams = &mut pf as *mut _ as *mut core::ffi::c_void;
                    let mut e: Option<IDirectInputEffect> = None;
                    match dev.CreateEffect(&GUID_Sine, &mut eff, &mut e, None) {
                        Ok(()) => vib = e,
                        Err(err) => log::warn!("{name}: the shaking's periodic effect could not be made ({err}): it is part of the constant force"),
                    }
                }
            }
            log::info!(
                "game controller (DirectInput): {name}, {} axes, {} buttons{}",
                has_axis.iter().filter(|a| **a).count(),
                caps.dwButtons,
                if ff.is_some() {
                    format!(", force feedback on axis {}", ff_axis / 4)
                } else if ff_capable && self.ff {
                    ", force feedback capable (effect unavailable)".into()
                } else if ff_capable {
                    ", force feedback capable".into()
                } else {
                    String::new()
                }
            );
            Some(Device {
                name: name.to_string(),
                guid: *guid,
                hardware_id,
                dev,
                path,
                retry_at: None,
                has_axis,
                ff_capable,
                ff_axis,
                state: RawState::default(),
                valid: false,
                read_failures: 0,
                ff,
                ff_error_logged: false,
                vib,
                vib_last: (0, 0),
                vib_at: None,
                buttons: caps.dwButtons as usize,
            })
        }
    }

    /// Read every device; devices plugged in or out since the last list are opened or let go.
    pub fn poll(&mut self) {
        if !self.focused {
            // Releases queued by set_focus(false) still have to reach the vehicle.
            return;
        }
        let found = self.found.lock().unwrap().take();
        if let Some(found) = found {
            let list = found.devices;
            self.pending_open
                .retain(|(g, ..)| list.iter().any(|(current, _)| g == current));
            for d in &mut self.devices {
                if !list.iter().any(|(g, _)| *g == d.guid)
                    || removed_path(d.path.as_deref(), &found.removed_paths)
                {
                    log::info!(
                        "{}: DirectInput device removed/re-enumerated; discarding old instance",
                        d.name
                    );
                    d.invalidate(&mut self.events);
                }
            }
            self.devices.retain(|d| {
                list.iter().any(|(g, _)| *g == d.guid)
                    && !removed_path(d.path.as_deref(), &found.removed_paths)
            });
            for (g, name) in list {
                if !self.devices.iter().any(|d| d.guid == g) {
                    match self.open(&g, &name, false) {
                        Some(d) => {
                            log::info!("{name}: DirectInput device opened after enumeration");
                            self.devices.push(d);
                        }
                        None => {
                            log::debug!(
                                "DirectInput device {name}: could not be opened; delaying retry"
                            );
                            if !self
                                .pending_open
                                .iter()
                                .any(|(pending, ..)| *pending == g)
                            {
                                self.pending_open.push((g, name, Instant::now() + retry_delay(0), 1));
                            }
                        }
                    }
                }
            }
        }
        let pending = std::mem::take(&mut self.pending_open);
        for (guid, name, at, tries) in pending {
            if self.devices.iter().any(|d| d.guid == guid) {
                continue;
            }
            if Instant::now() < at {
                self.pending_open.push((guid, name, at, tries));
            } else if let Some(d) = self.open(&guid, &name, true) {
                log::info!(
                    "{name}: DirectInput device reopened after delayed driver initialization"
                );
                self.devices.push(d);
            } else {
                // 1 s, 2 s, 4 s ... up to 30 s; a device plugged in or out starts over
                self.pending_open
                    .push((guid, name, Instant::now() + retry_delay(tries), tries + 1));
            }
        }
        let mut reopen = Vec::new();
        for d in &mut self.devices {
            if d.retry_at.is_some_and(|at| Instant::now() < at) {
                continue;
            }
            let log_failure = d.read_failures == 0;
            let read = || unsafe {
                if let Err(error) = d.dev.Poll() {
                    if log_failure {
                        log::warn!("{}: DirectInput Poll failed ({error})", d.name);
                    }
                    return Err(error);
                }
                let mut state = RawState::default();
                let result = d.dev.GetDeviceState(
                    std::mem::size_of::<RawState>() as u32,
                    &mut state as *mut _ as *mut core::ffi::c_void,
                );
                if let Err(error) = &result {
                    if log_failure {
                        log::warn!("{}: DirectInput GetDeviceState failed ({error})", d.name);
                    }
                }
                result.map(|_| state)
            };
            let attempted = Cell::new(false);
            let result = read_with_recovery(read, || {
                attempted.set(true);
                if log_failure {
                    log::info!("{}: DirectInput attempting reacquire", d.name);
                }
                let acquired = reacquire(&d.dev, d.ff.is_some());
                if log_failure {
                    log::info!(
                        "{}: DirectInput reacquire {}",
                        d.name,
                        if acquired { "succeeded" } else { "failed" }
                    );
                }
                acquired
            });
            if attempted.get() {
                d.vib_last = (0, 0);
                d.vib_at = None;
            }
            let s = match result {
                Ok(state) => state,
                Err(_) => {
                    // Release immediately, not after the stale-object threshold. No cached
                    // button, hat, axis or effect may remain active during recovery.
                    d.invalidate(&mut self.events);
                    d.read_failures = d.read_failures.saturating_add(1);
                    d.retry_at = Some(Instant::now() + Duration::from_millis(250));
                    if d.read_failures >= READ_FAILURE_LIMIT {
                        reopen.push((d.guid, d.name.clone()));
                    }
                    continue;
                }
            };
            if d.read_failures != 0 {
                log::info!("{}: DirectInput valid state recovered", d.name);
            }
            d.retry_at = None;
            d.read_failures = 0;
            d.valid = true;
            for b in 0..128 {
                let (was, now) = (d.state.buttons[b] & 0x80 != 0, s.buttons[b] & 0x80 != 0);
                if was != now {
                    self.events.push((d.name.clone(), b, now));
                }
            }
            // the hat switches as buttons after the 128 (up, right, down, left of each): the
            // D-pad of a wheel rim - Moza's among others - is a hat, and could not be given a key
            for k in 0..4 {
                let (was, now) = (pov_dirs(d.state.pov[k]), pov_dirs(s.pov[k]));
                for dir in 0..4 {
                    if was[dir] != now[dir] {
                        self.events.push((
                            d.name.clone(),
                            crate::controllers::HAT_BUTTONS + k * 4 + dir,
                            now[dir],
                        ));
                    }
                }
            }
            d.state = s;
        }
        self.reopen_devices(reopen);
        order_button_events(&mut self.events);
    }

    /// A hardware-timed calibration pulse; never leaves an infinite force running.
    pub(crate) fn pulse_force(&mut self, name: &str, force: f32) -> bool {
        if !self.focused
            || self
                .devices
                .iter()
                .filter(|d| d.name == name && d.valid && d.ff.is_some())
                .count()
                != 1
        {
            return false;
        }
        let Some(device) = self
            .devices
            .iter_mut()
            .find(|d| d.name == name && d.ff.is_some())
        else {
            return false;
        };
        let limit = crate::ffb_calibration::MAX_PULSE_FORCE;
        let mut constant = DICONSTANTFORCE {
            lMagnitude: (force.clamp(-limit, limit) * DI_FFNOMINALMAX as f32) as i32,
        };
        let mut axes = [device.ff_axis];
        let mut direction = [0i32];
        let mut effect = DIEFFECT {
            dwSize: std::mem::size_of::<DIEFFECT>() as u32,
            dwFlags: DIEFF_CARTESIAN | DIEFF_OBJECTOFFSETS,
            dwDuration: crate::ffb_calibration::PULSE_MS * 1000,
            cAxes: 1,
            rgdwAxes: axes.as_mut_ptr(),
            rglDirection: direction.as_mut_ptr(),
            cbTypeSpecificParams: std::mem::size_of::<DICONSTANTFORCE>() as u32,
            lpvTypeSpecificParams: &mut constant as *mut _ as *mut core::ffi::c_void,
            ..Default::default()
        };
        unsafe {
            let force_effect = device.ff.as_ref().unwrap();
            // Some drivers only allow a duration change while the effect is stopped.
            let result = force_effect.Stop().and_then(|_| {
                force_effect.SetParameters(
                    &mut effect,
                    DIEP_DURATION | DIEP_TYPESPECIFICPARAMS | DIEP_START,
                )
            });
            if let Err(error) = result {
                log::warn!("{name}: force feedback calibration pulse failed ({error})");
                return false;
            }
            true
        }
    }

    /// The force on the wheel of device `name`: -1 (full to the left) .. 1. Set at most 100
    /// times a second (each is a message to the device), which is also the rate that fixes
    /// how fast a vibration can be: past about half of it a tremble comes out of the motor
    /// as a beat of its own, so `crate::controllers` keeps the tarmac's grain and the
    /// engine's buzz below it.
    /// Returns whether a force-feedback effect with this exact device name exists.
    pub fn set_force(&mut self, name: &str, f: f32) -> bool {
        let found = self
            .devices
            .iter()
            .any(|d| d.name == name && d.ff.is_some());
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
        for d in self
            .devices
            .iter_mut()
            .filter(|d| d.name == name && d.valid)
        {
            let Some(e) = d.ff.as_ref() else { continue };
            let mut axes = [d.ff_axis; 1];
            let mut dirs = [0i32; 1];
            let mut cf = DICONSTANTFORCE {
                lMagnitude: (f.clamp(-1.0, 1.0) * DI_FFNOMINALMAX as f32) as i32,
            };
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

    /// The shaking on the wheel of device `name`: `amp` 0..1 of the full force, `period` as
    /// `FF_Vib_Period` (OMSI hands DirectInput Round(period × 10000) µs). Returns whether
    /// the device plays it as an effect of its own.
    pub fn set_vibration(&mut self, name: &str, amp: f32, period: f32) -> bool {
        let found = self
            .devices
            .iter()
            .any(|d| d.name == name && d.vib.is_some());
        if !found || !self.focused {
            return found;
        }
        let magnitude = (amp.clamp(0.0, 1.0) * DI_FFNOMINALMAX as f32) as u32;
        let period_us = if period.is_finite() {
            (period.max(0.0) * 10_000.0).round().min(u32::MAX as f32) as u32
        } else {
            0
        };
        let period_us = if magnitude == 0 { 0 } else { period_us.max(1) };
        for d in self
            .devices
            .iter_mut()
            .filter(|d| d.name == name && d.valid)
        {
            let Some(e) = d.vib.as_ref() else { continue };
            let switching = (d.vib_last.0 == 0) != (magnitude == 0);
            if d.vib_last == (magnitude, period_us)
                || (!switching
                    && d.vib_at
                        .is_some_and(|t| t.elapsed() < Duration::from_millis(10)))
            {
                continue;
            }
            let mut axes = [d.ff_axis; 1];
            let mut dirs = [0i32; 1];
            let mut pf = DIPERIODIC {
                dwMagnitude: magnitude,
                lOffset: 0,
                dwPhase: 0,
                dwPeriod: period_us.max(1),
            };
            let mut eff = DIEFFECT {
                dwSize: std::mem::size_of::<DIEFFECT>() as u32,
                dwFlags: DIEFF_CARTESIAN | DIEFF_OBJECTOFFSETS,
                cAxes: 1,
                rgdwAxes: axes.as_mut_ptr(),
                rglDirection: dirs.as_mut_ptr(),
                cbTypeSpecificParams: std::mem::size_of::<DIPERIODIC>() as u32,
                lpvTypeSpecificParams: &mut pf as *mut _ as *mut core::ffi::c_void,
                ..Default::default()
            };
            unsafe {
                let result = if magnitude == 0 {
                    e.Stop()
                } else {
                    // (DIEP_START restarts a playing sine from its phase 0: only when it
                    // starts, or an amplitude changing every frame cut it to its first 10 ms)
                    let flags = if switching {
                        DIEP_TYPESPECIFICPARAMS | DIEP_START
                    } else {
                        DIEP_TYPESPECIFICPARAMS
                    };
                    let r = e.SetParameters(&mut eff, flags);
                    if r.is_err() {
                        reacquire(&d.dev, true);
                        e.SetParameters(&mut eff, DIEP_TYPESPECIFICPARAMS | DIEP_START)
                    } else {
                        r
                    }
                };
                if result.is_ok() {
                    d.vib_last = (magnitude, period_us);
                    d.vib_at = Some(Instant::now());
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
                for e in [d.ff.as_ref(), d.vib.as_ref()].into_iter().flatten() {
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

    #[test]
    fn failed_opens_back_off_to_half_a_minute() {
        let secs: Vec<u64> = (0..8).map(|t| retry_delay(t).as_secs()).collect();
        assert_eq!(secs, [1, 2, 4, 8, 16, 30, 30, 30]);
    }

    #[test]
    fn disappearing_device_releases_buttons_and_hats() {
        let mut state = RawState::default();
        state.buttons[7] = 0x80;
        state.pov[1] = 9000;
        let events = state_release_events("wheel", &state);
        assert!(events.contains(&("wheel".to_string(), 7, false)));
        assert!(events.contains(&(
            "wheel".to_string(),
            crate::controllers::HAT_BUTTONS + 5,
            false
        )));
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn poll_failure_never_reads_stale_state_and_retries_after_acquire() {
        let calls = RefCell::new(Vec::new());
        let mut attempt = 0;
        let result = read_with_recovery(
            || {
                calls.borrow_mut().push("poll");
                attempt += 1;
                if attempt == 1 {
                    return Err("poll lost");
                }
                calls.borrow_mut().push("state");
                Ok(42)
            },
            || {
                calls.borrow_mut().push("acquire");
                true
            },
        );
        assert_eq!(result, Ok(42));
        assert_eq!(*calls.borrow(), ["poll", "acquire", "poll", "state"]);
    }

    #[test]
    fn state_failure_and_poll_failure_share_bounded_recovery() {
        for stage in ["poll", "state"] {
            let mut reads = 0;
            let mut acquires = 0;
            let result: Result<(), _> = read_with_recovery(
                || {
                    reads += 1;
                    Err(stage)
                },
                || {
                    acquires += 1;
                    true
                },
            );
            assert_eq!(result, Err(stage));
            assert_eq!((reads, acquires), (2, 1));
        }
        let mut reads = 0;
        let result: Result<(), _> = read_with_recovery(
            || {
                reads += 1;
                Err("lost")
            },
            || false,
        );
        assert_eq!(result, Err("lost"));
        assert_eq!(reads, 1);
    }

    #[test]
    fn hotplug_targets_exact_device_path_even_with_the_same_product() {
        let removed = vec!["hid#vid_046d&pid_c29b#first".to_string()];
        assert!(removed_path(Some("HID#VID_046D&PID_C29B#FIRST"), &removed));
        assert!(!removed_path(
            Some("hid#vid_046d&pid_c29b#second"),
            &removed
        ));
        assert!(!removed_path(Some("hid#keyboard"), &removed));
        assert!(!removed_path(None, &removed));
    }

    #[test]
    fn invalid_state_releases_shared_wheel_shifter_and_cannot_drive() {
        let mut state = RawState::default();
        state.axes[0] = RANGE;
        state.axes[1] = RANGE;
        state.buttons[0] = 0x80; // wheel rim
        state.buttons[8] = 0x80; // G27-style shifter in the same device
        state.buttons[127] = 0x80;
        state.pov[3] = 18000;
        let mut valid = true;
        let mut events = Vec::new();
        assert_eq!(state_axes(&state, valid, &[true; 8])[0], (0, 1.0));
        invalidate_state("G27", &mut state, &mut valid, &mut events);
        assert!(!valid);
        assert!(state_axes(&state, valid, &[true; 8]).is_empty());
        assert_eq!(events.len(), 4);
        for button in [0, 8, 127] {
            assert!(events.contains(&("G27".into(), button, false)));
        }
        invalidate_state("G27", &mut state, &mut valid, &mut events);
        assert_eq!(events.len(), 4); // no repeated releases
                                     // The first valid reconnect sample compares against an empty snapshot.
        assert_eq!(state.buttons[8], 0);
    }

    #[test]
    fn simultaneous_gate_change_releases_before_pressing_in_both_directions() {
        for (old, new) in [(1, 2), (2, 1)] {
            let mut events = vec![("wheel".into(), new, true), ("wheel".into(), old, false)];
            order_button_events(&mut events);
            assert_eq!(
                events,
                [("wheel".into(), old, false), ("wheel".into(), new, true)]
            );
        }
    }

    #[test]
    fn button_offsets_follow_instances_not_enumeration_order() {
        let objects = [7, 1, 7].map(|n| InputObject {
            guid: GUID_Button,
            ty: DIDFT_BUTTON | (n << 8),
            flags: 0,
        });
        let (format, _, _) = format_objects(&objects);
        assert_eq!(format.iter().map(|o| o.dwOfs).collect::<Vec<_>>(), [55, 49]);
    }

    /// The window that hears of devices plugged in or out opens (it is a thread's own).
    #[test]
    fn notification_window_opens() {
        let w = std::thread::spawn(|| {
            notification_window().map(|w| unsafe { DestroyWindow(w).is_ok() })
        })
        .join()
        .unwrap();
        assert_eq!(w, Some(true));
    }

    #[test]
    fn controller_device_filter_accepts_controllers_and_rejects_mice_keyboards_and_vendor_devices()
    {
        assert!(is_controller_device(DI8DEVTYPE_JOYSTICK, 0, 0));
        assert!(is_controller_device(DI8DEVTYPE_GAMEPAD, 0, 0));
        assert!(is_controller_device(DI8DEVTYPE_DRIVING, 0, 0));
        assert!(is_controller_device(DI8DEVTYPE_FLIGHT, 0, 0));
        assert!(is_controller_device(DI8DEVTYPE_DEVICECTRL, 0, 0));

        assert!(!is_controller_device(DI8DEVTYPE_KEYBOARD, 0, 0));
        assert!(!is_controller_device(DI8DEVTYPE_MOUSE, 0, 0));
        assert!(!is_controller_device(DI8DEVTYPE_SCREENPOINTER, 0, 0));

        // Generic devices (DI8DEVTYPE_DEVICE) with game usages (Arduino button boxes):
        assert!(is_controller_device(DI8DEVTYPE_DEVICE, 0x01, 0x04)); // Joystick
        assert!(is_controller_device(DI8DEVTYPE_DEVICE, 0x01, 0x05)); // Gamepad
        assert!(is_controller_device(DI8DEVTYPE_DEVICE, 0x01, 0x08)); // Multi-axis
        assert!(is_controller_device(DI8DEVTYPE_DEVICE, 0x02, 0x01)); // Simulation controls

        // Generic devices with consumer/vendor/power/undefined usages (Razer, Logitech mice/keyboards macro collections):
        assert!(!is_controller_device(DI8DEVTYPE_DEVICE, 0x0C, 0x01)); // Consumer / media keys
        assert!(!is_controller_device(DI8DEVTYPE_DEVICE, 0x01, 0x80)); // System control
        assert!(!is_controller_device(DI8DEVTYPE_DEVICE, 0x01, 0x00)); // Undefined
        assert!(!is_controller_device(DI8DEVTYPE_DEVICE, 0xFF00, 0x01)); // Vendor specific
    }

    /// A wheel's force goes to its X axis first, even where the driver lists another axis as
    /// its first actuator (the MOZA R5's Y); a device without an X axis gets its actuators.
    #[test]
    fn the_wheel_force_is_tried_on_the_x_axis_first() {
        let mut has = [false; 8];
        has[0] = true;
        has[1] = true;
        assert_eq!(force_axes(&has, &[4, 0]), vec![0, 4]);
        assert_eq!(force_axes(&has, &[4]), vec![0, 4]);
        assert_eq!(force_axes(&has, &[]), vec![0]);
        let only_z = [false, false, true, false, false, false, false, false];
        assert_eq!(force_axes(&only_z, &[8]), vec![8]);
    }

    #[test]
    fn button_box_without_axes_formats_successfully() {
        let objects: Vec<InputObject> = (0..16)
            .map(|n| InputObject {
                guid: GUID_Button,
                ty: DIDFT_BUTTON | (n << 8),
                flags: 0,
            })
            .collect();
        let (objs, has_axis, ff_axes) = format_objects(&objects);
        assert_eq!(objs.len(), 16);
        assert_eq!(has_axis, [false; 8]);
        assert!(ff_axes.is_empty());
        for (i, obj) in objs.iter().enumerate() {
            assert_eq!(obj.dwOfs, (48 + i) as u32);
        }
    }
}
