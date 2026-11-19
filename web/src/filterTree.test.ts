import { describe, expect, it } from "vitest";
import {
  emptyFilterGroup,
  filterActive,
  filterRuleCount,
  filterSummary,
  matchesFilter,
  normalizeFilter,
  pruneFilter,
  type FilterFieldSpec,
} from "./filterTree";

const fields: FilterFieldSpec[] = [
  { key: "name", label: "Name" },
  { key: "time", label: "Time", numeric: true },
];

describe("filter tree normalization", () => {
  it("keeps valid groups/rules and a numeric field", () => {
    const filter = normalizeFilter(
      {
        kind: "group",
        op: "or",
        negate: true,
        children: [
          { kind: "rule", field: "name", op: "contains", value: "Engine" },
          { kind: "rule", field: "time", op: "gte", value: "1.5" },
        ],
      },
      fields,
    );
    expect(filter).toMatchObject({ kind: "group", op: "or", negate: true });
    expect(filter.children).toHaveLength(2);
    expect(matchesFilter(filter, (field) => (field === "name" ? "Engine" : "2"))).toBe(
      false,
    );
  });

  it("drops unknown fields, bad operators and over-deep non-groups", () => {
    const filter = normalizeFilter(
      {
        kind: "group",
        op: "and",
        children: [
          { kind: "rule", field: "nope", op: "contains", value: "x" },
          { kind: "rule", field: "name", op: "gte", value: "1" },
          { kind: "rule", field: "time", op: "contains", value: "x" },
          { kind: "rule", field: "name", op: "equals", value: "ok" },
        ],
      },
      fields,
    );
    expect(filter.children).toEqual([
      { kind: "rule", field: "name", op: "equals", value: "ok", numeric: undefined },
    ]);
    expect(normalizeFilter({ kind: "rule", field: "name", op: "equals", value: "x" }, fields)).toEqual(
      emptyFilterGroup(),
    );
  });

  it("summarizes active conditions for hover help", () => {
    const filter = normalizeFilter(
      {
        kind: "group",
        op: "and",
        children: [
          { kind: "rule", field: "name", op: "contains", value: "Engine" },
          {
            kind: "group",
            op: "or",
            negate: true,
            children: [
              { kind: "rule", field: "time", op: "gte", value: "1" },
            ],
          },
        ],
      },
      fields,
    );
    expect(filterSummary(filter, fields)).toBe(
      "(Name contains Engine AND NOT (Time ≥ 1))",
    );
  });

  it("ignores blank rules and counts only active conditions", () => {
    const filter = emptyFilterGroup();
    filter.children = [
      { kind: "rule", field: "name", op: "contains", value: "" },
      {
        kind: "group",
        op: "or",
        children: [
          { kind: "rule", field: "name", op: "equals", value: "Engine" },
          { kind: "rule", field: "time", op: "gte", value: "2" },
        ],
      },
    ];
    expect(filterActive(filter)).toBe(true);
    expect(filterRuleCount(filter)).toBe(2);
    expect(pruneFilter(filter)).not.toBeNull();

    filter.children = [
      { kind: "rule", field: "name", op: "contains", value: "  " },
    ];
    expect(filterActive(filter)).toBe(false);
    expect(filterRuleCount(filter)).toBe(0);
    expect(pruneFilter(filter)).toBeNull();
  });
});
