//! `i8` min/max peak-pair computation for the waveform pane.
//!
//! A waveform view does not need every sample — it needs, per pixel column,
//! "how loud did this get, in both directions" over the window of samples
//! that column represents. Reducing to one `(min, max)` pair per window and
//! quantizing each to a single byte is what makes it cheap to hold a whole
//! song's waveform in memory and cheap to redraw on scroll/zoom.

/// One reduced window: the loudest excursion in each direction, quantized
/// to `i8`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeakPair {
    pub min: i8,
    pub max: i8,
}

/// Reduce `samples` (expected in `[-1.0, 1.0]`, symphonia's own decode
/// range) to one [`PeakPair`] per `window` samples.
///
/// Quantization is symmetric: a sample is clamped to `[-1.0, 1.0]`, scaled
/// by `i8::MAX` (127) and rounded, so `1.0` maps to `127` and `-1.0` maps to
/// `-127` — not `-128`. `i8`'s range is asymmetric (`-128..=127`) but audio
/// full-scale is symmetric; mapping `-1.0` to `-128` would make a
/// full-scale-negative sample quantize to a different magnitude than a
/// full-scale-positive one, which would draw a waveform that looks
/// lopsided around its own zero line for no acoustic reason.
///
/// The last window is shorter than `window` when `samples.len()` isn't a
/// multiple of it — `min`/`max` are still computed over whatever samples
/// that window has.
///
/// Never panics: `window == 0` or `samples.is_empty()` both return an empty
/// `Vec` (there is nothing to reduce by).
pub fn peaks(samples: &[f32], window: usize) -> Vec<PeakPair> {
    if window == 0 || samples.is_empty() {
        return Vec::new();
    }
    samples
        .chunks(window)
        .map(|chunk| {
            let mut min = 1.0f32;
            let mut max = -1.0f32;
            for &s in chunk {
                let clamped = s.clamp(-1.0, 1.0);
                min = min.min(clamped);
                max = max.max(clamped);
            }
            PeakPair {
                min: quantize(min),
                max: quantize(max),
            }
        })
        .collect()
}

fn quantize(sample: f32) -> i8 {
    (sample.clamp(-1.0, 1.0) * i8::MAX as f32).round() as i8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantizes_full_scale_symmetrically() {
        assert_eq!(quantize(1.0), 127);
        assert_eq!(quantize(-1.0), -127);
        assert_eq!(quantize(0.0), 0);
    }

    #[test]
    fn known_input_produces_expected_min_max_in_one_window() {
        let samples = [0.0, 0.5, -0.5, 1.0, -1.0, 0.25];
        let result = peaks(&samples, 6);
        assert_eq!(
            result,
            vec![PeakPair {
                min: -127,
                max: 127
            }]
        );
    }

    #[test]
    fn window_splits_into_multiple_pairs() {
        let samples = [0.1, 0.2, -0.9, -0.95, 0.5, 0.6];
        let result = peaks(&samples, 3);
        assert_eq!(
            result,
            vec![
                PeakPair {
                    min: quantize(-0.9),
                    max: quantize(0.2)
                },
                PeakPair {
                    min: quantize(-0.95),
                    max: quantize(0.6)
                },
            ]
        );
    }

    #[test]
    fn last_window_may_be_shorter_than_the_rest() {
        let samples = [0.0, 0.0, 0.0, 1.0];
        let result = peaks(&samples, 3);
        assert_eq!(result.len(), 2);
        assert_eq!(result[1], PeakPair { min: 127, max: 127 });
    }

    #[test]
    fn zero_window_returns_empty_not_a_panic() {
        assert!(peaks(&[0.1, 0.2], 0).is_empty());
    }

    #[test]
    fn empty_input_returns_empty() {
        assert!(peaks(&[], 8).is_empty());
    }

    #[test]
    fn out_of_range_samples_are_clamped_not_wrapped() {
        // A decode bug or an intentionally hot bounce could hand us
        // samples outside [-1, 1]; quantization must clamp, never wrap
        // (wrapping would draw a full-scale peak as near-silence).
        let samples = [2.0, -2.0];
        let result = peaks(&samples, 2);
        assert_eq!(
            result,
            vec![PeakPair {
                min: -127,
                max: 127
            }]
        );
    }
}
