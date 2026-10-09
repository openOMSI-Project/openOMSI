//! The start of a frame: the test hooks and fallbacks, the frame's modes and targets, and
//! the camera's uniforms.

use super::*;

impl Renderer {
    /// The test hooks and the fallbacks after a GPU error. False: the device is gone and
    /// nothing is drawn.
    pub(crate) fn frame_hooks(&mut self, scene: &mut Scene, env: &FrameEnv, with_overlays: bool) -> bool {
        #[cfg(not(feature = "test-hooks"))]
        let _ = (env, with_overlays);
        // test hook for a lost device (a driver reset): its resources are taken away and
        // the session has to end in order
        #[cfg(feature = "test-hooks")]
        if env.fake_gpu_error == Some("lost")
            && with_overlays
            && self.started.elapsed().as_secs_f32() > 3.0
            && self.device_lost().is_none()
        {
            log::error!("the graphics device was lost (test): OMSI_FAKE_GPU_ERROR=lost");
            self.device.destroy();
            *self.device_lost.lock().unwrap_or_else(|e| e.into_inner()) = Some("test".into());
        }
        // the device is gone: nothing can be drawn, and the readbacks (the exposure meter)
        // would find their buffers taken away - "Error in Buffer::get_mapped_range:
        // Validation Error" ended the game instead of the session ending in order
        if self.device_lost.lock().unwrap_or_else(|e| e.into_inner()).is_some() {
            return false;
        }
        #[cfg(feature = "test-hooks")]
        if env.fake_gpu_error == Some("frame")
            && self.options.msaa > 1
            && self.started.elapsed().as_secs_f32() > 3.0
        {
            // test hook for the fallback below
            let _ = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("invalid"),
                size: wgpu::Extent3d {
                    width: 4,
                    height: 4,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 3,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
        }
        if self.rt_error.load(std::sync::atomic::Ordering::Relaxed) {
            self.fall_back_without_ray_tracing(scene);
        } else if self.gpu_error.load(std::sync::atomic::Ordering::Relaxed) {
            self.fall_back_to_single_sample(scene);
        }
        true
    }

    /// The frame's modes (render scale, Enhanced, the reflections, the prepass, ray
    /// tracing) and the targets they need, the lights, coronas and smoke prepared.
    pub(crate) fn begin_frame<'a>(&mut self, scene: &mut Scene, env: FrameEnv, a: FrameArgs<'a>, clock: &mut StageClock) -> FrameCtx<'a> {
        let FrameArgs { target, width, height, camera, lighting, with_overlays, exclude_texture, projection, second_eye } = a;
        // render origin: the camera position rounded to 100 m, kept while the camera stays near it
        if (camera.position - scene.render_origin).abs().max_element() > 200.0 {
            self.set_render_origin(scene, (camera.position / 100.0).floor() * 100.0);
        }
        let ro = scene.render_origin;
        // The window's 3D picture may be drawn smaller and scaled up to it (render scale):
        // from here on `width` and `height` are the size of the picture, `full_*` the
        // window's (the HUD is drawn at that size). Mirrors keep their own size.
        let (full_w, full_h) = (width, height);
        let (width, height) = if with_overlays {
            self.scene_size(full_w, full_h)
        } else {
            (full_w, full_h)
        };
        // FXAA on the plain graphics too, where there is no multisampling to smooth the
        // edges (the Enhanced path has it in its post passes): the picture is drawn into a
        // texture of the window's size and smoothed on its way to the window, the HUD after
        let vanilla_fxaa = with_overlays
            && (width, height) == (full_w, full_h)
            && self.options.fxaa
            && self.options.msaa <= 1
            && !(lighting.enhanced && self.hdr_pass.is_some() && !env.no_enhanced)
            && !env.no_fxaa;
        // Rain films are drawn after copying the clean current-frame scene. Classic
        // graphics need a sampleable scene target too, even without puddles or scaling.
        let enhanced_view = lighting.enhanced && self.hdr_pass.is_some() && !env.no_enhanced;
        let glass_on = with_overlays
            && scene.glass_slot.is_some()
            && (lighting.rain > 0.001 || lighting.wetness > 0.02)
            && !env.no_glass_picture;
        let scaled = (width, height) != (full_w, full_h) || vanilla_fxaa || (glass_on && !enhanced_view);
        let scene_target: Option<(wgpu::TextureView, wgpu::BindGroup)> = if scaled {
            Some(self.scale_target(width, height))
        } else {
            None
        };
        let aspect = self.texture_aspect.unwrap_or(width as f32 / height.max(1) as f32);
        let cam_rel = (camera.position - ro).as_vec3();
        // The mirrors are drawn with the plain shading even in Enhanced: without the depth
        // prepass (sized for the window) every layer of a mirror's picture ran the enhanced
        // shader, 12.7 ms of GPU time for one 256-pixel mirror against 5.8 ms for the whole
        // window; plainly shaded it is 0.6 ms, and a mirror's small picture shows no
        // difference worth that. `OMSI_MIRROR_ENHANCED=1` draws them enhanced again.
        // The headset's eyes are the real picture as much as the window is (#784: VR showed
        // the plain graphics with Enhanced on); the first eye is the one that moves the
        // exposure, the sky cube and the frame clock on, as the window does without VR.
        let xr_view = projection.is_some();
        let lead_view = (with_overlays || xr_view) && !second_eye;
        let enhanced_frame = lighting.enhanced && self.hdr_pass.is_some() && !env.no_enhanced && (with_overlays || xr_view || env.mirror_enhanced);
        // the mirrors are drawn by the same path as the window (their picture graded with
        // the window's exposure, see the post passes)
        let enhanced = enhanced_frame;
        // Rain reflections belong to all graphics modes. Classic shading uses the same
        // scene/mask targets, then presents their linear colour without Enhanced grading.
        let puddles_wanted = with_overlays
            && self.puddles.is_some()
            && self.options.reflections
            && lighting.wetness * (1.0 - lighting.snow.clamp(0.0, 1.0)) > 0.05
            && scene.materials.iter().any(|m| m.uniform.params2[2] > 0.0)
            && debug_view() == 0.0
            && !env.no_puddle_reflections;
        let reflection_frame = !enhanced && puddles_wanted && self.reflection_pass.is_some();
        let masked_frame = enhanced || reflection_frame;
        let (grid, lamp_shadows) = self.prepare_lights(scene, cam_rel, enhanced_frame, lighting.lamp_shadows && with_overlays && projection.is_none() && self.shadow_pipelines.len() > 6);
        self.prepare_coronas(scene, lighting.night, lighting.inside.as_ref().filter(|v| point_in_vehicle_box(camera.position, v)));
        self.prepare_smoke(scene, camera.position);
        // ambient occlusion only for the real picture, not for the mirrors
        // Enhanced+: the window's picture traces its sun shadow and ambient occlusion (not the
        // mirrors, a headset's eyes or a triple screen's panels: they keep the shadow map)
        let rt_frame = self.rt.is_some() && enhanced && with_overlays && projection.is_none() && !env.no_rt_frame;
        let ao_on = with_overlays && self.rt.is_none() && self.options.ssao && self.ssao_pipeline.is_some() && !env.no_ao;
        // the enhanced path's shading is costly: the depth prepass keeps it to the visible
        // surface (without multisampling, see `share_depth`)
        let prepass_on = ao_on || puddles_wanted || glass_on || (enhanced && (with_overlays || xr_view));
        if prepass_on && self.ensure_ao(width, height) {
            // a new AO texture: the camera bind group must point at it
            scene.dirty = true;
            scene.model_buf = None;
            self.hdr_targets.clear();
        }
        if rt_frame && self.rt.as_mut().is_some_and(|rt| rt.ensure_targets(&self.device, width, height)) {
            // the camera bind group must point at the new lighting texture
            scene.dirty = true;
            scene.model_buf = None;
        }
        if masked_frame {
            self.hdr_targets(width, height);
        }
        if glass_on {
            self.prepare_glass_behind(scene, width, height, if masked_frame { HDR_FORMAT } else { self.format });
        }
        let dt = {
            let now = std::time::Instant::now();
            let dt = self
                .last_frame
                .map(|t| (now - t).as_secs_f32())
                .unwrap_or(0.0);
            if lead_view {
                self.last_frame = Some(now);
            }
            dt
        };
        clock.stage(self, "setup", "mirror.setup");
        self.prepare(scene);
        clock.stage(self, "prepare", "mirror.prepare");
        // overlay uniforms/bind groups (rects in pixels → NDC)
        let overlays: Vec<(TextureId, [f32; 4])> = if with_overlays {
            scene.overlays.clone()
        } else {
            Vec::new()
        };
        if with_overlays {
            self.prepare_overlays(scene, full_w, full_h);
        }
        FrameCtx {
            env,
            target,
            camera,
            lighting,
            with_overlays,
            exclude_texture,
            projection,
            second_eye,
            ro,
            full_w,
            width,
            height,
            vanilla_fxaa,
            glass_on,
            scaled,
            scene_target,
            aspect,
            cam_rel,
            xr_view,
            lead_view,
            enhanced_frame,
            enhanced,
            puddles_wanted,
            reflection_frame,
            masked_frame,
            grid,
            lamp_shadows,
            rt_frame,
            ao_on,
            prepass_on,
            dt,
            overlays,
        }
    }

    /// The camera's uniforms (and the light maps' place), the ray tracing's preparation,
    /// the lamps' light on the view and the sky. Returns the view-projection matrix, the
    /// uniform and whether the reflection probe is redrawn this frame.
    pub(crate) fn frame_uniforms(&mut self, scene: &Scene, f: &FrameCtx, sh: &ShadowPlan, clock: &mut StageClock) -> (Mat4, CameraUniform, bool) {
        let (camera, lighting, ro, cam_rel, aspect) = (f.camera, f.lighting, f.ro, f.cam_rel, f.aspect);
        let (enhanced, lead_view, rt_frame, ao_on, glass_on, shadows) = (f.enhanced, f.lead_view, f.rt_frame, f.ao_on, f.glass_on, sh.shadows);
        let lamp_shadows = &f.lamp_shadows;
        let vp_mat = f
            .projection
            .map(|p| p * Mat4::look_to_rh((camera.position - ro).as_vec3(), camera.forward(), camera.up()))
            .unwrap_or_else(|| camera.view_proj(aspect, ro));
        // where the tile light maps lie, relative to the render origin
        {
            let (lx, ly, side) = self.lm_place.get();
            let v: [f32; 4] = [(lx - ro.x) as f32, (ly - ro.y) as f32, side as f32, if side > 0.0 { 1.0 } else { 0.0 }];
            self.queue.write_buffer(&self.lm_uniform, 0, bytemuck::cast_slice(&v));
        }
        {
            let v = &lighting.vanilla_sky;
            let u = VanillaSkyUniform {
                cloud: [v.cloud_height, v.cloud_size, v.cloud_offset[0], v.cloud_offset[1]],
                haze: [v.fog_range, v.visibility, if v.overcast { 1.0 } else { 0.0 }, ro.z as f32],
            };
            self.queue.write_buffer(&self.vanilla_sky_buf, 0, bytemuck::bytes_of(&u));
        }
        let cu = CameraUniform {
            post: [
                if enhanced { 1.0 } else { 0.0 },
                lighting.animation_time.unwrap_or_else(|| self.started.elapsed().as_secs_f32()),
                // (z: the roads are kept clear of the snow, `Lighting::roads_clear`)
                if lighting.roads_clear { 1.0 } else { 0.0 },
                // (the sun's height on the screen is read by no shader any more: the close
                // shadow map's share of its half of the atlas)
                self.options.shadow_size.min(SHADOW_CLOSE_MAX) as f32 / self.options.shadow_size.max(1) as f32,
            ],
            view_proj: vp_mat.to_cols_array_2d(),
            cam_pos: cam_rel.extend(1.0).to_array(),
            // (modulo the shaders' PATTERN_PERIOD, 1000 m: the patterns repeat with it, and
            // the whole map coordinate has no precision left for them in 32 bits)
            // (zw: the origin modulo CLOUD_ORIGIN_PERIOD for the sky's clouds, which are
            // drawn over ground points: taken relative to the floating origin, the whole
            // cloud field jumped by the origin's step each time it moved on)
            world_origin: [
                ro.x.rem_euclid(1000.0) as f32,
                ro.y.rem_euclid(1000.0) as f32,
                ro.x.rem_euclid(CLOUD_ORIGIN_PERIOD) as f32,
                ro.y.rem_euclid(CLOUD_ORIGIN_PERIOD) as f32,
            ],
            sun_dir: lighting
                .sun_dir
                .normalize()
                .extend(lighting.sun_intensity)
                .to_array(),
            ambient: lighting
                .ambient
                .extend(lighting.snow.clamp(0.0, 1.0))
                .to_array(),
            fog: lighting.fog_color.extend(lighting.fog_density).to_array(),
            sun_color: lighting.sun_color.extend(lighting.night_maps.unwrap_or(lighting.night)).to_array(),
            sky_color: lighting
                .secondary
                .extend(if lighting.classic && !enhanced { 1.0 } else { 0.0 })
                .to_array(),
            light_grid: f.grid,
            sky: [
                lighting.sun_azimuth,
                lighting.sky_weights[0],
                lighting.sky_weights[1],
                lighting.sky_weights[2],
            ],
            clouds: [
                lighting.cloud_density,
                lighting.cloud_offset[0],
                lighting.cloud_offset[1],
                // (2: the traced lighting, full size, see `ao_at`)
                if rt_frame { 2.0 } else if ao_on { 1.0 } else { 0.0 },
            ],
            // (w: the heading the sphere maps are laid out by in the headset, see
            // `set_env_heading`; flagged by cam_up.w)
            cam_right: camera.right().extend(self.env_heading.get().map(|h| h.to_radians()).unwrap_or(0.0)).to_array(),
            cam_up: camera
                .right()
                .cross(camera.forward())
                .normalize_or_zero()
                .extend(if self.env_heading.get().is_some() { 1.0 } else { 0.0 })
                .to_array(),
            light_view_proj: sh.light_view_proj.to_cols_array_2d(),
            light_view_proj_far: sh.light_view_proj_far.to_cols_array_2d(),
            shadow: [
                if shadows { 1.0 } else { 0.0 },
                1.0 / self.options.shadow_size as f32,
                SHADOW_RANGE,
                lighting.wetness.clamp(0.0, 1.0),
            ],
            inside_a: match lighting.inside {
                Some((o, h, _)) => {
                    let r = (o - ro).as_vec3();
                    [r.x, r.y, r.z, (h as f32).to_radians().sin()]
                }
                None => [0.0; 4],
            },
            inside_b: match lighting.inside {
                Some((_, h, bb)) => [
                    (h as f32).to_radians().cos(),
                    bb[0] * 0.5,
                    bb[1] * 0.5,
                    bb[2] * 0.5,
                ],
                None => [1.0, 0.0, 0.0, 0.0],
            },
            inside_c: match lighting.inside {
                Some((_, _, bb)) => [bb[3], bb[4], bb[5], 1.0],
                None => [0.0; 4],
            },
            flags: [
                if lighting.detail { 1.0 } else { 0.0 },
                if enhanced { 1.0 } else { 0.0 },
                // (below zero: the rain films have the clean current picture to look through,
                // see `rain_behind`; above zero is an old branch never taken)
                if glass_on { -1.0 } else { 0.0 },
                if shadows { SHADOW_RANGE_CLOSE } else { 0.0 },
            ],
            light_view_proj_close: sh.light_view_proj_close.to_cols_array_2d(),
            wind: [lighting.glass_wind.x, lighting.glass_wind.y, lighting.glass_wind.z, 1.0],
            lamp_view_proj: std::array::from_fn(|k| lamp_shadows.get(k).map_or(Mat4::IDENTITY, |l| l.view_proj()).to_cols_array_2d()),
            lamp_shadow: std::array::from_fn(|k| lamp_shadows.get(k).map_or(-1.0, |l| l.index as f32)),
            tree_wind: {
                // the gusts drift with the wind: summed over the frames (the wind changes
                // with the weather), in f64 and modulo the shaders' pattern period
                let now = self.started.elapsed().as_secs_f64();
                let (drift, last) = self.tree_gust_drift.get();
                let drift = (drift + lighting.wind.truncate().as_dvec2() * (now - last).clamp(0.0, 1.0)).rem_euclid(glam::DVec2::splat(1000.0));
                self.tree_gust_drift.set((drift, now));
                let wind = if lighting.windy_trees { lighting.wind.truncate() } else { glam::Vec2::ZERO };
                [wind.x, wind.y, drift.x as f32, drift.y as f32]
            },
            inside2_a: match lighting.puddle_parts.first() {
                Some((o, h, _)) => {
                    let r = (*o - ro).as_vec3();
                    [r.x, r.y, r.z, (*h as f32).to_radians().sin()]
                }
                None => [0.0; 4],
            },
            inside2_b: match lighting.puddle_parts.first() {
                Some((_, h, bb)) => [(*h as f32).to_radians().cos(), bb[0] * 0.5, bb[1] * 0.5, bb[2] * 0.5],
                None => [1.0, 0.0, 0.0, 0.0],
            },
            inside2_c: match lighting.puddle_parts.first() {
                Some((_, _, bb)) if lighting.inside.is_some() => [bb[3], bb[4], bb[5], 1.0],
                _ => [0.0; 4],
            },
        };
        self.queue
            .write_buffer(&self.camera_buf, 0, bytemuck::bytes_of(&cu));
        if rt_frame {
            let proj = Mat4::perspective_rh(camera.fov_deg.to_radians(), aspect, camera.far, camera.near);
            self.prepare_ray_tracing(scene, camera, lighting, vp_mat, proj, f.width, f.height, f.dt);
            clock.stage(self, "ray tracing", "mirror.ray tracing");
        }
        // (a mirror takes the window's light - its own call would move the exposure on -
        // unless it comes before the window's first frame)
        if enhanced && lead_view {
            let ground = lighting.fog_base.or(lighting.inside.map(|v| v.0.z)).map(|z| (z - scene.render_origin.z) as f32);
            self.view_lamps = Some(view_lamp_light(&scene.lights, scene.render_origin, cam_rel, camera, aspect, ground));
            if f.env.debug_view_lamps {
                log::info!("view lamps: {:.6}", self.view_lamps.unwrap_or(0.0));
            }
        }
        // the night sky's glow from the lamps round the camera (the window's view leads), in
        // steps of a tenth: the sky is recomputed for a new value, not for every metre driven
        if enhanced && lead_view {
            let raw = lamp_sky_glow(scene, cam_rel);
            let target = raw.clamp(0.03, 1.5);
            let step = (target.ln() * 10.0).round() / 10.0;
            let changed = self.city_glow.is_none_or(|g| (g.ln() - step).abs() > 0.15);
            if changed {
                self.city_glow = Some(step.exp());
            }
            if f.env.debug_sky && changed {
                log::info!("sky glow from the lamps: {raw:.3} (taken {:.3})", step.exp());
            }
        }
        let probe_redraw = enhanced
            && (lead_view || self.sky_state.is_none())
            && self.prepare_enhanced(lighting, cam_rel, ro, f.dt);
        (vp_mat, cu, probe_redraw)
    }
}
