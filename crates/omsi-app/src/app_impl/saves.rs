//! The end of a session and the situations saved: the quicksave, the save slots, the last
//! situation, and the restart after the graphics device was lost.

use super::*;

impl App {
    /// Save the personnel file and the session summary (once: every caller ends the game,
    /// and the frames the loop still runs before it stops count no more time).
    pub(crate) fn finish_session(&mut self) {
        crate::game_lists::flush_settings(true);
        self.exiting = true;
        // (the tiles loaded on the way added to what the map lacks)
        if let Some(w) = self.world.clone() {
            let mut none = None;
            crate::app::report_missing_content(&w, &mut none);
        }
        // PluginFinalize, as OMSI calls it on the way out
        if let Some(mut p) = self.integrations.plugins.take() {
            p.finalize();
        }
        if self.player.is_none() || self.session.career.seconds <= 0.0 {
            return;
        }
        // the situation to continue next time (OMSI writes it when a map is left; once, as
        // the rest: `career.seconds` is zero after the first time)
        self.save_last_situation();
        if self.session.career.path.is_some() {
            if let Err(e) = self.session.career.save() {
                log::warn!("writing the personnel file: {e}");
            }
        }
        let bus = self.args.bus.clone().unwrap_or_default();
        if let Err(e) = self.session.career.write_session(
            &self.args.map,
            &bus,
            self.args.line.as_deref(),
            self.args.tour.as_deref(),
        ) {
            log::warn!("writing the session summary: {e}");
        }
        self.session.career.seconds = 0.0;
    }

    /// Start the game again on the quicksave (`Situations/quicksave.osn` of the content
    /// folder): a fresh process, as OMSI loads a situation into a fresh world. False when
    /// there is none.
    pub(crate) fn load_quicksave(&mut self) -> bool {
        let dir = crate::startup::content_dir().unwrap_or_else(|| self.args.root.clone()).join("Situations");
        let mut file = dir.join("quicksave.osn");
        // (the newer of the content folder's and the fallback folder's, see `save_or_fallback`)
        if let Some(f) = crate::startup::save_fallback_dir().map(|f| f.join("Situations").join("quicksave.osn")) {
            let age = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
            if f.exists() && (!file.exists() || age(&f) > age(&file)) {
                file = f;
            }
        }
        if !file.exists() {
            self.service_msg = Some(("No quicksave yet (Ctrl+S saves one)".into(), 4.0));
            return false;
        }
        let Ok(exe) = std::env::current_exe() else { return false };
        let mut cmd = std::process::Command::new(exe);
        cmd.arg("--root").arg(&self.args.root).arg("--no-menu").arg("--situation").arg(&file);
        // (the duty typed by itself goes on being typed: `--autostart` with a situation)
        if self.player.as_ref().is_some_and(|p| p.duty_typed) {
            cmd.arg("--autostart");
        }
        match cmd.spawn() {
            Ok(_) => {
                log::info!("loading {} in a new game", file.display());
                true
            }
            Err(e) => {
                self.service_msg = Some((format!("Could not start the game again: {e}"), 5.0));
                false
            }
        }
    }

    /// `laststn.osn` in the map's folder (the content folder's copy: the original is never
    /// written), as OMSI keeps it: the launcher offers to continue it. Not
    /// in a tutorial, a LAN session or without a bus of one's own.
    pub(crate) fn save_last_situation(&mut self) -> Option<std::path::PathBuf> {
        if self.menus.tutorial.is_some() || self.net.lan.is_some() || self.player.is_none() {
            return None;
        }
        let (Some(w), Some(cam)) = (self.world.as_ref(), self.camera.as_ref()) else { return None };
        let dir = std::path::Path::new(&self.args.map.replace('\\', "/")).parent().map(|d| d.to_path_buf())?;
        let base = crate::startup::content_dir()?;
        let dir = base.join(dir);
        let _ = std::fs::create_dir_all(&dir);
        let out = dir.join("laststn.osn");
        let sit = build_situation(&self.args, w, &self.clock, self.args.weather.as_deref(), self.player.as_ref(), &self.session.placed, cam, self.session.duty.as_ref(), "Last situation");
        match sit.save(&out) {
            Ok(()) => {
                if let Some(p) = self.player.as_ref() {
                    crate::situation::save_script_textures(&out, &p.vehicle);
                }
                log::info!("saved the last situation {}", out.display());
                Some(out)
            }
            Err(e) => {
                log::warn!("saving {}: {e}", out.display());
                None
            }
        }
    }

    /// The graphics device was lost (the driver reset the card: it ran out of memory, or a
    /// frame took it too long): the game starts again by itself on the situation just
    /// saved, with lighter graphics (`Settings::apply_safe_gpu`), and on Windows with the
    /// other graphics interface when the lost one was Vulkan. Twice at most in a row. False
    /// when it cannot (a LAN session, the tutorial, nothing to save): the session ends.
    pub(crate) fn restart_after_device_loss(&mut self) -> bool {
        let n = omsi_cfg::flags::OMSI_SAFE_GPU.parse::<u32>().unwrap_or(0);
        if n >= 2 {
            return false;
        }
        let Some(file) = self.save_last_situation() else { return false };
        let Ok(exe) = std::env::current_exe() else { return false };
        let mut cmd = std::process::Command::new(exe);
        cmd.arg("--root").arg(&self.args.root).arg("--no-menu").arg("--situation").arg(&file);
        // (the duty typed by itself goes on being typed: `--autostart` with a situation)
        if self.player.as_ref().is_some_and(|p| p.duty_typed) {
            cmd.arg("--autostart");
        }
        cmd.env("OMSI_SAFE_GPU", (n + 1).to_string());
        // (on Windows the other interface: DirectX 12 after Vulkan, Vulkan after DirectX 12 -
        // an AMD Radeon's DX12 driver lost the device where its Vulkan one did not, #274)
        let name = self.renderer.as_ref().map(|r| r.adapter_name.clone()).unwrap_or_default();
        if cfg!(windows) {
            if name.contains("(Vulkan)") {
                cmd.env("OMSI_BACKEND", "dx12");
            } else if name.contains("(Dx12)") {
                cmd.env("OMSI_BACKEND", "vulkan");
            }
        }
        match cmd.spawn() {
            Ok(_) => {
                log::warn!("starting again with safer graphics on {} (the graphics device was lost)", file.display());
                true
            }
            Err(e) => {
                log::warn!("could not start the game again: {e}");
                false
            }
        }
    }

    /// Quick save: `Situations/quicksave.osn` next to the game, as OMSI's `quicksave`.
    pub(crate) fn quick_save(&mut self) {
        let (Some(w), Some(cam)) = (self.world.as_ref(), self.camera.as_ref()) else { return };
        // into openOMSI's content folder, never the original installation (the menu and
        // --situation find it there as they find a mod's files)
        let dir = crate::startup::content_dir().unwrap_or_else(|| self.args.root.clone()).join("Situations");
        let out = dir.join("quicksave.osn");
        let sit = build_situation(&self.args, w, &self.clock, self.args.weather.as_deref(), self.player.as_ref(), &self.session.placed, cam, self.session.duty.as_ref(), "Quicksave");
        match save_or_fallback(&sit, &out, Path::new("Situations").join("quicksave.osn").as_path()) {
            Ok(out) => {
                if let Some(p) = self.player.as_ref() {
                    crate::situation::save_script_textures(&out, &p.vehicle);
                }
                log::info!("saved situation {} ({} vehicles)", out.display(), sit.vehicles.len());
                self.service_msg = Some(("Situation saved (quicksave)".into(), 3.0));
            }
            Err(e) => {
                log::warn!("saving {}: {e}", out.display());
                self.service_msg = Some((format!("Could not save: {e}"), 5.0));
            }
        }
    }

    /// A save of its own (#341): the situation into the next free `Saves/Slot <n>.osn` of
    /// the map's folder in the content folder - none is ever overwritten. The launcher
    /// offers them, with the last situation, to continue from.
    pub(crate) fn save_slot(&mut self) {
        let (Some(w), Some(cam)) = (self.world.as_ref(), self.camera.as_ref()) else { return };
        let Some(dir) = crate::startup::content_dir().and_then(|base| {
            std::path::Path::new(&self.args.map.replace('\\', "/")).parent().map(|d| base.join(d).join(SAVES))
        }) else {
            return;
        };
        let _ = std::fs::create_dir_all(&dir);
        // (the slots of the fallback folder count too: a number is never given twice)
        let rel_dir = std::path::Path::new(&self.args.map.replace('\\', "/")).parent().map(|d| d.join(SAVES)).unwrap_or_default();
        let fallback = crate::startup::save_fallback_dir().map(|f| f.join(&rel_dir));
        let taken = |n: usize| dir.join(format!("Slot {n}.osn")).exists() || fallback.as_ref().is_some_and(|f| f.join(format!("Slot {n}.osn")).exists());
        let Some(n) = (1..10_000).find(|n| !taken(*n)) else { return };
        let out = dir.join(format!("Slot {n}.osn"));
        let bus = self.player.as_ref().map(|p| {
            let d = &p.vehicle.ty.def;
            if d.type_name.trim().is_empty() { d.path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default() } else { d.type_name.trim().to_string() }
        });
        let t = self.clock.time;
        let what = match (bus, self.session.duty.as_ref()) {
            (Some(b), Some(d)) => format!("{b}, line {} / {}", d.line.trim(), d.tour.trim()),
            (Some(b), None) => b,
            (None, _) => "on foot".to_string(),
        };
        let name = format!("Slot {n}: {what}, {:02}:{:02}", (t / 3600.0) as i32 % 24, ((t % 3600.0) / 60.0) as i32);
        let sit = build_situation(&self.args, w, &self.clock, self.args.weather.as_deref(), self.player.as_ref(), &self.session.placed, cam, self.session.duty.as_ref(), &name);
        match save_or_fallback(&sit, &out, rel_dir.join(format!("Slot {n}.osn")).as_path()) {
            Ok(out) => {
                if let Some(p) = self.player.as_ref() {
                    crate::situation::save_script_textures(&out, &p.vehicle);
                }
                log::info!("saved situation {} ({} vehicles)", out.display(), sit.vehicles.len());
                self.service_msg = Some((format!("Saved as slot {n}: the launcher continues from it"), 4.0));
            }
            Err(e) => {
                log::warn!("saving {}: {e}", out.display());
                self.service_msg = Some((format!("Could not save: {e}"), 5.0));
            }
        }
    }
}

/// The folder of a map's save slots, inside the map's folder in the content folder (the
/// launcher reads it as well: `omsi_launcher_lib::saved_situations`).
pub(crate) const SAVES: &str = "Saves";

/// Save `sit` to `out`; where that folder takes no file, to `rel` under the fallback folder
/// (`startup::save_fallback_dir`) instead (#1673). The file written.
fn save_or_fallback(sit: &omsi_content::situation::Situation, out: &Path, rel: &Path) -> std::io::Result<PathBuf> {
    let first = out.parent().map(std::fs::create_dir_all).unwrap_or(Ok(())).and_then(|_| sit.save(out));
    let e = match first {
        Ok(()) => return Ok(out.to_path_buf()),
        Err(e) => e,
    };
    let Some(alt) = crate::startup::save_fallback_dir().map(|f| f.join(rel)).filter(|a| a != out) else { return Err(e) };
    log::warn!("saving {}: {e}; saved into {} instead", out.display(), alt.display());
    if let Some(d) = alt.parent() {
        std::fs::create_dir_all(d)?;
    }
    sit.save(&alt)?;
    Ok(alt)
}
