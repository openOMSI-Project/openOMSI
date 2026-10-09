//! The driver's compiled pipelines kept between runs (`~/.openomsi/cache/pipelines-*.bin`).
//!
//! Vulkan and OpenGL (the fork's `PIPELINE_CACHE` for program binaries: ANGLE, phones) compile
//! every pipeline from the shader text at each start; with the cache the second start takes the
//! driver's binaries instead - seconds on a phone's GL driver. Metal and Direct3D 12 keep their
//! own caches and do not offer the feature. Data from another driver or GPU is refused by wgpu
//! (`fallback`), so a driver update only costs one slow start. OMSI_NO_PIPELINE_CACHE=1 leaves it
//! out.

use std::path::PathBuf;
use std::sync::Mutex;

struct Slot {
    device: wgpu::Device,
    cache: wgpu::PipelineCache,
    path: PathBuf,
}

/// The cache of the device last opened (one game renderer at a time; a device opened again after
/// a fallback, or a second renderer, replaces it - the other one's pipelines are made without).
/// (Devices compare with their instance since the fork's oo/device-eq: two instances' devices
/// had the same ids, and a test's device took another's cache, "PipelineCache does not exist".)
static SLOT: Mutex<Option<Slot>> = Mutex::new(None);

/// Whether to ask the device for the feature.
pub(crate) fn wanted(adapter: &wgpu::Adapter) -> wgpu::Features {
    if omsi_cfg::flags::OMSI_NO_PIPELINE_CACHE.is_set() {
        return wgpu::Features::empty();
    }
    adapter.features() & wgpu::Features::PIPELINE_CACHE
}

fn path(info: &wgpu::AdapterInfo) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let home = PathBuf::from(home);
    home.is_absolute().then(|| home.join(".openomsi/cache").join(file_name(info.backend, info.vendor, info.device)))
}

// (one file per GPU: a laptop's two GPUs would throw each other's data away)
fn file_name(backend: wgpu::Backend, vendor: u32, device: u32) -> String {
    format!("pipelines-{}-{vendor:04x}-{device:04x}.bin", format!("{backend:?}").to_lowercase())
}

/// Open the cache of a device made with [`wanted`]'s feature, from last run's file.
pub(crate) fn open(device: &wgpu::Device, info: &wgpu::AdapterInfo) {
    let mut slot = SLOT.lock().unwrap_or_else(|e| e.into_inner());
    *slot = None;
    if !device.features().contains(wgpu::Features::PIPELINE_CACHE) {
        return;
    }
    let Some(path) = path(info) else { return };
    // (a cache is a few MB; anything far bigger is not ours)
    let data = std::fs::metadata(&path)
        .ok()
        .filter(|m| m.len() < 256 << 20)
        .and_then(|_| std::fs::read(&path).ok());
    // SAFETY: the data was written by `save` from `get_data` of a cache of this backend; wgpu
    // checks its header against the adapter and driver and starts empty on a mismatch
    // (`fallback`), and a file cut short fails the same check.
    let cache = unsafe {
        device.create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
            label: Some("omsi pipelines"),
            data: data.as_deref(),
            fallback: true,
        })
    };
    log::info!("pipeline cache: {} ({})", path.display(), match &data { Some(d) => format!("{} KB from the last run", d.len() / 1024), None => "new".into() });
    *slot = Some(Slot { device: device.clone(), cache, path });
}

/// The cache to build `device`'s pipelines with, if it has one.
pub(crate) fn get(device: &wgpu::Device) -> Option<wgpu::PipelineCache> {
    let slot = SLOT.lock().unwrap_or_else(|e| e.into_inner());
    slot.as_ref().filter(|s| &s.device == device).map(|s| s.cache.clone())
}

/// Write the cache to its file (after the pipelines were made, and at the end of the game, for
/// the ones made later: puddles, Enhanced+). Nothing happens without a cache.
pub fn save() {
    let slot = SLOT.lock().unwrap_or_else(|e| e.into_inner());
    let Some(s) = slot.as_ref() else { return };
    let Some(data) = s.cache.get_data() else { return };
    if data.is_empty() || std::fs::metadata(&s.path).is_ok_and(|m| m.len() == data.len() as u64) {
        return;
    }
    if let Some(dir) = s.path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // written beside and renamed over: a game ended mid-write leaves the old file whole
    let tmp = s.path.with_extension("tmp");
    match std::fs::write(&tmp, &data).and_then(|_| std::fs::rename(&tmp, &s.path)) {
        Ok(()) => log::info!("pipeline cache saved: {} KB", data.len() / 1024),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            log::warn!("pipeline cache not saved to {}: {e}", s.path.display());
        }
    }
}

/// Forget `device`'s cache (the device is lost or failed to make the renderer).
pub(crate) fn close(device: &wgpu::Device) {
    let mut slot = SLOT.lock().unwrap_or_else(|e| e.into_inner());
    if slot.as_ref().is_some_and(|s| &s.device == device) {
        *slot = None;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn file_names_backend_and_gpu() {
        assert_eq!(super::file_name(wgpu::Backend::Gl, 0x5143, 0x43050a01), "pipelines-gl-5143-43050a01.bin");
        assert_eq!(super::file_name(wgpu::Backend::Vulkan, 0x10de, 0x2684), "pipelines-vulkan-10de-2684.bin");
    }
}
