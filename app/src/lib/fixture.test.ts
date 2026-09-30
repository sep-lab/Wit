import { describe, expect, it } from "vitest";
import { demoLibrary } from "./fixture";
import { renderSpan } from "./spans";
import type { Sentence, Span, Story } from "./story";

describe("demo-library fixture", () => {
  it("parses into the generated Library type's shape", () => {
    expect(demoLibrary.schema_version).toBe(2);
    expect(Array.isArray(demoLibrary.shelf)).toBe(true);
    expect(Array.isArray(demoLibrary.stories)).toBe(true);
    expect(demoLibrary.shelf.length).toBeGreaterThan(0);
    expect(demoLibrary.stories.length).toBeGreaterThan(0);
    for (const story of demoLibrary.stories) {
      expect(typeof story.id).toBe("string");
      expect(typeof story.header.title).toBe("string");
      expect(Array.isArray(story.sessions)).toBe(true);
    }
    expect(demoLibrary.trust.statements.length).toBeGreaterThan(0);
  });

  it("every shelf card points at a story that exists", () => {
    const storyIds = new Set(demoLibrary.stories.map((s) => s.id));
    for (const card of demoLibrary.shelf) {
      for (const id of card.story_ids) {
        expect(storyIds.has(id)).toBe(true);
      }
    }
  });
});

/** Find the first span of a name kind anywhere in a Story, so the RTL test
 * below exercises a real fixture shape rather than a hand-built one. */
function findNameSpan(story: Story): { sentence: Sentence; span: Span } | null {
  for (const session of story.sessions) {
    for (const moment of session.moments) {
      for (const sentence of moment.sentences) {
        const span = sentence.spans.find(
          (s) => s.kind === "track" || s.kind === "region" || s.kind === "name"
        );
        if (span) return { sentence, span };
      }
    }
  }
  return null;
}

describe("RTL names (this lane's brief: 'test with a Persian track name')", () => {
  it("renders a Persian name injected into a copy of the fixture as plain, isolated text", () => {
    // A realistic Persian track name — "Night Guitar".
    const persianName = "گیتار شب";

    // Copy the whole library (never mutate the shared fixture import) and
    // find a real name span in it, wherever in the library it happens to be.
    const library = structuredClone(demoLibrary);
    let found: { sentence: Sentence; span: Span } | null = null;
    for (const story of library.stories) {
      found = findNameSpan(story);
      if (found) break;
    }
    expect(found).not.toBeNull();
    const { span } = found as { sentence: Sentence; span: Span };
    const original = span.text;

    span.text = persianName;

    const rendered = renderSpan(span);
    // The original fixture import is untouched — only the copy changed.
    expect(original).not.toBe(persianName);
    // The Persian text survives byte-for-byte and is flagged for
    // dir="auto" + unicode-bidi: isolate, never re-encoded or escaped.
    expect(rendered.text).toBe(persianName);
    expect(rendered.rtl).toBe(true);
  });
});
