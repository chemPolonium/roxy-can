import { describe, expect, it } from "vitest";
import {
  resolveDerivedSources,
  validateDerivedExpression,
  type DerivedReference,
} from "./derived";

const refs: DerivedReference[] = [
  { key: "0:256:EngineSpeed", name: "EngineSpeed", bus: "CAN1" },
  { key: "1:512:EngineSpeed", name: "EngineSpeed", bus: "CAN2" },
  { key: "derived:avg", name: "SpeedAvg", bus: "derived" },
];

describe("global derived references", () => {
  it("resolves catalog and computed channels without the chart selection", () => {
    expect(validateDerivedExpression("[CAN2:EngineSpeed] - [SpeedAvg]", refs)).toBeNull();
    expect(
      resolveDerivedSources("[CAN2:EngineSpeed] - [SpeedAvg]", refs),
    ).toEqual([
      { name: "CAN2:EngineSpeed", key: "1:512:EngineSpeed" },
      { name: "SpeedAvg", key: "derived:avg" },
    ]);
  });

  it("still refuses an ambiguous bare name", () => {
    expect(validateDerivedExpression("[EngineSpeed] * 2", refs)).toBe(
      "[EngineSpeed] is ambiguous; use [Bus:Message:Signal]",
    );
    expect(validateDerivedExpression("[derived:SpeedAvg]", refs)).toBeNull();
  });

  it("uses the frame qualifier when one bus repeats the name", () => {
    const sameBus: DerivedReference[] = [
      {
        key: "1:199:Sig",
        name: "Sig",
        bus: "CAN2",
        frame: "WheelInfoIEEE",
        frameId: "0x0C7",
      },
      {
        key: "1:200:Sig",
        name: "Sig",
        bus: "CAN2",
        frame: "WheelInfo",
        frameId: "0x0C8",
      },
    ];
    expect(validateDerivedExpression("[CAN2:WheelInfoIEEE:Sig]", sameBus)).toBeNull();
    expect(validateDerivedExpression("[CAN2:0x0C7:Sig]", sameBus)).toBeNull();
    expect(validateDerivedExpression("[CAN2:Sig]", sameBus)).toBe(
      "[CAN2:Sig] is ambiguous; use [Bus:Message:Signal]",
    );
    expect(
      resolveDerivedSources("[CAN2:WheelInfo:Sig]", sameBus),
    ).toEqual([{ name: "CAN2:WheelInfo:Sig", key: "1:200:Sig" }]);
  });
});
