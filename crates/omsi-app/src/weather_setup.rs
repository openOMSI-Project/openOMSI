//! The weather of a session: its `.owt` file, the sky and clouds it makes, and how wet the streets are.

use super::*;

pub(crate) const CUSTOM_CLOUDS: [&str; 5] = ["No clouds", "Cumulus 1", "Cumulus 2", "Cumulus 3", "Overcast 1"];
pub(crate) const CUSTOM_PRECIP: [&str; 3] = ["None", "Rain", "Snow"];

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CustomWeather {
    pub visibility_m: f32,
    pub brightness: f32,
    pub wind_dir: f32,
    pub wind_speed: f32,
    pub temp_c: f32,
    pub humidity: f32,
    pub pressure: f32,
    pub cloud: usize,
    pub cloud_base_m: f32,
    pub precip: i32,
    pub precip_intensity: f32,
    pub road_wetness: f32,
    pub snow_cover: bool,
    pub snow_on_road: bool,
}
impl Default for CustomWeather {
    fn default() -> Self {
        Self { visibility_m: 50_000.0, brightness: 1.0, wind_dir: 0.0, wind_speed: 0.0,
            temp_c: 15.0, humidity: 51.0, pressure: 1013.0, cloud: 0, cloud_base_m: 50.0,
            precip: 0, precip_intensity: 32.0, road_wetness: 0.0, snow_cover: false, snow_on_road: false }
    }
}
impl CustomWeather {
    pub(crate) fn normalize(&mut self) {
        self.visibility_m=self.visibility_m.clamp(50.0,50_000.0);
        self.brightness=self.brightness.clamp(0.0,1.5);
        self.wind_dir=self.wind_dir.rem_euclid(360.0);
        self.wind_speed=self.wind_speed.clamp(0.0,50.0);
        self.temp_c=self.temp_c.clamp(-40.0,50.0);
        self.humidity=self.humidity.clamp(0.0,100.0);
        self.pressure=self.pressure.clamp(900.0,1100.0);
        self.cloud=self.cloud.min(CUSTOM_CLOUDS.len()-1);
        self.cloud_base_m=self.cloud_base_m.clamp(50.0,5000.0);
        self.precip=self.precip.clamp(0,2);
        self.precip_intensity=self.precip_intensity.clamp(0.0,255.0);
        self.road_wetness=self.road_wetness.clamp(0.0,1.0);
    }
    pub(crate) fn parse(text:&str)->Option<Self>{
        let body=text.strip_prefix("custom:").or_else(||text.strip_prefix("CUSTOM:"))?;
        let mut c=Self::default();
        for part in body.split(';') {
            let Some((key,value))=part.split_once('=') else {continue};
            let n=value.trim().parse::<f32>().ok();
            match key.trim().to_ascii_lowercase().as_str() {
                "vis"=>if let Some(v)=n{c.visibility_m=v},
                "br"=>if let Some(v)=n{c.brightness=v},
                "wd"=>if let Some(v)=n{c.wind_dir=v},
                "ws"=>if let Some(v)=n{c.wind_speed=v},
                "t"=>if let Some(v)=n{c.temp_c=v},
                "rh"=>if let Some(v)=n{c.humidity=v},
                "p"=>if let Some(v)=n{c.pressure=v},
                "c"=>if let Some(v)=n{c.cloud=v.round().max(0.0) as usize},
                "cb"=>if let Some(v)=n{c.cloud_base_m=v},
                "pt"=>if let Some(v)=n{c.precip=v.round() as i32},
                "pi"=>if let Some(v)=n{c.precip_intensity=v},
                "wet"=>if let Some(v)=n{c.road_wetness=v},
                "snow"=>if let Some(v)=n{c.snow_cover=v>=0.5},
                "snowroad"=>if let Some(v)=n{c.snow_on_road=v>=0.5},
                _=>{}
            }
        }
        c.normalize(); Some(c)
    }
    pub(crate) fn encode(&self)->String{
        let mut c=self.clone(); c.normalize();
        format!("custom:vis={:.0};br={:.2};wd={:.0};ws={:.1};t={:.1};rh={:.0};p={:.0};c={};cb={:.0};pt={};pi={:.0};wet={:.2};snow={};snowroad={}",
            c.visibility_m,c.brightness,c.wind_dir,c.wind_speed,c.temp_c,c.humidity,c.pressure,c.cloud,c.cloud_base_m,
            c.precip,c.precip_intensity,c.road_wetness,c.snow_cover as u8,c.snow_on_road as u8)
    }
    pub(crate) fn from_weather(w:&omsi_content::weather::Weather,brightness:f32,wetness:f32)->Self{
        let kind=w.clouds.0.trim().to_ascii_lowercase();
        let cloud=if kind.starts_with("cumulus 1"){1}else if kind.starts_with("cumulus 2"){2}else if kind.starts_with("cumulus 3"){3}else if kind.starts_with("overcast"){4}else{0};
        let mut c=Self{
            visibility_m:w.fog.0,brightness,wind_dir:w.wind.0,wind_speed:w.wind.1,temp_c:w.temp.0,
            humidity:relative_humidity(w.temp.0,w.temp.1),pressure:if w.pressure>0.0{w.pressure}else{1013.0},
            cloud,cloud_base_m:w.clouds.1.max(50.0),precip:w.precip.first().copied().unwrap_or(0.0).round() as i32,
            precip_intensity:w.precip.get(1).copied().unwrap_or(32.0),road_wetness:wetness,snow_cover:w.snow,snow_on_road:w.snow_on_road};
        c.normalize(); c
    }
    pub(crate) fn to_weather(&self)->omsi_content::weather::Weather{
        let mut c=self.clone(); c.normalize();
        let cloud=match c.cloud{1=>"Cumulus 1",2=>"Cumulus 2",3=>"Cumulus 3",4=>"Overcast 1",_=>"-1"};
        omsi_content::weather::Weather{
            path:std::path::PathBuf::from(c.encode()),name:"Custom weather".into(),description:"User-defined weather".into(),
            fog:(c.visibility_m,1.0),wind:(c.wind_dir,c.wind_speed),temp:(c.temp_c,absolute_humidity(c.temp_c,c.humidity)),
            pressure:c.pressure,clouds:(cloud.into(),c.cloud_base_m),precip:vec![c.precip as f32,c.precip_intensity,0.0,0.0,0.0],
            ground_wet:[c.road_wetness*255.0,0.0,0.0],snow:c.snow_cover,snow_on_road:c.snow_on_road}
    }
}
fn saturation_vapour_pressure(temp_c:f32)->f32{6.112*((17.67*temp_c)/(temp_c+243.5)).exp()}
pub(crate) fn absolute_humidity(temp_c:f32,relative:f32)->f32{
    let vapour=saturation_vapour_pressure(temp_c)*relative.clamp(0.0,100.0)/100.0;
    (216.7*vapour/(temp_c+273.15).max(1.0)).max(0.0)
}
pub(crate) fn relative_humidity(temp_c:f32,absolute:f32)->f32{
    let vapour=absolute.max(0.0)*(temp_c+273.15).max(1.0)/216.7;
    (vapour/saturation_vapour_pressure(temp_c).max(0.001)*100.0).clamp(0.0,100.0)
}
pub(crate) fn dew_point_c(temp_c:f32,relative:f32)->f32{
    let rh=(relative.clamp(0.1,100.0)/100.0).ln();
    let g=rh+17.67*temp_c/(243.5+temp_c); 243.5*g/(17.67-g)
}
pub(crate) fn custom_weather(text:Option<&str>)->Option<CustomWeather>{text.and_then(CustomWeather::parse)}


/// Weather from `--weather`, else the clear-sky default.
pub(crate) fn load_weather(args: &Args) -> omsi_content::weather::Weather {
    let rel = args
        .weather
        .clone()
        .filter(|w| !crate::weather_cycle::is_cycle(Some(w)))
        .unwrap_or_else(|| "Weather/#CAVOK.owt".into());
    if let Some(w)=CustomWeather::parse(&rel).map(|c|c.to_weather()){
        log::info!("weather custom: fog range {} m, precip {:?}, temp {:?}",w.fog.0,w.precip,w.temp);
        scene::SNOW_WEATHER.store(w.snow,std::sync::atomic::Ordering::Relaxed);
        omsi_sim::host::set_ambient_weather(w.temp.0,w.temp.1);
        return w;
    }
    // OMSI 2's current weather: `metar:<ICAO>` fetches the airport's report
    let loaded = if rel.starts_with(REPORT) {
        // a report the host or server tells (see `report_wire`): its values, no download
        Ok(from_report(&rel).unwrap_or_else(|| omsi_content::weather::from_metar("", "CAVOK")))
    } else {
        match rel.strip_prefix("metar:").or_else(|| rel.strip_prefix("METAR:")) {
            Some(icao) => Ok(fetch_metar(icao.trim())),
            None => omsi_content::weather::Weather::load(&omsi_cfg::resolve_path(&args.root, &rel)),
        }
    };
    match loaded {
        Ok(w) => {
            log::info!(
                "weather {}: fog range {} m, precip {:?}, temp {:?}",
                w.name,
                w.fog.0,
                w.precip,
                w.temp
            );
            let mut w = w;
            // No weather chosen: the clear default (#CAVOK, no cloud at all) gets a few fair-
            // weather cumulus clouds, as long as clouds are wanted - a sky without a single
            // cloud was the first thing that looked wrong.
            if args.weather.is_none() && CLOUDS.load(std::sync::atomic::Ordering::Relaxed) && w.clouds.0.trim().starts_with("-1") {
                w.clouds = ("Cumulus 1".into(), 100.0);
            }
            // the vehicles ask for it while they go onto the GPU (the snow on the panes)
            scene::SNOW_WEATHER.store(w.snow, std::sync::atomic::Ordering::Relaxed);
            // and their {init} reads the temperature
            omsi_sim::host::set_ambient_weather(w.temp.0, w.temp.1);
            w
        }
        Err(e) => {
            log::warn!("weather {rel}: {e}");
            scene::SNOW_WEATHER.store(false, std::sync::atomic::Ordering::Relaxed);
            omsi_content::weather::Weather {
                fog: (50000.0, 1.0),
                ..Default::default()
            }
        }
    }
}

/// Sky gradient textures from envir.cfg uploaded into the scene.
pub(crate) fn setup_sky(
    args: &Args,
    renderer: &Renderer,
    scene: &mut Scene,
    envir: Option<&omsi_content::Envir>,
    weather: Option<&omsi_content::weather::Weather>,
) {
    const STOCK: [&str; 3] = ["Texture\\himmel01.bmp", "Texture\\himmel04.bmp", "Texture\\himmel05.bmp"];
    let names = envir.map(|e| e.sky_textures.clone()).unwrap_or_else(|| STOCK.map(String::from));
    let mut ids = Vec::new();
    for (n, stock) in names.iter().zip(STOCK) {
        let p = omsi_cfg::resolve_path(&args.root, n);
        // a sky pack's picture that cannot be read leaves the stock one in its place: giving
        // up here took the clouds with it, whatever the weather (#749)
        let img = omsi_texture::decode_file(&p).or_else(|e| {
            log::warn!("sky texture {}: {e}", p.display());
            omsi_texture::decode_file(&omsi_cfg::resolve_path(&args.root, stock))
        });
        match img {
            Ok(img) => ids.push(renderer.add_texture(scene, &img, false)),
            Err(e) => {
                log::warn!("sky texture {stock}: {e}");
                return;
            }
        }
    }
    // the weather's cloud type (Weather/clouds.cfg: Cumulus 1..3, Overcast 1) with its own
    // texture, whose alpha is the clouds' shape; `Texture\clouds.tga` when there is none
    let kind = weather.map(|w| w.clouds.0.trim().to_string()).unwrap_or_default();
    let typed = cloud_texture(&args.root, &kind);
    let cover = typed.or_else(|| omsi_texture::decode_file(&omsi_cfg::resolve_path(&args.root, "Texture\\clouds.tga")).ok());
    let t = std::time::Instant::now();
    let field = cloud_field(cover.as_ref());
    log::debug!("cloud field made in {:.0} ms", t.elapsed().as_secs_f64() * 1000.0);
    let clouds = Some(renderer.add_texture(scene, &field, true));
    renderer.set_sky_textures_clouds(scene, [ids[0], ids[1], ids[2]], clouds);
}

/// Edge of the cloud field texture (texels); it tiles.
const FIELD: usize = 512;

/// The texture both sky shaders draw their clouds from, seamless in both directions:
/// R the weather's own cloud picture (Weather/clouds.cfg) - where its clouds are, the sky
/// has more; G the shape of the cumulus, fractal noise bent by more noise, equalised so
/// that a threshold of 1 - f covers exactly the fraction f of the sky; B billows for the
/// cauliflower edges; A how tall each cloud grows. Stored in sRGB bytes so that the
/// shader reads the values back as they were made.
pub(crate) fn cloud_field(cover: Option<&omsi_texture::Image>) -> omsi_texture::Image {
    let n = FIELD;
    let lattice = |x: i64, y: i64, period: i64, seed: u32| -> f32 {
        let (x, y) = (x.rem_euclid(period) as u32, y.rem_euclid(period) as u32);
        let mut h = x.wrapping_mul(0x8da6_b343) ^ y.wrapping_mul(0xd816_3841) ^ seed.wrapping_mul(0xcb1a_b31f);
        h ^= h >> 13;
        h = h.wrapping_mul(0x5bd1_e995);
        h ^= h >> 15;
        (h & 0xffff) as f32 / 65535.0
    };
    // value noise with `period` cells across the tile, at a point of the tile (0..1)
    let noise = |u: f32, v: f32, period: i64, seed: u32| -> f32 {
        let (x, y) = (u * period as f32, v * period as f32);
        let (ix, iy) = (x.floor() as i64, y.floor() as i64);
        let (fx, fy) = (x - ix as f32, y - iy as f32);
        let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
        let a = lattice(ix, iy, period, seed) + (lattice(ix + 1, iy, period, seed) - lattice(ix, iy, period, seed)) * sx;
        let b = lattice(ix, iy + 1, period, seed) + (lattice(ix + 1, iy + 1, period, seed) - lattice(ix, iy + 1, period, seed)) * sx;
        a + (b - a) * sy
    };
    let fbm = |u: f32, v: f32, period: i64, octaves: u32, seed: u32| -> f32 {
        let (mut sum, mut amp, mut norm, mut p) = (0.0, 0.5, 0.0, period);
        for o in 0..octaves {
            sum += amp * noise(u, v, p, seed + o * 101);
            norm += amp;
            amp *= 0.5;
            p *= 2;
        }
        sum / norm
    };
    let mut shape = vec![0f32; n * n];
    let mut billow = vec![0f32; n * n];
    let mut tall = vec![0f32; n * n];
    // (rows in parallel: 512 x 512 texels of a few octaves each)
    use rayon::prelude::*;
    shape
        .par_chunks_mut(n)
        .zip(billow.par_chunks_mut(n))
        .zip(tall.par_chunks_mut(n))
        .enumerate()
        .for_each(|(y, ((s_row, b_row), t_row))| {
            let v = y as f32 / n as f32;
            for x in 0..n {
                let u = x as f32 / n as f32;
                // bend the cumulus field with a coarser one, so that clouds are not blobs on a grid
                let wu = fbm(u, v, 4, 3, 11) - 0.5;
                let wv = fbm(u, v, 4, 3, 23) - 0.5;
                s_row[x] = fbm(u + wu * 0.12, v + wv * 0.12, 6, 5, 37);
                let mut b = 0.0;
                let mut amp = 0.5;
                let mut p = 24;
                for o in 0..3 {
                    b += amp * (1.0 - (noise(u, v, p, 53 + o) * 2.0 - 1.0).abs());
                    amp *= 0.5;
                    p *= 2;
                }
                b_row[x] = b / 0.875;
                t_row[x] = fbm(u, v, 3, 2, 71);
            }
        });
    // equalise the shape: its rank, so that a threshold is a fraction of the sky
    let mut order: Vec<u32> = (0..(n * n) as u32).collect();
    order.par_sort_unstable_by(|a, b| shape[*a as usize].total_cmp(&shape[*b as usize]));
    let mut equal = vec![0f32; n * n];
    for (rank, &i) in order.iter().enumerate() {
        equal[i as usize] = rank as f32 / (n * n - 1) as f32;
    }
    let stretch = |v: &mut Vec<f32>| {
        let (lo, hi) = v.iter().fold((f32::MAX, f32::MIN), |(a, b), &x| (a.min(x), b.max(x)));
        for x in v.iter_mut() {
            *x = (*x - lo) / (hi - lo).max(1e-6);
        }
    };
    stretch(&mut billow);
    stretch(&mut tall);
    let to_srgb = |x: f32| -> u8 {
        let x = x.clamp(0.0, 1.0);
        let s = if x <= 0.003_130_8 { x * 12.92 } else { 1.055 * x.powf(1.0 / 2.4) - 0.055 };
        (s * 255.0 + 0.5) as u8
    };
    // the weather's picture, sampled bilinearly (it tiles as well), its brightness the cover
    let weather = |u: f32, v: f32| -> f32 {
        let Some(img) = cover else { return 0.5 };
        let (w, h) = (img.width as i64, img.height as i64);
        let (x, y) = (u * w as f32 - 0.5, v * h as f32 - 0.5);
        let (ix, iy) = (x.floor() as i64, y.floor() as i64);
        let (fx, fy) = (x - ix as f32, y - iy as f32);
        let px = |xx: i64, yy: i64| {
            let i = ((yy.rem_euclid(h) * w + xx.rem_euclid(w)) * 4) as usize;
            (img.rgba[i] as f32 + img.rgba[i + 1] as f32 + img.rgba[i + 2] as f32) / (3.0 * 255.0)
        };
        let a = px(ix, iy) + (px(ix + 1, iy) - px(ix, iy)) * fx;
        let b = px(ix, iy + 1) + (px(ix + 1, iy + 1) - px(ix, iy + 1)) * fx;
        a + (b - a) * fy
    };
    let mut rgba = vec![0u8; n * n * 4];
    for y in 0..n {
        for x in 0..n {
            let i = y * n + x;
            let o = i * 4;
            rgba[o] = to_srgb(weather(x as f32 / n as f32, y as f32 / n as f32));
            rgba[o + 1] = to_srgb(equal[i]);
            rgba[o + 2] = to_srgb(billow[i]);
            // (alpha is not sRGB-coded)
            rgba[o + 3] = (tall[i].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        }
    }
    omsi_texture::Image { width: n as u32, height: n as u32, rgba, has_alpha: true }
}

/// The texture of a cloud type named in `Weather/clouds.cfg` (`[cloudtype]` name, texture,
/// size in metres, sct/ovc), as the sky shaders read it: the cover in the colour channels.
/// A scattered type's texture is white with the clouds in its alpha; an overcast one is a
/// picture of the cloud deck, its brightness is the cover.
fn cloud_texture(root: &Path, kind: &str) -> Option<omsi_texture::Image> {
    if kind.is_empty() || kind.starts_with("-1") {
        return None;
    }
    let cfg = omsi_cfg::vfs::read(&root.join("Weather").join("clouds.cfg")).ok()?;
    let text = omsi_cfg::codepage::decode(&cfg);
    let lines: Vec<&str> = text.lines().map(|l| l.trim()).collect();
    let mut file = None;
    for i in 0..lines.len() {
        if lines[i].eq_ignore_ascii_case("[cloudtype]") && lines.get(i + 1).map(|n| n.eq_ignore_ascii_case(kind)).unwrap_or(false) {
            file = lines.get(i + 2).map(|s| s.to_string());
            break;
        }
    }
    let file = file?;
    let mut img = omsi_texture::decode_file(&omsi_cfg::resolve_path(&omsi_cfg::resolve_path(&root, "Texture"), &file)).ok()?;
    if img.has_alpha {
        for px in img.rgba.chunks_mut(4) {
            let a = px[3];
            px[0] = a;
            px[1] = a;
            px[2] = a;
            px[3] = 255;
        }
        img.has_alpha = false;
    }
    log::info!("clouds: {kind} ({file})");
    Some(img)
}

/// Cloud cover of a weather file: `[clouds] type density`, type -1 = clear, density up to
/// ~300 (Cumulus 3) - mapped to 0..1; the cover drifts with the wind.
pub(crate) fn clouds_of(w: &omsi_content::weather::Weather, drift: [f32; 2]) -> (f32, [f32; 2]) {
    let kind = w.clouds.0.trim();
    if kind.is_empty() || kind.starts_with("-1") || !CLOUDS.load(std::sync::atomic::Ordering::Relaxed) {
        return (0.0, [0.0; 2]);
    }
    // cover by type: Cumulus 1..3 scattered → broken, Overcast closed
    let lower = kind.to_ascii_lowercase();
    let density = if lower.starts_with("overcast") {
        1.0
    } else if lower.starts_with("cumulus") {
        match lower
            .trim_start_matches("cumulus")
            .trim()
            .parse::<i32>()
            .unwrap_or(1)
        {
            1 => 0.35,
            2 => 0.55,
            _ => 0.75,
        }
    } else {
        0.5
    };
    (density, drift)
}

/// The clouds' drift after `time` seconds of a steady wind (see `cloud_drift_step`).
pub(crate) fn cloud_drift_at(w: &omsi_content::weather::Weather, time: f64) -> [f32; 2] {
    let mut d = [0.0; 2];
    cloud_drift_step(&mut d, w, time);
    d
}

/// Move the clouds on by `secs` of [wind] direction (deg) speed (m/s), over the 2500 m
/// tiling; the field repeats every tile, so only the fraction of a tile is kept. (Taken
/// from the absolute time, every change of the wind while a weather blends in moved the
/// whole sky by time x change.)
pub(crate) fn cloud_drift_step(d: &mut [f32; 2], w: &omsi_content::weather::Weather, secs: f64) {
    let (dir, speed) = (w.wind.0.to_radians() as f64, w.wind.1 as f64);
    let s = secs * speed / 2500.0;
    d[0] = (d[0] as f64 + dir.sin() * s).rem_euclid(1.0) as f32;
    d[1] = (d[1] as f64 + dir.cos() * s).rem_euclid(1.0) as f32;
}

/// How wet the roads are: rain soaks them in a few minutes, sunshine dries them in about
/// twenty. `secs` is how long this weather has been running.
pub(crate) fn road_wetness(rate: f32, secs: f64, start: f32) -> f32 {
    if rate > 0.0 {
        (start + secs as f32 * rate / 180.0).clamp(0.0, 1.0)
    } else {
        (start - secs as f32 / 1200.0).clamp(0.0, 1.0)
    }
}

/// The renderer's lighting for this weather at this moment: the daylight, then what the
/// cloud cover, the rain and the snow make of it.
pub(crate) fn weather_lighting(
    daylight: &omsi_sim::Daylight,
    w: &omsi_content::weather::Weather,
    cloud_drift: [f32; 2],
    wetness: f32,
    shadows: bool,
) -> omsi_render::Lighting {
    let mut lighting = lights::lighting_from(daylight, w.fog.0);
    lighting.enhanced = ENHANCED.load(std::sync::atomic::Ordering::Relaxed);
    lighting.classic = CLASSIC.load(std::sync::atomic::Ordering::Relaxed);
    let (density, offset) = clouds_of(w, cloud_drift);
    lighting.cloud_density = density;
    lighting.cloud_offset = offset;
    let (kind, rate) = precip_of(w);
    lights::apply_weather(
        &mut lighting,
        density,
        kind,
        rate,
        if w.snow { 1.0 } else { 0.0 },
    );
    if let Some(custom)=CustomWeather::parse(&w.path.to_string_lossy()){
        let k=custom.brightness;
        lighting.sun_intensity*=k;
        // (Enhanced: the automatic exposure takes the picture back up to the eye's level
        // whatever the weather's brightness, so there it only takes the sun away. The
        // mirrors, drawn with the plain shading and graded by no exposure, went dark with
        // the light instead - black at 0 % beside a window still in daylight, #1018.)
        if !lighting.enhanced {
            lighting.secondary*=k;
            lighting.ambient*=k;
            lighting.sky_color*=k;
            lighting.fog_color*=k;
        }
    }
    lighting.wetness = wetness;
    // Omsi.exe hides the sun under an 'ovc' cloud type (the Overcast ones in clouds.cfg) and
    // draws no sun shadows below 350 m visibility
    let overcast = w.clouds.0.trim().to_ascii_lowercase().starts_with("overcast");
    lighting.shadows = shadows && !overcast && w.fog.0 > 350.0;
    lighting
}

/// Push the weather into a vehicle's script host.
pub(crate) fn precip_of(w: &omsi_content::weather::Weather) -> (i32, f32) {
    let kind = w.precip.first().copied().unwrap_or(0.0) as i32;
    let rate = if kind == 0 {
        0.0
    } else {
        (w.precip.get(1).copied().unwrap_or(0.0) / 255.0).clamp(0.0, 1.0)
    };
    (kind, rate)
}

pub(crate) fn apply_weather(
    v: &mut omsi_sim::VehicleInstance,
    w: &omsi_content::weather::Weather,
    wetness: f32,
) {
    let (kind, rate) = precip_of(w);
    v.host.precip_type = kind as f32;
    v.host.precip_rate = rate;
    v.host.wind = crate::rain::weather_wind(w);
    v.host.street_cond = street_condition(w, wetness);
    v.set_var("PrecipType", kind as f32);
    v.set_var("PrecipRate", rate);
    v.host.temperature = w.temp.0;
    v.host.abs_humidity = w.temp.1;
    omsi_sim::host::set_ambient_weather(w.temp.0, w.temp.1);
}

/// `OMSI_DEBUG_SOUND[=seconds]`: how often the mixer says what every sound entry of the
/// player's bus is doing, and what the environment sounds play (5 s by default).
pub(crate) fn debug_sound_every() -> Option<f32> {
    static EVERY: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
    *EVERY.get_or_init(|| {
        omsi_cfg::env::var("OMSI_DEBUG_SOUND")
            .ok()
            .map(|v| v.parse::<f32>().ok().filter(|s| *s > 0.0).unwrap_or(5.0))
    })
}

/// The road under the wheels as the scripts and the sound configurations read it
/// (`StreetCond`): 0 dry, 1 wet, 2 covered in snow. A snowfall (or a weather with
/// `[snowOnRoad]`) puts it into the upper half, everything else follows the water film.
pub(crate) fn street_condition(w: &omsi_content::weather::Weather, wetness: f32) -> f32 {
    let wet = wetness.clamp(0.0, 1.0);
    if w.snow_on_road || precip_of(w).0 == 2 {
        1.0 + wet
    } else {
        wet
    }
}

/// How wet the roads are when a session starts: what this weather has already left on them
/// (`[groundwet]`, 0 … 255), and at least what the rain falling now would soak them to.
pub(crate) fn initial_wetness(w: &omsi_content::weather::Weather) -> f32 {
    let rate = precip_of(w).1;
    let falling = if rate > 0.0 {
        (0.4 + rate).min(1.0)
    } else {
        0.0
    };
    (w.ground_wet[0] / 255.0).clamp(0.0, 1.0).max(falling)
}


/// The airports OMSI's METAR list (`Weather/ICAO.txt`) offers: (ICAO, "ICAO - name").
pub(crate) fn metar_airports(root:&std::path::Path)->Vec<(String,String)>{
    static CACHE:std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<std::path::PathBuf,Vec<(String,String)>>>>=std::sync::OnceLock::new();
    let cache=CACHE.get_or_init(||std::sync::Mutex::new(std::collections::HashMap::new()));
    if let Some(v)=cache.lock().unwrap_or_else(|e|e.into_inner()).get(root).cloned(){return v}
    let text=std::fs::read(omsi_cfg::resolve_path(root,"Weather/ICAO.txt")).map(|b|omsi_cfg::codepage::decode(&b)).unwrap_or_default();
    let mut v:Vec<(String,String)>=text.lines().filter_map(|l|l.split_once(" - ").map(|(c,n)|(c.trim().to_ascii_uppercase(),format!("{} - {}",c.trim(),n.trim()))))
        .filter(|(c,_)|c.len()==4&&c.chars().all(|x|x.is_ascii_alphabetic())).collect();
    if !v.iter().any(|a|a.0=="EDDB"){v.push(("EDDB".into(),"EDDB - Berlin Brandenburg".into()))}
    v.sort_by(|a,b|a.0.cmp(&b.0)); v.dedup_by(|a,b|a.0==b.0);
    cache.lock().unwrap_or_else(|e|e.into_inner()).insert(root.to_path_buf(),v.clone()); v
}

/// The weather of an airport's METAR report (aviationweather.gov), or a clear day when it
/// cannot be had (no network, an unknown station).
pub(crate) fn fetch_metar(icao: &str) -> omsi_content::weather::Weather {
    match try_metar(icao) {
        Some(w) => w,
        None => {
            log::warn!("current weather at {icao}: no METAR report could be had; a clear day instead");
            omsi_content::weather::from_metar(icao, "CAVOK")
        }
    }
}

/// The weather of an airport's METAR report, None when it cannot be had.
pub(crate) fn try_metar(icao: &str) -> Option<omsi_content::weather::Weather> {
    // (Tegel, OMSI's Berlin default, closed in 2020: Berlin's airport now reports)
    let icao = match icao.to_ascii_uppercase().as_str() {
        "EDDT" | "EDDI" | "" => "EDDB".to_string(),
        other => other.to_string(),
    };
    let url = format!("https://aviationweather.gov/api/data/metar?ids={icao}&format=raw");
    let text = ureq::get(&url)
        .timeout(std::time::Duration::from_secs(6))
        .call()
        .ok()
        .and_then(|r| r.into_string().ok())
        .map(|t| t.lines().next().unwrap_or("").trim().to_string())
        .filter(|t| !t.is_empty());
    let t = text?;
    log::info!("current weather at {icao}: {t}");
    let mut w = omsi_content::weather::from_metar(&icao, &t);
    w.path = std::path::PathBuf::from(format!("metar:{icao}"));
    Some(w)
}

/// The start of a weather text that carries a METAR report's values (not a file, not a
/// download): what a host or server with the METAR sync tells the players, who make the
/// weather from it themselves and need no sync of their own.
pub(crate) const REPORT: &str = "metar-report:";

/// `w` (made from a METAR report) as the text the session tells the players: the station and
/// the report up to its forecast, which `from_metar` does not read either.
pub(crate) fn report_wire(w: &omsi_content::weather::Weather) -> Option<String> {
    let path = w.path.to_string_lossy();
    let icao = path.strip_prefix("metar:")?;
    let mut raw: Vec<&str> = Vec::new();
    for t in w.description.split_whitespace() {
        if matches!(t.trim_end_matches('='), "TEMPO" | "BECMG" | "NOSIG" | "RMK" | "PROB30" | "PROB40") {
            break;
        }
        raw.push(t);
    }
    let raw = raw.join(" ");
    if icao.is_empty() || raw.is_empty() {
        return None;
    }
    // (the network's weather field holds 260 characters)
    Some(format!("{REPORT}{icao} {raw}").chars().take(250).collect())
}

/// The weather of a text made by `report_wire`.
pub(crate) fn from_report(s: &str) -> Option<omsi_content::weather::Weather> {
    let (icao, raw) = s.trim().strip_prefix(REPORT)?.split_once(' ')?;
    if !(3..=5).contains(&icao.len()) || !icao.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    let mut w = omsi_content::weather::from_metar(icao, raw);
    w.path = std::path::PathBuf::from(format!("metar:{icao}"));
    Some(w)
}
