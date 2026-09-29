"""
Tests for the Rosetta lab tools in tools/lab/ (capture, make_audio, steps, preflight,
analyze and their shared safety core, labcore).

The lab drives real DAWs on throwaway projects, next to a real music library that
Logic 12.3.1 would migrate if it ever saved into it. So the safety rules are what
these tests care about most: every refusal is exercised, including symlinks that
escape the lab and every denylisted library.

Hermetic by construction: every test runs with HOME, WIT_LAB_ROOT, WIT_CORPUS and
WIT_LAB_APPS_DIR pointed into tmp_path, and preflight gets a fake `defaults`. Nothing
here reads or writes the real ~/WitLab, the real corpus or a real preference. No
audio or project file is committed; every fixture is built in code.
"""

from __future__ import annotations

import ast
import gzip
import hashlib
import importlib
import json
import os
import plistlib
import random
import struct
import subprocess
import sys
import types
import unicodedata
import wave
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parent.parent
LAB_DIR = REPO / "tools" / "lab"
LAB_MODULES = ("labcore", "steps", "make_audio", "capture", "preflight", "analyze")
PERSIAN = unicodedata.normalize("NFC", "آواز")
MAGIC = b"\x23\x47\xc0\xab"


# --------------------------------------------------------------------------- #
# fixtures and builders
# --------------------------------------------------------------------------- #


@pytest.fixture(scope="module")
def lab():
    # tools/lab is the package tools.lab (relative imports inside), so the repo root
    # goes first on sys.path: a site-packages module called "tools" must not shadow it.
    if str(REPO) not in sys.path:
        sys.path.insert(0, str(REPO))
    mods = {name: importlib.import_module("tools.lab." + name) for name in LAB_MODULES}
    assert Path(mods["capture"].__file__).resolve().parent == LAB_DIR
    return types.SimpleNamespace(**mods)


@pytest.fixture
def env(tmp_path, monkeypatch):
    home = tmp_path / "home"
    home.mkdir()
    apps = tmp_path / "Applications"
    apps.mkdir()
    monkeypatch.setenv("HOME", str(home))
    monkeypatch.setenv("WIT_LAB_ROOT", str(home / "WitLab"))
    monkeypatch.setenv("WIT_CORPUS", str(home / "corpus"))
    monkeypatch.setenv("WIT_LAB_APPS_DIR", str(apps))
    assert Path.home() == home
    return types.SimpleNamespace(home=home, lab=home / "WitLab", corpus=home / "corpus", apps=apps)


def container(records, version=b"\xd0\x09"):
    """A ProjectData container: 24-byte root header + 36-byte-header records."""
    payload = b"".join(
        tag + b"\x00" * (0x1C - 4) + struct.pack("<I", len(body)) + b"\x00" * (0x24 - 0x20) + body
        for tag, body in records
    )
    return MAGIC + version + b"\x00" * 10 + struct.pack("<I", len(payload)) + b"\x00" * 4 + payload


BASE_RECORDS = [(b"gnoS", b"song-root"), (b"karT", b"\x01" * 16), (b"qeSM", b"seq"), (b"gRuA", b"region")]


def write(path: Path, data: bytes, mtime_ns=None) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)
    if mtime_ns is not None:
        os.utime(str(path), ns=(mtime_ns, mtime_ns))
    return path


def make_logic_package(run_dir: Path, name="Lab.logicx", project_data=None) -> Path:
    pkg = run_dir / name
    alt = pkg / "Alternatives" / "000"
    write(alt / "ProjectData", project_data if project_data is not None else container(BASE_RECORDS))
    write(alt / "MetaData.plist", plistlib.dumps({"NumberOfTracks": 1, "BeatsPerMinute": 120.0},
                                                 fmt=plistlib.FMT_BINARY))
    write(pkg / "Resources" / "ProjectInformation.plist", plistlib.dumps({"LastSavedFrom": "Logic Pro 12.3.1"}))
    # Everything below must never reach the corpus.
    write(alt / "DisplayState.plist", plistlib.dumps({"ui": 1}))
    write(alt / "WindowImage.jpg", b"\xff\xd8 not really a jpeg")
    write(alt / "Project File Backups" / "00" / "ProjectData", b"old backup")
    write(alt / "Undo Data.nosync" / "Trash" / "take.wav", b"RIFF deleted take")
    write(pkg / "Media" / "Audio Files" / "click-drums.wav", b"RIFF media")
    write(pkg / "Media" / "Samples" / "loop.aif", b"FORM media")
    return pkg


def fake_app(apps: Path, bundle: str, bundle_id: str, short: str, build: str) -> Path:
    app = apps / bundle
    (app / "Contents").mkdir(parents=True)
    with open(str(app / "Contents" / "Info.plist"), "wb") as fh:
        plistlib.dump({"CFBundleIdentifier": bundle_id, "CFBundleShortVersionString": short,
                       "CFBundleVersion": build, "CFBundleName": bundle[:-4]}, fh)
    return app


FAST = ["--stable-seconds", "0.2", "--poll", "0.02", "--timeout", "10"]


def run_cli(module, argv, capsys):
    rc = module.main(argv)
    out = capsys.readouterr()
    return rc, out.out, out.err


def tree_fingerprint(root: Path):
    out = {}
    for p in sorted(root.rglob("*")):
        if p.is_file() and not p.is_symlink():
            st = p.stat()
            out[str(p.relative_to(root))] = (st.st_size, st.st_mtime_ns, hashlib.sha256(p.read_bytes()).hexdigest())
    return out


# --------------------------------------------------------------------------- #
# the tools obey the repo's own rules for stdlib-only scripts
# --------------------------------------------------------------------------- #


def lab_scripts():
    return sorted(p for p in LAB_DIR.glob("*.py") if p.name != "__init__.py")


def test_lab_tools_are_stdlib_only_and_parse_as_python_3_9():
    import test_repo_hygiene as hygiene

    local = {p.stem for p in lab_scripts()}
    assert local, "no tools found under tools/lab"
    for script in lab_scripts():
        ast.parse(script.read_text(encoding="utf-8"), filename=str(script), feature_version=(3, 9))
        outside = sorted(m for m in hygiene.imported_top_level_modules(script)
                         if m not in local and not hygiene.is_stdlib(m))
        assert outside == [], "%s imports non-stdlib %s" % (script.name, outside)


def test_every_lab_tool_documents_what_it_does_how_to_run_it_and_its_limits():
    for script in lab_scripts():
        doc = ast.get_docstring(ast.parse(script.read_text(encoding="utf-8"))) or ""
        for section in ("WHAT THIS DOES", "USAGE", "WHAT THIS DOES NOT HANDLE"):
            assert section in doc, "%s: module docstring lacks %s" % (script.name, section)


# --------------------------------------------------------------------------- #
# safety: where the lab may read
# --------------------------------------------------------------------------- #


def test_init_refuses_a_project_outside_the_lab_root(lab, env, capsys):
    outside = make_logic_package(env.home / "Desktop")
    rc, _out, err = run_cli(lab.capture, ["init", "--daw", "logic", "--run", "r1", "--project", str(outside)], capsys)
    assert rc == lab.capture.EXIT_REFUSED
    assert "outside the lab run folder" in err
    assert not (env.corpus / "logic" / "r1" / "manifest.json").exists()


@pytest.mark.parametrize("rel", [
    "Music/Logic", "Music/GarageBand", "Projects/DAW/Logic Pro", "Projects/DAW/Abelton",
    "Projects/DAW/FL Studio", "Documents/Image-Line/FL Studio/Projects",
])
def test_init_refuses_every_real_library(lab, env, capsys, rel):
    real = make_logic_package(env.home / rel, name="Song.logicx")
    rc, _out, err = run_cli(lab.capture, ["init", "--daw", "logic", "--run", "r1", "--project", str(real)], capsys)
    assert rc == lab.capture.EXIT_REFUSED
    assert "real library" in err and ("~/" + rel) in err
    assert not env.corpus.exists()


@pytest.mark.parametrize("target_rel", ["Music/Logic", "Desktop"])
def test_a_symlinked_project_that_escapes_the_lab_is_refused(lab, env, capsys, target_rel):
    real = make_logic_package(env.home / target_rel, name="Song.logicx")
    run = env.lab / "logic" / "r1"
    run.mkdir(parents=True)
    os.symlink(str(real), str(run / "Lab.logicx"))
    rc, _out, err = run_cli(lab.capture, ["init", "--daw", "logic", "--run", "r1",
                                          "--project", str(run / "Lab.logicx")], capsys)
    assert rc == lab.capture.EXIT_REFUSED, err
    assert "REFUSED" in err
    assert not env.corpus.exists()


def test_a_symlinked_run_folder_into_a_real_library_is_refused(lab, env, capsys):
    library = env.home / "Music" / "Logic"
    make_logic_package(library, name="Song.logicx")
    (env.lab / "logic").mkdir(parents=True)
    os.symlink(str(library), str(env.lab / "logic" / "r1"))
    rc, _out, err = run_cli(lab.capture, ["init", "--daw", "logic", "--run", "r1",
                                          "--project", str(env.lab / "logic" / "r1" / "Song.logicx")], capsys)
    assert rc == lab.capture.EXIT_REFUSED
    assert "real library" in err or "outside the lab root" in err


def test_a_symlink_planted_inside_the_package_is_refused_by_watch(lab, env, capsys):
    real = make_logic_package(env.home / "Music" / "Logic", name="Song.logicx")
    pkg = make_logic_package(env.lab / "logic" / "r1")
    assert run_cli(lab.capture, ["init", "--daw", "logic", "--run", "r1", "--project", str(pkg)], capsys)[0] == 0
    pd = pkg / "Alternatives" / "000" / "ProjectData"
    pd.unlink()
    os.symlink(str(real / "Alternatives" / "000" / "ProjectData"), str(pd))
    before = tree_fingerprint(env.home / "Music")
    rc, _out, err = run_cli(lab.capture, ["watch", *FAST], capsys)
    assert rc == lab.capture.EXIT_REFUSED
    assert "symlink" in err
    assert not (env.corpus / "logic" / "r1" / "01-baseline").exists()
    assert json.loads((env.corpus / "logic" / "r1" / "manifest.json").read_text())["captures"] == []
    assert tree_fingerprint(env.home / "Music") == before


@pytest.mark.parametrize("root_rel", ["", "Music", "Projects", "Music/Logic/lab", "Documents"])
def test_a_lab_root_that_is_home_contains_or_sits_in_a_library_is_refused(lab, env, monkeypatch, root_rel):
    monkeypatch.setenv("WIT_LAB_ROOT", str(env.home / root_rel) if root_rel else str(env.home))
    with pytest.raises(lab.labcore.SafetyError):
        lab.labcore.check_lab_root()


def test_denylist_matching_is_case_insensitive(lab, env):
    assert lab.labcore.denylisted(env.home / "MUSIC" / "logic" / "x.logicx") == "~/Music/Logic"
    assert lab.labcore.denylisted(env.home / "Music" / "Logicx") is None  # a sibling, not a child
    assert lab.labcore.denylisted(env.lab / "logic" / "r1" / "Lab.logicx") is None


@pytest.mark.parametrize("where", ["lab", "repo", "home", "library"])
def test_the_corpus_may_not_live_in_the_lab_root_the_repo_home_or_a_library(lab, env, monkeypatch, where):
    target = {
        "lab": env.lab / "corpus",
        "repo": REPO / "wit-corpus-should-never-exist",
        "home": env.home,
        "library": env.home / "Music" / "Logic" / "corpus",
    }[where]
    monkeypatch.setenv("WIT_CORPUS", str(target))
    with pytest.raises(lab.labcore.SafetyError):
        lab.labcore.check_corpus_dir()
    assert not (REPO / "wit-corpus-should-never-exist").exists()


def test_capture_never_writes_inside_the_lab(lab, env, capsys):
    pkg = make_logic_package(env.lab / "logic" / "r1")
    before = tree_fingerprint(env.lab)
    assert run_cli(lab.capture, ["init", "--daw", "logic", "--run", "r1", "--project", str(pkg)], capsys)[0] == 0
    assert run_cli(lab.capture, ["watch", *FAST], capsys)[0] == 0
    assert run_cli(lab.capture, ["next"], capsys)[0] == 0
    assert run_cli(lab.capture, ["status"], capsys)[0] == 0
    assert tree_fingerprint(env.lab) == before
    assert sorted(p.name for p in env.lab.rglob("*") if p.name.startswith(".")) == []


# --------------------------------------------------------------------------- #
# the snapshot: project files only, never media
# --------------------------------------------------------------------------- #


def corpus_files(run_corpus: Path):
    return sorted(str(p.relative_to(run_corpus)) for p in run_corpus.rglob("*") if p.is_file())


def test_logic_snapshot_copies_the_three_project_files_and_lists_backups(lab, env, capsys):
    pkg = make_logic_package(env.lab / "logic" / "r1")
    assert run_cli(lab.capture, ["init", "--daw", "logic", "--run", "r1", "--project", str(pkg)], capsys)[0] == 0
    assert run_cli(lab.capture, ["watch", *FAST], capsys)[0] == 0
    run_corpus = env.corpus / "logic" / "r1"
    assert corpus_files(run_corpus) == [
        "01-baseline/Lab.logicx/Alternatives/000/MetaData.plist",
        "01-baseline/Lab.logicx/Alternatives/000/ProjectData",
        "01-baseline/Lab.logicx/Resources/ProjectInformation.plist",
        "manifest.json",
    ]
    cap = json.loads((run_corpus / "manifest.json").read_text())["captures"][0]
    assert [e["path"] for e in cap["listing"]] == ["Lab.logicx/Alternatives/000/Project File Backups/00/ProjectData"]
    for f in run_corpus.rglob("*"):
        assert not lab.labcore.is_media(str(f.relative_to(run_corpus))), f


def test_garageband_packages_are_snapshotted_like_logic(lab, env):
    run = env.lab / "garageband" / "r1"
    make_logic_package(run, name="Lab.band")
    entries = lab.capture.collect(run, "garageband")
    assert [(e["path"], e["role"]) for e in entries if e["role"] == "project"] == [
        ("Lab.band/Alternatives/000/MetaData.plist", "project"),
        ("Lab.band/Alternatives/000/ProjectData", "project"),
        ("Lab.band/Resources/ProjectInformation.plist", "project"),
    ]
    assert lab.capture.collect(run, "logic") == []  # a .band is not a Logic package


def test_ableton_snapshot_copies_the_set_and_lists_live_backups(lab, env, capsys):
    proj = env.lab / "ableton" / "r1" / "Lab Project"
    write(proj / "Lab.als", gzip.compress(b"<Ableton/>", mtime=0))
    write(proj / "Backup" / "Lab [2026-09-29 120000].als", gzip.compress(b"<Ableton old/>", mtime=0))
    write(proj / "Samples" / "Imported" / "click-drums.wav", b"RIFF")
    write(proj / "Samples" / "Processed" / "Crop" / "x.als", b"not a set")  # inside a media folder
    write(proj / "Ableton Project Info" / "Project8_1.cfg", b"cfg")
    assert run_cli(lab.capture, ["init", "--daw", "ableton", "--run", "r1", "--project", str(proj / "Lab.als")],
                   capsys)[0] == 0
    assert run_cli(lab.capture, ["watch", *FAST], capsys)[0] == 0
    run_corpus = env.corpus / "ableton" / "r1"
    assert corpus_files(run_corpus) == ["01-baseline/Lab Project/Lab.als", "manifest.json"]
    cap = json.loads((run_corpus / "manifest.json").read_text())["captures"][0]
    assert [e["path"] for e in cap["listing"]] == ["Lab Project/Backup/Lab [2026-09-29 120000].als"]


def test_fl_snapshot_copies_the_flp_and_its_backup_autosaves(lab, env, capsys):
    run = env.lab / "fl" / "r1"
    write(run / "Lab.flp", b"FLhd v1")
    write(run / "Backup" / "Lab (autosaved at 12-00-00).flp", b"FLhd autosave")
    write(run / "lab-bounces" / "Lab.wav", b"RIFF bounce")
    assert run_cli(lab.capture, ["init", "--daw", "fl", "--run", "r1", "--project", str(run / "Lab.flp")],
                   capsys)[0] == 0
    assert run_cli(lab.capture, ["watch", *FAST], capsys)[0] == 0
    run_corpus = env.corpus / "fl" / "r1"
    assert corpus_files(run_corpus) == [
        "01-baseline/Backup/Lab (autosaved at 12-00-00).flp",
        "01-baseline/Lab.flp",
        "manifest.json",
    ]
    cap = json.loads((run_corpus / "manifest.json").read_text())["captures"][0]
    assert {f["path"]: f["role"] for f in cap["files"]} == {
        "Backup/Lab (autosaved at 12-00-00).flp": "autosave", "Lab.flp": "project"}
    assert [e["path"] for e in cap["listing"]] == ["lab-bounces/Lab.wav"]  # listed, never copied


def test_an_fl_autosave_alone_does_not_count_as_a_changed_step(lab, env):
    run = env.lab / "fl" / "r1"
    write(run / "Lab.flp", b"FLhd v1", mtime_ns=10**18)
    write(run / "Backup" / "Lab (autosaved at 12-00-00).flp", b"one")
    entries = lab.capture.collect(run, "fl")
    hashes = lab.capture.hash_entries(run, entries)
    prev = [dict(e, sha256=hashes[e["path"]]) for e in entries if e["role"] != "listing"]
    write(run / "Backup" / "Lab (autosaved at 12-05-00).flp", b"two")
    entries = lab.capture.collect(run, "fl")
    cmp = lab.capture.compare(prev, entries, lab.capture.hash_entries(run, entries))
    assert cmp["bytes_differ"] is False and cmp["saved"] is False
    assert cmp["changed_files"] == ["Backup/Lab (autosaved at 12-05-00).flp"]


@pytest.mark.parametrize("rel, media", [
    ("Lab.logicx/Media/Audio Files/a.wav", True),
    ("Lab.logicx/Alternatives/000/Undo Data.nosync/Trash/b.wav", True),
    ("Lab Project/Samples/Imported/c.aif", True),
    ("x/y/take.flac", True),
    ("Lab.logicx/Alternatives/000/ProjectData", False),
    ("Lab Project/Lab.als", False),
    ("Backup/Lab.flp", False),
])
def test_is_media(lab, rel, media):
    assert lab.labcore.is_media(rel) is media


# --------------------------------------------------------------------------- #
# stability detection (fake clock: deterministic and instant)
# --------------------------------------------------------------------------- #


class FakeClock:
    def __init__(self):
        self.t = 0.0
        self.events = []  # (at, fn)

    def __call__(self):
        return self.t

    def at(self, t, fn):
        self.events.append((t, fn))

    def sleep(self, seconds):
        self.t += seconds
        for event in [e for e in self.events if e[0] <= self.t]:
            self.events.remove(event)
            event[1]()


def captured_state(lab, run, daw="logic"):
    entries = lab.capture.collect(run, daw)
    hashes = lab.capture.hash_entries(run, entries)
    return [dict(e, sha256=hashes[e["path"]]) for e in entries if e["role"] in lab.capture.COPIED_ROLES]


def test_watch_waits_for_the_save_to_settle_before_capturing(lab, env):
    run = env.lab / "logic" / "r1"
    pkg = make_logic_package(run)
    pd = pkg / "Alternatives" / "000" / "ProjectData"
    prev = captured_state(lab, run)
    clock = FakeClock()
    base = 2 * 10**18
    for i, t in enumerate((0.5, 1.0, 1.5), 1):  # a save that writes three times
        clock.at(t, lambda i=i: write(pd, b"v%d" % i * (10 * i), mtime_ns=base + i * 10**9))
    _entries, hashes, cmp, stable_for = lab.capture.wait_for_save(
        run, "logic", prev, allow_identical=False, stable_secs=2.0, poll=0.25, timeout=30,
        clock=clock, sleep=clock.sleep, log=lambda m: None)
    final = b"v3" * 30
    assert hashes["Lab.logicx/Alternatives/000/ProjectData"] == hashlib.sha256(final).hexdigest()
    assert clock.t >= 1.5 + 2.0, "captured before the last write had been stable for 2 s"
    assert stable_for >= 2.0
    assert cmp["bytes_differ"] is True
    assert cmp["changed_files"] == ["Lab.logicx/Alternatives/000/ProjectData"]


def test_the_first_capture_needs_only_stability(lab, env):
    run = env.lab / "logic" / "r1"
    make_logic_package(run)
    clock = FakeClock()
    _e, _h, cmp, _s = lab.capture.wait_for_save(run, "logic", None, False, stable_secs=2.0, poll=0.5,
                                               timeout=10, clock=clock, sleep=clock.sleep, log=lambda m: None)
    assert cmp["bytes_differ"] is None
    assert 2.0 <= clock.t < 3.5


def test_an_identical_resave_is_only_captured_when_identical_bytes_are_allowed(lab, env):
    run = env.lab / "logic" / "r1"
    pkg = make_logic_package(run)
    pd = pkg / "Alternatives" / "000" / "ProjectData"
    same = pd.read_bytes()
    prev = captured_state(lab, run)

    clock = FakeClock()
    clock.at(0.5, lambda: write(pd, same, mtime_ns=3 * 10**18))
    notes = []
    with pytest.raises(lab.capture.WatchTimeout):
        lab.capture.wait_for_save(run, "logic", prev, False, stable_secs=2.0, poll=0.25, timeout=8,
                                  clock=clock, sleep=clock.sleep, log=notes.append)
    assert any("identical" in n for n in notes)

    clock = FakeClock()
    _e, _h, cmp, _s = lab.capture.wait_for_save(run, "logic", prev, True, stable_secs=2.0, poll=0.25,
                                               timeout=8, clock=clock, sleep=clock.sleep, log=lambda m: None)
    assert cmp == {"bytes_differ": False, "saved": True, "changed_files": []}


def test_no_new_save_times_out_even_when_identical_bytes_are_allowed(lab, env):
    run = env.lab / "logic" / "r1"
    make_logic_package(run)
    prev = captured_state(lab, run)
    clock = FakeClock()
    with pytest.raises(lab.capture.WatchTimeout, match="no new save"):
        lab.capture.wait_for_save(run, "logic", prev, True, stable_secs=2.0, poll=0.25, timeout=6,
                                  clock=clock, sleep=clock.sleep, log=lambda m: None)


def test_watch_times_out_with_a_hint_and_exit_code_3(lab, env, capsys):
    pkg = make_logic_package(env.lab / "logic" / "r1")
    assert run_cli(lab.capture, ["init", "--daw", "logic", "--run", "r1", "--project", str(pkg)], capsys)[0] == 0
    assert run_cli(lab.capture, ["watch", *FAST], capsys)[0] == 0
    assert run_cli(lab.capture, ["next"], capsys)[0] == 0
    rc, out, _err = run_cli(lab.capture, ["watch", "--stable-seconds", "0.1", "--poll", "0.02", "--timeout", "0.5"],
                            capsys)
    assert rc == lab.capture.EXIT_TIMEOUT
    assert "TIMEOUT" in out and "~/Music" in out


# --------------------------------------------------------------------------- #
# the manifest
# --------------------------------------------------------------------------- #


def init_and_capture_two(lab, env, capsys):
    fake_app(env.apps, "Logic Pro.app", "com.apple.logic10", "12.3.1", "6682")
    pkg = make_logic_package(env.lab / "logic" / "r1")
    assert run_cli(lab.capture, ["init", "--daw", "logic", "--run", "r1", "--project", str(pkg)], capsys)[0] == 0
    assert run_cli(lab.capture, ["watch", *FAST], capsys)[0] == 0
    assert run_cli(lab.capture, ["next"], capsys)[0] == 0
    pd = pkg / "Alternatives" / "000" / "ProjectData"
    write(pd, pd.read_bytes(), mtime_ns=4 * 10**18)  # the pure-churn save: same bytes, new mtime
    assert run_cli(lab.capture, ["watch", "--note", "saved with ⌘S", *FAST], capsys)[0] == 0
    return pkg, env.corpus / "logic" / "r1"


def test_manifest_schema_after_init_and_two_captures(lab, env, capsys):
    _pkg, run_corpus = init_and_capture_two(lab, env, capsys)
    text = (run_corpus / "manifest.json").read_text(encoding="utf-8")
    m = json.loads(text)
    assert m["schema"] == "wit-lab-manifest/1"
    assert (m["daw"], m["run"], m["run_dir"], m["project"]) == ("logic", "r1", "logic/r1", "Lab.logicx")
    assert m["app"]["chosen"] == {"bundle": "Logic Pro.app", "location": m["app"]["chosen"]["location"],
                                  "bundle_id": "com.apple.logic10", "short_version": "12.3.1",
                                  "build": "6682", "name": "Logic Pro"}
    assert {"system", "release", "platform", "hardware_arch", "python_arch", "python"} <= set(m["os"])
    assert m["arch"] == m["os"]["hardware_arch"]
    assert m["started_at"].endswith("+00:00")
    assert [s["key"] for s in m["steps"]] == [s["key"] for s in lab.steps.for_daw("logic")]
    assert m["current_step"] == "02"
    first, second = m["captures"]
    assert (first["key"], first["label"], first["previous"], first["bytes_differ_from_previous"]) == \
        ("01", "01-baseline", None, None)
    assert (second["key"], second["previous"], second["bytes_differ_from_previous"],
            second["saved_since_previous"], second["allow_identical"], second["note"]) == \
        ("02", "01", False, True, True, "saved with ⌘S")
    assert second["expected"] == {"none": True}
    for cap in m["captures"]:
        for f in cap["files"]:
            copy = run_corpus / cap["label"] / f["path"]
            assert f["size"] == copy.stat().st_size
            assert f["sha256"] == hashlib.sha256(copy.read_bytes()).hexdigest()
            assert set(f) == {"path", "role", "size", "mtime_ns", "sha256", "differs_from_previous"}
    assert str(env.home) not in text, "the manifest must hold no absolute home path"


def test_manifest_writes_are_atomic(lab, env, capsys, monkeypatch):
    _pkg, run_corpus = init_and_capture_two(lab, env, capsys)
    mpath = run_corpus / "manifest.json"
    before = mpath.read_bytes()
    m = json.loads(before)

    def exploding_replace(src, dst):
        raise OSError("disk full")

    monkeypatch.setattr(lab.labcore.os, "replace", exploding_replace)
    with pytest.raises(OSError, match="disk full"):
        lab.capture.save_manifest(mpath, dict(m, captures=[]))
    monkeypatch.undo()
    assert mpath.read_bytes() == before
    with pytest.raises(TypeError):
        lab.capture.save_manifest(mpath, dict(m, oops=object()))
    assert mpath.read_bytes() == before
    assert sorted(p.name for p in run_corpus.iterdir() if p.name.startswith(".")) == []


def test_next_refuses_an_uncaptured_step_and_skip_records_why(lab, env, capsys):
    pkg = make_logic_package(env.lab / "logic" / "r1")
    assert run_cli(lab.capture, ["init", "--daw", "logic", "--run", "r1", "--project", str(pkg)], capsys)[0] == 0
    rc, out, _ = run_cli(lab.capture, ["next"], capsys)
    assert rc == lab.capture.EXIT_ERROR and "no capture yet" in out
    rc, out, _ = run_cli(lab.capture, ["next", "--skip", "testing skip"], capsys)
    assert rc == 0 and "Step 02" in out and "watch (identical bytes allowed" in out
    m = json.loads((env.corpus / "logic" / "r1" / "manifest.json").read_text())
    assert m["skipped"][0]["key"] == "01" and m["skipped"][0]["reason"] == "testing skip"
    assert m["current_step"] == "02"


def test_the_run_completes_after_the_last_step(lab, env, capsys):
    run = env.lab / "garageband" / "r1"
    make_logic_package(run, name="Lab.band")
    assert run_cli(lab.capture, ["init", "--daw", "garageband", "--run", "r1",
                                 "--project", str(run / "Lab.band")], capsys)[0] == 0
    n = len(lab.steps.for_daw("garageband"))
    for _ in range(n - 1):
        assert run_cli(lab.capture, ["next", "--skip", "fast-forward"], capsys)[0] == 0
    rc, out, _ = run_cli(lab.capture, ["next", "--skip", "last"], capsys)
    assert rc == 0 and "complete" in out
    rc, out, _ = run_cli(lab.capture, ["watch", *FAST], capsys)
    assert rc == lab.capture.EXIT_ERROR and "complete" in out


def test_recapturing_needs_replace_and_keeps_the_old_capture(lab, env, capsys):
    pkg, run_corpus = init_and_capture_two(lab, env, capsys)
    rc, _out, err = run_cli(lab.capture, ["watch", "--step", "02", *FAST], capsys)
    assert rc == lab.capture.EXIT_ERROR and "already captured" in err
    write(pkg / "Alternatives" / "000" / "ProjectData", container([*BASE_RECORDS, (b"karT", b"new")]))
    rc, _out, err = run_cli(lab.capture, ["watch", "--step", "02", "--replace", *FAST], capsys)
    assert rc == 0, err
    m = json.loads((run_corpus / "manifest.json").read_text())
    live = [c for c in m["captures"] if not c["superseded"]]
    old = [c for c in m["captures"] if c["superseded"]]
    assert [c["key"] for c in live] == ["01", "02"] and len(old) == 1
    assert (run_corpus / old[0]["superseded_dir"]).is_dir()
    assert live[1]["bytes_differ_from_previous"] is True


def test_init_refuses_to_reuse_a_run_name(lab, env, capsys):
    pkg = make_logic_package(env.lab / "logic" / "r1")
    args = ["init", "--daw", "logic", "--run", "r1", "--project", str(pkg)]
    assert run_cli(lab.capture, args, capsys)[0] == 0
    rc, _out, err = run_cli(lab.capture, args, capsys)
    assert rc == lab.capture.EXIT_ERROR and "already has a manifest" in err


@pytest.mark.parametrize("bad", ["../escape", "a/b", "", ".hidden", "x" * 70])
def test_run_names_cannot_escape_their_folder(lab, env, capsys, bad):
    rc, _out, err = run_cli(lab.capture, ["init", "--daw", "logic", "--run", bad, "--project", "/nonexistent"], capsys)
    assert rc == lab.capture.EXIT_REFUSED and "run name" in err


# --------------------------------------------------------------------------- #
# make_audio
# --------------------------------------------------------------------------- #


def test_make_audio_is_deterministic_per_seed(lab):
    ma = lab.make_audio
    for name in ma.FILES:
        assert ma.render(name, seed=3, scale=0.05) == ma.render(name, seed=3, scale=0.05)
    assert ma.render("click-drums.wav", seed=3, scale=0.05) != ma.render("click-drums.wav", seed=4, scale=0.05)
    assert ma.render("noise-pad.wav", seed=3, scale=0.05) != ma.render("noise-pad.wav", seed=4, scale=0.05)


def riff_header(data: bytes):
    riff, size, wave_id = struct.unpack_from("<4sI4s", data, 0)
    fmt, fmt_len, audio_format, channels, rate, byte_rate, block, bits = struct.unpack_from("<4sIHHIIHH", data, 12)
    data_id, data_len = struct.unpack_from("<4sI", data, 36)
    return {"riff": riff, "size": size, "wave": wave_id, "fmt": fmt, "fmt_len": fmt_len,
            "audio_format": audio_format, "channels": channels, "rate": rate, "byte_rate": byte_rate,
            "block": block, "bits": bits, "data_id": data_id, "data_len": data_len}


def test_make_audio_writes_valid_riff_files_including_the_persian_name(lab, env):
    out = env.lab / "audio"
    records = lab.make_audio.generate(out, seed=1, scale=0.05)
    names = sorted(unicodedata.normalize("NFC", p.name) for p in out.iterdir())
    assert names == sorted(["click-drums.wav", "sine-bass.wav", "noise-pad.wav", PERSIAN + ".wav", "make_audio.json"])
    for rec in records:
        data = (out / rec["name"]).read_bytes()
        h = riff_header(data)
        assert (h["riff"], h["wave"], h["fmt"], h["data_id"]) == (b"RIFF", b"WAVE", b"fmt ", b"data")
        assert (h["audio_format"], h["rate"], h["bits"]) == (1, 44100, 16)
        assert h["channels"] == rec["channels"] == (2 if rec["name"] == "noise-pad.wav" else 1)
        assert h["size"] == len(data) - 8 and h["data_len"] == len(data) - 44
        assert h["byte_rate"] == 44100 * 2 * h["channels"] and h["block"] == 2 * h["channels"]
        with wave.open(str(out / rec["name"])) as w:
            assert w.getnframes() * w.getnchannels() * 2 == h["data_len"]
        assert rec["sha256"] == hashlib.sha256(data).hexdigest()
    prov = json.loads((out / "make_audio.json").read_text(encoding="utf-8"))
    assert PERSIAN + ".wav" in [f["name"] for f in prov["files"]]


def test_make_audio_rerun_is_a_no_op_and_a_new_seed_needs_force(lab, env):
    out = env.lab / "audio"
    lab.make_audio.generate(out, seed=1, scale=0.05)
    before = tree_fingerprint(out)
    assert {r["status"] for r in lab.make_audio.generate(out, seed=1, scale=0.05)} == {"unchanged"}
    assert {k: v for k, v in tree_fingerprint(out).items() if k != "make_audio.json"} == \
        {k: v for k, v in before.items() if k != "make_audio.json"}
    with pytest.raises(lab.labcore.SafetyError, match="--force"):
        lab.make_audio.generate(out, seed=2, scale=0.05)
    assert "overwritten" in {r["status"] for r in lab.make_audio.generate(out, seed=2, force=True, scale=0.05)}


@pytest.mark.parametrize("where", ["Desktop", "Music/Logic/audio"])
def test_make_audio_refuses_to_write_outside_the_lab_root(lab, env, capsys, where):
    target = env.home / where
    rc, _out, err = run_cli(lab.make_audio, ["--out", str(target)], capsys)
    assert rc == 2 and "REFUSED" in err
    assert not target.exists()


@pytest.mark.slow
def test_full_length_files_have_the_documented_durations_and_never_clip(lab):
    expected = {"click-drums.wav": 8.0, "sine-bass.wav": 8.0, "noise-pad.wav": 8.0, PERSIAN + ".wav": 5.0}
    for name, seconds in expected.items():
        data = lab.make_audio.render(name, seed=1)
        h = riff_header(data)
        assert h["data_len"] / (44100 * 2 * h["channels"]) == seconds
        samples = struct.unpack("<%dh" % (h["data_len"] // 2), data[44:])
        assert max(abs(s) for s in samples) < 32767, "%s clips" % name


# --------------------------------------------------------------------------- #
# steps
# --------------------------------------------------------------------------- #


def test_the_edit_script_is_valid_and_covers_plan_rows_1_to_26(lab):
    assert lab.steps.validate() == []
    counts = {d: len(lab.steps.for_daw(d)) for d in ("logic", "ableton", "fl", "garageband")}
    assert counts["logic"] == len(lab.steps.STEPS)  # Logic runs everything
    assert "logic-alternative" not in [s["id"] for s in lab.steps.for_daw("ableton")]
    assert "key-d-minor" not in [s["id"] for s in lab.steps.for_daw("fl")]
    assert 8 <= counts["garageband"] <= 12  # the plan's "about 10"
    assert lab.steps.find("logic", "8a")["expected"] == {"track_renamed": {"from": "Drums", "to": PERSIAN}}
    assert lab.steps.find("logic", "14")["expected"] == {"volume_db": -6}
    assert lab.steps.find("ableton", "17a")["expected"] == {"pan": "L50"}
    assert lab.steps.find("fl", "save-no-change")["allow_identical"] is True


def test_every_step_label_is_a_safe_folder_name(lab):
    for daw in ("logic", "ableton", "fl", "garageband"):
        for s in lab.steps.for_daw(daw):
            assert lab.labcore.check_run_name(s["label"], "label") == s["label"]


# --------------------------------------------------------------------------- #
# analyze
# --------------------------------------------------------------------------- #


def test_walk_records_follows_the_container_framing(lab):
    a = lab.analyze
    data = container(BASE_RECORDS)
    recs = a.walk_records(data)
    assert [a.tag_text(t) for _o, t, _s in recs] == ["gnoS", "karT", "qeSM", "gRuA"]
    assert [a.tag_human(t) for _o, t, _s in recs] == ["Song", "Trak", "MSeq", "AuRg"]
    assert recs[0][0] == 0x18 and recs[1][0] == 0x18 + 0x24 + len(b"song-root")
    with pytest.raises(a.FramingError, match="magic"):
        a.walk_records(b"\x00" + data[1:])
    with pytest.raises(a.FramingError, match="LENGTH"):
        a.walk_records(data + b"\x00")
    bad = bytearray(data)
    bad[0x18 + 0x1C:0x18 + 0x20] = struct.pack("<I", 10**6)
    with pytest.raises(a.FramingError, match="past EOF"):
        a.walk_records(bytes(bad))


def test_analyze_localises_a_changed_byte_to_its_record_and_tag(lab, env, capsys):
    pkg = make_logic_package(env.lab / "logic" / "r1")
    assert run_cli(lab.capture, ["init", "--daw", "logic", "--run", "r1", "--project", str(pkg)], capsys)[0] == 0
    assert run_cli(lab.capture, ["watch", *FAST], capsys)[0] == 0
    assert run_cli(lab.capture, ["next"], capsys)[0] == 0
    changed = [(t, bytearray(b)) for t, b in BASE_RECORDS]
    changed[1][1][5] ^= 0xFF  # one byte of the karT payload
    write(pkg / "Alternatives" / "000" / "ProjectData", container([(t, bytes(b)) for t, b in changed]))
    assert run_cli(lab.capture, ["watch", *FAST], capsys)[0] == 0

    out_file = env.corpus / "logic" / "r1" / "analysis.jsonl"
    rc, _out, err = run_cli(lab.analyze, ["--out", str(out_file)], capsys)
    assert rc == 0, err
    rows = [json.loads(line) for line in out_file.read_text(encoding="utf-8").splitlines()]
    pd = next(r for r in rows if r["file"].endswith("ProjectData"))
    karT_payload = 0x18 + 0x24 + len(b"song-root") + 0x24
    assert pd["status"] == "changed" and pd["diff_bytes"] == 1
    assert pd["ranges"] == [[karT_payload + 5, karT_payload + 6]]
    loc = pd["localised"][0]["new"]
    assert (loc["record_index"], loc["tag"], loc["tag_human"], loc["in"], loc["payload_offset"]) == \
        (1, "karT", "Trak", "payload", 5)
    assert pd["records"]["same_layout"] is True
    assert pd["records"]["changed_records"] == [[1, "karT", 1]]
    assert pd["records"]["common_prefix"] == 1 and pd["records"]["common_suffix"] == 2
    assert {r["status"] for r in rows if not r["file"].endswith("ProjectData")} == {"unchanged"}
    assert (pd["from"], pd["to"], pd["expected"]) == ("01-baseline", "02-save-no-change", {"none": True})


def test_analyze_summarises_an_inserted_record(lab):
    old = container(BASE_RECORDS)
    new = container([*BASE_RECORDS[:2], (b"gRuA", b"new region"), *BASE_RECORDS[2:]])
    rec = lab.analyze.compare_file("Lab.logicx/Alternatives/000/ProjectData", old, new, 16)
    r = rec["records"]
    assert (r["old_records"], r["new_records"], r["common_prefix"], r["common_suffix"]) == (4, 5, 2, 2)
    assert r["between_new"] == [[2, "gRuA"]] and r["between_old"] == []
    assert r["census_delta"] == {"gRuA": 1} and r["same_layout"] is False


def test_analyze_reports_framing_errors_instead_of_crashing(lab):
    rec = lab.analyze.compare_file("x.logicx/Alternatives/000/ProjectData", b"junk-one", b"junk-two", 16)
    assert rec["status"] == "changed"
    assert "too short" in rec["framing_error"]  # 8 bytes cannot hold the 24-byte root header
    assert "localised" not in rec


def test_analyze_diffs_plists_by_key_and_redacts_paths(lab, env):
    old = plistlib.dumps({"NumberOfTracks": 1, "AudioFiles": ["a.wav"], "Note": "x"}, fmt=plistlib.FMT_BINARY)
    new = plistlib.dumps({"NumberOfTracks": 2, "AudioFiles": ["a.wav", "b.wav"],
                          "Note": str(env.home / "Music" / "private")}, fmt=plistlib.FMT_BINARY)
    rec = lab.analyze.compare_file("Lab.logicx/Alternatives/000/MetaData.plist", old, new, 16)
    changes = {c["key"]: c for c in rec["plist_changes"]}
    assert changes["NumberOfTracks"] == {"key": "NumberOfTracks", "old": 1, "new": 2}
    assert changes["AudioFiles"]["new"] == "<list 2>"
    assert str(env.home) not in json.dumps(rec) and changes["Note"]["new"].startswith("~")


def test_analyze_compares_ableton_sets_decompressed(lab):
    old = gzip.compress(b"<Tempo Value=\"120\"/>", mtime=1)
    new = gzip.compress(b"<Tempo Value=\"124\"/>", mtime=2)
    rec = lab.analyze.compare_file("Lab Project/Lab.als", old, new, 16)
    assert rec["domain"] == "gunzipped" and rec["ranges"] == [[16, 17]] and rec["diff_bytes"] == 1


def brute_ranges(a, b):
    ranges, start = [], None
    for i in range(max(len(a), len(b))):
        differ = i >= len(a) or i >= len(b) or a[i] != b[i]
        if differ and start is None:
            start = i
        if not differ and start is not None:
            ranges.append([start, i])
            start = None
    if start is not None:
        ranges.append([start, max(len(a), len(b))])
    return ranges


@pytest.mark.parametrize("seed", range(12))
def test_diff_ranges_matches_a_brute_force_comparison(lab, seed):
    rng = random.Random(seed)
    a = bytes(rng.getrandbits(8) for _ in range(rng.choice([0, 1, 63, 64, 65, 4095, 4096, 9000])))
    b = bytearray(a)
    for _ in range(rng.randint(0, 20)):
        if b:
            b[rng.randrange(len(b))] ^= 1 + rng.randrange(255)
    if rng.random() < 0.5:
        b = b[:rng.randint(0, len(b))] if rng.random() < 0.5 else b + bytes(rng.randint(1, 100))
    ranges, n = lab.analyze.diff_ranges(a, bytes(b))
    assert ranges == brute_ranges(a, bytes(b))
    assert n == sum(e - s for s, e in ranges)


def test_analyze_refuses_to_read_from_the_lab_root(lab, env, capsys):
    (env.lab / "logic" / "r1").mkdir(parents=True)
    rc, _out, err = run_cli(lab.analyze, ["--run-dir", str(env.lab / "logic" / "r1")], capsys)
    assert rc == 2 and "lab root" in err


# --------------------------------------------------------------------------- #
# preflight
# --------------------------------------------------------------------------- #


class FakeDefaults:
    """Stands in for /usr/bin/defaults. Records every command it is asked to run."""

    def __init__(self, domains, globals_=None, raw=None):
        self.domains, self.globals, self.raw, self.calls = domains, globals_ or {}, raw or {}, []

    def __call__(self, cmd, capture_output=True, timeout=None):
        self.calls.append(list(cmd))
        if cmd[1] == "export":
            if cmd[2] in self.raw:
                return subprocess.CompletedProcess(cmd, 0, self.raw[cmd[2]], b"")
            d = self.domains.get(cmd[2])
            if d is None:
                return subprocess.CompletedProcess(cmd, 1, b"", b"Domain does not exist")
            return subprocess.CompletedProcess(cmd, 0, plistlib.dumps(d), b"")
        if cmd[1] == "read" and cmd[2] == "-g" and cmd[3] in self.globals:
            return subprocess.CompletedProcess(cmd, 0, ("%s\n" % self.globals[cmd[3]]).encode(), b"")
        return subprocess.CompletedProcess(cmd, 1, b"", b"does not exist")


@pytest.mark.parametrize("cmd, ok", [
    (["defaults", "export", "com.apple.logic10", "-"], True),
    (["defaults", "read", "-g", "NSQuitAlwaysKeepsWindows"], True),
    (["defaults", "read", "com.apple.garageband10", "startupAction"], True),
    (["defaults", "write", "com.apple.logic10", "startupAction", "-int", "0"], False),
    (["defaults", "delete", "com.apple.logic10"], False),
    (["defaults", "import", "com.apple.logic10", "-"], False),
    (["defaults", "export", "com.apple.logic10", "/tmp/out.plist"], False),
    (["defaults", "read", "-currentHost", "x"], False),
    (["open", "-a", "Logic Pro"], False),
])
def test_preflight_can_only_run_read_only_commands(lab, cmd, ok):
    assert lab.preflight.is_read_only(cmd) is ok
    if not ok:
        called = []
        with pytest.raises(lab.labcore.SafetyError):
            lab.preflight.run_readonly(cmd, runner=lambda *a, **k: called.append(a))
        assert called == []


@pytest.mark.parametrize("value, verdict", [
    (None, "unknown"), (0, "safe"), (1, "would_reopen"), (2, "would_reopen"),
    (3, "safe"), (5, "safe"), (6, "unknown"), (99, "unknown"), ("x", "unknown"),
])
def test_startup_action_values_are_judged_by_every_plausible_meaning(lab, value, verdict):
    f = lab.preflight.startup_action_finding("com.apple.logic10", value)
    assert f["verdict"] == verdict
    if verdict == "safe":
        assert f["basis"] == "inferred"


def logic_prefs(**extra):
    d = {"NSQuitAlwaysKeepsWindows": False, "unsavedAutosavedURLs": [], "MostRecentPluginsToShow": 20}
    d.update(extra)
    return d


def test_preflight_exit_codes_for_logic(lab, env):
    fake_app(env.apps, "Logic Pro.app", "com.apple.logic10", "12.3.1", "6682")
    run = lab.preflight.run_checks
    # The Creator Studio domain is absent and its app is not installed: information only.
    assert run(["logic"], env.home, FakeDefaults({"com.apple.logic10": logic_prefs(startupAction=2)}))["exit"] == 3
    safe = FakeDefaults({"com.apple.logic10": logic_prefs(startupAction=3)})
    assert run(["logic"], env.home, safe)["exit"] == 2
    assert run(["logic"], env.home, safe, accept_inferred=True)["exit"] == 0
    unset = FakeDefaults({"com.apple.logic10": logic_prefs()})
    assert run(["logic"], env.home, unset, accept_inferred=True)["exit"] == 2
    autosaved = FakeDefaults({"com.apple.logic10": logic_prefs(
        startupAction=3, unsavedAutosavedURLs=["file://%s/Music/Logic/Song.logicx" % env.home])})
    assert run(["logic"], env.home, autosaved, accept_inferred=True)["exit"] == 3
    state = env.home / "Library" / "Saved Application State" / "com.apple.logic10.savedState"
    state.mkdir(parents=True)
    kept = FakeDefaults({"com.apple.logic10": logic_prefs(startupAction=3, NSQuitAlwaysKeepsWindows=True)})
    assert run(["logic"], env.home, kept, accept_inferred=True)["exit"] == 3
    for fake in (safe, unset, autosaved, kept):
        assert all(lab.preflight.is_read_only(c) for c in fake.calls)


def test_preflight_survives_raw_control_characters_in_exported_prefs(lab, env):
    # Logic's real domain carries raw \x01 / \x0c inside strings (measured on the lab Mac).
    xml = plistlib.dumps({"startupAction": 3, "Note": "PLACEHOLDER"}).replace(b"PLACEHOLDER", b"Edit\x0cDownbeat")
    fake = FakeDefaults({}, raw={"com.apple.garageband10": xml})
    assert lab.preflight.export_domain("com.apple.garageband10", fake)["startupAction"] == 3


def write_ableton_prefs(home: Path, version="12.4.2", paths=(), crash_files=0, keys=(b"MpeSettings",)):
    d = home / "Library" / "Preferences" / "Ableton" / ("Live %s" % version)
    blob = b"\xab\x1eVx" + b"".join(keys)
    for p in paths:
        blob += b"\x00\x00" + str(p).encode("utf-16-le") + b"\x00\x00"
    write(d / "Preferences.cfg", blob)
    (d / "Crash").mkdir(parents=True, exist_ok=True)
    for i in range(crash_files):
        write(d / "Crash" / ("CrashRecoveryInfo%d.cfg" % i), b"x")
    return d


def test_preflight_ableton_verdicts_and_no_path_is_ever_printed(lab, env, capsys):
    fake_app(env.apps, "Ableton Live 12 Suite.app", "com.ableton.live", "12.4.2 (2026-06-04)", "12.4.2")
    secret = env.home / "Music" / "secret-song Project" / "secret-song.als"
    write_ableton_prefs(env.home, paths=[secret, env.lab / "ableton" / "r1" / "Lab Project" / "Lab.als"])
    report = lab.preflight.run_checks(["ableton"], env.home, FakeDefaults({}))
    assert report["exit"] == 2
    assert lab.preflight.run_checks(["ableton"], env.home, FakeDefaults({}), accept_inferred=True)["exit"] == 0
    lab.preflight.print_report(report)
    printed = capsys.readouterr().out + json.dumps(report)
    assert "secret-song" not in printed and str(env.home) not in printed
    assert "1 <outside lab root>" in printed and "1 <inside lab root>" in printed

    write_ableton_prefs(env.home, crash_files=1)
    assert lab.preflight.run_checks(["ableton"], env.home, FakeDefaults({}), accept_inferred=True)["exit"] == 3
    write_ableton_prefs(env.home, keys=(b"OpenLastSetOnStartup",))
    for f in (env.home / "Library/Preferences/Ableton/Live 12.4.2/Crash").iterdir():
        f.unlink()
    assert lab.preflight.run_checks(["ableton"], env.home, FakeDefaults({}), accept_inferred=True)["exit"] == 2


def test_preflight_fl_reads_the_registry_and_redacts_recent_projects(lab, env, capsys):
    fake_app(env.apps, "FL Studio 20.app", "com.image-line.flstudio", None or "", "20.8.3.1574")
    secret = env.home / "Documents" / "Image-Line" / "FL Studio" / "Projects" / "secret-beat.flp"
    reg = """<?xml version="1.0" encoding="utf-8"?>
<XMLReg><Key Name="Key1"><Key Name="HKEY_CURRENT_USER"><Key Name="Software"><Key Name="Image-Line">
 <Key Name="FL Studio 20">
  <Key Name="General">
   <Value Name="Template" Type="2">%%FLStudioFactoryData%%/Data/Templates/Minimal/Basic/Basic.flp</Value>
   <Key Name="MIDIForm"><Value Name="StartupOptionBox" Type="2">1</Value></Key>
   <Key Name="FruityLoopsMainForm"><Value Name="LastProjectPath" Type="2"/></Key>
  </Key>
  <Key Name="MRU"><Value Name="0" Type="2">%s</Value><Value Name="1" Type="2">%s</Value></Key>
 </Key>
</Key></Key></Key></Key></XMLReg>
""" % (secret, env.lab / "fl" / "r1" / "Lab.flp")
    write(env.home / "Library" / "Preferences" / "Image-Line" / "reg.xml", reg.encode("utf-8"))
    report = lab.preflight.run_checks(["fl"], env.home, FakeDefaults({}), accept_inferred=True)
    assert report["exit"] == 2  # StartupOptionBox is not mapped: always a question for Sepehr
    lab.preflight.print_report(report)
    printed = capsys.readouterr().out + json.dumps(report)
    assert "factory template" in printed and "value 1: meaning not mapped" in printed
    assert "2 entries, 1 <inside lab root>, 1 <outside lab root>" in printed
    assert "secret-beat" not in printed and str(env.home) not in printed


def synthetic_bookmark(path: str) -> bytes:
    """A bookmark laid out like NSURL's: header, UTF-8 component items, a path array, one TOC."""
    parts = [p for p in path.split("/") if p]
    data = bytearray(struct.pack("<I", 0))  # first-TOC offset, patched below
    offsets = []
    for part in parts:
        raw = part.encode("utf-8")
        offsets.append(len(data))
        data += struct.pack("<II", len(raw), 0x0101) + raw + b"\x00" * (-len(raw) % 4)
    array_off = len(data)
    data += struct.pack("<II", 4 * len(offsets), 0x0601) + b"".join(struct.pack("<I", o) for o in offsets)
    toc_off = len(data)
    data += struct.pack("<IIIII", 12 + 20, 0xFFFFFFFE, 1, 0, 1) + struct.pack("<III", 0x1004, array_off, 0)
    struct.pack_into("<I", data, 0, toc_off)
    header = b"book" + struct.pack("<III", 48 + len(data), 0x10040000, 48) + b"\x00" * 32
    return header + bytes(data)


def test_bookmark_paths_are_read_and_recent_documents_are_only_counted(lab, env):
    inside = str(env.lab / "logic" / "r1" / (PERSIAN + ".logicx"))
    outside = str(env.home / "Music" / "Logic" / "Real Song.logicx")
    assert lab.preflight.bookmark_path(synthetic_bookmark(inside)) == inside
    assert lab.preflight.bookmark_path(b"alis-not-a-bookmark") is None
    assert lab.preflight.bookmark_path(b"book" + b"\x00" * 10) is None
    sfl = env.home / lab.preflight.SFL_DIR / "com.apple.logic10.sfl3"
    write(sfl, plistlib.dumps({"$objects": ["$null", synthetic_bookmark(inside), synthetic_bookmark(outside),
                                            {"Bookmark": 1}]}, fmt=plistlib.FMT_BINARY))
    rd = lab.preflight.recent_documents("com.apple.logic10", env.home, lab.labcore.real(env.lab))
    assert rd == {"readable": True, "counts": {"total": 2, "inside": 1, "outside": 1, "unknown": 0}}
    if os.geteuid() != 0:
        sfl.chmod(0)
        try:
            rd = lab.preflight.recent_documents("com.apple.logic10", env.home, lab.labcore.real(env.lab))
        finally:
            sfl.chmod(0o600)
        assert rd["readable"] is False and "privacy" in rd["why"]
