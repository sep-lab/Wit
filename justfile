# Wit — task runner.
#
# Recipes here are referenced by name from wit-planning/PLAN.md (outside this repo) and
# from CONTRIBUTING.md. `release-unsigned` and `release-signed` are stubs until M7-lite —
# they fail loudly with what's missing rather than silently doing nothing, on the same
# "never claim more than is true" doctrine as the rest of this repo (see AGENTS.md).

_default:
    @just --list

# --- Everything that exists today (Python research + the M0 Rust skeleton) ---

# Run the full test suite: Python (experiments/tests) + Rust (crates/).
test: test-python test-rust

test-python:
    python3 -m pytest tests/ -q

test-rust:
    cargo test --workspace --locked

# Run every lint CI runs, so a red PR never surprises you.
lint: lint-python lint-rust

lint-python:
    #!/usr/bin/env bash
    set -euo pipefail
    # tools/lab/ (stdlib-only Python, PLAN-V2 Phase B) lands from a separate
    # lane; include it once it exists so `just lint` matches CI exactly
    # (.github/workflows/ci.yml's lint job) without an edit here when it does.
    dirs=(experiments/ tests/)
    if [ -d tools ]; then
        dirs+=(tools/)
    fi
    ruff check "${dirs[@]}"

lint-rust:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets --locked -- -D warnings

# cargo-deny license/advisory check — same gate as the CI `licenses` job.
licenses:
    cargo deny check licenses advisories

# The M2 gate: fetch the MIT-licensed jonkubis/LogicProFormatWriter corpus at the
# pinned commit into a temp dir (never into the repo — CI refuses ProjectData
# paths) and walk every real Logic save in it. Needs network.
logic-fixtures-gate:
    #!/usr/bin/env bash
    set -euo pipefail
    dir="$(mktemp -d)"
    trap 'rm -rf "$dir"' EXIT
    git clone --quiet https://github.com/jonkubis/LogicProFormatWriter "$dir/lpfw"
    git -C "$dir/lpfw" checkout --quiet 1f77c5c37d49ccd9551cc8e9107750e8db2f1fed
    WIT_JONKUBIS_DIR="$dir/lpfw" cargo test -p wit-logic --test jonkubis_fixtures -- --ignored --nocapture

# --- What lands starting M5 (the Tauri app) — stubs until then ---

# Build a synthetic ~/Music-shaped tree (two .logicx packages with backups, a .band, and
# one .als lineage) so first-run, the timeline, and the watcher are demoable on any
# machine, not just a machine with real Logic projects on it. See issue #18.
#
# Refuses to write into a directory that already has anything in it, so pointing this at
# a real library by mistake cannot destroy anything.
demo-library dest="target/demo-library":
    cargo run -p wit-cli -- demo-library "{{dest}}"

# Build an unsigned, ad-hoc-signed .app + .zip for pilot distribution (no Apple Developer
# ID). Apple Silicon requires at least an ad-hoc signature to launch at all — this recipe
# asserts one is present rather than assuming `tauri build` added it.
release-unsigned:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ ! -d app/src-tauri ]; then
        echo "error: app/src-tauri doesn't exist yet — the Tauri app lands at M5." >&2
        echo "See docs/ROADMAP.md 'Now: the 0.0 pilot' for what's built so far." >&2
        exit 1
    fi
    (cd app && npm run tauri build)
    APP="app/src-tauri/target/release/bundle/macos/Wit.app"
    codesign -dv "$APP"   # fails loudly if no signature (even ad-hoc) is present
    ditto -c -k --sequesterRsrc --keepParent "$APP" "Wit-unsigned.zip"
    shasum -a 256 "Wit-unsigned.zip"
    echo "Unsigned build. Recipients need the Sequoia Gatekeeper walkthrough in PILOT.md."

# Build a notarized .dmg. Requires Apple Developer ID enrollment (M0's action item) and
# the usual notarytool credentials in the keychain — this is intentionally a hard stop
# until that exists, not a silently-degrading fallback to release-unsigned.
release-signed:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "error: release-signed needs an Apple Developer ID (\$99, enrollment takes" >&2
    echo "days-to-weeks) and notarytool credentials. Neither is set up yet." >&2
    echo "Use 'just release-unsigned' for the 0.0 pilot; see PILOT.md." >&2
    exit 1
