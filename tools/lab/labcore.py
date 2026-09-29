"""
labcore — the Rosetta lab's shared paths and hard safety rules.

WHAT THIS DOES
    One place that every tool in tools/lab/ uses to decide where it may read and
    where it may write, so the safety rules cannot drift between scripts:

    - The lab root is $WIT_LAB_ROOT (default ~/WitLab). Lab projects live in
      <lab root>/<daw>/<run>/ and are the ONLY projects any lab tool reads.
    - The corpus is $WIT_CORPUS (default ~/Projects/DAW/wit-corpus). capture.py
      writes only there; make_audio.py writes only under the lab root.
    - A path is judged by its realpath (symlinks resolved), so a symlink planted
      inside the lab root that points at a real project is refused — AND by
      filesystem identity (st_dev, st_ino) of every existing ancestor, because
      realpath does not undo macOS firmlinks: /System/Volumes/Data/Users/<you>/Music
      IS ~/Music (same inode), yet realpath leaves the long form alone (measured).
    - Nothing may resolve under a real library (DENYLIST_REL below, relative to
      the home directory), compared case-insensitively and Unicode-NFC-normalised
      because macOS volumes are case-insensitive by default.
    - The lab root itself may not be the home directory, the filesystem root, an
      ancestor of home, inside a real library, or contain one — otherwise "inside
      the lab root" would stop meaning "a throwaway project".
    - The corpus may not sit inside the lab root (capture never writes where a
      DAW is saving), inside a real library, or inside this git repository (CI
      guardrails forbid project files in the repo).
    - Neither the lab root nor the corpus may sit anywhere below a folder holding
      a `.git` entry: agent worktrees live inside the main clone, so "outside this
      checkout" is not enough.
    - Only regular files are ever opened (O_NONBLOCK + a regular-file check), so a
      FIFO or device named like a project file cannot hang a capture.

    Also: atomic file writes (temp file + fsync + rename), sha256 of a file read
    without following symlinks, redaction of home paths in messages, and a
    read-only lookup of the installed DAW apps (globbing /Applications, or
    $WIT_LAB_APPS_DIR, and reading each bundle's Info.plist).

USAGE
    Imported by capture.py, make_audio.py, preflight.py and analyze.py. It is not
    a command-line tool. Every function reads Path.home() and the environment at
    call time, so tests can point HOME / WIT_LAB_ROOT / WIT_CORPUS at a tmp dir.

WHAT THIS DOES NOT HANDLE
    - Hard links to FILES. A hard link inside the lab root to a file inside a real
      library is indistinguishable from an ordinary file (identity checks cover
      folders and firmlinks, not a file with two names). The lab never creates
      them; a DAW does not either.
    - A home directory that is itself a git checkout (a dotfiles repo) makes every
      lab root and corpus under it refused; set WIT_LAB_ROOT / WIT_CORPUS elsewhere.
    - Check-then-use races. Paths are re-validated on every capture and files are
      opened with O_NOFOLLOW where the OS supports it, so a symlink swapped in at
      the last moment fails to open instead of being read — but a directory
      renamed between the check and the read is not detected.
    - Case-sensitive volumes. The denylist comparison is case-insensitive, which
      can only refuse MORE, never less. The "inside the lab root" check is exact.
    - Windows paths. The lab runs on macOS; nothing here is tested elsewhere.
"""

from __future__ import annotations

import datetime as _dt
import hashlib
import json
import os
import plistlib
import re
import stat
import tempfile
import unicodedata
from pathlib import Path
from typing import Dict, List, Optional, Set, Tuple

LAB_ROOT_ENV = "WIT_LAB_ROOT"
CORPUS_ENV = "WIT_CORPUS"
APPS_DIR_ENV = "WIT_LAB_APPS_DIR"  # tests point this at a fake /Applications

DAWS = ("logic", "ableton", "fl", "garageband")

# Real libraries, relative to the home directory. Never read, never written, never
# opened by a lab tool. The first six are the plan's list (PLAN-V2, Phase B, "Hard
# safety rules"); "Projects/DAW/Ableton" is the correctly-spelled twin of the real
# "Abelton" folder, added so a future rename cannot silently lift the protection.
DENYLIST_REL = (
    "Music/Logic",
    "Music/GarageBand",
    "Projects/DAW/Logic Pro",
    "Projects/DAW/Abelton",
    "Projects/DAW/Ableton",
    "Projects/DAW/FL Studio",
    "Documents/Image-Line/FL Studio/Projects",
)

# Anything that is audio or lives in a media folder is never copied into the
# corpus. The snapshot is a whitelist already; this is the second gate.
MEDIA_EXTS = {
    ".wav", ".aif", ".aiff", ".caf", ".flac", ".mp3", ".m4a", ".ogg", ".opus",
    ".wv", ".aac", ".sd2", ".rex", ".rx2", ".sf2", ".sfz", ".mid", ".midi",
    ".mov", ".mp4", ".jpg", ".jpeg", ".png", ".asd",
}
MEDIA_DIRS = {
    "Media", "Audio Files", "Samples", "Undo Data.nosync", "Freeze", "Freeze Files",
    "Consolidate", "Bounces", "Ableton Project Info", "Recorded", "Imported",
    "Processed", "Trash", "Impulse Responses", "Sampler Instruments",
}


class SafetyError(Exception):
    """A lab safety rule would be broken. Callers exit non-zero with the message."""


# --------------------------------------------------------------------------- #
# locations
# --------------------------------------------------------------------------- #


def home() -> Path:
    return Path.home()


def lab_root() -> Path:
    raw = os.environ.get(LAB_ROOT_ENV, "").strip()
    return Path(raw).expanduser() if raw else home() / "WitLab"


def corpus_root() -> Path:
    raw = os.environ.get(CORPUS_ENV, "").strip()
    return Path(raw).expanduser() if raw else home() / "Projects" / "DAW" / "wit-corpus"


def apps_dir() -> Path:
    raw = os.environ.get(APPS_DIR_ENV, "").strip()
    return Path(raw).expanduser() if raw else Path("/Applications")


def repo_root() -> Path:
    """The git checkout this file lives in (tools/lab/labcore.py -> repo root)."""
    return Path(__file__).resolve().parents[2]


def denylist() -> List[Path]:
    h = home()
    return [h / rel for rel in DENYLIST_REL]


# --------------------------------------------------------------------------- #
# path comparison
# --------------------------------------------------------------------------- #


def _nfc(text: str) -> str:
    return unicodedata.normalize("NFC", text)


def real(path) -> str:
    """realpath, NFC-normalised. Resolves every symlink on the way."""
    return _nfc(os.path.realpath(os.path.expanduser(str(path))))


def absolute(path) -> str:
    """abspath without resolving symlinks, NFC-normalised."""
    return _nfc(os.path.abspath(os.path.expanduser(str(path))))


def is_within(child: str, parent: str, casefold: bool = False) -> bool:
    """True if `child` is `parent` or anywhere below it (string paths, already absolute)."""
    c, p = child, parent
    if casefold:
        c, p = c.casefold(), p.casefold()
    p = p.rstrip(os.sep) or os.sep
    if p == os.sep:
        return c.startswith(os.sep)
    return c == p or c.startswith(p + os.sep)


def stat_id(path) -> Optional[Tuple[int, int]]:
    """(st_dev, st_ino) of an existing path, following symlinks and firmlinks; else None."""
    try:
        st = os.stat(str(path))
    except OSError:
        return None
    return (st.st_dev, st.st_ino)


def ancestor_ids(path) -> Set[Tuple[int, int]]:
    """Identity of `path`'s realpath and of every existing folder above it."""
    ids = set()
    p = real(path)
    while True:
        i = stat_id(p)
        if i is not None:
            ids.add(i)
        parent = os.path.dirname(p)
        if parent == p:
            return ids
        p = parent


def same_or_inside(path, parent) -> bool:
    """True if `path` is `parent` or below it by filesystem identity (catches firmlinks)."""
    pid = stat_id(parent)
    return pid is not None and pid in ancestor_ids(path)


def denylisted(path) -> Optional[str]:
    """
    The real library `path` falls under, as a "~/..." label, or None.

    Both the literal absolute path and its realpath are checked against both
    forms of each denylist entry, so neither a symlink in the candidate nor a
    symlinked library (e.g. ~/Music/Logic on an external disk) slips through;
    then filesystem identity is checked, so a firmlink alias cannot either.
    """
    forms = {absolute(path), real(path)}
    for rel, entry in zip(DENYLIST_REL, denylist()):
        entry_forms = {absolute(entry), real(entry)}
        for f in forms:
            for e in entry_forms:
                if is_within(f, e, casefold=True):
                    return "~/" + rel
        if same_or_inside(path, entry):
            return "~/" + rel
    return None


def contains_denylisted(path) -> Optional[str]:
    """The real library that lives (or would live) somewhere below `path`, or None."""
    forms = {absolute(path), real(path)}
    pid = stat_id(path)
    for rel, entry in zip(DENYLIST_REL, denylist()):
        for e in (absolute(entry), real(entry)):
            for f in forms:
                if is_within(e, f, casefold=True):
                    return "~/" + rel
        if pid is not None and pid in ancestor_ids(entry):
            return "~/" + rel
    return None


def git_checkout_above(path) -> Optional[str]:
    """The nearest folder at or above `path`'s realpath that holds a `.git` entry."""
    p = real(path)
    while True:
        if os.path.lexists(os.path.join(p, ".git")):
            return p
        parent = os.path.dirname(p)
        if parent == p:
            return None
        p = parent


# A home path counts only when it ends at a path boundary: a home of "/path/to/sam"
# must not turn "/path/to/samantha/x" into "~antha/x". Sentence punctuation right after
# it ("... in /path/to/sam.") is a boundary; "/path/to/sam.old/x" is a different folder.
_BOUNDARY = r"(?=$|[/\s\"'<>|(),:;\[\]{}]|[.!?](?:$|\s))"


def redact(text) -> str:
    """Home directory -> "~", any other /Users/<name> or /home/<name> -> <redacted>."""
    s = str(text)
    for h in sorted({absolute(home()), real(home())}, key=len, reverse=True):
        if h and h != os.sep:
            s = re.sub(re.escape(h) + _BOUNDARY, "~", s)
    return re.sub(r"(/Users/|/home/)[^/\s\"']+", r"\1<redacted>", s)


# --------------------------------------------------------------------------- #
# the rules
# --------------------------------------------------------------------------- #


def check_lab_root() -> Path:
    """Validate the lab root and return its realpath. Raises SafetyError."""
    root = lab_root()
    r = real(root)
    h = real(home())
    if r == os.sep or is_within(h, r) or same_or_inside(home(), root):
        raise SafetyError(
            "lab root %s is the home directory, the filesystem root or an ancestor of home — "
            "set %s to a dedicated folder such as ~/WitLab" % (redact(root), LAB_ROOT_ENV)
        )
    git = git_checkout_above(root)
    if git:
        raise SafetyError(
            "lab root %s is inside a git checkout (%s). DAW projects must never sit in or under a "
            "repository; set %s outside it." % (redact(root), redact(git), LAB_ROOT_ENV)
        )
    hit = denylisted(root)
    if hit:
        raise SafetyError(
            "lab root %s is inside %s, a real library. The lab only ever works on throwaway "
            "projects in its own folder." % (redact(root), hit)
        )
    hit = contains_denylisted(root)
    if hit:
        raise SafetyError(
            "lab root %s contains %s, a real library — it is too broad to be a lab root"
            % (redact(root), hit)
        )
    return Path(r)


def check_run_name(name: str, what: str = "run") -> str:
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,63}", name or ""):
        raise SafetyError(
            "%s name %r must be 1-64 characters of letters, digits, '.', '_' or '-' "
            "(no slashes)" % (what, name)
        )
    return name


def check_daw(daw: str) -> str:
    if daw not in DAWS:
        raise SafetyError("unknown DAW %r (expected one of: %s)" % (daw, ", ".join(DAWS)))
    return daw


def run_dir(daw: str, run: str) -> Path:
    """<lab root>/<daw>/<run>, validated to resolve inside the lab root."""
    root = check_lab_root()
    check_daw(daw)
    check_run_name(run)
    d = lab_root() / daw / run
    hit = denylisted(d)
    if hit:
        raise SafetyError("run folder %s resolves into %s, a real library" % (redact(d), hit))
    if not is_within(real(d), str(root)):
        raise SafetyError(
            "run folder %s resolves outside the lab root %s (a symlink?) — refusing"
            % (redact(d), redact(root))
        )
    return Path(real(d))


def check_readable_lab_path(path, daw: str, run: str) -> Path:
    """
    Validate that `path` may be READ by a lab tool: it resolves strictly inside
    <lab root>/<daw>/<run>/ and not under a real library. Returns the realpath.
    """
    hit = denylisted(path)
    if hit:
        raise SafetyError(
            "refusing to touch %s: it is under %s, a real library. The lab only reads "
            "throwaway projects inside %s/<daw>/<run>/." % (redact(path), hit, redact(lab_root()))
        )
    base = run_dir(daw, run)
    p = real(path)
    if not is_within(p, str(base)) or p == str(base):
        raise SafetyError(
            "refusing: %s resolves outside the lab run folder %s. Projects must be saved into "
            "%s/%s/%s/ (Save As), and symlinks out of the lab are not followed."
            % (redact(path), redact(base), redact(lab_root()), daw, run)
        )
    return Path(p)


def check_lab_write_dir(path) -> Path:
    """A directory make_audio may write into: under the lab root, not a real library."""
    root = check_lab_root()
    hit = denylisted(path)
    if hit:
        raise SafetyError("refusing to write into %s: it is under %s" % (redact(path), hit))
    p = real(path)
    if not is_within(p, str(root)):
        raise SafetyError(
            "refusing to write into %s: it is outside the lab root %s"
            % (redact(path), redact(root))
        )
    return Path(p)


def check_corpus_dir(path=None) -> Path:
    """The corpus (or a folder inside it) capture may write into. Returns realpath."""
    target = Path(path) if path is not None else corpus_root()
    c = real(target)
    h = real(home())
    if c == os.sep or is_within(h, c) or same_or_inside(home(), target):
        raise SafetyError(
            "corpus %s is the home directory, the filesystem root or an ancestor of home — "
            "set %s to a dedicated folder" % (redact(target), CORPUS_ENV)
        )
    hit = denylisted(target)
    if hit:
        raise SafetyError("corpus %s is inside %s, a real library" % (redact(target), hit))
    lab = real(lab_root())
    if is_within(c, lab, casefold=True) or same_or_inside(target, lab_root()):
        raise SafetyError(
            "corpus %s is inside the lab root %s. Capture never writes where a DAW is saving; "
            "keep the corpus outside it." % (redact(target), redact(lab_root()))
        )
    repo = real(repo_root())
    if is_within(c, repo, casefold=True) or same_or_inside(target, repo_root()):
        raise SafetyError(
            "corpus %s is inside this git repository. DAW project files must never enter the "
            "repo (CI guardrails); keep the corpus outside it." % redact(target)
        )
    git = git_checkout_above(target)
    if git:
        raise SafetyError(
            "corpus %s is inside a git checkout (%s). DAW project files must never enter a "
            "repository; set %s outside it." % (redact(target), redact(git), CORPUS_ENV)
        )
    return Path(c)


# --------------------------------------------------------------------------- #
# media filter
# --------------------------------------------------------------------------- #


def is_media(relpath: str) -> bool:
    """True for anything that is audio/video/artwork or sits in a media folder."""
    parts = Path(relpath).parts
    if any(part in MEDIA_DIRS for part in parts[:-1]):
        return True
    return Path(relpath).suffix.lower() in MEDIA_EXTS


# --------------------------------------------------------------------------- #
# io helpers
# --------------------------------------------------------------------------- #


def open_regular(path, follow_symlinks: bool = False):
    """
    Open a REGULAR file for reading. O_NONBLOCK makes opening a FIFO return at once
    instead of hanging, and the fstat check then refuses anything that is not a
    regular file. Without follow_symlinks, a final symlink is not followed either.
    """
    flags = os.O_RDONLY | getattr(os, "O_NONBLOCK", 0)
    if not follow_symlinks:
        flags |= getattr(os, "O_NOFOLLOW", 0)
    fd = os.open(str(path), flags)
    try:
        if not stat.S_ISREG(os.fstat(fd).st_mode):
            raise OSError("not a regular file: %s" % redact(path))
    except BaseException:
        os.close(fd)
        raise
    return os.fdopen(fd, "rb")


def open_nofollow(path):
    """open_regular without following a final symlink — every lab-project read uses this."""
    return open_regular(path, follow_symlinks=False)


def read_regular(path, follow_symlinks: bool = True) -> bytes:
    with open_regular(path, follow_symlinks) as fh:
        return fh.read()


def sha256_file(path) -> str:
    h = hashlib.sha256()
    with open_nofollow(path) as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def atomic_write_bytes(path, data: bytes) -> None:
    """Write via a temp file in the same folder, fsync, then rename over `path`."""
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(prefix="." + path.name + ".", suffix=".tmp", dir=str(path.parent))
    try:
        with os.fdopen(fd, "wb") as fh:
            fh.write(data)
            fh.flush()
            os.fsync(fh.fileno())
        os.replace(tmp, str(path))
    except BaseException:
        try:
            os.unlink(tmp)
        except FileNotFoundError:
            pass
        raise


def atomic_write_json(path, obj) -> None:
    # Serialise first: a value json cannot encode must fail before any file exists.
    data = (json.dumps(obj, indent=2, ensure_ascii=False) + "\n").encode("utf-8")
    atomic_write_bytes(path, data)


def now_iso() -> str:
    return _dt.datetime.now(_dt.timezone.utc).replace(microsecond=0).isoformat()


# --------------------------------------------------------------------------- #
# installed DAW apps (read-only: Info.plist only)
# --------------------------------------------------------------------------- #

# Looked up by glob, never by a hardcoded personal path. Measured 2026-09-29: the
# only Logic installed is /Applications/Logic Pro.app (12.3.1, com.apple.logic10);
# "Logic Pro Creator Studio.app" is not installed. Which Logic the lab uses still
# awaits Sepehr's confirmation (PLAN-V2); until then Logic Pro.app is recorded first,
# other Logic bundles are recorded as candidates, and `capture.py init --app` pins one.
APP_GLOBS = {
    "logic": ["Logic Pro.app", "Logic Pro*.app"],
    "garageband": ["GarageBand*.app"],
    "ableton": ["Ableton Live 12*.app", "Ableton Live*.app"],
    "fl": ["FL Studio 20*.app", "FL Studio*.app"],
}


def app_info(bundle: Path) -> Optional[Dict]:
    plist = bundle / "Contents" / "Info.plist"
    try:
        info = plistlib.loads(read_regular(plist))
    except (OSError, plistlib.InvalidFileException, ValueError):
        return None
    if not isinstance(info, dict):
        return None
    return {
        "bundle": bundle.name,
        "location": redact(bundle.parent),
        "bundle_id": info.get("CFBundleIdentifier"),
        "short_version": info.get("CFBundleShortVersionString"),
        "build": info.get("CFBundleVersion"),
        "name": info.get("CFBundleName"),
    }


def check_app_override(override: str) -> Path:
    """
    `capture.py init --app PATH`. Reading an Info.plist is read-only and cannot migrate
    a project, but --app is operator input, so it gets the same rule as every other
    path: nothing under a real library is ever read (not even a plist), and only a
    folder named *.app is accepted.
    """
    bundle = Path(override).expanduser()
    hit = denylisted(bundle)
    if hit:
        raise SafetyError("refusing --app %s: it is under %s, a real library" % (redact(bundle), hit))
    if bundle.suffix != ".app" or not bundle.is_dir():
        raise ValueError("--app must be an .app bundle folder, got %s" % redact(bundle))
    return bundle


def find_app(daw: str, override: Optional[str] = None) -> Dict:
    candidates: List[Path] = []
    for pattern in APP_GLOBS[daw]:
        for p in sorted(apps_dir().glob(pattern)):
            if p not in candidates:
                candidates.append(p)
    chosen = check_app_override(override) if override else (candidates[0] if candidates else None)
    infos = [i for i in (app_info(c) for c in candidates) if i]
    result = {"chosen": app_info(chosen) if chosen else None, "candidates": infos}
    if chosen is None:
        result["note"] = "no %s app found in %s" % (daw, redact(apps_dir()))
    elif len(candidates) > 1 and not override:
        result["note"] = "several candidates; the first was recorded — pass --app to pin one"
    return result
