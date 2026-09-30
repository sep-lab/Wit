import type { Comparison, Library } from "./story";

/**
 * True when running inside a Tauri webview. `@tauri-apps/api`'s `invoke`
 * throws if called outside one (a plain browser tab, `vite dev`, or
 * `vitest`'s jsdom), so every call here checks first rather than letting
 * that throw look like a real IPC failure.
 */
export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/**
 * A feature the engine doesn't implement yet. This lane's brief, item 3:
 * "Stub commands that exist in the contract's future but not the engine
 * yet (`compare`, `open_as_copy`, `send`, `reveal`) must return a typed
 * 'not available yet' error the UI shows plainly — never invent results."
 * The Rust side returns `{ notAvailable: "<feature>" }`; this is the same
 * shape surfacing in a plain browser, where there is no IPC at all.
 */
export class NotAvailableError extends Error {
  constructor(public readonly feature: string) {
    super(`${feature} isn't available yet.`);
    this.name = "NotAvailableError";
  }
}

function isNotAvailablePayload(err: unknown): err is { notAvailable: string } {
  return (
    typeof err === "object" &&
    err !== null &&
    "notAvailable" in err &&
    typeof (err as { notAvailable: unknown }).notAvailable === "string"
  );
}

async function invokeOrStub<T>(
  command: string,
  args: Record<string, unknown> | undefined,
  feature: string
): Promise<T> {
  if (!isTauri()) {
    throw new NotAvailableError(feature);
  }
  const { invoke } = await import("@tauri-apps/api/core");
  try {
    return await invoke<T>(command, args);
  } catch (err) {
    if (isNotAvailablePayload(err)) {
      throw new NotAvailableError(err.notAvailable);
    }
    throw err;
  }
}

/**
 * The Library to show: the real IPC command when running inside Tauri,
 * or the fixture everywhere else (this lane's brief, item 3). Review
 * round 1, non-blocking #1: the command took an arbitrary root path with
 * no caller and no validation yet — dropped until `crates/wit-platform`
 * exists to hand it a real, checked root. Both sides serve the same
 * embedded fixture for now.
 */
export async function fetchLibrary(): Promise<Library> {
  if (!isTauri()) {
    const { demoLibrary } = await import("./fixture");
    return demoLibrary;
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<Library>("library");
}

/** Drag-compare on the timeline (this lane's brief, item 4). Stubbed until
 * the engine grows a two-arbitrary-moments compare; the return type is
 * already the real `Comparison` shape so the UI needs no change the day
 * this stops being a stub. */
export function compareMoments(
  storyId: string,
  from: string,
  to: string
): Promise<Comparison> {
  return invokeOrStub<Comparison>(
    "compare",
    { storyId, from, to },
    "Comparing two moments"
  );
}

/** "Open this moment as a copy" (ADR-0007). Stubbed until a DAW's restored
 * copy has been shown to open in it. */
export function openAsCopy(storyId: string, momentId: string): Promise<void> {
  return invokeOrStub<void>(
    "open_as_copy",
    { storyId, momentId },
    "Opening a moment as a copy"
  );
}

/** "Send to a friend" (Phase E share page). Stubbed until the share page
 * exists. */
export function sendToFriend(storyId: string, momentId: string): Promise<void> {
  return invokeOrStub<void>("send", { storyId, momentId }, "Sending to a friend");
}

/** Reveal a song's folder in Finder / Explorer / the file manager. Stubbed
 * until `crates/wit-platform` lands. */
export function reveal(songId: string): Promise<void> {
  return invokeOrStub<void>("reveal", { songId }, "Revealing this song's folder");
}

/** Show and focus the main window — used by the tray popover. A real
 * command, not a stub: it only needs Tauri's own window API. */
export function showMainWindow(): Promise<void> {
  return invokeOrStub<void>("show_main_window", undefined, "Opening the main window");
}
