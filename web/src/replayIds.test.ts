import { describe, expect, it } from "vitest";
import { formatReplayIds, parseReplayIds } from "./replayIds";

describe("replay id filters", () => {
  it("treats an empty field as no filter", () => {
    expect(parseReplayIds("  ")).toEqual({ ids: [], error: "" });
  });

  it("parses standard, extended and bare hex ids", () => {
    expect(parseReplayIds("0x100, 0x1ABCDEFx 3F4").ids).toEqual([
      [0x100, false],
      [0x1abcdef, true],
      [0x3f4, false],
    ]);
    expect(parseReplayIds("3F4x").ids).toEqual([[0x3f4, true]]);
  });

  it("extends ids above the 11-bit range", () => {
    expect(parseReplayIds("0x800").ids).toEqual([[0x800, true]]);
  });

  it("formats a round-trippable list", () => {
    const text = formatReplayIds([
      [0x100, false],
      [0x1abcdef, true],
    ]);
    expect(text).toBe("0x100, 0x1ABCDEFx");
    expect(parseReplayIds(text).ids).toEqual([
      [0x100, false],
      [0x1abcdef, true],
    ]);
  });

  it("rejects malformed tokens and duplicates are folded", () => {
    expect(parseReplayIds("0xZZ").error).toContain("invalid id");
    expect(parseReplayIds("0x100 0x100").ids).toEqual([[0x100, false]]);
  });
});
