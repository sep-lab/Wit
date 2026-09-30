import type { TrackHeat } from "./story";

/**
 * Build the dense `[track][moment]` heat matrix the track heat strip
 * draws, from each moment's sparse `TrackHeat[]` (PLAN-V2 Phase D: "track
 * heat strip ... via track_heat"). A cell Wit didn't mention for that
 * moment is 0 — no change to show, not "unknown".
 */
export function buildHeatMatrix(
  trackCount: number,
  moments: readonly { track_heat: readonly TrackHeat[] }[]
): number[][] {
  const matrix: number[][] = Array.from({ length: trackCount }, () =>
    new Array(moments.length).fill(0)
  );
  moments.forEach((m, col) => {
    for (const cell of m.track_heat) {
      if (cell.track >= 0 && cell.track < trackCount) {
        matrix[cell.track][col] = cell.level;
      }
    }
  });
  return matrix;
}

/**
 * The CSS variable for a heat level (0-3, clamped and rounded), matching
 * the approved mockup's tint scale (transparent, light, mid, dark green —
 * `--heat-0` … `--heat-3` in `app.css`). Darker means more changed.
 */
export function heatVar(level: number): string {
  const clamped = Math.max(0, Math.min(3, Math.round(level)));
  return `var(--heat-${clamped})`;
}
