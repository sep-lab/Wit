//! `symphonia`-based decode: turn a WAV/AIFF/CAF/ALAC/FLAC/MP4(AAC)/MP3 file
//! into `f32` samples, a sample rate, and a channel count.
//!
//! Every entry point here returns a [`Result`] — never a panic — on
//! malformed or unsupported input. That is the issue #17 requirement, and
//! it is also the general rule this workspace holds parsers to (see
//! `wit-logic`'s `frame.rs`): a background daemon must not go down because
//! one file in a library is truncated or is a format nobody enabled.
//!
//! # Format coverage
//!
//! `wav`, `aiff`, `caf`, `alac`, `flac`, `isomp4` (the MP4/M4A container,
//! covering `aac`), and `mp3` — exactly the issue #17 list, via symphonia's
//! per-format feature flags (`Cargo.toml`). Nothing else is compiled in;
//! anything else probes as [`DecodeError::UnsupportedFormat`], carrying the
//! issue's required hint verbatim (["bounce to WAV/AIFF and
//! retry"](UNSUPPORTED_FORMAT_HINT)).

use std::fmt;
use std::io::Cursor;
use std::path::Path;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// The exact hint issue #17 requires on every unsupported/unrecognized
/// format error.
pub const UNSUPPORTED_FORMAT_HINT: &str = "bounce to WAV/AIFF and retry";

/// Every way decoding can fail, all of them [`Result`]s rather than panics.
#[derive(Debug, Clone, PartialEq)]
pub enum DecodeError {
    /// symphonia's probe could not identify a container/codec it was built
    /// with support for. This covers both "genuinely unsupported format"
    /// and "not audio at all" — probing cannot always tell those apart, and
    /// the user-facing action is the same either way.
    UnsupportedFormat(String),
    /// The container was recognized, but decoding it failed: truncated
    /// data, a corrupt header, or a declared size that doesn't fit what
    /// remains of the stream. Never a panic or an unbounded allocation —
    /// symphonia's own decoders return `Result` for exactly this reason,
    /// and this variant just carries that message through.
    Malformed(String),
    /// The container was recognized and readable, but it has no audio
    /// track to decode (e.g. a video-only MP4).
    NoAudioTrack,
    /// Reading the underlying bytes failed (file missing, permissions,
    /// ...) — not a format problem.
    Io(String),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::UnsupportedFormat(detail) => write!(
                f,
                "unsupported or unrecognized audio format ({detail}) -- {UNSUPPORTED_FORMAT_HINT}"
            ),
            DecodeError::Malformed(detail) => write!(f, "malformed audio data: {detail}"),
            DecodeError::NoAudioTrack => {
                write!(
                    f,
                    "no audio track found in this file -- {UNSUPPORTED_FORMAT_HINT}"
                )
            }
            DecodeError::Io(msg) => write!(f, "failed to read audio file: {msg}"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// A fully decoded audio buffer, entirely in memory (see the crate doc's
/// "what this does not handle" — no streaming decode).
#[derive(Debug, Clone, PartialEq)]
pub struct Decoded {
    pub sample_rate: u32,
    pub channels: u16,
    /// Interleaved samples: `[L0, R0, L1, R1, ...]` for stereo, `[S0, S1,
    /// ...]` for mono. Interleaved, not `Vec<Vec<f32>>` per channel,
    /// because that is symphonia's own native decode layout
    /// (`SampleBuffer::copy_interleaved_ref`) — decoding does no extra
    /// shuffling. Callers that want mono for [`crate::align`] or
    /// [`crate::nulldiff`] call [`Decoded::to_mono`].
    pub samples: Vec<f32>,
}

impl Decoded {
    /// Reduce to a single mono channel by averaging every channel in each
    /// sample frame. This is the obvious default reduction for alignment
    /// and null-diff, which have no notion of "channel" of their own;
    /// anything more specific (e.g. left-only) is the caller's job.
    pub fn to_mono(&self) -> Vec<f32> {
        if self.channels <= 1 {
            return self.samples.clone();
        }
        let ch = self.channels as usize;
        self.samples
            .chunks(ch)
            .map(|frame| frame.iter().sum::<f32>() / ch as f32)
            .collect()
    }
}

/// Decode an audio file from disk. The extension is passed to symphonia's
/// probe as a hint (faster demuxer selection, and disambiguates formats
/// that share a magic-byte prefix) — probing still inspects the actual
/// bytes, so a wrong extension does not cause a wrong decode, at worst a
/// slower correct one.
pub fn decode_file(path: &Path) -> Result<Decoded, DecodeError> {
    let bytes = std::fs::read(path).map_err(|e| DecodeError::Io(e.to_string()))?;
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_owned);
    decode_bytes(bytes, ext.as_deref())
}

/// Decode audio from an in-memory byte buffer. See [`decode_file`] for what
/// `ext_hint` is for.
pub fn decode_bytes(bytes: Vec<u8>, ext_hint: Option<&str>) -> Result<Decoded, DecodeError> {
    let cursor = Cursor::new(bytes);
    let mss = MediaSourceStream::new(Box::new(cursor), MediaSourceStreamOptions::default());

    let mut hint = Hint::new();
    if let Some(ext) = ext_hint {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(probe_error)?;

    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .cloned()
        .ok_or(DecodeError::NoAudioTrack)?;
    let track_id = track.id;

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(probe_error)?;

    let mut samples = Vec::new();
    let mut sample_rate = 0u32;
    let mut channels = 0u16;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break
            }
            // The demuxer is telling us it needs to be reset to continue
            // (e.g. a mid-stream format change); we decode whole files up
            // front with no notion of "continuing", so this is end of
            // stream for our purposes, not an error to surface.
            Err(SymphoniaError::ResetRequired) => break,
            Err(e) => return Err(DecodeError::Malformed(e.to_string())),
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(audio_buf) => {
                let spec = *audio_buf.spec();
                sample_rate = spec.rate;
                channels = spec.channels.count() as u16;
                let mut sample_buf = SampleBuffer::<f32>::new(audio_buf.capacity() as u64, spec);
                sample_buf.copy_interleaved_ref(audio_buf);
                samples.extend_from_slice(sample_buf.samples());
            }
            // A single corrupt packet does not have to end the whole
            // decode -- skip it and keep going. This mirrors symphonia's
            // own documented usage (a player skips a bad packet rather
            // than stopping playback); if every packet in the file is bad,
            // `samples` simply stays empty.
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break
            }
            Err(e) => return Err(DecodeError::Malformed(e.to_string())),
        }
    }

    if channels == 0 {
        // Never decoded a single packet successfully -- from this
        // function's point of view, indistinguishable from "no usable
        // audio track", which is the honest thing to report.
        return Err(DecodeError::NoAudioTrack);
    }

    Ok(Decoded {
        sample_rate,
        channels,
        samples,
    })
}

fn probe_error(e: SymphoniaError) -> DecodeError {
    match e {
        SymphoniaError::Unsupported(msg) => DecodeError::UnsupportedFormat(msg.to_string()),
        SymphoniaError::IoError(io_err) => DecodeError::Io(io_err.to_string()),
        other => DecodeError::UnsupportedFormat(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_wav(samples_per_channel: &[i16], channels: u16, rate: u32) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut cursor = Cursor::new(&mut buf);
            let spec = hound::WavSpec {
                channels,
                sample_rate: rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let mut writer = hound::WavWriter::new(&mut cursor, spec).unwrap();
            for &s in samples_per_channel {
                writer.write_sample(s).unwrap();
            }
            writer.finalize().unwrap();
        }
        buf
    }

    #[test]
    fn decodes_a_simple_wav_and_reports_rate_and_channels() {
        let samples = [100i16, -100, 200, -200, 300, -300];
        let bytes = write_wav(&samples, 2, 44_100);
        let decoded = decode_bytes(bytes, Some("wav")).unwrap();
        assert_eq!(decoded.sample_rate, 44_100);
        assert_eq!(decoded.channels, 2);
        assert_eq!(decoded.samples.len(), samples.len());
    }

    #[test]
    fn mono_reduction_averages_channels() {
        let decoded = Decoded {
            sample_rate: 44_100,
            channels: 2,
            samples: vec![1.0, -1.0, 0.5, 0.5],
        };
        assert_eq!(decoded.to_mono(), vec![0.0, 0.5]);
    }

    #[test]
    fn mono_input_is_unchanged_by_to_mono() {
        let decoded = Decoded {
            sample_rate: 44_100,
            channels: 1,
            samples: vec![0.1, 0.2, 0.3],
        };
        assert_eq!(decoded.to_mono(), decoded.samples);
    }

    #[test]
    fn garbage_bytes_are_an_unsupported_format_error_not_a_panic() {
        // Plain text, not a byte pattern that could be mistaken for the
        // start of any enabled container's magic bytes or sync word.
        let bytes = b"this is definitely not an audio file, just text".to_vec();
        let err = decode_bytes(bytes, None).unwrap_err();
        match err {
            DecodeError::UnsupportedFormat(_) => {}
            other => panic!("expected UnsupportedFormat, got {other:?}"),
        }
        assert!(err.to_string().contains(UNSUPPORTED_FORMAT_HINT));
    }

    #[test]
    fn truncated_wav_header_is_a_typed_error_not_a_panic() {
        let full = write_wav(&[1, 2, 3, 4], 1, 44_100);
        let truncated = full[..20].to_vec();
        let err = decode_bytes(truncated, Some("wav")).unwrap_err();
        // Either framing is invalid (Unsupported: the probe never
        // recognizes it) or it's readable but the data runs out
        // (Malformed/Io) -- both are typed errors, and either is
        // acceptable, but a panic is not.
        match err {
            DecodeError::UnsupportedFormat(_) | DecodeError::Malformed(_) | DecodeError::Io(_) => {}
            DecodeError::NoAudioTrack => {}
        }
    }

    #[test]
    fn empty_bytes_never_panics() {
        let err = decode_bytes(Vec::new(), None).unwrap_err();
        assert!(matches!(
            err,
            DecodeError::UnsupportedFormat(_) | DecodeError::Io(_)
        ));
    }
}
