//! A fixed-capacity ring of audio samples, written by the audio callback and
//! read by everything else.
//!
//! The capture path's most important property lives here: the microphone stream
//! is opened at application start and **never closed**, so audio is always
//! already in memory when the shortcut is pressed. Opening a WASAPI capture
//! device costs 100-300 ms; paying that at hotkey time clips the first word and
//! makes the product feel broken in a way users cannot articulate.
//!
//! Because the buffer holds the recent past, a second capability comes free:
//! **retroactive capture** — the user can press the shortcut just *after*
//! saying something and the audio is still there.

use parking_lot::Mutex;

/// Sample rate every downstream component assumes. whisper.cpp requires 16 kHz
/// mono, so the capture stream is resampled to it once, here, rather than
/// leaving every consumer to cope with whatever the device offered.
pub const SAMPLE_RATE: u32 = 16_000;

/// Seconds of audio retained. 30 s covers a long command with room for
/// retroactive capture, and costs 16000 * 30 * 4 bytes ≈ 1.9 MB — negligible
/// beside the model footprint.
pub const RETAIN_SECS: usize = 30;

const CAPACITY: usize = SAMPLE_RATE as usize * RETAIN_SECS;

/// A monotonically increasing position in the audio stream.
///
/// Absolute rather than an index into the buffer: an index would silently
/// change meaning as the ring wraps, so a read started before a wrap would
/// return the wrong audio with no error. A cursor can be compared against what
/// is still retained and rejected honestly when it has aged out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cursor(pub u64);

impl Cursor {
    pub fn secs_since(&self, later: Cursor) -> f32 {
        later.0.saturating_sub(self.0) as f32 / SAMPLE_RATE as f32
    }
}

struct Inner {
    buf: Box<[f32]>,
    /// Total samples ever written. `written - CAPACITY` is the oldest sample
    /// still retained.
    written: u64,
}

pub struct RingBuffer {
    inner: Mutex<Inner>,
}

impl Default for RingBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl RingBuffer {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                buf: vec![0.0; CAPACITY].into_boxed_slice(),
                written: 0,
            }),
        }
    }

    /// Append samples. Called from the audio callback, so it must not allocate,
    /// block on I/O, or panic — a stalled callback produces an audible glitch
    /// and, if sustained, gets the stream torn down by the OS.
    pub fn push(&self, samples: &[f32]) {
        let mut g = self.inner.lock();
        for &s in samples {
            let idx = (g.written % CAPACITY as u64) as usize;
            g.buf[idx] = s;
            g.written += 1;
        }
    }

    /// Current write position.
    pub fn cursor(&self) -> Cursor {
        Cursor(self.inner.lock().written)
    }

    /// The oldest position still retained.
    pub fn oldest(&self) -> Cursor {
        let w = self.inner.lock().written;
        Cursor(w.saturating_sub(CAPACITY as u64))
    }

    /// Copy everything from `from` to the write head.
    ///
    /// Returns `None` if `from` has already been overwritten — better to report
    /// the gap than to hand back audio that silently starts in the wrong place.
    pub fn read_from(&self, from: Cursor) -> Option<Vec<f32>> {
        let g = self.inner.lock();
        let oldest = g.written.saturating_sub(CAPACITY as u64);
        if from.0 < oldest {
            return None;
        }
        let n = (g.written - from.0) as usize;
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let idx = ((from.0 + i as u64) % CAPACITY as u64) as usize;
            out.push(g.buf[idx]);
        }
        Some(out)
    }

    /// A cursor `secs` in the past, clamped to what is retained. This is what
    /// makes retroactive capture work: rewind before the key was pressed.
    pub fn cursor_secs_ago(&self, secs: f32) -> Cursor {
        let g = self.inner.lock();
        let back = (secs * SAMPLE_RATE as f32) as u64;
        let oldest = g.written.saturating_sub(CAPACITY as u64);
        Cursor(g.written.saturating_sub(back).max(oldest))
    }

    /// Root-mean-square level over the most recent `window_ms`, for the overlay
    /// waveform and for confirming the microphone is actually live.
    pub fn rms(&self, window_ms: u32) -> f32 {
        let g = self.inner.lock();
        let n = ((SAMPLE_RATE as u64 * window_ms as u64) / 1000)
            .min(g.written)
            .min(CAPACITY as u64) as usize;
        if n == 0 {
            return 0.0;
        }
        let start = g.written - n as u64;
        let mut sum = 0.0f64;
        for i in 0..n {
            let idx = ((start + i as u64) % CAPACITY as u64) as usize;
            let v = g.buf[idx] as f64;
            sum += v * v;
        }
        (sum / n as f64).sqrt() as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_back_what_was_written() {
        let r = RingBuffer::new();
        let start = r.cursor();
        r.push(&[0.1, 0.2, 0.3]);
        let got = r.read_from(start).unwrap();
        assert_eq!(got, vec![0.1, 0.2, 0.3]);
    }

    #[test]
    fn wraps_without_corrupting_recent_audio() {
        let r = RingBuffer::new();
        r.push(&vec![0.5; CAPACITY + 1000]);
        let recent = r.cursor_secs_ago(0.1);
        let got = r.read_from(recent).unwrap();
        assert!(got.iter().all(|&v| v == 0.5));
        assert_eq!(got.len(), (0.1 * SAMPLE_RATE as f32) as usize);
    }

    #[test]
    fn aged_out_cursor_is_refused_not_silently_wrong() {
        // The failure this guards against is subtle: without it, a stale cursor
        // returns audio starting at the wrong offset, and the transcript is
        // quietly of the wrong moment.
        let r = RingBuffer::new();
        let start = r.cursor();
        r.push(&vec![0.0; CAPACITY + 10]);
        assert!(r.read_from(start).is_none());
    }

    #[test]
    fn rms_distinguishes_silence_from_speech() {
        let r = RingBuffer::new();
        r.push(&vec![0.0; SAMPLE_RATE as usize]);
        assert!(r.rms(200) < 0.001, "silence should read near zero");

        let r2 = RingBuffer::new();
        r2.push(&vec![0.4; SAMPLE_RATE as usize]);
        assert!(r2.rms(200) > 0.3, "loud signal should read high");
    }

    #[test]
    fn retroactive_window_is_clamped_to_what_is_retained() {
        let r = RingBuffer::new();
        r.push(&vec![0.2; 1000]);
        // Asking for more history than exists must clamp rather than underflow.
        let c = r.cursor_secs_ago(120.0);
        assert_eq!(c, Cursor(0));
        assert_eq!(r.read_from(c).unwrap().len(), 1000);
    }
}
