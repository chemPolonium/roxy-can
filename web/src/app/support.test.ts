import { describe, expect, it } from "vitest";
import { panelMeta } from "./support";

describe("panel header counts", () => {
  it("uses one noun form for zero, one and many", () => {
    expect(panelMeta("message", 0)).toBe("0 messages");
    expect(panelMeta("message", 1)).toBe("1 message");
    expect(panelMeta("message", 9)).toBe("9 messages");
  });

  it("shows filtered totals as count / total with thousands separators", () => {
    expect(panelMeta("frame", 7076, 7076)).toBe("7,076 / 7,076 frames");
    expect(panelMeta("lane", 3, 8)).toBe("3 / 8 lanes");
    expect(panelMeta("bus", 2, undefined, "buses")).toBe("2 buses");
  });
});
