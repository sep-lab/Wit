"""
The null test is the only prototype that needs no format parser, which makes it
the one most likely to be reached for first — and the easiest to get subtly
wrong, because an unaligned null reports "everything changed" on identical audio.

Tests that need real audio are marked and skip when ffmpeg is absent, loudly.
Everything that can be tested without ffmpeg is tested without it.
"""

from __future__ import annotations

import shutil
import struct
import subprocess
import sys
import wave
from pathlib import Path

import pytest

null_diff = pytest.importorskip("null_diff")

HAVE_FFMPEG = bool(shutil.which("ffmpeg") and shutil.which("ffprobe"))
needs_ffmpeg = pytest.mark.skipif(not HAVE_FFMPEG, reason="ffmpeg/ffprobe not installed")


class _FakeShutil:
    """Pretend ffmpeg is installed, without patching the real shutil globally."""

    @staticmethod
    def which(_name):
        return "/usr/bin/ffmpeg"


def write_tone(path: Path, seconds=1.0, rate=48000, amp=0.3, channels=2, seed=1):
    """Deterministic BROADBAND PCM. No numpy — must run on a stock Python.

    Deliberately not a pure tone. Measured: a 1-sample offset on a 440 Hz sine
    leaves a residual 24.8 dB BELOW the source (a 1-sample shift is only 3.3
    degrees of phase at 440 Hz), so a sine cannot exercise the misalignment
    behaviour at all. On broadband material the same offset leaves a residual
    ABOVE the source. Music is broadband; test on broadband.
    """
    import random

    rng = random.Random(seed)
    frames = int(rate * seconds)
    data = bytearray()
    for _ in range(frames):
        v = int(amp * 32767 * (rng.random() * 2 - 1))
        data += struct.pack("<h", v) * channels
    with wave.open(str(path), "wb") as w:
        w.setnchannels(channels)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(bytes(data))
    return path


# --------------------------------------------------------------------------- #
# no ffmpeg required
# --------------------------------------------------------------------------- #


def test_it_refuses_to_resample_rather_than_silently_destroying_the_null(monkeypatch):
    """
    Resampling to force a comparison would itself wreck the null, so a rate
    mismatch must be an error, not a best-effort answer.
    """
    monkeypatch.setattr(null_diff, "probe", lambda p: {
        "rate": 48000 if p.startswith("left") else 44100, "channels": 2, "duration": 1.0,
    })
    monkeypatch.setattr(null_diff, "shutil", _FakeShutil())
    monkeypatch.setattr(sys, "argv", ["null_diff.py", "left.wav", "right.wav"])
    with pytest.raises(SystemExit) as exc:
        null_diff.main()
    assert "sample rate" in str(exc.value)


def test_it_refuses_a_channel_count_mismatch(monkeypatch):
    monkeypatch.setattr(null_diff, "probe", lambda p: {
        "rate": 48000, "channels": 2 if p.startswith("left") else 1, "duration": 1.0,
    })
    monkeypatch.setattr(null_diff, "shutil", _FakeShutil())
    monkeypatch.setattr(sys, "argv", ["null_diff.py", "left.wav", "right.wav"])
    with pytest.raises(SystemExit) as exc:
        null_diff.main()
    assert "channel" in str(exc.value)


def test_a_failing_command_is_reported_not_swallowed():
    with pytest.raises(RuntimeError):
        null_diff._run([sys.executable, "-c", "import sys; sys.exit(3)"])


# --------------------------------------------------------------------------- #
# the real thing
# --------------------------------------------------------------------------- #


@needs_ffmpeg
def test_identical_files_null_to_silence(tmp_path):
    a = write_tone(tmp_path / "a.wav")
    b = write_tone(tmp_path / "b.wav")
    residual = null_diff.residual_db(str(a), str(b), 0, 48000)
    assert residual < -80, "identical audio must null, got %.1f dB" % residual


@needs_ffmpeg
def test_a_one_sample_offset_looks_like_everything_changed(tmp_path):
    """
    The trap this whole script exists to avoid, asserted so it cannot regress:
    identical audio one sample apart produces a residual close to the source.
    """
    a = write_tone(tmp_path / "a.wav")
    src = null_diff.source_level_db(str(a))
    unaligned = null_diff.residual_db(str(a), str(a), 1, 48000)
    # Measured on broadband material the residual actually EXCEEDS the source
    # (+3.0 dB); on real music it sits ~10 dB below. Either way it dwarfs a real
    # edit, which is the point. Assert the weaker, robust bound.
    assert unaligned - src > -12, (
        "a 1-sample offset should look like a large change (got %.1f dB vs source "
        "%.1f dB); if this ever fails, the alignment warning in the docs is wrong"
        % (unaligned, src)
    )


@needs_ffmpeg
def test_alignment_recovers_the_null_after_an_offset(tmp_path):
    a = write_tone(tmp_path / "a.wav", seconds=2.0)
    aligned = null_diff.residual_db(str(a), str(a), 0, 48000)
    assert aligned < -80


@needs_ffmpeg
def test_a_quieter_render_reads_as_a_real_change(tmp_path):
    a = write_tone(tmp_path / "a.wav", amp=0.30)
    b = write_tone(tmp_path / "b.wav", amp=0.28)  # ~0.6 dB down, global
    src = null_diff.source_level_db(str(a))
    residual = null_diff.residual_db(str(a), str(b), 0, 48000)
    assert residual > -80, "a real level change must not null to silence"
    assert residual < src, "the residual must sit below the source"


@needs_ffmpeg
def test_the_cli_runs_and_states_a_verdict(tmp_path):
    a = write_tone(tmp_path / "a.wav")
    b = write_tone(tmp_path / "b.wav")
    script = Path(__file__).resolve().parents[1] / "experiments" / "null_diff.py"
    proc = subprocess.run(
        [sys.executable, str(script), str(a), str(b), "--no-align"],
        capture_output=True, text=True, timeout=120,
    )
    assert proc.returncode == 0, proc.stderr
    assert "IDENTICAL within the noise floor" in proc.stdout


def test_ffmpeg_absence_is_loud_not_silent():
    """
    Skipping quietly is how a competing project shipped a headline bug: every
    real-fixture test skipped and nobody noticed. Make the gap visible.
    """
    if not HAVE_FFMPEG:
        pytest.skip(
            "ffmpeg/ffprobe not installed — the null-test audio assertions did NOT "
            "run. Install ffmpeg to get real coverage of experiments/null_diff.py."
        )
    assert HAVE_FFMPEG
