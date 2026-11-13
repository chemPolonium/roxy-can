import { describe, expect, it } from "vitest";
import { fittingTraceColumns } from "./traceLayout";


describe("fittingTraceColumns", () => {
  it("keeps the anchor and data columns when a slot is narrow", () => {
    const narrow = fittingTraceColumns(
      { widths: {}, hidden: [] },
      260,
    ).map((column) => column.id);
    expect(narrow).toContain("time");
    expect(narrow).toContain("id");
    expect(narrow).toContain("data");
    expect(narrow).not.toContain("dir");
    expect(narrow).not.toContain("len");
  });

  it("uses the rendered widths, so a squeezed data column drops lower-value ones", () => {
    // A 1024-wide Quad slot: the default widths plus a usable Data column do
    // not fit, so Dir, Len and Bus go before Data is squeezed.
    const quad = fittingTraceColumns({ widths: {}, hidden: [] }, 496).map(
      (column) => column.id,
    );
    expect(quad).not.toContain("dir");
    expect(quad).not.toContain("len");
    expect(quad).not.toContain("bus");
    expect(quad).toContain("name");
    expect(quad).toContain("data");
  });

  it("counts resized columns when deciding what fits", () => {
    const resized = fittingTraceColumns(
      { widths: { name: 300 }, hidden: [] },
      600,
    ).map((column) => column.id);
    expect(resized).not.toContain("name");
    expect(resized).toContain("data");
  });

  it("keeps the user's hidden set and restores columns when there is room", () => {
    const wide = fittingTraceColumns(
      { widths: {}, hidden: ["dir"] },
      1200,
    ).map((column) => column.id);
    expect(wide).not.toContain("dir");
    expect(wide).toContain("name");
    expect(wide).toContain("bus");
  });
});
