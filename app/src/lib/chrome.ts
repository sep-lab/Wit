/**
 * Every UI-authored string in this app, gathered in one place, so a
 * reviewer (or `lint:vocab`) can audit them all at a glance. This lane's
 * brief, "Laws": "The UI may only author chrome: button labels from the
 * mockup ..., section titles, and empty/loading states." Nothing else in
 * `src/` should read as a claim about a musician's song — that always
 * comes from the Story JSON as data.
 */

/** Verbatim from the approved mockup (design-song-view-mockup.html). */
export const ACTION_LABELS = {
  listen: "▶ Listen to the new parts",
  open_as_copy: "Open this moment as a copy",
  send: "Send to a friend",
} as const;

export const NAV_LABELS = {
  shelf: "Shelf",
  song: "Song",
  family: "Family",
  send_ready: "Send-ready check",
  trust: "Trust panel",
} as const;

export const EMPTY_STATES = {
  loading: "Loading your library…",
  noLibrary:
    "Wit hasn't found any songs yet. Point it at a folder to see what's changed.",
  noSongSelected: "Pick a song from the Shelf to see what changed.",
  noHeatStrip: null, // intentionally absent: the capability note explains why.
  sendReadyUnknown: "Wit hasn't checked whether this song is ready to send yet.",
  dropZoneDisabled: "Comparing a bounce isn't available yet.",
  compareUnavailable: "Comparing two moments isn't available yet.",
  noFamily: "Wit hasn't found any other copies of this song.",
  reportCopied: "Copied.",
  reportCopyFailed: "Couldn't copy that — you can select the text above instead.",
} as const;

export const SECTION_TITLES = {
  shelf: "Your songs",
  family: "Family",
  sendReady: "Send-ready check",
  bounceDropZone: "Compare a bounce",
  trustWatched: "Watched folders",
  trustStatements: "What Wit promises",
  trustCounters: "This pilot, so far",
  copyReport: "Copy pilot report",
  copyReportPreviewTitle: "This is exactly what would be copied:",
  confirmCopy: "Copy",
} as const;

export const COUNTER_LABELS = {
  compares_opened: "Compares opened",
  shares_created: "Shares created",
  restores_made: "Restores made",
} as const;

export const RELATION_LABELS = {
  original: "Original",
  alternative: "Alternative",
  save_as_copy: "Copy",
  restored: "Restored copy",
} as const;

export const FAMILY_CURRENT_TAG = "Viewing";
