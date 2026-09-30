import { describe, expect, it } from "vitest";
import { demoLibrary } from "./fixture";
import { cardFromComparison, cardFromMoment, cardFromStubbedCompare, findMoment } from "./card";

const story = demoLibrary.stories.find((s) => s.overview)!;

describe("findMoment", () => {
  it("finds a moment by id across sessions", () => {
    const anyId = story.sessions[0].moments[0].id;
    expect(findMoment(story, anyId)?.id).toBe(anyId);
  });

  it("returns null for an id that doesn't exist", () => {
    expect(findMoment(story, "not-a-real-id")).toBeNull();
  });
});

describe("cardFromMoment", () => {
  it("carries the moment's own actions through untouched", () => {
    const moment = story.sessions[0].moments[0];
    const card = cardFromMoment(moment);
    expect(card.actions).toEqual(moment.actions);
    expect(card.stubNote).toBeNull();
  });
});

describe("cardFromComparison", () => {
  it("borrows the `to` moment's real actions, not a guess", () => {
    const cmp = story.overview!;
    const card = cardFromComparison(cmp, story);
    const to = findMoment(story, cmp.to);
    expect(card.actions).toEqual(to?.actions);
    expect(card.stubNote).toBeNull();
  });
});

describe("cardFromStubbedCompare", () => {
  it("never invents sentences — shows the stub note instead", () => {
    const ids = story.sessions.flatMap((s) => s.moments.map((m) => m.id));
    const card = cardFromStubbedCompare(
      story,
      ids[0],
      ids[ids.length - 1],
      "Comparing two moments isn't available yet."
    );
    expect(card.sentences).toEqual([]);
    expect(card.stubNote).toBe("Comparing two moments isn't available yet.");
  });

  it("still shows the real actions of the newer moment picked", () => {
    const ids = story.sessions.flatMap((s) => s.moments.map((m) => m.id));
    const to = findMoment(story, ids[ids.length - 1]);
    const card = cardFromStubbedCompare(story, ids[0], ids[ids.length - 1], "stub");
    expect(card.actions).toEqual(to?.actions);
  });
});
