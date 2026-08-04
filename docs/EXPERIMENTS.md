# Experiments

Every quantitative claim Wit makes comes from here. Each entry states the method, the
material, the result, and the limits of what it proves.

**Material.** Unless noted, measurements are on real, full-scale projects, not synthetic
fixtures:

| Fixture | What it is |
|---|---|
| `Artefakt - Undertow` (Ableton) | A commercial-grade Live 12 set, 28 tracks, 688 audio clips, 5,112 warp markers. **30 sequential autosaves** of the same project spanning Feb–May 2026. |
| `You make my crazy!` (Logic) | A real Logic 12 project, 459 MB, 33 audio files. **10 sequential saves** (`Project File Backups/00..08` + current). |
| `Aston Martin Music Remake` (FL Studio) | A real FL 10-era `.flp`, 136 KB, 18 channels. |
| A 30-project Logic library | 26 GB across 30 real projects — used for the duplication analysis. |

Labels used throughout: **measured** (we ran it), **cited** (someone else's result, with
a source), **inferred** (reasoning, not measurement).

---

## 1. How much of a DAW save is actually a change?

**Method.** Decompress consecutive `.als` autosaves (`gunzip -c`), diff the XML, count
changed lines.

**Result — measured.** Across 9 consecutive saves of the same project:

| Save pair | Changed lines | % of file |
|---|---|---|
| 1 → 2 | 8,387 | 3.59% |
| 2 → 3 | 1,258 | 0.54% |
| 3 → 4 | 202 | 0.09% |
| 4 → 5 | 168 | 0.07% |
| 5 → 6 | 592 | 0.25% |
| 6 → 7 | 170 | 0.07% |
| 7 → 8 | 408 | 0.18% |
| 8 → 9 | 344 | 0.15% |
| 9 → 10 | 517 | 0.22% |

A typical save changes **0.07–0.25%** of a 232,000-line file. The project is 8.9 MB of
XML; the change is a few kilobytes.

**What the changes actually are.** Inspecting them by hand was more informative than the
totals:

- The 202-line diff (3 → 4) is **entirely one sample being renamed** — Ableton rewrote
  the path in every `FileRef` that pointed at it.
- The 170-line diff (6 → 7) is **entirely scroll position, zoom level and selection
  state**. Nothing musical happened at all.

This is the core justification for semantic diff: the raw line count is dominated by
noise, and the noise is structured and therefore removable.

---

## 2. Are element IDs stable across saves?

This decides whether diff and merge are possible at all. If a DAW renumbers tracks on
each save, you cannot tell "the same track, changed" from "one track deleted, another
added".

**Method.** Extract `<AudioTrack Id="...">` across the save chain and compare.

**Result — measured.** Identical across all 10 saves:

```
153 154 147 148 141 110 100 119 145 128 143 114 122 106 112 142 152
```

**Ableton track IDs are stable.** Clip and warp-marker IDs are likewise stable within a
track. This is the single most important enabling fact for the whole project.

---

## 3. How localized is an edit?

**Method.** Hash each track's XML subtree independently, excluding view-state and
bookkeeping tags, and compare hashes across saves.

**Result — measured.** Of 28 tracks, the number changing per save was:

```
4, 11, 0, 3, 3, 1, 12, 1, 6      → median 3, range 0–12
```

Edits are **local**. Most saves touch a handful of tracks, which is what makes
track-granular merge viable — two people working on different tracks are, structurally,
editing different files.

> **Methodological note, and a correction.** A first pass at this used a *blacklist*
> (hash everything, exclude known churn tags) and reported 3–19 changed tracks including
> saves that were musically empty. Adding more excluded tags changed the answer.
> Blacklists leak. The numbers above use an explicit exclusion set, and the production
> approach should be a *whitelist* — name the fields that matter. This is recorded in
> [AGENTS.md](../AGENTS.md) as a standing convention.

---

## 4. Semantic diff, end to end

**Method.** `experiments/als_semantic_diff.py` builds a model (tracks, mixer state,
device chains, clips with positions and sample references) and diffs the models.

**Result — measured.** The save whose raw diff was 425 clip-level changes reduces to:

```
3 semantic change(s)
  SAMPLE~ '735987__maslovytygr__manual-air-pump.wav' -> 'Manual Air Pump.wav'  (418 clip reference(s))
  SAMPLE~ '221007 Athens, National Garden, Dried twig shaker 3.wav' -> '221007 ... Twig Shaker.wav'  (6 clip reference(s))
  CLIP~   [Source material] '...twig shaker 3' 68.0-96.0 -> 68.0-92.0
```

And a musically empty save reports:

```
no musical change detected (view / bookkeeping only)
```

Real output on a real edit:

```
MIX~    [Stem-mixing] volume: 0.794 -> 0.525
CLIP~   [Corpus metal] '221012_Dragonara Beach, Wood hits' 480.0-495.5 -> 480.0-496.0
CLIP-   [Corpus metal] removed '221012_0106' at bar 495.5
```

**Limit.** This prototype models device *presence*, not device *parameter values*. A save
that only moves a filter cutoff can still report "no musical change". Experiment 3's
subtree hashing does catch those, so the two disagree on 3 of 9 saves — the hashing is
right and the extractor is incomplete. Closing that gap is the main outstanding work on
the differ.

---

## 5. Does merge work?

**Method.** Take save 5 as the common ancestor. "Alice" = save 6, a real edit touching
*Stem-mixing*, *Mod shaker*, *8th shaker*. "Bob" = save 5 with a synthetic edit to a
different track (*Corpus metal* volume `0.146 → 0.331`). Three-way merge with
`git merge-file`, then validate and re-pack.

**Result — measured.** Clean merge, exit 0. Both edits present in the output. The result
parses as valid XML and re-packs to a valid 431 KB `.als`.

**Limits — important.** This is line-based merge succeeding because the two edits were
far apart in the file. It proves *disjoint edits are mergeable in principle*; it does not
prove line-based merge is safe in general — it is not, and production merge must operate
on the object model. **The merged file was not opened in Ableton Live**, so
DAW-acceptance is unverified. That verification is a release gate, not an optional extra.

---

## 6. Storage: what does version history actually cost?

Two techniques were compared on the same material. This experiment produced the most
consequential correction in the project.

**6a. Content-defined chunking (FastCDC-style, ~8 KB average).**

| Material | Result |
|---|---|
| 29 Ableton versions (283 MB logical XML) | 66.9 MB unique chunks → **9.1 MB** after zlib (~310 KB/save) |
| 10 Logic `ProjectData` versions | only **2–87%** reuse per pair (mean ~46%) |

CDC looked adequate for Ableton and poor for Logic.

**6b. Copy/insert delta chains (`zstd -19 --patch-from`).**

| Material | Result |
|---|---|
| 29 Ableton versions | base 135 KB + 28 patches 179 KB = **314 KB total**, ~**11 KB per save**, 901× smaller than logical |
| 10 Logic `ProjectData` versions | base 94 KB + 9 patches 78 KB = **171 KB total**, ~19 KB per save, **62×** |

Individual Logic patches, as % of the target file: 0.28, 0.56, 3.47, 0.34, 0.04, 0.05,
1.11, 0.22, 0.21. Median **0.28%**.

> **Correction.** An earlier iteration of this design assumed content-defined chunking
> was the right storage primitive for project files, based on 6a. It is not. Delta
> chains are **29× better** for Ableton history (11 KB vs 310 KB per save) and turn Logic
> from a weak case into a strong one. The reason is structural: DAW saves produce *many
> small scattered edits with long verbatim runs between them*, which is precisely the
> shape copy/insert deltas encode well and chunk boundaries handle badly. Both figures
> above were reproduced independently.

**6c. Where git actually lands.** Running all three strategies over the same 29 Ableton
versions (`experiments/storage_bench.sh`):

```
1. keep every version          269.80 MB
2. git (after gc --aggressive)   0.80 MB
3. delta chain (zstd)            0.30 MB    <-- Wit
```

**Be precise about this: git is not bad at project files.** `.als` is XML, git deltas
text well, and git gets within 3× of a tuned delta chain. Git's failure is specific and
total on **audio** (experiment 7), where it stores a full copy per version and its
compressor is not audio-aware. Any claim that "git can't do music" should be stated as
"git can't do *audio*" — the project file was never the problem.

**Design consequence.** Delta chains for project-file history; content-addressing plus
audio-aware compression for the audio. They solve different problems and Wit uses both.

---

## 7. The wave-as-binary question

The most important negative result. **Can you version rendered audio at the byte level?**

**Method.** Take a real 19.6 MB 24-bit/48 kHz stem. Produce three variants with `ffmpeg`,
each simulating a real producer action. Measure chunk-level reuse against the original.

**Result — measured.**

| Producer action | Bytes reusable |
|---|---|
| Prepend 250 ms (region moved in time) | **99.59%** |
| Attenuate one 5-second section, re-render | **92.98%** |
| Change gain by −0.5 dB across the whole file, re-render | **0.00%** |

The third row is the finding. A change that is inaudible in character — every part of the
track still sounds like itself — leaves **literally zero bytes** in common, because every
sample value is now a different number. Perceptual similarity is not byte similarity.

**What git does with the same material — measured, after `gc --aggressive`:**

| Repository contents | `.git` size |
|---|---|
| 1 version (baseline) | 16.3 MB |
| the same file committed 3× | 16.3 MB — content addressing dedups perfectly |
| original + localized 5s re-render | 17.4 MB — **+1.1 MB**, git deltas this *well* |
| original + localized + global re-render | 34.2 MB — **+16.8 MB**, a full second copy |

> **Correction to an earlier version of this document**, which claimed git "stores a full
> new copy every time". That is false and the measurement above disproves it. At 19.6 MB
> the file is far below `core.bigFileThreshold` (default 512 MB), so git *does* attempt
> delta compression, and on the localized edit it succeeds — adding only 1.1 MB for a
> change with 93% reusable content. Git's packfile heuristics are better than the
> folklore says.

**The real finding is sharper than "git is bad".** Git and content-defined chunking
*agree*:

| | localized re-render | global re-render |
|---|---|---|
| CDC reuse | 93.0% | **0.00%** |
| git incremental cost | +1.1 MB | **+16.8 MB (full copy)** |

Both handle a localized edit well. **Both fail totally on the global one** — and neither
is at fault, because there is genuinely no shared byte sequence to find. Compression does
not help either, since git's compressor is not audio-aware:

```
zlib -9 (what git uses) on PCM: keeps 82.6%
FLAC -8 (audio-aware)   on PCM: keeps 49.9%
```

So the conclusion is not "use a better delta algorithm". It is that **byte-level
versioning of rendered audio is unfixable in exactly the case that matters most** — the
producer who changes one plugin and re-bounces. A real session has 20–60 stems and that
workflow happens daily.

**This is why Wit versions the recipe, not the render.**

---

## 8. Lossless compression on real music

**Method.** FLAC `-8` on a random sample of 10 files from the 26 GB Logic library, plus
targeted files.

**Result — measured.** Sample total: 126.4 MB → 59.6 MB, **FLAC keeps 47.2%**.

Per-file it varies enormously, and the variance is the point:

| File | Keeps |
|---|---|
| `Orchestra Strings 27.wav` (full-length stem, mostly silence) | **1.1%** |
| `Silky Acid Bass.wav` | 10.4% |
| `Untitled 3_14 #03.wav` | 60.0% |
| `Artefakt - Dragonet [Stem Mixing].wav` (dense master) | 65.2% |

Full-length per-track stem exports are mostly silence and compress ~100×. Dense mastered
material barely compresses. **Never quote a single FLAC ratio without saying what
material it was measured on.**

---

## 9. What a real library actually costs

**Method.** Walk 30 real Logic projects (26 GB). Hash every audio file; group by size
first, then hash only size-collision candidates.

**Result — measured.**

```
audio files:                    882
total audio:                  21.95 GB
exact duplicate audio:         5.38 GB   (24.5%)
```

**A quarter of the library is byte-identical duplicates**, produced by `Save As`
branching — `Pr0oject 5 DESERT` / `Pr0oject 5 SAD DESERT` / `Project 5 – Intro` are three
copies of one song.

Modelling the whole library under Wit's design:

```
current on disk                          26.0 GB
  of which audio                         21.9 GB
  exact duplicate audio (measured)      - 5.4 GB
  unique audio                           16.6 GB
  after FLAC @47.2% (measured)            7.8 GB
  + project files & full version history   0.3 GB
  ----------------------------------------------
  TOTAL with content-addressed store       8.1 GB   (3.2x smaller)
```

Wit is **smaller than the status quo while adding complete version history that does not
exist today**. This is the strongest single argument for the design.

**Limits.** Exact-duplicate detection only; sub-file chunk dedup would find more. FLAC
ratio is extrapolated from a 10-file sample to the whole library — the sample was random
but small, so treat 3.2× as approximate.

---

## 10. Format survey results

Summarised here; full detail in [FORMATS.md](FORMATS.md).

- **Ableton `.als`** — gzipped XML. 473 KB → 8.9 MB.
- **Logic `.logicx`** — package; `ProjectData` is a chunked binary, magic
  `23 47 C0 AB`, little-endian FourCCs (`gnoS` = "Song"). Container structure is
  understood well enough to enumerate records.
- **GarageBand `.band`** — **the same container format as Logic** (`23 47 C0 AB`,
  `gnoS`, same package layout); only a version word differs (`cb09` vs `d009`). One
  parser serves both.
- **FL Studio `.flp`** — `FLhd`/`FLdt` chunks then a typed event stream. Measured on the
  real file: 1,491 events, 82 distinct IDs, and **92% of the file is opaque
  variable-length plugin state**. FL references samples with `%FLStudioData%` path
  tokens, which is more portable than Ableton's absolute paths.

**Sample references — measured.** Ableton stores absolute paths. The test project
contains **777 references to `/Users/nicklapien/...`**, 16 to `/Users/pma/...`, and 13
more — three different people's home directories baked into one file, on a fourth
person's machine. It also stores `OriginalFileSize` and `OriginalCrc` per sample: a
primitive content fingerprint already present in the format, and a usable hook for
automatic sample resolution.

**Derived vs source audio — measured.** In the Ableton project's sample folder:

```
Imported  (source, immutable)    715 MB   43.6%
Recorded  (source, immutable)     74 MB    4.5%
Processed (DERIVED: Freeze/Consolidate/Crop)  850 MB   51.8%
```

**52% of the audio is regenerable** and does not belong in version history at all.

---

## Reproducing these

```bash
python3 experiments/als_semantic_diff.py --chain 'path/to/Backup/*.als'
python3 experiments/cdc_dedup.py --store 'path/to/Backup/*.als' --gunzip
python3 experiments/flp_parse.py path/to/project.flp
bash    experiments/storage_bench.sh path/to/Backup
```

`ffmpeg`, `flac` and `zstd` are needed for experiments 6b, 7 and 8.

## Open questions these experiments did **not** settle

1. **Does Ableton open a Wit-merged file?** Not tested — no DAW was launched. Blocking
   for any release.
2. **Are DAW renders deterministic?** If bouncing the same project twice is not
   bit-identical, "did the audio change" cannot be answered by hashing renders.
3. **Do Ableton IDs stay stable across Live *versions*, and across
   duplicate/copy-paste?** Only within-version stability was measured.
4. **FL Studio save locality under a controlled edit.** Requires launching FL to produce
   two versions differing by one known change.
5. **Sub-file chunk dedup on a whole library** — experiment 9 measured exact duplicates
   only.
