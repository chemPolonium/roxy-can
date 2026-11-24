import { describe, expect, it } from "vitest";
import { bandAt, bandIndex, validateBands, type Band } from "./bands";

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
