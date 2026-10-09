//! The scene shader, its bind group layouts and the main pass's mesh pipelines; the depth
//! prepass drawn with the same vertex shader.

use super::common::*;
use crate::*;

/// The scene shader and what every pipeline drawing meshes with it shares.
pub(crate) struct SceneBase {
    pub shader: wgpu::ShaderModule,
    pub shadow_layout: wgpu::BindGroupLayout,
    pub camera_layout: wgpu::BindGroupLayout,
    pub lm_atlas: wgpu::Texture,
    pub lm_uniform: wgpu::Buffer,
    pub material_layout: wgpu::BindGroupLayout,
    pub corona_layout: wgpu::BindGroupLayout,
    layout: wgpu::PipelineLayout,
    pub vertex_layout: wgpu::VertexBufferLayout<'static>,
    bias: i32,
}

const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2];

impl SceneBase {
    /// `omit_enhanced`: the camera group without the enhanced path's textures (see
    /// `camera_layout_entries`).
    pub(crate) fn new(device: &wgpu::Device, omit_enhanced: bool) -> SceneBase {
        // One module for both paths: the enhanced fragment shader shares the vertex shader,
        // which the depth prepass relies on to the last bit (see `VsOut::clip`).
        log::info!("renderer: compiling the scene shaders");
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("omsi"),
            source: wgpu::ShaderSource::Wgsl(
                scene_shader_source(GL_BACKEND.load(std::sync::atomic::Ordering::Relaxed)).into(),
            ),
        });
        let shadow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow camera"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::VERTEX_FRAGMENT),
                array_layout_entry(1, wgpu::ShaderStages::VERTEX, true),
                array_layout_entry(2, wgpu::ShaderStages::VERTEX, true),
                array_layout_entry(10, wgpu::ShaderStages::VERTEX, false),
            ],
        });
        let camera_entries = camera_layout_entries(array_path(), omit_enhanced);
        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera"),
            entries: &camera_entries,
        });
        let lm_atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("light map atlas"),
            size: wgpu::Extent3d { width: LM_ATLAS_TILES * LM_TILE_PX, height: LM_ATLAS_TILES * LM_TILE_PX, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let lm_uniform = uniform_buffer(device, "light map atlas place", 16);
        let material_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("material"),
            entries: &material_layout_entries(),
        });
        // coronas: camera group + a corona texture group
        let corona_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("corona"),
            entries: &[float_texture_entry(0), sampler_entry(1)],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("omsi"),
            bind_group_layouts: &[Some(&camera_layout), Some(&material_layout)],
            immediate_size: 0,
        });
        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &VERTEX_ATTRIBUTES,
        };
        let bias: i32 = omsi_cfg::flags::OMSI_SURFACE_BIAS.parse()
            .unwrap_or(-24);
        SceneBase { shader, shadow_layout, camera_layout, lm_atlas, lm_uniform, material_layout, corona_layout, layout, vertex_layout, bias }
    }

    #[allow(clippy::too_many_arguments)]
    fn pipeline(
        &self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        fs: &str,
        blend: Option<wgpu::BlendState>,
        depth_write: bool,
        cull: bool,
        bias: i32,
        alpha_to_coverage: bool,
        terrain_paint: bool,
        samples: u32,
    ) -> wgpu::RenderPipeline {
        let use_alpha_to_coverage = alpha_to_coverage && samples > 1;
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("omsi"),
            layout: Some(&self.layout),
            vertex: wgpu::VertexState {
                module: &self.shader,
                entry_point: Some("vs_main"),
                buffers: &[self.vertex_layout.clone()],
                compilation_options: Default::default(),
            },
            primitive: one_sided_primitive(cull),
            // Reversed Z: the near plane is 1 and the far plane 0, so nearer means
            // greater. The depth bias keeps its meaning (negative = towards the viewer)
            // only if its sign is turned around with the axis.
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(depth_write),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: wgpu::DepthBiasState {
                    constant: -bias,
                    slope_scale: if bias != 0 {
                        -bias.signum() as f32 * 2.0
                    } else {
                        0.0
                    },
                    clamp: 0.0,
                },
            }),
            multisample: wgpu::MultisampleState {
                count: samples,
                mask: !0,
                alpha_to_coverage_enabled: use_alpha_to_coverage,
            },
            fragment: Some(wgpu::FragmentState {
                module: &self.shader,
                entry_point: Some(fs),
                targets: &color_targets(
                    format,
                    blend,
                    if fs == "fs_surface_depth" { wgpu::ColorWrites::empty() } else { wgpu::ColorWrites::ALL },
                    fs != "fs_surface_depth",
                    fs == "fs_enhanced",
                ),
                // Only cutouts and painted terrain keep their discard; opaque and
                // ordinary blended materials retain early depth testing.
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &[
                        ("ALPHA_TEST", if alpha_to_coverage { 1.0 } else { 0.0 }),
                        (
                            "ALPHA_TO_COVERAGE",
                            if use_alpha_to_coverage { 1.0 } else { 0.0 },
                        ),
                        ("TERRAIN_PAINT", if terrain_paint { 1.0 } else { 0.0 }),
                    ],
                    ..Default::default()
                },
            }),
            multiview_mask: None,
            cache: crate::pipeline_cache::get(device).as_ref(),
        })
    }

    /// One pipeline per `pipe_code`: the kind decides blending and the depth write.
    pub(crate) fn pipelines(&self, device: &wgpu::Device, f: wgpu::TextureFormat, fs: &str, samples: u32) -> Vec<wgpu::RenderPipeline> {
        let mut out = Vec::with_capacity(PIPE_KINDS as usize * 4);
        for kind in 0..PIPE_KINDS {
            let blend = matches!(kind, PIPE_BLEND | PIPE_BLEND_NO_WRITE | PIPE_TERRAIN_PAINT)
                .then_some(wgpu::BlendState::ALPHA_BLENDING);
            let depth_write = kind != PIPE_BLEND_NO_WRITE && kind != PIPE_TERRAIN_PAINT;
            for cull in [false, true] {
                for surface in [false, true] {
                    out.push(self.pipeline(
                        device,
                        f,
                        if kind == PIPE_SURFACE_DEPTH { "fs_surface_depth" } else { fs },
                        blend,
                        depth_write,
                        cull,
                        if surface { self.bias } else { 0 },
                        kind == PIPE_ALPHA_TEST,
                        kind == PIPE_TERRAIN_PAINT && fs == "fs_enhanced",
                        samples,
                    ));
                }
            }
        }
        out
    }
}

/// The depth prepass's pipelines: single-sampled, and multisampled with MSAA.
pub(crate) struct Prepass {
    pub pipelines: [wgpu::RenderPipeline; 6],
    pub msaa_pipelines: Option<[wgpu::RenderPipeline; 6]>,
    pub presurface_pipelines: Option<[wgpu::RenderPipeline; 4]>,
}

pub(crate) fn prepass(device: &wgpu::Device, scene: &SceneBase, msaa: u32) -> Prepass {
    let prepass_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("prepass"),
        bind_group_layouts: &[Some(&scene.camera_layout), Some(&scene.material_layout)],
        immediate_size: 0,
    });
    // the prepass culls exactly as the main pass does: a back face that wrote depth
    // here would hide what the main pass then draws behind it
    let make_prepass_samples = |kind: u8, cull: bool, samples: u32, in_main: bool| {
        let fragment = match kind {
            0 => "fs_shadow",
            1 => "fs_shadow_test",
            2 => "fs_transmap_depth",
            _ => unreachable!(),
        };
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("depth prepass"),
            layout: Some(&prepass_pl),
            vertex: wgpu::VertexState {
                module: &scene.shader,
                entry_point: Some("vs_main"),
                buffers: &[scene.vertex_layout.clone()],
                compilation_options: Default::default(),
            },
            primitive: one_sided_primitive(cull),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &scene.shader,
                entry_point: Some(fragment),
                targets: &if in_main { color_targets(HDR_FORMAT, None, wgpu::ColorWrites::empty(), false, false) } else { Vec::new() },
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: crate::pipeline_cache::get(device).as_ref(),
        })
    };
    let make_prepass = |kind: u8, cull: bool| make_prepass_samples(kind, cull, 1, false);
    let pipelines = [
        make_prepass(0, false),
        make_prepass(0, true),
        make_prepass(1, false),
        make_prepass(1, true),
        make_prepass(2, false),
        make_prepass(2, true),
    ];
    // Apple views with cutout/blended draws also need these pipelines: their
    // visibility depends on shading, unlike opaque hidden-surface removal.
    // Purely opaque Apple views still skip this pass below to avoid its cost.
    let msaa_pipelines = (msaa > 1).then(|| {
        [(0, false), (0, true), (1, false), (1, true), (2, false), (2, true)]
            .map(|(kind, cull)| make_prepass_samples(kind, cull, msaa, false))
    });
    let presurface_pipelines = (msaa > 1).then(|| {
        [(0, false), (0, true), (2, false), (2, true)]
            .map(|(kind, cull)| make_prepass_samples(kind, cull, msaa, true))
    });
    Prepass { pipelines, msaa_pipelines, presurface_pipelines }
}
