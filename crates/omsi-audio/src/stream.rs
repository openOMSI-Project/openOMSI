//! A buffer of sound that is filled while it plays: the decoded internet radio. A reader
//! thread pushes stereo frames at the stream's own rate; the mixer takes them at the
//! device rate (see `AudioEngine::play_stream`).

use parking_lot::{Mutex, MutexGuard};
use std::collections::VecDeque;

pub struct StreamBuf {
    inner: Mutex<StreamInner>,
    /// What the reader says about the stream: the song playing, or why it is silent.
    status: Mutex<String>,
}

pub struct StreamInner {
    /// Frames per second of what is in the buffer.
    pub rate: u32,
    frames: VecDeque<[f32; 2]>,
    /// Playing (else filling up: at the start and after the network fell behind).
    playing: bool,
    /// How much is gathered before playing starts again after a stall, in seconds: a
    /// connection that stalled once tends to stall again, so each stall asks for more.
    prebuffer: f32,
    /// A live voice (`StreamBuf::live`): the prebuffer stays as it is after a stall - a
    /// two-way radio goes quiet between two calls, it is no stall to wait longer for.
    live: bool,
    /// The voice ends (the radio was switched off or to another station).
    pub closed: bool,
}

impl StreamInner {
    /// The frame playing now and the next one, while there are both.
    pub(crate) fn pair(&mut self) -> Option<([f32; 2], [f32; 2])> {
        if !self.playing {
            if (self.frames.len() as f32) < self.rate as f32 * self.prebuffer {
                return None;
            }
            self.playing = true;
        }
        if self.frames.len() < 2 {
            self.playing = false;
            if !self.live {
                self.prebuffer = (self.prebuffer + 1.0).min(6.0);
            }
            return None;
        }
        Some((self.frames[0], self.frames[1]))
    }

    pub(crate) fn advance(&mut self) {
        self.frames.pop_front();
    }
}

impl Default for StreamBuf {
    fn default() -> Self {
        StreamBuf {
            inner: Mutex::new(StreamInner {
                rate: 44100,
                frames: VecDeque::new(),
                playing: false,
                prebuffer: 1.5,
                live: false,
                closed: false,
            }),
            status: Mutex::new(String::new()),
        }
    }
}

impl StreamBuf {
    /// A buffer for a live voice (the dispatch radio): it plays once `prebuffer` seconds
    /// are in, and again as soon after every pause.
    pub fn live(rate: u32, prebuffer: f32) -> StreamBuf {
        let b = StreamBuf::default();
        {
            let mut i = b.inner.lock();
            i.rate = rate;
            i.prebuffer = prebuffer;
            i.live = true;
        }
        b
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, StreamInner> {
        self.inner.lock()
    }

    /// Add decoded frames at `rate`. A new rate (another station, a stream that changed
    /// format) starts the buffer over.
    pub fn push(&self, rate: u32, frames: impl IntoIterator<Item = [f32; 2]>) {
        let mut b = self.inner.lock();
        if b.rate != rate {
            b.rate = rate;
            b.frames.clear();
            b.playing = false;
        }
        b.frames.extend(frames);
    }

    /// Seconds of sound waiting to be played.
    pub fn buffered(&self) -> f32 {
        let b = self.inner.lock();
        b.frames.len() as f32 / b.rate.max(1) as f32
    }

    /// Whether the mixer is playing it (not waiting for the buffer to fill).
    pub fn is_playing(&self) -> bool {
        self.inner.lock().playing
    }

    pub fn close(&self) {
        let mut b = self.inner.lock();
        b.closed = true;
        b.frames.clear();
    }

    pub fn set_status(&self, s: impl Into<String>) {
        *self.status.lock() = s.into();
    }

    pub fn status(&self) -> String {
        self.status.lock().clone()
    }

    pub fn is_closed(&self) -> bool {
        self.inner.lock().closed
    }
}
