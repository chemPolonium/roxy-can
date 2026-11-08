import { describe, expect, it } from "vitest";
import {
  DEFAULT_DOCK,
  LAYOUTS,
  PANELS,
  assignPanel,
  defaultDock,
  defaultPanelsFor,
  layoutInfo,
  moveSlot,
  normalizeDock,
  normalizePanels,
  normalizeRatios,
  panelsFor,
  setLayout,
  showPanel,
} from "./dockLayout";

describe("dock layout model", () => {
  it("defines a unique panel for every slot in every preset", () => {
    for (const layout of LAYOUTS) {
      const panels = defaultPanelsFor(layout.id);
      expect(panels).toHaveLength(layout.slots);
      expect(new Set(panels).size).toBe(panels.length);
      for (const panel of panels) {
        expect(PANELS.some((entry) => entry.id === panel)).toBe(true);
      }
    }
  });

  it("keeps the default layout assignments unique", () => {
    for (const layout of LAYOUTS) {
      const panels = DEFAULT_DOCK.panels[layout.id];
      expect(panels).toHaveLength(layout.slots);
      expect(new Set(panels).size).toBe(layout.slots);
    }
  });

  it("drops unknown and duplicate panels and fills the missing slots", () => {
    const panels = normalizePanels("quad", ["trace", "trace", "nope", "bus"]);
    expect(panels).toHaveLength(4);
    expect(new Set(panels).size).toBe(4);
    expect(panels.slice(0, 2)).toEqual(["trace", "bus"]);
  });

  it("clamps splitter ratios", () => {
    expect(normalizeRatios("main-side", { x: -1, y: 9 })).toEqual({
      x: 0.3,
      y: 0.8,
    });
    expect(normalizeRatios("quad", { x: 0.9, y: 0.1 })).toEqual({
      x: 0.75,
      y: 0.25,
    });
  });

  it("swaps when a panel already lives in another slot", () => {
    const dock = defaultDock();
    const before = panelsFor(dock);
    const next = assignPanel(dock, 0, before[1]);
    expect(panelsFor(next)[0]).toBe(before[1]);
    expect(panelsFor(next)[1]).toBe(before[0]);
    expect(new Set(panelsFor(next)).size).toBe(before.length);
  });

  it("moves a slot by swapping its content with the target", () => {
    const dock = defaultDock();
    const before = [...panelsFor(dock)];
    const next = moveSlot(dock, 0, 2);
    expect(panelsFor(next)[0]).toBe(before[2]);
    expect(panelsFor(next)[2]).toBe(before[0]);
    expect(panelsFor(next)[1]).toBe(before[1]);
  });

  it("reveals a panel in the preferred slot when it is not visible", () => {
    const dock = setLayout(defaultDock(), "split");
    const { dock: next, slot } = showPanel(dock, "write", 1);
    expect(slot).toBe(1);
    expect(panelsFor(next)[1]).toBe("write");
  });

  it("keeps an already visible panel where it is", () => {
    const dock = setLayout(defaultDock(), "split");
    const visible = panelsFor(dock)[0];
    const { dock: next, slot } = showPanel(dock, visible, 1);
    expect(slot).toBe(0);
    expect(panelsFor(next)).toEqual(panelsFor(dock));
  });

  it("migrates the old fixed shell into main + side", () => {
    const dock = normalizeDock(undefined, {
      mainView: "trace",
      chartTab: "signals",
      tab: "activity",
      layout: { main: 0.7, right: 0.4 },
    });
    expect(dock.layout).toBe("main-side");
    expect(panelsFor(dock)).toEqual(["trace", "signals", "write"]);
    expect(dock.ratios["main-side"]).toEqual({ x: 0.7, y: 0.4 });
  });

  it("normalizes a saved dock and keeps a valid layout", () => {
    const dock = normalizeDock({
      layout: "quad",
      panels: {
        quad: ["write", "spec", "someip", "bus"],
        split: ["trace", "trace"],
      },
      ratios: { quad: { x: 0.6, y: 0.4 } },
    });
    expect(dock.layout).toBe("quad");
    expect(panelsFor(dock)).toEqual(["write", "spec", "someip", "bus"]);
    expect(dock.ratios.quad).toEqual({ x: 0.6, y: 0.4 });
    expect(new Set(dock.panels.split).size).toBe(2);
    expect(layoutInfo(dock.layout).slots).toBe(4);
  });
});
