//! The null-test diff and its verdict ladder, ported from
//! `experiments/null_diff.py` — **verbatim thresholds** (issue #17), the
//! same alignment-first doctrine, and the same relative-not-absolute
//! reasoning. Read `experiments/null_diff.py`'s module docstring and its
//! comment block starting "The verdict is driven by the RELATIVE figure"
//! before changing anything here; that file is the frozen spec (do not
//! modify it — port its logic instead, which is what this module is).
//!
//! # What a null diff answers
//!
//! Align two renders, subtract one from the other, and report where the
//! residual rises above the material's own level. That answers "what
//! changed, and how much" directly from audio — no project-file parser
//! needed, which is why this tier of Wit works even when `wit-logic` or
//! `wit-als` can't see a change at all (a fader move inside a plugin, for
//! instance). It complements the project-file diff rather than replacing
//! it: the project diff says *why* (a fader moved); the null diff says
//! *what you can hear and where in the timeline*.
//!
//! # The verdict is relative, never absolute — carried forward from Python
//!
//! An earlier version of `null_diff.py` compared the residual against a
//! fixed dBFS floor, and that was wrong in a way that mattered: **the same
//! +3 dB EQ change reported "a clear change" on material at −37 dBFS and
//! "identical, nothing audible changed" on the identical edit at −62
//! dBFS.** Both had the same residual-minus-source delta, −17 dB. Quiet
//! music is not unchanged music. So the verdict here is driven entirely by
//! `residual_db - source_db` (computed once, over the whole aligned
//! overlap) — never by comparing `residual_db` to a fixed number on its
//! own. A caller with a *measured* noise floor for their own render chain
//! (rendering the same project twice with no edits and nulling those) has
//! a stronger, honest floor available; this port does not yet expose that
//! calibration knob (`--floor` in the Python original) — see the crate doc
//! for what else this crate does not handle.
//!
//! # Alignment gates the verdict, not just improves it
//!
//! [`null_diff`] calls [`crate::align::find_shift`] internally and returns
//! [`NullDiffError::Alignment`] if that call refuses on low confidence —
//! there is no path to a verdict built on an unaligned or barely-aligned
//! pair. See `align`'s module doc for why: a one-sample misalignment can
//! null to a residual *louder* than a real edit.

use std::fmt;

use crate::align::{find_shift, AlignError};

/// The four-band verdict ladder, **verbatim** from `experiments/null_diff.py`
/// (`-80`/`-40`/`-12` dB, all against `residual_db - source_db`, never an
/// absolute figure).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// `residual_db - source_db < -80` dB, or the residual is literal
    /// silence. The renders null to silence.
    Identical,
    /// `< -40` dB. A small, localized change; the two renders are
    /// substantially the same.
    SmallLocalized,
    /// `< -12` dB. A clear change, but the material is recognizably the
    /// same performance.
    ClearButSamePerformance,
    /// Everything else: a large change, OR the files never really aligned
    /// (different arrangement, different sample-rate lineage, time
    /// stretched). Treat with suspicion, per the Python original's own
    /// wording.
    LargeOrMisaligned,
}

/// One window's residual figure, for locating *where* in the timeline a
/// change lives. `delta_db` is always computed against the same
/// file-global `source_db` (never a per-window source level) — comparing a
/// quiet window's residual to its own tiny local level would make ordinary
/// quiet passages look like large changes, exactly the failure mode the
/// module doc's EQ example warns about.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowResidual {
    pub start_sample: usize,
    pub residual_db: f64,
    pub delta_db: f64,
    pub verdict: Verdict,
}

/// A full null-diff result.
#[derive(Debug, Clone, PartialEq)]
pub struct NullDiffReport {
    /// RMS level of the (aligned, trimmed-to-overlap) reference signal, in
    /// dBFS. `f64::NEG_INFINITY` for digital silence.
    pub source_db: f64,
    /// RMS level of the residual (`a - b`) over the full aligned overlap,
    /// in dBFS.
    pub residual_db: f64,
    /// `residual_db - source_db` — the figure the verdict is actually
    /// driven by.
    pub delta_db: f64,
    /// The shift [`crate::align::find_shift`] found and trusted (`>=
    /// `[`crate::align::MIN_CONFIDENCE`]``).
    pub shift_samples: i64,
    pub alignment_confidence: f64,
    pub verdict: Verdict,
    /// Per-window residual figures across the aligned overlap, in
    /// timeline order. Empty if `window_samples == 0` or the overlap is
    /// empty.
    pub windows: Vec<WindowResidual>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NullDiffError {
    /// The two signals could not be confidently aligned — forwarded
    /// verbatim from `align::find_shift`. No verdict is computed; a
    /// low-confidence shift is not a basis for one (see `align`'s module
    /// doc).
    Alignment(AlignError),
}

impl fmt::Display for NullDiffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NullDiffError::Alignment(e) => write!(f, "cannot null-diff: {e}"),
        }
    }
}

impl std::error::Error for NullDiffError {}

/// Align `a` and `b` (via [`crate::align::find_shift`]), then compute the
/// null-diff verdict and windowed residual over their aligned overlap.
///
/// `window_samples` controls the resolution of [`NullDiffReport::windows`];
/// pass `0` to skip windowed reporting (the overall verdict is still
/// computed). `a` and `b` should be mono (or already channel-reduced) at
/// the same sample rate — like [`crate::align`], this function has no rate
/// concept of its own; a caller working from [`crate::decode::Decoded`]
/// values is responsible for checking `sample_rate` equality itself before
/// calling this (resampling to force a match would destroy the null, per
/// the Python original — see the crate doc's "what this does not handle").
pub fn null_diff(
    a: &[f32],
    b: &[f32],
    window_samples: usize,
) -> Result<NullDiffReport, NullDiffError> {
    let alignment = find_shift(a, b).map_err(NullDiffError::Alignment)?;
    let (a_aligned, b_aligned) = apply_shift(a, b, alignment.shift_samples);
    let report = null_diff_aligned(&a_aligned, &b_aligned, window_samples);
    Ok(NullDiffReport {
        shift_samples: alignment.shift_samples,
        alignment_confidence: alignment.confidence,
        ..report
    })
}

/// Compute the null-diff verdict and windowed residual directly from two
/// **already sample-aligned**, equal-rate signals, with no alignment step
/// (`shift_samples` is always `0`, `alignment_confidence` is always
/// [`f64::INFINITY`] since no search happened). `a` and `b` need not be the
/// same length; only the overlapping prefix is compared.
///
/// This is what [`null_diff`] calls after alignment, and it is exposed
/// directly for callers who already know their two signals line up sample
/// for sample (and for this crate's own verdict-ladder tests, which
/// exercise arithmetic thresholds, not alignment).
pub fn null_diff_aligned(a: &[f32], b: &[f32], window_samples: usize) -> NullDiffReport {
    let n = a.len().min(b.len());
    let a = &a[..n];
    let b = &b[..n];

    let source_db = rms_db(a);
    let residual_db = residual_rms_db(a, b);
    let delta_db = relative_delta(residual_db, source_db);
    let verdict = classify(residual_db, delta_db);

    let mut windows = Vec::new();
    if window_samples > 0 {
        for (start, (a_chunk, b_chunk)) in a
            .chunks(window_samples)
            .zip(b.chunks(window_samples))
            .enumerate()
        {
            let start_sample = start * window_samples;
            let w_residual_db = residual_rms_db(a_chunk, b_chunk);
            let w_delta_db = relative_delta(w_residual_db, source_db);
            windows.push(WindowResidual {
                start_sample,
                residual_db: w_residual_db,
                delta_db: w_delta_db,
                verdict: classify(w_residual_db, w_delta_db),
            });
        }
    }

    NullDiffReport {
        source_db,
        residual_db,
        delta_db,
        shift_samples: 0,
        alignment_confidence: f64::INFINITY,
        verdict,
        windows,
    }
}

/// `residual_db - source_db`, handling the `-inf - -inf` (both digitally
/// silent) case as `-inf` rather than `NaN` -- two silent signals are
/// identical, not an error.
fn relative_delta(residual_db: f64, source_db: f64) -> f64 {
    let delta = residual_db - source_db;
    if delta.is_nan() {
        f64::NEG_INFINITY
    } else {
        delta
    }
}

/// The verdict ladder, verbatim from `experiments/null_diff.py`'s bottom
/// `if`/`elif` chain: `-80`/`-40`/`-12` dB against `delta`, with the
/// literal-silence check on `residual_db` ORed into the first band exactly
/// as the Python does (`if res == float("-inf") or delta < -80`).
fn classify(residual_db: f64, delta_db: f64) -> Verdict {
    if residual_db == f64::NEG_INFINITY || delta_db < -80.0 {
        Verdict::Identical
    } else if delta_db < -40.0 {
        Verdict::SmallLocalized
    } else if delta_db < -12.0 {
        Verdict::ClearButSamePerformance
    } else {
        Verdict::LargeOrMisaligned
    }
}

/// RMS level in dBFS. `-inf` for a silent (all-zero) signal or an empty
/// slice, matching `20*log10(0)`.
fn rms_db(samples: &[f32]) -> f64 {
    if samples.is_empty() {
        return f64::NEG_INFINITY;
    }
    let sum_sq: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
    let rms = (sum_sq / samples.len() as f64).sqrt();
    if rms <= 0.0 {
        f64::NEG_INFINITY
    } else {
        20.0 * rms.log10()
    }
}

/// RMS level of `a - b` in dBFS, over `min(a.len(), b.len())` samples.
fn residual_rms_db(a: &[f32], b: &[f32]) -> f64 {
    let n = a.len().min(b.len());
    let residual: Vec<f32> = (0..n).map(|i| a[i] - b[i]).collect();
    rms_db(&residual)
}

/// Trim `a` and `b` to their aligned overlap per `align`'s sign convention
/// (`a[i] ~= b[i + shift]`): a positive shift trims `b`'s front by `shift`;
/// a negative shift trims `a`'s front by `-shift`. Mirrors
/// `null_diff.py::residual_db`'s `delay_a`/`delay_b` computation, adapted
/// from an `ffmpeg atrim` filtergraph to direct slicing.
fn apply_shift(a: &[f32], b: &[f32], shift: i64) -> (Vec<f32>, Vec<f32>) {
    let trim_b = shift.max(0) as usize;
    let trim_a = (-shift).max(0) as usize;
    let a_trimmed = a.get(trim_a..).unwrap_or(&[]);
    let b_trimmed = b.get(trim_b..).unwrap_or(&[]);
    (a_trimmed.to_vec(), b_trimmed.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Seeded xorshift white noise -- see `align`'s tests and
    /// `tests/parity.rs` for why broadband, not a pure tone.
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

    /// Engineers an exact `delta_db` by scaling: `b = (1 - c) * a`, so
    /// `residual = a - b = c * a` and `residual_rms = c * rms(a)` *exactly*
    /// -- no independent randomness needed, so `delta_db == 20*log10(c)` to
    /// floating-point precision. This models the running example from
    /// AGENTS.md (a global gain change) and lands the pair in a chosen
    /// verdict band deterministically, per TESTING.md §2's carve-out for
    /// synthetic assertions of a mechanism/arithmetic property.
    fn scaled_pair(len: usize, delta_db_target: f64) -> (Vec<f32>, Vec<f32>) {
        let a = broadband(len, 0xA11A5E);
        let c = 10f32.powf((delta_db_target / 20.0) as f32);
        let b: Vec<f32> = a.iter().map(|&v| (1.0 - c) * v).collect();
        (a, b)
    }

    #[test]
    fn bit_identical_signals_are_identical() {
        let a = broadband(2000, 1);
        let report = null_diff_aligned(&a, &a, 0);
        assert_eq!(report.residual_db, f64::NEG_INFINITY);
        assert_eq!(report.verdict, Verdict::Identical);
    }

    #[test]
    fn verdict_band_small_localized() {
        // target delta ~ -50 dB, inside (-80, -40)
        let (a, b) = scaled_pair(4000, -50.0);
        let report = null_diff_aligned(&a, &b, 0);
        assert_eq!(
            report.verdict,
            Verdict::SmallLocalized,
            "delta was {}",
            report.delta_db
        );
        assert!(report.delta_db < -40.0 && report.delta_db >= -80.0);
    }

    #[test]
    fn verdict_band_clear_but_same_performance() {
        // target delta ~ -20 dB, inside (-40, -12)
        let (a, b) = scaled_pair(4000, -20.0);
        let report = null_diff_aligned(&a, &b, 0);
        assert_eq!(
            report.verdict,
            Verdict::ClearButSamePerformance,
            "delta was {}",
            report.delta_db
        );
        assert!(report.delta_db < -12.0 && report.delta_db >= -40.0);
    }

    #[test]
    fn verdict_band_large_or_misaligned() {
        // target delta ~ -6 dB, >= -12
        let (a, b) = scaled_pair(4000, -6.0);
        let report = null_diff_aligned(&a, &b, 0);
        assert_eq!(
            report.verdict,
            Verdict::LargeOrMisaligned,
            "delta was {}",
            report.delta_db
        );
        assert!(report.delta_db >= -12.0);
    }

    #[test]
    fn the_same_delta_is_identical_regardless_of_program_level() {
        // Direct regression for the module doc's ported EQ example: the
        // SAME delta must give the SAME verdict whether the material is
        // loud or quiet -- the bug in the earlier fixed-floor version of
        // null_diff.py that this crate must not repeat.
        let (loud_a, loud_b) = scaled_pair(4000, -20.0);
        let quiet_a: Vec<f32> = loud_a.iter().map(|&v| v * 0.05).collect();
        let quiet_b: Vec<f32> = loud_b.iter().map(|&v| v * 0.05).collect();

        let loud_report = null_diff_aligned(&loud_a, &loud_b, 0);
        let quiet_report = null_diff_aligned(&quiet_a, &quiet_b, 0);

        assert_eq!(loud_report.verdict, quiet_report.verdict);
        assert!((loud_report.delta_db - quiet_report.delta_db).abs() < 0.01);
        // But the absolute source level is very different -- that's the point.
        assert!(loud_report.source_db - quiet_report.source_db > 20.0);
    }

    #[test]
    fn windowed_reports_one_entry_per_window_in_timeline_order() {
        let (a, b) = scaled_pair(1000, -20.0);
        let report = null_diff_aligned(&a, &b, 300);
        assert_eq!(report.windows.len(), 4); // 300,300,300,100
        assert_eq!(report.windows[0].start_sample, 0);
        assert_eq!(report.windows[1].start_sample, 300);
        assert_eq!(report.windows[2].start_sample, 600);
        assert_eq!(report.windows[3].start_sample, 900);
    }

    #[test]
    fn full_pipeline_aligns_then_verdicts_a_shifted_pair() {
        let full = broadband(20_000, 7);
        let a = full[5000..15000].to_vec();
        // b is a's content shifted by +3 samples of extra head start, then
        // globally scaled down enough to land a clear-but-same-performance
        // delta once aligned.
        let shifted: Vec<f32> = full[4997..14997].iter().map(|&v| v * 0.9).collect();
        let report = null_diff(&a, &shifted, 0).unwrap();
        assert_eq!(report.shift_samples, 3);
        assert!(report.alignment_confidence >= crate::align::MIN_CONFIDENCE);
        // -0.9 dB-ish global scale-down: a clear but recognizable change.
        assert_eq!(report.verdict, Verdict::ClearButSamePerformance);
    }
}
