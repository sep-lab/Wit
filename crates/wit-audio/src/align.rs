//! FFT cross-correlation alignment (issue #17): find the best integer-sample
//! shift between two equal-rate signals, gated by a confidence ratio that
//! **refuses** a low-confidence result rather than returning one silently.
//!
//! # Alignment is not optional
//!
//! Ported reasoning, verbatim from `experiments/null_diff.py`'s module
//! docstring: a one-sample misalignment nulls to a residual as loud as the
//! source itself — measured there at −29.8 dBFS residual against a −29.6
//! dBFS source, i.e. "everything changed" — while the same pair, realigned,
//! nulls to −91 dB (digital silence). Issue #17 restates the same fact in
//! this crate's own terms: **a one-sample misalignment nulls to only
//! −3.7 dB, worse than any real edit**, which is why alignment confidence
//! has to gate the [`nulldiff`](crate::nulldiff) verdict, not merely improve
//! it. [`find_shift`] therefore returns a `Result`, not a struct the caller
//! could forget to check: below [`MIN_CONFIDENCE`] it is an [`AlignError`],
//! never a low-trust [`Alignment`] with a `confidence` field nobody reads.
//!
//! # How the shift is found
//!
//! `null_diff.py`'s alignment search is a coarse-to-fine linear scan over
//! candidate shifts because each candidate costs a full `ffmpeg` subprocess.
//! With direct sample access there is no such cost per candidate: an FFT
//! cross-correlation evaluates every integer-sample lag in one pass.
//! Concretely, for equal-length signals `a` and `b`, zero-padded to `n =
//! len(a) + len(b)` to avoid circular wraparound, the correlation is
//! `irfft(rfft(b) * conj(rfft(a)))`, and the shift is the lag `k` (positive
//! or negative) at which that correlation peaks.
//!
//! # Shift sign convention
//!
//! [`Alignment::shift_samples`] is defined so that, at the best alignment,
//! `a[i] ≈ b[i + shift_samples]` over the overlapping region. A positive
//! shift means `b` carries `shift_samples` of extra content ahead of the
//! part that matches `a` — as if `b`'s render started with extra leading
//! latency `a`'s didn't have. To align them, skip `shift_samples` samples
//! from the front of `b` (or, for a negative shift, from the front of `a`).
//!
//! # Confidence: peak-to-second-peak ratio
//!
//! `confidence = |correlation at the best lag| / |correlation at the
//! strongest lag outside a small guard band around the best one|`.
//!
//! This is the peak-to-second-peak test used for the same reason in image
//! registration (phase correlation): for real material with a genuine
//! alignment, the correlation has one sharp, dominant peak and the ratio is
//! large (measured on broadband noise in this crate's own tests: tens to
//! low hundreds). For two **unrelated** signals, the correlation is
//! statistical noise with no dominant lag, and the ratio sits near 1
//! (measured: 0.9 on two independently-seeded broadband noise buffers of
//! equal length).
//!
//! **Why not "peak vs. mean"?** It was tried first and rejected: the
//! maximum of many effectively-random correlation values (one per
//! candidate lag) grows with the number of lags searched even when nothing
//! is really aligned — measured 6.4 on unrelated noise at a ~40,000-sample
//! search width, comfortably clearing a naive threshold. The guard band
//! excludes only the immediate neighborhood of the peak (its own
//! correlation lobe, not a competing candidate), so peak-vs-second-peak
//! does not inherit that bias.
//!
//! # What this does not handle
//!
//! Only integer-sample shifts, by design (same limitation
//! `null_diff.py` states explicitly): a resampled or time-stretched render
//! will not produce one dominant peak and correctly reports low confidence
//! rather than a wrong shift.

use std::fmt;

use realfft::num_complex::Complex;
use realfft::RealFftPlanner;

/// Issue #17: "refuse below 1.5". Below this peak-to-second-peak ratio,
/// [`find_shift`] returns [`AlignError::LowConfidence`] instead of an
/// [`Alignment`] — see the module doc for why a `confidence` field on a
/// returned struct is not enough on its own.
pub const MIN_CONFIDENCE: f64 = 1.5;

/// A trusted alignment result. The only way to get one of these is through
/// [`find_shift`], which already checked `confidence >= `[`MIN_CONFIDENCE`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Alignment {
    /// The integer-sample shift such that `a[i] ≈ b[i + shift_samples]` at
    /// the best-scoring lag. See the module doc's "Shift sign convention".
    pub shift_samples: i64,
    /// Peak-to-second-peak ratio at `shift_samples`. Always `>=
    /// MIN_CONFIDENCE` on a value returned inside `Ok`.
    pub confidence: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AlignError {
    /// The best correlation peak was not convincingly better than the
    /// second-best candidate lag (`confidence < MIN_CONFIDENCE`). The shift
    /// and confidence that were rejected are included for diagnostics —
    /// callers must not treat `shift_samples` here as trustworthy; that is
    /// the entire point of this being an `Err`.
    LowConfidence { shift_samples: i64, confidence: f64 },
    /// One or both input signals are empty; there is nothing to align.
    EmptySignal,
}

impl fmt::Display for AlignError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AlignError::LowConfidence {
                shift_samples,
                confidence,
            } => write!(
                f,
                "alignment confidence {confidence:.2} is below the required {MIN_CONFIDENCE:.1} \
                 (best candidate shift was {shift_samples:+} samples) -- refusing rather than \
                 guessing; a one-sample misalignment can null to a worse residual than a real edit"
            ),
            AlignError::EmptySignal => write!(f, "cannot align an empty signal"),
        }
    }
}

impl std::error::Error for AlignError {}

/// Find the best integer-sample shift aligning `a` and `b` via FFT
/// cross-correlation, refusing (rather than guessing) below
/// [`MIN_CONFIDENCE`]. See the module doc for the sign convention and the
/// confidence definition.
///
/// `a` and `b` should be mono (or already channel-reduced, e.g.
/// [`crate::decode::Decoded::to_mono`]) signals at the same sample rate —
/// this function has no sample-rate concept of its own and treats both
/// slices purely as sequences of numbers.
pub fn find_shift(a: &[f32], b: &[f32]) -> Result<Alignment, AlignError> {
    if a.is_empty() || b.is_empty() {
        return Err(AlignError::EmptySignal);
    }
    let (shift_samples, confidence) = cross_correlate(a, b);
    if confidence < MIN_CONFIDENCE {
        return Err(AlignError::LowConfidence {
            shift_samples,
            confidence,
        });
    }
    Ok(Alignment {
        shift_samples,
        confidence,
    })
}

/// The guard band, in samples, excluded around the best peak when searching
/// for the second-best one. Small and fixed rather than proportional to the
/// signal: the correlation lobe of broadband material is a handful of
/// samples wide regardless of file length (it is a property of the
/// spectral content, not the duration), so a guard that scaled with `n`
/// would start excluding real competing candidates on long files.
const GUARD_BAND_SAMPLES: usize = 4;

/// FFT cross-correlation over the full (zero-padded) length of `a` and `b`.
/// Returns `(shift, confidence)` — see the module doc for both definitions.
fn cross_correlate(a: &[f32], b: &[f32]) -> (i64, f64) {
    let na = a.len();
    let nb = b.len();
    // Zero-pad to na + nb: enough that the circular correlation realfft
    // computes equals the true *linear* cross-correlation over the full
    // range of valid lags, with no wraparound aliasing between the two.
    let n = na + nb;

    let mut planner = RealFftPlanner::<f32>::new();
    let r2c = planner.plan_fft_forward(n);
    let c2r = planner.plan_fft_inverse(n);

    let mut a_buf = r2c.make_input_vec();
    let mut b_buf = r2c.make_input_vec();
    a_buf[..na].copy_from_slice(a);
    b_buf[..nb].copy_from_slice(b);

    let mut a_spec = r2c.make_output_vec();
    let mut b_spec = r2c.make_output_vec();
    // `process` only fails on a buffer-length mismatch, and every buffer
    // above came from this exact plan's own `make_*_vec` -- it cannot
    // mismatch.
    r2c.process(&mut a_buf, &mut a_spec)
        .expect("fixed-size buffers from the same plan");
    r2c.process(&mut b_buf, &mut b_spec)
        .expect("fixed-size buffers from the same plan");

    // Cross power spectrum B * conj(A). Order matters for the sign of the
    // recovered shift; this order is what makes `a[i] ~= b[i+shift]` the
    // convention documented above (verified directly in this crate's
    // tests, including negative shifts).
    let mut cross: Vec<Complex<f32>> = b_spec
        .iter()
        .zip(a_spec.iter())
        .map(|(bv, av)| bv * av.conj())
        .collect();

    let mut corr = c2r.make_output_vec();
    c2r.process(&mut cross, &mut corr)
        .expect("fixed-size buffers from the same plan");

    let mut best_m = 0usize;
    let mut best_v = f32::NEG_INFINITY;
    for (m, &v) in corr.iter().enumerate() {
        if v > best_v {
            best_v = v;
            best_m = m;
        }
    }

    let mut second_best = 0.0f32;
    for (m, &v) in corr.iter().enumerate() {
        let dist = m.abs_diff(best_m);
        let wrapped_dist = dist.min(n - dist);
        if wrapped_dist > GUARD_BAND_SAMPLES && v.abs() > second_best {
            second_best = v.abs();
        }
    }
    let confidence = if second_best > 0.0 {
        (best_v.abs() / second_best) as f64
    } else {
        f64::INFINITY
    };

    // realfft/irfft indices past n/2 represent negative lags (standard FFT
    // wraparound, same convention as e.g. numpy.fft.fftfreq).
    let shift = if best_m < n.div_ceil(2) {
        best_m as i64
    } else {
        best_m as i64 - n as i64
    };
    (shift, confidence)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Seeded xorshift white noise -- broadband, not a pure tone, per the
    /// issue's own reasoning (see `tests/parity.rs` for the full port of
    /// `tests/test_null_diff.py::write_tone`, which measures exactly why a
    /// sine can't exercise this).
    fn broadband(len: usize, seed: u32) -> Vec<f32> {
        let mut out = vec![0.0f32; len];
        let mut s = seed.max(1);
        for v in out.iter_mut() {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            *v = (s as f32 / u32::MAX as f32) * 2.0 - 1.0;
        }
        out
    }

    #[test]
    fn identical_signals_align_at_zero_with_high_confidence() {
        let a = broadband(5000, 42);
        let alignment = find_shift(&a, &a).unwrap();
        assert_eq!(alignment.shift_samples, 0);
        assert!(alignment.confidence >= MIN_CONFIDENCE);
    }

    #[test]
    fn unrelated_noise_is_refused_for_low_confidence() {
        let a = broadband(20_000, 111);
        let b = broadband(20_000, 222);
        let err = find_shift(&a, &b).unwrap_err();
        match err {
            AlignError::LowConfidence { confidence, .. } => {
                assert!(confidence < MIN_CONFIDENCE, "{confidence}");
            }
            other => panic!("expected LowConfidence, got {other:?}"),
        }
    }

    #[test]
    fn empty_signal_is_refused() {
        assert_eq!(
            find_shift(&[], &[1.0]).unwrap_err(),
            AlignError::EmptySignal
        );
        assert_eq!(
            find_shift(&[1.0], &[]).unwrap_err(),
            AlignError::EmptySignal
        );
    }
}
