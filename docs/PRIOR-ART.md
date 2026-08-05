# Prior art: what has been tried, and why it died

Version control for music has been attempted, seriously and repeatedly, since at least
2001. Almost all of it is dead. Anyone considering contributing to Wit deserves to know
that up front, and deserves a specific answer to *"why would this go differently?"*

This document is the graveyard, the causes of death, and an honest account of what Wit
does differently — and where the same forces still apply to us.

Sources are secondary (company announcements, press, practitioner forums) unless noted.
Treat these as **cited**, not measured.

---

## The single most important data point

**Splice Studio (2013–2023).** TechCrunch called it "A GitHub For Musicians" at launch.
A desktop agent watched your project folder, auto-uploaded a snapshot on every DAW save,
and kept a version timeline with per-revision comments and a browser player showing track
names, waveforms and plugin lists.

It was **free forever with unlimited storage**, backed by **$150M+** of venture capital,
and had the best distribution in the category. Splice killed it in 2023 and kept the
sample subscription business.

So the honest question is not "can this be built?" — it was built, well-funded, and given
away. The question is what it was actually missing.

**Two things stand out, and they define Wit's thesis:**

1. **It was a folder-watcher and object store, not a diff/merge system.** It could tell
   you *that* a version existed. It could not tell you *what changed*, and it could not
   merge two people's work. You still opened both and compared by ear.
2. **It explicitly refused the plugin problem** — the advice was to bounce to audio.

Splice proved that *storage plus a timeline is not enough to be a product.* That is a
genuinely useful negative result, and it is not the same thing as proving the category is
impossible.

---

## The graveyard

| Product | Years | What it was | Cause of death |
|---|---|---|---|
| **Splice Studio** | 2013–2023 | Auto-versioning of DAW projects, free, unlimited | Never monetised; killed while samples earned $100M+ |
| **Ohm Studio** | ~2010–2020 | Real-time multi-user pro DAW. **330K users, 640K projects** | "Never met a demand strong enough to be commercially sustainable" |
| **Steinberg VST Transit** | 2016–2023 | Cloud collaboration for Cubase, then any DAW via plugin | Discontinued: "no longer economically viable" — *with a captive install base* |
| **Blend.io** | 2013–2024 | Project sharing for Ableton/Maschine | Acquired by ROLI 2015, orphaned, died on a nine-year timer |
| **Indaba Music** | 2005–2018 | 1M+ creators, remix contests | Acquired by Splice; platform wound down |
| **Endlesss** | 2019–2024 | Real-time collaborative jamming | Servers closed; founder illness + capital |
| **Bandhub** | 2012–2019 | Collaborative multi-track video covers | "Could not raise enough revenues to keep the costs covered" |
| **eJamming** | 2001–~2019 | Networked real-time performance | Wound down |
| **Aux (aux.app)** | ~2021–2026 | WIP backup and feedback for bands/labels | Shut down; files deleted March 2026; pivoted to AI |
| **Soundtrap** | 2017–2023 *(Spotify era)* | Browser DAW with collaboration | Sold back to founder after ~6 years |
| **SoundBetter** | 2019–2021 *(Spotify era)* | Producer/engineer marketplace | Sold back to founders after 2 years |
| **Wwise SVN plug-in** | –2023 | Source control for game audio projects | Deprecated; only Perforce survives |
| **Avid Cloud Collaboration** | 2016– | In-DAW track-level sharing | **Not dead** — but a crippled, unmonetised retention feature |
| **git / git-LFS for DAWs** | ~2011– | `ableton-git`, `logics`, `gitable`, … | Never left hobbyist repos in 15 years |

Two DAW vendors with captive customer bases built this natively. One killed it as
uneconomic; the other keeps it alive as a retention feature it does not charge for.

---

## Causes of death, ranked

**1. Monetisation — versioning is treated as a free feature, never a product.**
This is the dominant killer. Splice, Steinberg and Avid all gave it away. Nobody has
demonstrated that musicians will pay for version control *as such*.

**2. There is no meaningful merge, so "collaboration" means "don't both edit at once".**
Even Splice Studio required manual coordination to avoid simultaneous edits. Version
control prevented overwrite; it did not enable concurrency. Practitioners on Hacker News
put it plainly: *"file formats in music production don't generally allow such things."*

**3. The plugin wall — and every product surrendered to it.**
In 20 years, every single product "solved" plugins by telling users to render to audio.
Avid went furthest, building Track Freeze and Track Commit into Pro Tools 12.5
specifically so collaborators without your plugins could open the session.

**4. DAW fragmentation is a permanent tax.**
A 2025 Production Expert survey (2,500+ respondents, pro-skewed) puts Pro Tools at 37.2%,
Logic 12.6%, Studio One 8.4%, Reaper 7.4%, Ableton 5.9% — and in post-production Pro
Tools reaches 63.8%. The professional and bedroom halves of the market barely share tools.

**5. Strategic orphaning.** Blend → ROLI → administration. Indaba → Splice → wound down.
Spotify bought both a collaboration marketplace and a collaborative DAW, and sold both
back within 2–6 years.

---

## What is actually alive, and why

The survivors are informative because of what they avoid.

- **Real-time remote monitoring** (Audiomovers Listento — acquired by Abbey Road in 2021,
  Source-Connect, Sessionwire) is unambiguously healthy. It works precisely because **it
  never touches project files or versioning**.
- **"Frame.io for audio"** (Highnote, Disco, Byta, Filepass, Samply, …) is alive, funded,
  and commoditised at $6–24/month with a dozen competitors. Note carefully: **every funded
  player versions *bounces*, not projects.** They walked away from the hard problem.
- **Browser DAWs** (BandLab: 100M+ users, ~$48M revenue, $425M valuation; Soundation;
  Audiotool) solved collaboration and versioning cleanly — **because they own the file
  format.** The price is that essentially no professional records are made in them.

Two structural lessons: the workflow is **culturally asynchronous and serial**, which is
why real-time *editing* products die and real-time *listening* products live. And
**stems, not projects, are already the default exchange format** for producer → mixer,
because DAW heterogeneity is assumed rather than exceptional.

**The incumbent Wit has to beat is not git or Perforce. It is `Save As` plus a shared
folder** — a workflow producers have hand-rolled for 15+ years and describe in the exact
vocabulary of version control.

---

## So what is Wit doing differently?

Honestly, and with the failures above in view:

| Failure mode | Wit's answer | Confidence |
|---|---|---|
| Storage + timeline is not a product | **Semantic diff.** Nobody has shipped `volume 0.794 → 0.525` instead of "something changed". This is the wedge, and the prototype does it today. | High — demonstrated |
| No meaningful merge | Track-granular merge on the object model. Edits are localised — though the specific median-3 figure is provisional (issue #11). | Medium — proven on one file pair, not in the wild |
| The plugin wall | We surrender too, but *explicitly and usefully*: content-address plugin state, report "the compressor changed", tell you which plugins you're missing **before** you open the session. | High — no reverse engineering needed |
| DAW fragmentation | Refuse to fight it. Be excellent for one DAW. Do **not** attempt cross-DAW conversion. | High as a scoping decision |
| Versioning is a free feature | **Unresolved.** See below. | — |

There is also a single-player argument the dead products mostly lacked: because history
costs ~11 KB per save, Wit is useful **before any collaborator exists** — a complete,
searchable history of your own work, and a real answer to "what did I change yesterday?"
Every product above needed two people to be worth anything.

---

## The risk we have not answered

**Nobody has shown that musicians will pay for this.** Splice gave it away and stopped.
Steinberg charged and quit. Avid does not monetise it. Venture funding in music tech has
moved decisively to AI generation (Suno raised $250M at $2.45B in Nov 2025); the 2024–2026
rounds are not funding collaboration infrastructure.

Being open source changes the calculus — Wit does not need to return venture capital, and
a tool that is genuinely useful single-player can survive on a much smaller base of
committed users than a VC-backed platform can. But it does not make the risk disappear,
and no contributor should be surprised by it later.

One more honest gap: **no rigorous quantitative survey of how producers actually
collaborate exists.** The "accidental workflow stack" — Dropbox + WeTransfer + email +
Discord + manually named bounces — is universally described and never measured. That is
why [workflow reports](../CONTRIBUTING.md) are one of the most valuable things you can
contribute.

---

## Worth studying rather than dismissing

- **Avid Cloud Collaboration** — the only shipped **track-level** sync in a pro DAW.
- **Perforce in game audio** — the industry that genuinely solved binary collaboration,
  by choosing **exclusive locking over merging**. Wit adopts this for unmergeable assets.
- **DawVert** — converts among ~40 formats; its ID-indexed intermediate representation is
  the best available reference for a project object model.
- **dawproject** (Bitwig + PreSonus) — the only real interchange standard that models
  musical time. Not supported by Ableton, Logic or Pro Tools, so it cannot be our lingua
  franca, but it is the right model to learn from.
- **OpenTimelineIO** — film/TV timeline interchange, and the most mature open data model
  in this shape. Its `RationalTime`/`TimeRange` is verified drift-free at audio sample
  rates and is worth adopting outright; its effects model is a stub and is not.
