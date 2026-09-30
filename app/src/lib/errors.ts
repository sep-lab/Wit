import { EMPTY_STATES } from "./chrome";
import { NotAvailableError } from "./ipc";

/**
 * A plain message safe to show a musician for any error this app can hit
 * — never a raw `Error.message` (review round 1, non-blocking #7: a raw
 * exception string is not something Wit's own voice ever wrote, and may
 * leak a stack frame or a path). A [`NotAvailableError`] already carries
 * an honest, UI-authored message ("... isn't available yet."), so that
 * one is shown as-is; anything else collapses to one generic, honest
 * fallback.
 */
export function userFacingErrorMessage(err: unknown): string {
  if (err instanceof NotAvailableError) {
    return err.message;
  }
  return EMPTY_STATES.genericError;
}
