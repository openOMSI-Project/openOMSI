//! The dispatch radio ("Phonie SAE"): a dedicated server's players and a dispatcher talk
//! over a two-way radio, the way a bus company's control room talks to its drivers.
//!
//! * The voice is 8 kHz mono - a radio's band, nothing above 4 kHz to carry - in IMA ADPCM
//!   (4 bits a sample): 32 kbit/s, 40 ms a frame (`FRAME_SAMPLES`), about 4 kB/s for one
//!   voice. A frame says the coder's state it starts from, so a lost one costs 40 ms and
//!   nothing after it.
//! * In the session a frame goes as a datagram of its own (`RADIO_MAGIC`, then the id of who
//!   speaks: 0 the dispatcher): a player's up to the server, the dispatcher's down to the
//!   players in the call. What else the radio says - a player asking to be called, the
//!   call a player is in - goes as commands (`radio …`, see the game's `phonie.rs`).
//! * A dispatcher's console (the server's web dispatch page) connects to the server's
//!   gateway at `/dispatch` (a WebSocket, from this machine only, with the admin password -
//!   see `ws.rs`): JSON text messages both ways, and the voice as binary messages - a
//!   frame from the console, `[speaker u32][frame]` to it (a player's id, or
//!   `console_speaker` for another console's dispatcher). The door's headers say who sits
//!   at the console (`X-Dispatcher`, percent-encoded) and whether it may listen to every
//!   call (`X-Dispatcher-Monitor: 1`). The consoles' messages wait in a
//!   process-wide hub (`take_console_input`) for the server loop, which owns the session.

use std::sync::mpsc;
use std::sync::Mutex;

/// The first byte of a radio datagram (no text message starts with it).
pub const RADIO_MAGIC: u8 = 0xB6;
/// The radio's sample rate (Hz).
pub const RATE: u32 = 8000;
/// Samples in one frame: 40 ms.
pub const FRAME_SAMPLES: usize = 320;
/// The longest frame taken in (bytes): its head and a little over 40 ms of samples.
pub const MAX_FRAME: usize = 5 + 400;
/// Who speaks in a datagram from the server when it is the dispatcher.
pub const DISPATCHER: u32 = 0;

/// Who speaks in a frame to a console when it is another console's dispatcher (a console
/// that listens to every call hears them): this bit and the console's id.
pub fn console_speaker(console: u64) -> u32 {
    0x8000_0000 | (console as u32 & 0x7FFF_FFFF)
}

// ---------------------------------------------------------------------------------------
// IMA ADPCM

const STEPS: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66, 73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493, 10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];
const INDEX: [i32; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];

/// The coder's state between samples: the last value and the step's index.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Adpcm {
    pub predictor: i32,
    pub index: i32,
}

impl Adpcm {
    fn decode_nibble(&mut self, n: u8) -> i16 {
        let step = STEPS[self.index as usize];
        let mut diff = step >> 3;
        if n & 4 != 0 {
            diff += step;
        }
        if n & 2 != 0 {
            diff += step >> 1;
        }
        if n & 1 != 0 {
            diff += step >> 2;
        }
        self.predictor = if n & 8 != 0 { self.predictor - diff } else { self.predictor + diff }.clamp(-32768, 32767);
        self.index = (self.index + INDEX[(n & 15) as usize]).clamp(0, 88);
        self.predictor as i16
    }

    fn encode_sample(&mut self, s: i16) -> u8 {
        let step = STEPS[self.index as usize];
        let mut diff = s as i32 - self.predictor;
        let mut n = 0u8;
        if diff < 0 {
            n = 8;
            diff = -diff;
        }
        let mut st = step;
        if diff >= st {
            n |= 4;
            diff -= st;
        }
        st >>= 1;
        if diff >= st {
            n |= 2;
            diff -= st;
        }
        st >>= 1;
        if diff >= st {
            n |= 1;
        }
        // (the decoder's own arithmetic: both sides stay on the same predictor)
        self.decode_nibble(n);
        n
    }
}

/// A frame of voice: where the coder starts and the samples, two to a byte (the first in
/// the low nibble).
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    /// Counts up per frame of one transmission (a receiver tells a lost one by it).
    pub seq: u16,
    pub start: Adpcm,
    pub data: Vec<u8>,
}

impl Frame {
    /// `[seq u16][predictor i16][index u8][data]`, little-endian.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(5 + self.data.len());
        b.extend_from_slice(&self.seq.to_le_bytes());
        b.extend_from_slice(&(self.start.predictor as i16).to_le_bytes());
        b.push(self.start.index as u8);
        b.extend_from_slice(&self.data);
        b
    }

    pub fn from_bytes(b: &[u8]) -> Option<Frame> {
        if b.len() < 6 || b.len() > MAX_FRAME {
            return None;
        }
        let seq = u16::from_le_bytes([b[0], b[1]]);
        let predictor = i16::from_le_bytes([b[2], b[3]]) as i32;
        let index = b[4] as i32;
        if index > 88 {
            return None;
        }
        Some(Frame { seq, start: Adpcm { predictor, index }, data: b[5..].to_vec() })
    }

    /// The samples, as -1..1.
    pub fn decode(&self) -> Vec<f32> {
        let mut st = self.start;
        let mut out = Vec::with_capacity(self.data.len() * 2);
        for &byte in &self.data {
            out.push(st.decode_nibble(byte & 15) as f32 / 32768.0);
            out.push(st.decode_nibble(byte >> 4) as f32 / 32768.0);
        }
        out
    }
}

/// Turns 8 kHz samples into frames, the coder's state kept from one to the next.
#[derive(Debug, Default)]
pub struct Encoder {
    state: Adpcm,
    seq: u16,
}

impl Encoder {
    /// A new transmission: the coder starts again (a receiver that missed the last one
    /// is not off by its state).
    pub fn restart(&mut self) {
        self.state = Adpcm::default();
    }

    /// One frame of `samples` (-1..1; an even count, `FRAME_SAMPLES` as a rule).
    pub fn encode(&mut self, samples: &[f32]) -> Frame {
        let start = self.state;
        let mut data = Vec::with_capacity(samples.len().div_ceil(2));
        for pair in samples.chunks(2) {
            let q = |x: f32| (x.clamp(-1.0, 1.0) * 32767.0) as i16;
            let lo = self.state.encode_sample(q(pair[0]));
            let hi = pair.get(1).map(|&x| self.state.encode_sample(q(x))).unwrap_or(0);
            data.push(lo | (hi << 4));
        }
        let seq = self.seq;
        self.seq = self.seq.wrapping_add(1);
        Frame { seq, start, data }
    }
}

/// A radio datagram: who speaks (`DISPATCHER` or a player's id) and the frame.
pub fn datagram(speaker: u32, frame: &[u8]) -> Vec<u8> {
    let mut d = Vec::with_capacity(5 + frame.len());
    d.push(RADIO_MAGIC);
    d.extend_from_slice(&speaker.to_le_bytes());
    d.extend_from_slice(frame);
    d
}

/// A radio datagram's speaker and frame bytes.
pub fn read_datagram(d: &[u8]) -> Option<(u32, &[u8])> {
    if d.len() < 6 || d[0] != RADIO_MAGIC || d.len() > 5 + MAX_FRAME {
        return None;
    }
    Some((u32::from_le_bytes([d[1], d[2], d[3], d[4]]), &d[5..]))
}

// ---------------------------------------------------------------------------------------
// the dispatchers' consoles

/// What came from a console, for the server loop.
#[derive(Debug, Clone, PartialEq)]
pub enum ConsoleIn {
    /// A console connected (its id: the server answers it alone with `to_console`), who sits
    /// at it, and whether it may listen to every call.
    Opened(u64, String, bool),
    Closed(u64),
    /// A JSON message.
    Text(u64, String),
    /// A frame of the dispatcher's voice (`Frame::to_bytes`).
    Voice(u64, Vec<u8>),
}

/// What goes to a console.
#[derive(Debug, Clone)]
pub enum ConsoleOut {
    Text(String),
    Binary(Vec<u8>),
}

struct Hub {
    /// The server takes consoles (a dedicated server that runs the radio).
    open: bool,
    inbox: Vec<ConsoleIn>,
    consoles: Vec<(u64, mpsc::Sender<ConsoleOut>)>,
    next: u64,
}

static HUB: Mutex<Hub> = Mutex::new(Hub { open: false, inbox: Vec::new(), consoles: Vec::new(), next: 1 });

fn hub() -> std::sync::MutexGuard<'static, Hub> {
    HUB.lock().unwrap_or_else(|e| e.into_inner())
}

/// What a console may have waiting for the server loop at most (a loop that stopped
/// taking them does not pile them up without end).
const INBOX: usize = 1024;

/// The server runs the radio: `/dispatch` takes consoles from now on.
pub fn open_consoles() {
    hub().open = true;
}

pub fn consoles_open() -> bool {
    hub().open
}

/// How many consoles are connected.
pub fn console_count() -> usize {
    hub().consoles.len()
}

/// Everything the consoles said since the last call.
pub fn take_console_input() -> Vec<ConsoleIn> {
    std::mem::take(&mut hub().inbox)
}

/// A message to every console.
pub fn to_consoles(msg: ConsoleOut) {
    let mut h = hub();
    h.consoles.retain(|(_, tx)| tx.send(msg.clone()).is_ok());
}

/// A message to one console.
pub fn to_console(id: u64, msg: ConsoleOut) {
    let h = hub();
    if let Some((_, tx)) = h.consoles.iter().find(|(c, _)| *c == id) {
        let _ = tx.send(msg);
    }
}

/// A console comes in: its id and where its messages arrive.
pub fn register_console(name: &str, monitor: bool) -> (u64, mpsc::Receiver<ConsoleOut>) {
    let (tx, rx) = mpsc::channel();
    let mut h = hub();
    let id = h.next;
    h.next += 1;
    h.consoles.push((id, tx));
    h.inbox.push(ConsoleIn::Opened(id, name.chars().filter(|c| !c.is_control()).take(48).collect(), monitor));
    (id, rx)
}

pub fn console_said(msg: ConsoleIn) {
    let mut h = hub();
    if h.inbox.len() < INBOX {
        h.inbox.push(msg);
    }
}

pub fn unregister_console(id: u64) {
    let mut h = hub();
    h.consoles.retain(|(c, _)| *c != id);
    h.inbox.push(ConsoleIn::Closed(id));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tone_comes_through_the_coder() {
        let tone: Vec<f32> = (0..FRAME_SAMPLES * 4).map(|i| (i as f32 * 2.0 * std::f32::consts::PI * 800.0 / RATE as f32).sin() * 0.5).collect();
        let mut enc = Encoder::default();
        let mut out = Vec::new();
        for chunk in tone.chunks(FRAME_SAMPLES) {
            let f = enc.encode(chunk);
            assert_eq!(f.data.len(), FRAME_SAMPLES / 2);
            out.extend(Frame::from_bytes(&f.to_bytes()).unwrap().decode());
        }
        assert_eq!(out.len(), tone.len());
        // (past the coder's first few samples, it follows the tone closely)
        let err: f32 = tone.iter().zip(&out).skip(64).map(|(a, b)| (a - b).powi(2)).sum::<f32>() / (tone.len() - 64) as f32;
        assert!(err.sqrt() < 0.05, "rms error {}", err.sqrt());
    }

    #[test]
    fn a_lost_frame_costs_only_itself() {
        let tone: Vec<f32> = (0..FRAME_SAMPLES * 3).map(|i| (i as f32 * 0.3).sin() * 0.4).collect();
        let mut enc = Encoder::default();
        let frames: Vec<Frame> = tone.chunks(FRAME_SAMPLES).map(|c| enc.encode(c)).collect();
        // the third decoded alone, without the second: the same samples as in a row
        let all: Vec<f32> = frames.iter().flat_map(|f| f.decode()).collect();
        assert_eq!(all[FRAME_SAMPLES * 2..], frames[2].decode()[..]);
    }

    #[test]
    fn datagrams_and_bad_frames() {
        let f = Encoder::default().encode(&[0.1; FRAME_SAMPLES]).to_bytes();
        let d = datagram(7, &f);
        assert_eq!(read_datagram(&d), Some((7, &f[..])));
        assert!(read_datagram(&d[..5]).is_none());
        // a step index out of the table is no frame
        let mut bad = f.clone();
        bad[4] = 200;
        assert!(Frame::from_bytes(&bad).is_none());
        assert!(Frame::from_bytes(&vec![0; MAX_FRAME + 1]).is_none());
    }
}
