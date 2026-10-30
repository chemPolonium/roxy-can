import { describe, expect, it } from "vitest";
import {
  freeCursorTime,
  nearestSampleTime,
  nextSampleTime,
  prevSampleTime,
} from "./components/SignalChart";

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

describe("sample snapping", () => {
  const a: [number, number][] = [
    [0, 10],
    [1, 20],
    [2, 30],
  ];
  const b: [number, number][] = [
    [0.5, 1],
    [1.5, 2],
  ];

  it("snaps only when a sample is close enough", () => {
    expect(nearestSampleTime([a], 1.04, 0.05)).toBe(1);
    expect(nearestSampleTime([a], 1.2, 0.05)).toBeNull();
    expect(nearestSampleTime([a, b], 0.6, 0.2)).toBe(0.5);
  });

  it("walks the union of sample times in both directions", () => {
    expect(nextSampleTime([a, b], 1)).toBe(1.5);
    expect(prevSampleTime([a, b], 1)).toBe(0.5);
    expect(nextSampleTime([a, b], 2)).toBeNull();
    expect(prevSampleTime([a], 0)).toBeNull();
  });
});
