#!/usr/bin/env python3
"""
Claim invariants — do the prototypes still behave the way docs/ says they do?

WHAT THIS IS
    Wit's credibility asset is that its published numbers are reproducible. The
    published numbers were measured on real commercial sessions that cannot be
    committed here (AGENTS.md: never commit audio; and they are other people's
    material). So CI cannot re-derive them.

    What CI *can* do is generate synthetic fixtures with the same structural
    shape and assert the qualitative facts the documented numbers depend on:
    direction and order of magnitude. If one of those flips, a published number
    is no longer safe to quote and a human has to go re-measure on real material.

WHAT THIS IS NOT
    This does not reproduce any figure in docs/EXPERIMENTS.md and must never be
    cited as if it did. Music is not white noise; a synthetic buffer is not a
    stem. Every check below prints the real documented figure alongside the
    synthetic one precisely so the difference stays visible.

USAGE
    python3 check_claims.py [--work DIR] [--quick]
    Exit 0 if every invariant holds, 1 otherwise. Writes a table to
    $GITHUB_STEP_SUMMARY when running under GitHub Actions.
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
EXP = os.path.join(REPO, "experiments")
SYNTH = os.path.join(HERE, "synth_fixtures.py")

RESULTS = []


def record(name, doc_ref, published, invariant, observed, ok, caveat=""):
    RESULTS.append(
        {
            "name": name,
            "doc": doc_ref,
            "published": published,
            "invariant": invariant,
            "observed": observed,
            "ok": bool(ok),
            "caveat": caveat,
        }
    )
    mark = "PASS" if ok else "FAIL"
    print(f"[{mark}] {name}")
    print(f"        doc claim (real material): {published}   [{doc_ref}]")
    print(f"        invariant asserted here  : {invariant}")
    print(f"        observed (synthetic)     : {observed}")
    if caveat:
        print(f"        note                     : {caveat}")
    print()


def run(cmd, **kw):
    proc = subprocess.run(cmd, capture_output=True, text=True, **kw)
    if proc.returncode != 0:
        sys.stderr.write(f"command failed: {' '.join(cmd)}\n{proc.stdout}\n{proc.stderr}\n")
        raise SystemExit(2)
    return proc.stdout


def synth(*args):
    return run([sys.executable, SYNTH, *args])


# ---------------------------------------------------------------------------
# Experiment 7 / ADR-0001 — chunk-level reuse
# ---------------------------------------------------------------------------


def _reuse_pct(a, b):
    out = run([sys.executable, os.path.join(EXP, "cdc_dedup.py"), a, b])
    m = re.search(r"reusable from A\s*:\s*([0-9.]+)%", out)
    if not m:
        raise SystemExit(f"could not parse cdc_dedup output:\n{out}")
    return float(m.group(1))


def check_chunking(work, mb):
    base = os.path.join(work, "base.bin")
    synth("buffer", base, "--mb", str(mb), "--mutate", "none")

    shifted = os.path.join(work, "shifted.bin")
    synth("buffer", shifted, "--mb", str(mb), "--mutate", "shift")
    pct = _reuse_pct(base, shifted)
    record(
        "CDC survives a shift (a region moved in time)",
        "EXPERIMENTS.md#7, ADR-0001",
        "99.59% reusable on a real 19.6 MB stem prepended by 250 ms",
        ">= 95% reusable",
        f"{pct:.2f}% reusable",
        pct >= 95.0,
    )

    perturbed = os.path.join(work, "global.bin")
    synth("buffer", perturbed, "--mb", str(mb), "--mutate", "global")
    pct_g = _reuse_pct(base, perturbed)
    record(
        "CDC collapses on a global perturbation (the re-render wall)",
        "EXPERIMENTS.md#7, ADR-0001",
        "0.00% reusable after a -0.5 dB change across a real stem",
        "<= 0.5% reusable",
        f"{pct_g:.2f}% reusable",
        pct_g <= 0.5,
        "This is the finding the entire architecture rests on. If it ever "
        "stops holding, ADR-0001 needs re-opening, not the test.",
    )

    localized = os.path.join(work, "local.bin")
    synth("buffer", localized, "--mb", str(mb), "--mutate", "local")
    pct_l = _reuse_pct(base, localized)
    record(
        "A localized edit stays largely reusable",
        "EXPERIMENTS.md#7",
        "92.98% reusable when one 5-second section is re-rendered",
        ">= 75%, and strictly worse than the shift case",
        f"{pct_l:.2f}% reusable",
        pct_l >= 75.0 and pct_l < pct,
    )


# ---------------------------------------------------------------------------
# Experiments 1, 3, 4 — semantic diff
# ---------------------------------------------------------------------------


def _diff(work, edit):
    d = os.path.join(work, "als-" + edit)
    synth("als-pair", d, "--edit", edit)
    return run(
        [
            sys.executable,
            os.path.join(EXP, "als_semantic_diff.py"),
            os.path.join(d, "old.als"),
            os.path.join(d, "new.als"),
        ]
    )


def check_semantic_diff(work):
    out = _diff(work, "view")
    ok = "no musical change detected" in out
    record(
        "View-state-only save reports no musical change",
        "EXPERIMENTS.md#1, #4",
        "a real 170-line raw diff was entirely scroll, zoom and selection state",
        "scroll/zoom churn produces zero semantic changes",
        out.strip().splitlines()[0] if out.strip() else "(no output)",
        ok,
    )

    out = _diff(work, "volume")
    ok = "MIX~" in out and "0.794 -> 0.525" in out
    record(
        "A real mixer move is reported",
        "EXPERIMENTS.md#4",
        "MIX~ [Stem-mixing] volume: 0.794 -> 0.525",
        "a volume change surfaces as exactly one MIX~ line",
        " ".join(line.strip() for line in out.strip().splitlines()[1:2]) or "(none)",
        ok,
    )

    out = _diff(work, "rename")
    m = re.search(r"SAMPLE~ .*\((\d+) clip reference\(s\)\)", out)
    fanout = int(m.group(1)) if m else 0
    lines = len([x for x in out.splitlines() if x.strip().startswith(("SAMPLE~", "CLIP", "MIX", "FX"))])
    ok = bool(m) and fanout > 1 and lines == 1
    record(
        "One sample rename coalesces instead of fanning out",
        "EXPERIMENTS.md#4, README 'A diff you can actually read'",
        "425 clip-level changes reduced to 3 readable lines; one rename covered 418 clips",
        "N clip references collapse to a single SAMPLE~ line",
        f"{fanout} clip references -> {lines} reported line(s)",
        ok,
    )

    out = _diff(work, "track")
    record(
        "Structural change (track added) is reported",
        "EXPERIMENTS.md#2",
        "track IDs are stable across saves, so add/remove is distinguishable from edit",
        "an added track surfaces as TRACK+, not as a wholesale rewrite",
        (out.strip().splitlines() or ["(none)"])[-1].strip(),
        "TRACK+" in out,
    )


# ---------------------------------------------------------------------------
# Experiment 6 / ADR-0002 — storage strategies
# ---------------------------------------------------------------------------


def check_storage(work, versions, lines):
    chain = os.path.join(work, "chain")
    synth("als-chain", chain, "--versions", str(versions), "--lines", str(lines))

    out = run(["bash", os.path.join(EXP, "storage_bench.sh"), chain])
    print(out)

    def mb(label):
        m = re.search(re.escape(label) + r"\s+([0-9.]+) MB", out)
        return float(m.group(1)) if m else None

    naive, git_sz, delta = mb("1. keep every version"), mb("2. git (after gc)"), mb("3. delta chain (zstd)")
    if None in (naive, git_sz, delta) or delta <= 0:
        record(
            "storage_bench.sh produces a parseable three-way comparison",
            "EXPERIMENTS.md#6c",
            "naive 269.80 MB / git 0.80 MB / delta chain 0.30 MB",
            "all three strategies report a size",
            f"naive={naive} git={git_sz} delta={delta}",
            False,
        )
        return

    factor = naive / delta
    record(
        "Delta chains beat keeping every version by orders of magnitude",
        "EXPERIMENTS.md#6c, ADR-0002",
        "901x smaller than logical on 29 real Ableton saves (~11 KB/save)",
        ">= 20x smaller than naive on a synthetic save chain",
        f"{factor:.0f}x smaller ({naive:.2f} MB -> {delta:.2f} MB)",
        factor >= 20.0,
        "Order of magnitude only. The real 901x depends on real save churn; "
        "this fixture's churn rate is an input, not a measurement.",
    )
    record(
        "Delta chains beat git on project-file history",
        "EXPERIMENTS.md#6c",
        "git lands within 3x of a tuned delta chain — git is NOT bad at project files",
        "delta chain < git < naive",
        f"naive {naive:.2f} MB > git {git_sz:.2f} MB > delta {delta:.2f} MB",
        delta < git_sz < naive,
        "The documented point is that git is respectable here. If git ever wins, "
        "ADR-0002's premise changes.",
    )

    # 6a vs 6b: the correction that CDC is the wrong primitive for project files.
    out = run(
        [sys.executable, os.path.join(EXP, "cdc_dedup.py"), "--store", os.path.join(chain, "*.als"), "--gunzip"]
    )
    print(out)
    m = re.search(r"average per version\s*:\s*([0-9.]+) MB", out)
    cdc_per_save = float(m.group(1)) if m else None
    delta_per_save = delta / versions
    ok = cdc_per_save is not None and delta_per_save < cdc_per_save
    record(
        "CDC is the wrong primitive for project files (the ADR-0002 correction)",
        "EXPERIMENTS.md#6, ADR-0002",
        "delta chains 29x better than CDC on Ableton history (11 KB vs 310 KB per save)",
        "delta chain cost per save < CDC store cost per save",
        f"delta {delta_per_save*1000:.0f} KB/save vs CDC {(cdc_per_save or 0)*1000:.0f} KB/save",
        ok,
    )


# ---------------------------------------------------------------------------
# Experiment 10 — FL Studio structure
# ---------------------------------------------------------------------------


def check_flp(work):
    path = os.path.join(work, "fixture.flp")
    synth("flp", path)
    out = run([sys.executable, os.path.join(EXP, "flp_parse.py"), path])
    m = re.search(r"variable-length payload: \d+ B \(([0-9.]+)%", out)
    pct = float(m.group(1)) if m else 0.0
    events = re.search(r"events: (\d+)", out)
    record(
        "FLP event stream parses and is dominated by opaque payload",
        "EXPERIMENTS.md#10, ADR-0003",
        "1,491 events, 82 distinct ids, ~92% of a real .flp is opaque plugin state",
        "parser reads the stream and reports opaque payload as the dominant share (>80%)",
        f"{events.group(1) if events else '?'} events, {pct:.1f}% opaque payload",
        pct > 80.0 and bool(events),
        "The synthetic fixture's ratio is chosen, not measured. What is asserted "
        "is that the parser still reads the format and still classifies payload.",
    )
    record(
        "FL path tokens are still recognised as portable",
        "EXPERIMENTS.md#10",
        "FL references samples with %FLStudioData% tokens rather than absolute paths",
        "the %VARIABLE% token note is emitted",
        "token note emitted" if "%FLStudioData%" in out else "missing",
        "%FLStudioData%" in out,
    )


# ---------------------------------------------------------------------------


def summarise():
    failed = [r for r in RESULTS if not r["ok"]]
    lines = [
        "## Claim invariants (synthetic fixtures — NOT the published numbers)",
        "",
        "Wit's published figures were measured on real commercial sessions that cannot be",
        "committed to this repo. CI therefore asserts the **direction and order of magnitude**",
        "each documented number depends on, using fixtures generated at run time.",
        "A green check here does **not** reproduce a published figure.",
        "",
        "| | Invariant | Observed (synthetic) | Published (real material) | Source |",
        "|---|---|---|---|---|",
    ]
    for r in RESULTS:
        icon = "✅" if r["ok"] else "❌"
        lines.append(
            f"| {icon} | {r['name']}<br><sub>{r['invariant']}</sub> | `{r['observed']}` | "
            f"{r['published']} | `{r['doc']}` |"
        )
    lines += ["", f"**{len(RESULTS) - len(failed)}/{len(RESULTS)} invariants hold.**", ""]
    if failed:
        lines += [
            "> A failure here means a documented claim may no longer be safe to quote.",
            "> Re-measure on real material before changing the assertion — see AGENTS.md,",
            "> \"Rules for claims and numbers\". Do not quietly relax the threshold.",
            "",
        ]
    text = "\n".join(lines)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as fh:
            fh.write(text + "\n")
    print(text)
    return 1 if failed else 0


def main():
    ap = argparse.ArgumentParser(description="Assert the invariants docs/ numbers rest on")
    ap.add_argument("--work", help="working directory (default: a temp dir)")
    ap.add_argument("--quick", action="store_true", help="smaller fixtures, for local runs")
    args = ap.parse_args()

    for tool in ("zstd", "git", "bash"):
        if not shutil.which(tool):
            sys.exit(f"required tool not found: {tool}")

    work = args.work or tempfile.mkdtemp(prefix="wit-claims-")
    os.makedirs(work, exist_ok=True)
    # Deliberately outside the repo: nothing generated here may ever be committed.
    print(f"synthetic fixtures in {work}\n")

    started = time.time()
    # Below ~0.8 MB the fixture holds too few 8 KB chunks for the shift
    # invariant to have any margin: losing one boundary costs a visible percent.
    check_chunking(work, 0.8 if args.quick else 1.2)
    check_semantic_diff(work)
    check_storage(work, 4 if args.quick else 6, 4000 if args.quick else 20000)
    check_flp(work)
    print(f"elapsed: {time.time() - started:.1f}s")

    if not args.work:
        shutil.rmtree(work, ignore_errors=True)
    return summarise()


if __name__ == "__main__":
    sys.exit(main())
