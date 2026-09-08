//! Voice activity detection.
//!
//! Energy-based rather than a neural VAD, deliberately: it runs on every audio
//! frame, costs microseconds, needs no model, and the job here is narrow —
//! decide where an utterance ends so transcription can finalise, and notice when
//! a capture contains no speech at all.
//!
//! Getting the *end* right is what matters. Cutting early truncates the last
//! word; waiting too long adds directly to the perceived latency in §4.

use crate::ring::SAMPLE_RATE;

/// Frame length for the energy calculation. 20 ms is short enough to catch a
/// gap between words and long enough not to jitter on individual glottal pulses.
pub const FRAME_MS: u32 = 20;
const FRAME_LEN: usize = (SAMPLE_RATE as usize * FRAME_MS as usize) / 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speech {
    Silence,
    Speaking,
    /// Speech was heard and has now stopped for long enough to be an ending.
    Ended,
}

pub struct Vad {
    /// Rolling estimate of the room's noise floor.
    noise_floor: f32,
    speaking: bool,
    heard_speech: bool,
    silent_frames: u32,
    /// Consecutive silent frames before an utterance counts as finished.
    hangover_frames: u32,
}

impl Default for Vad {
    fn default() -> Self {
        Self::new(500)
    }
}

impl Vad {
    /// `end_silence_ms` is the pause that ends an utterance. 500 ms tolerates a
    /// mid-sentence breath without making the user wait noticeably.
    pub fn new(end_silence_ms: u32) -> Self {
        Self {
            // Starts pessimistic and adapts down within a few frames, so the
            // first moments of a capture are not misread as speech.
            noise_floor: 0.01,
            speaking: false,
            heard_speech: false,
            silent_frames: 0,
            hangover_frames: end_silence_ms.max(FRAME_MS) / FRAME_MS,
        }
    }

    /// Feed one frame; returns the current state.
    pub fn push_frame(&mut self, frame: &[f32]) -> Speech {
        let rms = rms(frame);

        // Adapt only while quiet, and only upward slowly. Adapting during speech
        // would let a sustained voice raise the floor until it stops detecting
        // that very voice.
        if !self.speaking {
            self.noise_floor = self.noise_floor * 0.95 + rms * 0.05;
        }

        // Ratio rather than an absolute threshold: a laptop's built-in mic and a
        // USB condenser differ by an order of magnitude in level, and a fixed
        // number would work on one and fail on the other.
        let threshold = (self.noise_floor * 3.0).max(0.004);

        if rms > threshold {
            self.speaking = true;
            self.heard_speech = true;
            self.silent_frames = 0;
            Speech::Speaking
        } else if self.speaking {
            self.silent_frames += 1;
            if self.silent_frames >= self.hangover_frames {
                self.speaking = false;
                Speech::Ended
            } else {
                // Still inside the hangover — a pause, not an ending.
                Speech::Speaking
            }
        } else {
            Speech::Silence
        }
    }

    /// Feed a whole buffer.
    ///
    /// Returns `Ended` if the utterance finished anywhere within this buffer,
    /// not merely if it was still finishing on the last frame. `Ended` is an
    /// edge that fires once and then decays to `Silence`, so a caller passing
    /// audio in chunks would miss it entirely if only the final frame's state
    /// were reported — and missing it means the capture never finalises.
    pub fn push(&mut self, samples: &[f32]) -> Speech {
        let mut state = Speech::Silence;
        for chunk in samples.chunks(FRAME_LEN) {
            if chunk.len() < FRAME_LEN {
                continue;
            }
            match self.push_frame(chunk) {
                Speech::Ended => return Speech::Ended,
                s => state = s,
            }
        }
        state
    }

    /// Whether any speech was detected at all.
    ///
    /// A capture with no speech should not be sent for transcription: on the
    /// free plan that would burn one of the user's fifty weekly captures on
    /// silence, which is the kind of small unfairness people remember.
    pub fn heard_speech(&self) -> bool {
        self.heard_speech
    }

    pub fn noise_floor(&self) -> f32 {
        self.noise_floor
    }
}

fn rms(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    let sum: f64 = frame.iter().map(|&v| (v as f64) * (v as f64)).sum();
    (sum / frame.len() as f64).sqrt() as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(level: f32, count: usize) -> Vec<f32> {
        vec![level; FRAME_LEN * count]
    }

    #[test]
    fn silence_is_not_speech() {
        let mut v = Vad::default();
        assert_eq!(v.push(&frames(0.0, 10)), Speech::Silence);
        assert!(!v.heard_speech());
    }

    #[test]
    fn detects_speech_above_the_noise_floor() {
        let mut v = Vad::default();
        v.push(&frames(0.001, 20)); // settle the floor on quiet room tone
        assert_eq!(v.push(&frames(0.2, 5)), Speech::Speaking);
        assert!(v.heard_speech());
    }

    #[test]
    fn a_short_pause_does_not_end_the_utterance() {
        let mut v = Vad::new(500);
        v.push(&frames(0.001, 20));
        v.push(&frames(0.2, 5));
        // 200 ms of quiet — a breath, not an ending.
        assert_eq!(v.push(&frames(0.0, 10)), Speech::Speaking);
    }

    #[test]
    fn ending_is_reported_even_when_more_silence_follows() {
        // The edge fires mid-buffer and decays; reporting only the final frame
        // would swallow it and the capture would never finalise.
        let mut v = Vad::new(500);
        v.push(&frames(0.001, 20));
        v.push(&frames(0.2, 5));
        assert_eq!(v.push(&frames(0.0, 100)), Speech::Ended);
    }

    #[test]
    fn a_long_pause_ends_the_utterance() {
        let mut v = Vad::new(500);
        v.push(&frames(0.001, 20));
        v.push(&frames(0.2, 5));
        assert_eq!(v.push(&frames(0.0, 30)), Speech::Ended);
    }

    #[test]
    fn adapts_to_a_noisy_room() {
        // A loud room must not read as continuous speech, or the overlay would
        // never finalise.
        let mut v = Vad::default();
        assert_eq!(v.push(&frames(0.02, 200)), Speech::Silence);
        assert!(v.noise_floor() > 0.01);
    }
}
