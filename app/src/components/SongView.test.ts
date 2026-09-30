import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import SongView from "./SongView.svelte";
import { appState } from "../stores/app-state.svelte";
import { demoLibrary } from "../lib/fixture";

// Review round 1, non-blocking #9: a component-mount test proving the
// *actual* first view is the Story's own overview (oldest → newest,
// never an adjacent pair — this lane's brief, item 4), not just that the
// pure `cardFromComparison` helper can build one.

let target: HTMLElement | null = null;
let instance: unknown = null;

afterEach(() => {
  if (instance) unmount(instance);
  target?.remove();
  target = null;
  instance = null;
  appState.library = null;
  appState.selectedSongId = null;
  appState.selectedStoryId = null;
  appState.selectedMomentId = null;
  appState.compareRange = null;
});

describe("SongView (mounted)", () => {
  it("shows the Story's overview heading by default, with no moment or range picked", () => {
    const story = demoLibrary.stories.find((s) => s.overview);
    expect(story).toBeDefined();

    appState.library = demoLibrary;
    appState.selectedSongId = story!.song_id;
    appState.selectedStoryId = story!.id;
    appState.selectedMomentId = null;
    appState.compareRange = null;
    appState.loading = false;

    target = document.createElement("div");
    document.body.appendChild(target);
    instance = mount(SongView, { target });
    flushSync();

    // The change card's heading is the overview's, not any one moment's
    // "What changed at ..." — and the "Overview" pill (not a "back to
    // it" button) is shown, since this already *is* the overview.
    expect(target.textContent).toContain(story!.overview!.heading);
    expect(target.querySelector(".overview-row .pill")?.textContent).toMatch(/overview/i);
  });

  it("switches to a 'back to the overview' control once a moment is picked", () => {
    const story = demoLibrary.stories.find((s) => s.overview);
    expect(story).toBeDefined();

    appState.library = demoLibrary;
    appState.selectedSongId = story!.song_id;
    appState.selectedStoryId = story!.id;
    appState.selectedMomentId = story!.sessions[0].moments[0].id;
    appState.compareRange = null;
    appState.loading = false;

    target = document.createElement("div");
    document.body.appendChild(target);
    instance = mount(SongView, { target });
    flushSync();

    expect(target.querySelector(".overview-row .pill")).toBeNull();
    const backButton = Array.from(target.querySelectorAll("button")).find((b) =>
      /overview/i.test(b.textContent ?? "")
    );
    expect(backButton).toBeDefined();
  });
});
