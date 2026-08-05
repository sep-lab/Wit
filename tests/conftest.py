"""
Shared test plumbing for the Wit prototypes.

Three jobs:

1. Make ``experiments/`` importable, so the prototypes can be tested as modules
   without changing a byte of them. The prototypes are stdlib-only by design
   (AGENTS.md); that constraint applies to ``experiments/``, not to this suite.

2. Provide the synthetic-fixture factories. No DAW project file or audio file is
   ever committed to this repository, so every fixture is generated in code.

3. Provide *optional* real-material fixtures, gated on ``WIT_FIXTURES``, and make
   their absence LOUD. A neighbouring project shipped real-fixture tests that
   silently skipped in every environment; a serious bug survived for months
   because the suite looked green. If real-fixture coverage does not run here, the
   run ends with a banner saying so, and ``WIT_REQUIRE_FIXTURES=1`` turns the skip
   into a failure for nightly / pre-release runs.
"""

from __future__ import annotations

import os
import re
import shutil
import sys
from pathlib import Path

import pytest

TESTS_DIR = Path(__file__).resolve().parent
REPO_ROOT = TESTS_DIR.parent
EXPERIMENTS_DIR = REPO_ROOT / "experiments"

# experiments/ first so `import cdc_dedup` resolves to the prototype under test.
for _p in (str(EXPERIMENTS_DIR), str(TESTS_DIR)):
    if _p not in sys.path:
        sys.path.insert(0, _p)


# --------------------------------------------------------------------------- #
# markers
# --------------------------------------------------------------------------- #


def pytest_configure(config: pytest.Config) -> None:
    config.addinivalue_line(
        "markers",
        "real_fixtures: needs real DAW material; set WIT_FIXTURES=/path to enable",
    )
    config.addinivalue_line(
        "markers", "slow: takes more than a second or shells out to zstd/git"
    )
    config._wit_real_fixture_skips = 0  # type: ignore[attr-defined]
    config._wit_real_fixture_ran = 0  # type: ignore[attr-defined]


# --------------------------------------------------------------------------- #
# repo paths and prototype modules
# --------------------------------------------------------------------------- #


@pytest.fixture(scope="session")
def repo_root() -> Path:
    return REPO_ROOT


@pytest.fixture(scope="session")
def experiments_dir() -> Path:
    return EXPERIMENTS_DIR


@pytest.fixture(scope="session")
def als_diff():
    import als_semantic_diff

    return als_semantic_diff


@pytest.fixture(scope="session")
def cdc():
    import cdc_dedup

    return cdc_dedup


@pytest.fixture(scope="session")
def flp():
    import flp_parse

    return flp_parse


# --------------------------------------------------------------------------- #
# synthetic fixture helpers
# --------------------------------------------------------------------------- #


@pytest.fixture
def write_als(tmp_path):
    """``write_als(live_set, "v1.als") -> path``. Always inside tmp_path."""
    from factories import als as als_factory

    def _write(live_set, name="set.als"):
        return als_factory.write_als(live_set, tmp_path / name)

    return _write


@pytest.fixture
def model_of(als_diff, write_als):
    """``model_of(live_set) -> semantic model``, via a real gzipped file on disk."""
    counter = {"n": 0}

    def _model(live_set):
        counter["n"] += 1
        path = write_als(live_set, "model-%d.als" % counter["n"])
        return als_diff.build_model(path)

    return _model


@pytest.fixture
def diff_of(als_diff, model_of):
    """``diff_of(before, after) -> list[str]`` — the user-facing diff lines."""

    def _diff(before, after):
        return als_diff.diff_models(model_of(before), model_of(after))

    return _diff


@pytest.fixture
def flp_report(flp, tmp_path, capsys):
    """
    Run ``flp_parse.parse`` and return its report as structured data.

    The prototype only prints; it has no return value and no library API. Rather
    than reimplement it (which would just test a copy of the same logic), the
    tests read the report it actually shows a user — which is the interface that
    matters anyway.
    """
    counter = {"n": 0}

    def _run(data_or_path):
        if isinstance(data_or_path, (bytes, bytearray)):
            counter["n"] += 1
            path = tmp_path / ("gen-%d.flp" % counter["n"])
            path.write_bytes(bytes(data_or_path))
        else:
            path = Path(str(data_or_path))
        capsys.readouterr()  # discard anything buffered from earlier in the test
        flp.parse(str(path))
        return _parse_flp_report(capsys.readouterr().out)

    return _run


_HDR_RE = re.compile(r"FLhd\s+format=(-?\d+)\s+channels=(\d+)\s+ppq=(\d+)")
_DLEN_RE = re.compile(r"FLdt\s+(\d+) bytes")
_ROW_RE = re.compile(r"^\s*(\d+)\s+(\d+)\s{2}(\S.*?)\s{2,}(\d+)\s*$")
_TOTALS_RE = re.compile(r"events:\s+(\d+)\s+distinct ids:\s+(\d+)")
_BLOB_RE = re.compile(r"variable-length payload:\s+(\d+) B \(([\d.]+)% of the (\d+) B file\)")
_TEXT_RE = re.compile(r"^\s{2}\[(.+?)\]\s(.*)$")


def _parse_flp_report(out: str) -> dict:
    report = {
        "stdout": out,
        "rows": {},
        "texts": [],
        "format": None,
        "channels": None,
        "ppq": None,
        "data_length": None,
        "events": None,
        "distinct_ids": None,
        "blob_bytes": None,
        "blob_pct": None,
        "file_bytes": None,
    }
    m = _HDR_RE.search(out)
    if m:
        report["format"], report["channels"], report["ppq"] = (int(g) for g in m.groups())
    m = _DLEN_RE.search(out)
    if m:
        report["data_length"] = int(m.group(1))
    m = _TOTALS_RE.search(out)
    if m:
        report["events"], report["distinct_ids"] = int(m.group(1)), int(m.group(2))
    m = _BLOB_RE.search(out)
    if m:
        report["blob_bytes"] = int(m.group(1))
        report["blob_pct"] = float(m.group(2))
        report["file_bytes"] = int(m.group(3))

    in_texts = False
    for line in out.splitlines():
        if line.startswith("-- text events --"):
            in_texts = True
            continue
        if in_texts:
            t = _TEXT_RE.match(line)
            if t:
                report["texts"].append((t.group(1).strip(), t.group(2)))
            continue
        row = _ROW_RE.match(line)
        if row and not line.lstrip().startswith("id"):
            ev, count, name, payload = row.groups()
            report["rows"][int(ev)] = {
                "count": int(count),
                "name": name.strip(),
                "payload_bytes": int(payload),
            }
    return report


# --------------------------------------------------------------------------- #
# privacy helpers (AGENTS.md: never paste other people's paths into logs)
# --------------------------------------------------------------------------- #

_HOME_RE = re.compile(r"(/Users/|/home/)[^/\s\"']+")


def redact(text: str) -> str:
    """Replace home directories so real-material failures never leak identities."""
    return _HOME_RE.sub(r"\1<redacted>", str(text))


@pytest.fixture
def redactor():
    return redact


# --------------------------------------------------------------------------- #
# real-material fixtures (opt-in, loudly skipped)
# --------------------------------------------------------------------------- #

FIXTURES_ENV = "WIT_FIXTURES"
REQUIRE_ENV = "WIT_REQUIRE_FIXTURES"


def _fixtures_root() -> "Path | None":
    raw = os.environ.get(FIXTURES_ENV, "").strip()
    if not raw:
        return None
    root = Path(raw).expanduser()
    return root if root.is_dir() else None


def _skip_or_fail(request, reason: str) -> None:
    config = request.config
    if os.environ.get(REQUIRE_ENV, "").strip() not in ("", "0", "false", "no"):
        pytest.fail("%s=1 is set but %s" % (REQUIRE_ENV, reason))
    config._wit_real_fixture_skips += 1  # type: ignore[attr-defined]
    pytest.skip(reason)


@pytest.fixture(scope="session")
def fixtures_root() -> "Path | None":
    return _fixtures_root()


@pytest.fixture(scope="session")
def fixture_index(fixtures_root):
    """
    Walk WIT_FIXTURES exactly once per session.

    Real material lives in trees measured in gigabytes (a .logicx package is
    thousands of files). Globbing per test made the suite take minutes.
    """
    if fixtures_root is None:
        return None

    index = {"*.als": [], "*.flp": [], "ProjectData": []}
    for path in fixtures_root.rglob("*"):
        if not path.is_file():
            continue
        depth = len(path.relative_to(fixtures_root).parts)
        if depth > _RealMaterial.MAX_DEPTH:
            continue
        if path.suffix.lower() == ".als":
            index["*.als"].append(path)
        elif path.suffix.lower() == ".flp":
            index["*.flp"].append(path)
        elif path.name == "ProjectData":
            index["ProjectData"].append(path)
    return {key: sorted(value) for key, value in index.items()}


@pytest.fixture
def real_material(request, fixtures_root, fixture_index):
    """
    Discovered real DAW material, or a loud skip.

    Usage::

        @pytest.mark.real_fixtures
        def test_x(real_material):
            chain = real_material.als_chain(min_len=2)
    """
    if fixtures_root is None:
        _skip_or_fail(
            request,
            "no real DAW material: set %s=/path/to/material (a directory containing "
            ".als backups, .flp projects, and/or Logic ProjectData)" % FIXTURES_ENV,
        )
    request.config._wit_real_fixture_ran += 1  # type: ignore[attr-defined]
    return _RealMaterial(fixtures_root, request, fixture_index)


class _RealMaterial:
    """
    Read-only access to real DAW material.

    Nothing here hands a test a path it may write to. ``copy`` is the only way to
    get a writable file, and it copies into the test's tmp_path — AGENTS.md:
    "Never modify a user's DAW projects. Read-only, always."
    """

    # Logic buries ProjectData several levels inside a .logicx package; the cap
    # only exists to stop a mis-set WIT_FIXTURES from walking an entire home
    # directory.
    MAX_DEPTH = 8

    def __init__(self, root: Path, request, index) -> None:
        self.root = root
        self._request = request
        self._index = index

    # -- discovery ---------------------------------------------------------- #

    def _glob(self, pattern: str):
        return list(self._index.get(pattern, ()))

    def als_chain(self, min_len: int = 2):
        files = self._glob("*.als")
        if len(files) < min_len:
            _skip_or_fail(
                self._request,
                "need >= %d .als files under %s, found %d"
                % (min_len, FIXTURES_ENV, len(files)),
            )
        return files

    def flp_files(self, min_len: int = 1):
        files = self._glob("*.flp")
        if len(files) < min_len:
            _skip_or_fail(
                self._request,
                "need >= %d .flp files under %s, found %d"
                % (min_len, FIXTURES_ENV, len(files)),
            )
        return files

    def logic_project_data(self, min_len: int = 2):
        files = self._glob("ProjectData")
        if len(files) < min_len:
            _skip_or_fail(
                self._request,
                "need >= %d Logic ProjectData files under %s, found %d"
                % (min_len, FIXTURES_ENV, len(files)),
            )
        return files

    # -- safe access -------------------------------------------------------- #

    @staticmethod
    def fingerprint(paths):
        """(size, mtime_ns) per file — used to prove nothing was modified."""
        return {str(p): (p.stat().st_size, p.stat().st_mtime_ns) for p in paths}

    def copy(self, path: Path, dest_dir: Path) -> Path:
        """Copy real material into a scratch directory before anything writes."""
        dest_dir.mkdir(parents=True, exist_ok=True)
        dest = dest_dir / path.name
        shutil.copy2(str(path), str(dest))
        return dest


# --------------------------------------------------------------------------- #
# the loud banner
# --------------------------------------------------------------------------- #


def pytest_terminal_summary(terminalreporter, exitstatus, config) -> None:
    ran = getattr(config, "_wit_real_fixture_ran", 0)
    skipped = getattr(config, "_wit_real_fixture_skips", 0)
    if skipped == 0:
        if ran:
            terminalreporter.write_sep(
                "=", "real-fixture coverage RAN on %d test(s)" % ran, green=True
            )
        return

    terminalreporter.write_sep("=", "REAL-FIXTURE COVERAGE DID NOT RUN", red=True, bold=True)
    terminalreporter.write_line(
        "  %d test(s) that exercise real DAW material were skipped." % skipped
    )
    terminalreporter.write_line(
        "  Everything else passing means the synthetic fixtures are self-consistent."
    )
    terminalreporter.write_line(
        "  It does NOT mean the parsers work on a file Ableton or FL Studio wrote."
    )
    terminalreporter.write_line("")
    terminalreporter.write_line("  To run them:")
    terminalreporter.write_line("      WIT_FIXTURES=/path/to/YourProject/Backup pytest tests/")
    terminalreporter.write_line("  To make their absence a failure (nightly / pre-release):")
    terminalreporter.write_line("      WIT_REQUIRE_FIXTURES=1 WIT_FIXTURES=... pytest tests/")
    terminalreporter.write_sep("=", "", red=True, bold=True)
