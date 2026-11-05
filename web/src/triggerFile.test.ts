import { describe, expect, it } from "vitest";
import type { Bus, TriggerEntry } from "./types";
import {
  buildTriggerFile,
  parseTriggerFile,
  resolveTriggerFile,
  TRIGGER_FILE_FORMAT,
  TRIGGER_FILE_VERSION,
} from "./triggerFile";

function bus(index: number, name: string, kind: Bus["kind"]): Bus {
  return {
    index,
    id: index + 1,
    kind,
    name,
    frames: 0,
    source: "",
    descriptions: [],
  };
}

function trigger(overrides: Partial<TriggerEntry> = {}): TriggerEntry {
  return {
    summary: "slot 13 timeout",
    action: "insert marker",
    kind: "fr_timeout",
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
    bus: 2,
    slot: 13,
    varKey: "",
    derivedId: "",
    actionKind: "insert_marker",
    sendCh: 0,
    sendId: 0,
    cooldownMs: 100,
    maxFires: 3,
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
    preFrames: 8,
    postFrames: 16,
    ...overrides,
  };
}

describe("trigger files", () => {
  it("keeps SOME/IP send fields and remaps the Ethernet bus", () => {
    const rule = trigger({
      kind: "someip",
      summary: "ETH0 SOME/IP 0x1234",
      bus: 1,
      id: 0x1234,
      actionKind: "send_someip",
      action: "send SOME/IP",
      sendCh: 1,
      sendId: 0x1234,
      sendData: "01 02",
      someipMethod: 7,
      someipClient: 3,
      someipSession: 9,
      someipType: 2,
    });
    const file = buildTriggerFile([rule], [bus(1, "ETH-A", "ethernet")]);
    expect(file.triggers[0].sendBusName).toBe("ETH-A");
    const parsed = parseTriggerFile(JSON.parse(JSON.stringify(file)));
    if (!("file" in parsed)) return;
    const resolved = resolveTriggerFile(parsed.file, [
      bus(7, "ETH-A", "ethernet"),
    ]);
    expect(resolved.skipped).toEqual([]);
    expect(resolved.rules[0].sendChannel).toBe(7);
    expect(resolved.rules[0].someipMethod).toBe(7);
    expect(resolved.rules[0].someipClient).toBe(3);
    expect(resolved.rules[0].someipSession).toBe(9);
    expect(resolved.rules[0].someipType).toBe(2);
  });
  it("keeps a raw Ethernet send's bus name", () => {
    const rule = trigger({
      kind: "eth_flow",
      bus: 1,
      actionKind: "send_eth",
      action: "send Ethernet frame",
      sendCh: 1,
      sendFrame: "FrameA",
      sendData: "AA BB",
    });
    const file = buildTriggerFile([rule], [bus(1, "ETH-A", "ethernet")]);
    expect(file.triggers[0].sendBusName).toBe("ETH-A");
    const parsed = parseTriggerFile(JSON.parse(JSON.stringify(file)));
    if (!("file" in parsed)) return;
    const resolved = resolveTriggerFile(parsed.file, [
      bus(6, "ETH-A", "ethernet"),
    ]);
    expect(resolved.skipped).toEqual([]);
    expect(resolved.rules[0].sendChannel).toBe(6);
    expect(resolved.rules[0].sendFrame).toBe("FrameA");
  });
  it("round-trips rules and remaps buses by name", () => {
    const file = buildTriggerFile([trigger()], [bus(2, "FR0", "flexray")]);
    expect(file.format).toBe(TRIGGER_FILE_FORMAT);
    expect(file.triggers[0].busName).toBe("FR0");

    const parsed = parseTriggerFile(JSON.parse(JSON.stringify(file)));
    expect("file" in parsed).toBe(true);
    if (!("file" in parsed)) return;

    const resolved = resolveTriggerFile(parsed.file, [
      bus(0, "CAN1", "can"),
      bus(5, "FR0", "flexray"),
    ]);
    expect(resolved.skipped).toEqual([]);
    expect(resolved.rules[0].channel).toBe(5);
    expect(resolved.rules[0].slot).toBe(13);
    expect(resolved.rules[0].preFrames).toBe(8);
  });

  it("keeps the send bus by name as well", () => {
    const file = buildTriggerFile(
      [
        trigger({
          kind: "id",
          actionKind: "send_fr",
          sendCh: 2,
          sendId: 13,
        }),
      ],
      [bus(0, "CAN1", "can"), bus(2, "FR0", "flexray")],
    );
    expect(file.triggers[0].sendBusName).toBe("FR0");
    const parsed = parseTriggerFile(file);
    if (!("file" in parsed)) throw new Error(parsed.error);
    const resolved = resolveTriggerFile(parsed.file, [
      bus(0, "CAN1", "can"),
      bus(7, "FR0", "flexray"),
    ]);
    expect(resolved.rules[0].sendChannel).toBe(7);
  });

  it("rejects a foreign or newer file", () => {
    expect(
      "error" in parseTriggerFile({ format: "other", version: 1, triggers: [] }),
    ).toBe(true);
    const file = buildTriggerFile([trigger()], []);
    expect(
      "error" in parseTriggerFile({ ...file, version: TRIGGER_FILE_VERSION + 1 }),
    ).toBe(true);
  });

  it("skips rules whose bus is missing or ambiguous", () => {
    const file = buildTriggerFile([trigger()], [bus(2, "FR0", "flexray")]);
    const parsed = parseTriggerFile(file);
    if (!("file" in parsed)) throw new Error(parsed.error);
    const missing = resolveTriggerFile(parsed.file, [bus(0, "CAN1", "can")]);
    expect(missing.rules).toEqual([]);
    expect(missing.skipped[0]).toContain("no such bus");

    const doubled = resolveTriggerFile(parsed.file, [
      bus(1, "FR0", "flexray"),
      bus(2, "FR0", "flexray"),
    ]);
    expect(doubled.rules).toEqual([]);
    expect(doubled.skipped[0]).toContain("more than one bus");
  });
});
