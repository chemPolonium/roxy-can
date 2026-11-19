import { describe, expect, it } from "vitest";
import {
  emptySignalFilter,
  matchesSignalFilter,
  newSignalGroup,
  newSignalRule,
  pruneSignalFilter,
  signalFilterActive,
  signalFilterRuleCount,
  type SignalFilterGroup,
} from "./signalFilter";

const row = {
  name: "EngineSpeed",
  label: "Engine speed",
  bus: "CAN1",
  frame: "EngineStatus",
  path: "EngineStatus / EngineSpeed",
  unit: "rpm",
};

const rule = (
  field: "signal" | "label" | "bus" | "frame" | "path" | "unit",
  value: string,
  op: "contains" | "notContains" | "equals" | "notEquals" = "contains",
) => ({ ...newSignalRule(), field, op, value });

describe("signal filter tree", () => {
  it("treats blank rules and empty groups as no filter", () => {
    const empty = emptySignalFilter();
    expect(signalFilterActive(empty)).toBe(false);
    expect(pruneSignalFilter(empty)).toBeNull();
    expect(matchesSignalFilter(empty, row)).toBe(true);

    const blank = { ...empty, children: [newSignalRule()] };
    expect(signalFilterActive(blank)).toBe(false);
    expect(matchesSignalFilter(blank, row)).toBe(true);
  });

  it("ANDs, ORs and negates groups", () => {
    const can1 = rule("bus", "CAN1", "equals");
    const badId = rule("signal", "nope");
    const and: SignalFilterGroup = {
      ...newSignalGroup(),
      children: [can1, rule("unit", "rpm", "equals")],
    };
    expect(matchesSignalFilter(and, row)).toBe(true);
    expect(matchesSignalFilter({ ...and, children: [can1, badId] }, row)).toBe(
      false,
    );

    const or: SignalFilterGroup = {
      ...newSignalGroup(),
      op: "or",
      children: [badId, rule("frame", "EngineStatus")],
    };
    expect(matchesSignalFilter(or, row)).toBe(true);
    expect(matchesSignalFilter({ ...or, negate: true }, row)).toBe(false);
  });

  it("supports contains/equals and their negations", () => {
    expect(matchesSignalFilter(rule("signal", "speed"), row)).toBe(true);
    expect(matchesSignalFilter(rule("label", "ENGINE"), row)).toBe(true);
    expect(matchesSignalFilter(rule("unit", "rpm", "equals"), row)).toBe(true);
    expect(matchesSignalFilter(rule("unit", "rpm", "notEquals"), row)).toBe(
      false,
    );
    expect(matchesSignalFilter(rule("bus", "CAN2", "notContains"), row)).toBe(
      true,
    );
  });

  it("counts only active conditions", () => {
    const filter: SignalFilterGroup = {
      ...newSignalGroup(),
      children: [
        rule("bus", "CAN1"),
        newSignalRule(),
        { ...newSignalGroup(), children: [rule("unit", "rpm")] },
      ],
    };
    expect(signalFilterRuleCount(filter)).toBe(2);
    expect(pruneSignalFilter(filter)).not.toBeNull();
  });
});
