import { describe, expect, it } from "vitest";
import { ACTION_KEYS, disabledReason } from "./actions";
import type { Actions } from "./story";

describe("disabledReason", () => {
  it("is null when the action is enabled — nothing to explain", () => {
    const actions: Actions = { listen: true, open_as_copy: false, send: false };
    expect(disabledReason(actions, "listen")).toBeNull();
  });

  it("gives a short, plain, non-empty reason for every disabled action", () => {
    const allOff: Actions = { listen: false, open_as_copy: false, send: false };
    for (const key of ACTION_KEYS) {
      const reason = disabledReason(allOff, key);
      expect(reason).toBeTruthy();
      expect(reason).not.toMatch(/[<>{}]/);
      expect((reason as string).length).toBeLessThan(80);
    }
  });

  it("never hides an action — it always has a reason to show instead", () => {
    // The contract (this lane's brief, "Laws"): disabled, never hidden,
    // never faked. A reason string must exist for every action key.
    const allOff: Actions = { listen: false, open_as_copy: false, send: false };
    expect(ACTION_KEYS.every((k) => typeof disabledReason(allOff, k) === "string")).toBe(true);
  });
});
