import { describe, expect, it } from "vitest";
import { filterGroupedOptions, type GroupedOption } from "./groupedOptions";

const OPTIONS: GroupedOption[] = [
  {
    key: "engine-rpm",
    group: "EngineStatus",
    groupMeta: "0x100",
    label: "EngineSpeed",
    meta: "rpm · 0..8000",
  },
  {
    key: "engine-temp",
    group: "EngineStatus",
    groupMeta: "0x100",
    label: "EngineTemp",
    meta: "degC · -40..215",
  },
  {
    key: "brake-pressure",
    group: "BrakeInfo",
    groupMeta: "0x320",
    label: "BrakePressure",
    meta: "bar · 0..100",
    search: "hydraulic",
  },
];

describe("grouped option filtering", () => {
  it("keeps catalog order and groups by message", () => {
    const groups = filterGroupedOptions(OPTIONS, "");
    expect(groups.map((group) => group.label)).toEqual([
      "EngineStatus",
      "BrakeInfo",
    ]);
    expect(groups[0].options).toHaveLength(2);
    expect(groups[0].meta).toBe("0x100");
  });

  it("matches a message header and keeps all of its signals", () => {
    const groups = filterGroupedOptions(OPTIONS, "enginestatus");
    expect(groups).toHaveLength(1);
    expect(groups[0].options.map((option) => option.key)).toEqual([
      "engine-rpm",
      "engine-temp",
    ]);
  });

  it("matches id, unit and hidden search text", () => {
    expect(filterGroupedOptions(OPTIONS, "0x320")[0].options).toHaveLength(1);
    expect(filterGroupedOptions(OPTIONS, "rpm")[0].options).toHaveLength(1);
    expect(filterGroupedOptions(OPTIONS, "hydraulic")[0].options).toHaveLength(
      1,
    );
  });

  it("returns no groups when nothing matches", () => {
    expect(filterGroupedOptions(OPTIONS, "no-such-signal")).toEqual([]);
  });

  it("trims and lowercases the query", () => {
    expect(filterGroupedOptions(OPTIONS, "  BRAKEPRESSURE  ")).toHaveLength(1);
  });
});
