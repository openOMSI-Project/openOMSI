//! DDS decoder covering what D3DX accepts in the original: uncompressed formats with
//! arbitrary channel masks (A8R8G8B8, X8R8G8B8, R5G6B5, A1R5G5B5, A4R4G4B4, R8G8B8, A8, L8,
//! A8L8) and the BC1/BC2/BC3 (DXT1/3/5) block formats. Only the top mip level is used.

use crate::Image;

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// Decode a DDS file. Returns None when the header is not a DDS one.
pub fn decode(bytes: &[u8]) -> Result<Image, String> {
    if bytes.len() < 128 || &bytes[..4] != b"DDS " {
        return Err("not a DDS file".into());
    }
    let height = u32_at(bytes, 12) as usize;
    let width = u32_at(bytes, 16) as usize;
    let pf_flags = u32_at(bytes, 80);
    let fourcc = &bytes[84..88];
    let bpp = u32_at(bytes, 88) as usize;
    let masks = [u32_at(bytes, 92), u32_at(bytes, 96), u32_at(bytes, 100), u32_at(bytes, 104)];
    let mut data_start = 128;
    if fourcc == b"DX10" {
        data_start += 20;
    }
    let data = &bytes[data_start.min(bytes.len())..];
    if width == 0 || height == 0 {
        return Err("empty image".into());
    }
    // a cube map (six faces of the same size, one after the other): OMSI's reflections are
    // sphere maps, so the faces are turned into one (the first face alone had been taken)
    let caps2 = u32_at(bytes, 112);
    if caps2 & 0x200 != 0 && caps2 & 0xFC00 == 0xFC00 && data.len() >= 6 {
        let face_len = data.len() / 6;
        let mut faces = Vec::with_capacity(6);
        for k in 0..6 {
            let mut one = bytes[..data_start].to_vec();
            one[112..116].copy_from_slice(&0u32.to_le_bytes());
            one.extend_from_slice(&data[k * face_len..(k + 1) * face_len]);
            faces.push(decode(&one)?);
        }
        return Ok(cube_to_sphere(&faces, width.clamp(64, 512)));
    }
    // (checked before the picture is allocated: a damaged header asked for gigabytes)
    if width > crate::MAX_DIMENSION || height > crate::MAX_DIMENSION {
        return Err(format!("{width}x{height} is larger than any texture"));
    }
    let mut rgba = vec![0u8; width * height * 4];
    if pf_flags & 0x4 != 0 {
        // D3DFORMAT numbers in the fourcc: the wide RGBA formats D3DX writes for float or
        // 16-bit pictures (a Citaro repaint's `lublins.dds` is A32B32G32R32F)
        if let Some(img) = wide_rgba(u32_at(bytes, 84), data, width, height) {
            return img;
        }
        // the DX10 header names a DXGI format: the block formats as their DXT names, the
        // plain ones read as they lie (D3DX 9 itself knows none of them; newer tools write
        // them for mods)
        let dxgi = (fourcc == b"DX10" && bytes.len() >= 148).then(|| u32_at(bytes, 128));
        if let Some(img) = dxgi.and_then(|f| dxgi_plain(f, data, width, height)) {
            return img;
        }
        // compressed
        let (block_bytes, kind) = match (fourcc, dxgi) {
            (b"DXT1", _) | (_, Some(70..=72)) => (8, 1),
            (b"DXT2" | b"DXT3", _) | (_, Some(73..=75)) => (16, 3),
            (b"DXT4" | b"DXT5", _) | (_, Some(76..=78)) => (16, 5),
            (_, Some(f)) => return Err(format!("unsupported DXGI format {f}")),
            (other, None) => return Err(format!("unsupported fourcc {:?}", String::from_utf8_lossy(other))),
        };
        // DXT2 and DXT4 hold colour premultiplied by alpha
        let premultiplied = matches!(fourcc, b"DXT2" | b"DXT4");
        let bw = (width + 3) / 4;
        let bh = (height + 3) / 4;
        if data.len() < bw * bh * block_bytes {
            return Err("truncated DXT data".into());
        }
        for by in 0..bh {
            for bx in 0..bw {
                let block = &data[(by * bw + bx) * block_bytes..][..block_bytes];
                let px = decode_block(block, kind);
                for y in 0..4 {
                    for x in 0..4 {
                        let (ix, iy) = (bx * 4 + x, by * 4 + y);
                        if ix < width && iy < height {
                            let o = (iy * width + ix) * 4;
                            rgba[o..o + 4].copy_from_slice(&px[y * 4 + x]);
                        }
                    }
                }
            }
        }
        if premultiplied {
            for p in rgba.chunks_exact_mut(4) {
                if p[3] > 0 && p[3] < 255 {
                    for c in 0..3 {
                        p[c] = ((p[c] as u32 * 255 + p[3] as u32 / 2) / p[3] as u32).min(255) as u8;
                    }
                }
            }
        }
        let has_alpha = kind != 1 || rgba.chunks_exact(4).any(|p| p[3] < 255);
        return Ok(Image { width: width as u32, height: height as u32, rgba, has_alpha });
    }
    // uncompressed: masks describe the channels (luminance: r mask only, flag 0x20000)
    let luminance = pf_flags & 0x20000 != 0;
    let alpha_only = pf_flags & 0x2 != 0 && pf_flags & 0x40 == 0 && !luminance;
    let bytes_pp = (bpp.max(8) + 7) / 8;
    let row = width * bytes_pp;
    if data.len() < row * height {
        return Err("truncated pixel data".into());
    }
    let extract = |v: u32, mask: u32| -> u8 {
        if mask == 0 {
            return 0;
        }
        let shift = mask.trailing_zeros();
        let bits = (mask >> shift).count_ones();
        let x = (v & mask) >> shift;
        // scale to 8 bits
        if bits >= 8 {
            (x >> (bits - 8)) as u8
        } else {
            ((x * 255) / ((1u32 << bits) - 1)) as u8
        }
    };
    let has_alpha = masks[3] != 0 || alpha_only;
    for y in 0..height {
        for x in 0..width {
            let o = y * row + x * bytes_pp;
            let mut v = 0u32;
            for k in 0..bytes_pp.min(4) {
                v |= (data[o + k] as u32) << (8 * k);
            }
            let p = &mut rgba[(y * width + x) * 4..][..4];
            if alpha_only {
                p.copy_from_slice(&[255, 255, 255, extract(v, masks[3])]);
            } else if luminance {
                let l = extract(v, masks[0]);
                let a = if masks[3] != 0 { extract(v, masks[3]) } else { 255 };
                p.copy_from_slice(&[l, l, l, a]);
            } else {
                let a = if masks[3] != 0 { extract(v, masks[3]) } else { 255 };
                p.copy_from_slice(&[extract(v, masks[0]), extract(v, masks[1]), extract(v, masks[2]), a]);
            }
        }
    }
    Ok(Image { width: width as u32, height: height as u32, rgba, has_alpha })
}

/// The uncompressed DXGI formats of a DX10 header: R8G8B8A8 (28, 29 sRGB), B8G8R8A8 (87,
/// 91), B8G8R8X8 (88, 93), R16G16B16A16 float (10) and unorm (11), R32G32B32A32 float (2),
/// R8 (61), A8 (65).
fn dxgi_plain(format: u32, data: &[u8], width: usize, height: usize) -> Option<Result<Image, String>> {
    let wide = match format {
        10 => Some(113),
        11 => Some(36),
        2 => Some(116),
        _ => None,
    };
    if let Some(d3d) = wide {
        return wide_rgba(d3d, data, width, height);
    }
    let bytes_pp = match format {
        28 | 29 | 87 | 88 | 91 | 93 => 4,
        61 | 65 => 1,
        _ => return None,
    };
    if data.len() < width * height * bytes_pp {
        return Some(Err("truncated pixel data".into()));
    }
    let mut rgba = vec![0u8; width * height * 4];
    for (i, px) in data.chunks_exact(bytes_pp).take(width * height).enumerate() {
        let p = &mut rgba[i * 4..i * 4 + 4];
        match format {
            28 | 29 => p.copy_from_slice(px),
            87 | 91 => p.copy_from_slice(&[px[2], px[1], px[0], px[3]]),
            88 | 93 => p.copy_from_slice(&[px[2], px[1], px[0], 255]),
            61 => p.copy_from_slice(&[px[0], px[0], px[0], 255]),
            _ => p.copy_from_slice(&[255, 255, 255, px[0]]),
        }
    }
    let has_alpha = rgba.chunks_exact(4).any(|p| p[3] < 255);
    Some(Ok(Image { width: width as u32, height: height as u32, rgba, has_alpha }))
}

/// `D3DFMT_A16B16G16R16` (36), `D3DFMT_A16B16G16R16F` (113) and `D3DFMT_A32B32G32R32F`
/// (116): four channels in R, G, B, A order, tone-clamped to 0..1.
fn wide_rgba(format: u32, data: &[u8], width: usize, height: usize) -> Option<Result<Image, String>> {
    let (bytes_pp, read): (usize, fn(&[u8]) -> f32) = match format {
        36 => (8, |b| u16::from_le_bytes([b[0], b[1]]) as f32 / 65535.0),
        113 => (8, |b| half_to_f32(u16::from_le_bytes([b[0], b[1]]))),
        116 => (16, |b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        _ => return None,
    };
    let step = bytes_pp / 4;
    if data.len() < width * height * bytes_pp {
        return Some(Err("truncated pixel data".into()));
    }
    let mut rgba = vec![0u8; width * height * 4];
    for (i, px) in data.chunks_exact(bytes_pp).take(width * height).enumerate() {
        for c in 0..4 {
            let v = read(&px[c * step..]);
            rgba[i * 4 + c] = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        }
    }
    let has_alpha = rgba.chunks_exact(4).any(|p| p[3] < 255);
    Some(Ok(Image { width: width as u32, height: height as u32, rgba, has_alpha }))
}

fn half_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = ((h >> 10) & 0x1F) as i32;
    let frac = (h & 0x3FF) as f32;
    sign * match exp {
        0 => frac * 2f32.powi(-24),
        31 => f32::INFINITY,
        e => (1.0 + frac / 1024.0) * 2f32.powi(e - 15),
    }
}

fn rgb565(v: u16) -> [u8; 4] {
    let r = ((v >> 11) & 31) as u32;
    let g = ((v >> 5) & 63) as u32;
    let b = (v & 31) as u32;
    [((r * 255) / 31) as u8, ((g * 255) / 63) as u8, ((b * 255) / 31) as u8, 255]
}

pub(crate) fn decode_block(block: &[u8], kind: u8) -> [[u8; 4]; 16] {
    let mut out = [[0u8; 4]; 16];
    let color = if kind == 1 { block } else { &block[8..] };
    let c0 = u16::from_le_bytes([color[0], color[1]]);
    let c1 = u16::from_le_bytes([color[2], color[3]]);
    let p0 = rgb565(c0);
    let p1 = rgb565(c1);
    let mix = |a: [u8; 4], b: [u8; 4], wa: u32, wb: u32| -> [u8; 4] {
        let f = |i: usize| ((a[i] as u32 * wa + b[i] as u32 * wb) / (wa + wb)) as u8;
        [f(0), f(1), f(2), 255]
    };
    let palette = if kind == 1 && c0 <= c1 {
        [p0, p1, mix(p0, p1, 1, 1), [0, 0, 0, 0]]
    } else {
        [p0, p1, mix(p0, p1, 2, 1), mix(p0, p1, 1, 2)]
    };
    let bits = u32::from_le_bytes([color[4], color[5], color[6], color[7]]);
    for i in 0..16 {
        out[i] = palette[((bits >> (2 * i)) & 3) as usize];
    }
    match kind {
        3 => {
            let a = u64::from_le_bytes(block[..8].try_into().unwrap());
            for i in 0..16 {
                let v = ((a >> (4 * i)) & 15) as u8;
                out[i][3] = v * 17;
            }
        }
        5 => {
            let a0 = block[0] as u32;
            let a1 = block[1] as u32;
            let mut table = [0u8; 8];
            table[0] = a0 as u8;
            table[1] = a1 as u8;
            if a0 > a1 {
                for i in 1..7 {
                    table[i + 1] = (((7 - i as u32) * a0 + i as u32 * a1) / 7) as u8;
                }
            } else {
                for i in 1..5 {
                    table[i + 1] = (((5 - i as u32) * a0 + i as u32 * a1) / 5) as u8;
                }
                table[6] = 0;
                table[7] = 255;
            }
            let mut bits = 0u64;
            for i in 0..6 {
                bits |= (block[2 + i] as u64) << (8 * i);
            }
            for i in 0..16 {
                out[i][3] = table[((bits >> (3 * i)) & 7) as usize];
            }
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod dx10_tests {
    #[test]
    fn a_dx10_bgra_picture_is_read() {
        let mut b = vec![0u8; 148];
        b[..4].copy_from_slice(b"DDS ");
        b[12..16].copy_from_slice(&1u32.to_le_bytes()); // height
        b[16..20].copy_from_slice(&2u32.to_le_bytes()); // width
        b[80..84].copy_from_slice(&4u32.to_le_bytes()); // fourcc flag
        b[84..88].copy_from_slice(b"DX10");
        b[128..132].copy_from_slice(&87u32.to_le_bytes()); // B8G8R8A8_UNORM
        b.extend_from_slice(&[10, 20, 30, 40, 1, 2, 3, 255]);
        let img = super::decode(&b).unwrap();
        assert_eq!(&img.rgba[..8], &[30, 20, 10, 40, 3, 2, 1, 255]);
        assert!(img.has_alpha);
    }
}

/// A sphere map (as Direct3D's `D3DTSS_TCI_SPHEREMAP` reads it) of `size` pixels from the six
/// faces of a cube map (+X, −X, +Y, −Y, +Z, −Z, as DDS stores them): each pixel is the
/// direction a mirror sphere reflects towards the viewer there.
fn cube_to_sphere(faces: &[Image], size: usize) -> Image {
    let mut rgba = vec![0u8; size * size * 4];
    let mut has_alpha = false;
    for y in 0..size {
        for x in 0..size {
            let nx = (x as f32 + 0.5) / size as f32 * 2.0 - 1.0;
            let ny = 1.0 - (y as f32 + 0.5) / size as f32 * 2.0;
            let r2 = nx * nx + ny * ny;
            if r2 > 1.0 {
                continue;
            }
            // the sphere's normal, and the view along −z reflected about it
            let nz = (1.0 - r2).sqrt();
            let d = [2.0 * nz * nx, 2.0 * nz * ny, 2.0 * nz * nz - 1.0];
            let (ax, ay, az) = (d[0].abs(), d[1].abs(), d[2].abs());
            let (face, u, v) = if ax >= ay && ax >= az {
                if d[0] > 0.0 { (0, -d[2] / ax, -d[1] / ax) } else { (1, d[2] / ax, -d[1] / ax) }
            } else if ay >= az {
                if d[1] > 0.0 { (2, d[0] / ay, d[2] / ay) } else { (3, d[0] / ay, -d[2] / ay) }
            } else if d[2] > 0.0 {
                (4, d[0] / az, -d[1] / az)
            } else {
                (5, -d[0] / az, -d[1] / az)
            };
            let f = &faces[face];
            let fx = (((u + 1.0) * 0.5 * f.width as f32) as u32).min(f.width.saturating_sub(1));
            let fy = (((v + 1.0) * 0.5 * f.height as f32) as u32).min(f.height.saturating_sub(1));
            let si = ((fy * f.width + fx) * 4) as usize;
            let di = (y * size + x) * 4;
            if let Some(px) = f.rgba.get(si..si + 4) {
                rgba[di..di + 4].copy_from_slice(px);
                has_alpha |= px[3] < 255;
            }
        }
    }
    Image { width: size as u32, height: size as u32, rgba, has_alpha }
}

#[cfg(test)]
mod cube_tests {
    #[test]
    fn a_cube_map_becomes_a_sphere_map() {
        // six 4x4 uncompressed faces, each one colour
        let mut b = vec![0u8; 128];
        b[..4].copy_from_slice(b"DDS ");
        b[12..16].copy_from_slice(&4u32.to_le_bytes());
        b[16..20].copy_from_slice(&4u32.to_le_bytes());
        b[80..84].copy_from_slice(&0x41u32.to_le_bytes()); // RGB + alpha
        b[88..92].copy_from_slice(&32u32.to_le_bytes());
        b[92..96].copy_from_slice(&0x00FF_0000u32.to_le_bytes());
        b[96..100].copy_from_slice(&0x0000_FF00u32.to_le_bytes());
        b[100..104].copy_from_slice(&0x0000_00FFu32.to_le_bytes());
        b[104..108].copy_from_slice(&0xFF00_0000u32.to_le_bytes());
        b[112..116].copy_from_slice(&0xFE00u32.to_le_bytes());
        for k in 0..6u8 {
            for _ in 0..16 {
                b.extend_from_slice(&[k * 40, 0, 0, 255]); // B G R A
            }
        }
        let img = super::decode(&b).unwrap();
        assert_eq!(img.width, 64);
        // the middle looks straight back at the viewer: the +Z face (index 4)
        let c = ((32 * 64 + 32) * 4) as usize;
        assert_eq!(img.rgba[c + 2], 4 * 40);
    }
}

// --- writing ----------------------------------------------------------------------------------
//
// A DirectX 9 DDS as D3DX writes one and OMSI 2 reads it: the 128-byte header (no DX10 one), a
// DXT1 or DXT5 four-character code, and the whole mip chain down to 1 x 1, each level's blocks
// one after the other.

const DDSD_CAPS: u32 = 0x1;
const DDSD_HEIGHT: u32 = 0x2;
const DDSD_WIDTH: u32 = 0x4;
const DDSD_PIXELFORMAT: u32 = 0x1000;
const DDSD_MIPMAPCOUNT: u32 = 0x20000;
const DDSD_LINEARSIZE: u32 = 0x80000;
const DDPF_FOURCC: u32 = 0x4;
const DDSCAPS_COMPLEX: u32 = 0x8;
const DDSCAPS_TEXTURE: u32 = 0x1000;
const DDSCAPS_MIPMAP: u32 = 0x400000;

/// Bytes of one level of `w` x `h` pixels in a block format.
pub fn level_bytes(w: u32, h: u32, format: crate::bc::Bc) -> usize {
    w.max(1).div_ceil(4) as usize * h.max(1).div_ceil(4) as usize * format.block_bytes()
}

/// The 128 bytes in front of the blocks: `levels` mip levels of a `w` x `h` picture.
pub fn header(w: u32, h: u32, levels: u32, format: crate::bc::Bc) -> [u8; 128] {
    let mut k = [0u8; 128];
    let mut put = |o: usize, v: u32| k[o..o + 4].copy_from_slice(&v.to_le_bytes());
    put(4, 124);
    put(8, DDSD_CAPS | DDSD_HEIGHT | DDSD_WIDTH | DDSD_PIXELFORMAT | DDSD_MIPMAPCOUNT | DDSD_LINEARSIZE);
    put(12, h);
    put(16, w);
    put(20, level_bytes(w, h, format) as u32);
    // (depth 1 for a flat texture, as D3DX and the NVIDIA tools write it)
    put(24, 1);
    put(28, levels);
    put(76, 32);
    put(80, DDPF_FOURCC);
    put(108, DDSCAPS_TEXTURE | if levels > 1 { DDSCAPS_COMPLEX | DDSCAPS_MIPMAP } else { 0 });
    k[..4].copy_from_slice(b"DDS ");
    k[84..88].copy_from_slice(match format {
        crate::bc::Bc::Bc1 { .. } => b"DXT1",
        crate::bc::Bc::Bc2 => b"DXT3",
        crate::bc::Bc::Bc3 => b"DXT5",
    });
    k
}

/// An sRGB RGBA picture as a DDS file with its whole mip chain (each level a box in linear
/// light of the one above, see `bc::downsample`), encoded in `format`.
pub fn encode(rgba: &[u8], w: u32, h: u32, format: crate::bc::Bc) -> Vec<u8> {
    let levels = crate::gpu::mip_count(w, h);
    let mut out = header(w, h, levels, format).to_vec();
    let (mut pic, mut lw, mut lh) = (std::borrow::Cow::Borrowed(rgba), w.max(1), h.max(1));
    for l in 0..levels {
        let (blocks, _, _) = crate::bc::encode(&pic, lw, lh, format);
        debug_assert_eq!(blocks.len(), level_bytes(lw, lh, format));
        out.extend_from_slice(&blocks);
        if l + 1 < levels {
            let (next, nw, nh) = crate::bc::downsample(&pic, lw, lh);
            (pic, lw, lh) = (std::borrow::Cow::Owned(next), nw, nh);
        }
    }
    out
}

#[cfg(test)]
mod writer_tests {
    use crate::bc::Bc;

    fn picture(w: u32, h: u32, alpha: bool) -> Vec<u8> {
        (0..w * h).flat_map(|i| {
            let (x, y) = (i % w, i / w);
            [(x * 255 / w.max(2)) as u8, (y * 255 / h.max(2)) as u8, 90, if alpha && x < w / 2 { 40 } else { 255 }]
        }).collect()
    }

    #[test]
    fn a_written_dds_has_the_header_omsi_reads_and_the_whole_chain() {
        let (w, h) = (64u32, 32u32);
        let bytes = super::encode(&picture(w, h, false), w, h, Bc::Bc1 { punch: false });
        assert_eq!(&bytes[..4], b"DDS ");
        let at = |o: usize| u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
        assert_eq!((at(4), at(12), at(16), at(28)), (124, h, w, 7), "size, height, width, levels down to 1x1");
        assert_eq!(&bytes[84..88], b"DXT1");
        assert_eq!(at(20) as usize, 16 * 8 * 8, "the top level's bytes");
        assert_eq!(at(108), 0x1000 | 0x8 | 0x400000, "caps: texture, complex, mip-mapped");
        assert_eq!(at(24), 1, "depth 1, as OMSI's own textures");
        // the levels exactly: 64x32, 32x16, ... 1x1
        let mut n = 128;
        let (mut lw, mut lh) = (w, h);
        for _ in 0..7 {
            n += super::level_bytes(lw, lh, Bc::Bc1 { punch: false });
            (lw, lh) = ((lw / 2).max(1), (lh / 2).max(1));
        }
        assert_eq!(bytes.len(), n);
        // and it reads back as the picture
        let img = super::decode(&bytes).unwrap();
        assert_eq!((img.width, img.height), (w, h));
        let src = picture(w, h, false);
        let err: f64 = img.rgba.iter().zip(&src).map(|(a, b)| (*a as f64 - *b as f64).abs()).sum::<f64>() / src.len() as f64;
        assert!(err < 6.0, "mean error {err}");
    }

    #[test]
    fn a_picture_with_alpha_goes_into_dxt5_and_keeps_it() {
        let (w, h) = (16u32, 8u32);
        let bytes = super::encode(&picture(w, h, true), w, h, Bc::Bc3);
        assert_eq!(&bytes[84..88], b"DXT5");
        let img = super::decode(&bytes).unwrap();
        assert!((img.rgba[3] as i32 - 40).abs() <= 2 && img.rgba[(w as usize - 1) * 4 + 3] == 255);
    }
}
