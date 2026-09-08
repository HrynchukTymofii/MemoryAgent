//! Transcribe a 16 kHz mono WAV file. Exercises the same transcriber the live
//! capture path uses, without needing a microphone.
//!
//!   cargo run -p memos-stt --features whisper --example wav_check -- <file.wav>
use memos_stt::{find_model, Hints, Transcriber, WhisperTranscriber};
use std::time::Instant;

/// Minimal PCM WAV reader: walk the chunk list, take `fmt ` and `data`.
/// Deliberately hand-rolled rather than adding a dependency for one test.
fn read_wav(path: &str) -> Result<(Vec<f32>, u32, u16), String> {
    let b = std::fs::read(path).map_err(|e| e.to_string())?;
    if b.len() < 44 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".into());
    }
    let (mut rate, mut channels, mut bits) = (0u32, 0u16, 0u16);
    let mut data: Option<&[u8]> = None;
    let mut p = 12usize;
    while p + 8 <= b.len() {
        let id = &b[p..p + 4];
        let sz = u32::from_le_bytes([b[p + 4], b[p + 5], b[p + 6], b[p + 7]]) as usize;
        let body = &b[p + 8..(p + 8 + sz).min(b.len())];
        match id {
            b"fmt " if body.len() >= 16 => {
                channels = u16::from_le_bytes([body[2], body[3]]);
                rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
                bits = u16::from_le_bytes([body[14], body[15]]);
            }
            b"data" => data = Some(body),
            _ => {}
        }
        // Chunks are word-aligned; an odd size is followed by a pad byte.
        p += 8 + sz + (sz & 1);
    }
    let data = data.ok_or("no data chunk")?;
    if bits != 16 {
        return Err(format!("expected 16-bit PCM, got {bits}-bit"));
    }
    let samples = data
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
        .collect::<Vec<f32>>();
    // Downmix if needed, so the transcriber always receives mono.
    let mono = if channels > 1 {
        samples
            .chunks(channels as usize)
            .map(|f| f.iter().sum::<f32>() / channels as f32)
            .collect()
    } else {
        samples
    };
    Ok((mono, rate, channels))
}

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        println!("usage: wav_check <file.wav>");
        std::process::exit(1);
    });

    let (audio, rate, channels) = match read_wav(&path) {
        Ok(v) => v,
        Err(e) => {
            println!("could not read wav: {e}");
            std::process::exit(1);
        }
    };
    println!(
        "wav: {} - {} samples, {} Hz, {} ch ({:.2} s)",
        path,
        audio.len(),
        rate,
        channels,
        audio.len() as f32 / rate as f32
    );
    if rate != 16_000 {
        println!("WARNING: expected 16 kHz; transcription will be wrong");
    }

    let model = find_model(None, std::path::Path::new(".")).expect("no model found");
    println!("model: {}", model.display());

    let t0 = Instant::now();
    let stt = WhisperTranscriber::load(&model).expect("load model");
    println!("loaded in {} ms", t0.elapsed().as_millis());

    let hints = Hints {
        vocabulary: vec!["Flari".into(), "React".into(), "TypeScript".into()],
    };

    match stt.transcribe(&audio, &hints) {
        Ok(t) => {
            println!("\n--- transcript ---");
            println!("{}", if t.text.is_empty() { "(nothing)" } else { &t.text });
            println!("------------------");
            println!(
                "inference {} ms for {:.2} s audio = {:.1}x realtime",
                t.inference_ms,
                t.audio_secs,
                t.realtime_factor()
            );
        }
        Err(e) => println!("failed: {e}"),
    }
}
