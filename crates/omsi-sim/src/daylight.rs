//! Sun position and the three envir.cfg lights (direct sun A, from above B, ambient C)
//! interpolated over the sun altitude (unit `mc_himmel`).

use crate::clock::SimClock;
use glam::Vec3;
use omsi_content::Envir;

#[derive(Debug, Clone, Copy)]
pub struct Daylight {
    /// Unit vector towards the sun (x east, y north, z up).
    pub sun_dir: Vec3,
    pub altitude_deg: f32,
    pub sun_color: Vec3,
    pub secondary: Vec3,
    pub ambient: Vec3,
    pub sky: Vec3,
    /// 0 by day, 1 by night (nightmaps, coronas).
    pub night: f32,
    /// Street lights on (the `NightlightA` scenery variable).
    pub lamps_on: bool,
    /// The daylight by the sun's altitude (1 day … 0 night, a ramp from +6 to -6 degrees):
    /// what the street lamps and the lit windows switch by.
    pub brightness: f32,
    /// The mean of envir.cfg's light A at this altitude (0..1, as Omsi.exe keeps it at
    /// weather +0xac): the day's part of the `Envir_Brightness` vehicle variable, see
    /// [`Daylight::envir_brightness`].
    pub light_a: f32,
    /// Sun azimuth in radians, clockwise from north.
    pub azimuth_rad: f32,
    /// Blend of the envir sky textures: day, twilight (peaks at sunrise), night.
    pub sky_weights: [f32; 3],
    /// envir.cfg's three light colours relative to the stock file's at this sun altitude
    /// (A, B, C; 1 = stock): the enhanced renderer tints its physical sky with them.
    pub envir_tint: [Vec3; 3],
    /// Unit vector towards the moon (x east, y north, z up).
    pub moon_dir: Vec3,
    /// How much of the moon's disc is lit (0 new moon .. 1 full moon).
    pub moon_illum: f32,
    /// The day of the year (1..366) and the map's latitude (degrees north): the season.
    pub day_of_year: f32,
    pub latitude: f32,
    /// One number per calendar day: what the enhanced renderer draws the day's own air
    /// from (how clear or hazy it is), so that no two days look quite alike.
    pub day_seed: u32,
}

/// Days since 2000-01-01 12:00 UTC (J2000) for the map clock at `place`.
fn days_j2000(clock: &SimClock, place: &SunPlace) -> f64 {
    let y = clock.year;
    // whole days from 2000-01-01 to the start of this year
    let mut days = 0i64;
    if y >= 2000 {
        for yy in 2000..y {
            days += crate::clock::days_in_year(yy) as i64;
        }
    } else {
        for yy in y..2000 {
            days -= crate::clock::days_in_year(yy) as i64;
        }
    }
    let utc_hours = clock.time / 3600.0 - place.timezone - place.dst_hours(clock);
    days as f64 + (clock.day_of_year - 1) as f64 + utc_hours / 24.0 - 0.5
}

/// The moon's altitude, azimuth (degrees; azimuth clockwise from north) and lit fraction
/// for the map clock at `place`: a low-precision lunar theory (the main terms of the
/// longitude and latitude, about half a degree), which is all a sky needs.
pub fn moon_position(clock: &SimClock, place: &SunPlace) -> (f64, f64, f64) {
    let d = days_j2000(clock, place);
    let rad = |x: f64| x.to_radians();
    let l = 218.316 + 13.176_396 * d;
    let m = 134.963 + 13.064_993 * d;
    let f = 93.272 + 13.229_350 * d;
    let lon = rad(l + 6.289 * rad(m).sin());
    let lat = rad(5.128 * rad(f).sin());
    let eps = rad(23.439);
    let ra = (lon.sin() * eps.cos() - lat.tan() * eps.sin()).atan2(lon.cos());
    let dec = (lat.sin() * eps.cos() + lat.cos() * eps.sin() * lon.sin()).asin();
    let gmst = 280.460_618_37 + 360.985_647_366_29 * d;
    let ha = rad(gmst + place.longitude) - ra;
    let phi = rad(place.latitude);
    let sin_alt = phi.sin() * dec.sin() + phi.cos() * dec.cos() * ha.cos();
    let alt = sin_alt.clamp(-1.0, 1.0).asin();
    let az = (-ha.sin()).atan2(dec.tan() * phi.cos() - phi.sin() * ha.cos());
    // the sun's ecliptic longitude, for the phase
    let g = rad(357.529 + 0.985_600_28 * d);
    let sun_lon = rad(280.459 + 0.985_647_36 * d) + rad(1.915) * g.sin() + rad(0.020) * (2.0 * g).sin();
    let elong = (lat.cos() * (lon - sun_lon).cos()).clamp(-1.0, 1.0).acos();
    let illum = (1.0 - elong.cos()) * 0.5;
    (alt.to_degrees(), az.to_degrees().rem_euclid(360.0), illum)
}

/// Where the map lies and how its clocks run, from its `timezone.txt` (OMSI
/// the original: `[timezone]` hours east of UTC, `[location]` latitude and longitude,
/// `[DST]` periods). Without the file the exe keeps Spandau: 52.505 N, 13.2782 E, UTC+1.
#[derive(Debug, Clone, PartialEq)]
pub struct SunPlace {
    pub latitude: f64,
    pub longitude: f64,
    pub timezone: f64,
    /// Daylight saving periods: first and last day (`YYYYMMDD`), the hour it starts on the
    /// first day and ends on the last, and the hours the clocks go forward.
    pub dst: Vec<(i32, i32, f32, f32, f32)>,
}

impl Default for SunPlace {
    fn default() -> Self {
        SunPlace { latitude: 52.505, longitude: 13.2782, timezone: 1.0, dst: Vec::new() }
    }
}

impl SunPlace {
    /// The hours the clock is ahead of standard time at this moment (0 outside summer time).
    pub fn dst_hours(&self, clock: &SimClock) -> f64 {
        let date = clock.date_code();
        let hour = clock.time / 3600.0;
        for &(start, end, h0, h1, offset) in &self.dst {
            let after_start = date > start || (date == start && hour >= h0 as f64);
            let before_end = date < end || (date == end && hour < h1 as f64);
            if after_start && before_end {
                return offset as f64;
            }
        }
        0.0
    }

    /// Local solar time (hours) for the map clock: the clock minus its time zone and summer
    /// time is UTC, and the sun stands south at 12:00 plus 4 minutes a degree west.
    pub fn solar_hours(&self, clock: &SimClock) -> f64 {
        clock.time / 3600.0 - self.timezone - self.dst_hours(clock) + self.longitude / 15.0
    }
}

static PLACE: std::sync::RwLock<Option<std::sync::Arc<SunPlace>>> = std::sync::RwLock::new(None);

/// The loaded map's place (see [`SunPlace`]); every sun position is computed for it.
pub fn set_place(p: SunPlace) {
    *PLACE.write().unwrap_or_else(|e| e.into_inner()) = Some(std::sync::Arc::new(p));
}

pub fn place() -> std::sync::Arc<SunPlace> {
    PLACE.read().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default()
}

/// Sun altitude and azimuth (degrees; azimuth clockwise from north) for the map clock at
/// `place`.
pub fn sun_position(clock: &SimClock, place: &SunPlace) -> (f64, f64) {
    let doy = clock.day_of_year as f64;
    let decl = (23.44f64).to_radians() * ((2.0 * std::f64::consts::PI * (284.0 + doy) / 365.0).sin());
    let hour = place.solar_hours(clock);
    let h = (15.0 * (hour - 12.0)).to_radians();
    let lat = place.latitude.to_radians();
    let sin_alt = lat.sin() * decl.sin() + lat.cos() * decl.cos() * h.cos();
    let alt = sin_alt.clamp(-1.0, 1.0).asin();
    let cos_az = (decl.sin() - lat.sin() * sin_alt) / (lat.cos() * alt.cos()).max(1e-6);
    let mut az = cos_az.clamp(-1.0, 1.0).acos();
    if h.sin() > 0.0 {
        az = 2.0 * std::f64::consts::PI - az;
    }
    (alt.to_degrees(), az.to_degrees())
}

/// The blend of the envir.cfg sky pictures (day, twilight, night) at a sun height of `alt`
/// degrees, as Omsi.exe draws its dome (0x5d8e98): above the end of the twilight the day
/// picture, below its start the night one, and between them the twilight picture blended
/// towards the day (above the horizon) or the night (below it) by a weight in 256ths of
/// the sun's height over the threshold. Omsi.exe turns the envir.cfg thresholds into
/// radians by pi/200 where its light colours take pi/180, so its sky reaches the day at
/// 0.9 of the twilight's end (9 degrees for the stock 10) and the night at 0.9 of its start.
pub fn omsi_sky_weights(alt: f32, twilight: (f32, f32)) -> [f32; 3] {
    let (start, end) = (twilight.0 * 0.9, twilight.1 * 0.9);
    if alt > end {
        [1.0, 0.0, 0.0]
    } else if alt > 0.0 && end > 0.0 {
        let w = ((alt * 256.0 / end).round()).min(255.0) / 255.0;
        [w, 1.0 - w, 0.0]
    } else if alt >= start && start < 0.0 {
        let w = ((alt * 256.0 / start).round()).min(255.0) / 255.0;
        [0.0, 1.0 - w, w]
    } else if alt > 0.0 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 0.0, 1.0]
    }
}

fn stops(colors: &[[f32; 3]; 5], twilight: (f32, f32), alt: f32) -> Vec3 {
    let alts = [-90.0, twilight.0, 0.0, twilight.1, 90.0];
    let mut i = 0;
    while i + 1 < 4 && alt > alts[i + 1] {
        i += 1;
    }
    let t = ((alt - alts[i]) / (alts[i + 1] - alts[i]).max(1e-3)).clamp(0.0, 1.0);
    let a = Vec3::from(colors[i]);
    let b = Vec3::from(colors[i + 1]);
    a.lerp(b, t) / 255.0
}

impl Daylight {
    pub fn compute(clock: &SimClock, envir: Option<&Envir>) -> Daylight {
        let (alt, az) = sun_position(clock, &place());
        let (sa, ca) = (alt.to_radians().sin() as f32, alt.to_radians().cos() as f32);
        let (saz, caz) = (az.to_radians().sin() as f32, az.to_radians().cos() as f32);
        let sun_dir = Vec3::new(ca * saz, ca * caz, sa).normalize_or_zero();
        let alt = alt as f32;
        let default_envir;
        let envir = match envir {
            Some(e) => e,
            None => {
                default_envir = default_envir_values();
                &default_envir
            }
        };
        let tw = envir.twilight_start_end;
        let a = stops(&envir.light_color_a, tw, alt);
        let b = stops(&envir.light_color_b, tw, alt);
        let c = stops(&envir.light_color_c, tw, alt);
        // scaled so that a sunlit white surface does not blow out
        let sun_color = a * 0.7;
        let secondary = b * 0.5;
        let ambient = (c * 0.45).max(Vec3::splat(0.05));
        // the sky follows the light from above; deep blue at day, near black at night
        let day = ((alt + 6.0) / 16.0).clamp(0.0, 1.0);
        let sky = Vec3::new(0.55, 0.70, 0.92) * day + Vec3::new(0.01, 0.01, 0.03) * (1.0 - day) + (a * 0.15 * (1.0 - day) * day * 4.0).min(Vec3::splat(0.3));
        let night = ((tw.1 - alt) / tw.1.max(1.0)).clamp(0.0, 1.0);
        let sky_weights = omsi_sky_weights(alt, tw);
        let stock = default_envir_values();
        let ratio = |own: Vec3, base: Vec3| -> Vec3 {
            // where the stock light is (nearly) black there is nothing to compare with
            let r = |o: f32, b: f32| if b > 2.0 / 255.0 { (o / b).clamp(0.25, 4.0) } else { 1.0 };
            Vec3::new(r(own.x, base.x), r(own.y, base.y), r(own.z, base.z))
        };
        let envir_tint = [
            ratio(a, stops(&stock.light_color_a, stock.twilight_start_end, alt)),
            ratio(b, stops(&stock.light_color_b, stock.twilight_start_end, alt)),
            ratio(c, stops(&stock.light_color_c, stock.twilight_start_end, alt)),
        ];
        // the street lamps come on below a light value of 0.6, as Omsi.exe switches them
        // (FUN_006ff1bc), the same value at which a scenery object's NightlightA does
        let brightness = ((alt + 6.0) / 12.0).clamp(0.0, 1.0);
        let light_a = ((a.x + a.y + a.z) / 3.0).clamp(0.0, 1.0);
        let here = place();
        let (m_alt, m_az, moon_illum) = moon_position(clock, &here);
        let (ma, mz) = (m_alt.to_radians(), m_az.to_radians());
        let moon_dir = Vec3::new((ma.cos() * mz.sin()) as f32, (ma.cos() * mz.cos()) as f32, ma.sin() as f32).normalize_or_zero();
        let day_seed = (clock.year.rem_euclid(10_000) as u32) * 400 + clock.day_of_year.max(0) as u32;
        Daylight {
            sun_dir,
            altitude_deg: alt,
            sun_color,
            secondary,
            ambient,
            sky,
            night,
            lamps_on: brightness < 0.6,
            brightness,
            light_a,
            azimuth_rad: az.to_radians() as f32,
            sky_weights,
            envir_tint,
            moon_dir,
            moon_illum: moon_illum as f32,
            day_of_year: clock.day_of_year as f32,
            latitude: here.latitude as f32,
            day_seed,
        }
    }
}

impl Daylight {
    /// The `Envir_Brightness` vehicle variable as Omsi.exe sets it for every road vehicle
    /// (0x7d8735): the mean of the light the tile's light map throws on the vehicle (its
    /// colour at the vehicle's place while the lamps are on, 0x61378c) plus the mean of
    /// light A, at most 1. The stock buses scale their windows' alpha by it (`[alphascale]
    /// Envir_Brightness` on Fenster_braun.tga and the like): by the sun's ramp alone it was
    /// 0 at every night, and a bus under the street lamps had no glass at all (#624).
    pub fn envir_brightness(&self, light_map: Option<Vec3>) -> f32 {
        let lm = light_map.filter(|_| self.lamps_on).map(|c| (c.x + c.y + c.z) / 3.0).unwrap_or(0.0);
        (self.light_a + lm.max(0.0)).clamp(0.0, 1.0)
    }
}

fn default_envir_values() -> Envir {
    Envir {
        twilight_start_end: (-18.0, 10.0),
        light_color_a: [[0.0; 3], [0.0; 3], [200.0, 100.0, 20.0], [255.0, 255.0, 240.0], [255.0; 3]],
        light_color_b: [[5.0, 5.0, 10.0], [10.0, 10.0, 20.0], [30.0, 20.0, 40.0], [60.0, 70.0, 80.0], [80.0, 90.0, 100.0]],
        light_color_c: [[2.0, 2.0, 5.0], [15.0, 15.0, 25.0], [40.0, 50.0, 60.0], [220.0; 3], [230.0, 230.0, 255.0]],
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn the_sky_pictures_blend_as_omsi_blends_them() {
        let tw = (-18.0, 10.0);
        assert_eq!(omsi_sky_weights(30.0, tw), [1.0, 0.0, 0.0]);
        // (the day reached at 9 degrees, 0.9 of the twilight's end)
        assert_eq!(omsi_sky_weights(9.5, tw), [1.0, 0.0, 0.0]);
        assert_eq!(omsi_sky_weights(0.0, tw), [0.0, 1.0, 0.0]);
        let w = omsi_sky_weights(4.5, tw);
        assert!((w[0] - 128.0 / 255.0).abs() < 1e-6 && (w[0] + w[1] - 1.0).abs() < 1e-6, "{w:?}");
        let w = omsi_sky_weights(-8.1, tw);
        assert!((w[2] - 128.0 / 255.0).abs() < 1e-6 && w[0] == 0.0, "{w:?}");
        assert_eq!(omsi_sky_weights(-17.0, tw), [0.0, 0.0, 1.0]);
    }

    use super::*;

    fn clock_at(hours: f64) -> SimClock {
        SimClock { day_of_year: 182, time: hours * 3600.0, ..Default::default() }
    }

    #[test]
    fn the_sun_follows_the_maps_time_zone_and_summer_time() {
        let mut spandau = SunPlace::default();
        spandau.dst.push((19890326, 19890924, 2.0, 3.0, 1.0));
        // 21 June 1989: summer time, the sun due south at about 13:07 local
        let mut c = SimClock { year: 1989, day_of_year: 172, time: 13.12 * 3600.0, ..Default::default() };
        let (alt, az) = sun_position(&c, &spandau);
        assert!((az - 180.0).abs() < 3.0 && (alt - 61.0).abs() < 1.5, "summer noon: alt {alt:.1} az {az:.1}");
        // 21 December: standard time, south at about 12:07
        c.day_of_year = 355;
        c.time = 12.12 * 3600.0;
        let (alt, az) = sun_position(&c, &spandau);
        assert!((az - 180.0).abs() < 3.0 && (alt - 14.0).abs() < 1.5, "winter noon: alt {alt:.1} az {az:.1}");
        // without summer time the June sun would be an hour further on
        c.day_of_year = 172;
        c.time = 13.12 * 3600.0;
        let (_, az) = sun_position(&c, &SunPlace::default());
        assert!(az > 195.0, "no DST: az {az:.1}");
    }

    #[test]
    fn the_moon_rises_and_waxes_on_time() {
        // full moon of 20 January 2000 04:40 UTC: nearly all lit, opposite the sun, high
        // in a Berlin winter night
        let berlin = SunPlace { latitude: 52.5, longitude: 13.4, timezone: 0.0, dst: Vec::new() };
        let c = SimClock { year: 2000, day_of_year: 20, time: 4.67 * 3600.0, ..Default::default() };
        let (alt, _, illum) = moon_position(&c, &berlin);
        assert!(illum > 0.98, "illum {illum}");
        assert!(alt > 10.0, "alt {alt}");
        // new moon of 6 January 2000 18:14 UTC: dark
        let c = SimClock { year: 2000, day_of_year: 6, time: 18.23 * 3600.0, ..Default::default() };
        assert!(moon_position(&c, &berlin).2 < 0.02);
        // first quarter of 14 January 2000 13:34 UTC: half lit, in the south-east to south
        // in the afternoon
        let c = SimClock { year: 2000, day_of_year: 14, time: 13.57 * 3600.0, ..Default::default() };
        let (alt, az, illum) = moon_position(&c, &berlin);
        assert!((illum - 0.5).abs() < 0.08, "illum {illum}");
        assert!((90.0..200.0).contains(&az) && alt > 0.0, "alt {alt} az {az}");
    }

    #[test]
    fn envir_brightness_is_light_a_plus_the_light_map_under_the_lamps() {
        // a December midnight: no light A, so only the light map's light counts
        let night = Daylight::compute(&SimClock { day_of_year: 355, time: 0.0, ..Default::default() }, None);
        assert!(night.lamps_on);
        assert_eq!(night.envir_brightness(None), 0.0);
        let lit = night.envir_brightness(Some(Vec3::new(0.6, 0.5, 0.4)));
        assert!((lit - 0.5).abs() < 1e-5, "{lit}");
        // noon: light A alone is (nearly) 1, and the light map is off by day
        let noon = Daylight::compute(&clock_at(12.0), None);
        assert!(noon.envir_brightness(None) > 0.95);
        assert_eq!(noon.envir_brightness(Some(Vec3::ONE)), noon.envir_brightness(None));
    }

    #[test]
    fn stock_envir_leaves_the_enhanced_sky_untinted() {
        let own = default_envir_values();
        for h in [3.0, 6.0, 12.0, 19.0, 23.0] {
            assert_eq!(Daylight::compute(&clock_at(h), None).envir_tint, [Vec3::ONE; 3], "{h} h");
            assert_eq!(Daylight::compute(&clock_at(h), Some(&own)).envir_tint, [Vec3::ONE; 3], "{h} h");
        }
    }

    #[test]
    fn a_warmer_envir_tints_the_sun() {
        let mut warm = default_envir_values();
        // the midday sun as warm as a late afternoon's
        warm.light_color_a[4] = [255.0, 230.0, 170.0];
        warm.light_color_a[3] = [255.0, 230.0, 160.0];
        let d = Daylight::compute(&clock_at(12.0), Some(&warm));
        let a = d.envir_tint[0];
        assert!((a.x - 1.0).abs() < 1e-3 && a.y < 0.95 && a.z < 0.7, "{a:?}");
        assert_eq!(d.envir_tint[1], Vec3::ONE);
        // by night the stock sun is black: nothing to compare with, no tint
        assert_eq!(Daylight::compute(&clock_at(1.0), Some(&warm)).envir_tint[0], Vec3::ONE);
    }
}
