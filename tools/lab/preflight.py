"""
preflight — READ-ONLY checks, before a DAW is launched, that it will not reopen a real project.

WHAT THIS DOES
    Logic 12.3.1 migrates a project's files when it saves it, so the lab must never
    let a DAW auto-open one of Sepehr's real projects at launch. Before each DAW is
    launched, this reads what can be read without writing anything and reports,
    per DAW, a verdict and the evidence behind it:

    Logic / GarageBand  `defaults export <domain> -` (XML on stdout — the same read
                        path as `defaults read`, but parseable) for com.apple.logic10,
                        com.apple.mobilelogic (the Creator Studio build's domain) and
                        com.apple.garageband10:
                          startupAction         the Startup Action setting. Its values
                                                are NOT documented; the meaning table
                                                below is inferred (see STARTUP_ACTION)
                          unsavedAutosavedURLs  documents the DAW may offer to reopen
                          NSQuitAlwaysKeepsWindows + a Saved Application State
                                                folder: macOS window restoration
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
import plistlib
import re
import struct
import subprocess
import sys
import urllib.parse
import xml.etree.ElementTree as ET
from pathlib import Path
from typing import Callable, Dict, List, Optional

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
            with open(str(path), "rb") as fh:
                raw = fh.read()
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


def check_apple(daw: str, home: Path, lab_real: str, runner: Callable,
                installed_ids: Optional[set] = None) -> List[Dict]:
    """
    `installed_ids` are the bundle ids of the installed apps for this DAW. An absent
    domain matters only for an app that is installed (it would start on factory
    defaults); for an app that is not installed it is information. With no app
    found at all, every absent domain counts (conservative).
    """
    out = []
    global_keep = read_global("NSQuitAlwaysKeepsWindows", runner)
    for domain in APPLE_DOMAINS[daw]:
        prefs = export_domain(domain, runner)
        if prefs is None:
            relevant = not installed_ids or domain in installed_ids
            out.append(finding(domain, "preferences",
                               "domain absent or unreadable (never launched?) — factory defaults apply"
                               if relevant else "domain absent, and no installed app uses it",
                               "unknown" if relevant else None, "measured"))
        else:
            out.append(startup_action_finding(domain, prefs.get("startupAction")))
            urls = [str(u) for u in (prefs.get("unsavedAutosavedURLs") or [])]
            c = count_places(urls, lab_real)
            verdict = "would_reopen" if (c["outside"] or c["unknown"]) else "safe"
            out.append(finding(domain, "unsavedAutosavedURLs", describe_counts(c), verdict, "measured"))
            keep = prefs.get("NSQuitAlwaysKeepsWindows")
            keeps = bool(keep) if keep is not None else global_keep in ("1", "true", "YES")
            state = home / "Library" / "Saved Application State" / (domain + ".savedState")
            try:
                has_state = state.is_dir()
            except OSError:
                has_state = False
            if not has_state:
                out.append(finding(domain, "window restoration", "no saved window state", "safe", "measured"))
            elif keeps:
                out.append(finding(domain, "window restoration",
                                   "saved window state exists and windows are kept on quit — macOS may reopen "
                                   "the last open projects", "would_reopen", "measured"))
            else:
                out.append(finding(domain, "window restoration",
                                   "saved window state exists (restoration is off, but macOS can offer it after "
                                   "a crash)", "unknown", "measured"))
        rd = recent_documents(domain, home, lab_real)
        detail = describe_counts(rd["counts"]) if rd["readable"] else rd["why"]
        out.append(finding(domain, "recent documents", detail, None, "measured"))
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
        raw = cfg.read_bytes()
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
        root = ET.parse(str(reg)).getroot()
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
