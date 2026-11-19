import { describe, expect, it } from "vitest";
import { emptyFilterGroup, newFilterRule } from "./filterTree";
import { matchesMessageFilter } from "./messageFilter";
import type { Message } from "./types";

const row: Message = {
  key: "can:0:100",
  kind: "CAN",
  addr: "0x100",
  bus: 0,
  busName: "CAN1",
  id: 0x100,
  extended: false,
  name: "EngineStatus",
  count: 1,
  cycleMs: 10,
  len: 8,
  data: "11223344",
  signalCount: 2,
};

const rule = (
  field: "name" | "id" | "kind" | "data",
  value: string,
  op: "contains" | "equals" | "notContains" = "contains",
) => ({ ...newFilterRule(field), field, op, value });

describe("message filter", () => {
  it("filters name, id, kind and payload", () => {
    expect(matchesMessageFilter(rule("name", "engine"), row)).toBe(true);
    expect(matchesMessageFilter(rule("id", "0x100", "equals"), row)).toBe(true);
    expect(matchesMessageFilter(rule("kind", "CAN", "equals"), row)).toBe(true);
    expect(matchesMessageFilter(rule("data", "3344"), row)).toBe(true);
    expect(matchesMessageFilter(rule("name", "engine", "notContains"), row)).toBe(
      false,
    );
  });

  it("combines groups and ignores blank rules", () => {
    const filter = emptyFilterGroup();
    filter.children = [
      rule("name", "engine"),
      { ...newFilterRule("id"), field: "id", value: "" },
    ];
    expect(matchesMessageFilter(filter, row)).toBe(true);
    filter.children.push(rule("kind", "FR", "equals"));
    expect(matchesMessageFilter(filter, row)).toBe(false);
  });
});
