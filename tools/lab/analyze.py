"""
analyze — which bytes changed between consecutive lab captures, localised to records (skeleton).

WHAT THIS DOES
    Walks one captured run ($WIT_CORPUS/<daw>/<run>/manifest.json plus its step
    folders) and, for every consecutive pair of captured steps, emits one JSON
    line per snapshotted file:

      - status: added / removed / changed / unchanged, sizes and sha256s;
      - the byte ranges [start, end) that differ, compared position by position
        (a length difference adds one tail range), capped by --max-ranges;
      - for Logic / GarageBand ProjectData: every differing range localised to the
        record that holds it — record index, 4CC tag as on disk ("karT") and read
        forwards ("Trak"), header or payload, and the offset inside the record —
        in both the old and the new file; plus a record-level summary: the common
        prefix and suffix of byte-identical records, the records in between, the
        per-tag count delta, and (when both files have the same record layout)
        each changed record with its count of differing bytes;
      - for .plist files (MetaData.plist, ProjectInformation.plist): the keys whose
        values changed, with scalar values shown (strings redacted, capped);
      - for gzip files (Ableton .als): the diff runs over the decompressed XML.

    This feeds the Wave 2 Logic-decoding lane: a value sweep (steps 14-16 set the
    fader to -6, -12, +3 dB) shows up as the same few payload bytes of the same
    record changing each time.

    ProjectData framing (docs/FORMATS.md; crates/wit-logic/src/frame.rs), re-
    implemented stdlib-only here because experiments/ is frozen and not a stable
    importable API:
        root header 0x18 bytes: magic 23 47 C0 AB at +0x00, version word at +0x04,
                                u32 LE LENGTH = filesize - 0x18 at +0x10
        then records to EOF:    4-byte tag at +0x00, u32 LE payload size at +0x1C,
                                payload at +0x24 (record size = 0x24 + payload size)

USAGE
    python3 tools/lab/analyze.py                         # the run from the last init
    python3 tools/lab/analyze.py --daw logic --run r1 --out r1.jsonl
    python3 tools/lab/analyze.py --run-dir /path/to/wit-corpus/logic/r1 --max-ranges 64

WHAT THIS DOES NOT HANDLE
    - Positional comparison only. When a record is inserted, every later byte is
      "different"; the record-level prefix/suffix summary is the tool for that
      case, not the byte ranges. No sequence alignment is attempted.
    - It localises; it does not decode. Which payload field is the fader value is
      the Wave 2 lane's job.
    - FL .flp events and Ableton XML elements are not localised (plain ranges only).
    - Framing errors (bad magic, length mismatch, a record running past EOF) are
      reported per file and localisation is skipped for that file.
    - Large gunzipped .als files diff at pure-Python speed (seconds per pair).
    - Pairs follow SCRIPT order (a re-captured early step sits where its step
      belongs); a skipped step simply makes its neighbours a pair.
"""

from __future__ import annotations

import argparse
import bisect
import collections
import gzip
import json
import plistlib
import struct
import sys
from pathlib import Path
from typing import Dict, List, Optional, Tuple

if not __package__:
    # Run as a file (python3 tools/lab/analyze.py): import this folder as the package
    # tools.lab so the relative imports below resolve (PEP 366). Relative imports keep
    # the CI stdlib-only check honest: it skips them, and they can only be siblings.
    sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
    __package__ = "tools.lab"

from . import labcore

MAGIC = b"\x23\x47\xc0\xab"
ROOT_HEADER_LEN = 0x18
RECORD_HEADER_LEN = 0x24
LENGTH_FIELD_OFFSET = 0x10
PAYLOAD_SIZE_OFFSET = 0x1C
LIST_CAP = 64


class FramingError(ValueError):
    pass


# --------------------------------------------------------------------------- #
# ProjectData framing
# --------------------------------------------------------------------------- #


def walk_records(data: bytes) -> List[Tuple[int, bytes, int]]:
    """[(record offset, tag, payload size)] in file order, or FramingError."""
    if len(data) < ROOT_HEADER_LEN:
        raise FramingError("%d bytes is too short for a root header" % len(data))
    if data[:4] != MAGIC:
        raise FramingError("bad magic (not a ProjectData container)")
    declared = struct.unpack_from("<I", data, LENGTH_FIELD_OFFSET)[0]
    if declared != len(data) - ROOT_HEADER_LEN:
        raise FramingError("root LENGTH says %d but filesize-24 is %d" % (declared, len(data) - ROOT_HEADER_LEN))
    records = []
    pos = ROOT_HEADER_LEN
    while pos < len(data):
        if pos + RECORD_HEADER_LEN > len(data):
            raise FramingError("record header at %#x runs past EOF" % pos)
        size = struct.unpack_from("<I", data, pos + PAYLOAD_SIZE_OFFSET)[0]
        end = pos + RECORD_HEADER_LEN + size
        if end > len(data):
            raise FramingError("record at %#x declares a payload past EOF" % pos)
        records.append((pos, bytes(data[pos:pos + 4]), size))
        pos = end
    return records


def tag_text(tag: bytes) -> str:
    return tag.decode("latin-1")


def tag_human(tag: bytes) -> str:
    """On-disk tags are reversed FourCCs: b"gnoS" reads "Song"."""
    return tag[::-1].decode("latin-1")


def locate(records: List[Tuple[int, bytes, int]], offsets: List[int], pos: int) -> Dict:
    if pos < ROOT_HEADER_LEN:
        return {"in": "root_header", "offset": pos}
    i = bisect.bisect_right(offsets, pos) - 1
    off, tag, size = records[i]
    rel = pos - off
    in_payload = rel >= RECORD_HEADER_LEN
    return {
        "record_index": i,
        "tag": tag_text(tag),
        "tag_human": tag_human(tag),
        "in": "payload" if in_payload else "record_header",
        "offset_in_record": rel,
        "payload_offset": rel - RECORD_HEADER_LEN if in_payload else None,
        "payload_size": size,
    }


def record_summary(old: bytes, new: bytes, old_recs, new_recs) -> Dict:
    ob = [old[o:o + RECORD_HEADER_LEN + s] for o, _t, s in old_recs]
    nb = [new[o:o + RECORD_HEADER_LEN + s] for o, _t, s in new_recs]
    lim = min(len(ob), len(nb))
    p = 0
    while p < lim and ob[p] == nb[p]:
        p += 1
    s = 0
    while s < lim - p and ob[len(ob) - 1 - s] == nb[len(nb) - 1 - s]:
        s += 1
    oc = collections.Counter(tag_text(t) for _o, t, _s in old_recs)
    nc = collections.Counter(tag_text(t) for _o, t, _s in new_recs)
    delta = {t: nc[t] - oc[t] for t in sorted(set(oc) | set(nc)) if nc[t] != oc[t]}
    out = {
        "old_records": len(ob),
        "new_records": len(nb),
        "common_prefix": p,
        "common_suffix": s,
        "between_old": [[i, tag_text(old_recs[i][1])] for i in range(p, len(ob) - s)][:LIST_CAP],
        "between_new": [[i, tag_text(new_recs[i][1])] for i in range(p, len(nb) - s)][:LIST_CAP],
        "census_delta": delta,
        "same_layout": [t for _o, t, _s in old_recs] == [t for _o, t, _s in new_recs]
        and all(a[2] == b[2] for a, b in zip(old_recs, new_recs)),
    }
    if out["same_layout"]:
        changed = []
        for i, (a, b) in enumerate(zip(ob, nb)):
            if a != b:
                changed.append([i, tag_text(old_recs[i][1]), sum(1 for x, y in zip(a, b) if x != y)])
        out["changed_records"] = changed[:LIST_CAP * 4]
        out["changed_record_count"] = len(changed)
    return out


# --------------------------------------------------------------------------- #
# generic diffing
# --------------------------------------------------------------------------- #


def diff_ranges(a: bytes, b: bytes) -> Tuple[List[List[int]], int]:
    """Every [start, end) where a and b differ position by position, and the byte count."""
    n = min(len(a), len(b))
    ranges: List[List[int]] = []
    start: Optional[int] = None
    diff_bytes = 0

    def close(at: int) -> None:
        nonlocal start
        if start is not None:
            ranges.append([start, at])
            start = None

    for i in range(0, n, 4096):
        j = min(i + 4096, n)
        if a[i:j] == b[i:j]:
            close(i)
            continue
        for k0 in range(i, j, 64):
            k1 = min(k0 + 64, j)
            if a[k0:k1] == b[k0:k1]:
                close(k0)
                continue
            for k in range(k0, k1):
                if a[k] != b[k]:
                    diff_bytes += 1
                    if start is None:
                        start = k
                else:
                    close(k)
    close(n)
    if len(a) != len(b):
        tail = [n, max(len(a), len(b))]
        diff_bytes += tail[1] - tail[0]
        if ranges and ranges[-1][1] == n:
            ranges[-1][1] = tail[1]
        else:
            ranges.append(tail)
    return ranges, diff_bytes


def _scalar(v):
    if isinstance(v, bool) or isinstance(v, (int, float)) or v is None:
        return v
    if isinstance(v, str):
        s = labcore.redact(v)
        return s if len(s) <= 120 else s[:117] + "..."
    if isinstance(v, bytes):
        return "<bytes %d>" % len(v)
    if isinstance(v, (list, tuple)):
        return "<list %d>" % len(v)
    if isinstance(v, dict):
        return "<dict %d>" % len(v)
    return "<%s>" % type(v).__name__


_ABSENT = object()


def plist_changes(old: bytes, new: bytes) -> Optional[List[Dict]]:
    try:
        a, b = plistlib.loads(old), plistlib.loads(new)
    except Exception:  # not a parseable plist: plain byte ranges only
        return None
    changes: List[Dict] = []

    def walk(x, y, path: str, depth: int) -> None:
        if isinstance(x, dict) and isinstance(y, dict) and depth < 3:
            for k in sorted(set(x) | set(y), key=str):
                walk(x.get(k, _ABSENT), y.get(k, _ABSENT), "%s.%s" % (path, k) if path else str(k), depth + 1)
            return
        if x is _ABSENT or y is _ABSENT or x != y:
            changes.append({
                "key": path,
                "old": "<absent>" if x is _ABSENT else _scalar(x),
                "new": "<absent>" if y is _ABSENT else _scalar(y),
            })

    walk(a, b, "", 0)
    return changes[:LIST_CAP * 4]


def compare_file(path: str, old: Optional[bytes], new: Optional[bytes], max_ranges: int) -> Dict:
    rec: Dict = {"file": path}
    if old is None or new is None:
        rec["status"] = "added" if old is None else "removed"
        rec["old_size"] = None if old is None else len(old)
        rec["new_size"] = None if new is None else len(new)
        return rec
    rec["old_size"], rec["new_size"] = len(old), len(new)
    if old == new:
        rec["status"] = "unchanged"
        return rec
    rec["status"] = "changed"
    a, b, domain = old, new, "bytes"
    if old[:2] == b"\x1f\x8b" and new[:2] == b"\x1f\x8b":
        try:
            a, b, domain = gzip.decompress(old), gzip.decompress(new), "gunzipped"
            rec["old_size_gunzipped"], rec["new_size_gunzipped"] = len(a), len(b)
        except (OSError, EOFError):
            a, b, domain = old, new, "bytes"
    ranges, diff_bytes = diff_ranges(a, b)
    rec.update({
        "domain": domain,
        "diff_bytes": diff_bytes,
        "range_count": len(ranges),
        "ranges": ranges[:max_ranges],
        "ranges_truncated": len(ranges) > max_ranges,
    })
    if Path(path).name == "ProjectData":
        try:
            orecs, nrecs = walk_records(a), walk_records(b)
        except FramingError as exc:
            rec["framing_error"] = str(exc)
        else:
            ooff = [o for o, _t, _s in orecs]
            noff = [o for o, _t, _s in nrecs]
            rec["localised"] = [
                {
                    "range": r,
                    "old": locate(orecs, ooff, r[0]) if r[0] < len(a) and orecs else None,
                    "new": locate(nrecs, noff, r[0]) if r[0] < len(b) and nrecs else None,
                    "old_end": locate(orecs, ooff, min(r[1], len(a)) - 1) if r[0] < len(a) and orecs else None,
                }
                for r in ranges[:max_ranges]
            ]
            rec["records"] = record_summary(a, b, orecs, nrecs)
            rec["version_word"] = {"old": a[4:6].hex(), "new": b[4:6].hex()}
    if path.endswith(".plist"):
        rec["plist_changes"] = plist_changes(old, new)
    return rec


# --------------------------------------------------------------------------- #
# the run
# --------------------------------------------------------------------------- #


def check_run_dir(path) -> Path:
    """analyze reads the corpus only: never a lab project, never a real library."""
    p = Path(path).expanduser()
    hit = labcore.denylisted(p)
    if hit:
        raise labcore.SafetyError("refusing to read %s: it is under %s" % (labcore.redact(p), hit))
    if labcore.is_within(labcore.real(p), labcore.real(labcore.lab_root()), casefold=True):
        raise labcore.SafetyError("refusing: %s is inside the lab root; analyze reads the corpus" % labcore.redact(p))
    return p


def _read_capture(run_dir: Path, capture: Dict, path: str) -> bytes:
    """
    Read one snapshotted file. The label and path come from manifest.json, which is
    data, not code: a "../" (or an absolute path, or a symlink) that would leave the
    run folder is refused instead of read.
    """
    rel = Path(capture["label"]) / path
    if rel.is_absolute() or ".." in rel.parts:
        raise labcore.SafetyError("manifest path %r leaves the run folder" % str(rel))
    target = run_dir / rel
    if not labcore.is_within(labcore.real(target), labcore.real(run_dir)):
        raise labcore.SafetyError("manifest path %r resolves outside the run folder" % str(rel))
    with labcore.open_nofollow(target) as fh:
        return fh.read()


def live_captures_in_step_order(m: Dict) -> List[Dict]:
    """Live captures sorted by their step's position in the script, not by append order
    (a `watch --replace` of an early step is appended last)."""
    order = {s["key"]: i for i, s in enumerate(m.get("steps", []))}
    caps = [c for c in m["captures"] if not c.get("superseded")]
    return sorted(caps, key=lambda c: (order.get(c["key"], len(order)), c["key"]))


def analyze_run(run_dir: Path, max_ranges: int = 256):
    """Yield one dict per (consecutive step pair, file)."""
    with open(str(run_dir / "manifest.json"), encoding="utf-8") as fh:
        m = json.load(fh)
    caps = live_captures_in_step_order(m)
    for before, after in zip(caps, caps[1:]):
        bfiles = {f["path"]: f for f in before["files"]}
        afiles = {f["path"]: f for f in after["files"]}
        for path in sorted(set(bfiles) | set(afiles)):
            old = _read_capture(run_dir, before, path) if path in bfiles else None
            new = _read_capture(run_dir, after, path) if path in afiles else None
            rec = compare_file(path, old, new, max_ranges)
            role = (afiles.get(path) or bfiles.get(path))["role"]
            head = {
                "daw": m["daw"], "run": m["run"],
                "from": before["label"], "to": after["label"],
                "from_key": before["key"], "to_key": after["key"],
                "edit": after["edit"], "expected": after["expected"],
                "role": role,
                "old_sha256": bfiles[path]["sha256"] if path in bfiles else None,
                "new_sha256": afiles[path]["sha256"] if path in afiles else None,
            }
            head.update(rec)
            yield head


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description="Localise byte changes between consecutive lab captures (JSONL).")
    ap.add_argument("--daw", choices=labcore.DAWS)
    ap.add_argument("--run")
    ap.add_argument("--run-dir", help="a corpus run folder holding manifest.json")
    ap.add_argument("--out", help="write JSONL here instead of stdout")
    ap.add_argument("--max-ranges", type=int, default=256)
    args = ap.parse_args(argv)
    try:
        if args.run_dir:
            run_dir = check_run_dir(args.run_dir)
        else:
            if args.daw and args.run:
                daw, run = labcore.check_daw(args.daw), labcore.check_run_name(args.run)
            else:
                with open(str(labcore.check_corpus_dir() / "current.json"), encoding="utf-8") as fh:
                    p = json.load(fh)
                daw, run = p["daw"], p["run"]
            run_dir = check_run_dir(labcore.check_corpus_dir(labcore.corpus_root() / daw / run))
        if args.out:
            out_path = Path(args.out).expanduser()
            if labcore.denylisted(out_path) or labcore.is_within(
                    labcore.real(out_path), labcore.real(labcore.lab_root()), casefold=True):
                raise labcore.SafetyError("refusing to write %s (lab root or real library)" % labcore.redact(out_path))
        lines = []
        summary: Dict[str, List[int]] = collections.OrderedDict()
        for rec in analyze_run(run_dir, args.max_ranges):
            lines.append(json.dumps(rec, ensure_ascii=False))
            key = "%s → %s" % (rec["from"], rec["to"])
            s = summary.setdefault(key, [0, 0])
            if rec["status"] != "unchanged":
                s[0] += 1
                s[1] += rec.get("diff_bytes", 0)
    except labcore.SafetyError as exc:
        print("REFUSED: %s" % exc, file=sys.stderr)
        return 2
    except (OSError, ValueError, KeyError) as exc:
        print("error: %s" % labcore.redact(exc), file=sys.stderr)
        return 1
    text = "\n".join(lines) + ("\n" if lines else "")
    if args.out:
        labcore.atomic_write_bytes(Path(args.out).expanduser(), text.encode("utf-8"))
    else:
        sys.stdout.write(text)
    for key, (files, nbytes) in summary.items():
        print("%-48s %d file(s) changed, %d byte(s) differ" % (key, files, nbytes), file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
