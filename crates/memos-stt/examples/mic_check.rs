//! Proves the microphone opens and delivers real samples.
//!
//! `cargo run -p memos-stt --example mic_check`
use memos_stt::{AudioCapture, Vad};

fn main() {
    let cap = match AudioCapture::start() {
        Ok(c) => c,
        Err(e) => {
            println!("FAILED to open microphone: {e}");
            std::process::exit(1);
        }
    };
    println!(
        "device: {}\ninput:  {} Hz, {} ch  ->  resampled to 16 kHz mono",
        cap.device_name(),
        cap.input_rate(),
        cap.input_channels()
    );

    let ring = cap.ring();
    let mut vad = Vad::default();
    let start = ring.cursor();
    println!("\nlistening 4 s\n");

    for i in 0..8 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let level = cap.level();
        let bars = (level * 200.0).min(40.0) as usize;
        println!("{:>5} ms  {:<40}  rms {:.4}", (i + 1) * 500, "#".repeat(bars), level);
    }

    let audio = ring.read_from(start).unwrap_or_default();
    let state = vad.push(&audio);
    println!(
        "\ncaptured {} samples ({:.2} s)\nvad: {:?}, speech heard: {}, noise floor {:.4}",
        audio.len(),
        audio.len() as f32 / 16_000.0,
        state,
        vad.heard_speech(),
        vad.noise_floor()
    );
    let nonzero = audio.iter().filter(|v| v.abs() > 1e-6).count();
    println!(
        "non-silent samples: {nonzero} ({:.1}%)",
        100.0 * nonzero as f32 / audio.len().max(1) as f32
    );
}
