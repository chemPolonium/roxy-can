import { describe, expect, it } from "vitest";
import {
  PAGE_HEIGHT,
  PAGE_MAX_W,
  PAGE_WIDTH,
  PANEL_FILE_FORMAT,
  PANEL_WIDGET_KINDS,
  addPanelPage,
  addPanelPageFromFile,
  addPanelWidget,
  defaultPanelBook,
  defaultPanelPage,
  duplicatePanelPage,
  duplicatePanelWidget,
  exportPanelFile,
  formatPanelValue,
  importPanelFile,
  movePanelWidget,
  panelFileName,
  panelFrameProblem,
  panelWidgetSpec,
  parsePanelBook,
  parsePanelPage,
  parseSysVarRef,
  pastePanelWidget,
  removePanelPage,
  removePanelWidget,
  renamePanelPage,
  resizePanelPage,
  selectPanelPage,
  updatePanelWidget,
  widgetMinSize,
} from "./panelPage";

describe("panel page", () => {
  it("starts from an empty fixed 8px canvas", () => {
    const page = defaultPanelPage();
    expect(page.width).toBe(PAGE_WIDTH);
    expect(page.height).toBe(PAGE_HEIGHT);
    expect(page.widgets).toEqual([]);
  });

  it("keeps valid widgets and rejects malformed or overlapping entries", () => {
    const page = parsePanelPage({
      width: 160,
      height: 90,
      widgets: [
        {
          id: "ok",
          kind: "button",
          label: "Start",
          action: "none",
          x: 10,
          y: 10,
          w: 20,
          h: 6,
        },
        {
          id: "overlap",
          kind: "button",
          label: "Overlap",
          action: "none",
          x: 12,
          y: 12,
          w: 20,
          h: 6,
        },
        {
          id: "bad-action",
          kind: "button",
          label: "Nope",
          action: "fire",
          x: 40,
          y: 10,
          w: 20,
          h: 6,
        },
        {
          id: "too-small",
          kind: "button",
          label: "Tiny",
          action: "none",
          x: 60,
          y: 10,
          w: 2,
          h: 2,
        },
      ],
    });
    expect(page?.widgets.map((widget) => widget.id)).toEqual(["ok"]);
  });

  it("clamps stored coordinates into the canvas", () => {
    const page = parsePanelPage({
      width: 160,
      height: 90,
      widgets: [
        {
          id: "edge",
          kind: "button",
          label: "Edge",
          action: "none",
          x: 500,
          y: -20,
          w: 40,
          h: 8,
        },
      ],
    });
    expect(page?.widgets[0]).toMatchObject({ x: 120, y: 0, w: 40, h: 8 });
  });

  it("snaps moves to cells, clamps size and refuses overlap", () => {
    const page = parsePanelPage({
      width: 160,
      height: 90,
      widgets: [
        {
          id: "start",
          kind: "button",
          label: "Start",
          action: "none",
          x: 8,
          y: 8,
          w: 20,
          h: 6,
        },
        {
          id: "stop",
          kind: "button",
          label: "Stop",
          action: "none",
          x: 8,
          y: 16,
          w: 20,
          h: 6,
        },
      ],
    });
    if (!page) throw new Error("page");
    const moved = movePanelWidget(page, "start", 30, 12, 20, 6);
    expect(moved?.widgets.find((widget) => widget.id === "start")).toMatchObject(
      { x: 30, y: 12 },
    );
    const shrunk = movePanelWidget(page, "start", 8, 8, 1, 1);
    expect(shrunk?.widgets.find((widget) => widget.id === "start")).toMatchObject(
      { w: widgetMinSize("button").w, h: widgetMinSize("button").h },
    );
    expect(movePanelWidget(page, "start", 8, 16, 20, 6)).toBeNull();
    expect(movePanelWidget(page, "start", 200, 8, 20, 6)?.widgets[0].x).toBe(
      140,
    );
  });

  it("adds a free button and reports a full page", () => {
    const page = defaultPanelPage();
    const added = addPanelWidget(page, "button");
    expect(added).not.toBeNull();
    expect(added?.page.widgets).toHaveLength(page.widgets.length + 1);
    expect(added?.page.widgets.some((widget) => widget.id === added.id)).toBe(
      true,
    );

    const full = {
      ...page,
      widgets: [
        {
          id: "wall",
          kind: "button" as const,
          label: "Wall",
          action: "none" as const,
          x: 0,
          y: 0,
          w: PAGE_WIDTH,
          h: PAGE_HEIGHT,
        },
      ],
    };
    expect(addPanelWidget(full, "button")).toBeNull();
  });

  it("updates labels/actions and removes widgets", () => {
    const page = parsePanelPage({
      width: 160,
      height: 90,
      widgets: [
        {
          id: "start",
          kind: "button",
          label: "Start",
          action: "none",
          x: 8,
          y: 8,
          w: 20,
          h: 6,
        },
      ],
    });
    if (!page) throw new Error("page");
    const updated = updatePanelWidget(page, "start", {
      label: "Launch",
      action: "script",
    });
    expect(updated.widgets[0]).toMatchObject({
      label: "Launch",
      action: "script",
    });
    const removed = removePanelWidget(updated, "start");
    expect(removed.widgets).toHaveLength(page.widgets.length - 1);
  });

  it("keeps script-action bindings and updates their names", () => {
    const page = parsePanelPage({
      width: 160,
      height: 90,
      widgets: [
        {
          id: "script",
          kind: "button",
          label: "Launch",
          action: "script",
          script: "Start sequence",
          x: 8,
          y: 8,
          w: 20,
          h: 6,
        },
      ],
    });
    expect(page?.widgets[0]).toMatchObject({
      action: "script",
      script: "Start sequence",
    });
    const updated = updatePanelWidget(page!, "script", {
      script: "Stop sequence",
    });
    expect(updated.widgets[0]).toMatchObject({ script: "Stop sequence" });
  });

  it("resizes the canvas in cells and refuses to orphan widgets", () => {
    const page = parsePanelPage({
      width: 160,
      height: 90,
      widgets: [
        {
          id: "wall",
          kind: "value",
          label: "V",
          source: "",
          digits: 1,
          x: 45,
          y: 30,
          w: 12,
          h: 5,
        },
      ],
    });
    if (!page) throw new Error("page");
    expect(resizePanelPage(page, 200, 120)).toMatchObject({
      width: 200,
      height: 120,
    });
    expect(resizePanelPage(page, 60, 40)).not.toBeNull();
    expect(resizePanelPage(page, 40, 20)).toBeNull();
    expect(resizePanelPage(page, PAGE_MAX_W + 50, 100)?.width).toBe(
      PAGE_MAX_W,
    );
    expect(resizePanelPage(page, Number.NaN, 100)).toBeNull();
  });

  it("round-trips value and toggle widgets", () => {
    const page = parsePanelPage({
      width: 160,
      height: 90,
      widgets: [
        {
          id: "value1",
          kind: "value",
          label: "Speed",
          source: "0:256:EngineSpeed",
          digits: 2,
          x: 8,
          y: 8,
          w: 20,
          h: 6,
        },
        {
          id: "toggle1",
          kind: "toggle",
          label: "Ready",
          sysvar: "Bench::Ready",
          x: 8,
          y: 16,
          w: 16,
          h: 5,
        },
      ],
    });
    expect(page?.widgets).toHaveLength(2);
    const value = page?.widgets[0];
    expect(value).toMatchObject({
      kind: "value",
      source: "0:256:EngineSpeed",
      digits: 2,
    });
    const toggle = page?.widgets[1];
    expect(toggle).toMatchObject({ kind: "toggle", sysvar: "Bench::Ready" });
    const changed = updatePanelWidget(page!, "value1", { digits: 9 });
    expect(changed.widgets[0]).toMatchObject({ digits: 3 });
    const added = addPanelWidget(page!, "toggle");
    expect(
      added?.page.widgets.some((widget) => widget.kind === "toggle"),
    ).toBe(true);
    expect(
      addPanelWidget(page!, "value")?.page.widgets.filter(
        (widget) => widget.kind === "value",
      ),
    ).toHaveLength(2);
  });

  it("parses sysvar refs and formats values", () => {
    expect(parseSysVarRef("Bench::Ready")).toEqual({
      namespace: "Bench",
      name: "Ready",
    });
    expect(parseSysVarRef("::Bad")).toBeNull();
    expect(parseSysVarRef("Bench::")).toBeNull();
    expect(parseSysVarRef("Bench")).toBeNull();
    expect(formatPanelValue(12.345, 2)).toBe("12.35");
    expect(formatPanelValue(null, 1)).toBe("—");
    expect(formatPanelValue(Number.NaN, 1)).toBe("—");
  });

  it("normalises value colour bands", () => {
    const page = parsePanelPage({
      width: 160,
      height: 90,
      widgets: [
        {
          id: "value1",
          kind: "value",
          label: "Temp",
          source: "0:256:EngineTemp",
          digits: 1,
          x: 0,
          y: 0,
          w: 20,
          h: 6,
          bands: [
            { upTo: 80, color: "ok" },
            { upTo: null, color: "crit" },
            { upTo: 40, color: "warn" },
            { upTo: 40, color: "ok" },
            { upTo: "hot", color: "ok" },
            { upTo: 10, color: "nope" },
          ],
        },
      ],
    });
    const widget = page?.widgets[0];
    expect(widget?.kind === "value" ? widget.bands : null).toEqual([
      { upTo: 40, color: "warn" },
      { upTo: 80, color: "ok" },
      { upTo: null, color: "crit" },
    ]);
    const copied = pastePanelWidget(page!, widget!);
    const clone = copied?.page.widgets.find((entry) => entry.id === copied.id);
    expect(clone?.kind === "value" ? clone.bands : null).toEqual(
      widget?.kind === "value" ? widget.bands : null,
    );
    const capped = updatePanelWidget(page!, "value1", {
      bands: Array.from({ length: 12 }, (_, index) => ({
        upTo: index,
        color: "ok" as const,
      })),
    });
    const cappedWidget = capped.widgets[0];
    expect(cappedWidget.kind === "value" ? cappedWidget.bands : null).toEqual(
      Array.from({ length: 8 }, (_, index) => ({ upTo: index, color: "ok" })),
    );
    const cleared = updatePanelWidget(page!, "value1", { bands: [] });
    expect((cleared.widgets[0] as { bands?: unknown }).bands).toBeUndefined();
  });

  it("drives palette and inspector from the widget registry", () => {
    for (const entry of PANEL_WIDGET_KINDS) {
      const spec = panelWidgetSpec(entry.kind);
      expect(spec.kind).toBe(entry.kind);
      expect(spec.fields.length).toBeGreaterThan(0);
      expect(spec.fields[0]).toMatchObject({ key: "label" });
      expect(widgetMinSize(entry.kind)).toEqual({
        w: spec.minW,
        h: spec.minH,
      });
      const widget = spec.create(`${entry.kind}0`, 0, 0, spec.defaultW, spec.defaultH);
      expect(spec.applyPatch(widget, { label: "X" }).label).toBe("X");
    }
  });

  it("duplicates a widget next to its spot", () => {
    const page = parsePanelPage({
      width: 160,
      height: 90,
      widgets: [
        {
          id: "start",
          kind: "button",
          label: "Start",
          action: "none",
          x: 8,
          y: 8,
          w: 20,
          h: 6,
        },
      ],
    });
    if (!page) throw new Error("page");
    const copy = duplicatePanelWidget(page, "start");
    expect(copy?.page.widgets).toHaveLength(page.widgets.length + 1);
    const source = page.widgets.find((widget) => widget.id === "start");
    const clone = copy?.page.widgets.find((widget) => widget.id === copy.id);
    expect(clone).toMatchObject({
      kind: "button",
      label: source?.label,
    });
    expect((clone?.x ?? 0) > (source?.x ?? 0)).toBe(true);
    const pasted = pastePanelWidget(page, {
      id: "manual",
      kind: "value",
      label: "V",
      source: "0:256:EngineSpeed",
      digits: 2,
      x: 40,
      y: 40,
      w: 20,
      h: 6,
    });
    expect(
      pasted?.page.widgets.find((widget) => widget.id === pasted.id),
    ).toMatchObject({ kind: "value", digits: 2 });
  });

  it("round-trips numeric setpoints with their step", () => {
    const page = parsePanelPage({
      width: 160,
      height: 90,
      widgets: [
        {
          id: "numeric1",
          kind: "setpoint",
          label: "Target",
          sysvar: "Bench::Setpoint",
          digits: 2,
          step: 0.5,
          x: 8,
          y: 8,
          w: 18,
          h: 6,
        },
      ],
    });
    expect(page?.widgets[0]).toMatchObject({
      kind: "setpoint",
      sysvar: "Bench::Setpoint",
      digits: 2,
      step: 0.5,
    });
    const updated = updatePanelWidget(page!, "numeric1", { step: 2 });
    expect(updated.widgets[0]).toMatchObject({ step: 2 });
    const bad = updatePanelWidget(page!, "numeric1", { step: 0 });
    expect(bad.widgets[0]).toMatchObject({ step: 0.5 });
    const copy = duplicatePanelWidget(page!, "numeric1");
    expect(
      copy?.page.widgets.find((widget) => widget.id === copy.id),
    ).toMatchObject({ kind: "setpoint", step: 0.5 });
  });

  it("parses a book and manages pages", () => {
    const parsed = parsePanelBook({
      activeId: "p2",
      pages: [
        { id: "p1", name: "Overview", width: 160, height: 90, widgets: [] },
        { id: "p2", name: "Test", width: 120, height: 80, widgets: [] },
      ],
    });
    expect(parsed?.activeId).toBe("p2");
    expect(parsed?.pages.map((page) => page.name)).toEqual(["Overview", "Test"]);

    const book = defaultPanelBook();
    const added = addPanelPage(book);
    expect(added.book.pages).toHaveLength(2);
    expect(added.book.activeId).toBe(added.id);
    const copy = duplicatePanelPage(added.book, added.id);
    expect(copy?.book.pages).toHaveLength(3);
    expect(copy?.book.pages[2].name).toBe("Page 2 copy");
    const selected = selectPanelPage(copy!.book, book.pages[0].id);
    expect(selected.activeId).toBe(book.pages[0].id);
    const renamed = renamePanelPage(selected, book.pages[0].id, " Bench ");
    expect(renamed.pages[0].name).toBe("Bench");
    expect(renamePanelPage(renamed, book.pages[0].id, "  ").pages[0].name).toBe(
      "Bench",
    );
    const removed = removePanelPage(renamed, book.pages[0].id);
    expect(removed.pages).toHaveLength(2);
    expect(removePanelPage(defaultPanelBook(), "page1").pages).toHaveLength(1);
  });

  it("lets a value card shrink to its compact floor", () => {
    const page = parsePanelPage({
      width: 160,
      height: 90,
      widgets: [
        {
          id: "value1",
          kind: "value",
          label: "Speed",
          source: "0:512:VehicleSpeed",
          digits: 1,
          x: 8,
          y: 8,
          w: 20,
          h: 6,
        },
      ],
    });
    expect(widgetMinSize("value")).toEqual({ w: 12, h: 5 });
    expect(movePanelWidget(page!, "value1", 8, 8, 12, 6)).not.toBeNull();
    const floor = movePanelWidget(page!, "value1", 8, 8, 1, 1);
    expect(floor?.widgets[0]).toMatchObject({ w: 12, h: 5 });
  });

  it("accepts FlexRay and Ethernet message targets", () => {
    const page = parsePanelPage({
      width: 160,
      height: 90,
      widgets: [
        {
          id: "fr",
          kind: "button",
          label: "FR",
          action: "send",
          frame: { protocol: "fr", bus: 0, slot: 25, data: "01 02" },
          x: 0,
          y: 0,
          w: 20,
          h: 6,
        },
        {
          id: "eth",
          kind: "button",
          label: "ETH",
          action: "send",
          frame: {
            protocol: "eth",
            bus: 0,
            frame: "EthFrame",
            data: "01 02",
          },
          x: 0,
          y: 6,
          w: 20,
          h: 6,
        },
      ],
    });
    expect(page?.widgets.map((widget) => widget.kind === "button" && widget.frame?.protocol)).toEqual([
      "fr",
      "eth",
    ]);
    const bad = parsePanelPage({
      width: 160,
      height: 90,
      widgets: [
        {
          id: "bad",
          kind: "button",
          label: "Bad",
          action: "send",
          frame: { protocol: "fr", bus: 0, slot: 0, data: "01" },
          x: 0,
          y: 0,
          w: 20,
          h: 6,
        },
      ],
    });
    expect(bad?.widgets).toEqual([]);
  });

  it("keeps physical frame values and validates them", () => {
    const page = parsePanelPage({
      width: 160,
      height: 90,
      widgets: [
        {
          id: "send",
          kind: "button",
          label: "Engine",
          action: "send",
          frame: {
            protocol: "can",
            channel: 0,
            id: 0x100,
            extended: false,
            data: "",
            source: "catalog",
            mode: "physical",
            values: [{ name: "EngineSpeed", value: 1000 }],
          },
          x: 0,
          y: 0,
          w: 20,
          h: 6,
        },
      ],
    });
    expect(page?.widgets[0]).toMatchObject({
      frame: {
        source: "catalog",
        mode: "physical",
        values: [{ name: "EngineSpeed", value: 1000 }],
      },
    });
    const round = importPanelFile(JSON.parse(exportPanelFile(page!)));
    expect("page" in round && round.page.widgets[0]).toMatchObject({
      frame: {
        source: "catalog",
        mode: "physical",
        values: [{ name: "EngineSpeed", value: 1000 }],
      },
    });

    const frame = (values: { name: string; value: number }[]) =>
      ({
        protocol: "can" as const,
        channel: 0,
        id: 0x100,
        extended: false,
        data: "",
        mode: "physical" as const,
        values,
      });
    expect(panelFrameProblem(frame([]))).toBeNull();
    expect(panelFrameProblem(frame([{ name: "X", value: Number.NaN }]))).toBe(
      "Value for X must be a number",
    );
    expect(
      panelFrameProblem({ ...frame([]), data: "not hex" }),
    ).toBeNull();
    expect(
      panelFrameProblem({ ...frame([]), mode: "raw" }),
    ).toBeNull();
    expect(
      panelFrameProblem({ ...frame([]), mode: "raw", data: "not hex" }),
    ).toBe("Data must be hex bytes");
  });

  it("round-trips a one-shot message button", () => {
    const page = parsePanelPage({
      width: 160,
      height: 90,
      widgets: [
        {
          id: "send",
          kind: "button",
          label: "Diagnostic",
          action: "send",
          frame: { channel: 1, id: 0x7df, extended: false, data: "02 10 03" },
          x: 0,
          y: 0,
          w: 20,
          h: 6,
        },
      ],
    });
    expect(page?.widgets[0]).toMatchObject({
      action: "send",
      frame: { channel: 1, id: 0x7df, extended: false, data: "02 10 03" },
    });
    const file = JSON.parse(exportPanelFile(page!));
    const round = importPanelFile(file);
    expect("page" in round && round.page.widgets[0]).toMatchObject({
      action: "send",
      frame: { channel: 1, id: 0x7df, extended: false, data: "02 10 03" },
    });

    // A send button without a valid message is not a usable control.
    const bad = parsePanelPage({
      width: 160,
      height: 90,
      widgets: [
        {
          id: "bad",
          kind: "button",
          label: "Bad",
          action: "send",
          frame: { channel: 0, id: 0x800, extended: false, data: "01" },
          x: 0,
          y: 0,
          w: 20,
          h: 6,
        },
      ],
    });
    expect(bad?.widgets).toEqual([]);
    const fileBad = {
      format: PANEL_FILE_FORMAT,
      version: 1,
      page: {
        name: "Bad",
        width: 160,
        height: 90,
        widgets: [
          {
            kind: "button",
            label: "Bad",
            action: "send",
            frame: { channel: 0, id: 0x800, extended: false, data: "01" },
            x: 0,
            y: 0,
            w: 20,
            h: 6,
          },
        ],
      },
    };
    expect(importPanelFile(fileBad)).toHaveProperty("error");
  });

  it("keeps a button's signal writes through copy and file round-trips", () => {
    const page = parsePanelPage({
      width: 160,
      height: 90,
      widgets: [
        {
          id: "b",
          kind: "button",
          label: "Preset",
          action: "none",
          writes: [
            { source: "0:256:EngineSpeed", value: 1234 },
            { source: "fr:0:1:Temp", value: 2.5 },
          ],
          x: 0,
          y: 0,
          w: 20,
          h: 6,
        },
      ],
    });
    const button = page?.widgets[0];
    if (!page || button?.kind !== "button") throw new Error("button");
    const writes = button.writes;
    expect(button).toMatchObject({
      kind: "button",
      writes: [
        { source: "0:256:EngineSpeed", value: 1234 },
        { source: "fr:0:1:Temp", value: 2.5 },
      ],
    });
    const copy = pastePanelWidget(page, button);
    expect(copy?.page.widgets[1]).toMatchObject({ kind: "button", writes });
    const round = importPanelFile(JSON.parse(exportPanelFile(page)));
    if (!("page" in round)) throw new Error("round-trip");
    const imported = round.page.widgets[0];
    expect(imported.kind === "button" ? imported.writes : undefined).toEqual(
      writes,
    );

    // A write without a finite value is a malformed file, not a row to drop.
    const malformed = {
      format: PANEL_FILE_FORMAT,
      version: 1,
      page: {
        name: "Bad",
        width: 160,
        height: 90,
        widgets: [
          {
            kind: "button",
            label: "Bad",
            action: "none",
            x: 0,
            y: 0,
            w: 20,
            h: 6,
            writes: [{ source: "0:1:S" }],
          },
        ],
      },
    };
    expect(importPanelFile(malformed)).toHaveProperty("error");
  });

  it("accepts write and script actions only", () => {
    // A write-only preset is a valid button, not an unknown action.
    const none = parsePanelPage({
      width: 160,
      height: 90,
      widgets: [
        {
          id: "w",
          kind: "button",
          label: "Preset",
          action: "none",
          writes: [{ source: "0:256:EngineSpeed", value: 50 }],
          x: 0,
          y: 0,
          w: 20,
          h: 6,
        },
      ],
    });
    expect(none?.widgets[0]).toMatchObject({ action: "none" });
    // Simulation controls live in the top bar; old command buttons load away.
    const legacy = parsePanelPage({
      width: 160,
      height: 90,
      widgets: [
        {
          id: "old",
          kind: "button",
          label: "Start",
          action: "start",
          x: 0,
          y: 0,
          w: 20,
          h: 6,
        },
      ],
    });
    expect(legacy?.widgets).toEqual([]);
  });

  it("round-trips lamp, bar and gauge widgets", () => {
    const page = parsePanelPage({
      width: 160,
      height: 90,
      widgets: [
        {
          id: "bar1",
          kind: "bar",
          label: "Speed",
          source: "0:512:VehicleSpeed",
          min: -10,
          max: 120,
          digits: 2,
          x: 0,
          y: 0,
          w: 20,
          h: 7,
          bands: [
            { upTo: 0, color: "warn" },
            { upTo: null, color: "ok" },
          ],
        },
        {
          id: "gauge1",
          kind: "gauge",
          label: "Temp",
          source: "0:256:EngineTemp",
          min: 0,
          max: 100,
          digits: 1,
          x: 0,
          y: 7,
          w: 18,
          h: 11,
        },
        {
          id: "lamp1",
          kind: "lamp",
          label: "Ready",
          source: "sysvar:Bench::Ready",
          threshold: 0.5,
          color: "warn",
          x: 20,
          y: 0,
          w: 14,
          h: 4,
        },
      ],
    });
    expect(page?.widgets).toHaveLength(3);
    expect(page?.widgets[0]).toMatchObject({
      kind: "bar",
      min: -10,
      max: 120,
      digits: 2,
    });
    expect(
      page?.widgets[0].kind === "bar" ? page.widgets[0].bands : null,
    ).toEqual([
      { upTo: 0, color: "warn" },
      { upTo: null, color: "ok" },
    ]);
    expect(page?.widgets[1]).toMatchObject({ kind: "gauge", max: 100 });
    expect(page?.widgets[2]).toMatchObject({
      kind: "lamp",
      threshold: 0.5,
      color: "warn",
    });
    const copy = pastePanelWidget(page!, page!.widgets[0]);
    expect(copy?.page.widgets.find((widget) => widget.id === copy.id)).toMatchObject(
      { kind: "bar", min: -10, max: 120 },
    );
    expect(
      addPanelWidget(page!, "gauge")?.page.widgets.some(
        (widget) => widget.kind === "gauge",
      ),
    ).toBe(true);
  });

  it("carries action arguments and setpoint targets", () => {
    const page = parsePanelPage({
      width: 160,
      height: 90,
      widgets: [
        {
          id: "b",
          kind: "button",
          label: "Set",
          action: "script",
          script: "Set speed",
          argument: 42,
          x: 0,
          y: 0,
          w: 20,
          h: 6,
        },
        {
          id: "n",
          kind: "setpoint",
          label: "N",
          sysvar: "Bench::X",
          digits: 1,
          step: 2,
          panelAction: "Set speed",
          x: 0,
          y: 6,
          w: 18,
          h: 6,
        },
        {
          id: "s",
          kind: "setpoint",
          label: "Speed",
          target: "signal",
          source: "0:256:EngineSpeed",
          digits: 0,
          step: 5,
          x: 0,
          y: 12,
          w: 18,
          h: 6,
        },
      ],
    });
    expect(page?.widgets[0]).toMatchObject({
      action: "script",
      script: "Set speed",
      argument: 42,
    });
    expect(page?.widgets[1]).toMatchObject({
      target: "sysvar",
      panelAction: "Set speed",
      step: 2,
    });
    expect(page?.widgets[2]).toMatchObject({
      kind: "setpoint",
      target: "signal",
      source: "0:256:EngineSpeed",
      step: 5,
    });
    const copy = pastePanelWidget(page!, page!.widgets[2]);
    expect(
      copy?.page.widgets.find((widget) => widget.id === copy.id),
    ).toMatchObject({
      kind: "setpoint",
      target: "signal",
      source: "0:256:EngineSpeed",
    });
    expect(updatePanelWidget(page!, "b", { argument: 7.5 }).widgets[0]).toMatchObject(
      { argument: 7.5 },
    );
    expect(
      updatePanelWidget(page!, "n", { panelAction: "Reset" }).widgets[1],
    ).toMatchObject({ panelAction: "Reset" });
    expect(
      updatePanelWidget(page!, "n", {
        target: "signal",
        source: "0:256:EngineTemp",
      }).widgets[1],
    ).toMatchObject({ target: "signal", source: "0:256:EngineTemp" });
    expect(
      PANEL_WIDGET_KINDS.map((entry) => String(entry.kind)),
    ).toContain("setpoint");
    expect(panelWidgetSpec("setpoint").label).toBe("Setpoint");
  });

  it("exports and imports page files strictly", () => {
    const page = parsePanelPage({
      width: 160,
      height: 90,
      widgets: [
        {
          id: "b",
          kind: "button",
          label: "Preset",
          action: "none",
          x: 8,
          y: 8,
          w: 20,
          h: 6,
        },
      ],
    });
    if (!page) throw new Error("page");
    const file = JSON.parse(exportPanelFile(page));
    expect(file).toMatchObject({ format: PANEL_FILE_FORMAT, version: 1 });
    expect(file.page.widgets[0]).not.toHaveProperty("id");
    const result = importPanelFile(file);
    const imported = "page" in result ? result.page : null;
    expect(imported?.widgets).toHaveLength(1);
    expect(imported?.width).toBe(PAGE_WIDTH);
    const added = addPanelPageFromFile(defaultPanelBook(), imported!);
    expect(added.book.pages).toHaveLength(2);
    expect(added.book.activeId).toBe(added.page.id);
    expect(added.page.name).toBe("Overview 2");
    expect(panelFileName("Brake / Lamp")).toBe("brake-lamp.rxpanel");
    expect(panelFileName("  ")).toBe("panel.rxpanel");

    const errorOf = (raw: unknown) => {
      const parsed = importPanelFile(raw);
      return "error" in parsed ? parsed.error : "";
    };
    const fileOf = (widgets: unknown[], width = 160, height = 90) => ({
      format: PANEL_FILE_FORMAT,
      version: 1,
      page: { name: "P", width, height, widgets },
    });
    expect(errorOf(null)).toBe("Not a Panel file");
    expect(errorOf({ format: "x", version: 1 })).toBe("Not a Panel file");
    expect(
      errorOf({ format: PANEL_FILE_FORMAT, version: 9, page: {} }),
    ).toBe("Unsupported Panel file version 9");
    expect(
      errorOf(fileOf([{ kind: "hologram", x: 0, y: 0, w: 10, h: 5 }])),
    ).toBe('Control 1: unknown type "hologram"');
    expect(
      errorOf(fileOf([{ kind: "lamp", x: 150, y: 0, w: 14, h: 4 }])),
    ).toContain("outside");
    expect(
      errorOf(fileOf([{ kind: "lamp", x: 0, y: 0, w: 8, h: 4 }])),
    ).toContain("at least 10 × 3");
    expect(
      errorOf(
        fileOf([
          { kind: "button", action: "none", x: 0, y: 0, w: 20, h: 6 },
          { kind: "lamp", x: 0, y: 0, w: 14, h: 4 },
        ]),
      ),
    ).toContain("overlaps");
    expect(
      errorOf(fileOf([{ kind: "button", action: "launch", x: 0, y: 0, w: 20, h: 6 }])),
    ).toContain("unknown action");
    expect(errorOf(fileOf([], 1000, 90))).toContain("Canvas must be");
    expect("error" in importPanelFile(fileOf([]))).toBe(false);
  });
});
