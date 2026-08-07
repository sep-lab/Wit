//! Opt-in real-material check: generate a synthetic WAV, shell out to
//! macOS's `afconvert` to produce a CAF (raw PCM) and an ALAC-in-CAF and
//! ALAC-in-M4A fixture from it, and assert `wit_audio::decode` round-trips
//! all three **sample-exact** against the original synthetic PCM.
//!
//! Mirrors `wit-logic/tests/real_fixtures.rs`'s exact loud-skip discipline
//! (`WIT_FIXTURES` pattern, TESTING.md §7's Tier 1): **loudly skipped** by
//! default, never touches anything unless asked, and only runs at all on
//! macOS (`afconvert` is an Apple system tool with no cross-platform
//! equivalent — this is also why the M4 issue calls this leg
//! "macOS-CI-only" and loud-skips it on the Linux leg).
//!
//! ALAC is lossless, so sample-exact recovery through it is a legitimate
//! bit-exact assertion — not the "asserting a ratio on generated audio"
//! mistake AGENTS.md warns against for lossy formats like the FLAC
//! compression ratio (that number depends on program material; bit-exact
//! round-trip through a lossless codec does not).
//!
//! Run against a real macOS machine with `afconvert` on `PATH` (true by
//! default on any Mac):
//!
//! ```text
//! cargo test -p wit-audio --test real_fixtures -- --nocapture --ignored
//! ```
//!
//! Set `WIT_AUDIO_AFCONVERT=0` to force-skip even on a machine where
//! `afconvert` is present (e.g. to test the skip path itself).

use std::io::Cursor;
use std::path::PathBuf;
use std::process::Command;

fn afconvert_available() -> bool {
    if std::env::var_os("WIT_AUDIO_AFCONVERT").as_deref() == Some(std::ffi::OsStr::new("0")) {
        return false;
    }
    if !cfg!(target_os = "macos") {
        return false;
    }
    Command::new("which")
        .arg("afconvert")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Deterministic broadband PCM (see `tests/parity.rs` for why broadband,
/// not a tone) as 16-bit interleaved stereo samples, and its WAV bytes.
fn synthetic_wav(frames: usize, rate: u32, seed: u64) -> (Vec<i16>, Vec<u8>) {
    let mut state = seed.max(1);
    let mut interleaved = Vec::with_capacity(frames * 2);
    for _ in 0..frames * 2 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let unit = (state >> 11) as f64 / (1u64 << 53) as f64;
        let amp = 0.4;
        interleaved.push(((unit * 2.0 - 1.0) * amp * i16::MAX as f64) as i16);
    }

    let mut bytes = Vec::new();
    {
        let mut cursor = Cursor::new(&mut bytes);
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::new(&mut cursor, spec).unwrap();
        for &s in &interleaved {
            writer.write_sample(s).unwrap();
        }
        writer.finalize().unwrap();
    }
    (interleaved, bytes)
}

fn run_afconvert(args: &[&str]) {
    let output = Command::new("afconvert")
        .args(args)
        .output()
        .expect("afconvert failed to launch");
    assert!(
        output.status.success(),
        "afconvert {:?} failed:\nstdout: {}\nstderr: {}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// i16 reference samples, scaled the same way symphonia's PCM decoder
/// scales 16-bit ints to f32 (divide by `-i16::MIN`, i.e. 32768, not
/// 32767 — measured directly against a decoded full-scale sample: `i16::MAX`
/// decodes to `0.9999695`, `i16::MIN` decodes to exactly `-1.0`), so the
/// comparison below is sample-exact rather than approximate.
fn reference_f32(interleaved: &[i16]) -> Vec<f32> {
    const SCALE: f32 = -(i16::MIN as f32); // 32768.0
    interleaved.iter().map(|&s| s as f32 / SCALE).collect()
}

#[test]
#[ignore = "opt-in: needs macOS's afconvert on PATH; set WIT_AUDIO_AFCONVERT=0 to force-skip, \
            or run: cargo test -p wit-audio --test real_fixtures -- --nocapture --ignored"]
fn afconvert_caf_and_alac_decode_sample_exact() {
    if !afconvert_available() {
        eprintln!(
            "afconvert not available (needs macOS with afconvert on PATH, or \
             WIT_AUDIO_AFCONVERT=0 was set) — skipped. This is the check that lets the \
             product pitch 'no ffmpeg needed' stand for real CAF/ALAC files; see issue #17. \
             To run: cargo test -p wit-audio --test real_fixtures -- --nocapture --ignored \
             (macOS only)."
        );
        return;
    }

    let dir = std::env::temp_dir().join(format!("wit-audio-real-fixtures-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let rate = 44_100u32;
    let (interleaved, wav_bytes) = synthetic_wav(20_000, rate, 0xC0FFEE);
    let expected = reference_f32(&interleaved);

    let wav_path: PathBuf = dir.join("src.wav");
    std::fs::write(&wav_path, &wav_bytes).unwrap();

    let pcm_caf: PathBuf = dir.join("src_pcm.caf");
    let alac_caf: PathBuf = dir.join("src_alac.caf");
    let alac_m4a: PathBuf = dir.join("src_alac.m4a");

    run_afconvert(&[
        wav_path.to_str().unwrap(),
        "-f",
        "caff",
        "-d",
        "LEI16",
        pcm_caf.to_str().unwrap(),
    ]);
    run_afconvert(&[
        wav_path.to_str().unwrap(),
        "-f",
        "caff",
        "-d",
        "alac",
        alac_caf.to_str().unwrap(),
    ]);
    run_afconvert(&[
        wav_path.to_str().unwrap(),
        "-f",
        "m4af",
        "-d",
        "alac",
        alac_m4a.to_str().unwrap(),
    ]);

    for (label, path) in [
        ("PCM CAF", &pcm_caf),
        ("ALAC-in-CAF", &alac_caf),
        ("ALAC-in-M4A", &alac_m4a),
    ] {
        let decoded = wit_audio::decode_file(path)
            .unwrap_or_else(|e| panic!("{label} ({path:?}) failed to decode: {e}"));
        eprintln!(
            "  {label}: {} Hz, {} ch, {} samples",
            decoded.sample_rate,
            decoded.channels,
            decoded.samples.len()
        );
        assert_eq!(decoded.sample_rate, rate, "{label}: sample rate mismatch");
        assert_eq!(decoded.channels, 2, "{label}: channel count mismatch");
        assert_eq!(
            decoded.samples.len(),
            expected.len(),
            "{label}: sample count mismatch"
        );
        for (i, (&got, &want)) in decoded.samples.iter().zip(expected.iter()).enumerate() {
            assert_eq!(
                got, want,
                "{label}: sample {i} not bit-exact (got {got}, want {want})"
            );
        }
    }

    eprintln!(
        "\nall {} formats decoded sample-exact against the synthetic source",
        3
    );
    let _ = std::fs::remove_dir_all(&dir);
}
