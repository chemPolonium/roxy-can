import { describe, expect, it } from "vitest";
import type { MessageSignal } from "./types";
import {
  MESSAGE_SIGNAL_INITIAL_ROWS,
  messageSignalRows,
} from "./messageSignals";

function signal(name: string, unit = "", label?: string): MessageSignal {
  return { name, value: "0", raw: 0, unit, label };
}

describe("message signal rows", () => {
  it("matches name, label and unit without case sensitivity", () => {
    const signals = [
      signal("EngineSpeed", "rpm"),
      signal("EngineTemp", "degC", "Coolant"),
      signal("VehicleSpeed", "km/h"),
    ];
    expect(messageSignalRows(signals, "engine", false).visible).toHaveLength(2);
    expect(messageSignalRows(signals, "COOLANT", false).visible).toHaveLength(1);
    expect(messageSignalRows(signals, "rpm", false).visible).toHaveLength(1);
    expect(messageSignalRows(signals, "missing", false).visible).toHaveLength(0);
  });

  it("caps the first paint and expands on request", () => {
    const signals = Array.from({ length: 60 }, (_, index) =>
      signal(`Signal${index}`, "V"),
    );
    const capped = messageSignalRows(signals, "", false);
    expect(capped.total).toBe(60);
    expect(capped.matches).toBe(60);
    expect(capped.visible).toHaveLength(MESSAGE_SIGNAL_INITIAL_ROWS);
    expect(capped.hidden).toBe(10);

    const all = messageSignalRows(signals, "", true);
    expect(all.visible).toHaveLength(60);
    expect(all.hidden).toBe(0);
  });

  it("does not cap when the match set is small", () => {
    const signals = Array.from({ length: 60 }, (_, index) =>
      signal(`Signal${index}`, "V"),
    );
    const result = messageSignalRows(signals, "Signal1", false);
    expect(result.matches).toBe(11);
    expect(result.visible).toHaveLength(11);
    expect(result.hidden).toBe(0);
    expect(result.total).toBe(60);
  });

  it("trims the query before matching", () => {
    const signals = [signal("EngineSpeed", "rpm")];
    expect(messageSignalRows(signals, "  rpm  ", false).visible).toHaveLength(1);
  });
});
