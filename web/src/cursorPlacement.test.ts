import { describe, expect, it } from "vitest";
import { freeCursorTime } from "./components/SignalChart";

describe("cursor placement", () => {
  it("starts at the middle of the covered range", () => {
    expect(freeCursorTime(0, 10, [], 0.01)).toBe(5);
    expect(freeCursorTime(-4, -1, [], 0.01)).toBe(-2.5);
  });

  it("nudges away from an existing cursor", () => {
    expect(freeCursorTime(0, 10, [5], 0.01)).toBe(5.5);
    expect(freeCursorTime(0, 10, [5, 5.5], 0.01)).toBe(4.5);
  });

  it("returns nothing for an empty or saturated range", () => {
    expect(freeCursorTime(5, 5, [], 0.01)).toBeNull();
    expect(
      freeCursorTime(
        0,
        1,
        [0.5, 0.55, 0.45, 0.6, 0.4, 0.65, 0.35, 0.7, 0.3],
        0,
      ),
    ).toBeNull();
  });
});
