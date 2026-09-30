use super::upscaler::DlssQuality;

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
}
