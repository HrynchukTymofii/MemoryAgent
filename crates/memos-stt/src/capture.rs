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
//! The parked thread is not idle bookkeeping: dropping the stream stops capture
//! silently, so something must hold it for as long as the process lives.

use std::sync::mpsc;
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Sample, SampleFormat};

use crate::ring::{Cursor, RingBuffer, SAMPLE_RATE};

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("no input device available")]
    NoDevice,
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
}

impl AudioCapture {
    /// Open the default input device and start capturing immediately.
    ///
    /// Blocks only until the stream is running or has failed, so a missing
    /// microphone is reported here rather than surfacing later as silence.
    pub fn start() -> Result<Self, AudioError> {
        let ring = Arc::new(RingBuffer::new());
        let sink = ring.clone();
        let (tx, rx) = mpsc::channel::<Result<DeviceInfo, AudioError>>();

        std::thread::Builder::new()
            .name("audio-capture".into())
            .spawn(move || match open_stream(sink) {
                Ok((_stream, info)) => {
                    let _ = tx.send(Ok(info));
                    // `_stream` is deliberately never dropped: returning from
                    // this thread would drop it and stop the microphone, with no
                    // error anywhere. Parking here is what keeps capture alive.
                    loop {
                        std::thread::park();
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                }
            })
            .map_err(|e| AudioError::Build(e.to_string()))?;

        let info = rx.recv().map_err(|_| AudioError::ThreadDied)??;

        tracing::info!(
            device = %info.name,
            input_rate = info.input_rate,
            channels = info.channels,
            "microphone open — stays open for the process lifetime"
        );

        Ok(Self { ring, info })
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

fn open_stream(sink: Arc<RingBuffer>) -> Result<(cpal::Stream, DeviceInfo), AudioError> {
    let host = cpal::default_host();
    let device = host.default_input_device().ok_or(AudioError::NoDevice)?;
    let name = device.name().unwrap_or_else(|_| "unknown".into());

    let config = device
        .default_input_config()
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
