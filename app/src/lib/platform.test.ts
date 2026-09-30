import { afterEach, describe, expect, it, vi } from "vitest";
import { hasShortcutModifier, isMacPlatform, modifierKeyLabel } from "./platform";

afterEach(() => {
  vi.unstubAllGlobals();
});

function stubPlatform(platform: string) {
  vi.stubGlobal("navigator", { platform, userAgent: platform });
}

describe("isMacPlatform / modifierKeyLabel", () => {
  it("detects Mac and shows the ⌘ glyph", () => {
    stubPlatform("MacIntel");
    expect(isMacPlatform()).toBe(true);
    expect(modifierKeyLabel()).toBe("⌘");
  });

  it("shows Ctrl elsewhere", () => {
    stubPlatform("Win32");
    expect(isMacPlatform()).toBe(false);
    expect(modifierKeyLabel()).toBe("Ctrl");

    stubPlatform("Linux x86_64");
    expect(isMacPlatform()).toBe(false);
    expect(modifierKeyLabel()).toBe("Ctrl");
  });
});

describe("hasShortcutModifier", () => {
  it("checks metaKey on Mac, ctrlKey elsewhere", () => {
    stubPlatform("MacIntel");
    expect(hasShortcutModifier({ metaKey: true, ctrlKey: false } as KeyboardEvent)).toBe(true);
    expect(hasShortcutModifier({ metaKey: false, ctrlKey: true } as KeyboardEvent)).toBe(false);

    stubPlatform("Win32");
    expect(hasShortcutModifier({ metaKey: true, ctrlKey: false } as KeyboardEvent)).toBe(false);
    expect(hasShortcutModifier({ metaKey: false, ctrlKey: true } as KeyboardEvent)).toBe(true);
  });
});
