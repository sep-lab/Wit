import { describe, expect, it } from "vitest";
import { bannedWords } from "./vocab";

// Mirrors crates/wit-story/src/vocab.rs's own `whole_words_only` test
// exactly, proving the two implementations agree.
describe("bannedWords", () => {
  it("matches whole words only, case-insensitively", () => {
    expect(bannedWords("Pushed the tempo")).toEqual([]);
    expect(bannedWords("push it")).toEqual(["push"]);
    expect(bannedWords("Headphone mix")).toEqual([]);
    expect(bannedWords("Wit is version control")).toEqual(["version control"]);
    expect(bannedWords("A Snapshot of HEAD")).toEqual(["snapshot", "head"]);
  });

  it("does not flag a musician's own name that merely contains a banned word", () => {
    // A track called "Cloudy" or "Headrush" is the musician's to name.
    expect(bannedWords("Cloudy")).toEqual([]);
    expect(bannedWords("Headrush")).toEqual([]);
  });
});
