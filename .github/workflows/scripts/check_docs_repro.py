#!/usr/bin/env python3
"""
PR nudge: a changed number in docs/ should come with the command that produced it.

WHY
    CONTRIBUTING.md: "If you change a number in docs/, include the command that
    produced it." AGENTS.md: "Never invent a benchmark figure." That rule is the
    project's main credibility mechanism, and it is currently enforced by whoever
    happens to review the PR.

    This is a reminder, not a gate. It cannot tell a real measurement from a
    plausible-looking one, so failing the build on it would be pretending to a
    rigour it does not have — and a noisy check gets muted, which is worse than
    no check. It emits a notice and a job-summary note. Nothing more.

HOW IT DECIDES
    Numbers that appear in added lines of docs/ or README.md and did not appear
    in the base revision of the same file. If any are found and the PR body
    contains no runnable-looking reproduction command, say so.

USAGE
    BASE_SHA=... HEAD_SHA=... PR_BODY="..." python3 check_docs_repro.py
    Always exits 0.
"""

from __future__ import annotations

import os
import re
import subprocess
import sys

WATCHED = ("docs/", "README.md")

# Numbers that read like a claim: a magnitude with a unit, a ratio, a percentage,
# or a grouped integer. Bare small integers are ignored — "3 semantic changes" in
# prose is not a benchmark.
CLAIM_RE = re.compile(
    r"(?<![\w.])("
    r"\d{1,3}(?:,\d{3})+"                       # 232,000
    r"|\d+(?:\.\d+)?\s?(?:%|×|x\b)"             # 24.5%, 901x, 3×
    r"|\d+(?:\.\d+)?\s?(?:[KMGT]?B(?:/s)?)\b"   # 8.9 MB, 11 KB, 1.5 MB/s
    r"|\d+(?:\.\d+)?\s?(?:ms|s)\b"              # 250 ms
    r")",
)

# Something the reader could actually run.
REPRO_RE = re.compile(
    r"(python3?\s+\S*experiments/\S+"
    r"|\./?experiments/\S+\.sh"
    r"|bash\s+\S*experiments/\S+"
    r"|\bwit\s+(?:diff|log|status)\b"
    r"|ffmpeg\s+-i\b"
    r"|zstd\s+.*--patch-from"
    r"|flac\s+-\d)",
    re.I,
)


def git(*args) -> str:
    return subprocess.run(
        ["git", *args], capture_output=True, text=True, check=False
    ).stdout


def main() -> int:
    base = os.environ.get("BASE_SHA", "").strip()
    head = os.environ.get("HEAD_SHA", "HEAD").strip()
    body = os.environ.get("PR_BODY", "") or ""

    if not base:
        print("no BASE_SHA — skipping (nothing to compare against)")
        return 0

    changed = [
        f for f in git("diff", "--name-only", f"{base}...{head}").splitlines()
        if f.startswith(WATCHED)
    ]
    if not changed:
        print("no documentation changes in this PR")
        return 0

    findings = {}
    for path in changed:
        before = set(CLAIM_RE.findall(git("show", f"{base}:{path}")))
        diff = git("diff", "--unified=0", f"{base}...{head}", "--", path)
        added = []
        for line in diff.splitlines():
            if line.startswith("+") and not line.startswith("+++"):
                for tok in CLAIM_RE.findall(line):
                    if tok not in before:
                        added.append((tok, line[1:].strip()))
        if added:
            findings[path] = added

    if not findings:
        print("documentation changed, but no new quantitative claims")
        return 0

    has_repro = bool(REPRO_RE.search(body))
    total = sum(len(v) for v in findings.values())

    lines = ["## Documentation numbers changed in this PR", ""]
    for path, items in findings.items():
        lines.append(f"**`{path}`** — {len(items)} new figure(s)")
        for tok, context in items[:5]:
            snippet = context if len(context) <= 110 else context[:107] + "..."
            lines.append(f"- `{tok}` — {snippet}")
        if len(items) > 5:
            lines.append(f"- ... and {len(items) - 5} more")
        lines.append("")

    if has_repro:
        lines += [
            "✅ The PR description contains a reproduction command. Nothing to do.",
            "",
        ]
        print("\n".join(lines))
        note = None
    else:
        lines += [
            "ℹ️ No reproduction command found in the PR description.",
            "",
            "This project's credibility rests on its numbers being reproducible, so",
            "CONTRIBUTING.md asks for the command that produced any figure you change:",
            "",
            "```",
            "python3 experiments/als_semantic_diff.py --chain '/path/to/YourProject/Backup/*.als'",
            "```",
            "",
            "Also worth stating which material it was measured on, and whether the claim",
            "is **measured**, **cited** or **inferred** (AGENTS.md).",
            "",
            "If these are not measurements — a rewording, a table reflow, a citation — say",
            "so in a sentence and ignore this. It is a reminder, not a gate, and it never",
            "fails the build.",
            "",
        ]
        note = (
            f"{total} new figure(s) in docs/ and no reproduction command in the PR body. "
            "CONTRIBUTING.md asks for the command that produced them. This does not block merge."
        )

    text = "\n".join(lines)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as fh:
            fh.write(text + "\n")
    print(text)
    if note:
        # One annotation, on the file, not a comment — repeated bot comments on a
        # rebased PR are exactly the kind of noise that makes people ignore CI.
        first = next(iter(findings))
        print(f"::notice file={first}::{note}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
