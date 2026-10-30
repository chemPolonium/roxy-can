import { describe, expect, it } from "vitest";
import { fitText, niceAxis, niceTicks } from "./components/SignalChart";

describe("value axis ticks", () => {
  it("rounds to readable steps inside the range", () => {
    expect(niceTicks(180, 7620, 3)).toEqual([2000, 4000, 6000]);
    expect(niceTicks(-90, 110, 3)).toEqual([0, 100]);
    expect(niceTicks(-10, 110, 5)).toEqual([0, 25, 50, 75, 100]);
  });

  it("keeps the zero line when the range is bipolar", () => {
    expect(niceTicks(-762, 8382, 3)).toContain(0);
  });

  it("prefers an extra tick over falling back to the padded bounds", () => {
    expect(niceTicks(180, 7620, 2)).toEqual([2000, 4000, 6000]);
  });

  it("drops a cramped step when the lane is short", () => {
    expect(niceTicks(-321, 1822, 3, 60)).toEqual([0, 1000]);
    expect(niceTicks(-321, 1822, 3, 30)).toEqual([0, 1000]);
  });

  it("keeps a readable step when the lane has room", () => {
    expect(niceTicks(-321, 1822, 4, 96)).toEqual([0, 500, 1000, 1500]);
  });

  it("returns nothing for a flat range", () => {
    expect(niceTicks(5, 5, 3)).toEqual([]);
  });
});

describe("snapped axis bounds", () => {
  it("spreads the labels across the lane", () => {
    expect(niceAxis(0, 7620, 3, 60)).toEqual({
      low: 0,
      high: 10000,
      ticks: [0, 5000, 10000],
    });
    expect(niceAxis(0, 250, 3, 60)).toEqual({
      low: 0,
      high: 250,
      ticks: [0, 250],
    });
  });

  it("adds labels as the lane grows", () => {
    expect(niceAxis(0, 250, 4, 120)).toEqual({
      low: 0,
      high: 300,
      ticks: [0, 100, 200, 300],
    });
  });

  it("keeps the padded range when snapping would waste the lane", () => {
    expect(niceAxis(-10, 110, 3, 60)).toBeNull();
    expect(niceAxis(0, 1100, 3, 60)).toBeNull();
  });
});

describe("gutter labels", () => {
  const ctx = {
    measureText: (text: string) => ({ width: text.length * 10 }),
  } as unknown as CanvasRenderingContext2D;

  it("keeps short names unchanged", () => {
    expect(fitText(ctx, "EngSpeed", 91)).toBe("EngSpeed");
  });

  it("ellipsizes a name that would reach into the plot", () => {
    expect(fitText(ctx, "WheelSpeedFL", 60)).toBe("Wheel…");
  });
});
