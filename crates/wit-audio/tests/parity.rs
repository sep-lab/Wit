//! Parity with the Python prototype (issue #17): a Rust port of
//! `tests/test_null_diff.py::write_tone` — deterministic **broadband** PCM,
//! never a pure tone. That function's own docstring is the reason why: a
//! 1-sample offset on a 440 Hz sine leaves a residual 24.8 dB *below* the
//! source (a 1-sample shift is only 3.3 degrees of phase at 440 Hz), so a
//! sine cannot exercise misalignment at all. Broadband material shows it
//! immediately, which is exactly why `write_tone` there — and here — is
//! seeded white noise, not a tone despite the name.
//!
//! Exercises the exact sample-shift recovery issue #17 asks for by name:
//! **+1, -1, +4800, -12000**.

use wit_audio::align::{find_shift, MIN_CONFIDENCE};

/// Port of `tests/test_null_diff.py::write_tone`'s per-sample PRNG, minus
/// the WAV file I/O — `align::find_shift` works directly on `&[f32]`, so
/// there is nothing gained by round-tripping through bytes for the shift
/// assertion. `full_pipeline_wav_bytes_through_decode_and_align` below
/// covers the byte-stream decode path separately, so the WAV-writing
/// helper isn't skipped entirely.
fn write_tone(len: usize, amp: f32, seed: u64) -> Vec<f32> {
    let mut state = seed.max(1);
    let mut samples = Vec::with_capacity(len);
    for _ in 0..len {
        // xorshift64* — deterministic across platforms and Rust versions,
        // with no RNG crate dependency (mirrors the Python original's
        // choice of stdlib-only `random.Random`, adapted to Rust's
        // absence of a stdlib PRNG).
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let unit = (state >> 11) as f64 / (1u64 << 53) as f64;
        samples.push(amp * ((unit as f32) * 2.0 - 1.0));
    }
    samples
}

/// Builds an `(a, b)` pair from one longer broadband buffer such that, at
/// the best alignment, `a[i] ~= b[i + true_shift]` — see `wit_audio::align`'s
/// module doc ("Shift sign convention") for why that is the direction
/// [`find_shift`] is expected to recover `true_shift` in.
fn shifted_pair(true_shift: i64, len: usize) -> (Vec<f32>, Vec<f32>) {
    let margin = true_shift.unsigned_abs() as usize + 16;
    let full = write_tone(len + 2 * margin, 0.5, 0xC0FFEE);
    let base = margin as i64;
    let a: Vec<f32> = (0..len as i64).map(|i| full[(base + i) as usize]).collect();
    let b: Vec<f32> = (0..len as i64)
        .map(|i| full[(base - true_shift + i) as usize])
        .collect();
    (a, b)
}

#[test]
fn recovers_injected_sample_shifts_exactly() {
    // Issue #17, verbatim: "+1, -1, +4800, -12000".
    for &true_shift in &[1i64, -1, 4800, -12000] {
        let (a, b) = shifted_pair(true_shift, 20_000);
        let alignment = find_shift(&a, &b)
            .unwrap_or_else(|e| panic!("shift {true_shift}: find_shift refused: {e}"));
        assert_eq!(
            alignment.shift_samples, true_shift,
            "did not recover injected shift of {true_shift} samples (got {})",
            alignment.shift_samples
        );
        assert!(
            alignment.confidence >= MIN_CONFIDENCE,
            "shift {true_shift}: confidence {} is below the required {MIN_CONFIDENCE}",
            alignment.confidence
        );
    }
}

/// The one place this suite writes actual WAV bytes: proves the shift
/// recovery above still holds end to end, through `decode::decode_bytes`
/// and `Decoded::to_mono`, not just on hand-built `&[f32]` slices.
#[test]
fn full_pipeline_wav_bytes_through_decode_and_align() {
    let true_shift = 37i64;
    let (a, b) = shifted_pair(true_shift, 20_000);
    let rate = 48_000;

    let decoded_a = wit_audio::decode_bytes(write_wav_f32_mono(&a, rate), Some("wav")).unwrap();
    let decoded_b = wit_audio::decode_bytes(write_wav_f32_mono(&b, rate), Some("wav")).unwrap();
    assert_eq!(decoded_a.sample_rate, rate);
    assert_eq!(decoded_b.sample_rate, rate);

    let alignment = find_shift(&decoded_a.to_mono(), &decoded_b.to_mono()).unwrap();
    assert_eq!(alignment.shift_samples, true_shift);
    assert!(alignment.confidence >= MIN_CONFIDENCE);
}

/// Minimal hand-rolled WAV writer via `hound` (dev-dependency only,
/// Apache-2.0/MIT — see `crates/wit-audio/Cargo.toml`), used only by the
/// one test above that needs actual bytes.
fn write_wav_f32_mono(samples: &[f32], rate: u32) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut cursor = std::io::Cursor::new(&mut buf);
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: rate,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut writer = hound::WavWriter::new(&mut cursor, spec).unwrap();
        for &s in samples {
            writer.write_sample(s).unwrap();
        }
        writer.finalize().unwrap();
    }
    buf
}
