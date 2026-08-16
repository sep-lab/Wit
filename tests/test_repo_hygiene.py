"""
Repository rules that currently have no enforcement, or only have it in CI.

Two of these are AGENTS.md conventions that nothing checks today:

- ``experiments/`` is standard library only. A stray ``import numpy`` would pass
  every other test on a developer machine that happens to have numpy, and only
  fail for the musician the constraint exists for.
- ``experiments/`` targets Python 3.9. CI pins 3.9, but a local run on 3.12
  would not notice ``match`` or ``X | Y`` annotations until CI does.

The audio-file check duplicates CI on purpose: a fixture accidentally saved into
``tests/`` should fail in two seconds locally, not on a pull request.
"""

from __future__ import annotations

import ast
import importlib.util
import os
import re
import sys
import sysconfig
from pathlib import Path

import pytest

# Mirrors .github/workflows/ci.yml
FORBIDDEN_SUFFIXES = re.compile(
    r"\.(wav|aif|aiff|caf|flac|mp3|m4a|als|flp|ptx|cpr|song)$", re.IGNORECASE
)


# Build output, not repository content. `/target/` is root-anchored in
# .gitignore, so nothing under it can ever be committed — and the CI mirror of
# this check (.github/workflows/scripts/check_no_binaries.sh) reads `git
# ls-files`, so it never sees these either. Without this exclusion the local
# test diverges from CI the moment anything writes there: `just demo-library`
# generates a synthetic library with `.als` files under `target/`, which is
# exactly what it is supposed to do.
IGNORED_TOP_LEVEL_DIRS = {".git", "target"}
IGNORED_ANY_LEVEL_DIRS = {"__pycache__", ".pytest_cache", ".ruff_cache"}


def repo_files(repo_root: Path):
    for path in repo_root.rglob("*"):
        if not path.is_file():
            continue
        parts = path.relative_to(repo_root).parts
        if parts[0] in IGNORED_TOP_LEVEL_DIRS:
            continue
        if IGNORED_ANY_LEVEL_DIRS.intersection(parts):
            continue
        yield path


def experiment_scripts(repo_root: Path):
    return sorted((repo_root / "experiments").glob("*.py"))


# --------------------------------------------------------------------------- #
# no audio, no project files
# --------------------------------------------------------------------------- #


def test_no_audio_or_daw_project_file_is_in_the_repository(repo_root):
    offenders = [
        str(p.relative_to(repo_root))
        for p in repo_files(repo_root)
        if FORBIDDEN_SUFFIXES.search(p.name)
    ]
    assert offenders == [], (
        "audio or DAW project files must never be committed (AGENTS.md, "
        ".gitignore, CI): %s" % offenders
    )


def test_the_test_suite_generates_its_fixtures_rather_than_shipping_them(repo_root):
    """
    Every fixture must come from ``tests/factories``. If a real .als, .flp or a
    binary blob ever appears in tests/, this fails before CI does.
    """
    tests_dir = repo_root / "tests"
    allowed = {".py", ".md", ".txt", ".ini", ".cfg", ".toml"}
    unexpected = [
        str(p.relative_to(repo_root))
        for p in repo_files(tests_dir)
        if p.suffix.lower() not in allowed
    ]
    assert unexpected == [], "unexpected non-source files under tests/: %s" % unexpected


def test_gitignore_covers_the_formats_ci_rejects(repo_root):
    ignore = (repo_root / ".gitignore").read_text(encoding="utf-8")
    for suffix in ("wav", "als", "flp", "aif", "mp3"):
        assert "*.%s" % suffix in ignore, "%s missing from .gitignore" % suffix


# --------------------------------------------------------------------------- #
# the standard-library-only rule
# --------------------------------------------------------------------------- #


def is_stdlib(name):
    """True if `name` is a standard-library module.

    `sys.stdlib_module_names` is 3.10+. The 3.9 fallback used to be a hand-listed
    set of "modules the prototypes happen to use", which went stale the moment a
    prototype imported something new — it failed CI on the 3.9 leg only, flagging
    json/shutil/subprocess as third-party. Resolve the module and look at where it
    actually lives instead, so this cannot rot.
    """
    if name in sys.builtin_module_names:
        return True
    names = getattr(sys, "stdlib_module_names", None)
    if names is not None:
        return name in names
    try:
        spec = importlib.util.find_spec(name)
    except (ImportError, ValueError):
        return False
    if spec is None or spec.origin is None:
        return False
    if spec.origin in ("built-in", "frozen"):
        return True
    stdlib_dir = os.path.realpath(sysconfig.get_paths()["stdlib"])
    origin = os.path.realpath(spec.origin)
    return origin.startswith(stdlib_dir) and "site-packages" not in origin


def imported_top_level_modules(path: Path):
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    found = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            found.update(alias.name.split(".")[0] for alias in node.names)
        elif isinstance(node, ast.ImportFrom) and node.level == 0 and node.module:
            found.add(node.module.split(".")[0])
    return found


def test_experiments_import_only_the_standard_library(repo_root):
    """
    AGENTS.md: "Prototypes in experiments/ are Python 3.9+, standard library only,
    so a musician with a stock Mac can run them against their own session with no
    setup. Do not add dependencies there."
    """
    for script in experiment_scripts(repo_root):
        outside = sorted(m for m in imported_top_level_modules(script) if not is_stdlib(m))
        assert outside == [], (
            "%s imports non-stdlib module(s) %s — experiments/ must run with no "
            "install step" % (script.name, outside)
        )


def test_experiments_parse_as_python_3_9(repo_root):
    """CI pins 3.9. Catch 3.10+ syntax locally instead of on a pull request."""
    for script in experiment_scripts(repo_root):
        source = script.read_text(encoding="utf-8")
        try:
            ast.parse(source, filename=str(script), feature_version=(3, 9))
        except SyntaxError as exc:  # pragma: no cover - only on a real regression
            pytest.fail("%s is not valid Python 3.9: %s" % (script.name, exc))


# --------------------------------------------------------------------------- #
# the honest-limitations rule
# --------------------------------------------------------------------------- #


# AGENTS.md requires three things in every experiment's docstring. The wording of
# the first heading varies ("WHAT THIS MEASURES" / "WHAT THIS DOES"), so any of
# the accepted spellings satisfies it; the other two are checked literally because
# "what it does not handle" is the one AGENTS.md calls out as non-optional.
REQUIRED_SECTIONS = (
    ("WHAT THIS MEASURES", "WHAT THIS DOES"),
    ("USAGE",),
    ("WHAT THIS DOES NOT HANDLE",),
)


def assert_documents_itself(name: str, text: str) -> None:
    for alternatives in REQUIRED_SECTIONS:
        assert any(section in text for section in alternatives), (
            "%s does not state '%s' — AGENTS.md requires what it measures, how to "
            "run it, and what it does not handle" % (name, " / ".join(alternatives))
        )


def test_every_python_experiment_documents_itself(repo_root):
    for script in experiment_scripts(repo_root):
        doc = ast.get_docstring(ast.parse(script.read_text(encoding="utf-8"))) or ""
        assert_documents_itself(script.name, doc)


def test_every_shell_experiment_documents_itself(repo_root):
    for script in sorted((repo_root / "experiments").glob("*.sh")):
        assert_documents_itself(script.name, script.read_text(encoding="utf-8"))


# --------------------------------------------------------------------------- #
# the test suite's own dependencies
# --------------------------------------------------------------------------- #


def test_test_dependencies_stay_minimal(repo_root):
    """
    The tests may use pytest; that is the only concession. Keeping the list to two
    distinct packages means a contributor can run the suite from a clean machine in one
    command, which is the same reasoning that keeps experiments/ stdlib-only.

    A package may legitimately appear on more than one line when its version floor is
    split by an environment marker (tests/requirements-dev.txt splits pytest's floor by
    `python_version`, since pytest 9 requires >=3.10 and CI keeps a 3.9 leg) -- that is
    still one dependency, not two, so the set of distinct package names is what must
    stay minimal, not the line count.
    """
    text = (repo_root / "tests" / "requirements-dev.txt").read_text(encoding="utf-8")
    packages = {
        re.split(r"[<>=!\[;]", line.strip())[0].strip()
        for line in text.splitlines()
        if line.strip() and not line.strip().startswith("#")
    }
    assert sorted(packages) == ["pytest", "pytest-cov"]
