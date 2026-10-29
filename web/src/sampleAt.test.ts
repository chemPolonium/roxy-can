import { describe, expect, it } from "vitest";
import { sampleAt } from "./components/SignalChart";

describe("cursor sampling", () => {
  const points: [number, number][] = [
    [0, 10],
    [1, 20],
    [2, 30],
  ];

  it("returns the held sample, never an interpolated value", () => {
    expect(sampleAt(points, 1)).toBe(20);
    expect(sampleAt(points, 1.5)).toBe(20);
    expect(sampleAt(points, 1.999)).toBe(20);
    expect(sampleAt(points, 2)).toBe(30);
  });

  it("clamps outside the sampled range", () => {
    expect(sampleAt(points, -1)).toBe(10);
    expect(sampleAt(points, 3)).toBe(30);
    expect(sampleAt([], 1)).toBeNull();
  });
});
