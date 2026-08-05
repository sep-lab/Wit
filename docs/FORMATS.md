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
| Logic Pro | `.logicx` package | chunked binary | object inventory only, so far | 🟡 |
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

**What is demonstrated is an *object-inventory* diff, not a full semantic diff.** From
the container structure alone you can say *"6 regions added, 4 tracks added, 189 plugin
instances added"* and extract region names and audio paths. That is genuinely useful and
it is real. What is **not** demonstrated — and is gated on per-tag payload schemas — is
parameter-level diff: what a region's position actually became, what a fader moved to.
Do not describe Logic support as "semantic diff" until that lands.

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

It also ships **30 real Logic 11.2.2 fixtures**, which are arguably worth more than the
code.

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
- Its MIDI extraction recovers **zero notes** across all 30 real Logic 11.2.2 fixtures. Its
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
2. Full `ProjectData` payload schemas per chunk tag (only the container is mapped).
3. The FL Studio v25 scalar keystream.
4. Does a Wit-written `.als` open cleanly in Live? **Untested — release gate.**
5. Studio One and modern Cubase need first-hand verification; ours is second-hand.
