import { describe, expect, it } from "vitest";
import { filterSignalRows, rowPathWithoutName } from "./signalRows";
import { emptySignalFilter, newSignalRule } from "./signalFilter";

const ROWS = [
  {
    name: "EngineSpeed",
    bus: "CAN1",
    unit: "rpm",
    path: "EngineStatus / EngineSpeed",
  },
  {
    name: "Speed",
    bus: "CanA",
    unit: "km/h",
    path: "A / PDU_A / Speed",
  },
  {
    name: "ThrottlePos",
    bus: "CAN1",
    unit: "%",
    path: "EngineStatus / ThrottlePos",
    label: "Throttle",
  },
];

describe("signal row filtering", () => {
  it("returns the original rows for an empty query", () => {
    expect(filterSignalRows(ROWS, "")).toBe(ROWS);
    expect(filterSignalRows(ROWS, "   ")).toBe(ROWS);
  });

  it("matches name, label, bus, unit and path", () => {
    expect(filterSignalRows(ROWS, "enginespeed")).toHaveLength(1);
    expect(filterSignalRows(ROWS, "throttle")).toHaveLength(1);
    expect(filterSignalRows(ROWS, "cana")).toHaveLength(1);
    expect(filterSignalRows(ROWS, "rpm")).toHaveLength(1);
    expect(filterSignalRows(ROWS, "pdu_a")).toHaveLength(1);
  });

  it("is case-insensitive and reports no match", () => {
    expect(filterSignalRows(ROWS, "ENGINESTATUS")).toHaveLength(2);
    expect(filterSignalRows(ROWS, "not-there")).toEqual([]);
  });

  it("drops the trailing signal name from a path label", () => {
    expect(rowPathWithoutName("EngineStatus / EngineSpeed", "EngineSpeed")).toBe(
      "EngineStatus",
    );
    expect(rowPathWithoutName("A / PDU_A / Speed", "Speed")).toBe("A / PDU_A");
    expect(rowPathWithoutName("Other / Frame", "Speed")).toBe("Other / Frame");
  });

  it("ands the quick query with the advanced filter", () => {
    const filter = emptySignalFilter();
    filter.children = [
      { ...newSignalRule(), field: "bus", op: "equals", value: "CAN1" },
    ];
    expect(filterSignalRows(ROWS, "", filter)).toHaveLength(2);
    expect(filterSignalRows(ROWS, "throttle", filter)).toHaveLength(1);
    expect(filterSignalRows(ROWS, "speed", filter)).toHaveLength(1);
  });
});
