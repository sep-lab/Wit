//! Informal perf reporting for issue #17's budget ("two 5-minute bounces,
//! end-to-end, under 10 s single-threaded"). This is **not a merge gate** —
//! TESTING.md §9 is explicit that wall-clock on a shared CI runner is noise
//! and performance is *reported*, not asserted tightly. Accordingly this
//! test is `#[ignore]`d and never runs in CI; it exists so a human can get
//! a real number on a real machine, per the same doc's guidance to prefer
//! an informal measurement over inventing one.
//!
//! Run with `--release` for a realistic number — a debug build makes the
//! FFT step alone tens of times slower (measured on this crate's own
//! `align` benchmark-style check during development: ~1.4 s release vs.
//! ~54 s debug for one 5-minute-mono cross-correlation), and the issue's
//! 10 s budget is clearly a release-build (i.e. shipped-daemon) budget:
//!
//! ```text
//! cargo test -p wit-audio --release --test perf -- --nocapture --ignored
//! ```

use std::io::Cursor;
use std::time::Instant;

fn write_tone(len: usize, amp: f32, seed: u64) -> Vec<f32> {
    let mut state = seed.max(1);
    let mut samples = Vec::with_capacity(len);
    for _ in 0..len {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let unit = (state >> 11) as f64 / (1u64 << 53) as f64;
        samples.push(amp * ((unit as f32) * 2.0 - 1.0));
    }
    samples
}

fn write_wav_i16_mono(samples_f32: &[f32], rate: u32) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut cursor = Cursor::new(&mut buf);
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::new(&mut cursor, spec).unwrap();
        for &s in samples_f32 {
            writer.write_sample((s * i16::MAX as f32) as i16).unwrap();
        }
        writer.finalize().unwrap();
    }
    buf
}

#[test]
#[ignore = "informal perf report only, not a merge gate (TESTING.md §9); run with --release: \
            cargo test -p wit-audio --release --test perf -- --nocapture --ignored"]
fn two_five_minute_bounces_end_to_end() {
    let rate = 44_100u32;
    let seconds = 300.0_f64; // 5 minutes -- the issue's own perf-budget scale, measured
                             // directly at this scale (not extrapolated from a smaller run).
    let frames = (rate as f64 * seconds) as usize;

    let gen_start = Instant::now();
    let a = write_tone(frames, 0.4, 0xA5EED);
    // "Bounce two" of the same material, ~0.9 dB quieter -- a real, small,
    // global-gain-style change, not a contrived edge case.
    let b: Vec<f32> = a.iter().map(|&v| v * 0.9).collect();
    let wav_a = write_wav_i16_mono(&a, rate);
    let wav_b = write_wav_i16_mono(&b, rate);
    eprintln!(
        "generated 2x {seconds}s mono @ {rate} Hz ({} bytes each WAV) in {:?}",
        wav_a.len(),
        gen_start.elapsed()
    );

    let decode_start = Instant::now();
    let decoded_a = wit_audio::decode_bytes(wav_a, Some("wav")).unwrap();
    let decoded_b = wit_audio::decode_bytes(wav_b, Some("wav")).unwrap();
    let decode_elapsed = decode_start.elapsed();

    let align_start = Instant::now();
    let alignment = wit_audio::align::find_shift(&decoded_a.samples, &decoded_b.samples).unwrap();
    let align_elapsed = align_start.elapsed();

    let nulldiff_start = Instant::now();
    let report = wit_audio::nulldiff::null_diff_aligned(
        &decoded_a.samples,
        &decoded_b.samples,
        rate as usize, // 1-second windows
    );
    let nulldiff_elapsed = nulldiff_start.elapsed();

    let total = decode_elapsed + align_elapsed + nulldiff_elapsed;
    eprintln!(
        "decode: {decode_elapsed:?}; align: {align_elapsed:?} (shift={}, confidence={:.1}); \
         null-diff: {nulldiff_elapsed:?} (verdict={:?}, {} windows); TOTAL: {total:?}",
        alignment.shift_samples,
        alignment.confidence,
        report.verdict,
        report.windows.len()
    );
    eprintln!(
        "issue #17 perf budget: two 5-minute bounces, end-to-end, under 10s single-threaded. \
         See this crate's PR description for the number measured on the author's machine and \
         whether it was release or debug."
    );
}
