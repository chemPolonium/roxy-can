import { describe, expect, it } from "vitest";
import {
  chartYRange,
  extendYMemory,
  type YMemory,
} from "./chartScale";

describe("chart Y scaling", () => {
  it("auto follows the visible window", () => {
    expect(chartYRange("auto", { min: 1, max: 2 }, undefined, undefined)).toEqual({
      min: 1,
      max: 2,
    });
  });

  it("memory keeps old extremes and only reads new samples", () => {
    let memory: YMemory | undefined;
    memory = extendYMemory(memory, [
      [0, 10],
      [1, 20],
    ]);
    expect(memory).toEqual({ min: 10, max: 20, lastT: 1 });
    memory = extendYMemory(memory, [
      [1, 99],
      [2, 15],
      [3, 5],
    ]);
    expect(memory).toEqual({ min: 5, max: 20, lastT: 3 });
    expect(chartYRange("memory", { min: 15, max: 16 }, undefined, memory)).toEqual({
      min: 5,
      max: 20,
    });
  });

  it("dbc uses the declared range and falls back to auto", () => {
    const declared = { min: 0, max: 100 };
    expect(chartYRange("dbc", { min: 10, max: 20 }, declared, undefined)).toEqual(
      declared,
    );
    expect(
      chartYRange("dbc", { min: 10, max: 20 }, { min: 5, max: 5 }, undefined),
    ).toEqual({ min: 10, max: 20 });
    expect(chartYRange("dbc", { min: 10, max: 20 }, undefined, undefined)).toEqual({
      min: 10,
      max: 20,
    });
  });
});
