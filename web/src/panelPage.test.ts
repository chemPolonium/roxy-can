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
  mergeProjectPanels,
  panelFileName,
  panelFrameProblem,
  panelSetpointTarget,
  panelWidgetSpec,
  parsePanelBook,
  parsePanelHex,
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
  type PanelFrameSend,
  type PanelPage,
  type PanelSetpoint,
} from "./panelPage";

const pageWith = (widgets: unknown[]): PanelPage =>
  parsePanelPage({ width: PAGE_WIDTH, height: PAGE_HEIGHT, widgets })!;

describe("panel page", () => {
  it("starts empty on a fixed 8px canvas with the full palette", () => {
    const page = defaultPanelPage();
    expect(page.width).toBe(PAGE_WIDTH);
    expect(page.height).toBe(PAGE_HEIGHT);
    expect(page.widgets).toEqual([]);
    expect(PANEL_WIDGET_KINDS.map((entry) => entry.kind)).toEqual([
      "button",
      "value",
      "bar",
      "gauge",
      "lamp",
      "toggle",
      "setpoint",
    ]);
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
    const page = pageWith([
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
        id: "other",
        kind: "button",
        label: "Other",
        action: "none",
        x: 80,
        y: 8,
        w: 20,
        h: 6,
      },
    ]);
    const moved = movePanelWidget(page, "start", 30, 12, 20, 6);
    expect(moved?.widgets.find((widget) => widget.id === "start")).toMatchObject(
      { x: 30, y: 12 },
    );
    const shrunk = movePanelWidget(page, "start", 8, 8, 1, 1);
    expect(shrunk?.widgets.find((widget) => widget.id === "start")).toMatchObject(
      { w: widgetMinSize("button").w, h: widgetMinSize("button").h },
    );
    expect(movePanelWidget(page, "start", 80, 8, 20, 6)).toBeNull();
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

    const full: PanelPage = {
      ...page,
      widgets: [
        {
          id: "wall",
          kind: "button",
          label: "Wall",
          action: "none",
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
    const page = pageWith([
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
    ]);
    const updated = updatePanelWidget(page, "start", {
      label: "Launch",
      action: "script",
      script: "Go",
    });
    expect(updated.widgets[0]).toMatchObject({
      label: "Launch",
      action: "script",
      script: "Go",
    });
    const sent = updatePanelWidget(page, "start", {
      action: "send",
      frame: {
        protocol: "can",
        channel: 0,
        id: 0x100,
        extended: false,
        data: "11 22",
      },
    });
    expect(sent.widgets[0]).toMatchObject({ action: "send" });
    const removed = removePanelWidget(updated, "start");
    expect(removed.widgets).toHaveLength(0);
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
    const empty = defaultPanelPage();
    expect(resizePanelPage(empty, 200, 120)).toMatchObject({
      width: 200,
      height: 120,
    });
    expect(resizePanelPage(empty, 40, 20)).not.toBeNull();
    const wide = pageWith([
      {
        id: "wide",
        kind: "button",
        label: "Wide",
        action: "none",
        x: 30,
        y: 10,
        w: 20,
        h: 6,
      },
    ]);
    expect(resizePanelPage(wide, 40, 20)).toBeNull();
    expect(resizePanelPage(empty, PAGE_MAX_W + 50, 100)?.width).toBe(PAGE_MAX_W);
    expect(resizePanelPage(empty, Number.NaN, 100)).toBeNull();
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
    const page = pageWith([
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
    ]);
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

  it("round-trips setpoints that write a sysvar or a signal", () => {
    const page = parsePanelPage({
      width: 160,
      height: 90,
      widgets: [
        {
          id: "sp1",
          kind: "setpoint",
          label: "Target",
          target: "sysvar",
          sysvar: "Bench::Setpoint",
          digits: 2,
          step: 0.5,
          x: 8,
          y: 8,
          w: 18,
          h: 6,
        },
        {
          id: "sp2",
          kind: "setpoint",
          label: "Signal",
          target: "signal",
          source: "0:256:EngineSpeed",
          digits: 1,
          step: 10,
          x: 8,
          y: 16,
          w: 18,
          h: 6,
        },
      ],
    });
    expect(page?.widgets[0]).toMatchObject({
      kind: "setpoint",
      target: "sysvar",
      sysvar: "Bench::Setpoint",
      digits: 2,
      step: 0.5,
    });
    expect(page?.widgets[1]).toMatchObject({
      kind: "setpoint",
      target: "signal",
      source: "0:256:EngineSpeed",
      step: 10,
    });
    const legacy = { ...(page!.widgets[0] as PanelSetpoint), target: undefined };
    expect(panelSetpointTarget(legacy)).toBe("sysvar");
    const updated = updatePanelWidget(page!, "sp1", { step: 2 });
    expect(updated.widgets[0]).toMatchObject({ step: 2 });
    const bad = updatePanelWidget(page!, "sp1", { step: 0 });
    expect(bad.widgets[0]).toMatchObject({ step: 0.5 });
    const copy = duplicatePanelWidget(page!, "sp2");
    expect(
      copy?.page.widgets.find((widget) => widget.id === copy.id),
    ).toMatchObject({ kind: "setpoint", target: "signal", step: 10 });
  });

  it("parses and manages a page book", () => {
    expect(parsePanelBook(undefined)).toBeNull();
    expect(parsePanelBook({ pages: [] })).toBeNull();

    const parsed = parsePanelBook({
      activeId: "p2",
      pages: [
        { id: "p1", name: "Overview", width: 160, height: 90, widgets: [] },
        { id: "p2", name: "Test", width: 120, height: 80, widgets: [] },
      ],
    });
    expect(parsed?.activeId).toBe("p2");
    expect(parsed?.pages.map((page) => page.name)).toEqual(["Overview", "Test"]);

    const fallback = parsePanelBook({
      activeId: "missing",
      pages: [{ id: "p1", name: "One", widgets: [] }],
    });
    expect(fallback?.activeId).toBe("p1");

    const deduped = parsePanelBook({
      pages: [
        { id: "same", name: "Same", widgets: [] },
        { id: "same", name: "Same", widgets: [] },
      ],
    });
    expect(deduped?.pages.map((page) => page.id)).toEqual(["same", "page2"]);
    expect(deduped?.pages.map((page) => page.name)).toEqual(["Same", "Same 2"]);

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

  it("parses hex and checks send frames", () => {
    expect(parsePanelHex("11 22,33")).toEqual([0x11, 0x22, 0x33]);
    expect(parsePanelHex("1122")).toEqual([0x11, 0x22]);
    expect(parsePanelHex("")).toEqual([]);
    expect(parsePanelHex("1")).toBeNull();
    expect(parsePanelHex("zz")).toBeNull();

    const frame: PanelFrameSend = {
      protocol: "can",
      channel: 0,
      id: 0x100,
      extended: false,
      data: "11 22",
    };
    expect(panelFrameProblem(frame)).toBeNull();
    expect(panelFrameProblem({ ...frame, id: 0x800 })).toContain(
      "Standard IDs",
    );
    expect(
      panelFrameProblem({ ...frame, extended: true, id: 0x20000000 }),
    ).toContain("Extended IDs");
    expect(panelFrameProblem({ ...frame, data: "11 22 33 44 55 66 77 88 99" })).toContain(
      "0-8",
    );
    expect(
      panelFrameProblem({ protocol: "fr", bus: 0, slot: 0, data: "00" }),
    ).toContain("Slot");
    expect(
      panelFrameProblem({ protocol: "eth", bus: 0, frame: "", data: "" }),
    ).toBe("Select an Ethernet frame");
  });

  it("keeps a saved button on the action it names", () => {
    const scripted = pageWith([
      {
        id: "b",
        kind: "button",
        label: "B",
        action: "script",
        script: "Go",
        x: 0,
        y: 0,
        w: 20,
        h: 6,
      },
    ]);
    expect(scripted.widgets[0]).toMatchObject({ action: "script", script: "Go" });

    const sent = pageWith([
      {
        id: "b",
        kind: "button",
        label: "B",
        action: "send",
        frame: {
          protocol: "can",
          channel: 0,
          id: 0x100,
          extended: false,
          data: "11 22",
        },
        x: 0,
        y: 0,
        w: 20,
        h: 6,
      },
    ]);
    expect(sent.widgets[0]).toMatchObject({
      action: "send",
      frame: { protocol: "can", data: "11 22" },
    });

    const broken = pageWith([
      {
        id: "b",
        kind: "button",
        label: "B",
        action: "send",
        frame: { protocol: "can", channel: 0, id: 0x100, data: "123" },
        x: 0,
        y: 0,
        w: 20,
        h: 6,
      },
    ]);
    expect(broken.widgets).toHaveLength(0);
  });

  it("exports and imports page files strictly", () => {
    const page = pageWith([
      {
        id: "button1",
        kind: "button",
        label: "Go",
        action: "none",
        x: 0,
        y: 0,
        w: 20,
        h: 6,
      },
      {
        id: "value1",
        kind: "value",
        label: "Speed",
        source: "0:256:EngineSpeed",
        digits: 1,
        x: 0,
        y: 8,
        w: 20,
        h: 6,
      },
      {
        id: "toggle1",
        kind: "toggle",
        label: "Ready",
        sysvar: "Bench::Ready",
        x: 0,
        y: 16,
        w: 16,
        h: 5,
      },
    ]);
    const file = JSON.parse(exportPanelFile(page));
    expect(file).toMatchObject({ format: PANEL_FILE_FORMAT, version: 1 });
    expect(file.page.widgets[0]).not.toHaveProperty("id");
    const result = importPanelFile(file);
    const imported = "page" in result ? result.page : null;
    expect(imported?.widgets).toHaveLength(3);
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
    expect(
      errorOf(fileOf([{ kind: "button", action: "send", x: 0, y: 0, w: 20, h: 6 }])),
    ).toContain("Send message");
    expect(
      errorOf(
        fileOf([
          {
            kind: "button",
            action: "none",
            frame: { protocol: "can", channel: 0, id: 0x100, data: "00" },
            x: 0,
            y: 0,
            w: 20,
            h: 6,
          },
        ]),
      ),
    ).toContain("belongs to Send message");
    expect(errorOf(fileOf([], 1000, 90))).toContain("Canvas must be");
    expect("error" in importPanelFile(fileOf([]))).toBe(false);
  });

  it("merges project pages without duplicating names", () => {
    const book = defaultPanelBook();
    const page = book.pages[0];
    const document = JSON.parse(exportPanelFile(page));
    expect(mergeProjectPanels(book, [document]).pages).toHaveLength(
      book.pages.length,
    );

    const renamed = {
      ...document,
      page: { ...document.page, name: "Bench" },
    };
    const merged = mergeProjectPanels(book, [renamed, "not a page"]);
    expect(merged.pages.some((entry) => entry.name === "Bench")).toBe(true);
  });
});
