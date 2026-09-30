//! The binary vehicle state (`STATE`): what a player's bus does right now, bit-packed.
//!
//! A state is sent up to twenty times a second, so it is kept small: a bus with its lamps,
//! doors, wheels and the variables its outside sounds follow takes 50-70 bytes, where the
//! text pose of protocol 2 took about 200 without any of that. Every field has a fixed
//! width and range; a value outside the range is clamped when it is written, so whatever a
//! datagram holds decodes to finite numbers inside those ranges - the receiver never sees a
//! NaN, an infinity or a position a kilometre off the map's scale because of a bad packet.
//!
//! ```text
//! byte 0     0xB3 (no text message starts with it)
//! byte 1     protocol version
//! bytes 2-3  player id (little endian)
//! bytes 4-5  sequence number (little endian, wraps)
//! then, least significant bit first:
//!   flags 10                        FLAG_*; without FLAG_VEHICLE nothing follows (a heartbeat)
//!   x, y 32 each                    centimetres (±21 000 km: Spandau's world coordinates fit)
//!   z 24                            centimetres (±83 km)
//!   heading 16                      360/65536 degrees
//!   pitch, bank 12 each             0.01 degrees (±20.47)
//!   speed 14                        0.05 km/h (±409)
//!   steer 11                        0.05 degrees, front wheel angle, + right (±51)
//!   head light 2, interior light 2, indicators 2 (0 off, 1 left, 2 right, 3 hazard)
//!   engine speed 10                 5 rpm (0 … 5115)
//!   throttle 5, brake 5             /31
//!   passengers 8
//!   doors 3 + 4 each                opening /15
//!   wheels 4 + 7 each               suspension travel, 5 mm (±0.32 m)
//!   rear sections 2 + 60 each       dx, dy 16 (cm from the front origin), dz 12 (cm), heading 16
//!   lamps 7 + 2 each                /3 (the vehicle's lamp variables, see the game's sync table)
//!   switches 5 + 4 each             small integers −8 … 7 (`[visible]` variables)
//!   values 6 + 16 each              IEEE half floats (sound and moving-part variables)
//! ```

use crate::{Aboard, PartPose, Pose, Walker};

pub const STATE_MAGIC: u8 = 0xB3;
/// Bytes before the bit stream.
pub const STATE_HEADER: usize = 6;

/// The pose carries a vehicle (else it is only a heartbeat).
pub const FLAG_VEHICLE: u32 = 1;
/// The engine runs.
pub const FLAG_ENGINE: u32 = 2;
/// The electrics are on (the bus is in service).
pub const FLAG_ELECTRICS: u32 = 4;
/// The horn sounds.
pub const FLAG_HORN: u32 = 8;
pub const FLAG_BRAKE: u32 = 16;
pub const FLAG_REVERSE: u32 = 32;
pub const FLAG_FOG: u32 = 64;
/// The bus is lowered at a stop.
pub const FLAG_KNEELING: u32 = 128;
/// The wipers are running.
pub const FLAG_WIPERS: u32 = 256;
/// The bus stands at a stop with its stop brake (door release) set.
pub const FLAG_STOP_BRAKE: u32 = 512;
const FLAG_BITS: u32 = 10;

pub const MAX_DOORS: usize = 7;
pub const MAX_WHEELS: usize = 15;
pub const MAX_REAR: usize = 3;
pub const MAX_LAMPS: usize = 127;
pub const MAX_SWITCHES: usize = 31;
pub const MAX_VALUES: usize = 63;
/// The longest state a sender may put together (and a receiver accepts).
pub const MAX_STATE_BYTES: usize = 512;

/// Writes values of a given bit width, least significant bit first.
#[derive(Default)]
pub struct BitWriter {
    buf: Vec<u8>,
    acc: u64,
    n: u32,
}

impl BitWriter {
    pub fn with_header(header: &[u8]) -> BitWriter {
        BitWriter {
            buf: header.to_vec(),
            acc: 0,
            n: 0,
        }
    }

    /// The low `bits` bits of `v` (at most 32).
    pub fn put(&mut self, v: u64, bits: u32) {
        debug_assert!(bits <= 32);
        let v = v & ((1u64 << bits) - 1);
        self.acc |= v << self.n;
        self.n += bits;
        while self.n >= 8 {
            self.buf.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }

    /// `v` as a two's complement number of `bits` bits, clamped to what fits.
    pub fn put_signed(&mut self, v: i64, bits: u32) {
        let max = (1i64 << (bits - 1)) - 1;
        self.put(v.clamp(-max - 1, max) as u64, bits);
    }

    /// `v` clamped to `0 ..= 2^bits - 1`.
    pub fn put_unsigned(&mut self, v: i64, bits: u32) {
        self.put(v.clamp(0, (1i64 << bits) - 1) as u64, bits);
    }

    /// `v / step`, rounded, as a signed field; a NaN writes 0.
    pub fn put_fixed(&mut self, v: f64, step: f64, bits: u32) {
        let q = if v.is_finite() {
            (v / step).round()
        } else {
            0.0
        };
        self.put_signed(q.clamp(i64::MIN as f64, i64::MAX as f64) as i64, bits);
    }

    /// `v` in 0..=1 on `bits` bits.
    pub fn put_unit(&mut self, v: f32, bits: u32) {
        let top = ((1u32 << bits) - 1) as f32;
        let q = if v.is_finite() {
            (v.clamp(0.0, 1.0) * top).round()
        } else {
            0.0
        };
        self.put(q as u64, bits);
    }

    pub fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.buf.push(self.acc as u8);
        }
        self.buf
    }
}

/// Reads what `BitWriter` wrote; every read fails past the end.
pub struct BitReader<'a> {
    data: &'a [u8],
    bit: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> BitReader<'a> {
        BitReader { data, bit: 0 }
    }

    pub fn get(&mut self, bits: u32) -> Option<u64> {
        let end = self.bit + bits as usize;
        if end > self.data.len() * 8 {
            return None;
        }
        let mut v = 0u64;
        for k in 0..bits as usize {
            let b = self.bit + k;
            v |= (((self.data[b / 8] >> (b % 8)) & 1) as u64) << k;
        }
        self.bit = end;
        Some(v)
    }

    pub fn get_signed(&mut self, bits: u32) -> Option<i64> {
        let v = self.get(bits)? as i64;
        let sign = 1i64 << (bits - 1);
        Some((v ^ sign) - sign)
    }

    pub fn get_fixed(&mut self, step: f64, bits: u32) -> Option<f64> {
        Some(self.get_signed(bits)? as f64 * step)
    }

    pub fn get_unit(&mut self, bits: u32) -> Option<f32> {
        Some(self.get(bits)? as f32 / ((1u32 << bits) - 1) as f32)
    }
}

/// `v` as an IEEE 754 half float: NaN becomes 0, the rest is clamped to ±65504.
pub fn f16_from(v: f32) -> u16 {
    if v.is_nan() {
        return 0;
    }
    let x = v.clamp(-65504.0, 65504.0).to_bits();
    let sign = (x >> 16) & 0x8000;
    let exp = ((x >> 23) & 0xff) as i32 - 127 + 15;
    let man = x & 0x7f_ffff;
    if exp <= 0 {
        // too small for a normal half: a subnormal one, or zero
        if 14 - exp > 24 {
            return sign as u16;
        }
        let man = man | 0x80_0000;
        let mut half_man = man >> (14 - exp);
        let round = 1u32 << (13 - exp);
        if man & round != 0 && man & (3 * round - 1) != 0 {
            half_man += 1;
        }
        return (sign | half_man) as u16;
    }
    let half = sign | ((exp as u32) << 10) | (man >> 13);
    let round = 0x1000u32;
    if man & round != 0 && man & (3 * round - 1) != 0 {
        (half + 1) as u16
    } else {
        half as u16
    }
}

/// The value of an IEEE 754 half float; the infinities and NaNs a sender cannot produce
/// read as 0.
pub fn f16_to(h: u16) -> f32 {
    let h = h as u32;
    let sign = (h & 0x8000) << 16;
    let exp = h & 0x7c00;
    let man = h & 0x03ff;
    if h & 0x7fff == 0 {
        return f32::from_bits(sign);
    }
    if exp == 0x7c00 {
        return 0.0;
    }
    if exp == 0 {
        let e = (man as u16).leading_zeros() - 6;
        let exp = (127 - 15 - e) << 23;
        let man = (man << (14 + e)) & 0x7f_ffff;
        return f32::from_bits(sign | exp | man);
    }
    let exp = ((exp >> 10) + 127 - 15) << 23;
    f32::from_bits(sign | exp | (man << 13))
}

/// The state part of `pose` as a datagram.
pub fn encode_state(pose: &Pose, protocol: u8, seq: u16) -> Vec<u8> {
    let id = pose.id.min(u16::MAX as u32) as u16;
    let mut header = vec![STATE_MAGIC, protocol];
    header.extend_from_slice(&id.to_le_bytes());
    header.extend_from_slice(&seq.to_le_bytes());
    let mut w = BitWriter::with_header(&header);
    let flags = pose.flags & ((1 << FLAG_BITS) - 1);
    w.put(flags as u64, FLAG_BITS);
    if flags & FLAG_VEHICLE == 0 {
        put_walker(&mut w, pose.walker);
        put_tail(&mut w, pose);
        return w.finish();
    }
    w.put_fixed(pose.x, 0.01, 32);
    w.put_fixed(pose.y, 0.01, 32);
    w.put_fixed(pose.z, 0.01, 24);
    let heading = if pose.heading.is_finite() {
        pose.heading.rem_euclid(360.0)
    } else {
        0.0
    };
    w.put(
        (heading as f64 / 360.0 * 65536.0).round() as u64 % 65536,
        16,
    );
    w.put_fixed(pose.pitch as f64, 0.01, 12);
    w.put_fixed(pose.bank as f64, 0.01, 12);
    w.put_fixed(pose.speed_kmh as f64, 0.05, 14);
    w.put_fixed(pose.steer_deg as f64, 0.05, 11);
    w.put_unsigned(pose.head as i64, 2);
    w.put_unsigned(pose.interior as i64, 2);
    w.put_unsigned(pose.blinker as i64, 2);
    w.put_unsigned(
        if pose.rpm.is_finite() {
            (pose.rpm / 5.0).round() as i64
        } else {
            0
        },
        10,
    );
    w.put_unit(pose.throttle, 5);
    w.put_unit(pose.brake, 5);
    w.put_unsigned(pose.passengers as i64, 8);
    let doors = &pose.doors[..pose.doors.len().min(MAX_DOORS)];
    w.put(doors.len() as u64, 3);
    for d in doors {
        w.put_unit(*d, 4);
    }
    let wheels = &pose.suspension[..pose.suspension.len().min(MAX_WHEELS)];
    w.put(wheels.len() as u64, 4);
    for s in wheels {
        w.put_fixed(*s as f64, 0.005, 7);
    }
    // the rear sections from where the receiver will put the front (its rounded position)
    let rounded = |v: f64| {
        if v.is_finite() {
            (v / 0.01).round() * 0.01
        } else {
            0.0
        }
    };
    let rear = &pose.rear[..pose.rear.len().min(MAX_REAR)];
    w.put(rear.len() as u64, 2);
    for r in rear {
        w.put_fixed(r.x - rounded(pose.x), 0.01, 16);
        w.put_fixed(r.y - rounded(pose.y), 0.01, 16);
        w.put_fixed(r.z - rounded(pose.z), 0.01, 12);
        let h = if r.heading.is_finite() {
            r.heading.rem_euclid(360.0)
        } else {
            0.0
        };
        w.put((h as f64 / 360.0 * 65536.0).round() as u64 % 65536, 16);
    }
    let lamps = &pose.lamps[..pose.lamps.len().min(MAX_LAMPS)];
    w.put(lamps.len() as u64, 7);
    for l in lamps {
        w.put_unit(*l, 2);
    }
    let switches = &pose.switches[..pose.switches.len().min(MAX_SWITCHES)];
    w.put(switches.len() as u64, 5);
    for s in switches {
        w.put_fixed(*s as f64, 1.0, 4);
    }
    let values = &pose.values[..pose.values.len().min(MAX_VALUES)];
    w.put(values.len() as u64, 6);
    for v in values {
        w.put(f16_from(*v) as u64, 16);
    }
    // the player on foot (after the vehicle: a pose without it ends before)
    put_walker(&mut w, pose.walker);
    put_tail(&mut w, pose);
    w.finish()
}

/// What a state datagram says: (player id, sequence number, the state fields of a pose).
/// None for anything that is not a complete state of this protocol.
pub fn decode_state(data: &[u8], protocol: u8) -> Option<(u32, u16, Pose)> {
    if data.len() < STATE_HEADER + 2
        || data.len() > MAX_STATE_BYTES
        || data[0] != STATE_MAGIC
        || data[1] != protocol
    {
        return None;
    }
    let id = u16::from_le_bytes([data[2], data[3]]) as u32;
    let seq = u16::from_le_bytes([data[4], data[5]]);
    let mut r = BitReader::new(&data[STATE_HEADER..]);
    let mut p = Pose {
        id,
        ..Default::default()
    };
    p.flags = r.get(FLAG_BITS)? as u32;
    if p.flags & FLAG_VEHICLE == 0 {
        p.walker = get_walker(&mut r);
        get_tail(&mut r, &mut p);
        return Some((id, seq, p));
    }
    p.x = r.get_fixed(0.01, 32)?;
    p.y = r.get_fixed(0.01, 32)?;
    p.z = r.get_fixed(0.01, 24)?;
    p.heading = (r.get(16)? as f64 * 360.0 / 65536.0) as f32;
    p.pitch = r.get_fixed(0.01, 12)? as f32;
    p.bank = r.get_fixed(0.01, 12)? as f32;
    p.speed_kmh = r.get_fixed(0.05, 14)? as f32;
    p.steer_deg = r.get_fixed(0.05, 11)? as f32;
    p.head = r.get(2)? as u8;
    p.interior = r.get(2)? as u8;
    p.blinker = r.get(2)? as u8;
    p.rpm = r.get(10)? as f32 * 5.0;
    p.throttle = r.get_unit(5)?;
    p.brake = r.get_unit(5)?;
    p.passengers = r.get(8)? as u32;
    let n = r.get(3)? as usize;
    p.doors = (0..n).map(|_| r.get_unit(4)).collect::<Option<_>>()?;
    let n = r.get(4)? as usize;
    p.suspension = (0..n)
        .map(|_| r.get_fixed(0.005, 7).map(|v| v as f32))
        .collect::<Option<_>>()?;
    let n = r.get(2)? as usize;
    for _ in 0..n {
        let dx = r.get_fixed(0.01, 16)?;
        let dy = r.get_fixed(0.01, 16)?;
        let dz = r.get_fixed(0.01, 12)?;
        let h = (r.get(16)? as f64 * 360.0 / 65536.0) as f32;
        p.rear.push(PartPose {
            x: p.x + dx,
            y: p.y + dy,
            z: p.z + dz,
            heading: h,
        });
    }
    let n = r.get(7)? as usize;
    p.lamps = (0..n).map(|_| r.get_unit(2)).collect::<Option<_>>()?;
    let n = r.get(5)? as usize;
    p.switches = (0..n)
        .map(|_| r.get_signed(4).map(|v| v as f32))
        .collect::<Option<_>>()?;
    let n = r.get(6)? as usize;
    p.values = (0..n)
        .map(|_| r.get(16).map(|v| f16_to(v as u16)))
        .collect::<Option<_>>()?;
    p.walker = get_walker(&mut r);
    get_tail(&mut r, &mut p);
    Some((id, seq, p))
}

/// What came after the walker in later versions (an older game's state ends before it,
/// and an older game reads no further): the sender's clock, and where the walker is aboard
/// a player's bus.
fn put_tail(w: &mut BitWriter, pose: &Pose) {
    w.put(1, 1);
    w.put(pose.sent_ms as u64, 32);
    match pose.walker.and_then(|k| k.aboard) {
        Some(a) => {
            w.put(1, 1);
            w.put(a.owner.min(u16::MAX as u32) as u64, 16);
            for v in a.local {
                w.put_fixed(v as f64, 0.005, 14);
            }
            match a.seat {
                Some(k) => {
                    w.put(1, 1);
                    w.put(k as u64, 10);
                }
                None => w.put(0, 1),
            }
        }
        None => w.put(0, 1),
    }
    // later still: the doors to 1/255 (the 4 bits above opened a door in sixteen visible
    // steps, a door that moved "at 20 fps" in the others' games), and the way the walker
    // goes (a player stepping sideways was drawn walking forwards, the legs dragged across)
    w.put(1, 1);
    let doors = &pose.doors[..pose.doors.len().min(MAX_DOORS)];
    for d in doors {
        w.put_unit(*d, 8);
    }
    match pose.walker {
        Some(k) => {
            w.put(1, 1);
            let c = if k.course.is_finite() { k.course.rem_euclid(360.0) } else { k.heading.rem_euclid(360.0) };
            w.put((c as f64 / 360.0 * 65536.0).round() as u64 % 65536, 16);
        }
        None => w.put(0, 1),
    }
}

fn get_tail(r: &mut BitReader, p: &mut Pose) {
    if r.get(1).unwrap_or(0) == 0 {
        return;
    }
    let Some(ms) = r.get(32) else { return };
    p.sent_ms = ms as u32;
    let Some(has_aboard) = r.get(1) else { return };
    if has_aboard == 1 {
        let aboard = (|| {
            let owner = r.get(16)? as u32;
            let local = [r.get_fixed(0.005, 14)? as f32, r.get_fixed(0.005, 14)? as f32, r.get_fixed(0.005, 14)? as f32];
            let seat = if r.get(1)? == 1 { Some(r.get(10)? as u16) } else { None };
            Some(Aboard { owner, local, seat })
        })();
        let Some(aboard) = aboard else { return };
        if let Some(w) = p.walker.as_mut() {
            w.aboard = Some(aboard);
        }
    }
    // (an older game's state ends here)
    if r.get(1).unwrap_or(0) == 0 {
        return;
    }
    let fine: Option<Vec<f32>> = (0..p.doors.len()).map(|_| r.get_unit(8)).collect();
    let Some(fine) = fine else { return };
    p.doors = fine;
    if r.get(1).unwrap_or(0) == 1 {
        if let (Some(c), Some(w)) = (r.get(16), p.walker.as_mut()) {
            w.course = (c as f64 * 360.0 / 65536.0) as f32;
        }
    }
}

fn put_walker(w: &mut BitWriter, walker: Option<Walker>) {
    match walker {
        Some(k) => {
            w.put(1, 1);
            w.put_fixed(k.x, 0.01, 32);
            w.put_fixed(k.y, 0.01, 32);
            w.put_fixed(k.z, 0.01, 24);
            let h = if k.heading.is_finite() { k.heading.rem_euclid(360.0) } else { 0.0 };
            w.put((h as f64 / 360.0 * 65536.0).round() as u64 % 65536, 16);
            w.put_fixed(k.speed as f64, 0.05, 9);
            w.put(k.seated as u64, 1);
        }
        None => w.put(0, 1),
    }
}

/// The walker at the end of a state (None there, or the stream ends first).
fn get_walker(r: &mut BitReader) -> Option<Walker> {
    if r.get(1)? == 0 {
        return None;
    }
    Some(Walker {
        x: r.get_fixed(0.01, 32)?,
        y: r.get_fixed(0.01, 32)?,
        z: r.get_fixed(0.01, 24)?,
        heading: (r.get(16)? as f64 * 360.0 / 65536.0) as f32,
        speed: r.get_fixed(0.05, 9)? as f32,
        course: f32::NAN,
        seated: r.get(1)? == 1,
        aboard: None,
    })
}

/// Is sequence number `a` newer than `b` (they wrap around)?
pub fn seq_newer(a: u16, b: u16) -> bool {
    a != b && a.wrapping_sub(b) < 0x8000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_floats() {
        for v in [
            0.0f32, 1.0, -1.0, 0.5, 2.0, 65504.0, -65504.0, 1024.0, 0.25, 3.0, 1850.0, 0.001, -0.3,
        ] {
            let back = f16_to(f16_from(v));
            let tol = (v.abs() * 0.0006).max(1.0e-4);
            assert!((back - v).abs() <= tol, "{v} -> {back}");
        }
        assert_eq!(f16_to(f16_from(f32::NAN)), 0.0);
        assert_eq!(f16_to(f16_from(f32::INFINITY)), 65504.0);
        assert_eq!(f16_to(f16_from(-1.0e9)), -65504.0);
        assert_eq!(f16_to(f16_from(1.0e-9)), 0.0);
        // every bit pattern reads as a finite number
        for h in 0..=u16::MAX {
            assert!(f16_to(h).is_finite(), "{h:#x}");
        }
        // integers of an engine speed stay exact
        for rpm in [0.0f32, 480.0, 522.0, 989.0, 1023.0] {
            assert_eq!(f16_to(f16_from(rpm)), rpm);
        }
    }

    #[test]
    fn bits_round_trip() {
        let mut w = BitWriter::with_header(&[7]);
        w.put(5, 3);
        w.put_signed(-3, 4);
        w.put_signed(1000, 8); // clamped to 127
        w.put_unsigned(-5, 4); // clamped to 0
        w.put(0xDEAD_BEEF, 32);
        w.put_fixed(-12.345, 0.01, 16);
        w.put_unit(0.5, 5);
        let buf = w.finish();
        assert_eq!(buf[0], 7);
        let mut r = BitReader::new(&buf[1..]);
        assert_eq!(r.get(3), Some(5));
        assert_eq!(r.get_signed(4), Some(-3));
        assert_eq!(r.get_signed(8), Some(127));
        assert_eq!(r.get(4), Some(0));
        assert_eq!(r.get(32), Some(0xDEAD_BEEF));
        assert!((r.get_fixed(0.01, 16).unwrap() + 12.35).abs() < 0.006);
        assert_eq!(r.get_unit(5), Some(16.0 / 31.0));
        // past the end
        assert_eq!(r.get(32), None);
    }

    fn bus() -> Pose {
        Pose {
            id: 3,
            flags: FLAG_VEHICLE | FLAG_ENGINE | FLAG_ELECTRICS | FLAG_HORN,
            x: 892_248.37,
            y: 4_196_461.12,
            z: 33.21,
            heading: 271.3,
            pitch: -1.25,
            bank: 0.4,
            speed_kmh: -7.35,
            steer_deg: 12.5,
            head: 2,
            interior: 3,
            blinker: 1,
            rpm: 1850.0,
            throttle: 0.6,
            brake: 0.0,
            passengers: 37,
            doors: vec![1.0, 0.8, 0.0, 0.0, 0.2],
            suspension: vec![-0.105, -0.1, -0.02, 0.0],
            rear: vec![PartPose {
                x: 892_240.0,
                y: 4_196_461.5,
                z: 33.3,
                heading: 268.0,
            }],
            lamps: vec![1.0, 0.0, 1.0, 2.0 / 3.0],
            switches: vec![1.0, 0.0, -1.0, 3.0],
            values: vec![1850.0, 0.6, 412.5],
            ..Default::default()
        }
    }

    #[test]
    fn state_round_trip() {
        let p = bus();
        let data = encode_state(&p, 3, 4711);
        assert!(data.len() <= 70, "a bus state takes {} bytes", data.len());
        let (id, seq, q) = decode_state(&data, 3).expect("decodes");
        assert_eq!((id, seq), (3, 4711));
        assert_eq!(q.flags, p.flags);
        assert!(
            (q.x - p.x).abs() < 0.006 && (q.y - p.y).abs() < 0.006 && (q.z - p.z).abs() < 0.006,
            "{q:?}"
        );
        assert!((q.heading - p.heading).abs() < 0.01);
        assert!((q.pitch - p.pitch).abs() < 0.006 && (q.bank - p.bank).abs() < 0.006);
        assert!(
            (q.speed_kmh - p.speed_kmh).abs() < 0.03 && (q.steer_deg - p.steer_deg).abs() < 0.03
        );
        assert_eq!(
            (q.head, q.interior, q.blinker, q.rpm, q.passengers),
            (2, 3, 1, 1850.0, 37)
        );
        assert!((q.throttle - 0.6).abs() < 0.02 && q.brake == 0.0);
        assert_eq!(q.doors.len(), 5);
        assert!(
            q.doors
                .iter()
                .zip(&p.doors)
                .all(|(a, b)| (a - b).abs() < 0.04),
            "{:?}",
            q.doors
        );
        assert!(
            q.suspension
                .iter()
                .zip(&p.suspension)
                .all(|(a, b)| (a - b).abs() < 0.003),
            "{:?}",
            q.suspension
        );
        assert_eq!(q.rear.len(), 1);
        assert!(
            (q.rear[0].x - 892_240.0).abs() < 0.006
                && (q.rear[0].y - 4_196_461.5).abs() < 0.006
                && (q.rear[0].heading - 268.0).abs() < 0.01
        );
        assert_eq!(q.lamps, p.lamps);
        assert_eq!(q.switches, p.switches);
        assert_eq!(q.values[0], 1850.0);
        assert!((q.values[1] - 0.6).abs() < 0.001 && q.values[2] == 412.5);
        // a heartbeat is the header, two bytes and the sender's clock
        let beat = encode_state(
            &Pose {
                id: 9,
                ..Default::default()
            },
            3,
            1,
        );
        assert!(beat.len() <= STATE_HEADER + 7, "{}", beat.len());
        let (id, _, q) = decode_state(&beat, 3).unwrap();
        assert_eq!((id, q.flags & FLAG_VEHICLE), (9, 0));
    }

    #[test]
    fn states_out_of_range_are_clamped() {
        let mut p = bus();
        p.x = f64::NAN;
        p.y = 1.0e12;
        p.heading = f32::INFINITY;
        p.speed_kmh = 5000.0;
        p.rpm = -3.0;
        p.head = 9;
        p.passengers = 1000;
        p.doors = vec![2.0; 20];
        p.lamps = vec![0.5; 400];
        p.values = vec![f32::NAN, 1.0e30];
        let (_, _, q) = decode_state(&encode_state(&p, 3, 0), 3).unwrap();
        assert_eq!(q.x, 0.0);
        assert!((q.y - 21_474_836.47).abs() < 0.01, "{}", q.y);
        assert_eq!(q.heading, 0.0);
        assert!((q.speed_kmh - 409.55).abs() < 0.01, "{}", q.speed_kmh);
        assert_eq!((q.rpm, q.head, q.passengers), (0.0, 3, 255));
        assert_eq!(q.doors, vec![1.0; MAX_DOORS]);
        assert_eq!(q.lamps.len(), MAX_LAMPS);
        assert_eq!(q.values, vec![0.0, 65504.0]);
    }

    /// Every list at its longest - 63 values among them - still makes a state a receiver
    /// takes, and the values come back in order.
    #[test]
    fn a_state_with_every_list_full_fits() {
        let mut p = bus();
        p.doors = vec![0.5; MAX_DOORS];
        p.lamps = vec![0.5; MAX_LAMPS];
        p.switches = vec![3.0; MAX_SWITCHES];
        p.values = (0..MAX_VALUES).map(|k| k as f32).collect();
        p.rear = vec![PartPose { x: 1.0, y: -12.0, z: 0.0, heading: 5.0 }; MAX_REAR];
        p.walker = Some(Walker { x: 1.0, y: 2.0, z: 3.0, heading: 10.0, speed: 1.4, course: 100.0, seated: false, aboard: None });
        let data = encode_state(&p, 6, 0);
        assert!(data.len() <= MAX_STATE_BYTES, "{} bytes", data.len());
        let (_, _, q) = decode_state(&data, 6).unwrap();
        assert_eq!(q.values, p.values);
        assert_eq!(q.lamps.len(), MAX_LAMPS);
        assert!(q.walker.is_some());
    }

    #[test]
    fn doors_are_fine_and_walkers_keep_their_course() {
        let mut p = bus();
        p.doors = vec![0.37, 0.5, 1.0];
        p.walker = Some(Walker { x: 1.0, y: 2.0, z: 3.0, heading: 10.0, speed: 1.4, course: 100.0, seated: false, aboard: None });
        p.sent_ms = 1234;
        let (_, _, q) = decode_state(&encode_state(&p, 3, 0), 3).unwrap();
        // 1/255 now, not the sixteen steps of the 4 bits before the tail
        for (a, b) in q.doors.iter().zip(&p.doors) {
            assert!((a - b).abs() < 0.003, "{a} vs {b}");
        }
        let w = q.walker.unwrap();
        assert!((w.course - 100.0).abs() < 0.01 && (w.heading - 10.0).abs() < 0.01);
    }

    #[test]
    fn garbage_is_not_a_state() {
        let good = encode_state(&bus(), 3, 5);
        assert!(decode_state(&good, 2).is_none(), "another protocol");
        // (the sender's clock at the end is optional: an older game's state ends before it;
        // so are the fine door openings and the walker's course after it)
        let b = bus();
        let optional = 5 + (2 + b.doors.len().min(MAX_DOORS) * 8 + if b.walker.is_some() { 16 } else { 0 }).div_ceil(8);
        for cut in 0..good.len() - optional {
            assert!(decode_state(&good[..cut], 3).is_none(), "cut at {cut}");
        }
        assert!(decode_state(b"POSE|1|a|b", 3).is_none());
        let mut long = good.clone();
        long.resize(MAX_STATE_BYTES + 1, 0);
        assert!(decode_state(&long, 3).is_none());
        // random bytes behind a valid header never panic and never give a non-finite value
        let mut seed = 0x1234_5678u32;
        for _ in 0..20_000 {
            let len = STATE_HEADER + 2 + (seed % 90) as usize;
            let mut d = vec![STATE_MAGIC, 3];
            while d.len() < len {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                d.push(seed as u8);
            }
            if let Some((_, _, p)) = decode_state(&d, 3) {
                assert!(
                    p.x.is_finite()
                        && p.heading.is_finite()
                        && p.values.iter().all(|v| v.is_finite())
                );
                assert!(
                    p.doors.len() <= MAX_DOORS
                        && p.rear.len() <= MAX_REAR
                        && p.lamps.len() <= MAX_LAMPS
                );
                assert!(p.heading >= 0.0 && p.heading < 360.0);
            }
        }
    }

    #[test]
    fn sequence_numbers_wrap() {
        assert!(seq_newer(2, 1));
        assert!(!seq_newer(1, 2));
        assert!(seq_newer(0, 65535));
        assert!(!seq_newer(65535, 0));
        assert!(!seq_newer(7, 7));
    }
}
