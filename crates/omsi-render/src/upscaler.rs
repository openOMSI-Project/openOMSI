#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Upscaler {
    Native,
    Dlss,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DlssQuality {
    UltraPerformance,
    Performance,
    Balanced,
    Quality,
    UltraQuality,
}

impl DlssQuality {
    pub fn render_scale(self) -> f32 {
        match self {
            Self::UltraPerformance => 0.33,
            Self::Performance => 0.50,
            Self::Balanced => 0.58,
            Self::Quality => 0.67,
            Self::UltraQuality => 0.77,
        }
    }
}
