//! Authored heights become shading normals at load time, without guessing relief from a
//! colour photograph. Physical dimensions keep the slope independent of image resolution.

use crate::Image;
use std::path::Path;

#[derive(Debug, PartialEq)]
struct Dimensions {
    height: f32,
    width: f32,
    depth: f32,
}

impl Dimensions {
    fn parse(file: &omsi_cfg::CfgFile) -> Result<Self, String> {
        let mut reader = file.reader();
        let (mut height, mut size) = (None, None);
        let number = |text: &str| -> Result<f32, String> {
            text.trim()
                .parse::<f32>()
                .ok()
                .filter(|x| x.is_finite())
                .ok_or_else(|| "physical dimensions must be finite numbers in metres".into())
        };
        while let Some(key) = reader.next_keyword() {
            match key.as_str() {
                "height_scale" if height.is_none() => height = Some(number(reader.line())?),
                "texture_size" if size.is_none() => {
                    size = Some((number(reader.line())?, number(reader.line())?))
                }
                _ => {
                    return Err(format!(
                        "unknown or repeated [{key}] in height configuration"
                    ));
                }
            }
        }
        let height = height.ok_or("missing [height_scale]")?;
        let (width, depth) = size.ok_or("missing [texture_size]")?;
        if height <= 0.0 || height > 1.0 {
            return Err("[height_scale] must be greater than 0 and at most 1 metre".into());
        }
        if !(0.01..=1000.0).contains(&width) || !(0.01..=1000.0).contains(&depth) {
            return Err("[texture_size] dimensions must be between 0.01 and 1000 metres".into());
        }
        Ok(Self {
            height,
            width,
            depth,
        })
    }
}

pub(super) fn load(path: &Path, config: &Path) -> Result<Image, String> {
    let file = omsi_cfg::CfgFile::read(config).map_err(|e| e.to_string())?;
    let dimensions = Dimensions::parse(&file)?;
    let image = crate::decode_file(path).map_err(|e| e.to_string())?;
    if !super::is_grey(&image) {
        return Err("expected a linear grayscale height map".into());
    }
    Ok(normals(&capped_linear(image, super::MAX_SIDE), &dimensions))
}

fn capped_linear(image: Image, max: u32) -> Image {
    assert!(max > 0);
    let (mut width, mut height) = (image.width, image.height);
    while width > max || height > max {
        width = (width / 2).max(1);
        height = (height / 2).max(1);
    }
    if (width, height) == (image.width, image.height) {
        return image;
    }
    let (sw, sh) = (image.width as usize, image.height as usize);
    let (w, h) = (width as usize, height as usize);
    let mut rgba = vec![255; w * h * 4];
    // Heights are linear data, not sRGB colours. Average each output texel's
    // footprint once, with integer area weights so odd source edges also contribute.
    // The source decoder bounds dimensions; u64 holds the full weighted byte sum.
    let area = sw as u64 * sh as u64;
    for y in 0..h {
        let (y0, y1) = (y * sh, (y + 1) * sh);
        for x in 0..w {
            let (x0, x1) = (x * sw, (x + 1) * sw);
            let mut sum = 0u64;
            for sy in y0 / h..y1.div_ceil(h) {
                let wy = (y1.min((sy + 1) * h) - y0.max(sy * h)) as u64;
                for sx in x0 / w..x1.div_ceil(w) {
                    let wx = (x1.min((sx + 1) * w) - x0.max(sx * w)) as u64;
                    sum += image.rgba[(sy * sw + sx) * 4] as u64 * wx * wy;
                }
            }
            let value = ((sum + area / 2) / area) as u8;
            rgba[(y * w + x) * 4..(y * w + x) * 4 + 3].fill(value);
        }
    }
    Image {
        width,
        height,
        rgba,
        has_alpha: false,
    }
}

fn normals(image: &Image, dimensions: &Dimensions) -> Image {
    let (width, height) = (image.width as usize, image.height as usize);
    let mut rgba = vec![255; width * height * 4];
    let sample = |x: usize, y: usize| image.rgba[(y * width + x) * 4] as f32 / 255.0;
    let scale_x = dimensions.height * image.width as f32 / (2.0 * dimensions.width);
    let scale_y = dimensions.height * image.height as f32 / (2.0 * dimensions.depth);
    for y in 0..height {
        for x in 0..width {
            // Road and terrain maps tile. Use neighbours across the repeat boundary rather
            // than flattening each tile's edge and leaving a seam in the lighting.
            let dx = (sample((x + 1) % width, y) - sample((x + width - 1) % width, y)) * scale_x;
            let dy = (sample(x, (y + 1) % height) - sample(x, (y + height - 1) % height)) * scale_y;
            let inv_len = (dx * dx + dy * dy + 1.0).sqrt().recip();
            for (c, value) in [-dx * inv_len, -dy * inv_len, inv_len]
                .into_iter()
                .enumerate()
            {
                rgba[(y * width + x) * 4 + c] =
                    ((value * 0.5 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    Image {
        width: image.width,
        height: image.height,
        rgba,
        has_alpha: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(text: &str) -> Result<Dimensions, String> {
        Dimensions::parse(&omsi_cfg::CfgFile::from_str("road.pbr.cfg", text))
    }

    fn grey(width: u32, height: u32, f: impl Fn(u32, u32) -> u8) -> Image {
        let mut rgba = Vec::new();
        for y in 0..height {
            for x in 0..width {
                let h = f(x, y);
                rgba.extend_from_slice(&[h, h, h, 255]);
            }
        }
        Image {
            width,
            height,
            rgba,
            has_alpha: false,
        }
    }

    fn normal_at(image: &Image, x: usize, y: usize) -> [f32; 3] {
        std::array::from_fn(|c| {
            image.rgba[(y * image.width as usize + x) * 4 + c] as f32 / 255.0 * 2.0 - 1.0
        })
    }

    #[test]
    fn capped_heights_average_linear_values_and_include_odd_edges() {
        // Every 2x2 footprint is half black and half white. Height is 127.5,
        // whereas treating these bytes as sRGB colours would produce about 188.
        for limit in [2, 1] {
            let image = capped_linear(grey(4, 4, |x, y| ((x + y) % 2 * 255) as u8), limit);
            assert_eq!((image.width, image.height), (limit, limit));
            assert!(
                image
                    .rgba
                    .chunks_exact(4)
                    .all(|p| p == [128, 128, 128, 255])
            );
        }
        // Five texels become two equal UV footprints: the centre texel contributes
        // half its area to each side, and the last texel must not be discarded.
        let image = capped_linear(grey(5, 1, |x, _| [0, 0, 100, 200, 200][x as usize]), 2);
        assert_eq!((image.width, image.height), (2, 1));
        assert_eq!(image.rgba, [20, 20, 20, 255, 180, 180, 180, 255]);
    }

    #[test]
    fn dimensions_are_explicit_finite_and_bounded() {
        assert_eq!(
            config("[height_scale]\n0.025\n[texture_size]\n2\n4\n").unwrap(),
            Dimensions {
                height: 0.025,
                width: 2.0,
                depth: 4.0
            }
        );
        for text in [
            "",
            "[height_scale]\n0.1",
            "[texture_size]\n1\n1",
            "[height_scale]\n0.1\n[texture_size]\n1\n1\n[height_scale]\n0.2",
        ] {
            assert!(config(text).is_err(), "{text}");
        }
        for height in ["0", "-0.01", "1.01", "NaN", "inf", "bad"] {
            assert!(config(&format!("[height_scale]\n{height}\n[texture_size]\n1\n1")).is_err());
        }
        for size in ["0", "-1", "0.009", "1001", "NaN", "inf", "bad"] {
            for (width, depth) in [(size, "1"), ("1", size)] {
                assert!(
                    config(&format!(
                        "[height_scale]\n0.1\n[texture_size]\n{width}\n{depth}"
                    ))
                    .is_err()
                );
            }
        }
    }

    #[test]
    fn constant_height_is_flat_and_plane_slope_has_physical_units() {
        let dimensions = Dimensions {
            height: 0.255,
            width: 0.8,
            depth: 1.6,
        };
        let flat = normals(&grey(8, 8, |_, _| 157), &dimensions);
        assert!(flat.rgba.chunks_exact(4).all(|p| p == [128, 128, 255, 255]));
        // Each U texel advances 0.1 m and rises 0.02 m; each V texel advances
        // 0.2 m and falls 0.01 m. The plane's analytic normal is (-0.2, 0.05, 1).
        let plane = normals(
            &grey(8, 8, |x, y| (100 + x * 20 - y * 10) as u8),
            &dimensions,
        );
        let actual = normal_at(&plane, 3, 3);
        let inv_len = (1.0_f32 + 0.2 * 0.2 + 0.05 * 0.05).sqrt().recip();
        let expected = [-0.2 * inv_len, 0.05 * inv_len, inv_len];
        for c in 0..3 {
            assert!(
                (actual[c] - expected[c]).abs() < 0.008,
                "{actual:?} != {expected:?}"
            );
        }
    }

    #[test]
    fn repeating_height_uses_the_neighbour_across_the_edge() {
        let dimensions = Dimensions {
            height: 1.0,
            width: 4.0,
            depth: 1.0,
        };
        let image = normals(
            &grey(4, 1, |x, _| [128, 255, 128, 0][x as usize]),
            &dimensions,
        );
        let at_zero = normal_at(&image, 0, 0);
        let at_two = normal_at(&image, 2, 0);
        // The periodic profile has slopes +0.5 and -0.5, including at the wrap edge.
        let x = 0.5 / 1.25_f32.sqrt();
        assert!((at_zero[0] + x).abs() < 0.008, "{at_zero:?}");
        assert!((at_two[0] - x).abs() < 0.008, "{at_two:?}");
        assert!((at_zero[2] - at_two[2]).abs() < 0.008);
    }
}
