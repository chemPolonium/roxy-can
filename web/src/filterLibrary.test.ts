import { describe, expect, it } from "vitest";
import {
  cloneFilter,
  normalizeFilterLibrary,
  savedFilterId,
} from "./filterLibrary";
import { emptyFilterGroup, newFilterRule } from "./filterTree";

describe("saved filter library", () => {
  it("normalizes valid entries and drops bad scopes/fields", () => {
    const tree = emptyFilterGroup();
    tree.children = [
      { ...newFilterRule("bus"), field: "bus", value: "CAN1" },
    ];
    const entries = normalizeFilterLibrary([
      { id: "a", name: "CAN1", scope: "signal", filter: tree },
      { id: "b", name: "Bad", scope: "message", filter: tree },
      { id: "c", name: "Nope", scope: "unknown", filter: tree },
      { id: "d", name: "", scope: "trace", filter: tree },
    ]);
    expect(entries).toHaveLength(1);
    expect(entries[0]).toMatchObject({
      id: "a",
      name: "CAN1",
      scope: "signal",
    });
    expect(entries[0].filter.children).toHaveLength(1);
  });

  it("clones trees and generates distinct ids", () => {
    const tree = emptyFilterGroup();
    tree.children = [{ ...newFilterRule("name"), value: "Engine" }];
    const clone = cloneFilter(tree);
    (clone.children[0] as { value: string }).value = "Other";
    expect(tree.children[0]).toMatchObject({ value: "Engine" });
    expect(savedFilterId()).not.toBe(savedFilterId());
  });
});
