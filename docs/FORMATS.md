# DAW project formats — a reference for Wit

What each DAW actually writes to disk, how amenable it is to version control, and what
is still unknown.

Claims are labelled **[verified]** (we parsed it ourselves on real files),
**[cited]** (someone else's published work), or **[inferred]**.

---

## Summary

| DAW | Container | Encoding | Semantic diff | v1 |
|---|---|---|---|---|
| Ableton Live | single `.als` | gzip → XML | **feasible today** | 🟢 |
| Logic Pro | `.logicx` package | chunked binary | regions and their positions; no parameters | 🟡 |
| GarageBand | `.band` package | **same as Logic** | same as Logic | 🟡 |
| FL Studio | single `.flp` | typed event stream | feasible ≤ v24 | 🟡 |
| Studio One | `.song` | ZIP + XML | likely straightforward | 🟡 |
| Pro Tools | `.ptx` | XOR-obfuscated TLV | partial (~35% of blocks) | 🔴 |
| Cubase | `.cpr` | MFC object stream | no parser for modern files | 🔴 |
| Reaper | `.rpp` | **plain text** | trivial | ⚪ |

---

## Ableton Live — `.als` 🟢

**[verified]** A gzip stream containing a single UTF-8 XML document. The test project is
473 KB compressed, **8.9 MB** expanded, 232,000 lines.

```xml
<Ableton MajorVersion="5" MinorVersion="12.0_12300" Creator="Ableton Live 12.3.5" ...>
  <LiveSet>
    <Tracks>
      <AudioTrack Id="153"> ... </AudioTrack>
```

**Structure.** `LiveSet` → `Tracks` → `AudioTrack` / `MidiTrack` / `GroupTrack` /
`ReturnTrack`, each with a `DeviceChain` containing a `Mixer` and a device list, and
arrangement clips under `ArrangerAutomation/Events`.

Element census from the real project: 17 audio tracks, 5 MIDI, 12 group, 1 return; 688
audio clips, 16 MIDI clips; **5,112 warp markers**; 1,139 `FileRef`s; 60 automation
envelopes.

**Track IDs are stable** — verified across 10 consecutive saves of one project: never
renumbered, never recycled, and they survive a rename. This is what makes diff and merge
possible. See EXPERIMENTS.md §2 for the exact scope of that claim.

**Sample references** carry a built-in weak fingerprint:

```xml
<SampleRef><FileRef>
  <RelativePathType Value="3"/>
  <RelativePath Value="Samples/Imported/221007 Dragonara Beach, Wood Hits.wav"/>
  <Path Value="/Users/<producer-a>/Downloads/.../Samples/Imported/...wav"/>
  <OriginalFileSize Value="8303822"/>
  <OriginalCrc Value="46531"/>
</FileRef><LastModDate Value="1777964370"/></SampleRef>
```

`OriginalFileSize` + `OriginalCrc` is a content fingerprint Live already maintains — Wit
should use it for sample resolution rather than inventing one.

**The portability problem, measured.** The test project embeds **777 absolute paths under a
collaborator's home directory**, 16 under `/Users/<producer-b>/`, and 13 more — three people's home
directories, on a fourth person's machine. Absolute paths are the format's weakest point.

**Churn to normalise away:** `ScrollerPos`, `CurrentZoom`, `ClientSize`,
`HighlightedTrackIndex`, `IsContentSelectedInDocument`, `SelectedEnvelope`, `CurrentTime`,
`AnchorTime`, `LomId`, `PointeeId`, `NextPointeeId`, `OverwriteProtectionNumber`,
`LastModDate`, and the positional `FileRef`/`AuPreset` `Id` counters. Use a whitelist, not
this blacklist — see EXPERIMENTS.md §3.

⚠️ **`AudioTrack Id` and `FileRef Id` are different kinds of thing.** Track IDs are stable
identities that survive renames and are never recycled. `FileRef`/`AuPreset` IDs are
positional counters that shift by +1 when anything upstream is inserted — the cause of a
measured 130-line diff containing zero semantic change. Key your model on track IDs only.

**Plugin state — [verified], and not what you would guess.** Blobs are **uppercase
hexadecimal**, not base64, wrapped at 80 characters per line:

- `<Blob>` — Live-native device state (14 in the test file)
- `<Buffer>` inside `<Preset><AuPreset>` — AU/VST plugin state (3 in the test file)

And AU state is **double-encoded**: the hex decodes to an Apple XML plist, which itself
contains base64 `<data>` elements. Decoding one gives
`3C3F786D6C2076657273696F6E...` → `<?xml version=...`. A parser that assumes base64 at the
outer layer gets nothing.

Stock devices instead expand to plain parameter elements (a single VST added ~256
`PluginFloatParameter` entries in one observed save). The test project used mostly stock
devices, so its blob total was only ~16 KB — **not representative** of a third-party-heavy
session.

**Live ships its own machine-readable schema — [verified].**
`/Applications/Ableton Live 12 Suite.app/Contents/App-Resources/Schema/*.txt` contains
**173 files**, one per historical serialization version, from `8.1_220` to `12.0_12049`:

```xml
<AbletonSchema Version="4" TranslatorCount="327">
  <AbletonDefaultPresetRef>
    <FileRef Class="FileRef" Type="0" />
    <DeviceId Class="ClassId" Type="-4" />
```

Every element and field, with its `Class` and `Type`. Critically, **the `MinorVersion` in a
`.als` is exactly that `TranslatorCount`** — so version selection and migration are
table-driven rather than guesswork.

⚠️ But the tables **lag the shipping app**: the test project declares `12.0_12300` while
the installed build ships schemas only to `12.0_12049`. So the schemas are the right asset
for *validation and migration*, not a prerequisite for diffing — our semantic diff runs on
those same 12.3 files with no schema access at all.

**The gzip container is deterministic — [verified].** Header bytes are
`1f 8b 08 00 00 00 00 00 00 13`: `FLG=0` (no embedded filename) and **`MTIME=0`** — Ableton
deliberately zeroes the timestamp. So the container contributes no per-save byte churn;
everything comes from the deflate stream. Convenient, but irrelevant in practice, because
Wit gunzips before hashing anyway (see ADR-0002).

**Interchange — [verified]** by scanning the Live 12 binary: **no dawproject, no AAF**.
Live exports audio, MIDI files, and Live packs only.

---

## Logic Pro — `.logicx` 🟡

**[verified]** A macOS package (a directory), not a file:

```
You make my crazy!.logicx/
├── Alternatives/000/
│   ├── ProjectData              1.27 MB   <- the song, custom binary
│   ├── MetaData.plist           3.9 KB    <- binary plist index
│   ├── DisplayState.plist       17.7 KB   <- UI state
│   ├── DisplayStateArchive      29.9 KB   <- NSKeyedArchiver graph
│   ├── WindowImage.jpg          363 KB    <- Finder preview screenshot
│   ├── Project File Backups/00..08/       <- 9 rotating full copies
│   └── Undo Data.nosync/
├── Media/{Audio Files,Impulse Responses,Samples}   447 MB   <- shared
└── Resources/ProjectInformation.plist
```

**Logic already ships ~10 full copies of the project inside the package.** The demand for
version history is not hypothetical — Apple built a crude one.

### `ProjectData` binary format — [verified]

24-byte file header beginning `23 47 C0 AB`, then a stream of records with little-endian
FourCC tags (`gnoS` = "Song" reversed). **22 distinct chunk tags / 2,069 records** were
enumerated on the real project, and the census tracks real user edits:

| Tag | Meaning | Across saves |
|---|---|---|
| `AuFl` | audio files | 35 → 37 (**matches `MetaData.plist` exactly**) |
| `AuCU` | plugins | 70 → 76 → 265 |
| `AuRg` | regions | 79 → 90 → 96 |
| `Trak` | tracks | 248 → 256 → 260 |
| `MSeq`/`EvSq` | sequences | 147 → 155 → 159 |

Region names are readable directly from `AuRg` payloads. `AuFl` payloads revealed a real
semantic event: paths changed from absolute to package-relative between v00 and v09,
i.e. the project was relocated and relinked.

⚠️ **The tag names above are the logical ones. On disk every FourCC is reversed** —
`AuRg` is stored as `gRuA`, `Trak` as `karT`, `EvSq` as `qSvE`, `MSeq` as `qeSM`,
`AuFl` as `lFuA`, `Song` as `gnoS`. Code matches the reversed form; prose in this file
uses the logical one. Both conventions are in use, so state which you mean.

### Region payloads — [verified]

The container census says *"79 regions became 96"*. These two payloads say *which*
region, and where it went. All offsets are **record-relative** (from the start of the
36-byte record header); payload-relative = record − `0x24`. Fields sit on **2-byte**
boundaries, not 4 (the length at `+0x3A`, the name length at `+0x6E`).

**Material and dates.** Unless a figure says otherwise it was measured with
`experiments/logic_region_map.py` on the 10-save chain of the `You make my crazy!` fixture
([EXPERIMENTS.md §0](EXPERIMENTS.md)), re-measured **2026-09-29**; "backup 00" is that
chain's oldest save. Library-wide figures come from `logic_region_map.py --scan` over one
person's **Logic library — 42 `.logicx`/`.band` bundles, 206 saves (the scan does not split
the two kinds) — run 2026-09-29** with the corrected script,
and say so. Per-figure reproduction commands are in
[EXPERIMENTS.md §12](EXPERIMENTS.md#12-logic-region-payloads--which-region-and-where-it-went).

**Upstream.** This section corrects and extends §3, §8 and §8.1 of
[`PROJECTDATA_FORMAT.md`](https://github.com/jonkubis/LogicProFormatWriter/blob/1f77c5c37d49ccd9551cc8e9107750e8db2f1fed/PROJECTDATA_FORMAT.md)
in `jonkubis/LogicProFormatWriter` (MIT, pinned at the SHA `wit-logic` already cites).
Where they agree it says so below; where they differ, the difference is stated.

**`AuRg`/`gRuA` — the region object.**

| Offset | Type | Meaning |
|---|---|---|
| `+0x08` | u32 | `familyIndex << 18`. Low 18 bits are zero on 903/903 records of the chain. No region's family index changes between saves: 0 of 96 on the chain, 0 of the library's 3,121 UUIDs seen in 2+ saves. Upstream §8.1 documents the same `index << 18` encoding. |
| `+0x3A` | u32 | Region length, in frames **of the source file's sample rate** (upstream §8: payload `+0x16`, the same field) |
| `+0x6E` | u16 | Name length, in bytes (upstream §8: payload `+0x4a`, the same field) |
| `+0x70` | UTF-8 | Name, padded to an even length — decode as UTF-8, see below |
| name end `+0x56` | 16 B | Region UUID (RFC 4122 v1 layout) |

The name is variable-length **and the record is sized to fit it**:
`payload_size == 209 + nlen + (nlen & 1)`, exact on all 79 records of backup 00 (40 even
names → `+209`, 39 odd → `+210`) and on 903/903 across the chain. Upstream §8 found the
same resizing independently. Everything behind the name shifts with it, so the
suffix — a constant 133 bytes — must be addressed from the padded name end, never from a
fixed offset. The length field is a *region* length, not a file length: a trimmed region
reads shorter than its source (`Reverse Hat Beat 01.1` = 89,128 frames against the
source's 235,200). Untrimmed lengths match `afinfo`'s "valid frames" on six of the
fixture's source files, e.g. `Deep Down Shaker` at 173,509 frames = 3.934444 s × 44,100.

**Name encoding: decode UTF-8.** Upstream §8 calls the name ASCII. Every name on the
chain is 7-bit ASCII (903/903), where ASCII and UTF-8 are byte-identical, so nothing
measured tells them apart. The port should use UTF-8: it is a strict superset, it is
what `wit-logic`'s `extract.rs` already uses for this field, and a name that is not valid
UTF-8 fails the decode (and is counted) instead of being mis-read. How Logic writes a
non-ASCII name is **untested** — no such name exists in the material.

**The UUID is the important field.** It is unique per region and *stable across saves*:
all 79 region UUIDs of backup 00 are still present in the newest save, which adds exactly
17 more. That is Logic's equivalent of the Ableton `Id` this document relies on
elsewhere, and it is what makes a cross-save region diff possible. Its node field is
random per UUID (96 distinct over the newest save's 96 regions; the multicast bit is set
on 50 and the locally-administered bit on 43), so it is **not** a hardware MAC address
and carries no machine identity.

**`EvSq`/`qSvE` — where position actually lives.**

Region position is **not** in `AuRg`. Upstream §8 puts it in the placement event and
validated that in Logic with a one-bar move (+3840 ticks); the one move on the chain
agrees (backup 00 → 01, bar 55.5 → 55.75: the placement's `+0x04` changed by exactly
960). An earlier version of this section also said two sibling copies of a region differ
by only four bytes outside name and UUID; that did not reproduce — a byte-compare of
same-size sibling records on backup 00, done by hand and not by a script mode, found 2
to 25 differing bytes — and the claim is withdrawn.

> Every `qSvE` payload is a grid of **16-byte units**: 22,500 of 22,500 payloads in the
> library and 1,550 of 1,550 on the chain are an exact multiple of 16. The terminator unit
> is `f1 00 00 00 ff ff ff 3f` + 8 zero bytes, the tail upstream §3 documents. `--scan`
> counts two things about it separately:
>
> - payloads holding exactly one terminator unit;
> - payloads whose last unit is the terminator.
>
> On the chain both are 1,550 of 1,550, so every chain payload does both. In the library
> each is **22,480 of 22,500**. The scan does not count payloads that satisfy both, so
> that the same 22,480 do is **[inferred]**. Why the other 20 differ is **open** (below).

Byte `+7` of each unit behaves like a type code — **[inferred]**; nothing here interprets
it. Backup 00 carries **12** distinct values: `00 3f 88 89 8a a3 a4 a7 aa b2 bb bc`; the
library carries **21**, adding `85 8d a0 af b1 b3 b4 b8 be`. On the chain `3f` occurs only
on the terminator. Every one of the 1,550 payloads holds exactly one terminator unit, so
there are 1,550 terminator units, and there are 1,550 `3f` units in all. The library has
22,500 `3f` units, but the scan does not count terminator *units* (only the two payload
counters above), so whether `3f` appears outside the terminator there is **unmeasured**.
Upstream §3 gives fixed event sizes
(tempo 32 B, signature and marker 48 B) and §8.1 80-byte placement events; all are
multiples of 16, consistent with the grid, which is the one invariant that held on every
payload measured.

An audio placement is a 48-byte group — three units — headed by `24 00 00 00` on that grid.
Within the group:

| Offset | Type | Meaning |
|---|---|---|
| `+0x04` | u32 | Position = `34560 + tick@960` (regions use origin 34560, tempo/markers 38400). Its **high byte `+0x07` is also the head unit's byte `+7`**: `0x00` on every accepted placement head (9,142/9,142 in the library). |
| `+0x10` | u32 | Upstream §8.1 calls it a per-region id. **Not a per-placement key** — on backup 00 the 56 placements carry 30 distinct values, one per occupied track (30 tracks, 30 distinct value/track pairs). Do not key on it. |
| `+0x14` | u8 | **Track number, 1-based** |
| `+0x2c` | u32 | Region link; `link / 4` == `familyIndex` (upstream §8.1 agrees) |

**The unit types are a structural collision filter.** The three units of every accepted
placement group carry byte `+7` = `00` / `89` / `bc`: 56/56 on backup 00, 592/592 on the
chain, **9,142/9,142 in the library**. No rejected marker hit has `00` there: the library's
346 rejects carry `88` (180), `89` (143) or `bc` (23). On the chain all 10 rejects carry
`88`, as the previous version of this section said, scoped to the chain. The library shows
other values too, so the property to rely on is "never `00`", not "always `88`". Any
position at or past bar 4,362
(2²⁴ ticks) would put a nonzero value in that same byte. So checking head byte
`+7 == 0x00` separated every accepted placement from every reject in the library, with no
bar bound. The script **counts** this (`placements_with_unit_types_00_89_bc`,
`rejected_marker_head_byte`) and does not filter on it; the Rust port can.

**No fixed stride.** Upstream §8.1 models placement events as 80 bytes, `0x50` apart —
true of its 3-region fixture. On backup 00 the 55 gaps between consecutive placement heads
are 160, 240 (46 of them), 320, 480, 720 and 1,440 bytes, all multiples of 80. Across the
library the 8,873 gaps take 37 sizes from 80 to 11,120 bytes — 160 is the commonest
(4,306), then 80 (2,052) and 240 (1,510) — and **15 gaps (1,776, 2,176 and 2,976 bytes) are
not multiples of 80 at all**. So what held on backup 00's 55 gaps does not hold
library-wide. Every gap is a multiple of 16. Find heads by the marker on the 16-byte grid;
do not stride.

**Decode rates — library, 2026-09-29:**

- 206/206 saves walked to clean EOF.
- **17,913 of 18,015 `gRuA` records seen decoded (99.43%)**; every decoded UUID is distinct
  within its save.
- 9,076 of 9,142 placements resolved to a region family.
- 205 of 206 saves had every decoded track number within that save's `MetaData.plist`
  `NumberOfTracks`.
- 7,808 of 9,142 placements (85.4%) landed on the 960-tick grid. The rest are
  **[inferred]** to be regions placed with snap off: none was checked against Logic's
  display, and the chain has none to check (592/592 on the grid).

On the chain, 903 of 903 records decoded.

**Three published 100% figures do not hold on the current library.** The first version
of this section reported them from a 2026-08-16 scan of the library, when it held 32
projects / 132 saves.

- **"10,020/10,020" regions decoded was wrong by construction.** That scan counted only
  the records that decoded, so it could not have shown a failure. The current library has
  102 records that fail.
- **"5,282/5,282" placements resolved** — 66 now do not.
- **"132/132" saves within `NumberOfTracks`** — one now is not.

The 2026-08-16 counters for these last two could register a failure, and they were
measured on a different, smaller snapshot. They may well have held then. The aggregate
cannot say whether the 66 placements or the one save sit in bundles added since.

The four-byte marker collides with data inside other units. Of the library's 9,488 marker
hits, 346 were rejected:

- 334 carried track 0;
- 12 fell beyond bar 10,000;
- none fell before the origin, and none was truncated.

The highest accepted placement is bar **691**; the 2026-08-16 snapshot reported 689. On
the chain: 602 hits, 10 rejected, all track 0, and the highest bar is 149.

Rejections are counted, never silently dropped. A placement before bar 1 (position
< 34560, i.e. pre-roll) would be rejected as `before_origin`, not decoded; the library has
none.

**Open — counted, not explained, not guessed at.**

- **102 `gRuA` records fail to decode** (0.57%). The scan does not say which check failed:
  the size law, the name length, UTF-8, or a control character.
- **66 placements are unresolved.** Their link names a family that no decoded region
  carries. The aggregate does not record whether they sit in the same saves as the 102
  failed records.
- **20 payloads do not end in the terminator.** 22,480 of 22,500 hold exactly one
  terminator unit, and 22,480 of 22,500 end in it; those are two separate counters. The
  scan says nothing more about the 20 on either count.
- **One save has a decoded track number above its `NumberOfTracks`.**

**What is still open — which copy [measured].** A placement links to a region *family*
(one source file), not to a region record: `Angelic Vocal FX 03` and its `.1`/`.2`/`.3`
copies share one link value. On backup 00 there are 35 families: 20 hold one region object
(16 placed once, 4 not placed) and **15 hold two or more — and those 15 carry 40 of the 56
placements.**

Across the library it is **6,848 of 9,142 placements (75%)**. Another 2,228 sit in a
one-object family and 66 are unresolved. Counted per save, 2,663 of 7,134 families hold
two or more objects.

For those placements, which copy is on the timeline is not resolvable from any field
above; the map prints "one of N copies" and the diff "a '*stem*' region". 12 families hold
more region objects than placements (79 objects for 56 placements); the map reports a
per-family surplus count and never names which copies are off the timeline. An earlier
version of this section put the gap at "12 of 35 families" by counting families whose
record count equals their placement count — which includes 7 multi-copy families whose
copies cannot be told apart. The gap is **40 of 56 placements**.

**How the diff pairs placements.** `--diff` compares placements as (track, position,
family) multisets. Exact matches cancel; a family with exactly one placement gone and
exactly one new is reported as a move; everything else is reported as placements added
and removed, never paired by guess. Two copies of *one* family swapping places cancel out
and are not reported. The comparison relies on family indexes being stable across saves,
which holds on the chain (0 of 96 change).

**What is demonstrated is now a region-level diff, and still not a full semantic diff.**
Region add/remove/rename/resize and placement moves are readable, and
`experiments/logic_region_map.py` prints them. What is **not** demonstrated is
parameter-level diff: what a fader moved to, what a plugin knob became. Those payloads
are unmapped. Do not describe Logic support as "semantic diff" until that lands — and
note that `wit-logic`, the shipping crate, does not yet read any of the fields above;
the map exists in `experiments/` only.

### Start from LogicProFormatWriter, not from scratch — [cited]

[`jonkubis/LogicProFormatWriter`](https://github.com/jonkubis/LogicProFormatWriter)
(Python, **MIT**, ~4,100 LOC plus a ~1,000-line `PROJECTDATA_FORMAT.md`) is by a wide
margin the most valuable Logic asset in the ecosystem. It does not merely read
`ProjectData` — **it writes valid `.logicx` from scratch**, which is strictly harder and
proves the container is tractable.

What its spec already documents:

- Root frame: 24-byte header, little-endian `u32` length at `+0x10`
- **A universal 36-byte record header with the size field at `+0x1c`** — described as "the
  size field everyone missed", and the key to walking records reliably
- The reversed-FourCC tag table: `gnoS`, `karT`, `qeSM`, `qSvE`, `gRuA`, `lFuA`, `OgnS`, …
- Tempo as `uint32` BPM × 10000; meter and marker maps; MIDI note encoding; sample rate
- **Track names — solved.** `u16` length + ASCII at the paired `qeSM` payload `+0x34`.
  This is the item other Logic projects list as their #1 unsolved problem.
- **No absolute-offset pointers**, so records can be grown or inserted as long as the root
  length is fixed up. This is precisely what makes writing feasible.

It also ships real Logic-made fixtures, which are arguably worth more than the code: at
the pinned commit `1f77c5c` (measured 2026-09-29), **87 `.logicx` bundles holding 113
`ProjectData` files**, every bundle recording Logic Pro 11.2.2 as last saved from. An
earlier version of this section said "30 fixtures"; that count is not reproducible at the
pinned commit. Wit walks all 113 in `just logic-fixtures-gate`.

⚠️ **Two time origins at 960 PPQ**, and confusing them silently corrupts arrangement
placement: region placements use origin **34560** (9 bars); tempo, marker *and note*
events use **38400**.

### Two cautions

**Save churn is non-deterministic — [verified].** Saves 04, 05 and 06 have identical file
size *and* identical record census (no semantic change), yet differ in 97 and 140 bytes,
spread across the file. Causes identified: a plugin-state UUID regenerated on every save,
and ~20 float32 values rewritten at a regular stride. **Consequence: hashing
`ProjectData` cannot detect "nothing changed". Dirty-detection must be semantic.**

**Alternatives are copy-on-branch.** `Alternatives/NNN` is Logic's native branching, named
in `Resources/ProjectInformation.plist`. All alternatives share one `Media/` pool. There
is no common-ancestor tracking, no merge, and no cross-alternative diff — Logic gives you
branches with no way to compare or combine them. (Note: "Track Alternatives" is a
separate per-track take-lane feature.)

**Interchange — [verified]** by scanning `Logic.framework`: AAF and OMF support present,
Final Cut XML present, **no dawproject**.

---

## GarageBand — `.band` 🟡

**[verified]** GarageBand writes **the same container format as Logic Pro**:

```
Logic      ProjectData:  23 47 c0 ab d009 ... gnoS
GarageBand ProjectData:  23 47 c0 ab cb09 ... gnoS
```

Same magic, same FourCC, same package layout (`Alternatives/000/`, `MetaData.plist`,
`Media/Audio Files`). Only a version word differs.

**One parser covers Logic and GarageBand.** GarageBand ships free on every Mac and is
Apple's on-ramp to Logic, so this single integration reaches the entire Apple base.

---

## FL Studio — `.flp` 🟡

**[verified]** `FLhd` header chunk + `FLdt` data chunk, then a stream of typed events:

| Event ID | Payload |
|---|---|
| 0–63 | 1 byte |
| 64–127 | 2 bytes |
| 128–191 | 4 bytes |
| 192–255 | varint length, then that many bytes |

Measured on the real FL 10-era file: **1,491 events, 82 distinct IDs, **96.7%** of the file is
variable-length payload** — overwhelmingly opaque plugin/channel state.

**Plugin state comes in three flavours with very different diff properties — [verified]:**

1. **VST/AU via "Fruity Wrapper"** — nested TLV carrying vendor, name, an **absolute
   plugin DLL path**, and an opaque VST chunk (9,133 B for reFX Nexus).
2. **FL "advanced" natives** (Sytrus, FLEX) — a raw **zlib stream** (2,190 B inflating to
   65,188 B). One knob change re-deflates everything, so byte deltas there are
   meaningless; you must inflate, diff, re-deflate.
3. **Simple FL natives** — fixed-size plain structs (Parametric EQ 2 = 305 B), which
   *are* byte-diffable and field-decodable.

**Sample paths use `%FLStudioData%` tokens** — a variable-based path scheme that is
genuinely more portable than Ableton's absolute paths.

**Save locality — [cited from a second measurement on real FL autosaves]:** two real
autosaves of the same project differed in only **17 of 53,448 bytes (0.032%)**, touching
3 of 1,611 events. FL's writer is deterministic and positionally stable; it does not
re-serialise unrelated regions.

**⚠️ FL Studio v25 regression — [cited]:** v25-era files appear to add an additive,
file-offset-dependent obfuscation keystream over scalar (fixed-size) events. When file
size changes, ~96% of scalar events churn by exactly the size delta mod 256. Files from
v10 through v24 parse cleanly; v25 needs either the keystream solved or diffing
restricted to variable-length events. **Wit should target ≤ v24 first and treat v25 as an
open problem.**

### New this session — `wit-flp` (Rust reader), [verified] on real FL 10/20/25 files

Measured while porting `experiments/flp_parse.py` to `crates/wit-flp`, against a real FL
10.0.0 file (`Aston Martin Music Remake.flp`, this section's named fixture) plus a second
real FL 20.8.3 project and a real FL 25.2.5 project's own `Backup/` autosave chain (5
files total, not named here — personal material, per AGENTS.md):

- **v25's framing quirk, not just its keystream, breaks the naive event walk — [verified].**
  Event id 172 (`0xAC`) carries a **3-byte** payload on v25 files, not the general dword
  rule's 4 — [issue #7](https://github.com/sep-lab/Wit/issues/7) names this exactly.
  Walking all 5 real v25 files under the standard 4-byte assumption either fails outright
  (2 of 5, a declared payload running past EOF) or reaches a clean EOF only by
  coincidence, with mutually inconsistent event counts (1540–1652) for consecutive saves
  of one project. Treating id 172 as 3 bytes gives a clean EOF on all 5, with
  self-consistent counts (1616–1710). Id 172 does not appear at all (count zero) in
  either pre-v25 file checked, so this fix is inert for ≤ v24.
- **Tempo is event id 156** (dword, `round(BPM * 1000)`) — not named in
  `flp_parse.py`'s `EVENT_NAMES` table. Identified by scanning every dword-class id that
  occurs exactly once per file for a value that divides evenly by 1000; on the real FL
  20.8.3 file it decoded to exactly `130000` → `130.0` BPM. **Unreadable on v25**: the
  same field on the real v25 files decoded to values like `252566.982` BPM — obvious
  garbage, consistent with the keystream, not a framing bug. `wit-flp` returns a typed
  "partial: v25 scalars unreadable" marker for tempo on v25+ rather than surface it.
- **Mixer insert names are event id 204** (`InsertName`) — inside `flp_parse.py`'s own
  `TEXT_EVENTS` range but never named there as a semantic field. Verified decoding real
  insert names (`"Dream bell"`, `"REC"`) across the FL 10 and FL 20 fixtures.
- **A literal `"Arrangement"` text decoded from event id 241** on the FL 20 and FL 25
  fixtures (absent on the pre-arrangement-feature FL 10 one) — a plausible but
  **unverified beyond this session** candidate for a playlist/arrangement name, kept
  separate from the better-evidenced ids above in `wit-flp`'s extraction.
- **Text events (channel/plugin/insert names) decode cleanly on v25 files** — only the
  scalar keystream is a problem; `wit-flp` restricts extraction to variable-length events
  on v25+ rather than refusing the whole file.
- FL's own `Backup/` autosave folder is typically **shared across an entire "Projects"
  root**, not per-project, and its filenames (`"<name> (autosaved at <time>).flp"`) carry
  only a time of day, no date — measured on 4 real autosaves spanning 2 calendar days,
  where filename order and modification-time order disagreed. `wit-index`'s FL discovery
  orders a project's autosave chain by file modification time, never by filename.

---

## Pro Tools — `.ptx` 🔴

**[cited, with byte-exact roundtrip reported]** The "encryption" is a position-keyed XOR
whose key is derivable from the file itself — obfuscation, not cryptography. Because page
0's key byte is zero, **the first 4,096 bytes of every `.ptx` are plaintext**.

Block layout is a nested TLV tree (`0x5A` marker, `uint16` type, `uint32` size). Of 901
blocks in a small session, `ptformat`'s table names only **35%**. The largest single block
is the I/O channel list at 30.6% of the file — a small Pro Tools session is mostly mixer
boilerplate, not music.

**Deltas work fine on the raw obfuscated file** (the XOR is stateless and position-keyed,
so a 55-byte plaintext edit produces exactly 55 changed ciphertext bytes). Storage is
therefore easy; *semantic* diff is not.

> **Legal note.** Circumventing an access-control measure can carry DMCA §1201 exposure
> in the US regardless of the measure's weakness, and Avid's EULA restricts reverse
> engineering. Wit will **not** ship `.ptx` deobfuscation. Storage-level support (which
> needs no decryption) is fine. Get counsel before revisiting.

---

## Cubase — `.cpr` 🔴

**[cited]** Not encrypted or compressed. MFC `CArchive`-style class-tagged object
streaming with class-name interning; class names are readable in the binary
(`MAudioTrack`, `MMidiNote`, `PMixerChannel`, …). A sample file was **89.3% zero bytes**.

**Blocker:** the only structural parser targets Cubase SX2 (2004). No parser reads Cubase
13/14/15, and edited files are reportedly rejected by Cubase, implying checksums or
cross-references. Not viable now.

---

## Studio One — `.song` 🟡

**[cited, not verified by us]** A ZIP container wrapping XML plus binary plugin blobs —
the best-case container of the closed formats.

**Caveat:** ZIP entry order, timestamps and deflate non-determinism mean the outer
`.song` bytes can churn heavily for a trivial inner change. Any VCS must extract entries
before diffing. This generalises: **normalise the container before hashing** — the same
reason Wit gunzips `.als` rather than hashing the gzip stream.

---

## Reaper — `.rpp` ⚪

**[cited]** Plain text, human-readable, already git-friendly. Not a target precisely
because Reaper users can already use git. It is useful as a **reference implementation**
of what a diffable DAW format looks like.

---

## Interchange formats

**dawproject** (github.com/bitwig/dawproject, MIT) — the only real interchange standard
that models *musical* time, unlike AAF. A ZIP with `project.xml` + `metadata.xml`. Models
clips, fades, notes with expressions, automation, and embedded plugin state; plugin state
is a `FileReference` (`<State path="plugins/<uuid>.<ext>"/>`), not inline.

**Adoption is the problem — [verified]:** supported by Bitwig, Studio One, Cubase and
n-Track. **Not** by Logic, Ableton or Pro Tools — confirmed by scanning all three
binaries locally. So dawproject cannot be Wit's lingua franca for the DAWs Wit targets,
though it is the right model to *learn from*.

**AAF/OMF** — post-production interchange. Logic supports both. Carries audio and edits,
loses plugin state and DAW-specific behaviour.

**DawVert** (SatyrDiamond/DawVert) — converts among ~45 input formats and writes 11.
**No Logic, no Pro Tools.** Its internal representation (`cvpj`, ~7,500 LOC across 27
modules) is the most relevant prior art for Wit's object model: an ID-indexed graph rather
than a nested tree, time as `(ppq, is_float)` with one global rescale, and a
read → capability-negotiate → write pipeline.

> ⚠️ **Licensing:** DawVert is **GPL-3.0**. Wit is Apache-2.0. We may study the design;
> we may **not** copy the code. Keep that boundary explicit in any PR that cites it.

**logic2ableton** — worth a specific warning, since it is often cited as proof that
Logic→Ableton conversion is solved. **It is not**, and the details are instructive:

- It is a `MetaData.plist` reader plus audio-filename regexes and two opportunistic
  byte-scrapes. It never walks Logic's record framing. Clip positions come from WAV `bext`
  timestamps, not from `ProjectData`. Mixer state cannot be read at all — the user must
  hand-write a `mixer_overrides.json`.
- Its MIDI extraction recovered **zero notes** across the Logic 11.2.2 fixtures it was
  run on (the "30 fixtures" of an earlier snapshot; not re-run against the 113-file corpus
  at the pinned commit). Its
  15-byte signature expects `00` where real events carry note-off-velocity `0x40`.
- **The generalisable lesson:** even with that byte corrected, fixed-signature scanning
  *structurally* cannot find the last note of a region, because a terminal flag bit inside
  the signature window flips on the final event. **Wit must walk record framing, never
  scan for magic signatures.**
- Its plugin scan looks for `<?xml` in files that now embed binary plists (`bplist00`).
- Its tests against real projects all auto-skip, and its synthetic test generator emits the
  same wrong bytes the parser looks for — which is exactly why the bug survived. (This is
  why `docs/TESTING.md` requires **loud** skips.)

The one sound idea worth borrowing: to *write* a `.als`, clone a real template set and
reassign element IDs rather than synthesising Live's schema from nothing.

---

## Open format questions

1. Are Ableton IDs stable across **Live versions**, and across duplicate/copy-paste?
2. Full `ProjectData` payload schemas per remaining chunk tag. The container, the
   region object (`AuRg`) and the audio placement groups in `EvSq` are mapped; `Trak`,
   `AuCU`, MIDI placements, and every other `EvSq` unit are not — one save carries 12
   distinct byte-`+7` values (21 across the library), of which a placement group uses
   three (`00`/`89`/`bc`).
   Within the mapped set, which *copy* of a multi-region family a placement refers to is
   still unresolved (6,848 of 9,142 placements in the library, 2026-09-29), placement
   `+0x10` is unexplained beyond "one value per track", and the library's 102 undecoded
   `gRuA` records, 66 unresolved placements, 20 `qSvE` payloads that do not end in the
   terminator, and one save with a track number above its `NumberOfTracks` are counted
   but not explained.
3. The FL Studio v25 scalar keystream.
4. Does a Wit-written `.als` open cleanly in Live? **Untested — release gate.**
5. Studio One and modern Cubase need first-hand verification; ours is second-hand.
