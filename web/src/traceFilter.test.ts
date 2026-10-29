import { describe, expect, it } from "vitest";
import type { Frame } from "./types";
import { parseTraceQuery, traceQueryPredicate } from "./traceFilter";

const frame = (patch: Partial<Frame> = {}): Frame => ({
  kind: "can",
  addr: "0x100",
  tUs: 1_000_000,
  bus: 0,
  busName: "CAN1",
  id: 0x100,
  extended: false,
  name: "EngineStatus",
  dir: "Rx",
  len: 8,
  data: "A0 0F 78 00 00 00 00 00",
  fd: false,
  brs: false,
  ...patch,
});

const match = (query: string, value: Frame = frame()) =>
  traceQueryPredicate(query)(value);

describe("trace filter syntax", () => {
  it("keeps the bare substring behaviour", () => {
    expect(match("engine")).toBe(true);
    expect(match("can1")).toBe(true);
    expect(match("0x100")).toBe(true);
    expect(match("A00F")).toBe(false);
    expect(match("A0 0F")).toBe(true);
    expect(match("nothing")).toBe(false);
    expect(match("")).toBe(true);
  });

  it("matches id, bus, dir, name, data and kind fields", () => {
    expect(match("id:0x100")).toBe(true);
    expect(match("id:100")).toBe(true);
    expect(match("id:0x101")).toBe(false);
    expect(match("bus:CAN1")).toBe(true);
    expect(match("bus:1")).toBe(false);
    expect(match("bus:0")).toBe(true);
    expect(match("dir:rx")).toBe(true);
    expect(match("dir:tx")).toBe(false);
    expect(match("name:engine")).toBe(true);
    expect(match("data:A00F")).toBe(true);
    expect(match("data:0000")).toBe(true);
    expect(match("kind:can")).toBe(true);
    expect(match("kind:fr")).toBe(false);
  });

  it("requires every token and lets - exclude", () => {
    expect(match("engine 0x100")).toBe(true);
    expect(match("engine 0x101")).toBe(false);
    expect(match("bus:CAN1 -dir:rx")).toBe(false);
    expect(match("-tx")).toBe(true);
    expect(match("-rx")).toBe(false);
    expect(match("-id:0x101")).toBe(true);
    expect(match("-id:0x100")).toBe(false);
    expect(match("-error")).toBe(true);
  });

  it("keeps quoted spaces in one value", () => {
    const spaced = frame({ name: "Engine Status" });
    expect(match('"Engine Status"', spaced)).toBe(true);
    expect(match('name:"Engine Status"', spaced)).toBe(true);
    expect(match("name:Engine name:Status", spaced)).toBe(true);
  });

  it("falls back to a substring for unknown fields", () => {
    expect(parseTraceQuery("foo:bar")[0]).toMatchObject({ field: "foo" });
    expect(match("foo:bar")).toBe(false);
    expect(match("foo:bar", frame({ data: "66 6F 6F 3A 62 61 72" }))).toBe(
      false,
    );
  });
});
