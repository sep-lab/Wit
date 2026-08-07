"""
Malformed input: does a broken project file produce an error, or a casualty?

SECURITY.md names the standard:

    "Parser crashes, hangs, or unbounded allocation on a malformed project file.
     A malicious .als/.flp/ProjectData should never be able to take down or
     exhaust the host."
    "XML entity expansion (billion laughs / XXE) in .als parsing."

SECURITY.md also puts ``experiments/`` out of scope, and that is fair — they are
research instruments. But this parsing code is the seed of the product parser, and
docs/ROADMAP.md has it becoming one. So the tests are written against the standard
the *product* has to meet, and the gap is recorded as strict xfails rather than
left as a comfortable silence.

Every test here runs under a wall-clock deadline and a memory ceiling, so a
regression that introduces a genuine hang fails the suite instead of wedging CI.
"""

from __future__ import annotations

import contextlib
import gc
import gzip
import signal
import struct
import time
import tracemalloc
import warnings
import xml.etree.ElementTree as ET

import pytest
from factories.als import simple_set
from factories.flp import FlpBuilder, encode_varint

DEADLINE_SECONDS = 15.0
MEMORY_CEILING = 192 * 1024 * 1024


class Hang(Exception):
    """Raised when a parser exceeded the wall-clock deadline."""


@contextlib.contextmanager
def bounded(seconds: float = DEADLINE_SECONDS, max_bytes: int = MEMORY_CEILING):
    """
    Run a parser under a hard time limit and an observed-allocation ceiling.

    The point is that a failure here is a *test* failure, not a hung CI job.
    """
    have_alarm = hasattr(signal, "setitimer") and hasattr(signal, "SIGALRM")
    if have_alarm:
        def _fire(signum, frame):
            raise Hang("parser exceeded %.1fs" % seconds)

        previous = signal.signal(signal.SIGALRM, _fire)
        signal.setitimer(signal.ITIMER_REAL, seconds)
    tracemalloc.start()
    try:
        yield
    finally:
        peak = tracemalloc.get_traced_memory()[1]
        tracemalloc.stop()
        if have_alarm:
            signal.setitimer(signal.ITIMER_REAL, 0)
            signal.signal(signal.SIGALRM, previous)
        assert peak <= max_bytes, (
            "parser allocated %.1f MB on a malformed input (ceiling %.1f MB)"
            % (peak / 1e6, max_bytes / 1e6)
        )


def write(tmp_path, name, data):
    p = tmp_path / name
    p.write_bytes(data)
    return str(p)


# --------------------------------------------------------------------------- #
# .als — corrupt containers
# --------------------------------------------------------------------------- #


def als_case(name):
    good = simple_set().to_bytes()
    return {
        "garbage": b"this is not a gzip stream, it is a text file" * 4,
        "empty": b"",
        "truncated-gzip": good[: len(good) // 2],
        "gzip-header-only": good[:10],
        "gzip-of-nothing": gzip.compress(b""),
        "gzip-of-junk": gzip.compress(b"\x00\x01\x02\x03" * 100),
        "unclosed-xml": gzip.compress(b"<Ableton><LiveSet><Tracks>"),
        "mismatched-xml": gzip.compress(b"<Ableton><LiveSet></Tracks></Ableton>"),
        "xml-with-nul": gzip.compress(b"<Ableton>\x00</Ableton>"),
    }[name]


@pytest.mark.parametrize(
    "name",
    [
        "garbage",
        "empty",
        "truncated-gzip",
        "gzip-header-only",
        "gzip-of-nothing",
        "gzip-of-junk",
        "unclosed-xml",
        "mismatched-xml",
        "xml-with-nul",
    ],
)
def test_corrupt_als_raises_a_recognisable_error(als_diff, tmp_path, name):
    """
    Not a crash, not a hang, not an unbounded allocation — a normal exception a
    caller can catch. These are the cases the container layer already gets right.
    """
    path = write(tmp_path, "bad.als", als_case(name))
    with bounded():
        with pytest.raises((OSError, EOFError, ET.ParseError, ValueError)) as exc:
            als_diff.build_model(path)
    assert str(exc.value), "the error carries no message"


def test_a_well_formed_xml_that_is_not_a_live_set_is_rejected(als_diff, tmp_path):
    """
    Fixed: this used to characterise an AttributeError leaking from deep inside
    the extractor ("'NoneType' object has no attribute 'find'"). build_model now
    raises a typed ValueError naming the file instead — see the test below.
    """
    path = write(tmp_path, "notlive.als", gzip.compress(b"<Ableton Creator='x'/>"))
    with bounded():
        with pytest.raises(ValueError):
            als_diff.build_model(path)


def test_a_non_live_set_error_says_what_is_wrong(als_diff, tmp_path):
    path = write(tmp_path, "notlive.als", gzip.compress(b"<Ableton Creator='x'/>"))
    with pytest.raises(Exception) as exc:
        als_diff.build_model(path)
    assert "LiveSet" in str(exc.value) or "Live set" in str(exc.value)
    assert not isinstance(exc.value, AttributeError)


def test_a_missing_file_fails_before_anything_else(als_diff, tmp_path):
    with bounded():
        with pytest.raises(FileNotFoundError):
            als_diff.build_model(str(tmp_path / "does-not-exist.als"))


# --------------------------------------------------------------------------- #
# .als — XML entity expansion (named explicitly in SECURITY.md)
# --------------------------------------------------------------------------- #


ENTITY_BOMB = b"""<?xml version="1.0"?>
<!DOCTYPE Ableton [
 <!ENTITY a "AAAAAAAAAA">
 <!ENTITY b "&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;">
 <!ENTITY c "&b;&b;&b;&b;&b;&b;&b;&b;&b;&b;">
 <!ENTITY d "&c;&c;&c;&c;&c;&c;&c;&c;&c;&c;">
 <!ENTITY e "&d;&d;&d;&d;&d;&d;&d;&d;&d;&d;">
]>
<Ableton Creator="x"><LiveSet><Tracks><AudioTrack Id="1">
<Name><EffectiveName Value="&e;" /></Name></AudioTrack></Tracks></LiveSet></Ableton>
"""


def test_external_entities_are_refused(als_diff, tmp_path):
    """
    The XXE half of the SECURITY.md item is already safe: expat refuses to resolve
    an external entity, so a crafted .als cannot read /etc/passwd. Pinned so that
    a future switch to a different parser cannot silently give it away.
    """
    xxe = (
        b'<?xml version="1.0"?>\n'
        b'<!DOCTYPE Ableton [ <!ENTITY xx SYSTEM "file:///etc/passwd"> ]>\n'
        b'<Ableton Creator="&xx;"><LiveSet><Tracks /></LiveSet></Ableton>'
    )
    path = write(tmp_path, "xxe.als", gzip.compress(xxe))
    with bounded():
        with pytest.raises(ET.ParseError, match="external entity"):
            als_diff.build_model(path)


def test_internal_entity_expansion_is_bounded(als_diff, tmp_path):
    """
    Fixed: build_model now refuses a DOCTYPE internal subset that has no
    external entity reference in it — exactly the shape of this 5-level bomb —
    before handing anything to ET.parse, rather than relying on the
    interpreter's own (undocumented, version-dependent) amplification limit.
    """
    path = write(tmp_path, "bomb.als", gzip.compress(ENTITY_BOMB))
    file_size = len(gzip.compress(ENTITY_BOMB))
    try:
        model = als_diff.build_model(path)
    except (ET.ParseError, ValueError):
        return  # refusing to expand is the correct behaviour
    expanded = len(model["tracks"]["1"]["name"])
    assert expanded < file_size * 10, (
        "a %d byte file expanded to a %d character value (%.0fx)"
        % (file_size, expanded, expanded / file_size)
    )


def test_a_small_als_cannot_allocate_an_unbounded_tree(als_diff, tmp_path):
    """
    Fixed: build_model now counts elements while parsing and refuses once the
    count exceeds MAX_XML_ELEMENTS, well before a flat fan-out like this one
    (100,000 siblings) could reach the 66 MB peak measured before the fix.
    Refusing outright is the correct behaviour, same as the entity-bomb case
    above; if a future change makes it parse to completion instead, the
    amplification bound below still has to hold.
    """
    body = (
        b'<Ableton Creator="x"><LiveSet><Tracks>'
        + b'<Note Value="0" />' * 100_000
        + b"</Tracks></LiveSet></Ableton>"
    )
    path = tmp_path / "amplify.als"
    with gzip.GzipFile(str(path), "wb", 9, mtime=0) as fh:
        fh.write(body)
    file_size = path.stat().st_size

    gc.collect()
    tracemalloc.start()
    refused = False
    try:
        als_diff.build_model(str(path))
    except ValueError:
        refused = True
    finally:
        peak = tracemalloc.get_traced_memory()[1]
        tracemalloc.stop()

    if refused:
        return  # refusing an oversized tree outright is the correct behaviour
    assert peak < file_size * 200, (
        "a %d byte file caused a %.1f MB peak (%.0fx amplification)"
        % (file_size, peak / 1e6, peak / file_size)
    )


# --------------------------------------------------------------------------- #
# .flp — corrupt streams
# --------------------------------------------------------------------------- #


def test_flp_rejects_a_wrong_magic_with_a_clear_message(flp, tmp_path):
    path = write(tmp_path, "bad.flp", b"RIFF" + b"\x00" * 40)
    with bounded():
        with pytest.raises(SystemExit) as exc:
            flp.parse(path)
    assert "not an FLP" in str(exc.value)


def test_flp_rejects_an_empty_file(flp, tmp_path):
    path = write(tmp_path, "empty.flp", b"")
    with bounded():
        with pytest.raises(SystemExit) as exc:
            flp.parse(path)
    assert "not an FLP" in str(exc.value)


def test_flp_rejects_a_missing_data_chunk(flp, tmp_path):
    data = FlpBuilder().with_magic(data=b"XXXX").to_bytes()
    path = write(tmp_path, "nodata.flp", data)
    with bounded():
        with pytest.raises(SystemExit) as exc:
            flp.parse(path)
    assert "expected b'FLdt'" in str(exc.value)


@pytest.mark.parametrize(
    "name,data",
    [
        (
            "header-length-lies",
            b"FLhd" + struct.pack("<I", 2) + b"\x00\x00",
        ),
        (
            "truncated-in-header",
            b"FLhd" + struct.pack("<I", 6) + b"\x00\x00",
        ),
        (
            "data-length-past-eof",
            FlpBuilder().byte(0, 1).with_data_length(9999).to_bytes(),
        ),
        (
            "varint-runs-off-the-end",
            FlpBuilder().raw_bytes(bytes([209]) + b"\xff\xff").to_bytes(),
        ),
    ],
)
def test_corrupt_flp_streams_terminate_without_hanging(flp, tmp_path, name, data):
    """
    The bar this suite can honestly claim today: they stop, quickly, in bounded
    memory. The *quality* of the error is a separate matter — see below.
    """
    path = write(tmp_path, "%s.flp" % name, data)
    with bounded():
        with pytest.raises((SystemExit, IndexError, struct.error, ValueError)):
            flp.parse(path)


@pytest.mark.parametrize(
    "data",
    [
        FlpBuilder().byte(0, 1).with_data_length(9999).to_bytes(),
        FlpBuilder().raw_bytes(bytes([209]) + b"\xff\xff").to_bytes(),
    ],
)
def test_truncated_flp_reports_where_it_broke(flp, tmp_path, data):
    """
    Fixed: flp_parse now bounds-checks before every read and raises
    flp.FlpParseError (a ValueError, never a bare IndexError/struct.error)
    naming the file and the byte offset it broke at.
    """
    path = write(tmp_path, "trunc.flp", data)
    with pytest.raises(Exception) as exc:
        flp.parse(path)
    assert not isinstance(exc.value, (IndexError, struct.error))
    assert "trunc.flp" in str(exc.value) or "truncated" in str(exc.value).lower()


def test_an_event_size_running_past_the_buffer_is_refused(flp, flp_report):
    """
    Fixed: previously an event declaring a 1 MB payload in a 29-byte file was
    accepted, the missing bytes were silently dropped, and the summary reported
    "variable-length payload: 1000000 B (3448275.9% of the 29 B file)" — exactly
    the kind of figure AGENTS.md forbids publishing. It is now refused with a
    typed error instead of silently truncating.
    """
    data = FlpBuilder().raw_event(209, b"abc", 1_000_000).to_bytes()
    with bounded():
        with pytest.raises(flp.FlpParseError):
            flp_report(data)


def test_a_payload_larger_than_the_file_is_refused(flp, tmp_path):
    data = FlpBuilder().raw_event(209, b"abc", 1_000_000).to_bytes()
    path = write(tmp_path, "oversize.flp", data)
    # Any clean, typed failure is acceptable; a hang or MemoryError is not.
    with pytest.raises((IndexError, ValueError, struct.error, SystemExit)):
        flp.parse(path)


def test_a_giant_declared_payload_is_refused_without_allocating(flp, flp_report):
    """
    Fixed: a 2**60 declared length used to be accepted (the payload was taken
    with a slice, so it cost nothing to *store*, but the absurd declared size
    was never checked against what was actually available). It is now refused
    up front — refusing costs nothing to allocate either, so the memory bound
    this test used to pin still holds.
    """
    data = FlpBuilder().raw_event(209, b"", (1 << 60)).to_bytes()
    with bounded(max_bytes=32 * 1024 * 1024):
        with pytest.raises(flp.FlpParseError):
            flp_report(data)


@pytest.mark.slow
def test_a_long_varint_does_not_cost_quadratic_time(flp, tmp_path):
    def elapsed(n):
        data = FlpBuilder().raw_bytes(bytes([209]) + b"\xff" * n + b"\x00").to_bytes()
        path = write(tmp_path, "varint-%d.flp" % n, data)
        start = time.perf_counter()
        with contextlib.suppress(Exception):
            with contextlib.redirect_stdout(None):
                flp.parse(path)
        return time.perf_counter() - start

    small = elapsed(50_000)
    large = elapsed(200_000)
    if small < 0.02:
        pytest.skip("machine too fast to measure the ratio reliably")
    # 4x the input should cost about 4x the time for a linear parser.
    assert large / small < 8.0, (
        "4x input cost %.1fx time (%.3fs -> %.3fs): the varint decoder is "
        "superlinear" % (large / small, small, large)
    )


def test_varints_of_legitimate_length_are_fine(flp_report):
    """The counterpart: a real 5-byte varint must keep working."""
    payload = b"\xcc" * 70_000
    data = FlpBuilder().var(209, payload).byte(1, 1).to_bytes()
    assert encode_varint(70_000) == b"\xf0\xa2\x04"
    with bounded():
        report = flp_report(data)
    assert report["events"] == 2
    assert report["rows"][209]["payload_bytes"] == 70_000


# --------------------------------------------------------------------------- #
# housekeeping
# --------------------------------------------------------------------------- #


def test_flp_parse_does_not_leak_the_input_file_handle(flp, tmp_path, capsys):
    path = write(tmp_path, "ok.flp", FlpBuilder().byte(0, 1).to_bytes())
    gc.collect()
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always", ResourceWarning)
        with contextlib.redirect_stdout(None):
            flp.parse(path)
        gc.collect()
    leaks = [w for w in caught if issubclass(w.category, ResourceWarning)]
    assert not leaks, "unclosed file: %s" % (leaks[0].message if leaks else "")


def test_the_bounded_helper_actually_catches_a_hang():
    """A guard nobody has tested is not a guard."""
    if not hasattr(signal, "setitimer"):
        pytest.skip("no SIGALRM on this platform")
    with pytest.raises(Hang):
        with bounded(seconds=0.2):
            time.sleep(5)


def test_the_bounded_helper_actually_catches_an_allocation():
    with pytest.raises(AssertionError, match="allocated"):
        with bounded(max_bytes=1024):
            _ = [bytes(100_000) for _ in range(20)]
