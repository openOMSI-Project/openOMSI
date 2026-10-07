//! The dispatch radio's sound: the microphone taken at the radio's rate, and what comes in
//! made to sound like an analogue two-way radio in a bus cab - a narrow band (300 Hz to
//! 3 kHz), the level squeezed even and driven a little into the speaker's distortion, a
//! hiss under the voice, the squelch opening with a click and closing with its "kssh".

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::Arc;

/// A second-order filter (RBJ's cookbook), one channel.
#[derive(Debug, Clone, Copy)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn new(b: [f32; 3], a: [f32; 3]) -> Biquad {
        Biquad { b0: b[0] / a[0], b1: b[1] / a[0], b2: b[2] / a[0], a1: a[1] / a[0], a2: a[2] / a[0], z1: 0.0, z2: 0.0 }
    }

    pub fn lowpass(rate: f32, f: f32, q: f32) -> Biquad {
        let w = 2.0 * std::f32::consts::PI * f / rate;
        let (sn, cs) = w.sin_cos();
        let al = sn / (2.0 * q);
        Biquad::new([(1.0 - cs) / 2.0, 1.0 - cs, (1.0 - cs) / 2.0], [1.0 + al, -2.0 * cs, 1.0 - al])
    }

    pub fn highpass(rate: f32, f: f32, q: f32) -> Biquad {
        let w = 2.0 * std::f32::consts::PI * f / rate;
        let (sn, cs) = w.sin_cos();
        let al = sn / (2.0 * q);
        Biquad::new([(1.0 + cs) / 2.0, -(1.0 + cs), (1.0 + cs) / 2.0], [1.0 + al, -2.0 * cs, 1.0 - al])
    }

    /// A peak of `gain_db` round `f` (the small speaker's honk near 1.5 kHz).
    pub fn peak(rate: f32, f: f32, q: f32, gain_db: f32) -> Biquad {
        let w = 2.0 * std::f32::consts::PI * f / rate;
        let (sn, cs) = w.sin_cos();
        let a = 10f32.powf(gain_db / 40.0);
        let al = sn / (2.0 * q);
        Biquad::new([1.0 + al * a, -2.0 * cs, 1.0 - al * a], [1.0 + al / a, -2.0 * cs, 1.0 - al / a])
    }

    pub fn run(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// White noise (xorshift), -1..1.
#[derive(Debug, Clone)]
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

/// The radio's sound, at `rate` (the radio's 8 kHz): voice in, what the cab's speaker
/// gives out.
pub struct RadioFx {
    rate: f32,
    band: [Biquad; 5],
    /// The level the voice is squeezed to (a radio's AGC and the transmitter's limiter).
    env: f32,
    noise: Noise,
    hiss: [Biquad; 2],
    /// The signal fading a little, slowly (a bus driving through the city).
    fade_phase: f32,
}

impl RadioFx {
    pub fn new(rate: u32) -> RadioFx {
        let r = rate as f32;
        RadioFx {
            rate: r,
            band: [
                Biquad::highpass(r, 320.0, 0.7),
                Biquad::highpass(r, 320.0, 0.7),
                Biquad::lowpass(r, 2900.0, 0.7),
                Biquad::lowpass(r, 2900.0, 0.7),
                Biquad::peak(r, 1600.0, 1.2, 5.0),
            ],
            env: 0.0,
            noise: Noise(0x9E37_79B9),
            hiss: [Biquad::highpass(r, 900.0, 0.7), Biquad::lowpass(r, 3400.0, 0.7)],
            fade_phase: 0.0,
        }
    }

    fn hiss(&mut self) -> f32 {
        let n = self.noise.next();
        let n = self.hiss[0].run(n);
        self.hiss[1].run(n)
    }

    /// Voice samples (-1..1) as the radio gives them out.
    pub fn voice(&mut self, input: &[f32]) -> Vec<f32> {
        let attack = 1.0 - (-1.0 / (0.004 * self.rate)).exp();
        let release = 1.0 - (-1.0 / (0.25 * self.rate)).exp();
        let mut out = Vec::with_capacity(input.len());
        for &x in input {
            let mut y = x;
            for b in self.band.iter_mut() {
                y = b.run(y);
            }
            // the level held even: loud and quiet speakers come out alike, as through a
            // radio's limiter (a gain of 8 at the most: the room's murmur stays a murmur)
            let a = y.abs();
            self.env += (a - self.env) * if a > self.env { attack } else { release };
            let gain = (0.28 / self.env.max(0.035)).min(8.0);
            y *= gain;
            // driven into the small speaker's distortion
            let drive = 2.6;
            y = (y * drive).tanh() / drive.tanh();
            // fading, slow and slight
            self.fade_phase = (self.fade_phase + 0.7 / self.rate) % 1.0;
            let fade = 1.0 - 0.08 * (0.5 + 0.5 * (self.fade_phase * std::f32::consts::TAU).sin());
            let h = self.hiss() * 0.05;
            out.push((y * 0.62 * fade + h).clamp(-1.0, 1.0));
        }
        out
    }

    /// The squelch opening: a click and a breath of noise before the voice (it also fills
    /// the buffer the first frames play from).
    pub fn open(&mut self) -> Vec<f32> {
        let n = (0.09 * self.rate) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / self.rate;
                let click = if i < 24 { (1.0 - i as f32 / 24.0) * 0.5 * if i % 2 == 0 { 1.0 } else { -0.6 } } else { 0.0 };
                let env = (t / 0.01).min(1.0) * (1.0 - t / 0.09).max(0.0);
                click + self.hiss() * 0.3 * env
            })
            .collect()
    }

    /// The squelch closing: the "kssh" after the other side lets go of the key.
    pub fn tail(&mut self) -> Vec<f32> {
        let len = 0.22;
        let n = (len * self.rate) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / self.rate;
                let env = (t / 0.006).min(1.0) * if t > 0.15 { (1.0 - (t - 0.15) / (len - 0.15)).max(0.0) } else { 1.0 };
                (self.noise.next() * 0.55 + self.hiss() * 0.6) * 0.55 * env
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------------------
// the terminal's tones

/// The radio terminal's own tones (not through the radio's sound: the terminal plays them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// An individual call begins.
    IndividualStart,
    /// A selective or a general call begins.
    GroupStart,
    /// Any call ends.
    CallEnd,
    /// The other side (or we) let go of the key, in an individual call.
    PttRelease,
}

/// A tone's samples at the radio's 8 kHz, mono.
pub fn tone(t: Tone) -> &'static [f32] {
    static TONES: std::sync::OnceLock<[Vec<f32>; 4]> = std::sync::OnceLock::new();
    let all = TONES.get_or_init(|| {
        let read = |bytes: &[u8]| -> Vec<f32> {
            match crate::wav::parse_wav(bytes) {
                Ok(w) => w.samples.chunks(w.channels.max(1) as usize).map(|c| c.iter().map(|&s| s as f32).sum::<f32>() / (c.len() as f32 * 32768.0)).collect(),
                Err(e) => {
                    log::warn!("dispatch radio: a tone does not read: {e}");
                    Vec::new()
                }
            }
        };
        [
            read(include_bytes!("../sounds/phonie_individual_start.wav")),
            read(include_bytes!("../sounds/phonie_group_start.wav")),
            read(include_bytes!("../sounds/phonie_call_end.wav")),
            read(include_bytes!("../sounds/phonie_ptt_release.wav")),
        ]
    });
    &all[t as usize]
}

// ---------------------------------------------------------------------------------------
// the microphone

/// The microphone, open while it may be needed: what it hears, at `rate`, mono.
pub struct Mic {
    _stream: cpal::Stream,
    buf: Arc<Mutex<VecDeque<f32>>>,
    /// The device's name, for the log.
    pub name: String,
}

/// At most this much (s) waits in the microphone's buffer (an older part is dropped).
const MIC_KEEP: f32 = 1.0;

impl Mic {
    /// The system's default microphone, its sound at `rate` (Hz).
    pub fn open(rate: u32) -> Result<Mic, String> {
        let host = cpal::default_host();
        let dev = host.default_input_device().ok_or("no microphone")?;
        let name = dev.name().unwrap_or_default();
        let cfg = dev.default_input_config().map_err(|e| e.to_string())?;
        let channels = cfg.channels().max(1) as usize;
        let in_rate = cfg.sample_rate().0 as f32;
        let buf = Arc::new(Mutex::new(VecDeque::new()));
        let keep = (MIC_KEEP * rate as f32) as usize;
        // down to the radio's rate: a low-pass under its Nyquist frequency, then a sample
        // taken between two of the device's wherever the radio's next one falls
        let (mut lp1, mut lp2) = (Biquad::lowpass(in_rate, rate as f32 * 0.45, 0.7), Biquad::lowpass(in_rate, rate as f32 * 0.45, 0.7));
        let step = in_rate / rate as f32;
        let mut phase = 0.0f32;
        let mut prev = 0.0f32;
        let sink = buf.clone();
        let mut take = move |mono: &mut dyn Iterator<Item = f32>| {
            let mut out = Vec::new();
            for x in mono {
                let cur = lp2.run(lp1.run(x));
                phase += 1.0;
                while phase >= step {
                    phase -= step;
                    out.push(cur - (cur - prev) * phase.min(1.0));
                }
                prev = cur;
            }
            let mut b = sink.lock();
            b.extend(out);
            while b.len() > keep {
                b.pop_front();
            }
        };
        let err = |e: cpal::StreamError| log::warn!("microphone: {e}");
        let config: cpal::StreamConfig = cfg.clone().into();
        let stream = match cfg.sample_format() {
            cpal::SampleFormat::F32 => dev.build_input_stream(&config, move |d: &[f32], _| take(&mut d.chunks(channels).map(|c| c.iter().sum::<f32>() / channels as f32)), err, None),
            cpal::SampleFormat::I16 => dev.build_input_stream(&config, move |d: &[i16], _| take(&mut d.chunks(channels).map(|c| c.iter().map(|&s| s as f32 / 32768.0).sum::<f32>() / channels as f32)), err, None),
            cpal::SampleFormat::U16 => dev.build_input_stream(&config, move |d: &[u16], _| take(&mut d.chunks(channels).map(|c| c.iter().map(|&s| (s as f32 - 32768.0) / 32768.0).sum::<f32>() / channels as f32)), err, None),
            f => return Err(format!("the microphone's sample format {f:?} is not read")),
        }
        .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Mic { _stream: stream, buf, name })
    }

    /// `n` samples, once there are as many.
    pub fn take(&self, n: usize) -> Option<Vec<f32>> {
        let mut b = self.buf.lock();
        (b.len() >= n).then(|| b.drain(..n).collect())
    }

    /// Forget what it heard (the key was not pressed meanwhile).
    pub fn clear(&self) {
        self.buf.lock().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    fn tone(f: f32, n: usize, a: f32) -> Vec<f32> {
        (0..n).map(|i| (i as f32 * f * std::f32::consts::TAU / 8000.0).sin() * a).collect()
    }

    #[test]
    fn the_radio_keeps_the_voice_band_only() {
        // (past the first quarter second, the limiter settled)
        let level = |f: f32| {
            let mut fx = RadioFx::new(8000);
            rms(&fx.voice(&tone(f, 8000, 0.3))[2000..])
        };
        let (low, mid, high) = (level(90.0), level(1000.0), level(3900.0));
        assert!(mid > low * 2.0, "90 Hz {low} vs 1 kHz {mid}");
        assert!(mid > high * 2.0, "3.9 kHz {high} vs 1 kHz {mid}");
    }

    #[test]
    fn quiet_and_loud_come_out_alike() {
        let mut a = RadioFx::new(8000);
        let mut b = RadioFx::new(8000);
        let quiet = rms(&a.voice(&tone(800.0, 8000, 0.05))[4000..]);
        let loud = rms(&b.voice(&tone(800.0, 8000, 0.8))[4000..]);
        assert!(loud / quiet < 1.6, "quiet {quiet}, loud {loud}");
        assert!(loud < 1.0);
    }

    #[test]
    fn the_terminal_tones_are_there_at_the_radio_rate() {
        for (t, secs) in [(Tone::IndividualStart, 0.42), (Tone::GroupStart, 0.627), (Tone::CallEnd, 0.627), (Tone::PttRelease, 0.118)] {
            let s = super::tone(t);
            assert!((s.len() as f32 / 8000.0 - secs).abs() < 0.01, "{t:?}: {} samples", s.len());
            assert!(rms(s) > 0.02, "{t:?} is silent");
        }
    }

    #[test]
    fn the_squelch_sounds_are_short_and_end_quiet() {
        let mut fx = RadioFx::new(8000);
        let open = fx.open();
        let tail = fx.tail();
        assert!(open.len() < 1000 && tail.len() < 2000);
        assert!(tail.last().unwrap().abs() < 0.05);
        assert!(rms(&tail) > 0.05);
    }
}
