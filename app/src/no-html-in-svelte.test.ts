import { describe, expect, it } from "vitest";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

// vitest's runtime doesn't give `import.meta.url` a real file: URL, so this
// anchors on process.cwd() instead — vitest always runs from app/ (see
// vite.config.ts, which has no `root` override).
const srcDir = join(process.cwd(), "src");

function* walk(dir: string): Generator<string> {
  for (const entry of readdirSync(dir)) {
    const p = join(dir, entry);
    const s = statSync(p);
    if (s.isDirectory()) yield* walk(p);
    else if (p.endsWith(".svelte")) yield p;
  }
}

describe("no {@html} anywhere", () => {
  it("never renders a musician's name as markup (this lane's brief, Laws)", () => {
    const offenders: string[] = [];
    for (const file of walk(srcDir)) {
      // Strip HTML comments first — this file's own comments *talk about*
      // never using {@html}, which would otherwise trip this check on its
      // own prose.
      const withoutComments = readFileSync(file, "utf8").replace(/<!--[\s\S]*?-->/g, "");
      if (withoutComments.includes("{@html")) offenders.push(file);
    }
    expect(offenders).toEqual([]);
  });
});
