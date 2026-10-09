//! Measured weather in physical units, independent of the service supplying it.
use omsi_content::weather::Weather;

#[derive(Debug, Clone)]
pub(crate) struct Observations {
    pub temperature_c: f32,
    pub humidity_percent: f32,
    pub wind: (f32, f32),
    pub pressure_hpa: f32,
    pub visibility_m: f32,
    pub cloud_cover: f32,
    pub cloud_base_m: f32,
    /// Liquid rain and solid snowfall depths per hour, not a renderer intensity.
    pub rain_mm_h: f32,
    pub snow_mm_h: f32,
}

impl Observations {
    pub(crate) fn weather(&self, wetness: f32) -> Weather {
        let temp = self.temperature_c.clamp(-40.0, 50.0);
        let cover = self.cloud_cover.clamp(0.0, 1.0);
        let rain = self.rain_mm_h.max(0.0);
        let snow = self.snow_mm_h.max(0.0);
        let kind = if snow > 0.0 {
            2
        } else if rain > 0.0 {
            1
        } else {
            0
        };
        // The bounded visual response grows smoothly through light and heavy rain.
        // 2.5 mm/h marks the light/moderate rain boundary; snowfall is a solid
        // depth, so its reference is ten times that. These are visual scales,
        // not conversions of millimetres into a measured particle count.
        let rate = if kind == 2 {
            snow / (snow + 25.0)
        } else {
            rain / (rain + 2.5)
        };
        let deck = if cover > 0.85 { cover } else { 0.0 };
        Weather {
            name: "Observed weather".into(),
            fog: (self.visibility_m.clamp(50.0, 50_000.0), 1.0),
            wind: (self.wind.0.rem_euclid(360.0), self.wind.1.clamp(0.0, 50.0)),
            temp: (
                temp,
                crate::weather_setup::absolute_humidity(temp, self.humidity_percent),
            ),
            pressure: self.pressure_hpa.clamp(900.0, 1100.0),
            clouds: (
                super::cloud_kind(cover, deck).into(),
                self.cloud_base_m.clamp(50.0, 5000.0),
            ),
            precip: vec![kind as f32, rate * 255.0, 0.0, 0.0, 0.0],
            // Preserve the water already on the road. The ordinary surface model
            // continues to soak/dry it as simulation frames pass, including LAN.
            ground_wet: [wetness.clamp(0.0, 1.0) * 255.0, 255.0, 115.0],
            snow: kind == 2 && temp < 1.5,
            snow_on_road: false,
            ..Default::default()
        }
    }
}

/// The surface model shared by presets, reports and measured providers.
pub(crate) fn road_wetness(rate: f32, secs: f64, start: f32) -> f32 {
    if rate > 0.0 {
        (start + secs as f32 * rate / 180.0).clamp(0.0, 1.0)
    } else {
        (start - secs as f32 / 1200.0).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> Observations {
        Observations {
            temperature_c: 12.0,
            humidity_percent: 80.0,
            wind: (370.0, 6.6),
            pressure_hpa: 1010.0,
            visibility_m: 2000.0,
            cloud_cover: 0.9,
            cloud_base_m: 700.0,
            rain_mm_h: 0.0,
            snow_mm_h: 0.0,
        }
    }
    #[test]
    fn observations_keep_measured_air_visibility_and_cloud() {
        let w = sample().weather(0.2);
        assert_eq!(w.temp.0, 12.0);
        assert_eq!(w.fog.0, 2000.0);
        assert_eq!(w.clouds, ("Overcast 1".into(), 700.0));
        assert_eq!(w.wind, (10.0, 6.6));
        assert_eq!(w.ground_wet[0], 51.0);
        assert_eq!(w.precip[1], 0.0);
    }
    #[test]
    fn rain_changes_strength_and_soaks_then_drains_over_time() {
        let mut o = sample();
        o.rain_mm_h = 0.1;
        let light = o.weather(0.0);
        o.rain_mm_h = 20.0;
        let heavy = o.weather(0.0);
        assert!(light.precip[1] > 0.0 && light.precip[1] < heavy.precip[1]);
        let wet = road_wetness(heavy.precip[1] / 255.0, 60.0, 0.0);
        assert!(wet > 0.0 && wet < 0.5);
        assert!(road_wetness(0.0, 60.0, wet) < wet);
        o.rain_mm_h = 0.0;
        assert_eq!(o.weather(wet).ground_wet[0], wet * 255.0);
    }
    #[test]
    fn snowfall_sets_snow_without_inventing_historical_road_cover() {
        let mut o = sample();
        o.temperature_c = -3.0;
        o.snow_mm_h = 10.0;
        let w = o.weather(0.0);
        assert_eq!(w.precip[0], 2.0);
        assert!(w.snow);
        assert!(!w.snow_on_road);
    }
}
