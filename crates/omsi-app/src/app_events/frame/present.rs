//! The picture in the window's frame: its lighting, the mirrors, the headset, the window's
//! picture and its presentation, the frame rate's limit and the session's statistics.

use super::*;

impl App {
    /// The lighting the frame is drawn with.
    pub(super) fn frame_lighting(&mut self, dt: f32, daylight: omsi_sim::Daylight) -> omsi_render::Lighting {
        if let Some(w) = self.session.weather.as_ref() {
            self.session.wetness = road_wetness(precip_of(w).1, dt as f64, self.session.wetness);
        }
        let inside = match self.net.inside_remote.and_then(|id| self.net.remotes.remotes.get(&id)) {
            // (in another player's bus: its box is the one the camera is in)
            Some(rv) => Some(rv.vehicle()),
            None => self.player.as_ref().map(|p| &p.vehicle),
        };
        let mut lighting = steps::picture_lighting(
            &daylight,
            self.session.weather.as_ref(),
            self.session.cloud_drift,
            self.session.wetness,
            self.world.as_deref(),
            inside,
            self.player.as_ref().map(|p| &p.vehicle),
            self.session.cabin_air.appearance(),
            &self.settings,
            self.clock.run_time as f32,
        );
        if crate::road_snow::enabled() {
            self.session.road_snow.light(&mut lighting, &self.session.snow_tracks);
        }
        lighting
    }

    /// The frame drawn and shown (or, with the window hidden, the simulation kept at a
    /// display's pace), the frame rate limited, the session's statistics.
    pub(super) fn frame_render(
        &mut self,
        event_loop: &ActiveEventLoop,
        time: &FrameTime,
        lighting: &omsi_render::Lighting,
        vr_nav_display: Option<crate::vr_navigator::Display>,
    ) {
        let mut finish = false;
        let mut reconfigure = false;
        let shot = self.perf.shot.take();
        if let Some(s) = self.gfx.surface.as_ref() {
            let (w, h) = (s.config.width, s.config.height);
            self.touch_prepare(w, h);
        }
        if self.gfx.surface.is_some()
            && self.renderer.is_some()
            && self.scene.is_some()
            && self.camera.is_some()
            && self.window.is_some()
        {
            if let Some(shot) = shot {
                self.frame_shot(shot, lighting);
            }
            let (frame, view, shown_nothing) = self.frame_acquire(&mut reconfigure);
            match view {
                Some(view) => self.frame_draw(time.raw_dt, lighting, vr_nav_display, frame, view),
                None => self.frame_hidden(time.now),
            }
            self.frame_pace(time.now, shown_nothing);
            finish = self.frame_count(event_loop);
            if let Some(win) = self.window.as_ref() {
                win.request_redraw();
            }
        }
        if reconfigure {
            // the drawable went away under us (display change, lost surface)
            if let (Some(s), Some(r), Some(win)) = (
                self.gfx.surface.as_mut(),
                self.renderer.as_ref(),
                self.window.as_ref(),
            ) {
                let size = win.inner_size();
                s.resize(r, size.width, size.height);
            }
        }
        if finish {
            self.finish_session();
        }
    }

    /// `shot <file>` from the input script.
    fn frame_shot(&mut self, shot: (PathBuf, bool), lighting: &omsi_render::Lighting) {
        let (Some(s), Some(r), Some(scene), Some(cam)) = (
            self.gfx.surface.as_ref(),
            self.renderer.as_mut(),
            self.scene.as_mut(),
            self.camera.as_ref(),
        ) else {
            return;
        };
        // `shot <file>` from the input script: the scene the window is showing,
        // from its camera and lighting, into a PNG - the only way to look at
        // what an automated window run draws (also when the window is hidden,
        // so it does not depend on a frame being acquired)
        let (path, include_touch) = shot;
        match r.render_to_image(
            scene,
            s.config.width,
            s.config.height,
            cam,
            lighting,
        ) {
            Ok(mut px) => match {
                // (with the on-screen controls, when there are)
                if include_touch {
                    if let Some(over) = self.input.touch.picture(r, s.config.width, s.config.height) {
                        crate::touch::composite(&mut px, &over);
                    }
                }
                image::save_buffer(
                    &path,
                    &px,
                    s.config.width,
                    s.config.height,
                    image::ColorType::Rgba8,
                ) } {
                Ok(()) => log::info!(
                    "input script: window picture written to {}",
                    path.display()
                ),
                Err(e) => log::warn!(
                    "input script: {} could not be written: {e}",
                    path.display()
                ),
            },
            Err(e) => log::warn!(
                "input script: the window picture could not be rendered: {e}"
            ),
        }
    }

    /// The window's picture to draw into (none: the window is hidden; `reconfigure`: the
    /// drawable went away), and whether nothing is shown this frame.
    fn frame_acquire(&mut self, reconfigure: &mut bool) -> (Option<wgpu::SurfaceTexture>, Option<wgpu::TextureView>, bool) {
        let (Some(s), Some(r)) = (self.gfx.surface.as_ref(), self.renderer.as_ref()) else {
            return (None, None, true);
        };
        // A window that is hidden (another app covers it, another Space) gets
        // no frames on macOS. OMSI_RENDER_OCCLUDED=1 draws them into a texture
        // of the window's size anyway and waits for the GPU as a present would,
        // so frame times can be measured with the window out of sight.
        let __t = Instant::now();
        // OMSI_HIDE_WINDOW=from,to: treat the window as hidden between these
        // seconds of the session (the frame is acquired and dropped unshown), to
        // check the hidden-window path without covering the window by hand
        let hide_test = omsi_cfg::flags::OMSI_HIDE_WINDOW.var().and_then(|v| {
            let mut it = v.split(',').filter_map(|x| x.trim().parse::<f32>().ok());
            Some((it.next()?, it.next()?))
        });
        let hidden_now = hide_test
            .map(|(a, b)| (a..b).contains(&self.started.elapsed().as_secs_f32()))
            .unwrap_or(false);
        let acquired = match s.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(_)
            | wgpu::CurrentSurfaceTexture::Suboptimal(_)
            if hidden_now =>
                {
                    wgpu::CurrentSurfaceTexture::Occluded
                }
            other => other,
        };
        let (frame, stand_in) = match acquired {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => (Some(frame), None),
            wgpu::CurrentSurfaceTexture::Occluded
            if omsi_cfg::flags::OMSI_RENDER_OCCLUDED.is_set() =>
                {
                    let (w, h) = (s.config.width, s.config.height);
                    if self
                        .gfx.stand_in
                        .as_ref()
                        .map(|t| (t.width(), t.height()) != (w, h))
                        .unwrap_or(true)
                    {
                        self.gfx.stand_in =
                            Some(r.device.create_texture(&wgpu::TextureDescriptor {
                                label: Some("hidden window"),
                                size: wgpu::Extent3d {
                                    width: w,
                                    height: h,
                                    depth_or_array_layers: 1,
                                },
                                mip_level_count: 1,
                                sample_count: 1,
                                dimension: wgpu::TextureDimension::D2,
                                format: r.format(),
                                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                                view_formats: &[],
                            }));
                    }
                    (
                        None,
                        self.gfx.stand_in
                            .as_ref()
                            .map(|t| t.create_view(&Default::default())),
                    )
                }
            wgpu::CurrentSurfaceTexture::Outdated
            | wgpu::CurrentSurfaceTexture::Lost => {
                *reconfigure = true;
                (None, None)
            }
            _ => (None, None),
        };
        *self.perf.profile.entry("acquire").or_default() += __t.elapsed().as_secs_f64();
        if frame.is_none() {
            self.gfx.hidden_frames += 1;
        }
        let shown_nothing = frame.is_none() && stand_in.is_none();
        let view = frame
            .as_ref()
            .map(|f| f.texture.create_view(&Default::default()))
            .or(stand_in);
        (frame, view, shown_nothing)
    }

    /// The mirrors, the headset's picture, the mirror panels, the window's picture and the
    /// on-screen controls, and the picture shown.
    fn frame_draw(
        &mut self,
        raw_dt: f32,
        lighting: &omsi_render::Lighting,
        vr_nav_display: Option<crate::vr_navigator::Display>,
        frame: Option<wgpu::SurfaceTexture>,
        view: wgpu::TextureView,
    ) {
        #[cfg(not(windows))]
        let _ = vr_nav_display;
        self.frame_mirrors(raw_dt, lighting);
        let (Some(s), Some(r), Some(scene), Some(cam), Some(win)) = (
            self.gfx.surface.as_ref(),
            self.renderer.as_mut(),
            self.scene.as_mut(),
            self.camera.as_ref(),
            self.window.as_ref(),
        ) else {
            return;
        };
        let __t = Instant::now();
        #[cfg(windows)]
        let mut mirrored = false;
        #[cfg(not(windows))]
        let mirrored = false;
        #[cfg(windows)]
        if let Some(vr) = self.xr.vr.as_mut() {
            let menu_range = self.ui.as_ref().map(|u| u.menu_overlay_range.clone()).unwrap_or(0..0);
            let cursor_overlay = self.ui.as_ref().and_then(|u| u.vr_cursor_overlay).filter(|_| self.xr.vr_nav_edit.is_none());
            let tooltip_overlay = self.ui.as_ref().and_then(|u| u.vr_tooltip_overlay).filter(|_| self.xr.vr_nav_edit.is_none());
            match vr.render(
                r,
                scene,
                cam,
                &lighting,
                &view,
                (s.config.width, s.config.height),
                menu_range,
                cursor_overlay,
                tooltip_overlay,
                self.input.cursor,
                self.player.as_ref().map(|p| (p.vehicle.position, p.vehicle.body_rotation())),
                vr_nav_display.filter(|d| d.placement.enabled).and_then(|d| {
                    self.menus.navigator.as_ref().and_then(|n| n.panel_overlay).map(|index| (index, d))
                }),
                self.player.as_ref().map(|p| p.uid),
                self.settings.vr_head_smoothing_ms,
                !self.input.mouse_drive,
                self.xr.vr_zoom_active,
            ) {
                Ok(visible) => mirrored = visible,
                Err(e) => {
                    log::error!("OpenXR rendering stopped: {e:#}");
                    self.xr.vr = None;
                }
            }
        }
        if self.cam.in_cab {
            if let Some(w) = self.world.as_ref() {
                self.gfx.mirror_hud.ensure_frame(r, scene);
                let hud = self
                    .settings
                    .hud_viewport((s.config.width, s.config.height));
                steps::push_mirror_hud(&self.gfx.mirror_hud, scene, w, hud, (self.input.cursor.0 - hud[0], self.input.cursor.1));
            }
        }
        if !mirrored
            && self.settings.triple.enabled
            && !self.settings.vr_requested()
        {
            let rig = self.settings.triple.zoomed(
                s.config.width,
                s.config.height,
                self.cam.view_zoom.get(&self.view).copied().unwrap_or(1.0),
            );
            r.render_triple(
                scene,
                &view,
                s.config.width,
                s.config.height,
                cam,
                lighting,
                &rig,
            );
        } else if !mirrored {
            r.render(
                scene,
                &view,
                s.config.width,
                s.config.height,
                cam,
                lighting,
            );
        }
        // the on-screen controls over the picture (a phone)
        self.input.touch.render(r, &view, s.config.width, s.config.height);
        *self.perf.profile.entry("render").or_default() += __t.elapsed().as_secs_f64();
        if omsi_cfg::flags::OMSI_PROFILE_GPU.is_set() {
            // wait for the GPU here, so that its time shows as a stage of its own
            let __t = Instant::now();
            let _ = omsi_render::wait_gpu(&r.device, None);
            *self.perf.profile.entry("gpu").or_default() += __t.elapsed().as_secs_f64();
        }
        let __t = Instant::now();
        match frame {
            Some(frame) => {
                // (without V-sync max_fps paces the frames: waiting for the compositor's frame callback cost a missed refresh each slow frame)
                if self.settings.vsync {
                    win.pre_present_notify();
                }
                frame.present();
            }
            None => {
                let _ = omsi_render::wait_gpu(&r.device, None);
            }
        }
        *self.perf.profile.entry("present").or_default() += __t.elapsed().as_secs_f64();
    }

    /// The bus's mirrors redrawn, in turn, within their budget.
    fn frame_mirrors(&mut self, raw_dt: f32, lighting: &omsi_render::Lighting) {
        let (Some(s), Some(r), Some(scene), Some(cam)) = (
            self.gfx.surface.as_ref(),
            self.renderer.as_mut(),
            self.scene.as_mut(),
            self.camera.as_ref(),
        ) else {
            return;
        };
        let __t = Instant::now();
        // One mirror a turn, in turn, at most MIRROR_RATE pictures a second in
        // all: a mirror costs half the main picture's CPU time, and at 140 fps
        // five mirrors were each redrawn 28 times a second, a small picture
        // that nobody can tell from 15.
        // Every mirror at least MIRROR_MIN_HZ, though: with eight of them (the
        // Procity) at 25 fps each was redrawn three times a second, and the
        // street jerked past in them - up to two a frame then (each costs a
        // few milliseconds of the frame).
        if let (Some(p), Some(w)) = (self.player.as_ref(), self.world.as_ref()) {
            steps::mirror_hud_sync(&mut self.gfx.mirror_hud, w, p, self.settings.mirror_hud);
        }
        if self.settings.mirror_size == 0 {
            self.gfx.mirror_budget = 0.0;
            self.gfx.mirrors_seen = 0;
        } else if self.settings.mirror_refresh == "off" {
            self.gfx.mirror_budget = 0.0;
            if let (Some(w), Some(p)) = (self.world.as_ref(), self.player.as_ref()) {
                let since = match &self.gfx.frozen_mirrors {
                    Some(m) if m.bus == p.uid => m.since,
                    _ => -1.0,
                };
                let next = since.max(0.0) + raw_dt.min(0.1);
                // (and while the driver turns a mirror, so it can be aimed)
                if since < 0.0 || (since < MIRROR_FREEZE_REDRAW && next >= MIRROR_FREEZE_REDRAW) || p.mirrors_dirty {
                    self.gfx.mirrors_seen = render_mirrors(r, scene, w, p, lighting, None, None);
                }
                self.gfx.frozen_mirrors = Some(FrozenMirrors { bus: p.uid, since: next });
            }
        } else {
            let mirrors = self.player.as_ref().map(|p| p.vehicle.ty.def.cameras_reflexion.len()).unwrap_or(0);
            #[cfg(windows)]
            let vr_active = self.xr.vr.is_some();
            #[cfg(not(windows))]
            let vr_active = false;
            let rate = {
                if vr_active {
                    // Preserve the user's total redraw budget. A negative
                    // value explicitly requests every mirror each frame.
                    omsi_cfg::flags::OMSI_OPENXR_MIRROR_RATE.parse::<f32>()
                        .filter(|rate| rate.is_finite() && *rate >= -1.0)
                        .unwrap_or(self.settings.vr_mirror_rate)
                } else {
                    let max_hz = if self.settings.mirror_refresh == "full" { MIRROR_MAX_HZ_FULL } else { MIRROR_MAX_HZ_ECO };
                    MIRROR_RATE.max(mirrors as f32 * MIRROR_MIN_HZ).min(max_hz * self.gfx.mirrors_seen.max(1) as f32)
                }
            };
            // The desktop camera does not follow the headset. Culling by
            // its frustum can leave a mirror visible in VR uninitialised
            // (black). Refresh all bus mirrors in VR, still taking turns
            // within the configured budget; keep desktop visibility culling.
            let mirror_view = if vr_active
                || self.settings.triple.enabled
                || (self.gfx.mirror_hud.active() && self.cam.in_cab)
            {
                None
            } else {
                Some((*cam, s.config.width as f32 / s.config.height.max(1) as f32))
            };
            // (in the cab, and from outside too while the bus is near: its
            // mirrors are seen from the pavement and stood frozen)
            let near = self.player.as_ref().zip(self.camera.as_ref()).is_some_and(|(p, c)| (p.vehicle.position - c.position).length() < 12.0);
            let draw_limit = if vr_active {
                if self.cam.in_cab || near {
                    vr_mirror_updates(&mut self.gfx.mirror_budget, raw_dt, rate, mirrors)
                } else {
                    self.gfx.mirror_budget = 0.0;
                    0
                }
            } else {
                self.gfx.mirror_budget = (self.gfx.mirror_budget + raw_dt.min(0.1) * rate).min(2.5);
                self.gfx.mirrors_seen.clamp(1, 2)
            };
            let mut drawn = 0;
            if vr_active && draw_limit > 0 && draw_limit == mirrors {
                // Prepare the cameras and textures only once when all
                // mirrors are due, including the Every frame mode.
                if let (Some(w), Some(p)) = (self.world.as_ref(), self.player.as_ref()) {
                    self.gfx.mirror_turn = self.gfx.mirror_turn.wrapping_add(draw_limit);
                    self.gfx.mirrors_seen = render_mirrors(r, scene, w, p, lighting, None, mirror_view);
                    drawn = draw_limit;
                }
            }
            while (self.cam.in_cab || near) && drawn < (if vr_active { draw_limit } else { self.gfx.mirrors_seen.clamp(1, 2) }) && (vr_active || self.gfx.mirror_budget >= 1.0) {
                let (Some(w), Some(p)) = (self.world.as_ref(), self.player.as_ref()) else { break };
                if !vr_active {
                    self.gfx.mirror_budget -= 1.0;
                }
                drawn += 1;
                self.gfx.mirror_turn = self.gfx.mirror_turn.wrapping_add(1);
                self.gfx.mirrors_seen = render_mirrors(
                    r,
                    scene,
                    w,
                    p,
                    lighting,
                    Some(self.gfx.mirror_turn),
                    mirror_view,
                );
            }
        }
        *self.perf.profile.entry("mirrors").or_default() += __t.elapsed().as_secs_f64();
    }

    /// Nothing to draw into: what was uploaded let go, the simulation at a display's pace.
    fn frame_hidden(&mut self, now: Instant) {
        let Some(r) = self.renderer.as_ref() else { return };
        // Nothing to draw into (a hidden window): the simulation goes on at
        // a display's pace instead of spinning a core a thousand times a
        // second. What it uploaded (traffic instances, streamed tiles,
        // people, the navigator) waits in wgpu's staging buffers until the
        // next submit, so submit nothing to let them go: without it a hidden
        // window on Ahlheim grew by 100 MB a second (5.6 GB after 55 s).
        let __t = Instant::now();
        r.queue.submit(std::iter::empty::<wgpu::CommandBuffer>());
        let _ = r.device.poll(wgpu::PollType::Poll);
        *self.perf.profile.entry("present").or_default() += __t.elapsed().as_secs_f64();
        if let Some(rest) =
            std::time::Duration::from_millis(16).checked_sub(now.elapsed())
        {
            std::thread::sleep(rest);
        }
    }

    /// The frame rate's limit: the rest of the frame's time slept.
    fn frame_pace(&mut self, now: Instant, shown_nothing: bool) {
        // max_fps (the original's [maxFPS]; OMSI_MAX_FPS for a test): the rest of
        // the frame's time is slept, not spun, so a limit gives the CPU back
        // (and keeps a laptop cool enough not to slow itself down)
        let max_fps = omsi_cfg::flags::OMSI_MAX_FPS.parse::<u32>()
            .unwrap_or(self.settings.max_fps);
        // 0 = the screen's refresh rate: frames the screen never shows only heat the
        // machine (with V-sync off and no limit an M4 drew 300 frames a second in the
        // depot and ran hot); 1000 and more = no limit at all
        let max_fps = if max_fps == 0 {
            self.window.as_ref().and_then(|w| w.current_monitor()).and_then(|m| m.refresh_rate_millihertz()).map(|mhz| (mhz as f64 / 1000.0).round() as u32).filter(|r| *r >= 30).unwrap_or(120)
        } else if max_fps >= 1000 {
            0
        } else {
            max_fps
        };
        // a frame not shown (the window minimised or out of sight): 30 frames a
        // second keep the simulation and the sound going; more is only heat
        // (OMSI_RENDER_OCCLUDED, which draws them anyway, keeps its pace)
        let max_fps = if shown_nothing { if max_fps == 0 { 30 } else { max_fps.min(30) } } else { max_fps };
        #[cfg(windows)]
        let vr_active = self.xr.vr.is_some();
        #[cfg(not(windows))]
        let vr_active = false;
        if max_fps > 0 && !vr_active {
            let __t = Instant::now();
            if let Some(rest) = std::time::Duration::from_secs_f64(1.0 / max_fps as f64)
                .checked_sub(now.elapsed())
            {
                std::thread::sleep(rest);
            }
            *self.perf.profile.entry("limiter").or_default() += __t.elapsed().as_secs_f64();
        }
    }

    /// The frames counted, `--exit-after` with its statistics (true: the session ends), and
    /// the window's title once a second.
    fn frame_count(&mut self, event_loop: &ActiveEventLoop) -> bool {
        let (Some(s), Some(r), Some(cam), Some(win)) = (
            self.gfx.surface.as_ref(),
            self.renderer.as_mut(),
            self.camera.as_ref(),
            self.window.as_ref(),
        ) else {
            return false;
        };
        let mut finish = false;
        self.perf.frames += 1;
        let profiling = omsi_cfg::flags::OMSI_PROFILE.is_set();
        if profiling
            && self.perf.cpu_mark.is_none()
            && self.started.elapsed().as_secs_f32() > 15.0
        {
            self.perf.cpu_mark =
                process_cpu_seconds().map(|c| (c, Instant::now(), self.perf.total_frames));
            self.perf.profile_mark = Some(crate::perf_report::ProfileMark::take(&self.perf.profile, r));
        }
        if let (Some(limit), false) = (self.args.exit_after, self.exiting) {
            if self.started.elapsed().as_secs_f32() > limit {
                self.exiting = true;
                log::info!("exit after {limit} s: {} frames total ({} with the window hidden{}), {:.1} fps average, {} frames over 50 ms, worst {:.0} ms", self.perf.total_frames, self.gfx.hidden_frames, if omsi_cfg::flags::OMSI_RENDER_OCCLUDED.is_set() { ", drawn off-screen" } else { ", not drawn" }, self.perf.total_frames as f32 / self.started.elapsed().as_secs_f32(), self.perf.spikes, self.perf.worst_ms);
                if let (Some(st), Some(w)) =
                    (self.gfx.streamer.as_ref(), self.world.as_ref())
                {
                    log::info!("tile streaming: {} tiles loaded now, {} loaded and {} unloaded in all, {:.1} s preparing on the worker, slowest upload {:.0} ms, streaming over 16 ms in {} frames (worst {:.0} ms); {} objects + {} trees, {} rows, {} attached ({} without parent), {} unresolved", w.loaded_tiles().len(), st.loaded_total, st.unloaded_total, st.prepare_secs, st.worst_upload_ms, st.slow_frames, st.worst_frame_ms, st.stats.objects, st.stats.trees, st.stats.rows, st.stats.attached, st.stats.unattached, st.stats.failed_objects);
                    st.stats.log_ground();
                }
                if omsi_cfg::flags::OMSI_PROFILE.is_set() {
                    let n = self.perf.total_frames.max(1) as f64;
                    for (k, v) in &self.perf.profile {
                        log::info!("profile {k:10}: {:.1} ms/frame", v / n * 1000.0);
                    }
                    if let Some(h) = self.session.humans.as_ref() {
                        log::info!(
                            "profile people: {} ({})",
                            h.people.len(),
                            h.summary(&self.gfx.sim_view.people)
                        );
                    }
                    for (k, v) in r.stats.borrow().iter() {
                        log::info!(
                            "profile render.{k:10}: {:.2} ms/frame",
                            v / n * 1000.0
                        );
                    }
                    for (k, v) in r.counts.borrow().iter() {
                        log::info!("profile count {k}: {:.0} a frame", v / n);
                    }
                    for (pass, ms, frames) in r.gpu_pass_times() {
                        log::info!("profile gpu pass {pass:12}: {ms:.2} ms ({frames} frames measured)");
                    }
                    if let (Some((c0, t0, f0)), Some(c1)) =
                        (self.perf.cpu_mark, process_cpu_seconds())
                    {
                        let frames = self.perf.total_frames.saturating_sub(f0).max(1) as f64;
                        log::info!("profile: since 15 s {:.1} ms wall and {:.1} ms CPU (all threads) per frame, {:.1} cores busy", t0.elapsed().as_secs_f64() / frames * 1000.0, (c1 - c0) / frames * 1000.0, (c1 - c0) / t0.elapsed().as_secs_f64().max(1e-3));
                    }
                    let (sw, sh) = r.scene_size(s.config.width, s.config.height);
                    log::info!(
                        "profile: window {}x{}, scene drawn at {sw}x{sh}, {}x MSAA",
                        s.config.width,
                        s.config.height,
                        r.options.msaa
                    );
                    let frames = crate::perf_report::frame_summary(&self.perf.frame_times);
                    log::info!("{}", crate::perf_report::log_line(&frames));
                    if let Some(path) = omsi_cfg::flags::OMSI_PROFILE_JSON.var() {
                        let report = crate::perf_report::report(crate::perf_report::Run { perf: &self.perf, settings: &self.settings, args: &self.args, lan: self.net.lan.is_some() }, r, (s.config.width, s.config.height), (sw, sh), frames);
                        match serde_json::to_vec_pretty(&report).map_err(anyhow::Error::from).and_then(|b| Ok(std::fs::write(path, b)?)) {
                            Ok(()) => log::info!("profile: the summary is in {path}"),
                            Err(e) => log::warn!("profile: the summary could not be written to {path}: {e}"),
                        }
                    }
                }
                finish = true;
                crate::platform::exit(event_loop);
            }
        }
        self.perf.total_frames += 1;
        if self.perf.fps_t.elapsed().as_secs_f32() >= 1.0 {
            if omsi_cfg::flags::OMSI_PROFILE.is_set() {
                let secs = self.perf.fps_t.elapsed().as_secs_f32();
                log::info!("profile interval: {:.1} fps over {secs:.2} s", self.perf.frames as f32 / secs);
            }
            let speed = self
                .player
                .as_ref()
                .map(|p| format!(" - {:.0} km/h", p.vehicle.physics.velocity_kmh()))
                .unwrap_or_default();
            self.perf.fps = self.perf.frames as f32;
            win.set_title(&format!(
                "openOMSI - {} fps{speed} - {:.0},{:.0},{:.0} yaw {:.0}",
                self.perf.frames,
                cam.position.x,
                cam.position.y,
                cam.position.z,
                cam.yaw.rem_euclid(360.0)
            ));
            self.perf.frames = 0;
            self.perf.fps_t = Instant::now();
        }
        finish
    }
}
