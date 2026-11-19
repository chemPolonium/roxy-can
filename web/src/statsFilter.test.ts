import { describe, expect, it } from "vitest";
import { emptyFilterGroup, newFilterRule } from "./filterTree";
import { matchesStatsFilter } from "./statsFilter";
import type { StatsRow } from "./types";

const row: StatsRow = {
  label: "EngineStatus",
  bus: "CAN1",
  count: 120,
  minMs: "49.8",
  avgMs: "50.0",
  maxMs: "50.4",
  jitterMs: "0.3",
  share: "12.5%",
  len: 8,
};

describe("stats filter", () => {
  it("filters labels, buses and numeric aggregates", () => {
    const rule = (field: string, op: "contains" | "gte", value: string) => ({
      ...newFilterRule(field, op === "gte"),
      field,
      op,
      value,
    });
    expect(matchesStatsFilter(rule("label", "contains", "engine"), row)).toBe(
      true,
    );
    expect(matchesStatsFilter(rule("bus", "contains", "CAN2"), row)).toBe(false);
    expect(matchesStatsFilter(rule("count", "gte", "100"), row)).toBe(true);
    expect(matchesStatsFilter(rule("count", "gte", "200"), row)).toBe(false);
  });

  it("combines groups", () => {
    const filter = emptyFilterGroup();
    filter.children = [
      { ...newFilterRule("label"), field: "label", value: "Engine" },
      { ...newFilterRule("bus"), field: "bus", value: "CAN1" },
    ];
    expect(matchesStatsFilter(filter, row)).toBe(true);
  });
});
