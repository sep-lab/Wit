#!/usr/bin/env python3
"""
Guardrail: every internal documentation link resolves.

WHY
    Wit's docs are the product right now — the README sends readers to
    DECISION-DOC, EXPERIMENTS, PRIOR-ART and the ADRs, and CONTRIBUTING gives a
    reading order. A dead link in that chain costs a would-be contributor more
    than a broken build would.

WHAT IT CHECKS
    - relative links between markdown files (and to any file in the repo) exist
    - in-page anchors (#section) resolve against the target file's own headings,
      using GitHub's slug rules. The README's whole nav bar is anchors, and
      renaming one heading silently breaks it.
    - explicit HTML anchors (<a name="..."> / id="...") count as valid targets

WHAT IT DOES NOT CHECK
    External http(s) links. Deliberately: network flakiness and rate limits make
    that a source of false failures, and a link-rot check that cries wolf gets
    ignored. Prior-art URLs going stale is a real risk but not a CI-blocking one.

USAGE
    python3 check_docs_links.py [--root DIR]
"""

from __future__ import annotations

import argparse
import os
import re
import sys
import unicodedata

LINK_RE = re.compile(r"\[[^\]]*\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)")
# The README's nav bar is raw HTML anchors, so markdown-only matching would miss
# exactly the links a first-time reader clicks first.
HREF_RE = re.compile(r"<a\b[^>]*\bhref=[\"']([^\"']+)[\"']", re.I)
HEADING_RE = re.compile(r"^(#{1,6})\s+(.*?)\s*#*\s*$")
HTML_ANCHOR_RE = re.compile(r"<(?:a|h[1-6]|div|span)[^>]*\b(?:name|id)=[\"']([^\"']+)[\"']", re.I)
FENCE_RE = re.compile(r"^\s*(```|~~~)")


def slugify(text: str) -> str:
    """GitHub's heading -> anchor rules, closely enough for our headings."""
    text = re.sub(r"<[^>]+>", "", text)                 # strip inline HTML
    text = re.sub(r"!?\[([^\]]*)\]\([^)]*\)", r"\1", text)  # links/images -> label
    text = text.replace("`", "").replace("*", "").replace("_", "")
    text = unicodedata.normalize("NFKD", text)
    text = text.lower().strip()
    text = re.sub(r"[^\w\- ]", "", text, flags=re.UNICODE)
    return text.replace(" ", "-")


def collect_anchors(path: str) -> set:
    anchors = set()
    try:
        with open(path, encoding="utf-8") as fh:
            text = fh.read()
    except (OSError, UnicodeDecodeError):
        return anchors
    in_fence = False
    seen = {}
    for line in text.splitlines():
        if FENCE_RE.match(line):
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        m = HEADING_RE.match(line)
        if m:
            slug = slugify(m.group(2))
            # GitHub disambiguates repeats with -1, -2, ...
            n = seen.get(slug, 0)
            seen[slug] = n + 1
            anchors.add(slug if n == 0 else f"{slug}-{n}")
            if n == 0:
                anchors.add(slug)
    anchors.update(HTML_ANCHOR_RE.findall(text))
    return anchors


def markdown_files(root: str) -> list:
    out = []
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d not in {".git", "node_modules", "__pycache__", "target"}]
        for name in filenames:
            if name.lower().endswith(".md"):
                out.append(os.path.join(dirpath, name))
    return sorted(out)


def main() -> int:
    ap = argparse.ArgumentParser(description="Check internal documentation links")
    ap.add_argument("--root", default=".")
    args = ap.parse_args()
    root = os.path.abspath(args.root)

    anchor_cache = {}
    broken = []
    checked = 0

    for md in markdown_files(root):
        with open(md, encoding="utf-8") as fh:
            lines = fh.read().splitlines()
        in_fence = False
        for lineno, line in enumerate(lines, 1):
            if FENCE_RE.match(line):
                in_fence = not in_fence
                continue
            if in_fence:
                continue
            for link in LINK_RE.findall(line) + HREF_RE.findall(line):
                if link.startswith(("http://", "https://", "mailto:", "tel:", "data:")):
                    continue
                checked += 1
                file_part, _, anchor = link.partition("#")
                if file_part:
                    target = os.path.normpath(os.path.join(os.path.dirname(md), file_part))
                else:
                    target = md
                rel_md = os.path.relpath(md, root)
                if not os.path.exists(target):
                    broken.append((rel_md, lineno, link, "target does not exist"))
                    continue
                if anchor:
                    if os.path.isdir(target):
                        continue
                    if target not in anchor_cache:
                        anchor_cache[target] = collect_anchors(target)
                    if anchor.lower() not in anchor_cache[target]:
                        broken.append(
                            (rel_md, lineno, link,
                             f"no heading in {os.path.relpath(target, root)} slugifies to '#{anchor}'")
                        )

    for path, lineno, link, why in broken:
        print(f"::error file={path},line={lineno}::Broken link {link} — {why}")
        print(f"  {path}:{lineno}  {link}  ({why})")

    if broken:
        print(
            f"\n  {len(broken)} broken internal link(s) out of {checked} checked.\n"
            "\n"
            "  The docs are the product at this stage — README and CONTRIBUTING send\n"
            "  new contributors down a reading chain, and a dead link ends it. Anchor\n"
            "  failures usually mean a heading was reworded; update the link too.\n"
        )
        return 1

    print(f"clean: {checked} internal link(s) resolve")
    return 0


if __name__ == "__main__":
    sys.exit(main())
