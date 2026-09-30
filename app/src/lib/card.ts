import type { Actions, Comparison, Library, Moment, Sentence, ShelfCard, Story, TrackHeat } from "./story";

/** What the change card needs, normalized from either a single [`Moment`]
 * or a [`Comparison`] between two — the mockup's card looks the same
 * either way (heading, subheading, sentences, the three actions). */
export interface CardData {
  heading: string;
  subheading: string;
  sentences: Sentence[];
  summary: string | null;
  note: string | null;
  actions: Actions;
  /** The moment id the three actions above actually act on — "listen" /
   * "open as copy" / "send" are always about one particular save. `null`
   * only when there is truly no moment to act on. */
  momentId: string | null;
  /**
   * Set only when `sentences` is not the real answer — a two-moment
   * compare the engine can't do yet (this lane's brief, item 3: "never
   * invent results"). The UI shows this instead of the (empty)
   * `sentences` list when it is set.
   */
  stubNote: string | null;
  trackHeat: readonly TrackHeat[];
}

/**
 * The DAW to show on a Shelf card or tray line, as Story text — never a
 * UI-side enum-to-label mapping (review round 1, blocking #2: two
 * "Coastline"s, Logic and Live, looked identical with no DAW shown).
 * `ShelfCard` doesn't carry its own `daw_label` yet (that's landing in a
 * small wit-story contract PR — `Library.daw_label`); until then this
 * reads it off the card's first story, `story_ids[0]`, per the
 * coordinator's interim instruction.
 */
export function firstStoryDawLabel(library: Library, card: ShelfCard): string | null {
  const storyId = card.story_ids[0];
  if (!storyId) return null;
  const story = library.stories.find((s) => s.id === storyId);
  return story?.header.daw_label ?? null;
}

export function findMoment(story: Story, momentId: string): Moment | null {
  for (const session of story.sessions) {
    const m = session.moments.find((mm) => mm.id === momentId);
    if (m) return m;
  }
  return null;
}

const NO_ACTIONS: Actions = { listen: false, open_as_copy: false, send: false };

export function cardFromMoment(moment: Moment): CardData {
  return {
    heading: moment.heading,
    subheading: moment.subheading,
    sentences: moment.sentences,
    summary: moment.summary ?? null,
    note: moment.note ?? null,
    actions: moment.actions,
    momentId: moment.id,
    stubNote: null,
    trackHeat: moment.track_heat,
  };
}

/**
 * A real, precomputed [`Comparison`] (the Story's own `overview`, or a
 * future real `compare` result). The three actions belong to its `to`
 * moment: "listen" / "open as copy" / "send" are always about one
 * particular save, and `to` is the one being compared *to* — its own
 * `actions` are real facts, not a guess.
 */
export function cardFromComparison(cmp: Comparison, story: Story): CardData {
  const to = findMoment(story, cmp.to);
  return {
    heading: cmp.heading,
    subheading: cmp.subheading,
    sentences: cmp.sentences,
    summary: cmp.summary ?? null,
    note: cmp.note ?? null,
    actions: to?.actions ?? NO_ACTIONS,
    momentId: to?.id ?? cmp.to,
    stubNote: null,
    trackHeat: cmp.track_heat,
  };
}

/**
 * A drag-compare between two arbitrary moments while the engine's
 * `compare` command is still a stub: never invent sentences — show the
 * honest "not available yet" message, with the three actions still drawn
 * from the real `to` moment (a real fact, independent of the compare
 * itself being unavailable).
 */
export function cardFromStubbedCompare(
  story: Story,
  fromId: string,
  toId: string,
  stubMessage: string
): CardData {
  const from = findMoment(story, fromId);
  const to = findMoment(story, toId);
  const subheading = from && to ? `comparing ${from.label} with ${to.label}` : "";
  return {
    heading: "Comparing two moments",
    subheading,
    sentences: [],
    summary: null,
    note: null,
    actions: to?.actions ?? NO_ACTIONS,
    momentId: to?.id ?? toId,
    stubNote: stubMessage,
    trackHeat: [],
  };
}
