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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dlss_quality_render_scales() {
        assert_eq!(DlssQuality::UltraPerformance.render_scale(), 0.33);
        assert_eq!(DlssQuality::Performance.render_scale(), 0.50);
        assert_eq!(DlssQuality::Balanced.render_scale(), 0.58);
        assert_eq!(DlssQuality::Quality.render_scale(), 0.67);
        assert_eq!(DlssQuality::UltraQuality.render_scale(), 0.77);
    }
}