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
  listen: "No audio to play for this moment yet.",
  open_as_copy: "Opening a moment as a copy isn't available yet.",
  send: "Sending to a friend isn't available yet.",
};

export function disabledReason(actions: Actions, key: ActionKey): string | null {
  return actions[key] ? null : REASONS[key];
}
