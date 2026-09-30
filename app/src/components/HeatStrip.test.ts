import { mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import HeatStrip from "./HeatStrip.svelte";
import { demoLibrary } from "../lib/fixture";
import type { Moment, TrackLane } from "../lib/story";

// Review round 1, non-blocking #9: a component-mount test for "empty
// tracks → no strip at all" (this lane's brief, "Laws"), not just an
// assertion on the pure `buildHeatMatrix` helper.

let target: HTMLElement | null = null;
let instance: unknown = null;

afterEach(() => {
  if (instance) unmount(instance);
  target?.remove();
  target = null;
  instance = null;
});

function mountHeatStrip(tracks: TrackLane[], moments: Moment[]) {
  target = document.createElement("div");
  document.body.appendChild(target);
  instance = mount(HeatStrip, {
    target,
    props: { tracks, moments, selectedMomentId: null, compareRange: null },
  });
  return target;
}

describe("HeatStrip (mounted)", () => {
  it("renders no strip at all when the Story has no track lanes (e.g. Logic)", () => {
    const logicStory = demoLibrary.stories.find((s) => s.tracks.length === 0);
    expect(logicStory).toBeDefined();
    const moments = logicStory!.sessions.flatMap((s) => s.moments);

    const el = mountHeatStrip([], moments);

    expect(el.querySelector(".heat-strip")).toBeNull();
    expect(el.textContent?.trim()).toBe("");
  });

  it("renders a row per track when the Story does have lanes (e.g. Ableton)", () => {
    const abletonStory = demoLibrary.stories.find((s) => s.tracks.length > 0);
    expect(abletonStory).toBeDefined();
    const moments = abletonStory!.sessions.flatMap((s) => s.moments);

    const el = mountHeatStrip(abletonStory!.tracks, moments);

    expect(el.querySelectorAll(".row").length).toBe(abletonStory!.tracks.length);
  });
});
