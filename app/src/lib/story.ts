/**
 * GENERATED FILE — do not hand-edit.
 * Source: crates/wit-story/schema/library.schema.json (itself generated from
 * crates/wit-story/src/types.rs and pinned by crates/wit-story/tests/contract.rs).
 * Regenerate with: npm run gen:types
 * CI fails the build if this file is stale (app-build.yml runs
 * `npm run gen:types` then `git diff --exit-code`).
 */

export type Artwork =
  | {
      kind: "window_image";
      [k: string]: unknown;
    }
  | {
      kind: "generated";
      seed: number;
      [k: string]: unknown;
    };
export type Daw = ("logic" | "garage_band" | "ableton" | "fl_studio") | "other";
/**
 * Seconds since the Unix epoch, UTC.
 */
export type Timestamp = number;
/**
 * One song on the Shelf: a project, with every lineage it has.
 */
export type SongId = string;
/**
 * One line of saves: a Logic alternative, an Ableton lineage, an FL
 * project with its autosaves.
 */
export type StoryId = string;
/**
 * The honesty tiers (wit-planning/PLAN.md "Honesty tiers").
 */
export type Tier = "structure" | "semantic" | "ears" | "history";
export type FamilyEvidence =
  | {
      kind: "same_project";
      [k: string]: unknown;
    }
  | {
      files: number;
      kind: "shared_audio";
      [k: string]: unknown;
    }
  | {
      kind: "shared_regions";
      regions: number;
      [k: string]: unknown;
    }
  | {
      kind: "similar_name";
      [k: string]: unknown;
    };
export type Relation =
  | {
      kind: "original";
      [k: string]: unknown;
    }
  | {
      kind: "alternative";
      [k: string]: unknown;
    }
  | {
      kind: "save_as_copy";
      [k: string]: unknown;
    }
  | {
      from: MomentId;
      kind: "restored";
      [k: string]: unknown;
    };
/**
 * One kept save inside a Story.
 */
export type MomentId = string;
export type Confidence = "exact" | "approximate" | "inferred";
export type Icon =
  | "add"
  | "remove"
  | "move"
  | "trim"
  | "duplicate"
  | "rename"
  | "record"
  | "tempo"
  | "key"
  | "meter"
  | "mute"
  | "mix"
  | "plugin"
  | "marker"
  | "automation"
  | "midi"
  | "tracks"
  | "audio_file";
export type Place =
  | {
      kind: "whole_song";
      [k: string]: unknown;
    }
  | {
      /**
       * "about bar N": Wit assumed a tempo or meter to get here.
       */
      approximate: boolean;
      end?: number | null;
      kind: "bars";
      /**
       * The marker or locator the start falls in ("Chorus 2").
       */
      section?: string | null;
      start: number;
      [k: string]: unknown;
    };
export type SpanKind = ("region" | "file" | "plugin" | "marker") | "plain" | "track" | "name" | "value";
export type Verdict = "first" | "changed" | "nothing_visible" | "identical" | "unreadable";
export type DurationSource = "save_times" | "daw_running";
export type MomentSource =
  | {
      alternative: string;
      kind: "logic_backup";
      slot: string;
      [k: string]: unknown;
    }
  | {
      alternative: string;
      kind: "logic_current";
      [k: string]: unknown;
    }
  | {
      kind: "ableton_autosave";
      [k: string]: unknown;
    }
  | {
      kind: "ableton_save";
      [k: string]: unknown;
    }
  | {
      kind: "fl_autosave";
      [k: string]: unknown;
    }
  | {
      kind: "fl_save";
      [k: string]: unknown;
    }
  | {
      kind: "kept_by_wit";
      [k: string]: unknown;
    }
  | {
      from: MomentId;
      kind: "restored";
      [k: string]: unknown;
    };
export type CounterKind = "compare_opened" | "share_created" | "restore_made";

/**
 * Everything the app's first screens need, in one document.
 */
export interface Library {
  schema_version: number;
  /**
   * One card per song, most recently worked first.
   */
  shelf: ShelfCard[];
  stories: Story[];
  trust: TrustPanel;
  [k: string]: unknown;
}
export interface ShelfCard {
  artwork: Artwork;
  /**
   * Other members of this song's family (copies, alternatives,
   * restores) — the "+2 copies" badge. 0 = no badge.
   */
  copies: number;
  /**
   * The badge text: "+1 alternative", "+2 copies". `None` = no badge.
   */
  copies_label?: string | null;
  daw: Daw;
  /**
   * The tray's one line for this song. Worded "no changes Wit can see",
   * never "nothing new".
   */
  digest: string;
  last_worked?: Timestamp | null;
  /**
   * "Thu 23:05", or "21 Sep" when older than a week.
   */
  last_worked_label?: string | null;
  moments_kept: number;
  song_id: SongId;
  /**
   * The Story for each lineage, newest-worked first. Always non-empty.
   */
  story_ids: StoryId[];
  title: string;
  [k: string]: unknown;
}
/**
 * One line of saves, read as a story.
 */
export interface Story {
  /**
   * What Wit can and can't see for this DAW, in plain words. Shown under
   * every change card.
   */
  capability: CapabilityNote[];
  family?: Family | null;
  header: SongHeader;
  id: StoryId;
  /**
   * The first compare the Song view shows: oldest kept moment against
   * the newest. Never an adjacent pair (a quarter of real adjacent saves
   * show nothing — design-review-1). `None` with fewer than two moments.
   */
  overview?: Comparison | null;
  send_ready?: SendReady | null;
  /**
   * Oldest first. Saves more than 45 minutes apart start a new session.
   */
  sessions: Session[];
  song_id: SongId;
  /**
   * Heat-strip rows. Empty when Wit can't tell the DAW's tracks apart yet
   * (Logic today; a capability note says so). Ableton rows are in
   * track-id order until display order is read.
   */
  tracks: TrackLane[];
  [k: string]: unknown;
}
export interface CapabilityNote {
  text: string;
  tier?: Tier | null;
  [k: string]: unknown;
}
/**
 * A song's copies and lines, drawn as branches.
 */
export interface Family {
  /**
   * Why Wit thinks these belong together.
   */
  evidence: FamilyEvidence[];
  members: FamilyMember[];
  [k: string]: unknown;
}
export interface FamilyMember {
  /**
   * This member is the Story being viewed.
   */
  is_current: boolean;
  last_worked?: Timestamp | null;
  /**
   * Index into [`Family::members`] of the member this one came from.
   */
  parent?: number | null;
  relation: Relation;
  song_id: SongId;
  story_id?: StoryId | null;
  title: string;
  [k: string]: unknown;
}
export interface SongHeader {
  daw: Daw;
  /**
   * "Logic", "GarageBand", "Live", "FL Studio".
   */
  daw_label: string;
  /**
   * The family pill: "2 alternatives of this song", "3 copies of this
   * song". `None` when the song has no family.
   */
  family_label?: string | null;
  kept: KeptSummary;
  /**
   * "C minor". Only when the project states it.
   */
  key?: string | null;
  last_worked?: Timestamp | null;
  /**
   * Which line of the song this is, when it has more than one
   * ("Alternative 2"). `None` for a single-lineage song.
   */
  lineage?: string | null;
  /**
   * "Logic · 98 BPM · C minor · last worked Thu 23:05".
   */
  subtitle: string;
  tempo_bpm?: number | null;
  /**
   * "4/4". Only when the project states it.
   */
  time_signature?: string | null;
  title: string;
  [k: string]: unknown;
}
export interface KeptSummary {
  /**
   * How many saves the DAW itself keeps (Logic keeps 10 backups, Live 10
   * autosaves per set). `None` when unknown.
   */
  daw_keeps?: number | null;
  /**
   * How many of `moments` exist only because Wit kept a copy (the DAW
   * has since recycled them).
   */
  kept_by_wit: number;
  /**
   * Credits whoever actually kept the saves: "41 moments kept · Logic
   * keeps 10" once Wit has kept some, "10 moments on disk · Logic keeps
   * 10" while every moment is still the DAW's own.
   */
  label: string;
  moments: number;
  [k: string]: unknown;
}
/**
 * A compare between any two moments of one Story.
 */
export interface Comparison {
  from: MomentId;
  /**
   * "Since Mon 21 Sep" / "What changed at Thu 23:05".
   */
  heading: string;
  listen: ListenRef[];
  note?: string | null;
  sentences: Sentence[];
  /**
   * "compared with the save before" / "oldest kept moment to newest".
   */
  subheading: string;
  /**
   * As [`Moment::summary`].
   */
  summary?: string | null;
  to: MomentId;
  track_heat: TrackHeat[];
  verdict: Verdict;
  [k: string]: unknown;
}
/**
 * Source audio to play for "Listen to the new parts". The app resolves
 * `file_name` inside the song's own folder; a Story never holds a path.
 */
export interface ListenRef {
  duration_secs?: number | null;
  file_name: string;
  region?: string | null;
  start_secs?: number | null;
  [k: string]: unknown;
}
/**
 * One plain sentence about one change.
 */
export interface Sentence {
  confidence: Confidence;
  icon: Icon;
  /**
   * Index into [`Story::tracks`] — the heat-strip row — when Wit knows
   * exactly which row. Two tracks with the same name get `None`.
   */
  lane?: number | null;
  place?: Place | null;
  /**
   * "bars 17–32 · Verse 2", "whole song", "about bar 9".
   */
  place_label?: string | null;
  /**
   * The same text, split so a UI can style names without parsing.
   */
  spans: Span[];
  /**
   * The whole sentence as plain text (copy-as-text, the CLI, screen
   * readers). Always equal to the concatenation of `spans`.
   */
  text: string;
  tier: Tier;
  /**
   * The track this sentence is about, when known (its display name).
   */
  track?: string | null;
  [k: string]: unknown;
}
export interface Span {
  kind: SpanKind;
  text: string;
  [k: string]: unknown;
}
export interface TrackHeat {
  /**
   * 1 = a little, 2 = more, 3 = a lot (or the track was added/removed).
   */
  level: number;
  /**
   * Index into [`Story::tracks`].
   */
  track: number;
  [k: string]: unknown;
}
/**
 * "Will this open on my friend's machine?"
 */
export interface SendReady {
  /**
   * "Ready to send" / "2 things to check before you send".
   */
  headline: string;
  /**
   * Audio the project uses from outside its own folder. Names only.
   */
  outside_files: OutsideFile[];
  plugins: PluginUse[];
  ready: boolean;
  [k: string]: unknown;
}
export interface OutsideFile {
  file_name: string;
  /**
   * "outside the project folder", "missing on this computer".
   */
  reason: string;
  [k: string]: unknown;
}
export interface PluginUse {
  name: string;
  /**
   * Ships with the DAW. `None` when Wit can't tell.
   */
  stock?: boolean | null;
  tracks: string[];
  [k: string]: unknown;
}
export interface Session {
  duration?: SessionDuration | null;
  ended: Timestamp;
  id: string;
  /**
   * "Mon · night · about 1h 40m".
   */
  label: string;
  /**
   * Oldest first.
   */
  moments: Moment[];
  started: Timestamp;
  [k: string]: unknown;
}
export interface SessionDuration {
  secs: number;
  source: DurationSource;
  [k: string]: unknown;
}
/**
 * One kept save.
 */
export interface Moment {
  actions: Actions;
  at: Timestamp;
  /**
   * The moment this one was compared with — usually the one before, but
   * after an unreadable save it is the last one Wit could read.
   */
  compared_with?: MomentId | null;
  /**
   * The change card's title: "What changed at Thu 23:05", "Routine
   * save", "The oldest moment", "Wit couldn't read this save".
   */
  heading: string;
  /**
   * One kept save inside a Story.
   */
  id: string;
  /**
   * "Thu 23:05".
   */
  label: string;
  /**
   * Source audio behind the new parts, for "Listen to the new parts".
   */
  listen: ListenRef[];
  /**
   * The line shown when there are no sentences ("No change Wit can see —
   * maybe a knob or fader move, or you just hit save."). It says Wit kept
   * a copy only for a [`MomentSource::KeptByWit`] moment.
   */
  note?: string | null;
  /**
   * Compared with [`Moment::compared_with`].
   */
  sentences: Sentence[];
  source: MomentSource;
  /**
   * "Logic backup 04", "current save", "kept by Wit".
   */
  source_label: string;
  /**
   * "compared with the save at Thu 22:41".
   */
  subheading: string;
  /**
   * A one-line digest when there are many sentences ("14 changes Wit can
   * see: 9 regions added, 3 new audio files, 2 probable renames"), so the card and timeline stay
   * readable on a busy save. `None` for three sentences or fewer.
   */
  summary?: string | null;
  /**
   * Sparse heat-strip column for this moment.
   */
  track_heat: TrackHeat[];
  verdict: Verdict;
  /**
   * How much changed, for the timeline tick's height: the number of
   * sentences. 0 for a routine save.
   */
  weight: number;
  [k: string]: unknown;
}
/**
 * Which of the change card's actions work for this moment today.
 */
export interface Actions {
  /**
   * "Listen to the new parts": there is source audio to play.
   */
  listen: boolean;
  /**
   * "Open this moment as a copy": restore is enabled for this DAW
   * (ADR-0007: only after the restored copy is shown to open in it).
   */
  open_as_copy: boolean;
  /**
   * "Send to a friend": the share page can be made.
   */
  send: boolean;
  [k: string]: unknown;
}
export interface TrackLane {
  /**
   * "#RRGGBB" in the DAW's own track colour, when Wit can read it.
   */
  color?: string | null;
  /**
   * Stable within a Story: the DAW's track id where it has one,
   * otherwise the name.
   */
  key: string;
  name: string;
  [k: string]: unknown;
}
export interface TrustPanel {
  counters: PilotCounters;
  /**
   * "Wit never changes your projects.", "Nothing leaves this computer."
   */
  statements: string[];
  /**
   * Folder names as the owner sees them ("~/Music/Logic"). Shown only on
   * the owner's own screen; never copied into a share page.
   */
  watched_folders: string[];
  [k: string]: unknown;
}
/**
 * Local-only counters (no telemetry, no network). The pilot report copies
 * them only when the owner presses *Copy pilot report*.
 */
export interface PilotCounters {
  compares_opened: number;
  /**
   * Every counted event with its time, so the success metric ("two
   * compares in any 7-day window") can be checked, not just totalled.
   */
  events: CounterEvent[];
  restores_made: number;
  shares_created: number;
  since?: Timestamp | null;
  [k: string]: unknown;
}
export interface CounterEvent {
  at: Timestamp;
  kind: CounterKind;
  [k: string]: unknown;
}
