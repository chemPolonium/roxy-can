import { describe, expect, it } from "vitest";
import type { ScriptInfo, SignalSeries, SysVar } from "./types";
import {
  clampPanelNumber,
  filterPanelOptions,
  findPanelSysVar,
  panelButtonCapability,
  panelGaugeArc,
  panelNumberCapability,
  panelSetpointCapability,
  panelSourceOptions,
  panelToggleCapability,
  panelLampLit,
  panelMeterRatio,
  panelValueBand,
  panelValueCapability,
  panelWidgetCapability,
  resolvePanelSource,
  someIpMemberFor,
  runningPanelActions,
  type PanelActionContext,
  type PanelView,
} from "./panelBinding";
import type {
  PanelBand,
  PanelBar,
  PanelButton,
  PanelLamp,
  PanelSetpoint,
  PanelToggle,
  PanelValue,
} from "./panelPage";

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
      measuring: true,
      paused: false,
      scriptActions: new Set(["Go"]),
    };
    const empty: PanelView = { signals: [], sysvars: [] };
    const button = (over: Partial<PanelButton>): PanelButton => ({
      id: "b",
      kind: "button",
      label: "B",
      action: "none",
      x: 0,
      y: 0,
      w: 20,
      h: 6,
      ...over,
    });
    expect(panelButtonCapability(button({}), empty, ctx)).toMatchObject({
      enabled: false,
      reason: "No action configured",
    });

    expect(
      panelButtonCapability(button({ action: "script" }), empty, ctx).reason,
    ).toBe("No script action name");
    expect(
      panelButtonCapability(button({ action: "script", script: "Go" }), empty, {
        ...ctx,
        measuring: false,
      }).reason,
    ).toContain("needs a running measurement");
    expect(
      panelButtonCapability(
        button({ action: "script", script: "Missing" }),
        empty,
        ctx,
      ).reason,
    ).toContain("No running script");
    expect(
      panelButtonCapability(button({ action: "script", script: "Go" }), empty, ctx)
        .enabled,
    ).toBe(true);

    expect(
      panelButtonCapability(button({ action: "send" }), empty, ctx).reason,
    ).toBe("No message configured");
    const frame = {
      protocol: "can" as const,
      channel: 0,
      id: 0x100,
      extended: false,
      data: "11 22",
    };
    expect(
      panelButtonCapability(button({ action: "send", frame }), empty, ctx).reason,
    ).toContain("Unknown CAN bus");
    const canView: PanelView = {
      signals: [],
      sysvars: [],
      frameBuses: [{ index: 0, name: "CAN1" }],
    };
    expect(
      panelButtonCapability(button({ action: "send", frame }), canView, ctx)
        .enabled,
    ).toBe(true);
    expect(
      panelButtonCapability(
        button({ action: "send", frame: { ...frame, data: "123" } }),
        canView,
        ctx,
      ).reason,
    ).toContain("hex bytes");
    expect(
      panelButtonCapability(button({}), empty, { ...ctx, online: false })
        .reason,
    ).toContain("No connection");
  });

  it("reports unbound or unknown value/toggle/setpoint sources", () => {
    const view: PanelView = { signals: [], sysvars: [sysvar(0)] };
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
    const setpoint = (over: Partial<PanelSetpoint> = {}): PanelSetpoint => ({
      id: "n",
      kind: "setpoint",
      label: "N",
      target: "sysvar",
      sysvar: "",
      source: "",
      digits: 1,
      step: 1,
      x: 0,
      y: 0,
      w: 18,
      h: 6,
      ...over,
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
    expect(panelNumberCapability(setpoint(), view).reason).toBe(
      "No system variable bound",
    );
    expect(
      panelNumberCapability(setpoint({ sysvar: "Bench::Nope" }), view).reason,
    ).toContain("Unknown sysvar");
    expect(
      panelNumberCapability(setpoint({ sysvar: "Bench::Ready" }), view).enabled,
    ).toBe(true);
    expect(clampPanelNumber(9000, 0, 500)).toBe(500);
    expect(clampPanelNumber(-5, 0, 500)).toBe(0);
    expect(clampPanelNumber(12.5, null, null)).toBe(12.5);

    const ctx: PanelActionContext = {
      online: true,
      measuring: true,
      paused: false,
      scriptActions: new Set(),
    };
    const writeView: PanelView = {
      signals: [],
      sysvars: [],
      writeTargets: [
        { key: "0:256:EngineSpeed", min: 0, max: 7000, unit: "rpm" },
      ],
    };
    expect(panelSetpointCapability("derived:speed", writeView, ctx).reason).toBe(
      "Derived signals are read-only",
    );
    expect(panelSetpointCapability("  ", writeView, ctx).reason).toBe(
      "No signal bound",
    );
    expect(
      panelSetpointCapability("0:256:Missing", writeView, ctx).reason,
    ).toContain("Unknown signal");
    expect(
      panelSetpointCapability("0:256:EngineSpeed", writeView, {
        ...ctx,
        measuring: false,
      }).reason,
    ).toBe("Start the measurement first");
    expect(
      panelSetpointCapability("0:256:EngineSpeed", writeView, ctx).enabled,
    ).toBe(true);
  });

  it("picks the first colour band the value fits", () => {
    const bands: PanelBand[] = [
      { upTo: 0, color: "neutral" },
      { upTo: 40, color: "ok" },
      { upTo: 80, color: "warn" },
      { upTo: null, color: "crit" },
    ];
    expect(panelValueBand(-5, bands)).toBe("neutral");
    expect(panelValueBand(0, bands)).toBe("neutral");
    expect(panelValueBand(12, bands)).toBe("ok");
    expect(panelValueBand(40, bands)).toBe("ok");
    expect(panelValueBand(40.5, bands)).toBe("warn");
    expect(panelValueBand(80, bands)).toBe("warn");
    expect(panelValueBand(120, bands)).toBe("crit");
    expect(panelValueBand(12, [{ upTo: 10, color: "ok" }])).toBeNull();
    expect(panelValueBand(12, [])).toBeNull();
    expect(panelValueBand(null, bands)).toBeNull();
    expect(panelValueBand(Number.NaN, bands)).toBeNull();
  });

  it("fills meters and lights lamps from their range", () => {
    expect(panelMeterRatio(50, 0, 100)).toBe(0.5);
    expect(panelMeterRatio(-5, 0, 100)).toBe(0);
    expect(panelMeterRatio(150, 0, 100)).toBe(1);
    expect(panelMeterRatio(null, 0, 100)).toBeNull();
    expect(panelMeterRatio(Number.NaN, 0, 100)).toBeNull();
    expect(panelMeterRatio(50, 100, 100)).toBeNull();

    const lamp: PanelLamp = {
      id: "lamp1",
      kind: "lamp",
      label: "Ready",
      source: "sysvar:Bench::Ready",
      threshold: 0.5,
      color: "ok",
      x: 0,
      y: 0,
      w: 14,
      h: 4,
    };
    expect(panelLampLit(lamp, 1)).toBe(true);
    expect(panelLampLit(lamp, 0.5)).toBe(true);
    expect(panelLampLit(lamp, 0.4)).toBe(false);
    expect(panelLampLit(lamp, null)).toBe(false);

    const bar: PanelBar = {
      id: "bar1",
      kind: "bar",
      label: "Speed",
      source: "0:512:VehicleSpeed",
      min: 0,
      max: 100,
      digits: 1,
      x: 0,
      y: 0,
      w: 20,
      h: 7,
    };
    const view = { signals: [], sysvars: [] };
    const ctx: PanelActionContext = {
      online: true,
      measuring: true,
      paused: false,
      scriptActions: new Set(),
    };
    expect(panelWidgetCapability(bar, view, ctx).reason).toContain(
      "Unknown source",
    );
    expect(
      panelWidgetCapability({ ...lamp, source: "" }, view, ctx).reason,
    ).toBe("No source bound");
  });

  it("filters source suggestions by substring", () => {
    const options = [
      "0:256:EngineSpeed",
      "0:800:BrakePressure",
      "sysvar:Bench::Pressure",
      "derived:WheelSlip",
    ];
    expect(filterPanelOptions(options, "")).toEqual(options);
    expect(filterPanelOptions(options, "  PRESSURE ")).toEqual([
      "0:800:BrakePressure",
      "sysvar:Bench::Pressure",
    ]);
    expect(filterPanelOptions(options, "0:2")).toEqual(["0:256:EngineSpeed"]);
    expect(filterPanelOptions(options, "nothing")).toEqual([]);
    expect(filterPanelOptions(options, "", 2)).toHaveLength(2);
  });

  it("draws the gauge fill as a short arc", () => {
    expect(panelGaugeArc(0)).toBe("M6 50 A44 44 0 0 1 6.00 50.00");
    expect(panelGaugeArc(0.25)).toBe("M6 50 A44 44 0 0 1 18.89 18.89");
    expect(panelGaugeArc(0.5)).toBe("M6 50 A44 44 0 0 1 50.00 6.00");
    expect(panelGaugeArc(0.8)).toBe("M6 50 A44 44 0 0 1 85.60 24.14");
    expect(panelGaugeArc(1)).toBe("M6 50 A44 44 0 0 1 94.00 50.00");
    expect(panelGaugeArc(2)).toBe(panelGaugeArc(1));
    expect(panelGaugeArc(-1)).toBe(panelGaugeArc(0));
  });
});
describe("SOME/IP panel fields", () => {
  it("finds a method or event by service id and prefers the message type", () => {
    const services = [
      {
        id: 0x1234,
        name: "Speed",
        bus: 0,
        busName: "ETH0",
        major: 1,
        source: "",
        methods: [{ id: 1, name: "Get", fields: ["speed"] }],
        events: [{ id: 1, name: "OnSpeed", fields: ["speed"] }],
        instances: [],
      },
    ];
    expect(someIpMemberFor(services, 0, 0x1234, 1, 0x00)?.name).toBe("Get");
    expect(someIpMemberFor(services, 0, 0x1234, 1, 0x02)?.name).toBe(
      "OnSpeed",
    );
    expect(someIpMemberFor(services, 1, 0x1234, 1, 0x00)).toBeUndefined();
  });
});
