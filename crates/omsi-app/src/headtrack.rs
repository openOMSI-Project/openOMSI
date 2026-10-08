//! Head tracking: the head's pose from opentrack's "UDP over network" output (six little-
//! endian doubles - x, y, z in cm, yaw, pitch, roll in degrees - to UDP port 4242). opentrack
//! takes TrackIR, Tobii, webcams (neuralnet tracker) and phones, on every platform.
//!
//! On Windows openOMSI also reads NaturalPoint's native NPClient interface directly when
//! available, and keeps opentrack's `FT_SharedMem` output as a fallback. A recent UDP pose wins
//! over native TrackIR, which in turn wins over `FT_SharedMem`.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The last pose received, in opentrack's terms: x, y, z (cm; left, up, back) and yaw, pitch,
/// roll (degrees; yaw to the right, pitch up). (opentrack's own outputs to simulators that
/// count x to the right, FlightGear's and SimConnect's, send them `-x`.)
#[derive(Debug, Clone, Copy, Default)]
pub struct HeadPose {
    pub pos: [f32; 3],
    pub rot: [f32; 3],
}

impl HeadPose {
    /// How far the head moved the eye, in the bus's frame (m; right, forward, up). Omsi.exe
    /// (0x829860) moves its camera across by `-x` of the TrackIR pose, which opentrack fills
    /// with its own x: a head moved to the right took the camera to the left (#1157).
    pub fn seat_offset(&self) -> glam::Vec3 {
        glam::Vec3::new(-self.pos[0], -self.pos[2], self.pos[1]).clamp(glam::Vec3::splat(-60.0), glam::Vec3::splat(60.0)) / 100.0
    }
}

pub struct HeadTracker {
    last: Arc<Mutex<Option<(HeadPose, Instant)>>>,
}

impl HeadTracker {
    /// Listen on `port` (on every interface: opentrack may run on another machine).
    /// On Windows, `window_handle` is the native game window used by NPClient.
    pub fn start(port: u16, window_handle: Option<isize>) -> Option<HeadTracker> {
        #[cfg(not(windows))]
        let _ = window_handle;
        // (the port may be taken: opentrack's own "UDP over network" input listens on 4242
        // too, which is how FreePIE hands it a TrackIR's pose. On Windows the freetrack
        // mapping is still read then; elsewhere there is nothing to read.)
        let sock = match std::net::UdpSocket::bind(("0.0.0.0", port)) {
            Ok(s) => Some(s),
            Err(e) if cfg!(windows) => {
                log::warn!("head tracking: cannot listen on UDP port {port} ({e}), reading opentrack's freetrack output only");
                None
            }
            Err(e) => {
                log::warn!("head tracking: cannot listen on UDP port {port}: {e}");
                return None;
            }
        };
        // (on Windows the loop also polls the freetrack mapping between datagrams)
        let wait = if cfg!(windows) { 10 } else { 500 };
        if let Some(sock) = &sock {
            let _ = sock.set_read_timeout(Some(Duration::from_millis(wait)));
        }
        let listening = sock.is_some();
        let last: Arc<Mutex<Option<(HeadPose, Instant)>>> = Arc::default();
        let out = last.clone();
        std::thread::Builder::new()
            .name("head tracking".into())
            .spawn(move || {
                let mut buf = [0u8; 64];
                let mut announced = false;
                #[cfg(windows)]
                let mut freetrack = freetrack::Reader::default();
                #[cfg(windows)]
                let mut npclient = npclient::Reader::new(window_handle);
                #[cfg(windows)]
                let mut last_udp: Option<Instant> = None;
                loop {
                    // (the game's end takes the thread with it)
                    if Arc::strong_count(&out) == 1 {
                        return;
                    }
                    let got = match &sock {
                        Some(sock) => sock.recv(&mut buf),
                        None => {
                            std::thread::sleep(Duration::from_millis(wait));
                            Err(std::io::ErrorKind::WouldBlock.into())
                        }
                    };
                    let Ok(n) = got else {
                        #[cfg(windows)]
                        if last_udp.is_none_or(|t| t.elapsed() >= Duration::from_millis(500)) {
                            if let Some(pose) = npclient.poll().or_else(|| freetrack.poll()) {
                                *out.lock().unwrap() = Some((pose, Instant::now()));
                            }
                        }
                        continue;
                    };
                    if n < 48 {
                        continue;
                    }
                    let d = |i: usize| f64::from_le_bytes(buf[i * 8..i * 8 + 8].try_into().unwrap()) as f32;
                    let v = [d(0), d(1), d(2), d(3), d(4), d(5)];
                    if v.iter().any(|x| !x.is_finite()) {
                        continue;
                    }
                    if !announced {
                        announced = true;
                        log::info!("head tracking: receiving poses on UDP port {port}");
                    }
                    #[cfg(windows)]
                    {
                        last_udp = Some(Instant::now());
                    }
                    *out.lock().unwrap() = Some((HeadPose { pos: [v[0], v[1], v[2]], rot: [v[3], v[4], v[5]] }, Instant::now()));
                }
            })
            .ok()?;
        if listening {
            log::info!("head tracking: listening for opentrack on UDP port {port}");
        }
        Some(HeadTracker { last })
    }

    /// The pose, while poses keep coming (none for half a second: the head is centred).
    pub fn pose(&self) -> Option<HeadPose> {
        let l = self.last.lock().ok()?;
        l.filter(|(_, t)| t.elapsed() < Duration::from_millis(500)).map(|(p, _)| p)
    }
}

/// A freetrack pose as opentrack writes it into `FT_SharedMem` - yaw, pitch, roll in
/// radians with yaw and pitch negated, x, y, z in mm - in the UDP output's terms (cm and
/// degrees).
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn freetrack_pose(yaw: f32, pitch: f32, roll: f32, x: f32, y: f32, z: f32) -> HeadPose {
    HeadPose {
        pos: [x / 10.0, y / 10.0, z / 10.0],
        rot: [-yaw.to_degrees(), -pitch.to_degrees(), roll.to_degrees()],
    }
}

#[cfg(windows)]
mod freetrack {
    use super::{freetrack_pose, HeadPose};
    use std::time::{Duration, Instant};
    use windows::core::w;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Memory::{MapViewOfFile, OpenFileMappingW, FILE_MAP_READ};

    /// `FTData`: DataID, CamWidth, CamHeight (i32), Yaw, Pitch, Roll, X, Y, Z (f32), ...
    const FIELDS: usize = 9;

    /// The mapping, opened once opentrack has made it (looked for once a second).
    #[derive(Default)]
    pub struct Reader {
        view: Option<(HANDLE, *const u32)>,
        tried: Option<Instant>,
        last_id: Option<u32>,
        announced: bool,
    }

    // (the view is only read, from the head tracking thread)
    unsafe impl Send for Reader {}

    impl Reader {
        /// A new pose, when opentrack has written one since the last call.
        pub fn poll(&mut self) -> Option<HeadPose> {
            if self.view.is_none() {
                if self.tried.is_some_and(|t| t.elapsed() < Duration::from_secs(1)) {
                    return None;
                }
                self.tried = Some(Instant::now());
                // SAFETY: plain Win32 calls; the view stays mapped for the life of the game.
                unsafe {
                    let handle = OpenFileMappingW(FILE_MAP_READ.0, false, w!("FT_SharedMem")).ok()?;
                    let view = MapViewOfFile(handle, FILE_MAP_READ, 0, 0, FIELDS * 4);
                    if view.Value.is_null() {
                        let _ = windows::Win32::Foundation::CloseHandle(handle);
                        return None;
                    }
                    self.view = Some((handle, view.Value as *const u32));
                }
            }
            let (_, base) = self.view?;
            // SAFETY: the view spans the FIELDS words read here; opentrack writes them
            // from its own process, hence the volatile reads.
            let word = |i: usize| unsafe { base.add(i).read_volatile() };
            let id = word(0);
            if self.last_id.replace(id) == Some(id) {
                return None;
            }
            let f = |i: usize| f32::from_bits(word(i));
            let v = [f(3), f(4), f(5), f(6), f(7), f(8)];
            if v.iter().any(|x| !x.is_finite()) {
                return None;
            }
            if !self.announced {
                self.announced = true;
                log::info!("head tracking: receiving poses from freetrack (FT_SharedMem)");
            }
            Some(freetrack_pose(v[0], v[1], v[2], v[3], v[4], v[5]))
        }
    }
}


#[cfg(windows)]
mod npclient {
    use super::HeadPose;
    use std::ffi::c_void;
    use std::mem::transmute_copy;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};
    use windows::core::{s, w, PCWSTR};
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    use windows::Win32::System::Registry::{
        RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ,
    };

    type NpResult = i32;

    // NaturalPoint's public NPClient pose structure. The layout is 60 bytes:
    // two u16 fields, one u32 field, followed by fifteen f32 values.
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct TrackIrData {
        w_np_status: u16,
        w_p_frame_signature: u16,
        dw_np_io_data: u32,
        f_np_roll: f32,
        f_np_pitch: f32,
        f_np_yaw: f32,
        f_np_x: f32,
        f_np_y: f32,
        f_np_z: f32,
        f_np_raw_x: f32,
        f_np_raw_y: f32,
        f_np_raw_z: f32,
        f_np_delta_x: f32,
        f_np_delta_y: f32,
        f_np_delta_z: f32,
        f_np_smooth_x: f32,
        f_np_smooth_y: f32,
        f_np_smooth_z: f32,
    }

    type RegisterWindowHandle = unsafe extern "system" fn(*mut c_void) -> NpResult;
    type UnregisterWindowHandle = unsafe extern "system" fn() -> NpResult;
    type RegisterProgramProfileId = unsafe extern "system" fn(u16) -> NpResult;
    type QueryVersion = unsafe extern "system" fn(*mut u16) -> NpResult;
    type RequestData = unsafe extern "system" fn(u16) -> NpResult;
    type GetData = unsafe extern "system" fn(*mut TrackIrData) -> NpResult;
    type StartDataTransmission = unsafe extern "system" fn() -> NpResult;
    type StopDataTransmission = unsafe extern "system" fn() -> NpResult;

    const NP_YAW: u16 = 0x0001;
    const NP_PITCH: u16 = 0x0002;
    const NP_ROLL: u16 = 0x0004;
    const NP_X: u16 = 0x0008;
    const NP_Y: u16 = 0x0010;
    const NP_Z: u16 = 0x0020;
    const NP_REQUEST: u16 = NP_YAW | NP_PITCH | NP_ROLL | NP_X | NP_Y | NP_Z;

    // This developer profile ID is accepted by current TrackIR 5.4 installations
    // and is sufficient for direct NPClient polling.
    const PROFILE_ID: u16 = 1001;

    struct Native {
        _dll: HMODULE,
        unregister_window: UnregisterWindowHandle,
        get_data: GetData,
        stop_transmission: StopDataTransmission,
        last_frame: Option<u16>,
    }

    impl Drop for Native {
        fn drop(&mut self) {
            unsafe {
                let _ = (self.stop_transmission)();
                let _ = (self.unregister_window)();
            }
        }
    }

    /// Direct reader for NaturalPoint's NPClient64.dll.
    ///
    /// This is deliberately kept private to the Windows head-tracking thread:
    /// the vendor DLL owns process-global state and is not something the rest of
    /// openOMSI should have to know about.
    pub struct Reader {
        native: Option<Native>,
        hwnd: Option<isize>,
        tried: Option<Instant>,
        announced: bool,
        native_enabled: bool,
    }

    impl Reader {
        pub fn new(hwnd: Option<isize>) -> Self {
            let native_enabled = std::env::var_os("OMSI_TRACKIR_NATIVE")
                .map(|v| v != "0")
                .unwrap_or(true);
            if !native_enabled {
                log::info!("head tracking: native TrackIR NPClient disabled by OMSI_TRACKIR_NATIVE=0");
            }
            Self {
                native: None,
                hwnd,
                tried: None,
                announced: false,
                native_enabled,
            }
        }

        fn dll_path() -> Option<PathBuf> {
            let mut bytes = vec![0u8; 4096];
            let mut kind = windows::Win32::System::Registry::REG_VALUE_TYPE::default();
            let mut len = bytes.len() as u32;

            let result = unsafe {
                RegGetValueW(
                    HKEY_CURRENT_USER,
                    w!("Software\\NaturalPoint\\NATURALPOINT\\NPClient Location"),
                    w!("Path"),
                    RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ,
                    Some(&mut kind),
                    Some(bytes.as_mut_ptr() as *mut c_void),
                    Some(&mut len),
                )
            };
            if result.is_err() || len < 2 {
                return None;
            }

            let units = &bytes[..len as usize];
            let wide: Vec<u16> = units
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .take_while(|&c| c != 0)
                .collect();
            let path = String::from_utf16_lossy(&wide);
            if path.is_empty() {
                None
            } else {
                Some(PathBuf::from(path))
            }
        }

        fn load_path() -> Option<(HMODULE, PathBuf)> {
            let mut candidates = Vec::new();

            if let Some(dir) = Self::dll_path() {
                if dir.extension().is_some_and(|e| e.eq_ignore_ascii_case("dll")) {
                    candidates.push(dir.clone());
                } else {
                    candidates.push(dir.join("NPClient64.dll"));
                    candidates.push(dir.join("NPClient.dll"));
                }
            }

            // Keep conventional fallbacks as well. This helps installations where
            // the registry value is missing or stale but the standard TrackIR 5 directory exists.
            candidates.push(Path::new(r"C:\Program Files (x86)\NaturalPoint\TrackIR5\NPClient64.dll").to_path_buf());
            candidates.push(Path::new(r"C:\Program Files\NaturalPoint\TrackIR5\NPClient64.dll").to_path_buf());

            for path in candidates {
                if !path.is_file() {
                    continue;
                }
                let wide: Vec<u16> = path.to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
                if let Ok(dll) = unsafe { LoadLibraryW(PCWSTR(wide.as_ptr())) } {
                    return Some((dll, path));
                }
            }
            None
        }

        unsafe fn symbol<T: Copy>(dll: HMODULE, name: windows::core::PCSTR) -> Option<T> {
            let proc = GetProcAddress(dll, name)?;
            Some(transmute_copy(&proc))
        }

        fn try_start(&mut self) -> Option<()> {
            if !self.native_enabled {
                return None;
            }
            if self.native.is_some() {
                return Some(());
            }
            if self.tried.is_some_and(|t| t.elapsed() < Duration::from_secs(2)) {
                return None;
            }
            self.tried = Some(Instant::now());

            let (dll, path) = Self::load_path()?;
            let symbols = unsafe {
                Some((
                    Self::symbol::<RegisterWindowHandle>(dll, s!("NP_RegisterWindowHandle"))?,
                    Self::symbol::<UnregisterWindowHandle>(dll, s!("NP_UnregisterWindowHandle"))?,
                    Self::symbol::<RegisterProgramProfileId>(dll, s!("NP_RegisterProgramProfileID"))?,
                    Self::symbol::<QueryVersion>(dll, s!("NP_QueryVersion"))?,
                    Self::symbol::<RequestData>(dll, s!("NP_RequestData"))?,
                    Self::symbol::<GetData>(dll, s!("NP_GetData"))?,
                    Self::symbol::<StartDataTransmission>(dll, s!("NP_StartDataTransmission"))?,
                    Self::symbol::<StopDataTransmission>(dll, s!("NP_StopDataTransmission"))?,
                ))
            }?;

            let (
                register_window,
                unregister_window,
                register_profile,
                query_version,
                request_data,
                get_data,
                start_transmission,
                stop_transmission,
            ) = symbols;

            let hwnd = self.hwnd.map(|h| h as *mut c_void).unwrap_or(std::ptr::null_mut());
            let register_result = unsafe { register_window(hwnd) };
            if register_result != 0 {
                                return None;
            }

            let mut version = 0u16;
            let version_result = unsafe { query_version(&mut version) };
            if version_result != 0 {
                unsafe {
                    let _ = unregister_window();
                }
                return None;
            }

            let profile_result = unsafe { register_profile(PROFILE_ID) };
            if profile_result != 0 {
                log::warn!("head tracking: TrackIR NPClient rejected profile ID {PROFILE_ID} (result {profile_result})");
            }

            let request_result = unsafe { request_data(NP_REQUEST) };
            if request_result != 0 {
                unsafe {
                    let _ = unregister_window();
                }
                return None;
            }

            let start_result = unsafe { start_transmission() };
            if start_result != 0 {
                unsafe {
                    let _ = unregister_window();
                }
                return None;
            }

            log::info!(
                "head tracking: native TrackIR NPClient {}.{} loaded from {}",
                version >> 8,
                version & 0xff,
                path.display()
            );

            self.native = Some(Native {
                _dll: dll,
                unregister_window,
                get_data,
                stop_transmission,
                last_frame: None,
            });
            Some(())
        }

        /// Poll a fresh TrackIR frame. NPClient position values are in millimetres;
        /// openOMSI's HeadPose uses centimetres, so translation is divided by ten.
        pub fn poll(&mut self) -> Option<HeadPose> {
            self.try_start()?;
            let native = self.native.as_mut()?;

            let mut data = TrackIrData::default();
            let result = unsafe { (native.get_data)(&mut data) };
            if result != 0 {
                return None;
            }

            if native.last_frame.replace(data.w_p_frame_signature) == Some(data.w_p_frame_signature) {
                return None;
            }

            // NPClient uses the NaturalPoint/TrackIR coordinate convention. Keep the
            // native pose in openOMSI's common head-tracking convention here. User-facing
            // sensitivity and inversion are applied later, in the camera path, so native
            // TrackIR and OpenTrack behave identically.
            // Keep the native TrackIR translation signs here.  seat_offset()
            // applies the OMSI camera-frame conversion below.  In particular,
            // this makes the native TrackIR X and Z directions match the
            // desired default camera movement, so users do not need to enable
            // the X/Z inversion switches for a normal TrackIR installation.
            // (NPClient's units: +-16383 for +-180 degrees and for +-50 cm, as opentrack's
            // NPClient writes them - into the cm and degrees of the UDP and freetrack poses, so
            // the sensitivities' 100 is 1:1 whichever source the pose comes from)
            const DEG: f32 = 180.0 / 16383.0;
            const CM: f32 = 50.0 / 16383.0;
            let v = [
                data.f_np_x * CM,
                data.f_np_y * CM,
                data.f_np_z * CM,
                -data.f_np_yaw * DEG,
                -data.f_np_pitch * DEG,
                // TrackIR native roll is opposite to the OMSI camera convention.
                -data.f_np_roll * DEG,
            ];
            if v.iter().any(|x| !x.is_finite()) {
                return None;
            }

            if !self.announced {
                self.announced = true;
                log::info!("head tracking: receiving native TrackIR poses from NPClient");
            }

            Some(HeadPose {
                pos: [v[0], v[1], v[2]],
                rot: [v[3], v[4], v[5]],
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn freetrack_pose_is_in_cm_and_degrees() {
        let p = freetrack_pose(-0.5f32.to_radians() * 60.0, 10f32.to_radians(), 5f32.to_radians(), 15.0, -20.0, 100.0);
        assert!((p.rot[0] - 30.0).abs() < 1e-3 && (p.rot[1] + 10.0).abs() < 1e-3 && (p.rot[2] - 5.0).abs() < 1e-3);
        assert_eq!(p.pos, [1.5, -2.0, 10.0]);
    }

    /// The head 10 cm to the right (opentrack's x -10), 5 cm up and 20 cm back: the eye goes
    /// right, up and back with it (#1157).
    #[test]
    fn the_eye_follows_the_head_across() {
        let p = HeadPose { pos: [-10.0, 5.0, 20.0], rot: [0.0; 3] };
        let o = p.seat_offset();
        assert!((o.x - 0.1).abs() < 1e-6 && (o.y + 0.2).abs() < 1e-6 && (o.z - 0.05).abs() < 1e-6, "{o}");
        // (held within 60 cm)
        assert!((HeadPose { pos: [-200.0, 0.0, 0.0], rot: [0.0; 3] }.seat_offset().x - 0.6).abs() < 1e-6);
    }
}
