# tools/lab — the Rosetta lab

The lab makes the **same scripted edits in every DAW, one edit per save, each save
labelled**. The resulting corpus is Wit's ground truth: the test suite, the source of
new Logic field mappings, and the in-app "What Wit can see" list.

A human or a computer-use driver makes the edits in the DAW. These tools only prepare,
check, record and compare. **None of them launches a DAW, sends a keystroke or touches
the network.** All of them are Python 3.9+, standard library only.

| Tool | Does |
|---|---|
| `preflight.py` | Read-only: will this DAW reopen a real project at launch? Exit 0 safe, 2 unknown, 3 would reopen. |
| `make_audio.py` | Writes the four synthetic test WAVs (incl. `آواز.wav`) into `~/WitLab/audio/`. Deterministic. |
| `steps.py` | The edit script as data: 31 saves over the plan's 26 rows, per-DAW instructions and expected changes. |
| `capture.py` | `init` / `watch` / `next` / `status`: snapshots each save into the corpus with a manifest. |
| `analyze.py` | Per consecutive step pair: which files changed and which bytes, localised to Logic records (JSONL). |
| `labcore.py` | The shared safety rules. Not a CLI. |

Where things live (never inside this repository):

| What | Where | Override |
|---|---|---|
| Lab root: throwaway projects + test audio | `~/WitLab/` (`<daw>/<run>/`, `audio/`) | `WIT_LAB_ROOT` |
| Corpus: snapshots + `manifest.json` per run | `~/Projects/DAW/wit-corpus/<daw>/<run>/` | `WIT_CORPUS` |

## Safety rules (verbatim; enforced in code, tested in `tests/test_lab_tools.py`)

From the plan (Phase B, "Hard safety rules"):

- Work only in `~/WitLab/<daw>/<run>/`, on new projects.
- Never open anything under `~/Music/Logic`, `~/Projects/DAW/{Logic Pro,Abelton,FL Studio}`
  or `~/Documents/Image-Line/FL Studio/Projects`. Logic 12.3.1 migrates files when it
  saves.
- Before launching a DAW, read its startup-action preference (`defaults read`, read-only)
  so it can't auto-reopen a real project. If it would, stop and ask Sepehr.
- Always use Save As into `~/WitLab`. Logic's default location is `~/Music/Logic`.
- Stop and hand over to Sepehr for any sign-in, license, microphone or system dialog.
- Computer-use is one screen, so runs are serial.

For the tooling:

- Capture only ever READS from projects under the lab root (`~/WitLab/<daw>/<run>/`).
  Refuse (exit non-zero, clear message) if the watched project resolves (realpath)
  outside the lab root, or anywhere under `~/Music/Logic`, `~/Music/GarageBand`,
  `~/Projects/DAW/Logic Pro`, `~/Projects/DAW/Abelton`, `~/Projects/DAW/FL Studio`,
  `~/Documents/Image-Line/FL Studio/Projects`.
- Capture never writes inside the lab project; it writes only into the corpus dir.
- Snapshot project files ONLY, never media: Logic/GarageBand `Alternatives/*/ProjectData`,
  `Alternatives/*/MetaData.plist`, `Resources/ProjectInformation.plist`; Ableton the
  `.als`; FL the `.flp` plus its `Backup/*.flp` autosaves. Skip `Media/`, `Samples/`,
  audio extensions, `Undo Data.nosync`.
- Nothing in this lane launches a DAW, sends keystrokes, or talks to the network.

Where each rule lives in code: `labcore.py` (lab root / denylist / corpus / realpath
checks, atomic writes, `O_NOFOLLOW` reads), `capture.py` (`classify`, `collect` — the
whitelist and symlink refusal — and `copy_snapshot`), `preflight.py` (`is_read_only` —
the only commands it can run), `make_audio.py` (`generate` — writes only under the lab
root).

## The runbook (what the driver does, in order)

One DAW at a time. Keep hands off the Mac during a run (about 1–2 h per DAW).

**0. Check the tools pass.** From the repo root:

```bash
python3 -m pytest tests/test_lab_tools.py -q
python3 tools/lab/steps.py --check
```

**1. Preflight — before the DAW is launched.**

```bash
python3 tools/lab/preflight.py --daw logic      # or ableton | fl | garageband
```

- Exit **0**: go on.
- Exit **2** (unknown) or **3** (would reopen a real project): **stop. Show Sepehr the
  report and ask.** Do not launch the DAW until he says so. The `next:` line in the
  report says what he is being asked to confirm. When a value's meaning is only
  inferred and he confirms it, re-run with `--accept-inferred`.

As measured on 2026-09-29 every DAW is exit 2 until confirmed: Logic's `startupAction`
is unset (the factory default can't be read without launching Logic), GarageBand's
value 3 and Live's absence of a reopen option are safe only by inference, and FL's
`StartupOptionBox` value is not mapped.

**2. Test audio (once per machine).**

```bash
python3 tools/lab/make_audio.py
```

**3. Make the run folder.** Pick a run name (`r1`, `r2`, …; letters, digits, `.`, `_`, `-`):

```bash
mkdir -p ~/WitLab/logic/r1
```

**4. In the DAW: a NEW project, saved straight into the run folder.** Follow step 1 of
`python3 tools/lab/steps.py --daw logic`. In the Save As dialog, confirm the location
reads `~/WitLab/logic/r1` before clicking Save. If the dialog points anywhere else and
you cannot change it with certainty, **stop and ask.**

**5. Start the capture.**

```bash
python3 tools/lab/capture.py init --daw logic --run r1 --project ~/WitLab/logic/r1/Lab.logicx
```

`--project` is the `.logicx` / `.band` package, the `.als` (or its `Lab Project` folder)
or the `.flp`. If the DAW app is ambiguous, pin it with `--app "/Applications/Logic Pro.app"`.

**6. The loop.** For every step:

```bash
python3 tools/lab/capture.py watch     # step 1 is the baseline you just saved
python3 tools/lab/capture.py next      # prints the next edit: do it in the DAW, then save
# … make exactly that edit, save (⌘S), then:
python3 tools/lab/capture.py watch
```

`watch` waits until the project has been unchanged for 2 s **and** differs from the
previous capture (or, for steps marked "identical bytes allowed", until a new save has
landed), copies the project files into the corpus and records them in `manifest.json`.
It times out after 5 minutes with a hint (exit 3).

When reality differs from the instructions:

| Situation | Do |
|---|---|
| The DAW did something the instruction didn't say (e.g. forced a first track) | `watch --note "what really happened"` |
| This DAW can't do the step | `next --skip "why"` |
| A save was captured before the edit was made | make the edit, save, `watch --replace` (the old capture is kept, marked superseded) |
| You want to see where you are | `capture.py status` |

**7. Finish.** When `next` says the run is complete: quit the DAW **without** saving
anything else, then

```bash
python3 tools/lab/capture.py status
python3 tools/lab/analyze.py --out ~/Projects/DAW/wit-corpus/logic/r1/analysis.jsonl
```

## Stop rules — hand over to Sepehr, do not click through

- Any **sign-in, licence, authorisation, trial, subscription or paywall** dialog.
- Any **microphone, camera, accessibility, files-and-folders or other macOS permission** prompt.
- Any **system dialog**, software-update offer, or "download additional content / sound
  library" prompt.
- Any dialog that offers to **open, recover or reopen** a project, or reports **missing
  files** or a **project format upgrade/migration**.
- The DAW opens **any project you did not just create** in the lab run folder.
- A Save or Open dialog you cannot point at `~/WitLab/...` with certainty.
- Never use Open Recent. Never open anything outside `~/WitLab`. Never accept a default
  save location in `~/Music`, `~/Documents` or `~/Projects/DAW`.

## Per-DAW notes

- **Logic.** On 2026-09-29 only `/Applications/Logic Pro.app` (bundle id
  `com.apple.logic10`, 12.3.1, build 6682) is installed; `Logic Pro Creator Studio.app`
  (`com.apple.mobilelogic`) is **not** in `/Applications`, though its preferences domain
  still exists. The plan says to confirm the Logic build with Sepehr; `init` records
  whichever it finds and `--app` pins one. preflight checks both preference domains.
- **GarageBand.** Defaults to saving in `~/Music/GarageBand` — a real library. Save As
  into the lab. 12 steps (the plan's "about 10").
- **Ableton Live.** `Save Live Set As…` creates `Lab Project/Lab.als`; pass that `.als`
  (or the `Lab Project` folder) to `init`. Live's own `Backup/*.als` chain is listed in
  the manifest (name, size, mtime), not copied.
- **FL Studio 20.** Autosaves are captured only from a `Backup/` folder inside the lab
  run folder. If FL writes them to its own user-data `Projects/Backup` folder (a real
  library), capture will not see them — changing FL's backup location is a settings
  change, so ask Sepehr first.

## What a capture looks like

```
~/Projects/DAW/wit-corpus/
  current.json                       ← which run watch/next/status act on
  logic/r1/
    manifest.json                    ← app build, OS, arch, steps, every capture
    01-baseline/Lab.logicx/Alternatives/000/ProjectData
    01-baseline/Lab.logicx/Alternatives/000/MetaData.plist
    01-baseline/Lab.logicx/Resources/ProjectInformation.plist
    02-save-no-change/…
    03-tempo-124/…
```

Each capture entry records the step key and id, the edit, the expected change, the
capture time, per-file `size` / `mtime_ns` / `sha256`, whether each file's bytes differ
from the previous capture, the listing of backup slots and bounces, and any `--note`.
The manifest never holds an absolute path: the project is stored relative to the run
folder, and the app location with the home directory redacted.

## Tests

`tests/test_lab_tools.py` covers the safety refusals (outside the lab root, symlinks
escaping it, every denylisted library, a lab root that contains a library, the corpus
inside the lab root or the repo), stability detection with a fake clock, the no-media
snapshot filter for all four DAWs, manifest atomicity and schema, `make_audio`
determinism and valid RIFF headers including the Persian file name, `analyze`
localisation on a synthetic container, and preflight's read-only command gate and
redaction. Every test runs in a temporary HOME; none touches `~/WitLab` or the corpus.
