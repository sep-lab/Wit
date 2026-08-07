//! Audio engine: decode, peaks, alignment, null-diff (M4, issue #17).
//!
//! This is the tier of Wit that works even when the project-file parser is
//! blind — a fader move or a knob turn inside a plugin never touches the
//! Ableton XML or the Logic `ProjectData` container in a way either parser
//! can see (`wit-als`, `wit-logic`). Given two renders, this crate answers
//! "what changed, and how much" directly from the audio, independent of
//! which DAW produced it.
//!
//! # What this crate does
//!
//! - [`decode`]: turn a WAV/AIFF/CAF/ALAC/FLAC/MP4(AAC)/MP3 file into decoded
//!   samples via [`symphonia`], with a typed error instead of a panic on
//!   anything malformed or unsupported.
//! - [`peaks`]: reduce a signal to i8 min/max peak pairs over a fixed window,
//!   for drawing a waveform without holding every sample in a UI buffer.
//! - [`align`]: find the best integer-sample shift between two signals via
//!   FFT cross-correlation, and refuse to report one it isn't confident in.
//! - [`nulldiff`]: the null-test verdict ladder from
//!   `experiments/null_diff.py`, ported verbatim (same four bands, same
//!   thresholds, same relative-not-absolute reasoning).
//!
//! # What this crate does NOT handle
//!
//! - **Resampling.** Two signals at different sample rates are rejected, not
//!   resampled — resampling would itself destroy the null the diff depends
//!   on (see `nulldiff`'s module doc, ported from the Python original).
//! - **Sub-sample / fractional-sample alignment.** [`align`] only recovers
//!   integer-sample shifts. A time-stretched or pitch-shifted render will not
//!   align and will report a large, honest diff rather than a wrong one.
//! - **Lossy-codec perceptual comparison.** Decoding an MP3 and nulling it
//!   against a WAV will show the codec's own quantization noise as "change" —
//!   that is correct, not a bug, but it is worth knowing going in.
//! - **Multi-channel-aware alignment beyond mono reduction.** [`align`] and
//!   [`nulldiff`] work on the channel-reduced (or single-channel) signal the
//!   caller hands them; they do not attempt per-channel independent shift
//!   detection.
//! - **Realtime / streaming decode.** Every entry point here decodes a whole
//!   file into memory. Fine for the bounces this is built for (order of
//!   minutes); not designed for hours-long material.

pub mod align;
pub mod decode;
pub mod nulldiff;
pub mod peaks;

pub use align::{find_shift, AlignError, Alignment};
pub use decode::{decode_bytes, decode_file, DecodeError, Decoded};
pub use nulldiff::{null_diff, NullDiffError, Verdict};
pub use peaks::{peaks, PeakPair};
