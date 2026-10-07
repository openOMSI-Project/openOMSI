//! QR codes for pairing a phone or tablet: the page's address with the pairing code in it,
//! scanned with the camera, opens the page and pairs at once (nothing to type).
//!
//! A compact encoder of its own rather than a crate: one kind of content (a short address,
//! byte mode), versions 1 to 10 (up to 213 bytes at level M, an address needs some forty),
//! no new dependency to build. It follows ISO/IEC 18004 the way Nayuki's well-known
//! `qrcodegen` does (the same tables, the same order of drawing), and the tests hold its
//! output against codes made by `qrcode-generator` (the library Omsi-Hub drew its QR codes
//! with) bit for bit.

/// How much of the code may be damaged and still read: about 7 % (L) or 15 % (M).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ecc {
    L,
    M,
}

impl Ecc {
    fn index(self) -> usize {
        match self {
            Ecc::L => 0,
            Ecc::M => 1,
        }
    }

    /// The two bits the format information carries for it.
    fn format_bits(self) -> u32 {
        match self {
            Ecc::L => 1,
            Ecc::M => 0,
        }
    }
}

/// A QR code: its side in modules and the modules row by row (true: dark).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Qr {
    pub size: usize,
    pub modules: Vec<bool>,
}

impl Qr {
    /// The version (1-10), from the side.
    #[cfg(test)]
    pub(crate) fn version(&self) -> usize {
        (self.size - 17) / 4
    }

    #[cfg(test)]
    pub(crate) fn dark(&self, x: usize, y: usize) -> bool {
        x < self.size && y < self.size && self.modules[y * self.size + x]
    }

    /// The modules as a string of `0` and `1`, row by row (what the page draws from).
    pub(crate) fn bits(&self) -> String {
        self.modules.iter().map(|&d| if d { '1' } else { '0' }).collect()
    }
}

pub(crate) const MAX_VERSION: usize = 10;

/// Error correction codewords per block, by level (L, M) and version (index 0 unused).
const ECC_PER_BLOCK: [[usize; MAX_VERSION + 1]; 2] = [[0, 7, 10, 15, 20, 26, 18, 20, 24, 30, 18], [0, 10, 16, 26, 18, 24, 16, 18, 22, 22, 26]];
/// Error correction blocks, by level (L, M) and version.
const BLOCKS: [[usize; MAX_VERSION + 1]; 2] = [[0, 1, 1, 1, 1, 1, 2, 2, 2, 2, 4], [0, 1, 1, 1, 2, 2, 4, 4, 4, 5, 5]];

/// Modules that carry data or error correction in a code of `version`.
fn raw_modules(version: usize) -> usize {
    let mut n = (16 * version + 128) * version + 64;
    if version >= 2 {
        let align = version / 7 + 2;
        n -= (25 * align - 10) * align - 55;
        if version >= 7 {
            n -= 36;
        }
    }
    n
}

/// Data codewords a code of `version` at `ecc` holds.
fn data_codewords(version: usize, ecc: Ecc) -> usize {
    raw_modules(version) / 8 - ECC_PER_BLOCK[ecc.index()][version] * BLOCKS[ecc.index()][version]
}

/// Bytes of content that fit in byte mode.
pub(crate) fn capacity(version: usize, ecc: Ecc) -> usize {
    let count_bits = if version < 10 { 8 } else { 16 };
    (data_codewords(version, ecc) * 8 - 4 - count_bits) / 8
}

/// `text` as a QR code at `ecc`, in the smallest version it fits, with the mask the
/// standard's penalty rules choose. None when it does not fit in version 10.
pub(crate) fn encode(text: &str, ecc: Ecc) -> Option<Qr> {
    encode_with(text.as_bytes(), ecc, None)
}

/// The pairing address as a QR code: at level M (a phone's camera reads it off a screen at
/// an angle, in a reflection), at L when an address that long (IPv6) does not fit so.
pub(crate) fn encode_address(url: &str) -> Option<Qr> {
    encode(url, Ecc::M).or_else(|| encode(url, Ecc::L))
}

/// The same with a given mask (0-7), or the best one.
pub(crate) fn encode_with(data: &[u8], ecc: Ecc, mask: Option<u8>) -> Option<Qr> {
    let version = (1..=MAX_VERSION).find(|&v| data.len() <= capacity(v, ecc))?;
    let codewords = add_ecc(&data_bits(data, version, ecc), version, ecc);
    let mut q = Grid::new(version);
    q.function_patterns(ecc);
    q.place(&codewords);
    let mask = match mask {
        Some(m) => m.min(7),
        None => (0..8u8)
            .min_by_key(|&m| {
                let mut t = q.clone();
                t.apply_mask(m);
                t.format(ecc, m);
                t.penalty()
            })
            .unwrap_or(0),
    };
    q.apply_mask(mask);
    q.format(ecc, mask);
    Some(Qr { size: q.size, modules: q.modules })
}

/// The content as codewords before error correction: byte mode, the count, the bytes, the
/// terminator and the padding.
fn data_bits(data: &[u8], version: usize, ecc: Ecc) -> Vec<u8> {
    let mut bits: Vec<bool> = Vec::new();
    let push = |value: u32, n: u32, bits: &mut Vec<bool>| (0..n).rev().for_each(|i| bits.push((value >> i) & 1 == 1));
    push(0b0100, 4, &mut bits);
    push(data.len() as u32, if version < 10 { 8 } else { 16 }, &mut bits);
    for &b in data {
        push(b as u32, 8, &mut bits);
    }
    let capacity = data_codewords(version, ecc) * 8;
    let terminator = (capacity - bits.len()).min(4) as u32;
    push(0, terminator, &mut bits);
    let fill = (8 - bits.len() % 8) % 8;
    push(0, fill as u32, &mut bits);
    let mut out: Vec<u8> = bits.chunks(8).map(|c| c.iter().fold(0u8, |a, &b| (a << 1) | b as u8)).collect();
    let mut pad = [0xEC, 0x11].into_iter().cycle();
    while out.len() < capacity / 8 {
        out.push(pad.next().unwrap_or(0xEC));
    }
    out
}

/// The codewords split into blocks, each with its error correction, interleaved.
fn add_ecc(data: &[u8], version: usize, ecc: Ecc) -> Vec<u8> {
    let blocks = BLOCKS[ecc.index()][version];
    let ecc_len = ECC_PER_BLOCK[ecc.index()][version];
    let raw = raw_modules(version) / 8;
    let short = blocks - raw % blocks;
    let short_len = raw / blocks;
    let divisor = rs_divisor(ecc_len);
    let mut split: Vec<Vec<u8>> = Vec::with_capacity(blocks);
    let mut k = 0;
    for i in 0..blocks {
        let len = short_len - ecc_len + usize::from(i >= short);
        let mut block = data[k..k + len].to_vec();
        k += len;
        let rem = rs_remainder(&block, &divisor);
        if i < short {
            // (a place holder: the short blocks are one data codeword shorter)
            block.push(0);
        }
        block.extend(rem);
        split.push(block);
    }
    let mut out = Vec::with_capacity(raw);
    for i in 0..split[0].len() {
        for (j, b) in split.iter().enumerate() {
            if i != short_len - ecc_len || j >= short {
                out.push(b[i]);
            }
        }
    }
    out
}

/// Multiply in GF(2^8) modulo x^8 + x^4 + x^3 + x^2 + 1.
fn gf_mul(x: u8, y: u8) -> u8 {
    let mut z: u32 = 0;
    for i in (0..8).rev() {
        z = (z << 1) ^ ((z >> 7) * 0x11D);
        z ^= ((y as u32 >> i) & 1) * x as u32;
    }
    z as u8
}

/// The Reed-Solomon generator polynomial of `degree` (its coefficients, the highest
/// first, without the leading 1).
fn rs_divisor(degree: usize) -> Vec<u8> {
    let mut out = vec![0u8; degree];
    out[degree - 1] = 1;
    let mut root: u8 = 1;
    for _ in 0..degree {
        for j in 0..degree {
            out[j] = gf_mul(out[j], root);
            if j + 1 < degree {
                out[j] ^= out[j + 1];
            }
        }
        root = gf_mul(root, 0x02);
    }
    out
}

fn rs_remainder(data: &[u8], divisor: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; divisor.len()];
    for &b in data {
        let factor = b ^ out.remove(0);
        out.push(0);
        for (o, &d) in out.iter_mut().zip(divisor) {
            *o ^= gf_mul(d, factor);
        }
    }
    out
}

/// The code while it is drawn: the modules and which of them are the fixed patterns.
#[derive(Clone)]
struct Grid {
    version: usize,
    size: usize,
    modules: Vec<bool>,
    function: Vec<bool>,
}

impl Grid {
    fn new(version: usize) -> Grid {
        let size = version * 4 + 17;
        Grid { version, size, modules: vec![false; size * size], function: vec![false; size * size] }
    }

    fn set(&mut self, x: usize, y: usize, dark: bool) {
        let i = y * self.size + x;
        self.modules[i] = dark;
        self.function[i] = true;
    }

    fn get(&self, x: usize, y: usize) -> bool {
        self.modules[y * self.size + x]
    }

    /// Where the alignment patterns' centres are, across and down.
    fn alignment_positions(&self) -> Vec<usize> {
        if self.version == 1 {
            return Vec::new();
        }
        let n = self.version / 7 + 2;
        let step = (self.version * 8 + n * 3 + 5) / (n * 4 - 4) * 2;
        let mut out = vec![6];
        let mut at = self.size - 7;
        let mut rest = Vec::new();
        for _ in 1..n {
            rest.push(at);
            at -= step;
        }
        rest.reverse();
        out.extend(rest);
        out
    }

    /// The finder, timing and alignment patterns, the version and (a place holder for) the
    /// format information.
    fn function_patterns(&mut self, ecc: Ecc) {
        let size = self.size;
        for i in 0..size {
            self.set(6, i, i % 2 == 0);
            self.set(i, 6, i % 2 == 0);
        }
        for (cx, cy) in [(3, 3), (size - 4, 3), (3, size - 4)] {
            for dy in -4i32..=4 {
                for dx in -4i32..=4 {
                    let (x, y) = (cx as i32 + dx, cy as i32 + dy);
                    if (0..size as i32).contains(&x) && (0..size as i32).contains(&y) {
                        let d = dx.abs().max(dy.abs());
                        self.set(x as usize, y as usize, d != 2 && d != 4);
                    }
                }
            }
        }
        let at = self.alignment_positions();
        let n = at.len();
        for (i, &ax) in at.iter().enumerate() {
            for (j, &ay) in at.iter().enumerate() {
                // (not over the three finders)
                if (i == 0 && j == 0) || (i == 0 && j + 1 == n) || (i + 1 == n && j == 0) {
                    continue;
                }
                for dy in -2i32..=2 {
                    for dx in -2i32..=2 {
                        self.set((ax as i32 + dx) as usize, (ay as i32 + dy) as usize, dx.abs().max(dy.abs()) != 1);
                    }
                }
            }
        }
        self.format(ecc, 0);
        if self.version >= 7 {
            let mut rem = self.version as u32;
            for _ in 0..12 {
                rem = (rem << 1) ^ ((rem >> 11) * 0x1F25);
            }
            let bits = (self.version as u32) << 12 | rem;
            for i in 0..18 {
                let dark = (bits >> i) & 1 == 1;
                let (a, b) = (size - 11 + i % 3, i / 3);
                self.set(a, b, dark);
                self.set(b, a, dark);
            }
        }
    }

    /// The format information (level and mask) in both its places, and the dark module.
    fn format(&mut self, ecc: Ecc, mask: u8) {
        let data = ecc.format_bits() << 3 | mask as u32;
        let mut rem = data;
        for _ in 0..10 {
            rem = (rem << 1) ^ ((rem >> 9) * 0x537);
        }
        let bits = (data << 10 | rem) ^ 0x5412;
        let bit = |i: usize| (bits >> i) & 1 == 1;
        let size = self.size;
        for i in 0..6 {
            self.set(8, i, bit(i));
        }
        self.set(8, 7, bit(6));
        self.set(8, 8, bit(7));
        self.set(7, 8, bit(8));
        for i in 9..15 {
            self.set(14 - i, 8, bit(i));
        }
        for i in 0..8 {
            self.set(size - 1 - i, 8, bit(i));
        }
        for i in 8..15 {
            self.set(8, size - 15 + i, bit(i));
        }
        self.set(8, size - 8, true);
    }

    /// The codewords into the free modules, two columns at a time from the bottom right,
    /// up and down in turn.
    fn place(&mut self, data: &[u8]) {
        let size = self.size;
        let mut i = 0;
        let mut right = size as i32 - 1;
        while right >= 1 {
            if right == 6 {
                right = 5;
            }
            for vert in 0..size {
                for j in 0..2 {
                    let x = (right - j) as usize;
                    let upward = (right + 1) & 2 == 0;
                    let y = if upward { size - 1 - vert } else { vert };
                    if !self.function[y * size + x] && i < data.len() * 8 {
                        self.modules[y * size + x] = (data[i >> 3] >> (7 - (i & 7))) & 1 == 1;
                        i += 1;
                    }
                }
            }
            right -= 2;
        }
    }

    fn apply_mask(&mut self, mask: u8) {
        let size = self.size;
        for y in 0..size {
            for x in 0..size {
                let flip = match mask {
                    0 => (x + y) % 2 == 0,
                    1 => y % 2 == 0,
                    2 => x % 3 == 0,
                    3 => (x + y) % 3 == 0,
                    4 => (x / 3 + y / 2) % 2 == 0,
                    5 => x * y % 2 + x * y % 3 == 0,
                    6 => (x * y % 2 + x * y % 3) % 2 == 0,
                    _ => ((x + y) % 2 + x * y % 3) % 2 == 0,
                };
                let i = y * size + x;
                if flip && !self.function[i] {
                    self.modules[i] = !self.modules[i];
                }
            }
        }
    }

    /// The standard's penalty: runs of five or more of a colour, two by two blocks, finder
    /// look-alikes, and the balance of dark and light.
    fn penalty(&self) -> u32 {
        let size = self.size;
        let mut p = 0u32;
        for pass in 0..2 {
            for a in 0..size {
                let at = |b: usize| if pass == 0 { self.get(b, a) } else { self.get(a, b) };
                let mut run = 1;
                for b in 1..size {
                    if at(b) == at(b - 1) {
                        run += 1;
                    } else {
                        if run >= 5 {
                            p += 3 + (run - 5);
                        }
                        run = 1;
                    }
                }
                if run >= 5 {
                    p += 3 + (run - 5);
                }
                // dark-light-dark-dark-dark-light-dark with four light on one side
                let line: Vec<bool> = (0..size).map(at).collect();
                const CORE: [bool; 7] = [true, false, true, true, true, false, true];
                for s in 0..size.saturating_sub(6) {
                    if line[s..s + 7] != CORE {
                        continue;
                    }
                    let light = |from: i32, to: i32| (from..to).all(|k| k < 0 || k >= size as i32 || !line[k as usize]);
                    if light(s as i32 - 4, s as i32) || light(s as i32 + 7, s as i32 + 11) {
                        p += 40;
                    }
                }
            }
        }
        for y in 0..size - 1 {
            for x in 0..size - 1 {
                let c = self.get(x, y);
                if c == self.get(x + 1, y) && c == self.get(x, y + 1) && c == self.get(x + 1, y + 1) {
                    p += 3;
                }
            }
        }
        let dark = self.modules.iter().filter(|&&d| d).count() as i64;
        let total = (size * size) as i64;
        let k = (((dark * 20 - total * 10).abs() + total - 1) / total - 1).max(0);
        p + k as u32 * 10
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A code as rows of hex digits (four modules each, the last digit padded), as the
    /// reference script printed it from `qrcode-generator`.
    fn rows(qr: &Qr) -> Vec<String> {
        (0..qr.size)
            .map(|y| {
                (0..qr.size.div_ceil(4))
                    .map(|k| {
                        let v = (0..4).fold(0u32, |a, b| (a << 1) | qr.dark(k * 4 + b, y) as u32);
                        char::from_digit(v, 16).unwrap()
                    })
                    .collect()
            })
            .collect()
    }

    /// The code of `text` at `ecc` equals the reference with one of the eight masks (the
    /// library chooses its mask by rules of its own).
    fn matches(text: &str, ecc: Ecc, reference: &[&str]) -> u8 {
        for m in 0..8 {
            let q = encode_with(text.as_bytes(), ecc, Some(m)).unwrap();
            if rows(&q) == reference {
                return m;
            }
        }
        let q = encode_with(text.as_bytes(), ecc, Some(0)).unwrap();
        panic!("no mask gives the reference for {text:?} (version {}, {} modules; ours with mask 0: {:?})", q.version(), q.size, rows(&q));
    }

    #[test]
    fn the_tables_add_up() {
        // (the standard's capacities in byte mode)
        assert_eq!((capacity(1, Ecc::L), capacity(1, Ecc::M)), (17, 14));
        assert_eq!((capacity(3, Ecc::L), capacity(3, Ecc::M)), (53, 42));
        assert_eq!((capacity(7, Ecc::L), capacity(7, Ecc::M)), (154, 122));
        assert_eq!((capacity(10, Ecc::L), capacity(10, Ecc::M)), (271, 213));
        assert_eq!(raw_modules(1), 208);
        assert_eq!(raw_modules(7), 1568);
        assert_eq!(Grid::new(7).alignment_positions(), vec![6, 22, 38]);
        assert_eq!(Grid::new(10).alignment_positions(), vec![6, 28, 50]);
        assert!(encode(&"x".repeat(214), Ecc::M).is_none());
        assert_eq!(encode(&"x".repeat(213), Ecc::M).unwrap().version(), 10);
        // a long address at L when M has no room for it
        assert_eq!(encode_address(&"x".repeat(250)).unwrap().version(), 10);
        assert!(encode_address(&"x".repeat(272)).is_none());
    }

    #[test]
    fn reed_solomon_of_the_standards_example() {
        // ISO/IEC 18004 annex I: "01234567" at 1-M
        let data = [0x10, 0x20, 0x0C, 0x56, 0x61, 0x80, 0xEC, 0x11, 0xEC, 0x11, 0xEC, 0x11, 0xEC, 0x11, 0xEC, 0x11];
        assert_eq!(rs_remainder(&data, &rs_divisor(10)), vec![0xA5, 0x24, 0xD4, 0xC1, 0xED, 0x36, 0xC7, 0x87, 0x2C, 0x55]);
    }

    include!("qr_reference.rs");
}
