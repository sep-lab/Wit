import type { Icon } from "./story";

export interface IconGlyph {
  /** A single decorative glyph. Never a claim about the music — purely a
   * visual marker for `Sentence.icon`, same role as the mockup's `+ − ↔ ✂`
   * column. */
  glyph: string;
  /** Structural accessibility label (`aria-label`), not musician-facing
   * prose — the sentence text itself (from the Story) carries the words a
   * screen reader should really announce; this only names the glyph. */
  label: string;
}

/**
 * `Icon` → glyph, one pure mapping, so both the change card and any test
 * agree on it. Kept deliberately un-fancy (no emoji, monochrome symbols
 * that inherit `currentColor`) to match the approved mockup's icon column.
 */
const GLYPHS: Record<Icon, IconGlyph> = {
  add: { glyph: "+", label: "add" },
  remove: { glyph: "−", label: "remove" }, // −
  move: { glyph: "↔", label: "move" }, // ↔
  trim: { glyph: "✂", label: "trim" }, // ✂
  duplicate: { glyph: "⧉", label: "duplicate" }, // ⧉
  rename: { glyph: "✎", label: "rename" }, // ✎
  record: { glyph: "●", label: "record" }, // ●
  tempo: { glyph: "♩", label: "tempo" }, // ♩
  key: { glyph: "♭", label: "key" }, // ♭
  meter: { glyph: "⊡", label: "meter" }, // ⊡
  mute: { glyph: "◌", label: "mute" }, // ◌
  mix: { glyph: "≡", label: "mix" }, // ≡
  plugin: { glyph: "▣", label: "plugin" }, // ▣
  marker: { glyph: "◆", label: "marker" }, // ◆
  automation: { glyph: "∿", label: "automation" }, // ∿
  midi: { glyph: "♫", label: "midi" }, // ♫
  tracks: { glyph: "☰", label: "tracks" }, // ☰
  audio_file: { glyph: "♪", label: "audio file" }, // ♪
};

/** The glyph and accessibility label for a `Sentence.icon`. */
export function iconGlyph(icon: Icon): IconGlyph {
  return GLYPHS[icon] ?? { glyph: "•", label: icon };
}
