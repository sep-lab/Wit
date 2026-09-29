"""
make_audio — synthesise the Rosetta lab's test WAVs (stdlib only, deterministic).

WHAT THIS DOES
    Writes four short, synthetic, copyright-free WAV files (44.1 kHz, 16-bit PCM)
    that the lab's edit script imports into throwaway DAW projects:

      click-drums.wav   mono,   8.0 s  4 bars at 120 BPM 4/4: kick on 1 and 3,
                                       snare on 2 and 4, closed hat on every 8th
      sine-bass.wav     mono,   8.0 s  one sine note per bar (A1, F1, C2, G1)
      noise-pad.wav     stereo, 8.0 s  low-passed noise, independent L/R, slow swell
      صدا.wav           mono,   5.0 s  a 440 Hz tone with vibrato — the Persian
                                       FILE name is the point (Logic stores audio
                                       file names as UTF-16LE; this tests that).
                                       Deliberately NOT آواز: that is the track name
                                       step 8a sets, and FL names a new channel after
                                       the imported file (see steps.py)

    Everything is computed with math + random.Random(seed): the same seed gives
    the same bytes, every run, on the same machine. Also writes make_audio.json
    (seed, per-file size and sha256) next to the WAVs as provenance.

    Output goes to $WIT_LAB_ROOT/audio/ (default ~/WitLab/audio/). It refuses to
    write anywhere that is not under the lab root, and refuses to overwrite an
    existing file whose bytes differ unless --force is given (a re-run with the
    same seed finds identical bytes and leaves the files alone).

USAGE
    python3 tools/lab/make_audio.py                     # into ~/WitLab/audio/
    python3 tools/lab/make_audio.py --seed 7 --force    # different noise, overwrite
    python3 tools/lab/make_audio.py --out ~/WitLab/audio-alt
    python3 tools/lab/make_audio.py --list              # what it would write

WHAT THIS DOES NOT HANDLE
    - It is not music, and it must never be used to benchmark compression or
      dedup (AGENTS.md: music is not white noise). It exists only so the DAWs
      have something to import, move, trim and bounce.
    - Cross-platform bit-identity is not promised: math.sin may differ in the
      last float bit between libm builds, which can flip a 16-bit sample. The
      determinism promise is "same seed, same machine, same bytes".
    - No other sample rates, bit depths or formats (no AIFF, no 24-bit).
    - It downloads nothing and reads no audio from anywhere.
"""

from __future__ import annotations

import argparse
import hashlib
import io
import math
import random
import struct
import sys
import unicodedata
import wave
from pathlib import Path
from typing import Dict, List, Tuple

if not __package__:
    # Run as a file (python3 tools/lab/make_audio.py): import this folder as the package
    # tools.lab so the relative imports below resolve (PEP 366). Relative imports keep
    # the CI stdlib-only check honest: it skips them, and they can only be siblings.
    sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
    __package__ = "tools.lab"

from . import labcore

SAMPLE_RATE = 44100
SAMPLE_WIDTH = 2  # bytes: 16-bit PCM
DEFAULT_SEED = 1

PERSIAN_NAME = unicodedata.normalize("NFC", "صدا.wav")  # must match steps.PERSIAN_FILE

# name -> (channels, seconds, per-file salt so each file gets its own noise stream)
FILES: Dict[str, Tuple[int, float, int]] = {
    "click-drums.wav": (1, 8.0, 101),
    "sine-bass.wav": (1, 8.0, 202),
    "noise-pad.wav": (2, 8.0, 303),
    PERSIAN_NAME: (1, 5.0, 404),
}


# --------------------------------------------------------------------------- #
# synthesis — each returns a list of channels, each a list of floats in [-1, 1]
# --------------------------------------------------------------------------- #


def _hit(buf: List[float], start: int, length: int, fn) -> None:
    end = min(len(buf), start + length)
    for i in range(start, end):
        buf[i] += fn(i - start)


def _click_drums(n: int, rng: random.Random) -> List[List[float]]:
    # Peak levels are chosen so kick + hat or snare + hat never exceed full scale.
    buf = [0.0] * n
    beat = int(SAMPLE_RATE * 0.5)  # 120 BPM
    sr = float(SAMPLE_RATE)

    def kick(i):
        # A pitch drop from 110 Hz to 45 Hz: freq(t) = 45 + 65·e^(-30t), and the
        # phase below is its integral, so the sweep has no discontinuities.
        t = i / sr
        phase = 2 * math.pi * (45.0 * t + 65.0 * (1 - math.exp(-t * 30.0)) / 30.0)
        return 0.7 * math.exp(-t * 9.0) * math.sin(phase)

    snare_noise = [rng.uniform(-1.0, 1.0) for _ in range(int(sr * 0.2))]

    def snare(i):
        t = i / sr
        return math.exp(-t * 22.0) * (0.4 * snare_noise[i] + 0.25 * math.sin(2 * math.pi * 185.0 * t))

    hat_noise = [rng.uniform(-1.0, 1.0) for _ in range(int(sr * 0.05) + 1)]

    def hat(i):
        t = i / sr
        return 0.12 * math.exp(-t * 90.0) * (hat_noise[i + 1] - hat_noise[i])  # crude high-pass

    beats = -(-n // beat)  # ceiling: a partial last beat still gets its hits (they are cut at the end)
    for b in range(beats):
        pos = b * beat
        if b % 4 in (0, 2):
            _hit(buf, pos, int(sr * 0.3), kick)
        else:
            _hit(buf, pos, int(sr * 0.2), snare)
        _hit(buf, pos, int(sr * 0.05), hat)
        _hit(buf, pos + beat // 2, int(sr * 0.05), hat)
    return [buf]


def _sine_bass(n: int, rng: random.Random) -> List[List[float]]:
    del rng  # deterministic without noise; the signature stays uniform
    notes = [55.0, 43.654, 65.406, 48.999]  # A1 F1 C2 G1
    bar = SAMPLE_RATE * 2  # 4 beats at 120 BPM
    ramp = int(SAMPLE_RATE * 0.01)
    buf = []
    for i in range(n):
        idx = min(i // bar, len(notes) - 1)
        within = i - idx * bar
        remaining = min(bar, n - idx * bar) - within
        env = min(1.0, within / ramp, remaining / ramp) if ramp else 1.0
        buf.append(0.5 * env * math.sin(2 * math.pi * notes[idx] * i / SAMPLE_RATE))
    return [buf]


def _noise_pad(n: int, rng: random.Random) -> List[List[float]]:
    out = []
    attack = release = max(1, int(SAMPLE_RATE * 2.0))
    for _channel in range(2):
        lp = 0.0
        ch = []
        for i in range(n):
            lp += 0.02 * (rng.uniform(-1.0, 1.0) - lp)  # one-pole low-pass
            env = min(1.0, i / attack, (n - i) / release)
            ch.append(1.6 * env * lp)
        out.append(ch)
    return out


def _persian_tone(n: int, rng: random.Random) -> List[List[float]]:
    del rng
    attack = int(SAMPLE_RATE * 0.05)
    release = int(SAMPLE_RATE * 0.3)
    buf = []
    phase = 0.0
    for i in range(n):
        t = i / SAMPLE_RATE
        freq = 440.0 * (1.0 + 0.01 * math.sin(2 * math.pi * 5.0 * t))
        phase += 2 * math.pi * freq / SAMPLE_RATE
        env = min(1.0, i / attack, (n - i) / release)
        buf.append(0.3 * env * (math.sin(phase) + 0.2 * math.sin(2 * phase) + 0.1 * math.sin(3 * phase)))
    return [buf]


SYNTHS = {
    "click-drums.wav": _click_drums,
    "sine-bass.wav": _sine_bass,
    "noise-pad.wav": _noise_pad,
    PERSIAN_NAME: _persian_tone,
}


# --------------------------------------------------------------------------- #
# encoding
# --------------------------------------------------------------------------- #


def _to_pcm16(channels: List[List[float]]) -> bytes:
    n = len(channels[0])
    interleaved = []
    for i in range(n):
        for ch in channels:
            v = round(ch[i] * 32767.0)
            interleaved.append(32767 if v > 32767 else (-32768 if v < -32768 else v))
    return struct.pack("<%dh" % len(interleaved), *interleaved)


def render(name: str, seed: int = DEFAULT_SEED, scale: float = 1.0) -> bytes:
    """The complete WAV file bytes for `name`. `scale` shortens files (tests only)."""
    nchannels, seconds, salt = FILES[name]
    n = max(1, round(SAMPLE_RATE * seconds * scale))
    rng = random.Random(seed * 1000003 + salt)
    channels = SYNTHS[name](n, rng)
    assert len(channels) == nchannels
    bio = io.BytesIO()
    w = wave.open(bio, "wb")
    try:
        w.setnchannels(nchannels)
        w.setsampwidth(SAMPLE_WIDTH)
        w.setframerate(SAMPLE_RATE)
        w.writeframes(_to_pcm16(channels))
    finally:
        w.close()
    return bio.getvalue()


def generate(out_dir, seed: int = DEFAULT_SEED, force: bool = False, scale: float = 1.0) -> List[Dict]:
    """
    Write every file into `out_dir` (must be under the lab root). Returns one
    record per file: name, status (written / unchanged / overwritten), bytes, sha256.
    Raises labcore.SafetyError on a refused location or an unforced overwrite.
    """
    out = Path(out_dir).expanduser()
    labcore.check_lab_write_dir(out)
    rendered = {name: render(name, seed, scale) for name in FILES}

    # Check every overwrite before writing anything, so a refusal leaves no half-set.
    for name, data in rendered.items():
        target = out / name
        if target.is_symlink():
            raise labcore.SafetyError("refusing to write through a symlink: %s" % labcore.redact(target))
        if target.exists() and not target.is_file():
            raise labcore.SafetyError("refusing: %s exists and is not a regular file" % labcore.redact(target))
        if target.exists() and not force and labcore.read_regular(target, follow_symlinks=False) != data:
            raise labcore.SafetyError(
                "%s exists with different bytes (another seed?). Re-run with --force to replace it."
                % labcore.redact(target)
            )

    out.mkdir(parents=True, exist_ok=True)
    labcore.check_lab_write_dir(out)  # re-check after mkdir: realpath is now concrete
    records = []
    for name, data in rendered.items():
        target = out / name
        existed = target.exists()
        same = existed and labcore.read_regular(target, follow_symlinks=False) == data
        if not same:
            labcore.atomic_write_bytes(target, data)
        nchannels, _seconds, _salt = FILES[name]
        records.append({
            "name": name,
            "status": "unchanged" if same else ("overwritten" if existed else "written"),
            "channels": nchannels,
            "seconds": round((len(data) - 44) / (SAMPLE_RATE * SAMPLE_WIDTH * nchannels), 3),
            "bytes": len(data),
            "sha256": hashlib.sha256(data).hexdigest(),
        })
    labcore.atomic_write_json(out / "make_audio.json", {
        "tool": "tools/lab/make_audio.py",
        "seed": seed,
        "sample_rate": SAMPLE_RATE,
        "bits": 8 * SAMPLE_WIDTH,
        "scale": scale,
        "files": [{k: r[k] for k in ("name", "channels", "seconds", "bytes", "sha256")} for r in records],
    })
    return records


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description="Synthesise the lab's test WAVs (deterministic).")
    ap.add_argument("--seed", type=int, default=DEFAULT_SEED, help="noise seed (default %d)" % DEFAULT_SEED)
    ap.add_argument("--out", help="output folder under the lab root (default $WIT_LAB_ROOT/audio)")
    ap.add_argument("--force", action="store_true", help="overwrite files whose bytes differ")
    ap.add_argument("--list", action="store_true", help="print what would be written and exit")
    args = ap.parse_args(argv)

    if args.list:
        for name, (ch, secs, _salt) in FILES.items():
            print("%-18s %s %4.1f s  44.1 kHz 16-bit" % (name, "stereo" if ch == 2 else "mono  ", secs))
        return 0

    out = Path(args.out).expanduser() if args.out else labcore.lab_root() / "audio"
    try:
        records = generate(out, seed=args.seed, force=args.force)
    except labcore.SafetyError as exc:
        print("REFUSED: %s" % exc, file=sys.stderr)
        return 2
    print("audio folder: %s (seed %d)" % (labcore.redact(out), args.seed))
    for r in records:
        print("  %-10s %-18s %7d bytes  %5.2f s  sha256 %s…" % (
            r["status"], r["name"], r["bytes"], r["seconds"], r["sha256"][:16]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
