import { describe, expect, it } from "vitest";
import type { TriggerEntry } from "./types";
import { checkTriggers, type TriggerCheckContext } from "./triggerCheck";

function trigger(overrides: Partial<TriggerEntry> = {}): TriggerEntry {
  return {
    summary: "",
    action: "",
    kind: "id",
    enabled: true,
    fired: 0,
    level: false,
    lastFireUs: 0,
    ch: 0,
    id: 0x100,
    ext: false,
    signal: "",
    threshold: 0,
    rising: false,
    bus: 0,
    slot: 0,
    varKey: "",
    derivedId: "",
    actionKind: "insert_marker",
    sendCh: 0,
    sendId: 0,
    cooldownMs: 0,
    maxFires: 0,
    sysVarKey: "",
    sysVarValue: 0,
    scriptName: "",
    scriptValue: null,
    sendFrame: "",
    sendData: "",
    offset: 0,
    mask: 0xff,
    value: 0,
    vlan: null,
    preFrames: null,
    postFrames: null,
    ...overrides,
  };
}

function context(overrides: Partial<TriggerCheckContext> = {}): TriggerCheckContext {
  return {
    buses: [],
    sysvars: [],
    derived: [],
    tx: [],
    frTx: [],
    scripts: [],
    catalog: null,
    ...overrides,
  };
}

describe("trigger checks", () => {
  it("flags a rule aimed at a missing bus", () => {
    const issues = checkTriggers([trigger({ ch: 4 })], context());
    expect(issues[0]).toContain("bus 4");
  });

  it("flags undeclared sysvars and send targets", () => {
    const issues = checkTriggers(
      [
        trigger({
          kind: "sysvar",
          varKey: "Bench::Missing",
        }),
        trigger({ actionKind: "send", sendCh: 0, sendId: 0x200 }),
        trigger({ actionKind: "script", scriptName: "Go" }),
      ],
      context({
        buses: [{ index: 0, id: 1, kind: "can", name: "CAN1", frames: 0, source: "", descriptions: [] }],
      }),
    );
    expect(issues.some((line) => line.includes("Bench::Missing"))).toBe(true);
    expect(issues.some((line) => line.includes("no CAN generator entry"))).toBe(true);
    expect(issues.some((line) => line.includes("panel action \"Go\""))).toBe(true);
  });

  it("accepts fully resolved targets", () => {
    const issues = checkTriggers(
      [
        trigger({ kind: "sysvar", varKey: "Bench::Ready" }),
        trigger({ actionKind: "send_fr", sendCh: 0, sendId: 13 }),
      ],
      context({
        buses: [{ index: 0, id: 1, kind: "flexray", name: "FR0", frames: 0, source: "", descriptions: [] }],
        sysvars: [
          { namespace: "Bench", name: "Ready", value: 0, init: 0, min: null, max: null, unit: "", comment: "" },
        ],
        frTx: [
          {
            bus: 0,
            busName: "FR0",
            slot: 13,
            name: "F",
            node: "",
            active: true,
            cycleUs: 10_000,
            dataText: "",
            sentText: "",
            muted: false,
            undescribed: false,
          },
        ],
      }),
    );
    expect(issues).toEqual([]);
  });
});
