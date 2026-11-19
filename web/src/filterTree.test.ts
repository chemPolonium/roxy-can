import { describe, expect, it } from "vitest";
import {
  emptyFilterGroup,
  filterSummary,
  matchesFilter,
  normalizeFilter,
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
});
