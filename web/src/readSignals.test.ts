import { describe, expect, it } from "vitest";
import {
  catalogSignalCommand,
  catalogSignalSpec,
  findCatalogSignal,
  parseReadSignals,
  readSpecCommand,
} from "./readSignals";
import type { CatalogChannel, CatalogMessage, CatalogSignal } from "./types";

const canSignal: CatalogSignal = {
  key: "0:256:EngineTemp",
  name: "EngineTemp",
  unit: "degC",
  min: -40,
  max: 215,
};
const canMessage: CatalogMessage = {
  id: 0x100,
  extended: false,
  name: "EngineStatus",
  signals: [canSignal],
};
const canChannel: CatalogChannel = {
  kind: "can",
  index: 0,
  name: "CAN1",
  messages: [canMessage],
};

describe("read signal specs", () => {
  it("keeps the fields needed to restore a read set", () => {
    const reads = parseReadSignals([
      {
        key: "0:256:EngineTemp",
        kind: "can",
        bus: 0,
        id: 256,
        extended: false,
        name: "EngineTemp",
      },
      {
        key: "fr:0:13:CarSpeed",
        kind: "fr",
        bus: 0,
        id: 13,
        name: "CarSpeed",
      },
      {
        key: "eth:0:SpeedFrame:Speed",
        kind: "eth",
        bus: 0,
        frame: "SpeedFrame",
        name: "Speed",
      },
    ]);
    expect(reads.map((read) => read.kind)).toEqual(["can", "fr", "eth"]);
    expect(reads[0].extended).toBe(false);
    expect(reads[1].id).toBe(13);
    expect(reads[2].frame).toBe("SpeedFrame");
  });

  it("drops malformed, incomplete and duplicate entries", () => {
    const reads = parseReadSignals([
      { kind: "can", bus: 0, id: 1, name: "Ok" },
      { kind: "can", bus: 0, id: 1, name: "Ok" },
      { kind: "can", bus: 0, name: "NoId" },
      { kind: "eth", bus: 0, frame: "", name: "NoFrame" },
      { kind: "lin", bus: 0, id: 1, name: "WrongKind" },
      "nope",
    ]);
    expect(reads).toHaveLength(1);
    expect(reads[0].key).toBe("can:0:1:Ok");
  });

  it("builds read-only select commands for every bus kind", () => {
    expect(
      readSpecCommand({
        key: "0:256:EngineTemp",
        kind: "can",
        bus: 0,
        id: 256,
        extended: true,
        name: "EngineTemp",
      }),
    ).toMatchObject({ type: "select_signal", read: true, ext: true });
    expect(
      readSpecCommand({
        key: "fr:1:13:CarSpeed",
        kind: "fr",
        bus: 1,
        id: 13,
        name: "CarSpeed",
      }),
    ).toMatchObject({ type: "select_fr_signal", read: true, slot: 13 });
    expect(
      readSpecCommand({
        key: "eth:1:SpeedFrame:Speed",
        kind: "eth",
        bus: 1,
        frame: "SpeedFrame",
        name: "Speed",
      }),
    ).toMatchObject({ type: "select_eth_signal", read: true, frame: "SpeedFrame" });
  });

  it("maps a catalog signal to plot, Read and unselect commands", () => {
    expect(catalogSignalSpec(canChannel, canMessage, canSignal)).toMatchObject({
      key: "0:256:EngineTemp",
      kind: "can",
      bus: 0,
      id: 256,
      extended: false,
    });
    expect(
      catalogSignalCommand(canChannel, canMessage, canSignal, true),
    ).toMatchObject({ type: "select_signal", read: true });
    expect(
      catalogSignalCommand(canChannel, canMessage, canSignal, false),
    ).toMatchObject({ type: "select_signal", read: false });
    expect(
      catalogSignalCommand(canChannel, canMessage, canSignal, null),
    ).toMatchObject({ type: "unselect_signal" });
  });

  it("finds the catalog entry behind a stored key", () => {
    const found = findCatalogSignal(
      { channels: [canChannel] },
      "0:256:EngineTemp",
    );
    expect(found?.signal.name).toBe("EngineTemp");
    expect(found?.message.id).toBe(0x100);
    expect(findCatalogSignal({ channels: [canChannel] }, "0:9:Nope")).toBeNull();
    expect(findCatalogSignal(null, "0:256:EngineTemp")).toBeNull();
  });
});
