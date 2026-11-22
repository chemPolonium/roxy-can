import { describe, expect, it } from "vitest";
import {
  addCutBand,
  cutError,
  cutIsInvalid,
  fitBandColors,
  nextCut,
  rangeText,
  removeCutBand,
} from "./bandCuts";

describe("band cut helpers", () => {
  it("labels every band around ascending cuts", () => {
    expect(rangeText([], 0)).toBe("all values");
    expect(rangeText([10], 0)).toBe("< 10");
    expect(rangeText([10], 1)).toBe(">= 10");
    expect(rangeText([10, 20], 1)).toBe("10 .. 20");
  });

  it("rejects non-numeric and out-of-order cuts", () => {
    expect(cutError([0, 10])).toBe("");
    expect(cutError([0, Number.NaN])).toBe("cuts must be numbers");
    expect(cutError([10, 10])).toBe("cuts must be strictly ascending");
    expect(cutError([10, 5])).toBe("cuts must be strictly ascending");
  });

  it("marks the cut that breaks the order", () => {
    expect(cutIsInvalid([10, 20], 0)).toBe(false);
    expect(cutIsInvalid([10, 20], 1)).toBe(false);
    expect(cutIsInvalid([10, 5], 1)).toBe(true);
    expect(cutIsInvalid([10, Number.NaN], 1)).toBe(true);
  });

  it("suggests a cut after the last valid value", () => {
    expect(nextCut([])).toBe("0");
    expect(nextCut(["10", "20"])).toBe("21");
    expect(nextCut(["10", "oops"])).toBe("0");
  });

  it("keeps one palette slot per band when cuts are added or removed", () => {
    expect(fitBandColors([], 3)).toEqual([0, 0, 0]);
    expect(fitBandColors([2], 3)).toEqual([2, 2, 2]);
    expect(addCutBand([], [], 1)).toEqual({ cuts: [0], colors: [0, 1] });
    expect(addCutBand([10], [2, 3], 4)).toEqual({
      cuts: [10, 11],
      colors: [2, 3, 4],
    });
    expect(removeCutBand([10, 20], [2, 3, 4], 0)).toEqual({
      cuts: [20],
      colors: [2, 4],
    });
  });
});
