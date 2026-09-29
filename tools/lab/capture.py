"""
capture — snapshot a throwaway lab project after each scripted edit, into the corpus.

WHAT THIS DOES
    The Rosetta lab makes one known edit per save in a real DAW (steps.py). This
    tool turns each of those saves into a labelled corpus entry:

      init    validates the safety rules, then creates
              $WIT_CORPUS/<daw>/<run>/manifest.json recording the DAW name and
              build (read from the installed app's Info.plist), the OS version,
              the CPU architecture, the start time and the run's step list.
      watch   waits until the project's files are STABLE (the size and mtime of
              every relevant file unchanged for 2 s, by polling) AND different
              from the previous captured step, then copies the project files into
              $WIT_CORPUS/<daw>/<run>/<NN-label>/ (same relative layout) and
              appends to the manifest: step, edit, expected change, capture time,
              per-file size + sha256, and whether the bytes differ from the
              previous step. Steps marked allow_identical (the pure-churn save,
              the Save-a-copy, the bounce) only need a NEW save (mtime moved),
              not new bytes; --allow-identical forces that for any step.
              "Previous" is the nearest EARLIER step in script order, so a
              `--replace` of step 02 after 03 compares against 01. A step folder
              that already exists without a live capture (an interrupted run) is
              never written over: watch refuses, and --replace keeps the old
              folder as <label>.orphaned-<time>. Only regular files are read, so
              a FIFO named like a project file is skipped, not waited on.
      next    advances the step pointer and prints the next edit's instructions.
      status  prints run progress and the current step's instructions.

    What is snapshotted — project files only, never media:
      Logic / GarageBand  every *.logicx / *.band package in the run folder:
                          Alternatives/*/ProjectData, Alternatives/*/MetaData.plist,
                          Resources/ProjectInformation.plist (copied).
                          Alternatives/*/Project File Backups/** is LISTED (name,
                          size, mtime — never read or copied): whether and how the
                          DAW rotates its backup slots on each save is what the
                          plan's watcher design rests on ("Logic keeps 10 backups").
      Ableton             every *.als outside Backup/ (copied); Backup/*.als LISTED
                          (Live keeps its own 10-deep chain; the listing shows it).
      FL Studio           every *.flp outside Backup/ (copied, role "project") and
                          Backup/*.flp autosaves (copied, role "autosave"; they are
                          snapshotted but do not by themselves make a step "changed").
      all DAWs            <run>/lab-bounces/* is LISTED (step 26 pairs a bounce with
                          a save; the audio itself is never read or copied).
    Media/, Samples/, Undo Data.nosync/, audio/video/artwork extensions and every
    other file are skipped (labcore.is_media is the second gate).

HARD SAFETY RULES (enforced in code, tested in tests/test_lab_tools.py)
    - Capture only READS projects under the lab root: <lab root>/<daw>/<run>/.
      The project, the run folder and every file are judged by realpath; a
      symlink pointing out of the run folder is refused, not followed.
    - It refuses anything under ~/Music/Logic, ~/Music/GarageBand,
      ~/Projects/DAW/{Logic Pro,Abelton,FL Studio} or
      ~/Documents/Image-Line/FL Studio/Projects (labcore.DENYLIST_REL), and a lab
      root that is, or contains, one of them.
    - It never writes inside the lab project; it writes only into the corpus,
      which may not sit inside the lab root, a real library or this repository.
    - Manifest writes are atomic (temp file + fsync + rename). A capture lands in
      a hidden .partial folder first, is re-hashed against the stable state, and
      is renamed into place only if the project did not change during the copy.
    - It launches nothing, sends no keystrokes and makes no network calls.

USAGE
    python3 tools/lab/capture.py init --daw logic --run r1 --project ~/WitLab/logic/r1/Lab.logicx
    python3 tools/lab/capture.py watch [--step 03] [--allow-identical] [--note "..."]
    python3 tools/lab/capture.py next [--skip "reason"]
    python3 tools/lab/capture.py status
    (watch/next/status use the run from the last init; pass --daw/--run to pick another.)

    Environment: WIT_LAB_ROOT (default ~/WitLab), WIT_CORPUS (default
    ~/Projects/DAW/wit-corpus). Exit codes: 0 ok, 1 error, 2 refused by a safety
    rule, 3 watch timed out.

WHAT THIS DOES NOT HANDLE
    - It cannot tell WHICH edit a save contains; it trusts the step pointer. If a
      save fired before the edit was made, re-capture with `watch --replace`.
    - Stability is a heuristic: a DAW that pauses more than 2 s mid-save would be
      captured early. The copy is re-verified against the stable state, which
      catches a write DURING the copy, not a pause before it.
    - Hard links and directories renamed between check and read (see labcore).
    - FL Studio autosaves written to FL's own user-data Projects/Backup folder
      are out of reach on purpose (that folder is a real library); only a Backup/
      folder inside the lab run folder is read.
    - Concurrent watchers on the same run are not locked against each other.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import shutil
import stat
import subprocess
import sys
import time
from pathlib import Path
from typing import Dict, List, Optional, Tuple

if not __package__:
    # Run as a file (python3 tools/lab/capture.py): import this folder as the package
    # tools.lab so the relative imports below resolve (PEP 366). Relative imports keep
    # the CI stdlib-only check honest: it skips them, and they can only be siblings.
    sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
    __package__ = "tools.lab"

from . import labcore
from . import steps as lab_steps

SCHEMA = "wit-lab-manifest/1"
EXIT_OK, EXIT_ERROR, EXIT_REFUSED, EXIT_TIMEOUT = 0, 1, 2, 3
POINTER = "current.json"
BOUNCE_DIR = "lab-bounces"
MAX_DEPTH = 8

ROLE_PROJECT = "project"    # copied; decides whether a step "changed"
ROLE_AUTOSAVE = "autosave"  # copied (FL Backup/*.flp); never decides "changed" on its own
ROLE_LISTING = "listing"    # name/size/mtime only — never opened, never copied
COPIED_ROLES = (ROLE_PROJECT, ROLE_AUTOSAVE)

PACKAGE_EXT = {"logic": ".logicx", "garageband": ".band"}


class WatchTimeout(Exception):
    pass


class _ChangedDuringCopy(Exception):
    pass


NON_REGULAR = "  ! skipping %s: it is not a regular file (FIFO, socket or device) and is never opened"


def _log(msg: str) -> None:
    print(msg, flush=True)


# --------------------------------------------------------------------------- #
# what belongs in a snapshot
# --------------------------------------------------------------------------- #


def classify(daw: str, rel: str) -> Optional[str]:
    """The role of run-folder-relative path `rel` for `daw`, or None to ignore it."""
    parts = Path(rel).parts
    if not parts:
        return None
    if parts[0] == BOUNCE_DIR:
        return ROLE_LISTING if len(parts) >= 2 else None
    if daw in PACKAGE_EXT:
        ext = PACKAGE_EXT[daw]
        idx = next((i for i, p in enumerate(parts[:-1]) if p.casefold().endswith(ext)), None)
        if idx is None:
            return None
        inner = parts[idx + 1:]
        if inner == ("Resources", "ProjectInformation.plist"):
            return ROLE_PROJECT
        if len(inner) == 3 and inner[0] == "Alternatives" and inner[2] in ("ProjectData", "MetaData.plist"):
            return ROLE_PROJECT
        if len(inner) >= 4 and inner[0] == "Alternatives" and inner[2] == "Project File Backups":
            return ROLE_LISTING
        return None
    if labcore.is_media(rel):
        return None
    suffix = Path(rel).suffix.casefold()
    if daw == "ableton" and suffix == ".als":
        return ROLE_LISTING if "Backup" in parts[:-1] else ROLE_PROJECT
    if daw == "fl" and suffix == ".flp":
        return ROLE_AUTOSAVE if len(parts) >= 2 and parts[-2] == "Backup" else ROLE_PROJECT
    return None


def collect(run_real: Path, daw: str, warn=None) -> List[Dict]:
    """
    Walk the run folder (no symlink following) and return the snapshot set:
    [{"path", "role", "size", "mtime_ns"}], sorted by path. Raises SafetyError if
    any symlink under the run folder points outside it. A FIFO, socket or device
    named like a project file is never opened; `warn(relpath)` is told about it.
    """
    base = str(run_real)
    out = []
    for dirpath, dirnames, filenames in os.walk(base, followlinks=False):
        rel_dir = os.path.relpath(dirpath, base)
        depth = 0 if rel_dir == "." else len(Path(rel_dir).parts)
        keep = []
        for name in sorted(dirnames):
            full = os.path.join(dirpath, name)
            if os.path.islink(full):
                _check_symlink(full, base)
                continue  # never descend through a symlink, even an internal one
            if name.startswith(".") or depth >= MAX_DEPTH or name in labcore.MEDIA_DIRS:
                continue  # hidden folders, runaway depth, and media folders are never walked
            keep.append(name)
        dirnames[:] = keep
        for name in sorted(filenames):
            full = os.path.join(dirpath, name)
            if os.path.islink(full):
                _check_symlink(full, base)
                continue
            rel = labcore._nfc(os.path.relpath(full, base)).replace(os.sep, "/")
            role = classify(daw, rel)
            if role is None:
                continue
            st = os.lstat(full)
            if not stat.S_ISREG(st.st_mode):
                if warn is not None:
                    warn(rel)
                continue  # a FIFO, socket or device named like a project file is never opened
            out.append({"path": rel, "role": role, "size": st.st_size, "mtime_ns": st.st_mtime_ns})
    out.sort(key=lambda e: e["path"])
    return out


def _check_symlink(full: str, base: str) -> None:
    target = labcore.real(full)
    if not labcore.is_within(target, base) or labcore.denylisted(full):
        raise labcore.SafetyError(
            "refusing: %s is a symlink that points outside the lab run folder. The lab never "
            "follows links out of %s." % (labcore.redact(full), labcore.redact(base))
        )


def signature(entries: List[Dict]) -> Tuple:
    return tuple((e["path"], e["role"], e["size"], e["mtime_ns"]) for e in entries)


def hash_entries(run_real: Path, entries: List[Dict]) -> Dict[str, str]:
    return {
        e["path"]: labcore.sha256_file(run_real / e["path"])
        for e in entries if e["role"] in COPIED_ROLES
    }


def compare(prev_files: Optional[List[Dict]], entries: List[Dict], hashes: Dict[str, str]) -> Dict:
    """How the current state relates to the previous capture."""
    if prev_files is None:
        return {"bytes_differ": None, "saved": None, "changed_files": []}
    prev = {f["path"]: f for f in prev_files if f["role"] in COPIED_ROLES}
    cur = {e["path"]: e for e in entries if e["role"] in COPIED_ROLES}
    changed = sorted(
        set(prev) ^ set(cur)
        | {p for p in set(prev) & set(cur) if prev[p].get("sha256") != hashes.get(p)}
    )
    prev_proj = {p for p, f in prev.items() if f["role"] == ROLE_PROJECT}
    cur_proj = {p for p, e in cur.items() if e["role"] == ROLE_PROJECT}
    bytes_differ = prev_proj != cur_proj or any(prev[p].get("sha256") != hashes.get(p) for p in cur_proj)
    saved = prev_proj != cur_proj or any(
        (prev[p]["size"], prev[p]["mtime_ns"]) != (cur[p]["size"], cur[p]["mtime_ns"]) for p in cur_proj
    )
    return {"bytes_differ": bytes_differ, "saved": saved, "changed_files": changed}


# --------------------------------------------------------------------------- #
# waiting and copying
# --------------------------------------------------------------------------- #


def wait_for_save(run_real: Path, daw: str, prev_files: Optional[List[Dict]], allow_identical: bool,
                  stable_secs: float = 2.0, poll: float = 0.25, timeout: float = 300.0,
                  clock=time.monotonic, sleep=time.sleep, log=_log):
    """
    Poll until the snapshot set has been unchanged for `stable_secs` and (unless
    this is the first capture) differs from the previous capture. Returns
    (entries, hashes, comparison, seconds_stable). Raises WatchTimeout.
    """
    deadline = clock() + timeout
    last_sig, since, judged, last_note = None, clock(), None, None
    warned = set()

    def warn(rel: str) -> None:
        if rel not in warned:  # once per path, not once per poll
            warned.add(rel)
            log(NON_REGULAR % rel)

    while True:
        entries = collect(run_real, daw, warn)
        sig = signature(entries)
        now = clock()
        if sig != last_sig:
            last_sig, since = sig, now
        elif now - since >= stable_secs and sig != judged:
            judged = sig
            note = None
            if not any(e["role"] == ROLE_PROJECT for e in entries):
                note = "no project file in the run folder yet"
            else:
                try:
                    hashes = hash_entries(run_real, entries)
                except OSError:
                    last_sig, judged = None, None  # a file moved under us: not stable after all
                    hashes = None
                if hashes is not None:
                    cmp = compare(prev_files, entries, hashes)
                    if prev_files is None or cmp["bytes_differ"] or (allow_identical and cmp["saved"]):
                        return entries, hashes, cmp, now - since
                    note = ("stable, but no new save since the previous capture — waiting for the save"
                            if not cmp["saved"] else
                            "a new save landed with bytes identical to the previous capture; this step "
                            "expects a change (use --allow-identical if that is the finding) — waiting")
            if note and note != last_note:
                log("  … " + note)
                last_note = note
        if now >= deadline:
            raise WatchTimeout(last_note or "the project never stayed unchanged for %.1f s" % stable_secs)
        sleep(poll)


def copy_snapshot(run_real: Path, daw: str, entries: List[Dict], hashes: Dict[str, str], dest: Path) -> None:
    """
    Copy the COPIED_ROLES files into `dest` (which must not exist yet) via a hidden
    partial folder. Raises _ChangedDuringCopy if the project moved during the copy.
    """
    partial = dest.parent / (".%s.partial-%d" % (dest.name, os.getpid()))
    _remove_partial(partial)
    try:
        for e in entries:
            if e["role"] not in COPIED_ROLES:
                continue
            src = run_real / e["path"]
            if labcore.is_media(e["path"]):
                raise labcore.SafetyError("refusing to copy media: %s" % e["path"])
            if not labcore.is_within(labcore.real(src), str(run_real)):
                raise labcore.SafetyError("refusing: %s resolves outside the run folder" % labcore.redact(src))
            dst = partial / e["path"]
            dst.parent.mkdir(parents=True, exist_ok=True)
            h = hashlib.sha256()
            with labcore.open_nofollow(src) as fin, open(str(dst), "wb") as fout:
                for chunk in iter(lambda: fin.read(1 << 20), b""):
                    h.update(chunk)
                    fout.write(chunk)
            if h.hexdigest() != hashes[e["path"]]:
                raise _ChangedDuringCopy(e["path"])
            os.utime(str(dst), ns=(e["mtime_ns"], e["mtime_ns"]))
        if signature(collect(run_real, daw)) != signature(entries):
            raise _ChangedDuringCopy("(file set)")
        os.rename(str(partial), str(dest))
    except BaseException:
        _remove_partial(partial)
        raise


def _remove_partial(partial: Path) -> None:
    # Only ever deletes a folder this tool created: hidden, named *.partial-<pid>,
    # inside the corpus run folder.
    if partial.name.startswith(".") and ".partial-" in partial.name and partial.exists():
        labcore.check_corpus_dir(partial)
        shutil.rmtree(str(partial))


# --------------------------------------------------------------------------- #
# environment facts for the manifest (read-only)
# --------------------------------------------------------------------------- #


def _read_cmd(cmd: List[str]) -> Optional[str]:
    """Run a read-only query (sw_vers / sysctl -n) and return its stdout, or None."""
    assert cmd[0] in ("sw_vers", "sysctl") and "-w" not in cmd, cmd
    try:
        out = subprocess.run(cmd, capture_output=True, text=True, timeout=10)
    except (OSError, subprocess.SubprocessError):
        return None
    return out.stdout.strip() if out.returncode == 0 else None


def hardware_arch() -> str:
    """
    The machine's CPU architecture — NOT this Python's. A Python running under
    Rosetta reports x86_64 on Apple Silicon (measured on the lab Mac: an x86_64
    python3 on an M1 Pro), and TESTING.md §8 records architecture because
    denormal handling differs between x86 and aarch64.
    """
    if platform.system() == "Darwin" and _read_cmd(["sysctl", "-n", "hw.optional.arm64"]) == "1":
        return "arm64"
    return platform.machine()


def os_info() -> Dict:
    info = {
        "system": platform.system(),
        "release": platform.release(),
        "platform": platform.platform(),
        "hardware_arch": hardware_arch(),
        "python_arch": platform.machine(),
        "python": platform.python_version(),
    }
    if platform.system() == "Darwin":
        info["macos_version"] = platform.mac_ver()[0]
        info["macos_build"] = _read_cmd(["sw_vers", "-buildVersion"])
        info["cpu"] = _read_cmd(["sysctl", "-n", "machdep.cpu.brand_string"])
        info["python_under_rosetta"] = _read_cmd(["sysctl", "-n", "sysctl.proc_translated"]) == "1"
    return info


# --------------------------------------------------------------------------- #
# manifest
# --------------------------------------------------------------------------- #


def corpus_run_dir(daw: str, run: str) -> Path:
    return labcore.check_corpus_dir(labcore.corpus_root() / daw / run)


def manifest_path(daw: str, run: str) -> Path:
    return corpus_run_dir(daw, run) / "manifest.json"


def load_manifest(path: Path) -> Dict:
    with open(str(path), encoding="utf-8") as fh:
        m = json.load(fh)
    if m.get("schema") != SCHEMA:
        raise ValueError("%s is not a %s manifest" % (labcore.redact(path), SCHEMA))
    return m


def save_manifest(path: Path, manifest: Dict) -> None:
    labcore.check_corpus_dir(path.parent)
    manifest["updated_at"] = labcore.now_iso()
    labcore.atomic_write_json(path, manifest)


def live_captures(m: Dict) -> List[Dict]:
    return [c for c in m["captures"] if not c.get("superseded")]


def _step_index(m: Dict, key: str) -> int:
    for i, s in enumerate(m["steps"]):
        if s["key"] == key:
            return i
    raise ValueError("step %r is not in this run's step list — was manifest.json edited by hand? "
                     "Fix it or start a new run." % (key,))


def _find_step(m: Dict, want: str) -> Optional[Dict]:
    w = want.strip().lower()
    for s in m["steps"]:
        if w in (s["key"], s["key"].lstrip("0"), s["id"]):
            return s
    return None


def resolve_run(args) -> Tuple[str, str]:
    if args.daw and args.run:
        return labcore.check_daw(args.daw), labcore.check_run_name(args.run)
    if args.daw or args.run:
        raise labcore.SafetyError("pass both --daw and --run, or neither (to use the last init)")
    pointer = labcore.check_corpus_dir() / POINTER
    if not pointer.exists():
        raise FileNotFoundError("no current run: run `capture.py init` first, or pass --daw and --run")
    with open(str(pointer), encoding="utf-8") as fh:
        p = json.load(fh)
    return labcore.check_daw(p["daw"]), labcore.check_run_name(p["run"])


# --------------------------------------------------------------------------- #
# commands
# --------------------------------------------------------------------------- #


def _check_project_kind(daw: str, project: Path) -> None:
    if daw in PACKAGE_EXT:
        if not (project.is_dir() and project.name.casefold().endswith(PACKAGE_EXT[daw])):
            raise ValueError("a %s project is a %s package folder; got %s"
                             % (daw, PACKAGE_EXT[daw], labcore.redact(project)))
        return
    ext = ".als" if daw == "ableton" else ".flp"
    if project.is_file() and project.suffix.casefold() == ext:
        return
    if project.is_dir() and any(p.suffix.casefold() == ext for p in project.iterdir()):
        return
    raise ValueError("a %s project is a %s file (or a folder holding one); got %s"
                     % (daw, ext, labcore.redact(project)))


def cmd_init(args) -> int:
    daw = labcore.check_daw(args.daw)
    run = labcore.check_run_name(args.run)
    rd = labcore.run_dir(daw, run)
    project = labcore.check_readable_lab_path(args.project, daw, run)
    if not project.exists():
        raise FileNotFoundError("project %s does not exist — Save As into %s first"
                                % (labcore.redact(args.project), labcore.redact(rd)))
    _check_project_kind(daw, project)
    # collect() also refuses escaping symlinks anywhere in the run folder.
    entries = collect(rd, daw, lambda rel: _log(NON_REGULAR % rel))

    cdir = corpus_run_dir(daw, run)
    mpath = cdir / "manifest.json"
    if mpath.exists():
        raise FileExistsError("run %s/%s already has a manifest; pick a new --run name" % (daw, run))

    run_steps = lab_steps.for_daw(daw)
    manifest = {
        "schema": SCHEMA,
        "daw": daw,
        "run": run,
        "run_dir": "%s/%s" % (daw, run),  # relative to the lab root; no absolute paths
        "project": os.path.relpath(str(project), str(rd)).replace(os.sep, "/"),
        "app": labcore.find_app(daw, args.app),
        "os": os_info(),
        "arch": hardware_arch(),
        "started_at": labcore.now_iso(),
        "tool": "tools/lab/capture.py",
        "script_version": lab_steps.SCRIPT_VERSION,
        "steps": run_steps,
        "current_step": run_steps[0]["key"],
        "captures": [],
        "skipped": [],
    }
    cdir.mkdir(parents=True, exist_ok=True)
    save_manifest(mpath, manifest)
    labcore.atomic_write_json(labcore.check_corpus_dir() / POINTER,
                              {"daw": daw, "run": run, "set_at": labcore.now_iso()})

    chosen = manifest["app"]["chosen"] or {}
    _log("initialised %s/%s" % (daw, run))
    _log("  project:  %s" % labcore.redact(project))
    _log("  app:      %s %s (build %s)" % (chosen.get("bundle"), chosen.get("short_version"), chosen.get("build")))
    _log("  os:       %s %s" % (manifest["os"].get("platform"), manifest["arch"]))
    _log("  corpus:   %s" % labcore.redact(cdir))
    _log("  files:    %d project file(s) seen now" % sum(1 for e in entries if e["role"] == ROLE_PROJECT))
    if manifest["app"].get("note"):
        _log("  note:     %s" % manifest["app"]["note"])
    _log("")
    _log(lab_steps.format_instructions(run_steps[0], daw, len(run_steps), 1))
    return EXIT_OK


def cmd_watch(args) -> int:
    daw, run = resolve_run(args)
    mpath = manifest_path(daw, run)
    m = load_manifest(mpath)
    rd = labcore.run_dir(daw, run)
    labcore.check_readable_lab_path(rd / m["project"], daw, run)  # re-validated on every watch

    step = _find_step(m, args.step) if args.step else (
        _find_step(m, m["current_step"]) if m["current_step"] else None)
    if step is None and not args.step and m["current_step"]:
        _step_index(m, m["current_step"])  # raises the clear hand-edited-manifest error
    if step is None:
        _log("nothing to watch: %s" % ("no such step for %s: %s" % (daw, args.step) if args.step
                                         else "the run is complete"))
        return EXIT_ERROR
    existing = [c for c in live_captures(m) if c["key"] == step["key"]]
    if existing and not args.replace:
        raise FileExistsError("step %s is already captured (%s). Use --replace to capture it again "
                              "(the old capture is kept, marked superseded)." % (step["key"], existing[0]["label"]))
    cdir = corpus_run_dir(daw, run)
    dest = cdir / step["label"]
    orphan = dest.exists() and not existing
    if orphan and not args.replace:
        raise FileExistsError(
            "corpus folder %s already exists but the manifest has no live capture of step %s (an "
            "interrupted capture?). Nothing was overwritten. Re-run with --replace to capture again; "
            "the old folder is kept, renamed %s.orphaned-<time>." % (step["label"], step["key"], step["label"]))
    # "Previous" means the nearest EARLIER step in script order, not the last capture
    # appended to the manifest: re-capturing step 02 after 03 compares against 01.
    here = _step_index(m, step["key"])
    earlier = [c for c in live_captures(m) if _step_index(m, c["key"]) < here]
    prev = max(earlier, key=lambda c: _step_index(m, c["key"])) if earlier else None
    allow_identical = bool(args.allow_identical or step["allow_identical"])

    _log("watching %s for step %s (%s)%s — save in the DAW now if you have not"
         % (labcore.redact(rd), step["key"], step["id"], " [identical bytes allowed]" if allow_identical else ""))
    while True:
        try:
            entries, hashes, cmp, stable_for = wait_for_save(
                rd, daw, prev["files"] if prev else None, allow_identical,
                stable_secs=args.stable_seconds, poll=args.poll, timeout=args.timeout)
        except WatchTimeout as exc:
            _log("TIMEOUT after %.0f s: %s." % (args.timeout, exc))
            _log("  Did the DAW save into %s? (Logic and GarageBand default to ~/Music; the lab never "
                 "reads there.) Save again, then re-run watch." % labcore.redact(rd))
            return EXIT_TIMEOUT
        staging = dest if not dest.exists() else cdir / (".%s.incoming" % step["label"])
        try:
            if staging.exists():
                _remove_partial_named(staging)
            copy_snapshot(rd, daw, entries, hashes, staging)
            break
        except _ChangedDuringCopy as exc:
            _log("  … the project changed during the copy (%s); waiting for it to settle again" % exc)

    captured_at = labcore.now_iso()
    aside = None
    if staging != dest:
        stamp = captured_at.replace(":", "").replace("-", "")
        aside = cdir / ("%s.%s-%s" % (step["label"], "superseded" if existing else "orphaned", stamp))
        os.rename(str(dest), str(aside))
        os.rename(str(staging), str(dest))
    for c in m["captures"]:
        if c["key"] == step["key"] and not c.get("superseded"):
            c["superseded"] = True
            c["superseded_dir"] = aside.name if aside else None

    prev_hash = {f["path"]: f.get("sha256") for f in (prev["files"] if prev else [])}
    files = []
    for e in entries:
        if e["role"] not in COPIED_ROLES:
            continue
        rec = dict(e, sha256=hashes[e["path"]])
        rec["differs_from_previous"] = None if prev is None else prev_hash.get(e["path"]) != hashes[e["path"]]
        files.append(rec)
    entry = {
        "key": step["key"],
        "id": step["id"],
        "label": step["label"],
        "edit": step["edit"],
        "expected": step["expected"],
        "target": step.get("target"),
        "track_effects": {k: step[k] for k in ("adds_track", "renames_track", "deletes_track") if step.get(k)},
        "captured_at": captured_at,
        "stable_for_s": round(stable_for, 2),
        "allow_identical": allow_identical,
        "previous": prev["key"] if prev else None,
        "bytes_differ_from_previous": cmp["bytes_differ"],
        "saved_since_previous": cmp["saved"],
        "changed_files": cmp["changed_files"],
        "files": files,
        "listing": [{k: e[k] for k in ("path", "size", "mtime_ns")} for e in entries if e["role"] == ROLE_LISTING],
        "note": args.note,
        "replaced_orphan": aside.name if (aside and orphan) else None,
        "superseded": False,
    }
    m["captures"].append(entry)
    save_manifest(mpath, m)

    _log("captured step %s → %s" % (step["key"], labcore.redact(dest)))
    _log("  %d file(s) copied, %d listed; stable for %.1f s" % (len(files), len(entry["listing"]), stable_for))
    if prev is None:
        _log("  first capture of the run (nothing to compare with)")
    else:
        _log("  vs step %s: bytes %s; changed: %s" % (
            prev["key"], "DIFFER" if cmp["bytes_differ"] else "identical",
            ", ".join(cmp["changed_files"]) or "none"))
    _log("next: python3 tools/lab/capture.py next")
    return EXIT_OK


def _remove_partial_named(path: Path) -> None:
    # The ".incoming" staging folder of a --replace capture: ours, hidden, in the corpus.
    if path.name.startswith(".") and path.name.endswith(".incoming") and path.exists():
        labcore.check_corpus_dir(path)
        shutil.rmtree(str(path))


def cmd_next(args) -> int:
    daw, run = resolve_run(args)
    mpath = manifest_path(daw, run)
    m = load_manifest(mpath)
    cur = m["current_step"]
    if cur is None:
        _log("the run is complete: %d captured, %d skipped" % (len(live_captures(m)), len(m["skipped"])))
        return EXIT_OK
    i = _step_index(m, cur)  # a hand-edited, unknown step is a clear error, not "no capture yet"
    done = any(c["key"] == cur for c in live_captures(m)) or any(s["key"] == cur for s in m["skipped"])
    if args.skip:
        if not done:
            m["skipped"].append({"key": cur, "reason": args.skip, "at": labcore.now_iso()})
    elif not done:
        _log("step %s has no capture yet. Run `capture.py watch` after saving, or "
             "`capture.py next --skip \"why\"` if this DAW cannot do it." % cur)
        return EXIT_ERROR
    nxt = m["steps"][i + 1] if i + 1 < len(m["steps"]) else None
    m["current_step"] = nxt["key"] if nxt else None
    save_manifest(mpath, m)
    if nxt is None:
        _log("the run is complete: %d captured, %d skipped. Stop here." % (len(live_captures(m)), len(m["skipped"])))
        return EXIT_OK
    _log(lab_steps.format_instructions(nxt, daw, len(m["steps"]), i + 2))
    return EXIT_OK


def cmd_status(args) -> int:
    daw, run = resolve_run(args)
    m = load_manifest(manifest_path(daw, run))
    captured = {c["key"]: c for c in live_captures(m)}
    skipped = {s["key"]: s for s in m["skipped"]}
    chosen = m["app"]["chosen"] or {}
    _log("%s/%s — %s %s (build %s), %s %s, started %s" % (
        daw, run, chosen.get("bundle"), chosen.get("short_version"), chosen.get("build"),
        m["os"].get("platform"), m["arch"], m["started_at"]))
    _log("project: <lab root>/%s/%s" % (m["run_dir"], m["project"]))
    for s in m["steps"]:
        mark = "✓" if s["key"] in captured else ("–" if s["key"] in skipped else " ")
        cur = "→" if s["key"] == m["current_step"] else " "
        extra = ""
        if s["key"] in captured:
            c = captured[s["key"]]
            differ = c["bytes_differ_from_previous"]
            extra = "  %s  %s" % (c["captured_at"], "first" if differ is None else ("changed" if differ else "identical"))
        elif s["key"] in skipped:
            extra = "  skipped: %s" % skipped[s["key"]]["reason"]
        _log(" %s%s %-4s %-24s%s" % (cur, mark, s["key"], s["id"], extra))
    _log("%d of %d captured, %d skipped" % (len(captured), len(m["steps"]), len(skipped)))
    if m["current_step"]:
        index = _step_index(m, m["current_step"])
        _log("")
        _log(lab_steps.format_instructions(m["steps"][index], daw, len(m["steps"]), index + 1))
    else:
        _log("the run is complete")
    return EXIT_OK


def build_parser() -> argparse.ArgumentParser:
    ap = argparse.ArgumentParser(description="Capture labelled lab saves into the Wit corpus.")
    sub = ap.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("init", help="validate and start a run")
    p.add_argument("--daw", required=True, choices=labcore.DAWS)
    p.add_argument("--run", required=True, help="run name, e.g. r1")
    p.add_argument("--project", required=True, help="the project you just Saved As into the lab run folder")
    p.add_argument("--app", help="pin the DAW .app bundle to record (default: found in /Applications)")
    p.set_defaults(fn=cmd_init)

    p = sub.add_parser("watch", help="wait for a stable, new save and capture it")
    p.add_argument("--daw", choices=labcore.DAWS)
    p.add_argument("--run")
    p.add_argument("--step", help="step key or id (default: the current step)")
    p.add_argument("--allow-identical", action="store_true", help="a new save with identical bytes is enough")
    p.add_argument("--replace", action="store_true", help="re-capture a step (the old capture is kept, superseded)")
    p.add_argument("--note", help="what the driver really did, if it differs from the instructions")
    p.add_argument("--stable-seconds", type=float, default=2.0)
    p.add_argument("--poll", type=float, default=0.25)
    p.add_argument("--timeout", type=float, default=300.0)
    p.set_defaults(fn=cmd_watch)

    p = sub.add_parser("next", help="advance to the next step and print its instructions")
    p.add_argument("--daw", choices=labcore.DAWS)
    p.add_argument("--run")
    p.add_argument("--skip", metavar="REASON", help="skip the current step, recording why")
    p.set_defaults(fn=cmd_next)

    p = sub.add_parser("status", help="print run progress")
    p.add_argument("--daw", choices=labcore.DAWS)
    p.add_argument("--run")
    p.set_defaults(fn=cmd_status)
    return ap


def main(argv=None) -> int:
    args = build_parser().parse_args(argv)
    try:
        return args.fn(args)
    except labcore.SafetyError as exc:
        print("REFUSED: %s" % exc, file=sys.stderr)
        return EXIT_REFUSED
    except (FileNotFoundError, FileExistsError, ValueError, KeyError) as exc:
        print("error: %s" % labcore.redact(exc), file=sys.stderr)
        return EXIT_ERROR


if __name__ == "__main__":
    sys.exit(main())
