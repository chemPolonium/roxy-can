import { describe, expect, it } from "vitest";
import { cutError, cutIsInvalid, nextCut, rangeText } from "./bandCuts";

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
});
