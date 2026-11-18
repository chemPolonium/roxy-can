import { describe, expect, it } from "vitest";
import { filterSignalRows } from "./signalRows";

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
});
