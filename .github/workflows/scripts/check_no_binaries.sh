#!/usr/bin/env bash
#
# Guardrail: no audio and no DAW project files may ever enter this repository.
#
# WHY
#   This is not tidiness. Wit exists because git is bad at audio (ADR-0001), so a
#   repo full of committed stems would be self-refuting. It is also a legal and
#   privacy matter: real project files carry other people's copyrighted samples
#   and other people's absolute home paths. See AGENTS.md, "Rules for handling
#   user data", and CONTRIBUTING.md.
#
# WHAT IT CHECKS
#   1. extension / path blocklist — the formats .gitignore already refuses
#   2. per-file size ceiling      — because the binary format nobody listed yet
#                                   will still be big, and a blocklist only
#                                   catches what someone thought of
#
# USAGE
#   .github/workflows/scripts/check_no_binaries.sh [max_file_bytes]

set -euo pipefail

MAX_FILE_BYTES="${1:-2097152}"   # 2 MiB. Nothing in a design-phase repo is bigger.

fail=0

# --- 1. extensions and package paths ----------------------------------------
# Deliberately wider than .gitignore: a check that only matches what is already
# ignored can never catch a new mistake. DAW "files" are often directories, so
# path segments are matched as well as extensions.
audio='\.(wav|aif|aiff|caf|flac|mp3|m4a|ogg|opus|wv|aac|sd2|rex|rx2|sf2|sfz)$'
projects='\.(als|alp|asd|flp|ptx|ptf|cpr|npr|song|rpp|rpp-bak|dawproject|omf|aaf)$'
packages='(^|/)[^/]+\.(logicx|band|ptx)/|(^|/)(ProjectData|Ableton Project Info|Project File Backups|Freeze|Consolidate)(/|$)'

matches="$(git ls-files | grep -Ei "${audio}|${projects}|${packages}" || true)"
if [ -n "$matches" ]; then
  while IFS= read -r f; do
    [ -n "$f" ] || continue
    echo "::error file=${f}::Audio and DAW project files must never be committed. See .gitignore and AGENTS.md."
    echo "  refused: $f"
  done <<EOF
$matches
EOF
  cat <<'EOF'

  Wit never commits audio or DAW projects — not as fixtures, not as "a small
  sample". Tests generate synthetic fixtures at run time instead; see
  .github/workflows/scripts/synth_fixtures.py for how, and CONTRIBUTING.md
  ("Test fixtures") for what to contribute instead.
EOF
  fail=1
fi

# --- 2. per-file size ceiling -----------------------------------------------
while IFS= read -r f; do
  [ -f "$f" ] || continue
  size=$(wc -c < "$f" | tr -d ' ')
  if [ "$size" -gt "$MAX_FILE_BYTES" ]; then
    echo "::error file=${f}::${f} is $((size / 1024)) KB, over the $((MAX_FILE_BYTES / 1024)) KB per-file limit."
    cat <<EOF

  ${f} is $((size / 1024)) KB.

  This is a design-phase repo of documents and small prototypes; nothing in it
  should exceed $((MAX_FILE_BYTES / 1024)) KB. If it is audio or a DAW project it cannot go in at
  all. If it is something else that genuinely belongs here, raise MAX_FILE_BYTES
  in this script in the same PR and say why in the commit message.
EOF
    fail=1
  fi
done < <(git ls-files)

if [ "$fail" -eq 0 ]; then
  echo "clean: no audio, no DAW projects, nothing over $((MAX_FILE_BYTES / 1024)) KB"
fi
exit "$fail"
