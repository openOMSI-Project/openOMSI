//! Colours in the livery studio: sRGB and linear light, HSV for the picker, CIELAB for the
//! colour zones (ΔE as Omsi-Hub measures it).

/// sRGB byte → linear light.
pub fn to_linear(c: u8) -> f32 {
    LIN.get_or_init(|| std::array::from_fn(|i| srgb_to_linear(i as f32 / 255.0)))[c as usize]
}

static LIN: std::sync::OnceLock<[f32; 256]> = std::sync::OnceLock::new();
static BACK: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();

/// Linear light (0..1) → sRGB byte, through a table of 4096 steps.
pub fn to_srgb(v: f32) -> u8 {
    let t = BACK.get_or_init(|| (0..=4096).map(|i| (linear_to_srgb(i as f32 / 4096.0) * 255.0 + 0.5).clamp(0.0, 255.0) as u8).collect());
    t[((v.clamp(0.0, 1.0) * 4096.0 + 0.5) as usize).min(4096)]
}

pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

pub fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// A colour's linear light from `#rrggbb` (white when it cannot be read).
pub fn linear_of(hex: &str) -> [f32; 3] {
    let c = super::model::parse_hex(hex).unwrap_or([255, 255, 255]);
    [to_linear(c[0]), to_linear(c[1]), to_linear(c[2])]
}

/// sRGB bytes → CIELAB (D65).
pub fn lab(c: [u8; 3]) -> [f32; 3] {
    lab_of_linear([to_linear(c[0]), to_linear(c[1]), to_linear(c[2])])
}

pub fn lab_of_linear(l: [f32; 3]) -> [f32; 3] {
    let x = (0.4124 * l[0] + 0.3576 * l[1] + 0.1805 * l[2]) / 0.95047;
    let y = 0.2126 * l[0] + 0.7152 * l[1] + 0.0722 * l[2];
    let z = (0.0193 * l[0] + 0.1192 * l[1] + 0.9505 * l[2]) / 1.08883;
    let f = |t: f32| if t > 0.008856 { t.cbrt() } else { 7.787 * t + 16.0 / 116.0 };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// CIELAB → sRGB bytes.
#[cfg(test)]
pub fn rgb_of_lab(lab: [f32; 3]) -> [u8; 3] {
    let fy = (lab[0] + 16.0) / 116.0;
    let fx = fy + lab[1] / 500.0;
    let fz = fy - lab[2] / 200.0;
    let g = |t: f32| if t.powi(3) > 0.008856 { t.powi(3) } else { (t - 16.0 / 116.0) / 7.787 };
    let (x, y, z) = (g(fx) * 0.95047, g(fy), g(fz) * 1.08883);
    let r = 3.2406 * x - 1.5372 * y - 0.4986 * z;
    let gg = -0.9689 * x + 1.8758 * y + 0.0415 * z;
    let b = 0.0557 * x - 0.2040 * y + 1.0570 * z;
    [to_srgb(r), to_srgb(gg), to_srgb(b)]
}

/// ΔE (CIE76) between two CIELAB colours.
pub fn delta_e(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// sRGB (0..1 each) → hue (degrees), saturation and value (0..1).
pub fn hsv(rgb: [f32; 3]) -> [f32; 3] {
    let [r, g, b] = rgb;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d <= 1e-6 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    [h, if max <= 1e-6 { 0.0 } else { d / max }, max]
}

pub fn rgb_of_hsv(hsv: [f32; 3]) -> [f32; 3] {
    let [h, s, v] = hsv;
    let c = v * s;
    let hp = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [r + m, g + m, b + m]
}

pub fn bytes(rgb: [f32; 3]) -> [u8; 3] {
    rgb.map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)
}

pub fn unit(c: [u8; 3]) -> [f32; 3] {
    c.map(|v| v as f32 / 255.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_goes_there_and_back() {
        for c in [[255u8, 0, 0], [29, 63, 143], [255, 255, 255], [0, 0, 0], [12, 200, 90], [200, 180, 7]] {
            let back = bytes(rgb_of_hsv(hsv(unit(c))));
            for k in 0..3 {
                assert!((back[k] as i32 - c[k] as i32).abs() <= 1, "{c:?} → {back:?}");
            }
        }
        let [h, s, v] = hsv([0.0, 0.0, 1.0]);
        assert!((h - 240.0).abs() < 1e-4 && s == 1.0 && v == 1.0);
    }

    #[test]
    fn lab_goes_there_and_back_and_measures_as_cie() {
        let w = lab([255, 255, 255]);
        assert!((w[0] - 100.0).abs() < 0.1 && w[1].abs() < 0.5 && w[2].abs() < 0.5, "{w:?}");
        let k = lab([0, 0, 0]);
        assert!(k[0].abs() < 0.1);
        let red = lab([255, 0, 0]);
        assert!((red[0] - 53.2).abs() < 0.5 && (red[1] - 80.1).abs() < 1.0 && (red[2] - 67.2).abs() < 1.0, "{red:?}");
        for c in [[29u8, 63, 143], [250, 240, 10], [128, 128, 128]] {
            let back = rgb_of_lab(lab(c));
            for k in 0..3 {
                assert!((back[k] as i32 - c[k] as i32).abs() <= 1, "{c:?} → {back:?}");
            }
        }
        assert!(delta_e(lab([128, 128, 128]), lab([130, 128, 128])) < 1.5);
    }

    #[test]
    fn linear_light_goes_there_and_back() {
        for v in 0..=255u8 {
            assert_eq!(to_srgb(to_linear(v)), v);
        }
        assert!((linear_of("#808080")[0] - 0.2159).abs() < 1e-3);
    }
}
