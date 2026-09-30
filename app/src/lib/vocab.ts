/**
 * The banned-vocabulary lint, shared by `scripts/lint-vocab.mjs` (which
 * scans `app/src` string literals and template text at build time) and by
 * `vocab.test.ts` (which proves the whole-word matching behaves).
 *
 * This list mirrors `crates/wit-story/src/vocab.rs` exactly: Wit's engine
 * already enforces it over every Story it builds, and this file holds the
 * app lane to the same bar for the chrome text the UI is allowed to author
 * itself (button labels, section titles, empty/loading states — see this
 * lane's brief, "Laws"). A name the musician chose is never checked: those
 * come from the Story JSON as data, never as a UI string literal.
 */
import wordsData from "./vocab-words.json";

export const BANNED_WORDS: readonly string[] = wordsData.words;
export const BANNED_PHRASES: readonly string[] = wordsData.phrases;

const wordSet = new Set(BANNED_WORDS);

/**
 * Banned words found in `text`, lowercased, in order of appearance.
 * Matched as whole words (split on any run of non-alphanumeric characters),
 * exactly like `wit_story::vocab::banned_words` — so "Pushed the tempo"
 * does not flag "push", but "push it" flags "push".
 */
export function bannedWords(text: string): string[] {
  const lower = text.toLowerCase();
  const found: string[] = [];
  for (const word of lower.split(/[^a-z0-9]+/)) {
    if (word && wordSet.has(word)) found.push(word);
  }
  for (const phrase of BANNED_PHRASES) {
    if (lower.includes(phrase)) found.push(phrase);
  }
  return found;
}
