import { describe, expect, it } from "vitest";
import {
  clampColumnWidth,
  defaultColumnWidths,
  parseColumnWidths,
  type ColumnWidthDef,
} from "./tableColumns";

const DEFS: ColumnWidthDef[] = [
  { id: "bus", width: 46, min: 36, max: 120 },
  { id: "name", width: 180, min: 90, max: 420 },
  { id: "data", width: 0, min: 0, max: 0, flex: true },
];

describe("table column widths", () => {
  it("clamps a dragged width to the column bounds", () => {
    expect(clampColumnWidth(10, 36, 120)).toBe(36);
    expect(clampColumnWidth(200, 36, 120)).toBe(120);
    expect(clampColumnWidth(60, 36, 120)).toBe(60);
  });

  it("clamps stale stored widths instead of rejecting them", () => {
    expect(parseColumnWidths({ bus: 999, name: 12 }, DEFS)).toEqual({
      bus: 120,
      name: 90,
    });
  });

  it("ignores unknown ids and keeps defaults for junk input", () => {
    expect(parseColumnWidths({ nope: 10 }, DEFS)).toEqual({
      bus: 46,
      name: 180,
    });
    expect(parseColumnWidths(null, DEFS)).toBeNull();
    expect(defaultColumnWidths(DEFS)).toEqual({ bus: 46, name: 180 });
  });
});
