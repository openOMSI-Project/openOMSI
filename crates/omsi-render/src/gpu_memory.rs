//! GPU dedicated-memory detection for the rendering backends.
//! The OpenGL and ANGLE paths may omit PCI IDs even on a discrete GPU.

use super::*;

/// The card's own memory in MB where the system tells it: Windows, through DXGI, for
/// whichever backend draws; Linux, through the DRM driver's sysfs (amdgpu; not
/// NVIDIA's own driver, whose memory [`vulkan_vram_mb`] reads from Vulkan instead).
fn dedicated_vram_mb(info: &wgpu::AdapterInfo) -> Option<u64> {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};
        let f: IDXGIFactory1 = CreateDXGIFactory1().ok()?;
        let mut i = 0;
        let mut name_match = None;
        let mut ambiguous_name = false;
        // ANGLE wraps the GPU name and embeds the PCI device ID as "(0x00006613)":
        // check both the name and that ID, when present.
        let angle_device = info.name
            .split("(0x")
            .nth(1)
            .and_then(|s| s.split(')').next())
            .and_then(|s| u32::from_str_radix(s, 16).ok());
        while let Ok(a) = f.EnumAdapters1(i) {
            i += 1;
            let Ok(d) = a.GetDesc1() else { continue };
            let mb = d.DedicatedVideoMemory as u64 >> 20;
            if mb == 0 {
                continue;
            }
            // OpenGL/ANGLE often report vendor/device as 0 even on a discrete GPU.
            // Prefer matching PCI IDs when they are available.
            if info.vendor != 0 && info.device != 0
                && d.VendorId == info.vendor && d.DeviceId == info.device
            {
                return Some(mb);
            }
            // DXGI still knows the card's name. Use it only if it identifies exactly
            // one adapter; never borrow another card's budget on a multi-GPU PC.
            let dxgi_name = String::from_utf16_lossy(&d.Description);
            let dxgi_name = dxgi_name.trim_end_matches('\0').trim();
            let angle_matches = info.name.starts_with("ANGLE (")
                && info.name.contains(dxgi_name)
                && angle_device == Some(d.DeviceId)
                && (info.vendor == 0 || info.vendor == d.VendorId);
            if (info.vendor == 0 || info.device == 0)
                && (dxgi_name.eq_ignore_ascii_case(info.name.trim()) || angle_matches)
            {
                ambiguous_name |= name_match.replace(mb).is_some();
            }
        }
        if ambiguous_name { None } else { name_match }
    }
    #[cfg(target_os = "linux")]
    {
        let hex = |p: std::path::PathBuf| {
            let t = std::fs::read_to_string(p).ok()?;
            u32::from_str_radix(t.trim().trim_start_matches("0x"), 16).ok()
        };
        for e in std::fs::read_dir("/sys/class/drm").ok()?.flatten() {
            // (card0, card1, ...; not their connectors, card1-DP-1)
            let name = e.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with("card") || name.contains('-') {
                continue;
            }
            let dev = e.path().join("device");
            if hex(dev.join("vendor")) != Some(info.vendor) || hex(dev.join("device")) != Some(info.device) {
                continue;
            }
            let bytes = std::fs::read_to_string(dev.join("mem_info_vram_total"))
                .ok()
                .and_then(|t| t.trim().parse::<u64>().ok());
            if let Some(b) = bytes.filter(|b| *b > 0) {
                return Some(b >> 20);
            }
        }
        None
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = info;
        None
    }
}

/// The largest device-local memory heap of a Vulkan adapter (MB).
#[cfg(target_os = "linux")]
fn vulkan_vram_mb(adapter: &wgpu::Adapter) -> Option<u64> {
    // SAFETY: the adapter outlives the borrow, and only its memory properties are read
    let hal = unsafe { adapter.as_hal::<wgpu::hal::api::Vulkan>() }?;
    // SAFETY: the physical device belongs to this instance
    let props = unsafe { hal.shared_instance().raw_instance().get_physical_device_memory_properties(hal.raw_physical_device()) };
    props.memory_heaps[..props.memory_heap_count as usize]
        .iter()
        .filter(|h| h.flags.contains(ash::vk::MemoryHeapFlags::DEVICE_LOCAL))
        .map(|h| h.size >> 20)
        .max()
}

#[cfg(not(target_os = "linux"))]
fn vulkan_vram_mb(_adapter: &wgpu::Adapter) -> Option<u64> {
    None
}

/// The card's own memory in MB: wgpu's report on DirectX 12 (the adapter's DXGI
/// `DedicatedVideoMemory`, the number [`dedicated_vram_mb`] reads), else what the system
/// tells. (wgpu's Vulkan report sums every device-local heap where [`vulkan_vram_mb`] takes
/// the largest, and Metal's is the working set the system recommends, not the card's own
/// memory: those keep their own paths, so the texture budgets stay as they were.)
pub(super) fn adapter_vram_mb(adapter: &wgpu::Adapter, info: &wgpu::AdapterInfo, mem: Option<&wgpu::AdapterMemoryInfo>) -> Option<u64> {
    match mem {
        Some(m) if info.backend == wgpu::Backend::Dx12 => Some(m.dedicated_bytes >> 20),
        _ => dedicated_vram_mb(info).or_else(|| vulkan_vram_mb(adapter)),
    }
}

/// What the driver says it really hands out (MB): the memory budget, the amount the
/// adapter may take all together. None where nothing is told, as on the older phones
/// whose allowance stays the 1000 MB guess (#323).
fn budget_mb(mem: Option<&wgpu::AdapterMemoryInfo>) -> Option<u64> {
    let m = mem?;
    let budget = m.budget_bytes? >> 20;
    (budget > 0).then_some(budget)
}

/// Conservative texture allowance in MB, distinct from physical VRAM.
/// Low-memory discrete GPUs need room for render targets and driver allocations.
pub(super) fn texture_allowance_mb(info: &wgpu::AdapterInfo, vram: Option<u64>, mem: Option<&wgpu::AdapterMemoryInfo>) -> u64 {
    let discrete_allowance = |fallback| {
        vram.filter(|v| *v >= 512).map_or(fallback, |v| {
            if v <= 2560 { v * 35 / 100 }
            else if v <= 6144 { (v / 2).min(1600) }
            else { v * 3 / 10 }
        })
    };
    match info.device_type {
        wgpu::DeviceType::DiscreteGpu => discrete_allowance(1600),
        wgpu::DeviceType::IntegratedGpu if info.backend == wgpu::Backend::Metal => 3000,
        // (an APU's memory is the system's own and its driver tells how much it really
        // hands out: 1000 was a guess for the chips that report nothing, and on a desktop
        // APU with 11 GB of budget it kept a mod's 4k livery on its low mip levels -
        // the discrete branch's 3/10 share now, up to 4000)
        wgpu::DeviceType::IntegratedGpu | wgpu::DeviceType::VirtualGpu => {
            let guess = 1000;
            budget_mb(mem).filter(|b| *b >= 2_600).map_or(guess, |b| (b * 3 / 10).min(4_000))
        }
        _ => discrete_allowance(800),
    }
}

/// How much lighter than the settings ask a chip draws.
#[derive(Debug, PartialEq)]
enum Picture {
    /// a small or shared chip: no SSAO, no MSAA, shadow maps of at most 1024
    Weak,
    /// a card of up to 4 GB: no SSAO, at most 2x MSAA and 2048 shadow maps
    Modest,
    AsAsked,
}

/// A small or shared graphics chip (the processor's graphics outside a Mac, a phone, a
/// card of up to 2.5 GB, OpenGL whose memory nothing tells) is a weak one; a card of up
/// to 4 GB a modest one. (The settings' "High" on such a machine ran out of memory or
/// at a dozen frames a second.)
fn picture(backend: wgpu::Backend, device_type: wgpu::DeviceType, vram: Option<u64>) -> Picture {
    let weak = (backend == wgpu::Backend::Gl && vram.is_none())
        // (a phone's chip, whatever type its driver reports: some say "other")
        || cfg!(target_os = "android")
        || (device_type == wgpu::DeviceType::IntegratedGpu && backend != wgpu::Backend::Metal)
        || vram.is_some_and(|v| v <= 2560);
    if weak {
        Picture::Weak
    } else if vram.is_some_and(|v| v <= 4200) {
        Picture::Modest
    } else {
        Picture::AsAsked
    }
}

/// The settings `options` lightened for this chip (see [`picture`]), and whether they
/// were; SSAO is off on OpenGL whatever its memory (#422). OMSI_FULL_GPU=1 keeps the
/// settings as they are.
pub(super) fn lighter_picture(info: &wgpu::AdapterInfo, vram: Option<u64>, options: RenderOptions) -> (RenderOptions, bool) {
    let options = RenderOptions { ssao: options.ssao && info.backend != wgpu::Backend::Gl, ..options };
    if omsi_cfg::flags::OMSI_FULL_GPU.is_set() {
        return (options, false);
    }
    match picture(info.backend, info.device_type, vram) {
        Picture::Weak => {
            log::warn!("{}: a small or shared graphics chip - no SSAO, no MSAA, shadow maps of at most 1024 (OMSI_FULL_GPU=1 keeps the settings)", info.name);
            (RenderOptions { msaa: 1, ssao: false, shadow_size: options.shadow_size.min(1024), ..options }, true)
        }
        Picture::Modest => {
            log::info!("{}: {} MB of its own - no SSAO, at most 2x MSAA and 2048 shadow maps (OMSI_FULL_GPU=1 keeps the settings)", info.name, vram.unwrap_or(0));
            (RenderOptions { msaa: options.msaa.min(2), ssao: false, shadow_size: options.shadow_size.min(2048), ..options }, true)
        }
        Picture::AsAsked => (options, false),
    }
}

#[cfg(all(test, not(target_os = "android")))]
mod tests {
    use super::{picture, Picture};
    use wgpu::{Backend, DeviceType};

    /// OpenGL and ANGLE report a discrete card as "other": its memory, once DXGI tells
    /// it, decides, as on Vulkan; unknown memory on OpenGL stays the weak profile.
    #[test]
    fn opengl_card_with_known_memory_is_not_taken_for_a_weak_chip() {
        assert_eq!(picture(Backend::Gl, DeviceType::Other, None), Picture::Weak);
        assert_eq!(picture(Backend::Gl, DeviceType::Other, Some(4076)), Picture::Modest);
        assert_eq!(picture(Backend::Vulkan, DeviceType::DiscreteGpu, Some(4076)), Picture::Modest);
        assert_eq!(picture(Backend::Gl, DeviceType::Other, Some(16304)), Picture::AsAsked);
        assert_eq!(picture(Backend::Gl, DeviceType::Other, Some(2048)), Picture::Weak);
        assert_eq!(picture(Backend::Dx12, DeviceType::IntegratedGpu, Some(8192)), Picture::Weak);
    }
}
