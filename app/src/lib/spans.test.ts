import { describe, expect, it } from "vitest";
import { renderSpan, renderSpans } from "./spans";
import type { Span } from "./story";

describe("renderSpan", () => {
  it("passes span.text through byte-for-byte — it is data, never parsed", () => {
    const hostile: Span = { kind: "track", text: "<b>evil</b> & \"quoted\"" };
    expect(renderSpan(hostile).text).toBe(hostile.text);
  });

  it("bolds track and plugin names only", () => {
    expect(renderSpan({ kind: "track", text: "Rhodes" }).className).toContain("span-bold");
    expect(renderSpan({ kind: "plugin", text: "AutoFilter" }).className).toContain("span-bold");
    expect(renderSpan({ kind: "region", text: "verse" }).className).not.toContain("span-bold");
    expect(renderSpan({ kind: "plain", text: "Added " }).className).not.toContain("span-bold");
    expect(renderSpan({ kind: "value", text: "124 BPM" }).className).not.toContain("span-bold");
  });

  it("marks every name kind (not plain, not value) for RTL isolation", () => {
    const nameKinds: Span["kind"][] = ["track", "region", "file", "plugin", "marker", "name"];
    for (const kind of nameKinds) {
      expect(renderSpan({ kind, text: "x" }).rtl).toBe(true);
    }
    expect(renderSpan({ kind: "plain", text: "x" }).rtl).toBe(false);
    expect(renderSpan({ kind: "value", text: "x" }).rtl).toBe(false);
  });

  it("never turns a name into markup, even one that looks like a tag", () => {
    const spans: Span[] = [
      { kind: "plain", text: "Renamed '" },
      { kind: "name", text: "<script>alert(1)</script>" },
      { kind: "plain", text: "'" },
    ];
    const rendered = renderSpans(spans);
    expect(rendered.map((r) => r.text).join("")).toBe(
      "Renamed '<script>alert(1)</script>'"
    );
    // Nothing here is interpreted as HTML: renderSpan never strips,
    // escapes or evaluates the text — the component binds it as a text
    // node ({span.text}, never {@html span.text}), so the string above
    // is displayed literally, not executed.
    for (const r of rendered) {
      expect(typeof r.text).toBe("string");
    }
  });
});
