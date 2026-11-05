import { describe, expect, it } from "vitest";
import type { SomeIpEvent } from "./types";
import { groupSomeIpServices } from "./someipView";

function event(overrides: Partial<SomeIpEvent> = {}): SomeIpEvent {
  return {
    tUs: 0,
    bus: 0,
    tx: false,
    transport: "UDP",
    src: "10.0.0.1",
    dst: "10.0.0.2",
    srcPort: 30501,
    dstPort: 30490,
    service: 0x1234,
    method: 0x0001,
    messageType: 0x02,
    returnCode: 0,
    client: 1,
    session: 1,
    interfaceVersion: 1,
    protocolVersion: 1,
    length: 8,
    payload: [],
    decoded: [],
    sd: null,
    ...overrides,
  };
}

describe("SOME/IP service summary", () => {
  it("groups methods and attaches SD endpoints", () => {
    const rows = [
      event({ tUs: 1_000, method: 1 }),
      event({ tUs: 2_000, method: 2 }),
      event({
        service: 0xffff,
        method: 0x8100,
        sd: [
          {
            kind: 1,
            service: 0x1234,
            instance: 1,
            major: 1,
            ttl: 3,
            minor: 0,
            eventgroup: null,
            endpoints: [{ address: "10.0.0.2", port: 30501, protocol: 0x11 }],
            configuration: [],
          },
        ],
      }),
    ];
    const services = groupSomeIpServices(rows);
    expect(services).toHaveLength(1);
    expect(services[0].messages).toBe(2);
    expect(services[0].methods).toEqual([1, 2]);
    expect(services[0].endpoints).toEqual(["10.0.0.2:30501/UDP"]);
    expect(services[0].lastTus).toBe(2_000);
    expect(services[0].spanUs).toBe(1_000);
    expect(services[0].periodUs).toBe(1_000);
    expect(services[0].jitterUs).toBe(0);
  });
});
