import { describe, expect, it } from "vitest";
import {
  bandAt,
  bandIndex,
  bandSetMatchesQuery,
  parseBandBound,
  validateBands,
  type Band,
} from "./bands";

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

  it("matches band sets by name or id, ignoring case and padding", () => {
    expect(bandSetMatchesQuery("Engine", "bands1", "")).toBe(true);
    expect(bandSetMatchesQuery("Engine", "bands1", "  ")).toBe(true);
    expect(bandSetMatchesQuery("Engine", "bands1", "eng")).toBe(true);
    expect(bandSetMatchesQuery("Engine", "bands1", "BANDS1")).toBe(true);
    expect(bandSetMatchesQuery("Engine", "bands1", "brake")).toBe(false);
  });

  it("keeps blank or half-typed bounds from becoming open bands", () => {
    expect(parseBandBound("30")).toBe(30);
    expect(parseBandBound("-5.5")).toBe(-5.5);
    expect(parseBandBound("")).toBeNull();
    expect(parseBandBound("   ")).toBeNull();
    expect(parseBandBound("1e")).toBeNull();
    expect(parseBandBound("abc")).toBeNull();
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
