//! The lamps' coronas and the smoke puffs: sprites drawn over the scene; and the snowfall.

use super::common::*;
use super::scene::SceneBase;
use crate::*;

pub(crate) const ADDITIVE: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent::REPLACE,
};
// Omsi's lamp sprites: SRCBLEND ONE, DESTBLEND INVSRCCOLOR (src + dst * (1 - src)),
// which keeps a coloured sprite's hue over a lit background instead of washing it to white
pub(crate) const SCREEN: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::OneMinusSrc,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent::REPLACE,
};
pub(crate) const ALPHA_BLEND: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::SrcAlpha,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent::REPLACE,
};

const CORONA_ATTRIBUTES: [wgpu::VertexAttribute; 6] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4];

pub(crate) struct Coronas {
    pub sampler: wgpu::Sampler,
    pub bind_group: wgpu::BindGroup,
    pub smoke_bind_group: wgpu::BindGroup,
    shader: wgpu::ShaderModule,
    pl: wgpu::PipelineLayout,
}

impl Coronas {
    /// The coronas' and the smoke's bind groups and shader, and the corona sprite (its
    /// bind group holds it).
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue, scene: &SceneBase) -> (Coronas, GpuTexture) {
        // corona sprite: soft radial falloff
        let cs = 64u32;
        let mut corona_img = omsi_texture::Image {
            width: cs,
            height: cs,
            rgba: vec![0; (cs * cs * 4) as usize],
            has_alpha: false,
        };
        for y in 0..cs {
            for x in 0..cs {
                let dx = (x as f32 + 0.5) / cs as f32 * 2.0 - 1.0;
                let dy = (y as f32 + 0.5) / cs as f32 * 2.0 - 1.0;
                let r = (dx * dx + dy * dy).sqrt();
                let v = ((1.0 - r).max(0.0)).powf(1.6) * 255.0;
                let o = ((y * cs + x) * 4) as usize;
                corona_img.rgba[o..o + 4].copy_from_slice(&[v as u8, v as u8, v as u8, 255]);
            }
        }
        let corona_texture = upload_texture(device, queue, &corona_img, false);
        let corona_sampler = clamped_linear_sampler(device);
        let corona_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("corona"),
            layout: &scene.corona_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&corona_texture.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&corona_sampler),
                },
            ],
        });
        // a soft grey puff until the app hands over the game's own `Texture/rauch.tga`
        let mut puff = omsi_texture::Image { width: cs, height: cs, rgba: vec![0; (cs * cs * 4) as usize], has_alpha: true };
        for y in 0..cs {
            for x in 0..cs {
                let dx = (x as f32 + 0.5) / cs as f32 * 2.0 - 1.0;
                let dy = (y as f32 + 0.5) / cs as f32 * 2.0 - 1.0;
                let a = (1.0 - (dx * dx + dy * dy).sqrt()).max(0.0).powf(1.2) * 255.0;
                let o = ((y * cs + x) * 4) as usize;
                puff.rgba[o..o + 4].copy_from_slice(&[255, 255, 255, a as u8]);
            }
        }
        let puff_texture = upload_texture(device, queue, &puff, false);
        let smoke_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("smoke"),
            layout: &scene.corona_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&puff_texture.view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&corona_sampler) },
            ],
        });
        drop(puff_texture);
        log::info!("renderer: compiling the coronas shaders");
        let corona_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("corona"),
            source: wgpu::ShaderSource::Wgsl(corona_shader_source().into()),
        });
        let corona_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("corona"),
            bind_group_layouts: &[Some(&scene.camera_layout), Some(&scene.corona_layout)],
            immediate_size: 0,
        });
        let coronas = Coronas {
            sampler: corona_sampler,
            bind_group: corona_bind_group,
            smoke_bind_group,
            shader: corona_shader,
            pl: corona_pl,
        };
        (coronas, corona_texture)
    }

    pub(crate) fn pipeline(&self, device: &wgpu::Device, f: wgpu::TextureFormat, fs: &str, blend: wgpu::BlendState, msaa: u32) -> wgpu::RenderPipeline {
        let corona_vertex = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GpuCorona>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &CORONA_ATTRIBUTES,
        };
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("corona"),
            layout: Some(&self.pl),
            vertex: wgpu::VertexState {
                module: &self.shader,
                entry_point: Some("vs_main"),
                buffers: &[corona_vertex],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                front_face: wgpu::FrontFace::Ccw,
                ..Default::default()
            },
            depth_stencil: Some(depth_test(wgpu::CompareFunction::GreaterEqual)),
            multisample: wgpu::MultisampleState {
                count: msaa,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            fragment: Some(wgpu::FragmentState {
                module: &self.shader,
                entry_point: Some(fs),
                targets: &color_targets(f, Some(blend), wgpu::ColorWrites::COLOR, false, false),
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: crate::pipeline_cache::get(device).as_ref(),
        })
    }
}

/// The snowfall (snow.wgsl): the camera group and its own parameters, no vertices.
pub(crate) struct Snow {
    shader: wgpu::ShaderModule,
    pub buf: wgpu::Buffer,
    pub bind_group: wgpu::BindGroup,
    pl: wgpu::PipelineLayout,
}

const PREMULTIPLIED_SNOW: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
    alpha: wgpu::BlendComponent::REPLACE,
};

impl Snow {
    pub(crate) fn new(device: &wgpu::Device, camera_layout: &wgpu::BindGroupLayout) -> Snow {
        // the snowfall (snow.wgsl): the camera group and its own parameters, no vertices
        let snow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("snow"),
            source: wgpu::ShaderSource::Wgsl(snow_shader_source().into()),
        });
        let snow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("snow"),
            entries: &[uniform_entry(0, wgpu::ShaderStages::VERTEX)],
        });
        let snow_buf = uniform_buffer(device, "snow params", std::mem::size_of::<SnowUniform>() as u64);
        let snow_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("snow"),
            layout: &snow_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: snow_buf.as_entire_binding() }],
        });
        let snow_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("snow"),
            bind_group_layouts: &[Some(camera_layout), Some(&snow_layout)],
            immediate_size: 0,
        });
        Snow { shader: snow_shader, buf: snow_buf, bind_group: snow_bind_group, pl: snow_pl }
    }

    pub(crate) fn pipeline(&self, device: &wgpu::Device, f: wgpu::TextureFormat, fs: &str, msaa: u32) -> wgpu::RenderPipeline {
        log::info!("renderer: compiling the snowfall shader ({fs})");
        let targets = color_targets(f, Some(PREMULTIPLIED_SNOW), wgpu::ColorWrites::COLOR, false, false);
        no_vertex_pipeline("snow", &self.pl, &self.shader, fs, &targets)
            .vs("vs_snow")
            .depth(depth_test(wgpu::CompareFunction::GreaterEqual))
            .samples(msaa)
            .create(device)
    }
}
