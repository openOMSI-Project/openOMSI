//! What the machine has and gives back: physical memory, the texture budget, and returning freed memory to the system.

use super::*;

/// Bytes of memory the machine has.
pub(crate) fn physical_memory() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        extern "C" {
            fn sysctlbyname(
                name: *const std::ffi::c_char,
                oldp: *mut std::ffi::c_void,
                oldlenp: *mut usize,
                newp: *mut std::ffi::c_void,
                newlen: usize,
            ) -> i32;
        }
        let mut v: u64 = 0;
        let mut len = std::mem::size_of::<u64>();
        // SAFETY: a u64 buffer of the size given, a nul-terminated name
        let r = unsafe {
            sysctlbyname(
                c"hw.memsize".as_ptr(),
                &mut v as *mut u64 as *mut std::ffi::c_void,
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        (r == 0 && v > 0).then_some(v)
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        // MemTotal in kB
        let text = std::fs::read_to_string("/proc/meminfo").ok()?;
        let line = text.lines().find(|l| l.starts_with("MemTotal:"))?;
        let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
        Some(kb * 1024)
    }
    #[cfg(windows)]
    {
        use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut m = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
        // SAFETY: a MEMORYSTATUSEX with its length set, as the call wants it
        unsafe { GlobalMemoryStatusEx(&mut m) }.ok()?;
        (m.ullTotalPhys > 0).then_some(m.ullTotalPhys)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "android", windows)))]
    {
        None
    }
}

/// The texture budget in bytes: `OMSI_TEXTURE_MEMORY` (MB), else the setting, else an
/// eighth of the machine's memory (2 GB on a 16 GB Mac - Ahlheim's main station needs
/// about 1.5).
pub(crate) fn texture_budget(settings: &settings::Settings) -> u64 {
    let mb = omsi_cfg::env::var("OMSI_TEXTURE_MEMORY")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(settings.texture_memory as u64);
    if mb > 0 {
        // (a budget the graphics card cannot hold is taken down to what it can: the
        // settings offer up to 6 GB, and a 2 GB card lost its device at the first frames,
        // #323)
        let card = omsi_render::ADAPTER_TEXTURE_MB.load(std::sync::atomic::Ordering::Relaxed);
        // (a bigger card may take up to half of its own memory when it is set)
        let vram = omsi_render::ADAPTER_VRAM_MB.load(std::sync::atomic::Ordering::Relaxed);
        let limit = if vram > 2560 { card.max(vram / 2) } else { card };
        if limit > 0 && mb > limit * 5 / 4 {
            log::warn!("texture memory {mb} MB is more than the graphics card holds: {} MB", limit * 5 / 4);
            return limit * 5 / 4 * 1_000_000;
        }
        return mb * 1_000_000;
    }
    // automatic: an eighth of the system's memory, but no more than the graphics adapter
    // is taken to hold (a PC with 32 GB and a 4 GB card let the textures grow to 4 GB and
    // the card ran out of memory)
    // (and no more than 2.5 GB: since Windows reads its memory as well - 0.1.237 - a PC
    // with 32 or 64 GB let the textures grow to 4-8 GB, the game took 9 GB at the start and
    // ran at single-digit frame rates, #277; the setting still gives more when asked)
    let ram = physical_memory().map(|m| m / 8).unwrap_or(2_000_000_000).min(2_500_000_000);
    match omsi_render::ADAPTER_TEXTURE_MB.load(std::sync::atomic::Ordering::Relaxed) {
        0 => ram.min(1_600_000_000),
        g => ram.min(g * 1_000_000),
    }
}

/// Give the heap's free pages back to the system, on a thread of its own (the allocator
/// keeps what was freed for later, and it counts as the game's memory until then).
pub fn release_free_memory() {
    #[cfg(target_os = "macos")]
    {
        extern "C" {
            fn malloc_zone_pressure_relief(zone: *mut std::ffi::c_void, goal: usize) -> usize;
        }
        let _ = std::thread::Builder::new()
            .name("heap relief".into())
            .spawn(|| {
                let t = Instant::now();
                // SAFETY: a null zone asks every malloc zone; the call only returns free pages
                let freed = unsafe { malloc_zone_pressure_relief(std::ptr::null_mut(), 0) };
                if omsi_cfg::env::var_os("OMSI_PROFILE").is_some() {
                    log::info!(
                        "heap: {:.0} MB of free pages given back in {:.0} ms",
                        freed as f64 / 1e6,
                        t.elapsed().as_secs_f64() * 1000.0
                    );
                }
            });
    }
}
