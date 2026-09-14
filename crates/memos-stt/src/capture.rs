//! The always-open microphone stream.
//!
//! Opened once at startup and held for the process lifetime. The hotkey never
//! touches the device — it only records a cursor into a ring buffer that is
//! already being filled.
//!
//! ## Why audio owns a thread
//!
//! `cpal::Stream` is `!Send` on Windows: a WASAPI stream belongs to the thread
//! that created it and cannot be moved or shared. So the stream is created on a
//! dedicated thread that then parks forever, keeping it alive. What the rest of
//! the application receives is the `Arc<RingBuffer>` and some device facts —
//! all `Send + Sync`, and all anyone else actually needs.
//!
//! The waiting thread is not idle bookkeeping: dropping the stream stops capture
//! silently, so it holds the stream until the `AudioCapture` itself is dropped.

use std::sync::mpsc;
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Sample, SampleFormat};

use crate::ring::{Cursor, RingBuffer, SAMPLE_RATE};

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("no input device available")]
    NoDevice,
    #[error("no output device to record from")]
    NoOutputDevice,
    #[error("device offers no usable input configuration: {0}")]
    NoConfig(String),
    #[error("could not build input stream: {0}")]
    Build(String),
    #[error("could not start input stream: {0}")]
    Start(String),
    #[error("audio thread stopped before the stream was ready")]
    ThreadDied,
}

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub name: String,
    pub input_rate: u32,
    pub channels: u16,
}

/// A live microphone feeding a ring buffer.
pub struct AudioCapture {
    ring: Arc<RingBuffer>,
    info: DeviceInfo,
    /// Dropped with the capture, which is what lets its thread let go of the
    /// stream. Never sent on.
    _stop: mpsc::Sender<()>,
}

/// Which sound a capture records.
#[derive(Clone, Copy)]
enum Source {
    Microphone,
    /// Whatever the machine is playing — the other side of a call. WASAPI
    /// records a render device in loopback when it is opened as an input, so
    /// this is the default output device with nothing else to it.
    Output,
}

impl AudioCapture {
    /// Open the default input device and start capturing immediately.
    ///
    /// Blocks only until the stream is running or has failed, so a missing
    /// microphone is reported here rather than surfacing later as silence.
    pub fn start() -> Result<Self, AudioError> {
        let capture = Self::open(Source::Microphone)?;
        tracing::info!(
            device = %capture.info.name,
            input_rate = capture.info.input_rate,
            channels = capture.info.channels,
            "microphone open — stays open for the process lifetime"
        );
        Ok(capture)
    }

    /// Record what the machine is playing, for as long as this is held.
    ///
    /// Windows only: elsewhere the system does not offer its output as an
    /// input, and this fails rather than quietly recording the microphone.
    pub fn loopback() -> Result<Self, AudioError> {
        Self::open(Source::Output)
    }

    fn open(source: Source) -> Result<Self, AudioError> {
        let ring = Arc::new(RingBuffer::new());
        let sink = ring.clone();
        let (tx, rx) = mpsc::channel::<Result<DeviceInfo, AudioError>>();
        let (stop, stopped) = mpsc::channel::<()>();

        std::thread::Builder::new()
            .name("audio-capture".into())
            .spawn(move || match open_stream(source, sink) {
                Ok((_stream, info)) => {
                    let _ = tx.send(Ok(info));
                    // `_stream` must outlive this wait: returning drops it and
                    // stops capture, with no error anywhere. The wait ends only
                    // when the `AudioCapture` is dropped and its sender with it.
                    let _ = stopped.recv();
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                }
            })
            .map_err(|e| AudioError::Build(e.to_string()))?;

        let info = rx.recv().map_err(|_| AudioError::ThreadDied)??;

        Ok(Self {
            ring,
            info,
            _stop: stop,
        })
    }

    pub fn ring(&self) -> Arc<RingBuffer> {
        self.ring.clone()
    }

    pub fn device_name(&self) -> &str {
        &self.info.name
    }

    pub fn input_rate(&self) -> u32 {
        self.info.input_rate
    }

    pub fn input_channels(&self) -> u16 {
        self.info.channels
    }

    /// Where to start reading for a capture beginning now.
    ///
    /// Deliberately rewinds a little: the user starts speaking fractionally
    /// before the chord fully registers, and the hold threshold adds more. The
    /// audio is already buffered, so including it is free — and it is the
    /// difference between "ave this to React" and "save this to React".
    pub fn capture_start(&self, lead_ms: u32) -> Cursor {
        self.ring.cursor_secs_ago(lead_ms as f32 / 1000.0)
    }

    /// Signal level over the recent window. Non-zero proves the stream is
    /// genuinely running rather than merely opened.
    pub fn level(&self) -> f32 {
        self.ring.rms(120)
    }
}

fn open_stream(
    source: Source,
    sink: Arc<RingBuffer>,
) -> Result<(cpal::Stream, DeviceInfo), AudioError> {
    let host = cpal::default_host();
    let device = match source {
        Source::Microphone => host.default_input_device().ok_or(AudioError::NoDevice)?,
        Source::Output if cfg!(windows) => host
            .default_output_device()
            .ok_or(AudioError::NoOutputDevice)?,
        Source::Output => return Err(AudioError::NoOutputDevice),
    };
    let name = device.name().unwrap_or_else(|_| "unknown".into());

    // An output device has no input formats of its own. Loopback records it in
    // the format it plays, so that is the one to ask for.
    let config = match source {
        Source::Microphone => device.default_input_config(),
        Source::Output => device.default_output_config(),
    }
    .map_err(|e| AudioError::NoConfig(e.to_string()))?;
    let input_rate = config.sample_rate().0;
    let channels = config.channels();
    let sample_format = config.sample_format();
    let stream_config: cpal::StreamConfig = config.into();

    let ch = channels as usize;
    // Resample and downmix inside the callback so the ring holds exactly what
    // whisper expects (16 kHz mono) and no consumer has to care what the
    // hardware offered.
    let ratio = SAMPLE_RATE as f64 / input_rate as f64;

    // A device-level error must not panic the audio thread — that would take
    // the microphone down for the rest of the session with no way back.
    let on_error = |e| tracing::error!(?e, "audio stream error");

    macro_rules! build {
        ($t:ty) => {
            device.build_input_stream(
                &stream_config,
                move |data: &[$t], _: &cpal::InputCallbackInfo| {
                    let frames = data.len() / ch.max(1);
                    let out_len = ((frames as f64) * ratio).ceil() as usize;
                    let mut out = Vec::with_capacity(out_len);
                    // Nearest-neighbour decimation. Adequate for speech at the
                    // 48k->16k integer ratio virtually all Windows hardware
                    // reports; revisit if a device offers an awkward rate and
                    // aliasing becomes audible.
                    for i in 0..out_len {
                        let src = ((i as f64) / ratio) as usize;
                        if src >= frames {
                            break;
                        }
                        let base = src * ch;
                        let mut acc = 0.0f32;
                        for c in 0..ch {
                            acc += data[base + c].to_float_sample();
                        }
                        out.push(acc / ch as f32);
                    }
                    sink.push(&out);
                },
                on_error,
                None,
            )
        };
    }

    let stream = match sample_format {
        SampleFormat::F32 => build!(f32),
        SampleFormat::I16 => build!(i16),
        SampleFormat::U16 => build!(u16),
        other => return Err(AudioError::NoConfig(format!("unsupported format {other:?}"))),
    }
    .map_err(|e| AudioError::Build(e.to_string()))?;

    stream.play().map_err(|e| AudioError::Start(e.to_string()))?;

    Ok((
        stream,
        DeviceInfo {
            name,
            input_rate,
            channels,
        },
    ))
}
