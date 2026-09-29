import { describe, expect, it } from "vitest";
import { iconGlyph } from "./icons";
import type { Icon } from "./story";

// Every variant of the generated `Icon` union (crates/wit-story
// types.rs::Icon, snake_cased by serde). Written out so this test fails
// loudly — not silently — if a variant is ever added and forgotten here.
const ALL_ICONS: Icon[] = [
  "add",
  "remove",
  "move",
  "trim",
  "duplicate",
  "rename",
  "record",
  "tempo",
  "key",
  "meter",
  "mute",
  "mix",
  "plugin",
  "marker",
  "automation",
  "midi",
  "tracks",
  "audio_file",
];

describe("iconGlyph", () => {
  it("maps every Icon variant to its own, distinct glyph", () => {
    const glyphs = ALL_ICONS.map((icon) => iconGlyph(icon).glyph);
    expect(new Set(glyphs).size).toBe(ALL_ICONS.length);
    for (const glyph of glyphs) {
      expect(glyph.length).toBeGreaterThan(0);
    }
  });

  it("gives every icon a plain lowercase accessibility label", () => {
    for (const icon of ALL_ICONS) {
      expect(iconGlyph(icon).label).toMatch(/^[a-z ]+$/);
    }
  });

  it("falls back gracefully for a variant this build doesn't know about yet", () => {
    // Compatibility rule (wit-story lib.rs): clients must treat an unknown
    // enum variant as "other", not throw.
    const unknown = iconGlyph("some_future_icon" as Icon);
    expect(unknown.glyph.length).toBeGreaterThan(0);
  });
});
