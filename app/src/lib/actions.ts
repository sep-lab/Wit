import type { Actions } from "./story";

export type ActionKey = keyof Actions;

export const ACTION_KEYS: readonly ActionKey[] = ["listen", "open_as_copy", "send"];

/**
 * The short plain reason shown under a disabled change-card action (this
 * lane's brief, "Laws": "actions shown DISABLED when `actions.*` is false
 * ... with a short plain reason"). `null` when the action is enabled —
 * there is nothing to explain.
 *
 * These are the only three UI-authored strings here; they name a fact
 * about the *feature* ("not built yet"), never the music, so they carry
 * no claim the vocabulary lint would need to catch.
 */
const REASONS: Record<ActionKey, string> = {
  // Review round 1, blocking #6: "No audio to play" contradicted the
  // Story itself (a sentence can say "New audio file 'Brushed Kit
  // 124.caf'" in the very same moment) — there can be plenty of audio;
  // the engine just never fills `listen: ListenRef[]` yet. Worded like
  // the other two: the *feature*, not the data, is what's missing.
  listen: "Listening to the new parts isn't available yet.",
  open_as_copy: "Opening a moment as a copy isn't available yet.",
  send: "Sending to a friend isn't available yet.",
};

export function disabledReason(actions: Actions, key: ActionKey): string | null {
  return actions[key] ? null : REASONS[key];
}
