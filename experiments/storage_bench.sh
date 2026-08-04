#!/usr/bin/env bash
#
# storage_bench.sh — compare storage strategies on a real chain of DAW saves.
#
# WHAT THIS MEASURES
#   How much disk a full version history costs under three strategies:
#     1. naive        — keep every version (what "Save As" does today)
#     2. git          — commit every version to a git repo
#     3. delta chain  — zstd --patch-from against the previous version  <-- Wit
#
#   This is the experiment that overturned an earlier design assumption. Delta
#   chains beat content-defined chunking by ~29x on Ableton project history,
#   because DAW saves produce many small scattered edits with long verbatim runs
#   between them.
#
# USAGE
#   ./storage_bench.sh /path/to/Ableton/Backup          # *.als (auto-gunzipped)
#   ./storage_bench.sh "/path/to/Project File Backups"  # Logic ProjectData
#
# REQUIRES
#   zstd (>= 1.4.4 for --patch-from), git
#
# WHAT THIS DOES NOT HANDLE
#   - Chained deltas mean restoring version N costs N patch applications. A real
#     implementation checkpoints periodically. That trade-off is not modelled.
#   - Audio is deliberately excluded; audio is not versioned this way. See
#     docs/decisions/0001-version-the-recipe-not-the-render.md
#   - Read-only: everything happens in a temp dir.

set -euo pipefail

SRC="${1:?usage: storage_bench.sh <dir-of-sequential-saves>}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

command -v zstd >/dev/null || { echo "zstd required"; exit 1; }

# `stat` differs between BSD/macOS (-f%z) and GNU/Linux (-c%s).
filesize() { stat -f%z "$1" 2>/dev/null || stat -c%s "$1"; }

# --- collect versions in chronological order --------------------------------
mkdir -p "$WORK/v"
n=0
if compgen -G "$SRC/*.als" >/dev/null; then
  echo "Material: Ableton .als (gzipped XML — decompressing to compare true content)"
  while IFS= read -r f; do
    n=$((n+1)); gunzip -c "$f" > "$WORK/v/$(printf '%03d' $n).bin"
  done < <(find "$SRC" -maxdepth 1 -name '*.als' | sort)
elif compgen -G "$SRC/*/ProjectData" >/dev/null; then
  echo "Material: Logic ProjectData"
  while IFS= read -r f; do
    n=$((n+1)); cp "$f" "$WORK/v/$(printf '%03d' $n).bin"
  done < <(find "$SRC" -maxdepth 2 -name 'ProjectData' | sort)
else
  echo "No *.als or */ProjectData found in $SRC"; exit 1
fi
[ "$n" -ge 2 ] || { echo "need >= 2 versions, found $n"; exit 1; }

logical=$(cat "$WORK"/v/*.bin | wc -c | tr -d ' ')
echo "Versions: $n    logical total: $(echo "scale=1; $logical/1048576" | bc) MB"
echo

# --- 1. naive ---------------------------------------------------------------
printf "  %-26s %10.2f MB\n" "1. keep every version" "$(echo "scale=4; $logical/1048576" | bc)"

# --- 2. git -----------------------------------------------------------------
G="$WORK/git"; mkdir -p "$G"; git init -q "$G"
git -C "$G" config user.email bench@wit.local
git -C "$G" config user.name  bench
for f in "$WORK"/v/*.bin; do
  cp "$f" "$G/project.bin"
  git -C "$G" add project.bin
  git -C "$G" commit -qm "$(basename "$f")"
done
git -C "$G" gc -q --aggressive 2>/dev/null || true
gitsz=$(du -sk "$G/.git" | cut -f1)
printf "  %-26s %10.2f MB\n" "2. git (after gc)" "$(echo "scale=4; $gitsz/1024" | bc)"

# --- 3. delta chain (Wit) ---------------------------------------------------
# Version files are named %03d.bin by the collector above, so a sorted glob is
# already chronological. Use an array rather than parsing `ls` output.
versions=("$WORK"/v/*.bin)
first="${versions[0]}"
zstd -19 -q -f "$first" -o "$WORK/base.zst"
total=$(filesize "$WORK/base.zst")
prev="$first"
for f in "${versions[@]:1}"; do
  zstd -19 --long=27 -q -f --patch-from="$prev" "$f" -o "$f.patch" 2>/dev/null
  total=$((total + $(filesize "$f.patch")))
  prev="$f"
done
printf "  %-26s %10.2f MB    <-- Wit\n" "3. delta chain (zstd)" "$(echo "scale=4; $total/1048576" | bc)"

echo
printf "  delta chain is %.0fx smaller than keeping every version\n" \
  "$(echo "$logical/$total" | bc -l)"
printf "  delta chain is %.0fx smaller than git\n" \
  "$(echo "($gitsz*1024)/$total" | bc -l)"
printf "  average cost per save: %.1f KB\n" "$(echo "$total/$n/1024" | bc -l)"
