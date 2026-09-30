import { mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import SentenceLine from "./SentenceLine.svelte";
import type { Sentence } from "../lib/story";

// Review round 1, non-blocking #9: spans.test.ts's old "never turns a
// name into markup" test only checked the pure `renderSpan` helper's
// return value — it never actually rendered anything, so it could not
// have caught a real `{@html}` regression. This mounts the real
// component in jsdom and inspects the DOM it produces.

let target: HTMLElement | null = null;
let instance: unknown = null;

afterEach(() => {
  if (instance) unmount(instance);
  target?.remove();
  target = null;
  instance = null;
});

function mountSentence(sentence: Sentence) {
  target = document.createElement("div");
  document.body.appendChild(target);
  instance = mount(SentenceLine, { target, props: { sentence } });
  return target;
}

describe("SentenceLine (mounted)", () => {
  it("renders a hostile name as text content, never as an element", () => {
    const hostile = "<script>window.__pwned = true;</script>";
    const sentence: Sentence = {
      icon: "rename",
      text: `Renamed '${hostile}'`,
      spans: [
        { kind: "plain", text: "Renamed '" },
        { kind: "name", text: hostile },
        { kind: "plain", text: "'" },
      ],
      track: null,
      lane: null,
      place: null,
      place_label: null,
      confidence: "exact",
      tier: "structure",
    };

    const el = mountSentence(sentence);

    // No <script> element was created — the string was never parsed as
    // markup, only inserted as a text node.
    expect(el.querySelector("script")).toBeNull();
    // The hostile string is nonetheless fully present, verbatim, as text.
    expect(el.textContent).toContain(hostile);
  });

  it("bolds a track name span and isolates it for RTL, as an element attribute", () => {
    const sentence: Sentence = {
      icon: "add",
      text: "Added region 'verse' on Rhodes",
      spans: [
        { kind: "plain", text: "Added region '" },
        { kind: "region", text: "verse" },
        { kind: "plain", text: "' on " },
        { kind: "track", text: "Rhodes" },
      ],
      track: "Rhodes",
      lane: null,
      place: null,
      place_label: null,
      confidence: "exact",
      tier: "semantic",
    };

    const el = mountSentence(sentence);
    const trackSpan = el.querySelector(".span-track");
    expect(trackSpan).not.toBeNull();
    expect(trackSpan?.classList.contains("span-bold")).toBe(true);
    expect(trackSpan?.getAttribute("dir")).toBe("auto");
    expect(el.querySelector(".span-region")?.classList.contains("span-bold")).toBe(false);
  });
});
