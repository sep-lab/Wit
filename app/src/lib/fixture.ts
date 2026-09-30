import type { Library } from "./story";
// Vite JSON import, resolved and inlined at build time — this lane's brief,
// item 3: "load crates/wit-story/fixtures/demo-library.json (import it at
// build time) — so the UI is developed and screenshot-able in any browser
// against the fixture." `just demo-library` regenerates this file; nothing
// here modifies it.
import demoLibraryJson from "../../../crates/wit-story/fixtures/demo-library.json";

export const demoLibrary = demoLibraryJson as unknown as Library;
