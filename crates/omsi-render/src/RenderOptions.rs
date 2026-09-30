pub struct RenderOptions {
    pub msaa: u32,
    pub anisotropy: u16,
    pub shadow_size: u32,
    pub ssao: bool,

    pub render_scale: f32,

    pub upscaler: Upscaler,
    pub dlss_quality: DlssQuality,

    pub compress_textures: bool,
    pub fxaa: bool,

    pub min_obj_size: f32,
    pub max_obj_dist: f32,

    pub omsi_shadow_casters: bool,
    pub reflections: bool,
}
