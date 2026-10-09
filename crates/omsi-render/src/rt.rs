//! Enhanced+: ray tracing with the device's hardware ray queries.
//!
//! Every solid and alpha-tested mesh within `RT_RANGE` of the camera is put into an
//! acceleration structure each frame (a bottom-level structure per mesh and set of drawn
//! material slots, kept while it is used; a top-level one with the instances, built anew).
//! The window's picture then traces, per pixel of the depth prepass, the sun's shadow and
//! the sky's occlusion (rt.wgsl); the main pass takes them in place of the shadow map and
//! the screen-space AO where they are there. What a hit looks like - for the reflections -
//! comes from a record per geometry: its material's colour and its texture's mean colour.
//!
//! OMSI_NO_RT=1 opens no ray queries (Enhanced+ draws as Enhanced), OMSI_NO_RT_FRAME=1 keeps
//! them but traces nothing, OMSI_NO_RT_GRADE=1 leaves out Enhanced+'s grade, and
//! OMSI_RT_REFL_HALF=1 traces the reflections at half size. OMSI_DEBUG_RT=1 logs the
//! structures now and then; its numbers show: 3 cut-out meshes as solid, 4 cut-out meshes
//! as clear, 7 the rays unfiltered, 8 / 9 / 10 the reflections' rays and weights (OMSI_DEBUG_ENHANCED=1 and 2 show the traced shadow and occlusion).

use super::*;

/// How far from the camera meshes are traced (m): beyond it the shadow map holds.
pub(super) const RT_RANGE: f32 = 420.0;
/// How far the ambient occlusion looks (m).
const AO_RADIUS: f32 = 2.2;
/// The sun disc as traced (tangent of its radius): larger than the real 0.27 deg, for the
/// soft edge a shadow takes on away from its caster (the shadow map's is softer still).
const SUN_CONE: f32 = 0.012;
/// Bottom-level structures built (or rebuilt) per frame at most: a map tile streamed in
/// brings hundreds of meshes; they come into the shadows over a few frames instead of a hitch.
const BLAS_BUDGET: usize = 320;
/// Texture means worked out per frame at most.
const AVG_BUDGET: usize = 128;
/// Frames a pixel's rays are accumulated over at most.
const HISTORY: f32 = 24.0;
const NO_TEX: u32 = u32::MAX;

#[derive(Clone, PartialEq, Eq, Hash)]
struct BlasKey {
    vb: wgpu::Buffer,
    ib: wgpu::Buffer,
    /// The mesh (its `gen`) and where it lies in its page's buffers.
    gen: u64,
    first_vertex: u32,
    first_index: u32,
    vertex_count: u32,
    /// The mesh's ranges traced as solid, and as alpha-tested (a bit per range).
    solid: u64,
    cut: u64,
    /// ... and as see-through panes (the sun's shadow only: they let part of it through)
    glass: u64,
}

struct BlasEntry {
    blas: wgpu::Blas,
    sizes: Vec<wgpu::BlasTriangleGeometrySizeDescriptor>,
    /// (first index, index count) of each geometry, in the order of `ranges`.
    spans: Vec<(u32, u32)>,
    mesh: MeshId,
    built: bool,
    used: u64,
}

/// What a traced geometry looks like (rt.wgsl `Record`).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Record {
    tex: u32,
    trans: u32,
    flags: u32,
    pad: u32,
    color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    inv_view_proj: [[f32; 4]; 4],
    view_proj: [[f32; 4]; 4],
    prev_view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    sun: [f32; 4],
    size: [f32; 4],
    proj: [f32; 4],
    temporal: [f32; 4],
    vehicle_now: [f32; 4],
    vehicle_prev: [f32; 4],
    vehicle_box: [f32; 4],
    vehicle_centre: [f32; 4],
    sky: [f32; 4],
    sun_light: [f32; 4],
}

/// The traced lighting's textures for one picture size (all rgba16float: r the ambient
/// occlusion, g the view depth it was traced at, b the sun's visibility (< 0: not traced),
/// a the frames accumulated).
pub(super) struct Targets {
    pub(super) size: (u32, u32),
    raw: wgpu::TextureView,
    tmp: wgpu::TextureView,
    /// What the main pass reads (camera bind group binding 8 in place of the SSAO).
    pub(super) out: wgpu::TextureView,
}

pub(super) struct RayTracer {
    blas: HashMap<BlasKey, BlasEntry>,
    /// Meshes whose vertices changed (skinned people): their structures are built again.
    dirty: std::cell::RefCell<std::collections::HashSet<MeshId>>,
    tlas: wgpu::Tlas,
    tlas_cap: u32,
    tlas_used: usize,
    records: Vec<Record>,
    records_buf: wgpu::Buffer,
    tex_avg: wgpu::Buffer,
    tex_gen: Vec<u64>,
    avg_stride: u64,
    avg_layout: wgpu::BindGroupLayout,
    avg_pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    trace: wgpu::ComputePipeline,
    temporal: wgpu::ComputePipeline,
    params: wgpu::Buffer,
    pub(super) targets: Option<Targets>,
    frame: u64,
    /// Last frame's view-projection, render origin and player vehicle (origin, heading deg).
    prev: Option<(Mat4, DVec3, Option<(DVec3, f64)>)>,
    /// Which structures to build this frame.
    to_build: Vec<BlasKey>,
    pub(super) instances: usize,
    /// The reflections: their layout, the rays, their history and the composite.
    refl_layout: wgpu::BindGroupLayout,
    reflect: wgpu::ComputePipeline,
    composite: wgpu::RenderPipeline,
    /// The reflections' rays (one per `refl_div()` x `refl_div()` pixels).
    refl: Option<((u32, u32), wgpu::TextureView)>,
    /// A 1 x 1 storage texture for the composite's unused output binding.
    spare: wgpu::TextureView,
}

/// One reflection ray per this many pixels each way (OMSI_RT_REFL_HALF=1: per 2 x 2).
fn refl_div() -> u32 {
    if omsi_cfg::flags::OMSI_RT_REFL_HALF.is_set() { 2 } else { 1 }
}

/// The traced lighting's shader: the shared ray tracing and its own.
pub(super) fn lighting_source() -> String {
    [include_str!("rt_common.wgsl"), include_str!("rt.wgsl")].join("\n")
}

/// The size of the shaders' `RtParams` (for the shader test).
#[cfg(test)]
pub(super) const PARAMS_SIZE: usize = std::mem::size_of::<Params>();

/// The reflections' shader: the shared ray tracing, the enhanced uniform's layout, its own.
pub(super) fn reflect_source() -> String {
    let common = include_str!("enhanced_common.wgsl");
    let enh = &common[..common.find("@group(0) @binding(11)").expect("Enhanced struct")];
    [include_str!("rt_common.wgsl"), enh, include_str!("rt_reflect.wgsl")].join("\n")
}

fn storage_texture(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: wgpu::TextureFormat::Rgba16Float,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

fn texture_entry(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture { sample_type, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
        count: None,
    }
}

fn storage_buffer(binding: u32, read_only: bool, dynamic: bool, min: u64) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: dynamic, min_binding_size: wgpu::BufferSize::new(min) },
        count: None,
    }
}

impl RayTracer {
    pub(super) fn new(device: &wgpu::Device) -> RayTracer {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ray tracing"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<Params>() as u64) },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::AccelerationStructure { vertex_return: false },
                    count: None,
                },
                texture_entry(2, wgpu::TextureSampleType::Depth),
                storage_buffer(3, true, false, 32),
                storage_buffer(4, true, false, 256),
                texture_entry(5, wgpu::TextureSampleType::Float { filterable: false }),
                texture_entry(6, wgpu::TextureSampleType::Float { filterable: false }),
                storage_texture(7),
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("ray tracing"), source: wgpu::ShaderSource::Wgsl(lighting_source().into()) });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("ray tracing"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = |entry: &str, step: Option<f64>| {
            let constants: Vec<(&str, f64)> = step.map(|s| vec![("STEP", s)]).unwrap_or_default();
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pl),
                module: &module,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions { constants: &constants, ..Default::default() },
                cache: crate::pipeline_cache::get(device).as_ref(),
            })
        };
        let trace = pipeline("cs_trace", None);
        let temporal = pipeline("cs_denoise", None);
        let avg_stride = (device.limits().min_storage_buffer_offset_alignment as u64).max(256);
        let avg_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("texture means"),
            entries: &[
                texture_entry(0, wgpu::TextureSampleType::Float { filterable: true }),
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                storage_buffer(2, false, true, 16),
            ],
        });
        let avg_module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("texture means"), source: wgpu::ShaderSource::Wgsl(include_str!("rt_avg.wgsl").into()) });
        let avg_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("texture means"), bind_group_layouts: &[Some(&avg_layout)], immediate_size: 0 });
        let avg_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("texture means"),
            layout: Some(&avg_pl),
            module: &avg_module,
            entry_point: Some("cs_tex_avg"),
            compilation_options: Default::default(),
            cache: crate::pipeline_cache::get(device).as_ref(),
        });
        let vis = wgpu::ShaderStages::COMPUTE | wgpu::ShaderStages::FRAGMENT;
        let tex = |binding, sample_type, view_dimension, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Texture { sample_type, view_dimension, multisampled: false },
            count: None,
        };
        let unfilt = wgpu::TextureSampleType::Float { filterable: false };
        let filt = wgpu::TextureSampleType::Float { filterable: true };
        let d2 = wgpu::TextureViewDimension::D2;
        let refl_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ray-traced reflections"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: vis,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<Params>() as u64) },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::AccelerationStructure { vertex_return: false }, count: None },
                tex(2, wgpu::TextureSampleType::Depth, d2, vis),
                storage_buffer(3, true, false, 32),
                storage_buffer(4, true, false, 256),
                tex(5, unfilt, d2, vis),
                tex(6, unfilt, d2, vis),
                storage_texture(7),
                wgpu::BindGroupLayoutEntry { binding: 8, visibility: vis, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                tex(9, filt, d2, wgpu::ShaderStages::COMPUTE),
                tex(10, filt, wgpu::TextureViewDimension::Cube, vis),
                wgpu::BindGroupLayoutEntry {
                    binding: 11,
                    visibility: vis,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                tex(12, unfilt, d2, vis),
            ],
        });
        let refl_module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("ray-traced reflections"), source: wgpu::ShaderSource::Wgsl(reflect_source().into()) });
        let refl_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("ray-traced reflections"), bind_group_layouts: &[Some(&refl_layout)], immediate_size: 0 });
        let div = [("REFL_DIV", refl_div() as f64)];
        let refl_compute = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&refl_pl),
                module: &refl_module,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions { constants: &div, ..Default::default() },
                cache: crate::pipeline_cache::get(device).as_ref(),
            })
        };
        let reflect = refl_compute("cs_reflect");
        let add = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
        let composite = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("reflections composite"),
            layout: Some(&refl_pl),
            vertex: wgpu::VertexState { module: &refl_module, entry_point: Some("vs_full"), buffers: &[], compilation_options: Default::default() },
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &refl_module,
                entry_point: Some("fs_composite"),
                targets: &[Some(wgpu::ColorTargetState { format: HDR_FORMAT, blend: Some(wgpu::BlendState { color: add, alpha: add }), write_mask: wgpu::ColorWrites::COLOR })],
                compilation_options: wgpu::PipelineCompilationOptions { constants: &div, ..Default::default() },
            }),
            multiview_mask: None,
            cache: crate::pipeline_cache::get(device).as_ref(),
        });
        let tlas_cap = 8192;
        RayTracer {
            blas: HashMap::new(),
            dirty: Default::default(),
            tlas: Self::make_tlas(device, tlas_cap),
            tlas_cap,
            tlas_used: 0,
            records: Vec::new(),
            records_buf: Self::make_records(device, 1 << 16),
            tex_avg: Self::make_avg(device, 4096, avg_stride),
            tex_gen: Vec::new(),
            avg_stride,
            avg_layout,
            avg_pipeline,
            layout,
            trace,
            temporal,
            params: device.create_buffer(&wgpu::BufferDescriptor { label: Some("ray tracing params"), size: std::mem::size_of::<Params>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }),
            targets: None,
            frame: 0,
            prev: None,
            to_build: Vec::new(),
            instances: 0,
            refl_layout,
            reflect,
            composite,
            refl: None,
            spare: device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("rt spare"),
                    size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba16Float,
                    usage: wgpu::TextureUsages::STORAGE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default()),
        }
    }

    fn make_tlas(device: &wgpu::Device, cap: u32) -> wgpu::Tlas {
        device.create_tlas(&wgpu::CreateTlasDescriptor {
            label: Some("ray tracing instances"),
            max_instances: cap,
            flags: wgpu::AccelerationStructureFlags::PREFER_FAST_BUILD,
            update_mode: wgpu::AccelerationStructureUpdateMode::Build,
        })
    }

    fn make_records(device: &wgpu::Device, n: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor { label: Some("ray tracing records"), size: n * std::mem::size_of::<Record>() as u64, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false })
    }

    fn make_avg(device: &wgpu::Device, n: u64, stride: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor { label: Some("texture means"), size: n * stride, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false })
    }

    /// The vertices of these meshes changed: their structures are built again.
    pub(super) fn meshes_changed(&self, ids: impl IntoIterator<Item = MeshId>) {
        self.dirty.borrow_mut().extend(ids);
    }

    /// The traced lighting's textures for this size; true when they were made anew (the
    /// camera bind group must then point at the new one).
    pub(super) fn ensure_targets(&mut self, device: &wgpu::Device, w: u32, h: u32) -> bool {
        if self.targets.as_ref().is_some_and(|t| t.size == (w, h)) {
            return false;
        }
        let mk = |label: &str| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba16Float,
                    usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        self.targets = Some(Targets { size: (w, h), raw: mk("rt rays"), tmp: mk("rt filter"), out: mk("rt lighting") });
        self.prev = None;
        true
    }
}

impl Renderer {
    /// Which of an instance's mesh ranges are traced, solid or cut out (a bit per range), as
    /// its materials draw them; blended ones (glass, decals, dirt) are not.
    fn traced_ranges(scene: &Scene, inst: &Instance) -> (u64, u64, u64) {
        let m = &scene.meshes[inst.mesh];
        let (mut solid, mut cut, mut glass) = (0u64, 0u64, 0u64);
        for (ri, (_, count, slot)) in m.ranges.iter().enumerate().take(64) {
            if *count < 3 {
                continue;
            }
            let Some(mat) = inst.materials.get(*slot as usize).and_then(|&id| scene.materials.get(id)) else { continue };
            if mat.no_z_check || inst.slot_alpha.get(*slot as usize).is_some_and(|a| *a < 0.01) {
                continue;
            }
            match mat.alpha {
                AlphaMode::Opaque => solid |= 1 << ri,
                AlphaMode::Test => cut |= 1 << ri,
                // a body whose paint is cut by its transmap writes depth: cut out as well
                AlphaMode::Blend if mat.transmap.is_some() && !mat.no_z_write => cut |= 1 << ri,
                // a pane (any other blended layer): the shadow rays' tint
                AlphaMode::Blend => glass |= 1 << ri,
            }
        }
        (solid, cut, glass)
    }

    /// The frame's ray-tracing input: the instances near the camera with their structures
    /// (made for meshes met for the first time) and the records of their geometries, and the
    /// shaders' parameters. False when there is nothing to trace with.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare_ray_tracing(&mut self, scene: &Scene, camera: &Camera, lighting: &Lighting, view_proj: Mat4, proj: Mat4, w: u32, h: u32, dt: f32) -> bool {
        let Some(rt) = self.rt.as_mut() else { return false };
        rt.frame += 1;
        let frame = rt.frame;
        let ro = scene.render_origin;
        let cam = (camera.position - ro).as_vec3();
        let lod_fov = camera.fov_deg.to_radians().max(1e-3);
        // structures of meshes whose vertices changed are built again
        if !rt.dirty.borrow().is_empty() {
            let dirty = std::mem::take(&mut *rt.dirty.borrow_mut());
            for e in rt.blas.values_mut() {
                if dirty.contains(&e.mesh) {
                    e.built = false;
                }
            }
        }
        rt.to_build.clear();
        rt.records.clear();
        let mut chosen: Vec<(BlasKey, [f32; 12], u32, u8)> = Vec::new();
        for inst in &scene.instances {
            // (a decal - a surface object lying on the road - is no geometry for the rays,
            // but what such an object raises and casts a shadow from is: the Spandau depot's
            // halls, one object with its yard, lost their shadow close up in Enhanced+, #1783)
            if !inst.visible || inst.blob || (inst.decal && !inst.casts_shadow) || inst.ground_layer {
                continue;
            }
            let m = &scene.meshes[inst.mesh];
            if m.ranges.is_empty() || m.bounds_radius <= 0.0 {
                continue;
            }
            if let Some([x0, y0, x1, y1]) = inst.near_only {
                if camera.position.x < x0 || camera.position.x > x1 || camera.position.y < y0 || camera.position.y > y1 {
                    continue;
                }
            }
            let (c, r) = Self::bounding_sphere(scene, inst);
            let d = (c - cam).length();
            if d - r > RT_RANGE {
                continue;
            }
            // the level of detail the picture shows
            if inst.lod.0 > 0.0 || inst.lod.1 < f32::MAX {
                let scale = Self::instance_scale(scene, inst);
                let radius = if inst.object_radius > 0.0 { inst.object_radius } else { m.bounds_radius } * scale;
                let dd = ((inst.origin - ro).as_vec3() - cam).length();
                let size = if dd <= radius { f32::MAX } else { 2.0 * radius / (dd.max(0.01) * lod_fov) };
                if size < inst.lod.0 || (inst.lod.1 < f32::MAX && size >= inst.lod.1) {
                    continue;
                }
            }
            let (solid, cut, glass) = Self::traced_ranges(scene, inst);
            if solid | cut | glass == 0 {
                continue;
            }
            let caster = inst.casts_shadow && !(self.options.omsi_shadow_casters && !inst.omsi_caster);
            let seen = !inst.mirror_only;
            let t = Mat4::from_translation((inst.origin - ro).as_vec3()) * inst.transform;
            let c = t.to_cols_array();
            // (3 x 4, rows first)
            let rows = [c[0], c[4], c[8], c[12], c[1], c[5], c[9], c[13], c[2], c[6], c[10], c[14]];
            // the solid ranges and the cut-out ones as two instances: the shadow rays take the
            // first hit of the solid ones alone, the others stop at the nearest of either
            for (bits, kind) in [(solid, 0), (cut, 1), (glass, 2)] {
                let is_cut = kind == 1;
                let mask = match kind {
                    0 => (caster as u8) | ((seen as u8) << 1),
                    1 => (seen as u8) << 2,
                    _ => (caster as u8) << 3,
                };
                if bits == 0 || mask == 0 {
                    continue;
                }
                let key = BlasKey {
                    vb: m.vertex_buf.clone(),
                    ib: m.index_buf.clone(),
                    gen: m.gen,
                    first_vertex: m.base_vertex.max(0) as u32,
                    first_index: m.first_index,
                    vertex_count: (m.vertex_bytes / std::mem::size_of::<Vertex>() as u64) as u32,
                    solid: if kind == 0 { bits } else { 0 },
                    cut: if kind == 1 { bits } else { 0 },
                    glass: if kind == 2 { bits } else { 0 },
                };
                let entry = match rt.blas.get_mut(&key) {
                    Some(e) => e,
                    None => {
                        if rt.to_build.len() >= BLAS_BUDGET {
                            continue;
                        }
                        let vertex_count = key.vertex_count;
                        let mut sizes = Vec::new();
                        let mut spans = Vec::new();
                        for (ri, (first, count, _)) in m.ranges.iter().enumerate().take(64) {
                            if bits & (1 << ri) == 0 {
                                continue;
                            }
                            let count = count / 3 * 3;
                            sizes.push(wgpu::BlasTriangleGeometrySizeDescriptor {
                                vertex_format: wgpu::VertexFormat::Float32x3,
                                vertex_count,
                                index_format: Some(wgpu::IndexFormat::Uint32),
                                index_count: Some(count),
                                flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE,
                            });
                            spans.push((*first, count));
                        }
                        let blas = self.device.create_blas(
                            &wgpu::CreateBlasDescriptor { label: None, flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE, update_mode: wgpu::AccelerationStructureUpdateMode::Build },
                            wgpu::BlasGeometrySizeDescriptors::Triangles { descriptors: sizes.clone() },
                        );
                        rt.blas.entry(key.clone()).or_insert(BlasEntry { blas, sizes, spans, mesh: inst.mesh, built: false, used: frame })
                    }
                };
                entry.used = frame;
                if !entry.built {
                    if rt.to_build.len() >= BLAS_BUDGET {
                        continue;
                    }
                    entry.built = true;
                    rt.to_build.push(key.clone());
                }
                // the geometries' records, in the structure's order
                let base = rt.records.len() as u32;
                if base >= (1 << 24) - 64 {
                    break;
                }
                for (ri, (_, _, slot)) in m.ranges.iter().enumerate().take(64) {
                    if bits & (1 << ri) == 0 {
                        continue;
                    }
                    let mat = &scene.materials[inst.materials[*slot as usize]];
                    let tex = mat.texture.map_or(NO_TEX, |t| t as u32);
                    let (trans, from_trans) = match mat.transmap {
                        Some((t, true)) => (t as u32, true),
                        _ => (NO_TEX, false),
                    };
                    let mut flags = 0u32;
                    if is_cut {
                        flags |= 1;
                    }
                    if mat.unlit {
                        flags |= 2;
                    }
                    if from_trans {
                        flags |= 4;
                    }
                    rt.records.push(Record { tex, trans, flags, pad: 0, color: mat.color });
                }
                chosen.push((key, rows, base, mask));
            }
        }
        // the top level: grown when the instances outnumber it
        if chosen.len() as u32 > rt.tlas_cap {
            rt.tlas_cap = (chosen.len() as u32).next_power_of_two();
            rt.tlas = RayTracer::make_tlas(&self.device, rt.tlas_cap);
            rt.tlas_used = 0;
        }
        for (i, (key, rows, base, mask)) in chosen.iter().enumerate() {
            let blas = &rt.blas[key].blas;
            rt.tlas[i] = Some(wgpu::TlasInstance::new(blas, *rows, *base, *mask));
        }
        for i in chosen.len()..rt.tlas_used {
            rt.tlas[i] = None;
        }
        rt.tlas_used = chosen.len();
        rt.instances = chosen.len();
        // the records, in a buffer grown as needed
        let bytes: &[u8] = bytemuck::cast_slice(&rt.records);
        if bytes.len() as u64 > rt.records_buf.size() {
            rt.records_buf = RayTracer::make_records(&self.device, (rt.records.len() as u64).next_power_of_two());
        }
        if !bytes.is_empty() {
            self.queue.write_buffer(&rt.records_buf, 0, bytes);
        }
        // structures nobody used for a while are let go
        if frame % 240 == 0 {
            rt.blas.retain(|_, e| frame - e.used < 1200);
        }
        if omsi_cfg::flags::OMSI_DEBUG_RT.is_set() && frame % 30 == 1 {
            log::info!("ray tracing: {} instances, {} geometries, {} structures ({} built this frame)", chosen.len(), rt.records.len(), rt.blas.len(), rt.to_build.len());
        }
        // --- the parameters
        let sun = lighting.sun_dir.normalize_or_zero();
        let sun_on = lighting.casts_sun_shadows();
        let vehicle = lighting.inside.map(|(o, h, _)| (o, h));
        let (prev_vp, history) = match rt.prev {
            Some((vp, prev_ro, _)) => (vp * Mat4::from_translation((ro - prev_ro).as_vec3()), dt < 3.0),
            None => (view_proj, false),
        };
        let vrel = |o: DVec3| (o - ro).as_vec3();
        let (vehicle_now, vehicle_prev, vehicle_box, vehicle_centre) = match (lighting.inside, rt.prev.and_then(|p| p.2)) {
            (Some((o, hdg, bb)), Some((po, ph))) => (
                vrel(o).extend((hdg as f32).to_radians()).to_array(),
                vrel(po).extend((ph as f32).to_radians()).to_array(),
                [bb[0] * 0.5, bb[1] * 0.5, bb[2] * 0.5, 1.0],
                [bb[3], bb[4], bb[5], 0.0],
            ),
            _ => ([0.0; 4], [0.0; 4], [0.0; 4], [0.0; 4]),
        };
        let debug = omsi_cfg::flags::OMSI_DEBUG_RT.parse::<f32>().unwrap_or(0.0);
        let p = Params {
            inv_view_proj: view_proj.inverse().to_cols_array_2d(),
            view_proj: view_proj.to_cols_array_2d(),
            prev_view_proj: prev_vp.to_cols_array_2d(),
            eye: cam.extend((frame % 4096) as f32).to_array(),
            sun: sun.extend(if sun_on { SUN_CONE } else { 0.0 }).to_array(),
            size: [w as f32, h as f32, 1.0 / w.max(1) as f32, 1.0 / h.max(1) as f32],
            proj: [proj.z_axis.z, proj.w_axis.z, RT_RANGE, AO_RADIUS],
            temporal: [history as u8 as f32, 0.06, debug, HISTORY],
            vehicle_now,
            vehicle_prev,
            vehicle_box,
            vehicle_centre,
            sky: [0.0; 4],
            sun_light: [0.0; 4],
        };
        self.queue.write_buffer(&rt.params, 0, bytemuck::bytes_of(&p));
        rt.prev = Some((view_proj, ro, vehicle));
        true
    }

    /// The mean colours of the textures the records name that have none yet (or were replaced).
    fn encode_texture_means(&mut self, encoder: &mut wgpu::CommandEncoder, scene: &Scene) {
        let Some(rt) = self.rt.as_mut() else { return };
        if scene.textures.len() > rt.tex_gen.len() {
            rt.tex_gen.resize(scene.textures.len(), 0);
        }
        let need = (scene.textures.len() as u64) * rt.avg_stride;
        if need > rt.tex_avg.size() {
            // (grown: the means made so far are copied over)
            let bigger = RayTracer::make_avg(&self.device, (scene.textures.len() as u64).next_power_of_two(), rt.avg_stride);
            encoder.copy_buffer_to_buffer(&rt.tex_avg, 0, &bigger, 0, rt.tex_avg.size());
            rt.tex_avg = bigger;
        }
        let mut wanted: Vec<usize> = Vec::new();
        for r in &rt.records {
            for t in [r.tex, r.trans] {
                if t == NO_TEX {
                    continue;
                }
                let t = t as usize;
                if t < scene.textures.len() && rt.tex_gen[t] != scene.textures[t].gen && !wanted.contains(&t) {
                    wanted.push(t);
                    if wanted.len() >= AVG_BUDGET {
                        break;
                    }
                }
            }
            if wanted.len() >= AVG_BUDGET {
                break;
            }
        }
        if wanted.is_empty() {
            return;
        }
        let groups: Vec<wgpu::BindGroup> = wanted
            .iter()
            .map(|&t| {
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("texture mean"),
                    layout: &rt.avg_layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&scene.textures[t].view) },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.lin_sampler) },
                        wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &rt.tex_avg, offset: 0, size: wgpu::BufferSize::new(16) }) },
                    ],
                })
            })
            .collect();
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("texture means"), timestamp_writes: None });
            pass.set_pipeline(&rt.avg_pipeline);
            for (&t, bg) in wanted.iter().zip(&groups) {
                pass.set_bind_group(0, bg, &[(t as u64 * rt.avg_stride) as u32]);
                pass.dispatch_workgroups(1, 1, 1);
            }
        }
        for &t in &wanted {
            rt.tex_gen[t] = scene.textures[t].gen;
        }
    }

    /// Build this frame's structures and trace the lighting into `Targets::out`, after the
    /// depth prepass.
    pub(super) fn encode_ray_tracing(&mut self, encoder: &mut wgpu::CommandEncoder, scene: &Scene, queries: Option<&wgpu::QuerySet>, timed: &mut Vec<&'static str>) {
        self.encode_texture_means(encoder, scene);
        let Some(rt) = self.rt.as_mut() else { return };
        let Some(depth) = self.ao.as_ref().map(|a| &a.depth_view) else { return };
        let Some(t) = rt.targets.as_ref() else { return };
        {
            let entries: Vec<wgpu::BlasBuildEntry> = rt
                .to_build
                .iter()
                .map(|k| {
                    let e = &rt.blas[k];
                    wgpu::BlasBuildEntry {
                        blas: &e.blas,
                        geometry: wgpu::BlasGeometries::TriangleGeometries(
                            e.sizes
                                .iter()
                                .zip(&e.spans)
                                .map(|(size, (first, _))| wgpu::BlasTriangleGeometry {
                                    size,
                                    vertex_buffer: &k.vb,
                                    first_vertex: k.first_vertex,
                                    vertex_stride: std::mem::size_of::<Vertex>() as u64,
                                    index_buffer: Some(&k.ib),
                                    first_index: Some(k.first_index + *first),
                                    transform_buffer: None,
                                    transform_buffer_offset: None,
                                })
                                .collect(),
                        ),
                    }
                })
                .collect();
            encoder.build_acceleration_structures(entries.iter(), std::iter::once(&rt.tlas));
        }
        let group = |inp: &wgpu::TextureView, raw: &wgpu::TextureView, out: &wgpu::TextureView| {
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ray tracing"),
                layout: &rt.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: rt.params.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: rt.tlas.as_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(depth) },
                    wgpu::BindGroupEntry { binding: 3, resource: rt.records_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 4, resource: rt.tex_avg.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(inp) },
                    wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(raw) },
                    wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(out) },
                    wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::Sampler(&self.lin_sampler) },
                ],
            })
        };
        // (each pass's input never is its output)
        let trace_bg = group(&t.tmp, &t.tmp, &t.raw);
        let denoise_bg = group(&t.raw, &t.tmp, &t.out);
        let (gx, gy) = (t.size.0.div_ceil(8), t.size.1.div_ceil(8));
        for (label, pipe, bg) in [("rt trace", &rt.trace, &trace_bg), ("rt denoise", &rt.temporal, &denoise_bg)] {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(label),
                timestamp_writes: pass_timer(queries, timed, label).map(|t| wgpu::ComputePassTimestampWrites {
                    query_set: t.query_set,
                    beginning_of_pass_write_index: t.beginning_of_pass_write_index,
                    end_of_pass_write_index: t.end_of_pass_write_index,
                }),
            });
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, bg, &[]);
            pass.dispatch_workgroups(gx, gy, 1);
        }
    }
}

impl Renderer {
    /// The ray-traced reflections of the window's picture, added to `HdrTargets::view` after
    /// the main pass (and before the rain on the glass, which looks through it).
    pub(super) fn encode_rt_reflections(&mut self, encoder: &mut wgpu::CommandEncoder, w: u32, h: u32, queries: Option<&wgpu::QuerySet>, timed: &mut Vec<&'static str>) {
        let (Some(rt), Some(hdr), Some(ao), Some(probe)) = (self.rt.as_mut(), self.hdr_targets.get(&(w, h)), self.ao.as_ref(), self.probe.as_ref()) else { return };
        let Some(gbuf) = hdr.gbuf.as_ref() else { return };
        let half = (w.div_ceil(refl_div()), h.div_ceil(refl_div()));
        if rt.refl.as_ref().is_none_or(|r| r.0 != half) {
            let mk = |label: &str| {
                self.device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some(label),
                        size: wgpu::Extent3d { width: half.0, height: half.1, depth_or_array_layers: 1 },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba16Float,
                        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    })
                    .create_view(&Default::default())
            };
            rt.refl = Some((half, mk("rt reflection rays")));
        }
        let (_, raw) = rt.refl.as_ref().unwrap();
        let dummy = &self.black_texture.view;
        let group = |out: &wgpu::TextureView, scene: &wgpu::TextureView, h12: &wgpu::TextureView| {
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ray-traced reflections"),
                layout: &rt.refl_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: rt.params.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: rt.tlas.as_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&ao.depth_view) },
                    wgpu::BindGroupEntry { binding: 3, resource: rt.records_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 4, resource: rt.tex_avg.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&gbuf[0].1) },
                    wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&gbuf[1].1) },
                    wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(out) },
                    wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::Sampler(&self.lin_sampler) },
                    wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(scene) },
                    wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(&probe.view) },
                    wgpu::BindGroupEntry { binding: 11, resource: self.enh_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 12, resource: wgpu::BindingResource::TextureView(h12) },
                ],
            })
        };
        // (no pass's output is among its inputs)
        let trace_bg = group(raw, &hdr.view, dummy);
        let composite_bg = group(&rt.spare, dummy, raw);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rt reflections"),
                timestamp_writes: pass_timer(queries, timed, "rt reflections").map(|t| wgpu::ComputePassTimestampWrites {
                    query_set: t.query_set,
                    beginning_of_pass_write_index: t.beginning_of_pass_write_index,
                    end_of_pass_write_index: t.end_of_pass_write_index,
                }),
            });
            let (gx, gy) = (half.0.div_ceil(8), half.1.div_ceil(8));
            pass.set_pipeline(&rt.reflect);
            pass.set_bind_group(0, &trace_bg, &[]);
            pass.dispatch_workgroups(gx, gy, 1);
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("rt reflections composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &hdr.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: pass_timer(queries, timed, "rt composite"),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&rt.composite);
        pass.set_bind_group(0, &composite_bg, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod spv_dump {
    /// `SPV_DUMP=<dir> cargo test -p omsi-render spv_dump -- --ignored`: the ray tracing's
    /// shaders as wgpu's Vulkan backend writes them, for spirv-val.
    #[test]
    #[ignore]
    fn spv_dump() {
        let dir = std::path::PathBuf::from(std::env::var("SPV_DUMP").expect("SPV_DUMP"));
        let modules = [
            ("lighting", super::lighting_source()),
            ("reflect", super::reflect_source()),
            ("scene_plus", super::super::scene_shader_source(false).replace("//RT ", "")),
        ];
        for (name, src) in &modules {
            let module = naga::front::wgsl::parse_str(src).unwrap();
            let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module).unwrap();
            let (module, info) = naga::back::pipeline_constants::process_overrides(&module, &info, None, &Default::default()).unwrap();
            let opts = naga::back::spv::Options {
                lang_version: (1, 5),
                flags: naga::back::spv::WriterFlags::empty(),
                force_loop_bounding: true,
                ray_query_initialization_tracking: true,
                ..Default::default()
            };
            for entry in &module.entry_points {
                let pipeline = naga::back::spv::PipelineOptions { shader_stage: entry.stage, entry_point: entry.name.clone() };
                let words = naga::back::spv::write_vec(&module, &info, &opts, Some(&pipeline)).unwrap();
                let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
                std::fs::write(dir.join(format!("{name}_{}.spv", entry.name)), bytes).unwrap();
            }
            // and the HLSL wgpu's Direct3D 12 backend hands DXC (shader model 6.5)
            let hopts = naga::back::hlsl::Options { shader_model: naga::back::hlsl::ShaderModel::V6_5, ..Default::default() };
            let mut out = String::new();
            let popts = Default::default();
            let mut w = naga::back::hlsl::Writer::new(&mut out, &hopts, &popts);
            match w.write(&module, &info, None) {
                Ok(_) => std::fs::write(dir.join(format!("{name}.hlsl")), &out).unwrap(),
                Err(e) => std::fs::write(dir.join(format!("{name}.hlsl.err")), format!("{e:?}")).unwrap(),
            }
        }
    }
}
