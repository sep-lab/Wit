#!/usr/bin/env python3
"""
Guardrail: no absolute personal filesystem paths in committed files.

WHY THIS IS A PRIVACY CHECK, NOT A STYLE CHECK
    Ableton stores absolute sample paths. One real test project in this project's
    research contained 777 references to one person's home directory, 16 to
    another's, and 13 more — three different people's home directories baked into
    a file sitting on a fourth person's machine (docs/EXPERIMENTS.md #10).

    That means a contributor pasting a snippet of real project XML into a doc, an
    issue, or a test fixture leaks someone else's username, folder structure, and
    often the names of their unreleased work. AGENTS.md is explicit: "Treat those
    as private data: do not paste them into issues or logs."

    Pasted paths also break reproducibility — a documented command containing
    /Users/someone/ cannot be run by anyone else.

WHAT IS ALLOWED
    Placeholders (/path/to/..., /Users/<you>/..., $HOME, ~/) always pass, because
    that is what a doc should use instead.

    Specific literals may be allowlisted in allowed-paths.txt with a reason. Every
    allowlisted hit is still reported as a warning, so the exemption stays
    visible instead of becoming permanent by accident.

USAGE
    python3 check_personal_paths.py [--allowlist FILE]
    Exit 0 if clean (warnings allowed), 1 if a new personal path was found.
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))

# A home directory followed by something under it. Requiring the trailing
# separator avoids flagging bare "/Users" or "/home" in prose.
PATTERNS = [
    re.compile(r"/Users/(?P<user>[^/\s\"'<>|)\]}]+)/"),
    re.compile(r"/home/(?P<user>[^/\s\"'<>|)\]}]+)/"),
    re.compile(r"[Cc]:\\\\?Users\\\\?(?P<user>[^\\\s\"'<>|)\]}]+)\\"),
]

# Names that are obviously stand-ins, not real people. This list is deliberately
# generous: a check that flags `/Users/synthetic/` teaches contributors to reach
# for the allowlist, and an allowlist people use reflexively stops protecting
# anyone. Anonymous placeholders are the behaviour we want, so let them through.
PLACEHOLDERS = {
    "you", "user", "username", "name", "someone", "somebody", "me", "yourname",
    "your-name", "your_name", "youruser", "path", "to", "home", "example",
    "synthetic", "fixture", "fixtures", "test", "tests", "testuser",
    "anon", "anonymous", "alice", "bob", "producer", "ci",
    "runner",           # GitHub Actions' own workspace path
    "$user", "${user}", "$home", "%username%", "%userprofile%",
}
PLACEHOLDER_RE = re.compile(r"^[<\[{(].*[>\]})]$|^\.{3}$|^\*+$")

# Files that are allowed to contain paths by nature.
SKIP_SUFFIXES = (".lock",)


def is_placeholder(user: str) -> bool:
    low = user.lower()
    return low in PLACEHOLDERS or bool(PLACEHOLDER_RE.match(user))


def load_allowlist(path: str) -> list:
    entries = []
    if not os.path.exists(path):
        return entries
    with open(path, encoding="utf-8") as fh:
        for line in fh:
            line = line.split("#", 1)[0].strip()
            if line:
                entries.append(line)
    return entries


def tracked_files() -> list:
    out = subprocess.run(
        ["git", "ls-files"], capture_output=True, text=True, check=True
    ).stdout
    return [
        f
        for f in out.splitlines()
        # The allowlist obviously contains the strings it allows; scanning it
        # would just echo itself back on every run.
        if f and not f.endswith(SKIP_SUFFIXES) and os.path.basename(f) != "allowed-paths.txt"
    ]


def main() -> int:
    ap = argparse.ArgumentParser(description="Refuse committed absolute personal paths")
    ap.add_argument("--allowlist", default=os.path.join(HERE, "allowed-paths.txt"))
    args = ap.parse_args()

    allowed = load_allowlist(args.allowlist)
    errors, warnings = [], []

    for path in tracked_files():
        try:
            with open(path, encoding="utf-8", errors="strict") as fh:
                lines = fh.read().splitlines()
        except (UnicodeDecodeError, IsADirectoryError, FileNotFoundError):
            continue  # binary or vanished; the size/extension guardrail covers those
        for lineno, line in enumerate(lines, 1):
            for pat in PATTERNS:
                for m in pat.finditer(line):
                    user = m.group("user")
                    if is_placeholder(user):
                        continue
                    hit = m.group(0)
                    entry = (path, lineno, hit)
                    if any(a in line for a in allowed):
                        warnings.append(entry)
                    else:
                        errors.append(entry)

    for path, lineno, hit in warnings:
        print(f"::warning file={path},line={lineno}::Allowlisted personal path {hit!r} "
              f"— still a real person's home directory. Redact it when this doc is next revised.")
        print(f"  allowlisted: {path}:{lineno}  {hit}")

    for path, lineno, hit in errors:
        print(f"::error file={path},line={lineno}::Absolute personal path {hit!r} must not be committed.")
        print(f"  found: {path}:{lineno}  {hit}")

    if errors:
        print(
            "\n"
            "  These are absolute paths into somebody's home directory.\n"
            "\n"
            "  If it is yours: replace it with a placeholder, e.g.\n"
            "      /path/to/YourProject/Backup/*.als\n"
            "      ~/Music/Ableton/...\n"
            "  A documented command has to be runnable by the reader; a path with\n"
            "  your username in it is not.\n"
            "\n"
            "  If it came out of a real DAW project: that is somebody else's private\n"
            "  data, and DAW files are full of it (docs/EXPERIMENTS.md #10 found three\n"
            "  people's home directories in one file). Redact it. See AGENTS.md,\n"
            "  \"Rules for handling user data\".\n"
            "\n"
            "  If the literal path genuinely has to stay, add it to\n"
            "  .github/workflows/scripts/allowed-paths.txt with a reason — it will\n"
            "  then warn instead of failing.\n"
        )
        return 1

    print(f"clean: no un-allowlisted personal paths ({len(warnings)} allowlisted warning(s))")
    return 0


if __name__ == "__main__":
    sys.exit(main())
