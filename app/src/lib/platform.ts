/**
 * Which modifier key a shortcut hint should show: ⌘ on Mac, Ctrl
 * elsewhere (this lane's brief, "Layout & theming"). Detected once from
 * the browser/webview, never from a build-time flag, since the same
 * bundle runs unsigned on all three of ADR-0008's OSes.
 */
export function isMacPlatform(): boolean {
  if (typeof navigator === "undefined") return false;
  const uaDataPlatform = (navigator as { userAgentData?: { platform?: string } }).userAgentData
    ?.platform;
  const platform = uaDataPlatform ?? navigator.platform ?? navigator.userAgent ?? "";
  return /mac/i.test(platform);
}

/** "⌘" on Mac, "Ctrl" elsewhere — a symbol/word, not a claim about
 * the music, so it's outside the vocabulary lint's concern. */
export function modifierKeyLabel(): string {
  return isMacPlatform() ? "⌘" : "Ctrl";
}

/** Whether this keyboard event carries the platform's own shortcut
 * modifier (⌘ on Mac, Ctrl elsewhere). */
export function hasShortcutModifier(event: KeyboardEvent): boolean {
  return isMacPlatform() ? event.metaKey : event.ctrlKey;
}
