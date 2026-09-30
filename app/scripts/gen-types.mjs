#!/usr/bin/env node
// Generates src/lib/story.ts from the Story contract's JSON schema
// (crates/wit-story/schema/library.schema.json).
//
// This is a plain Node script rather than `json2ts ... --bannerComment
// "$(cat scripts/story-ts-banner.txt)"` in package.json, because npm runs
// package.json scripts through cmd.exe on Windows regardless of which
// shell the *calling* process used — `$(cat ...)` bash command
// substitution is never expanded there, so json2ts received the literal
// text `$(cat scripts/story-ts-banner.txt)` as its banner argument and
// tried to parse it as TypeScript. Reading the banner file directly in
// Node sidesteps the host shell entirely.
import { compileFromFile } from "json-schema-to-typescript";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const scriptsDir = dirname(fileURLToPath(import.meta.url));
const appRoot = join(scriptsDir, "..");
const schemaPath = join(appRoot, "..", "crates", "wit-story", "schema", "library.schema.json");
const outPath = join(appRoot, "src", "lib", "story.ts");
const bannerComment = readFileSync(join(scriptsDir, "story-ts-banner.txt"), "utf8").trimEnd();

const ts = await compileFromFile(schemaPath, { bannerComment });
writeFileSync(outPath, ts);
console.log(`wrote ${outPath}`);
