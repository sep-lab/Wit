"""
preflight — READ-ONLY checks, before a DAW is launched, that it will not reopen a real project.

WHAT THIS DOES
    Logic 12.3.1 migrates a project's files when it saves it, so the lab must never
    let a DAW auto-open one of Sepehr's real projects at launch. Before each DAW is
    launched, this reads what can be read without writing anything and reports,
    per DAW, a verdict and the evidence behind it:

    Logic / GarageBand  `defaults export <domain> -` (XML on stdout — the same read
                        path as `defaults read`, but parseable), plus the plist files in
                        the app's SANDBOX CONTAINER (~/Library/Containers/<domain>/Data/
                        Library/Preferences — GarageBand is sandboxed) and in
                        ~/Library/Preferences, for com.apple.logic10 (the lab's Logic:
                        /Applications/Logic Pro.app 12.3.1), com.apple.garageband10 and
                        com.apple.mobilelogic (Creator Studio; not installed on the lab
                        Mac, so information only). Where sources disagree the worst wins:
                          startupAction         the Startup Action setting. Its values
                                                are NOT documented; the meaning table
                                                below is inferred (see STARTUP_ACTION)
                          unsavedAutosavedURLs  documents the DAW may offer to reopen
                          NSQuitAlwaysKeepsWindows + a savedState folder in
                                                ~/Library/Saved Application State OR in
                                                the sandbox container: window restoration
                        and the recent-documents list macOS keeps for the app (usually
                        unreadable: macOS privacy protection; reported as such).
    Ableton Live        ~/Library/Preferences/Ableton/Live <version>/: whether
                        Preferences.cfg holds any startup/reopen-like key (Live 12 has
                        no "open last Set" preference — inferred), whether Crash/ holds
                        recovery files (Live then offers to reopen the crashed Set), and
                        a count of .als paths it remembers.
    FL Studio           ~/Library/Preferences/Image-Line/reg.xml (FL's registry on
                        macOS) for the installed major version: the startup template,
                        the StartupOptionBox value (meaning unknown), the most-recent
                        list and the last project path; plus the defaults domain.

    Recent-document entries are reported ONLY as counts and as inside / outside the
    lab root. Actual paths are never printed (they show as <outside lab root>).

    Exit code: 0 = safe (every signal says no real project opens at launch),
    3 = would (or may) reopen a real project — ask Sepehr, 2 = unknown — ask Sepehr.
    With several DAWs the worst verdict wins (3 over 2 over 0). A "safe" that rests
    on an INFERRED meaning counts as unknown unless --accept-inferred is given.

USAGE
    python3 tools/lab/preflight.py                 # all four DAWs
    python3 tools/lab/preflight.py --daw logic     # one DAW; exit code for that DAW
    python3 tools/lab/preflight.py --json          # machine-readable
    python3 tools/lab/preflight.py --daw garageband --accept-inferred

WHAT THIS DOES NOT HANDLE
    - It never writes preferences: the only commands it can run are
      `defaults export <domain> -` and `defaults read ...` (run_readonly refuses
      anything else, and a test proves it). It never launches a DAW.
    - Logic's startupAction values are undocumented. STARTUP_ACTION is inferred
      from the order of the popup items in Logic 12.3.1's PrefsGlobal.nib (read
      from the app bundle) and from GarageBand's stored value 3 matching its
      visible behaviour (the project chooser). Two orderings are plausible, so a
      value is judged by BOTH; any value that could mean "Open Most Recent
      Project" is treated as would-reopen. An unset key means the factory
      default, which cannot be read without launching the app: unknown.
    - FL Studio's StartupOptionBox value is not mapped at all: FL is "unknown"
      until someone confirms the setting in FL's own settings window.
    - The macOS recent-documents store (com.apple.sharedfilelist) is privacy-
      protected; without Full Disk Access for the terminal it is unreadable and
      is reported as such rather than guessed.
    - A DAW's behaviour after a crash, or a project opened by double-clicking in
      Finder, is outside what preferences can predict.
"""

from __future__ import annotations

import argparse
import json
import os
import plistlib
import re
import struct
import subprocess
import sys
import urllib.parse
import xml.etree.ElementTree as ET
from pathlib import Path
from typing import Callable, Dict, List, Optional, Tuple

if not __package__:
    # Run as a file (python3 tools/lab/preflight.py): import this folder as the package
    # tools.lab so the relative imports below resolve (PEP 366). Relative imports keep
    # the CI stdlib-only check honest: it skips them, and they can only be siblings.
    sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
    __package__ = "tools.lab"

from . import labcore

EXIT_SAFE, EXIT_UNKNOWN, EXIT_REOPEN = 0, 2, 3
SEVERITY = {"safe": 0, "unknown": 1, "would_reopen": 2}
EXIT_FOR = {"safe": EXIT_SAFE, "unknown": EXIT_UNKNOWN, "would_reopen": EXIT_REOPEN}

APPLE_DOMAINS = {
    "logic": ["com.apple.logic10", "com.apple.mobilelogic"],
    "garageband": ["com.apple.garageband10"],
}

# startupAction value -> its two candidate meanings (inferred; see module docstring).
# Ordering A: Do Nothing, Open Most Recent, Open Existing, Select a Template,
#             Create New Empty, Create New Using Default Template.
# Ordering B: the same list with "Ask" in front.
STARTUP_ACTION = {
    0: ("Do Nothing", "Ask"),
    1: ("Open Most Recent Project", "Do Nothing"),
    2: ("Open Existing Project", "Open Most Recent Project"),
    3: ("Select a Template", "Open Existing Project"),
    4: ("Create New Empty Project", "Select a Template"),
    5: ("Create New Project Using Default Template", "Create New Empty Project"),
    6: (None, "Create New Project Using Default Template"),
}
REOPENS = {"Open Most Recent Project"}

ADVICE = {
    "logic": "Ask Sepehr to check Logic Pro ▸ Settings ▸ General ▸ Project Handling ▸ Startup Action "
             "(anything but 'Open Most Recent Project' is fine). While startupAction is unset in the "
             "preferences no flag can make this exit 0: his confirmation is the go-ahead. Once the value "
             "is stored, a value whose inferred meaning is safe passes with --accept-inferred.",
    "garageband": "GarageBand normally shows its project chooser at launch; if Sepehr confirms that, "
                  "re-run with --accept-inferred.",
    "ableton": "Live starts from its default template unless it is recovering from a crash; if Sepehr "
               "agrees, re-run with --accept-inferred.",
    "fl": "Ask Sepehr to check FL Studio's startup setting (Options ▸ General settings) — FL's stored "
          "value is not mapped by this script, so his confirmation is the go-ahead.",
}

SFL_DIR = "Library/Application Support/com.apple.sharedfilelist/com.apple.LSSharedFileList.ApplicationRecentDocuments"


# --------------------------------------------------------------------------- #
# read-only command runner
# --------------------------------------------------------------------------- #


def is_read_only(cmd: List[str]) -> bool:
    """The complete list of commands preflight may run. Everything else is refused."""
    if len(cmd) == 4 and cmd[:2] == ["defaults", "export"] and cmd[3] == "-":
        return True  # XML to stdout; "-" means no file is written
    if len(cmd) in (3, 4) and cmd[:2] == ["defaults", "read"]:
        return all(not a.startswith("-") or a == "-g" for a in cmd[2:])
    return False


def run_readonly(cmd: List[str], runner: Callable = subprocess.run):
    if not is_read_only(cmd):
        raise labcore.SafetyError("preflight only runs read-only commands; refused: %s" % " ".join(cmd))
    return runner(cmd, capture_output=True, timeout=30)


_BAD_XML = re.compile(rb"[\x00-\x08\x0b\x0c\x0e-\x1f]")


def export_domain(domain: str, runner: Callable) -> Optional[Dict]:
    """A defaults domain as a dict, or None if it does not exist / cannot be parsed."""
    try:
        res = run_readonly(["defaults", "export", domain, "-"], runner)
    except (OSError, subprocess.SubprocessError):
        return None
    out = res.stdout if isinstance(res.stdout, bytes) else (res.stdout or "").encode("utf-8")
    if res.returncode != 0 or not out.strip():
        return None
    try:
        # Logic's domain holds a few strings with raw control characters that
        # `defaults export` emits verbatim and XML forbids (measured on the lab Mac).
        data = plistlib.loads(_BAD_XML.sub(b"", out))
    except Exception:  # any parse failure means "unreadable", never a crash
        return None
    return data if isinstance(data, dict) and data else None


def read_global(key: str, runner: Callable) -> Optional[str]:
    try:
        res = run_readonly(["defaults", "read", "-g", key], runner)
    except (OSError, subprocess.SubprocessError):
        return None
    if res.returncode != 0:
        return None
    out = res.stdout.decode("utf-8", "replace") if isinstance(res.stdout, bytes) else (res.stdout or "")
    return out.strip()


# --------------------------------------------------------------------------- #
# evidence helpers
# --------------------------------------------------------------------------- #


def finding(source: str, signal: str, detail: str, verdict: Optional[str], basis: str) -> Dict:
    """verdict is "safe" / "unknown" / "would_reopen", or None for information only."""
    return {"source": source, "signal": signal, "detail": detail, "verdict": verdict, "basis": basis}


def _as_path(value: str) -> str:
    if value.startswith("file://"):
        value = urllib.parse.unquote(urllib.parse.urlparse(value).path)
    return labcore._nfc(value)


def place(value: str, lab_real: str) -> str:
    """ "inside" / "outside" the lab root — the only thing ever said about a remembered path."""
    p = _as_path(value)
    if not p.startswith("/"):
        return "unknown"
    return "inside" if labcore.is_within(labcore.real(p), lab_real) else "outside"


def count_places(values: List[str], lab_real: str) -> Dict[str, int]:
    counts = {"total": 0, "inside": 0, "outside": 0, "unknown": 0}
    for v in values:
        counts["total"] += 1
        counts[place(v, lab_real)] += 1
    return counts


def describe_counts(c: Dict[str, int]) -> str:
    parts = ["%d entr%s" % (c["total"], "y" if c["total"] == 1 else "ies")]
    if c["inside"]:
        parts.append("%d <inside lab root>" % c["inside"])
    if c["outside"]:
        parts.append("%d <outside lab root>" % c["outside"])
    if c["unknown"]:
        parts.append("%d <location unknown>" % c["unknown"])
    return ", ".join(parts)


def bookmark_path(blob: bytes) -> Optional[str]:
    """
    The file path inside a macOS bookmark ("book" blob), or None. Minimal reader:
    header size at +12, first TOC offset at data+0, TOC entries (key, offset,
    reserved), key 0x1004 = the path as an array of UTF-8 component strings.
    Validated against a bookmark made by NSURL on the lab Mac.
    """
    try:
        if blob[:4] != b"book":
            return None
        header = struct.unpack_from("<I", blob, 12)[0]
        data = blob[header:]
        toc = struct.unpack_from("<I", data, 0)[0]
        seen = set()
        while toc and toc not in seen:
            seen.add(toc)
            _size, magic, _ident, next_toc, count = struct.unpack_from("<IIIII", data, toc)
            if magic != 0xFFFFFFFE:
                return None
            for i in range(count):
                key, off, _ = struct.unpack_from("<III", data, toc + 20 + 12 * i)
                if key != 0x1004:
                    continue
                length, typ = struct.unpack_from("<II", data, off)
                if typ != 0x0601:
                    return None
                parts = []
                for j in range(length // 4):
                    item = struct.unpack_from("<I", data, off + 8 + 4 * j)[0]
                    slen, _styp = struct.unpack_from("<II", data, item)
                    parts.append(data[item + 8:item + 8 + slen].decode("utf-8"))
                return "/" + "/".join(parts)
            toc = next_toc
    except (struct.error, UnicodeDecodeError, IndexError):
        return None
    return None


def recent_documents(bundle_id: str, home: Path, lab_real: str) -> Dict:
    """Counts from macOS's per-app recent-documents list, or why it could not be read."""
    base = home / SFL_DIR
    for suffix in (".sfl3", ".sfl2"):
        path = base / (bundle_id + suffix)
        try:
            raw = labcore.read_regular(path)
        except FileNotFoundError:
            continue
        except PermissionError:
            return {"readable": False, "why": "privacy-protected (grant the terminal Full Disk Access to count)"}
        except OSError as exc:
            return {"readable": False, "why": "unreadable (%s)" % exc.__class__.__name__}
        try:
            archive = plistlib.loads(raw)
        except Exception:  # unparseable archive: report, do not crash
            return {"readable": False, "why": "unparseable"}
        blobs = [o for o in archive.get("$objects", []) if isinstance(o, bytes) and o[:4] == b"book"]
        paths = [bookmark_path(b) or "?" for b in blobs]
        return {"readable": True, "counts": count_places(paths, lab_real)}
    return {"readable": True, "counts": {"total": 0, "inside": 0, "outside": 0, "unknown": 0}}


# --------------------------------------------------------------------------- #
# per-DAW checks
# --------------------------------------------------------------------------- #


def startup_action_finding(domain: str, value) -> Dict:
    if value is None:
        return finding(domain, "startupAction",
                       "not set: the factory default applies and cannot be read without launching the app",
                       "unknown", "measured")
    candidates = STARTUP_ACTION.get(value) if isinstance(value, int) else None
    if candidates is None:
        return finding(domain, "startupAction", "value %r: meaning unknown" % (value,), "unknown", "measured")
    shown = " or ".join(c for c in candidates if c) or "?"
    if any(c in REOPENS for c in candidates):
        return finding(domain, "startupAction", "value %d: may mean %s" % (value, shown), "would_reopen", "inferred")
    if None in candidates:
        return finding(domain, "startupAction", "value %d: may mean %s (or unmapped)" % (value, shown),
                       "unknown", "inferred")
    return finding(domain, "startupAction", "value %d: means %s — neither opens an existing project"
                   % (value, shown), "safe", "inferred")


def container_library(home: Path, domain: str) -> Path:
    """A sandboxed app's ~/Library lives inside its container (GarageBand is sandboxed)."""
    return home / "Library" / "Containers" / domain / "Data" / "Library"


def read_plist_file(path: Path) -> Tuple[Optional[Dict], str]:
    """(prefs, status) from a plist file, read-only; status is ok / absent / unreadable."""
    try:
        raw = labcore.read_regular(path)
    except FileNotFoundError:
        return None, "absent"
    except OSError as exc:
        return None, "unreadable (%s)" % exc.__class__.__name__
    if not raw.startswith(b"bplist"):
        raw = _BAD_XML.sub(b"", raw)
    try:
        data = plistlib.loads(raw)
    except Exception:  # an unparseable plist is "unreadable", never a crash
        return None, "unreadable (unparseable)"
    return (data, "ok") if isinstance(data, dict) else (None, "unreadable (not a dictionary)")


def pref_sources(domain: str, home: Path, runner: Callable) -> List[Tuple[str, Optional[Dict], str]]:
    """
    Every place this domain's preferences can live: `defaults export` (cfprefsd —
    it resolves a sandboxed app's container itself), the sandbox container's plist
    and the plain ~/Library/Preferences plist. Where they disagree, the worst wins.
    """
    exported = export_domain(domain, runner)
    out = [("defaults export", exported, "ok" if exported else "absent or unreadable")]
    for label, path in (
        ("sandbox container plist", container_library(home, domain) / "Preferences" / (domain + ".plist")),
        ("~/Library/Preferences plist", home / "Library" / "Preferences" / (domain + ".plist")),
    ):
        prefs, status = read_plist_file(path)
        out.append((label, prefs, status))
    return out


def saved_state_finding(domain: str, home: Path, keeps: bool) -> Dict:
    """macOS window restoration: look in BOTH places a savedState can live."""
    name = domain + ".savedState"
    places = (
        ("~/Library/Saved Application State", home / "Library" / "Saved Application State" / name),
        ("the app's sandbox container", container_library(home, domain) / "Saved Application State" / name),
    )
    present, unreadable = [], []
    for label, path in places:
        try:
            os.stat(str(path))
            present.append(label)
        except FileNotFoundError:
            continue
        except OSError:
            unreadable.append(label)
    if present and keeps:
        return finding(domain, "window restoration",
                       "saved window state in %s and windows are kept on quit — macOS may reopen the last "
                       "open projects" % " and ".join(present), "would_reopen", "measured")
    if present:
        return finding(domain, "window restoration",
                       "saved window state in %s (restoration is off, but macOS can offer it after a crash)"
                       % " and ".join(present), "unknown", "measured")
    if unreadable:
        return finding(domain, "window restoration",
                       "could not look in %s (privacy-protected)" % " and ".join(unreadable), "unknown", "measured")
    return finding(domain, "window restoration",
                   "no saved window state in ~/Library/Saved Application State or the app's sandbox container",
                   "safe", "measured")


def check_apple(daw: str, home: Path, lab_real: str, runner: Callable,
                installed_ids: Optional[set] = None) -> List[Dict]:
    """
    `installed_ids` are the bundle ids of the installed apps for this DAW. The
    verdict rests on the domains of installed apps only: the lab launches
    /Applications/Logic Pro.app (com.apple.logic10), and a stale domain left by an
    app that is no longer installed (com.apple.mobilelogic on the lab Mac) is shown
    as information. With no app found at all, every domain counts (conservative).
    """
    out = []
    global_keep = read_global("NSQuitAlwaysKeepsWindows", runner)
    for domain in APPLE_DOMAINS[daw]:
        relevant = not installed_ids or domain in installed_ids
        found = []
        sources = pref_sources(domain, home, runner)
        readable = [(label, prefs) for label, prefs, _status in sources if prefs]
        found.append(finding(domain, "preference sources", "; ".join(
            "%s: %s" % (label, status) for label, _prefs, status in sources), None, "measured"))
        if not readable:
            found.append(finding(domain, "preferences",
                                 "absent or unreadable everywhere (never launched?) — factory defaults apply",
                                 "unknown", "measured"))
        else:
            values = []
            for label, prefs in readable:
                if "startupAction" in prefs and prefs["startupAction"] not in [v for v, _ in values]:
                    values.append((prefs["startupAction"], label))
            if not values:
                found.append(startup_action_finding(domain, None))
            else:
                worst = max((startup_action_finding(domain, v) for v, _ in values),
                            key=lambda f: SEVERITY[f["verdict"]])
                if len(values) > 1:
                    worst["detail"] += " (sources disagree: %s)" % ", ".join("%r in %s" % vl for vl in values)
                found.append(worst)
            urls = []
            for _label, prefs in readable:
                urls.extend(str(u) for u in (prefs.get("unsavedAutosavedURLs") or []))
            c = count_places(sorted(set(urls)), lab_real)
            found.append(finding(domain, "unsavedAutosavedURLs", describe_counts(c),
                                 "would_reopen" if (c["outside"] or c["unknown"]) else "safe", "measured"))
            keep_values = [prefs["NSQuitAlwaysKeepsWindows"] for _l, prefs in readable
                           if "NSQuitAlwaysKeepsWindows" in prefs]
            if keep_values:
                keeps = any(bool(v) for v in keep_values)
            else:
                keeps = global_keep in ("1", "true", "YES")
            found.append(saved_state_finding(domain, home, keeps))
        rd = recent_documents(domain, home, lab_real)
        found.append(finding(domain, "recent documents",
                             describe_counts(rd["counts"]) if rd["readable"] else rd["why"], None, "measured"))
        if not relevant:
            out.append(finding(domain, "not the lab's app",
                               "no installed app uses this domain — shown as information only", None, "measured"))
            for f in found:
                f["verdict"] = None
        out.extend(found)
    return out


_IDENT_ASCII = re.compile(rb"[A-Za-z][A-Za-z0-9_]{3,60}")
_UTF16 = re.compile(rb"(?:[\x20-\x7e]\x00){4,400}")
# Names a "reopen the last Set at launch" option would plausibly carry. Deliberately
# not "autoopen"/"lastopen" alone: Live 12.4.2 has AutoOpenCustomEditor (plugin
# windows) and LastOpenedHelpModuleType (help), neither about Sets (measured).
_STARTUPISH = re.compile(
    r"startup|reopen|openlast|loadlast|lastset|lastdocument|lastopened(set|document|project)",
    re.IGNORECASE,
)


def _version_key(name: str):
    return tuple(int(x) for x in re.findall(r"\d+", name))


def check_ableton(home: Path, lab_real: str, app: Optional[Dict]) -> List[Dict]:
    out = []
    prefs_root = home / "Library" / "Preferences" / "Ableton"
    dirs = sorted((d for d in prefs_root.glob("Live *") if d.is_dir()), key=lambda d: _version_key(d.name))
    if not dirs:
        return [finding("~/Library/Preferences/Ableton", "preferences", "no Live preferences folder",
                        "unknown", "measured")]
    want = None
    if app and app.get("short_version"):
        want = app["short_version"].split(" ")[0]
    chosen = next((d for d in dirs if d.name == "Live %s" % want), dirs[-1])
    src = "~/Library/Preferences/Ableton/%s" % chosen.name
    cfg = chosen / "Preferences.cfg"
    try:
        raw = labcore.read_regular(cfg)
    except OSError:
        out.append(finding(src, "Preferences.cfg", "missing or unreadable", "unknown", "measured"))
        raw = b""
    if raw:
        keys = {m.group(0).decode("ascii") for m in _IDENT_ASCII.finditer(raw)}
        strings = [m.group(0).decode("utf-16-le") for m in _UTF16.finditer(raw)]
        keys |= {s for s in strings if re.fullmatch(r"[A-Za-z][A-Za-z0-9_]{3,60}", s)}
        startupish = sorted(k for k in keys if _STARTUPISH.search(k))
        if startupish:
            out.append(finding(src, "startup option", "key(s) that look like a startup option: %s"
                               % ", ".join(startupish), "unknown", "measured"))
        else:
            out.append(finding(src, "startup option",
                               "no startup/reopen-like key in Preferences.cfg; Live 12 opens its default "
                               "template at launch", "safe", "inferred"))
        paths = [s for s in strings if s.startswith("/")]
        out.append(finding(src, "remembered absolute paths (recent Sets, folders)",
                           describe_counts(count_places(paths, lab_real)), None, "measured"))
    crash = chosen / "Crash"
    files = [f for f in crash.rglob("*") if f.is_file() and not f.name.startswith(".")] if crash.is_dir() else []
    if files:
        out.append(finding(src, "crash recovery", "%d file(s) in Crash/: Live will offer to recover the Set "
                           "that was open when it crashed" % len(files), "would_reopen", "inferred"))
    else:
        out.append(finding(src, "crash recovery", "Crash/ is empty or absent", "safe", "measured"))
    return out


def _fl_key(root: ET.Element, major: str) -> Optional[ET.Element]:
    for key in root.iter("Key"):
        if key.get("Name") == "FL Studio %s" % major:
            return key
    return None


def _fl_value(key: ET.Element, *path: str) -> Optional[str]:
    node = key
    for name in path[:-1]:
        node = next((k for k in node.findall("Key") if k.get("Name") == name), None)
        if node is None:
            return None
    val = next((v for v in node.findall("Value") if v.get("Name") == path[-1]), None)
    return None if val is None else (val.text or "")


def check_fl(home: Path, lab_real: str, app: Optional[Dict], runner: Callable) -> List[Dict]:
    out = []
    prefs = export_domain("com.image-line.flstudio", runner)
    out.append(finding("com.image-line.flstudio", "defaults domain",
                       "%d key(s) (window/panel state only)" % len(prefs) if prefs else "absent", None, "measured"))
    major = "20"
    if app:
        m = re.search(r"(\d+)", app.get("bundle") or "") or re.match(r"(\d+)", app.get("build") or "")
        major = m.group(1) if m else major
    src = "~/Library/Preferences/Image-Line/reg.xml [FL Studio %s]" % major
    reg = home / "Library" / "Preferences" / "Image-Line" / "reg.xml"
    try:
        root = ET.fromstring(labcore.read_regular(reg))
    except (OSError, ET.ParseError):
        return [*out, finding(src, "registry", "missing or unparseable", "unknown", "measured")]
    key = _fl_key(root, major)
    if key is None:
        return [*out, finding(src, "registry", "no key for this version (never launched?)", "unknown",
                              "measured")]
    template = _fl_value(key, "General", "Template")
    if template and template.startswith("%FLStudioFactoryData%"):
        out.append(finding(src, "startup template", "a factory template", "safe", "measured"))
    elif template:
        out.append(finding(src, "startup template", "a user template (%s)" % ("<inside lab root>" if place(
            template, lab_real) == "inside" else "<outside lab root>"), "unknown", "measured"))
    else:
        out.append(finding(src, "startup template", "not set", "unknown", "measured"))
    opt = _fl_value(key, "General", "MIDIForm", "StartupOptionBox")
    out.append(finding(src, "StartupOptionBox", "value %s: meaning not mapped" % (opt if opt is not None else "absent"),
                       "unknown", "measured"))
    mru_key = next((k for k in key.findall("Key") if k.get("Name") == "MRU"), None)
    mru = [v.text for v in mru_key.findall("Value") if v.text] if mru_key is not None else []
    out.append(finding(src, "recent projects (MRU)", describe_counts(count_places(mru, lab_real)), None, "measured"))
    last = _fl_value(key, "General", "FruityLoopsMainForm", "LastProjectPath")
    out.append(finding(src, "last project folder", "empty" if not last else
                       "<%s lab root>" % place(last, lab_real), None, "measured"))
    return out


# --------------------------------------------------------------------------- #
# verdicts and output
# --------------------------------------------------------------------------- #


def effective(f: Dict, accept_inferred: bool) -> Optional[str]:
    if f["verdict"] == "safe" and f["basis"] == "inferred" and not accept_inferred:
        return "unknown"
    return f["verdict"]


def daw_verdict(findings: List[Dict], accept_inferred: bool) -> str:
    verdicts = [effective(f, accept_inferred) for f in findings]
    verdicts = [v for v in verdicts if v]
    return max(verdicts, key=lambda v: SEVERITY[v]) if verdicts else "unknown"


def run_checks(daws: List[str], home: Optional[Path] = None, runner: Callable = subprocess.run,
               accept_inferred: bool = False) -> Dict:
    home = home or labcore.home()
    lab_real = labcore.real(labcore.lab_root())
    report = {"lab_root": labcore.redact(labcore.lab_root()), "accept_inferred": accept_inferred, "daws": {}}
    for daw in daws:
        apps = labcore.find_app(daw)
        app = apps["chosen"]
        if daw in APPLE_DOMAINS:
            installed = {c["bundle_id"] for c in apps["candidates"] if c.get("bundle_id")}
            findings = check_apple(daw, home, lab_real, runner, installed)
        elif daw == "ableton":
            findings = check_ableton(home, lab_real, app)
        else:
            findings = check_fl(home, lab_real, app, runner)
        verdict = daw_verdict(findings, accept_inferred)
        report["daws"][daw] = {
            "app": app,
            "findings": findings,
            "verdict": verdict,
            "exit": EXIT_FOR[verdict],
            "advice": None if verdict == "safe" else ADVICE[daw],
        }
    worst = max((d["verdict"] for d in report["daws"].values()), key=lambda v: SEVERITY[v])
    report["verdict"] = worst
    report["exit"] = EXIT_FOR[worst]
    return report


def print_report(report: Dict) -> None:
    print("Preflight (read-only; no preference was written, no DAW was launched). Lab root: %s"
          % report["lab_root"])
    for daw, r in report["daws"].items():
        app = r["app"] or {}
        print("\n== %s: %s %s (build %s) ==" % (daw, app.get("bundle", "not installed"),
                                               app.get("short_version") or "", app.get("build")))
        for f in r["findings"]:
            tag = effective(f, report["accept_inferred"]) or "info"
            print("  [%-12s] %s — %s: %s (%s)" % (tag, f["source"], f["signal"], f["detail"], f["basis"]))
        print("  verdict: %s → exit %d" % (r["verdict"].upper().replace("_", " "), r["exit"]))
        if r["advice"]:
            print("  next: %s" % r["advice"])
    print("\noverall: %s → exit %d" % (report["verdict"].upper().replace("_", " "), report["exit"]))


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description="Read-only check that a DAW will not reopen a real project.")
    ap.add_argument("--daw", choices=[*labcore.DAWS, "all"], default="all")
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--accept-inferred", action="store_true",
                    help="treat a 'safe' resting on an inferred meaning as safe (after Sepehr confirms)")
    args = ap.parse_args(argv)
    daws = list(labcore.DAWS) if args.daw == "all" else [args.daw]
    try:
        report = run_checks(daws, accept_inferred=args.accept_inferred)
    except labcore.SafetyError as exc:
        print("REFUSED: %s" % exc, file=sys.stderr)
        return EXIT_UNKNOWN
    if args.json:
        print(json.dumps(report, indent=2, ensure_ascii=False))
    else:
        print_report(report)
    return report["exit"]


if __name__ == "__main__":
    sys.exit(main())
