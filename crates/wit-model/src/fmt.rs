//! Number formatting that matches Python's float behavior byte-for-byte.
//!
//! The M1 golden-parity guardrail (wit-planning/PLAN.md): Rust's `Display` for
//! `f64` prints `124` for `124.0`, and `f64::round()` is round-half-away-from-zero
//! where Python's `round()` is round-half-to-even (banker's rounding). Either
//! divergence silently breaks byte-for-byte parity with
//! `tests/test_als_golden.py`'s `GOLDEN_DIFF` literals — the spec.

/// Round to `places` decimal digits using round-half-to-even, matching
/// Python's `round(x, places)`.
///
/// Implemented via string formatting rather than scaled float arithmetic:
/// Rust's float-to-decimal formatter produces the correctly-rounded decimal
/// representation, which resolves an exact tie to the even digit — the same
/// rule Python's `round()` uses. Scaling by `10^places` first and rounding
/// the scaled value would reintroduce the divergence this function exists to
/// avoid (multiplying `0.0005` by `1000` before rounding can shift which side
/// of the tie the value lands on).
pub fn round_places(x: f64, places: usize) -> f64 {
    if !x.is_finite() {
        return x;
    }
    // parse() cannot fail on our own format! output for a finite value.
    format!("{x:.places$}").parse().unwrap()
}

/// Round to 3 decimal places — the noise gate `wit-als` applies to every
/// mixer value it reads, matching `als_semantic_diff.py`'s `_num(s, places=3)`.
pub fn round3(x: f64) -> f64 {
    round_places(x, 3)
}

/// Format a float the way Python's `str()`/`repr()` does: integral values
/// keep a trailing `.0` ("124.0", never "124"). Used only at render time —
/// values are already rounded (see [`round3`]) before they reach here.
pub fn fmt_num(x: f64) -> String {
    if x.is_finite() && x.fract() == 0.0 {
        format!("{x:.1}")
    } else {
        format!("{x}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round3_matches_the_python_corpus_from_test_als_model_py() {
        // tests/test_als_model.py::test_mixer_values_are_rounded_to_three_places
        assert_eq!(round3(1.0), 1.0);
        assert_eq!(round3(0.0), 0.0);
        assert_eq!(round3(0.7943282127), 0.794);
        assert_eq!(round3(0.5254999995), 0.525);
        assert_eq!(round3(-0.25), -0.25);
        assert_eq!(round3(0.0005), 0.001);
    }

    #[test]
    fn round_places_resolves_an_exact_tie_to_even() {
        // 0.125 is exactly representable in binary (1/8) — a genuine tie
        // between 0.12 and 0.13. Banker's rounding picks the even digit.
        assert_eq!(round_places(0.125, 2), 0.12);
        // 0.375 is exactly representable (3/8) — tie between 0.37 and 0.38;
        // 8 is even.
        assert_eq!(round_places(0.375, 2), 0.38);
    }

    #[test]
    fn fmt_num_matches_python_repr_for_integral_and_fractional_values() {
        assert_eq!(fmt_num(124.0), "124.0");
        assert_eq!(fmt_num(0.0), "0.0");
        assert_eq!(fmt_num(0.794), "0.794");
        assert_eq!(fmt_num(-0.5), "-0.5");
        assert_eq!(fmt_num(-0.15), "-0.15");
    }
}
