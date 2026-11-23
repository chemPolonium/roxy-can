import { describe, expect, it } from "vitest";
import { bandAt, bandIndex, bandsFromCuts, validateBands, type Band } from "./bands";

describe("bands", () => {
  it("matches < and <= thresholds with an open final band", () => {
    const bands: Band[] = [
      { upper: 1, inclusive: false, text: "below one" },
      { upper: 1, inclusive: true, text: "one" },
      { upper: 2, inclusive: false, text: "below two" },
      { upper: null, inclusive: false, text: "rest" },
    ];
    expect(bandAt(bands, 0.5)?.text).toBe("below one");
    expect(bandAt(bands, 1)?.text).toBe("one");
    expect(bandAt(bands, 1.5)?.text).toBe("below two");
    expect(bandAt(bands, 2)?.text).toBe("rest");
    expect(bandIndex(bands, null)).toBeNull();
  });

  it("converts legacy cuts into left-closed bands", () => {
    const bands = bandsFromCuts([10, 20], [1, 2, 3], ["low", "mid", "high"]);
    expect(bands).toEqual([
      { upper: 10, inclusive: false, text: "low", color: 1 },
      { upper: 20, inclusive: false, text: "mid", color: 2 },
      { upper: null, inclusive: false, text: "high", color: 3 },
    ]);
    expect(bandAt(bands, 10)?.text).toBe("mid");
  });

  it("rejects missing or multiple open bands", () => {
    expect(validateBands([{ upper: 1, inclusive: false }])).not.toBe("");
    expect(
      validateBands([
        { upper: null, inclusive: false },
        { upper: 1, inclusive: false },
      ]),
    ).not.toBe("");
  });
});
