use super::upscaler::DlssQuality;

#[derive(Debug, Clone, Copy)]
pub struct DlssSettings {
    pub enabled: bool,
    pub quality: DlssQuality,
    pub sharpness: f32,
}

impl Default for DlssSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            quality: DlssQuality::Quality,
            sharpness: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DlssInput {
    pub width: u32,
    pub height: u32,
    pub render_scale: f32,
    pub near_plane: f32,
    pub far_plane: f32,
}

impl DlssInput {
    pub fn new(
        width: u32,
        height: u32,
        render_scale: f32,
        near_plane: f32,
        far_plane: f32,
    ) -> Self {
        Self {
            width,
            height,
            render_scale,
            near_plane,
            far_plane,
        }
    }

    pub fn render_width(&self) -> u32 {
        ((self.width as f32) * self.render_scale).round() as u32
    }

    pub fn render_height(&self) -> u32 {
        ((self.height as f32) * self.render_scale).round() as u32
    }
}

pub struct DlssRenderer {
    settings: DlssSettings,
}

impl DlssRenderer {
    pub fn new(settings: DlssSettings) -> Self {
        Self { settings }
    }

    pub fn enabled(&self) -> bool {
        self.settings.enabled
    }

    pub fn quality(&self) -> DlssQuality {
        self.settings.quality
    }

    pub fn render_scale(&self) -> f32 {
        self.settings.quality.render_scale()
    }

    pub fn settings(&self) -> DlssSettings {
        self.settings
    }

    pub fn set_settings(&mut self, settings: DlssSettings) {
        self.settings = settings;
    }

    pub fn input(
        &self,
        width: u32,
        height: u32,
        near_plane: f32,
        far_plane: f32,
    ) -> DlssInput {
        DlssInput::new(
            width,
            height,
            self.render_scale(),
            near_plane,
            far_plane,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dlss_input_calculates_render_resolution() {
        let input = DlssInput::new(
            1920,
            1080,
            0.67,
            0.1,
            1000.0,
        );

        assert_eq!(input.render_width(), 1286);
        assert_eq!(input.render_height(), 724);
    }
}