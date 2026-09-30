#!/usr/bin/env node
// Banned-vocabulary lint over app/src (this lane's brief, "Laws": "Every
// UI-authored string must pass the banned-vocabulary list"). Mirrors
// crates/wit-story/src/vocab.rs::banned_words exactly, sharing the word
// list in src/lib/vocab-words.json so the app and the engine never drift.
//
// What counts as a "UI-authored string" here:
//   - every quoted string / template literal in a .ts file or inside a
//     .svelte file's <script> block, mustache expression or attribute
//     value (covers chrome.ts constants, ternaries, aria-labels, …);
//   - every static text node in a .svelte file's markup (covers a bare
//     heading like `<h1>Family</h1>`);
//   - every quoted string literal in app/src-tauri/src/*.rs (review round
//     1, non-blocking #7): a Rust `NotAvailable::new("...")` message
//     crosses the IPC boundary and is shown to the musician exactly as
//     written (see src/lib/ipc.ts's NotAvailableError), so it is held to
//     the same bar as a TS string literal, not treated as backend-only.
//
// Deliberately NOT scanned, because it is never shown to a musician:
//   - <style> block contents (CSS, not prose — this is also why no CSS
//     class name in this app may literally be one of the banned words:
//     a `class="head"` attribute value *is* scanned, since it's a quoted
//     string in the markup, even though the matching `<style>.head{}`
//     selector is not);
//   - comments (// and /* */) — never string literals to begin with, so
//     the extraction below simply never reaches into them;
//   - src/lib/story.ts — generated TS types (npm run gen:types). Type-only,
//     erased at compile time, never rendered — and it carries schema doc
//     comments straight from the Rust source (e.g. SongId: "a hash of the
//     location"), which are developer documentation, not UI text;
//   - src/lib/vocab.ts, src/lib/vocab-words.json — the list itself;
//   - *.test.ts / *.spec.ts — test fixtures deliberately exercise banned
//     words to prove the checker catches them.
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const here = fileURLToPath(new URL(".", import.meta.url));
const appRoot = join(here, "..");
const srcDir = join(appRoot, "src");
const rustSrcDir = join(appRoot, "src-tauri", "src");

const { words, phrases } = JSON.parse(
  readFileSync(join(srcDir, "lib", "vocab-words.json"), "utf8")
);
const wordSet = new Set(words);

const EXCLUDE_FILES = new Set([join(srcDir, "lib", "story.ts"), join(srcDir, "lib", "vocab.ts")]);

function bannedWords(text) {
  const lower = text.toLowerCase();
  const found = [];
  for (const word of lower.split(/[^a-z0-9]+/)) {
    if (word && wordSet.has(word)) found.push(word);
  }
  for (const phrase of phrases) {
    if (lower.includes(phrase)) found.push(phrase);
  }
  return found;
}

const QUOTED_LITERAL = /'(?:\\.|[^'\\])*'|"(?:\\.|[^"\\])*"|`(?:\\.|[^`\\])*`/g;

/** Contents of every quoted string / template literal in `text` (quote
 * characters stripped). Good enough for our own source, which doesn't use
 * exotic escaping. Call this only on code with comments already stripped
 * — a JSDoc comment's inline `` `code span` `` markdown otherwise looks
 * exactly like an opened-and-later-closed template literal. */
function quotedLiterals(text) {
  return [...text.matchAll(QUOTED_LITERAL)].map((m) => m[0].slice(1, -1));
}

// Strip line and block comments before any literal extraction — a
// comment is developer documentation, never a string a user sees, and
// (per the note on quotedLiterals above) its backticks can otherwise be
// misread as an open template literal. Naive (doesn't know about strings
// containing "//"), but sufficient for this codebase, which has none.
function stripJsComments(code) {
  return code.replace(/\/\*[\s\S]*?\*\//g, " ").replace(/\/\/.*$/gm, " ");
}

function stripHtmlComments(markup) {
  return markup.replace(/<!--[\s\S]*?-->/g, " ");
}

/** `.svelte` markup with `<script>`/`<style>` blocks, HTML comments, tags
 * and mustache expressions removed — what's left is the plain text nodes
 * a musician (or reviewer) actually reads on screen, e.g. a bare
 * `<h1>Family</h1>`. */
function bareMarkupText(markupWithoutScriptStyleOrComments) {
  return markupWithoutScriptStyleOrComments.replace(/\{[^{}]*\}/g, " ").replace(/<[^>]*>/g, " ");
}

/** Every string this file could show a user: quoted literals from .ts
 * files, or from a .svelte file's script/markup/attributes, plus a
 * .svelte file's bare markup text nodes. */
function stringsToCheck(filePath, text) {
  if (!filePath.endsWith(".svelte")) {
    return quotedLiterals(stripJsComments(text));
  }
  const scriptBlocks = [...text.matchAll(/<script[^>]*>([\s\S]*?)<\/script>/g)];
  const styleBlocks = [...text.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)];
  let markup = text;
  for (const m of scriptBlocks) markup = markup.replace(m[0], " ");
  for (const m of styleBlocks) markup = markup.replace(m[0], " "); // CSS, never scanned
  markup = stripHtmlComments(markup);

  const out = [];
  for (const m of scriptBlocks) out.push(...quotedLiterals(stripJsComments(m[1])));
  out.push(...quotedLiterals(markup)); // mustache literals + attribute values
  out.push(bareMarkupText(markup)); // static text nodes
  return out;
}

function shouldScan(path) {
  if (EXCLUDE_FILES.has(path)) return false;
  if (/\.(test|spec)\.tsx?$/.test(path)) return false;
  return /\.(ts|svelte|rs)$/.test(path);
}

function* walk(dir) {
  for (const entry of readdirSync(dir)) {
    const p = join(dir, entry);
    const s = statSync(p);
    if (s.isDirectory()) yield* walk(p);
    else yield p;
  }
}

let scanned = 0;
const violations = [];
for (const file of [...walk(srcDir), ...walk(rustSrcDir)]) {
  if (!shouldScan(file)) continue;
  scanned += 1;
  const text = readFileSync(file, "utf8");
  for (const str of stringsToCheck(file, text)) {
    for (const w of bannedWords(str)) {
      violations.push(`${relative(appRoot, file)}: '${w}' in: ${str.trim().slice(0, 120)}`);
    }
  }
}

if (violations.length > 0) {
  console.error("lint:vocab failed — banned word(s) found in app/src:\n");
  for (const v of violations) console.error("  " + v);
  console.error(
    `\n${violations.length} violation(s). These words are reserved for Wit's own vocabulary ` +
      "ban (crates/wit-story/src/vocab.rs) — the UI's own chrome text may not use them either."
  );
  process.exit(1);
} else {
  console.log(`lint:vocab — clean (${scanned} file(s) scanned)`);
}
