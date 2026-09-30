import { describe, expect, it } from "vitest";
import { buildHeatMatrix, heatVar } from "./heat";

describe("buildHeatMatrix", () => {
  it("builds a dense [track][moment] matrix from sparse per-moment rows", () => {
    const moments = [
      { track_heat: [{ track: 0, level: 2 }] },
      { track_heat: [] },
      {
        track_heat: [
          { track: 1, level: 3 },
          { track: 0, level: 1 },
        ],
      },
    ];
    expect(buildHeatMatrix(2, moments)).toEqual([
      [2, 0, 1],
      [0, 0, 3],
    ]);
  });

  it("is all-zero for a story with no track lanes", () => {
    expect(buildHeatMatrix(0, [{ track_heat: [] }, { track_heat: [] }])).toEqual([]);
  });

  it("ignores an out-of-range track index rather than throwing", () => {
    expect(buildHeatMatrix(1, [{ track_heat: [{ track: 5, level: 3 }] }])).toEqual([[0]]);
  });
});

describe("heatVar", () => {
  it("clamps to the 0-3 tint scale", () => {
    expect(heatVar(-1)).toBe("var(--heat-0)");
    expect(heatVar(0)).toBe("var(--heat-0)");
    expect(heatVar(2)).toBe("var(--heat-2)");
    expect(heatVar(3)).toBe("var(--heat-3)");
    expect(heatVar(4)).toBe("var(--heat-3)");
  });

  it("rounds a fractional level", () => {
    expect(heatVar(1.6)).toBe("var(--heat-2)");
  });
});
