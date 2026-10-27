import { describe, expect, it } from "vitest";
import type { ScriptInfo, SignalSeries, SysVar } from "./types";
import {
  clampPanelNumber,
  findPanelSysVar,
  panelButtonCapability,
  panelNumberCapability,
  panelSourceOptions,
  panelToggleCapability,
  panelValueCapability,
  resolvePanelSource,
  runningPanelActions,
  type PanelActionContext,
} from "./panelBinding";
import type { PanelNumeric, PanelToggle, PanelValue } from "./panelPage";

const series = (key: string, name: string, points: [number, number][]): SignalSeries => ({
  key,
  name,
  bus: "CAN1",
  unit: "rpm",
  color: 0,
  read: false,
  points,
});

const sysvar = (value: number): SysVar => ({
  namespace: "Bench",
  name: "Ready",
  value,
  init: 0,
  min: 0,
  max: 1,
  unit: "",
  comment: "",
});

const script = (over: Partial<ScriptInfo>): ScriptInfo => ({
  id: 1,
  name: "panel",
  channel: 0,
  attachedBus: null,
  attachedNode: null,
  enabled: true,
  running: true,
  errored: false,
  filePath: "",
  logLen: 0,
  lastLog: "",
  panelActions: [],
  ...over,
});

describe("panel bindings", () => {
  it("resolves signal, derived and sysvar sources", () => {
    const view = {
      signals: [
        series("0:256:EngineSpeed", "EngineSpeed", [
          [0, 800],
          [1, 900],
        ]),
        series("derived:speed", "speed", [[1, 12.5]]),
      ],
      sysvars: [sysvar(1)],
    };
    expect(resolvePanelSource("0:256:EngineSpeed", view)).toEqual({
      value: 900,
      unit: "rpm",
      known: true,
    });
    expect(resolvePanelSource("derived:speed", view)).toEqual({
      value: 12.5,
      unit: "rpm",
      known: true,
    });
    expect(resolvePanelSource("sysvar:Bench::Ready", view)).toEqual({
      value: 1,
      unit: "",
      known: true,
    });
    expect(resolvePanelSource("nope", view).known).toBe(false);
    expect(resolvePanelSource("", view).known).toBe(false);
    expect(findPanelSysVar("Bench::Ready", view)?.name).toBe("Ready");
    expect(findPanelSysVar("Bench::Missing", view)).toBeUndefined();
  });

  it("lists sources and running action names", () => {
    const view = { signals: [series("k", "n", [])], sysvars: [sysvar(0)] };
    expect(panelSourceOptions(view, ["speed"])).toEqual([
      "k",
      "derived:speed",
      "sysvar:Bench::Ready",
    ]);
    expect(
      runningPanelActions([
        script({ panelActions: ["Go"] }),
        script({ id: 2, running: false, panelActions: ["Hidden"] }),
        script({ id: 3, errored: true, panelActions: ["Bad"] }),
        script({ id: 4, panelActions: ["Go", "Stop"] }),
      ]),
    ).toEqual(new Set(["Go", "Stop"]));
  });

  it("reports why a button cannot act", () => {
    const ctx: PanelActionContext = {
      online: true,
      measuring: false,
      paused: false,
      scriptActions: new Set(["Go"]),
    };
    const button = (action: string, scriptName = "") =>
      ({
        id: "b",
        kind: "button" as const,
        label: "B",
        action: action as never,
        script: scriptName,
        x: 0,
        y: 0,
        w: 20,
        h: 6,
      });
    expect(panelButtonCapability(button("start"), ctx).enabled).toBe(true);
    expect(panelButtonCapability(button("stop"), ctx)).toMatchObject({
      enabled: false,
      reason: "Start the measurement first",
    });
    expect(panelButtonCapability(button("script", "Go"), ctx)).toMatchObject({
      enabled: false,
    });
    expect(
      panelButtonCapability(button("script", "Go"), {
        ...ctx,
        measuring: true,
      }).enabled,
    ).toBe(true);
    expect(
      panelButtonCapability(button("script", "Missing"), {
        ...ctx,
        measuring: true,
      }).reason,
    ).toContain("No running script");
    expect(
      panelButtonCapability(button("pause"), { ...ctx, measuring: true, paused: true })
        .reason,
    ).toBe("Trace is already paused");
    expect(
      panelButtonCapability(button("stop"), { ...ctx, online: false }).reason,
    ).toContain("No connection");
  });

  it("reports unbound or unknown value/toggle sources", () => {
    const view = { signals: [], sysvars: [sysvar(0)] };
    const value = (source: string): PanelValue => ({
      id: "v",
      kind: "value",
      label: "V",
      source,
      digits: 1,
      x: 0,
      y: 0,
      w: 20,
      h: 6,
    });
    const toggle = (ref: string): PanelToggle => ({
      id: "t",
      kind: "toggle",
      label: "T",
      sysvar: ref,
      x: 0,
      y: 0,
      w: 16,
      h: 5,
    });
    expect(panelValueCapability(value(""), resolvePanelSource("", view)).reason).toBe(
      "No source bound",
    );
    expect(
      panelValueCapability(value("missing"), resolvePanelSource("missing", view))
        .reason,
    ).toContain("Unknown source");
    expect(panelValueCapability(value("k"), { value: null, unit: "", known: true }).enabled).toBe(
      true,
    );
    expect(panelToggleCapability(toggle(""), view).reason).toBe(
      "No system variable bound",
    );
    expect(panelToggleCapability(toggle("Bench::Nope"), view).reason).toContain(
      "Unknown sysvar",
    );
    expect(panelToggleCapability(toggle("Bench::Ready"), view).enabled).toBe(true);
    const numeric = (ref: string): PanelNumeric => ({
      id: "n",
      kind: "numeric",
      label: "N",
      sysvar: ref,
      digits: 1,
      step: 1,
      x: 0,
      y: 0,
      w: 18,
      h: 6,
    });
    expect(panelNumberCapability(numeric(""), view).reason).toBe(
      "No system variable bound",
    );
    expect(panelNumberCapability(numeric("Bench::Nope"), view).reason).toContain(
      "Unknown sysvar",
    );
    expect(panelNumberCapability(numeric("Bench::Ready"), view).enabled).toBe(
      true,
    );
    expect(clampPanelNumber(9000, 0, 500)).toBe(500);
    expect(clampPanelNumber(-5, 0, 500)).toBe(0);
    expect(clampPanelNumber(12.5, null, null)).toBe(12.5);
  });
});