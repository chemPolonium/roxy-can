import { describe, expect, it } from "vitest";
import type { SomeIpEvent, SdEntry } from "./types";
import { sdServiceStatuses } from "./sdView";

function sdEvent(
  tUs: number,
  entries: SdEntry[],
): SomeIpEvent {
  return {
    tUs,
    bus: 0,
    tx: false,
    transport: "UDP",
    src: "10.0.0.2",
    dst: "10.0.0.1",
    srcPort: 30490,
    dstPort: 30490,
    service: 0xffff,
    method: 0x8100,
    messageType: 0x02,
    returnCode: 0,
    client: 0,
    session: 1,
    interfaceVersion: 1,
    protocolVersion: 1,
    length: 8,
    payload: [],
    decoded: [],
    sd: entries,
  };
}

function entry(overrides: Partial<SdEntry> = {}): SdEntry {
  return {
    kind: 0x01,
    service: 0x1234,
    instance: 1,
    major: 1,
    ttl: 3,
    minor: 0,
    eventgroup: null,
    endpoints: [{ address: "10.0.0.2", port: 30501, protocol: 0x11 }],
    configuration: [],
    ...overrides,
  };
}

describe("SD availability", () => {
  it("expires an offer when the capture continues past its TTL", () => {
    const rows = [sdEvent(1_000_000, [entry()]), sdEvent(5_000_000, [])];
    const [status] = sdServiceStatuses(rows);
    expect(status.state).toBe("expired");
    expect(status.endpoints).toEqual(["10.0.0.2:30501/UDP"]);
  });

  it("keeps an infinite TTL and applies a later stop offer", () => {
    const rows = [
      sdEvent(1_000_000, [entry({ ttl: 0xffffff })]),
      sdEvent(2_000_000, [entry({ kind: 0x01, ttl: 0 })]),
    ];
    expect(sdServiceStatuses(rows)[0].state).toBe("stopped");
  });

  it("counts subscribers and eventgroups", () => {
    const rows = [
      sdEvent(1_000_000, [
        entry({ kind: 0x06, eventgroup: 0x0001 }),
        entry({ kind: 0x06, eventgroup: 0x0001 }),
      ]),
    ];
    const [status] = sdServiceStatuses(rows);
    expect(status.subscribers).toBe(2);
    expect(status.eventgroups).toEqual([1]);
  });
});
