//! Record from the microphone and transcribe, end to end.
//!
//!   cargo run -p memos-stt --features whisper --example transcribe_check
use memos_stt::{find_model, AudioCapture, Hints, Transcriber, Vad, WhisperTranscriber};
use std::time::Instant;

fn main() {
    let model = match find_model(None, std::path::Path::new(".")) {
        Some(m) => m,
        None => {
            println!("No model found. Run: scripts/fetch-models.ps1 base.en");
            std::process::exit(1);
        }
    };
    println!("model: {}", model.display());

    let t0 = Instant::now();
    let stt = match WhisperTranscriber::load(&model) {
        Ok(s) => s,
        Err(e) => {
            println!("load failed: {e}");
            std::process::exit(1);
        }
    };
    println!("loaded in {} ms\n", t0.elapsed().as_millis());

    let cap = AudioCapture::start().expect("microphone");
    println!("device: {}", cap.device_name());

    let secs: u64 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(5);
    println!("\nSPEAK NOW - recording {secs} s\n");

    let ring = cap.ring();
    let start = ring.cursor();
    for i in 0..secs {
        std::thread::sleep(std::time::Duration::from_secs(1));
        let bars = (cap.level() * 200.0).min(40.0) as usize;
        println!("{:>2}s  {:<40} rms {:.4}", i + 1, "#".repeat(bars), cap.level());
    }

    let audio = ring.read_from(start).unwrap_or_default();
    let mut vad = Vad::default();
    vad.push(&audio);
    println!(
        "\ncaptured {:.2} s, speech detected: {}",
        audio.len() as f32 / 16_000.0,
        vad.heard_speech()
    );

    // The vocabulary bias that makes proper nouns survive.
    let hints = Hints {
        vocabulary: vec![
            "Flari".into(), "React".into(), "TypeScript".into(),
            "Programming".into(), "Tymofii".into(),
        ],
    };

    match stt.transcribe(&audio, &hints) {
        Ok(t) => {
            println!("\n--- transcript ---\n{}\n------------------", if t.text.is_empty() { "(nothing)" } else { &t.text });
            println!(
                "inference {} ms for {:.2} s of audio = {:.1}x realtime",
                t.inference_ms,
                t.audio_secs,
                t.realtime_factor()
            );
        }
        Err(e) => println!("transcription failed: {e}"),
    }
}
