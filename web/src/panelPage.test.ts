import { describe, expect, it } from "vitest";
import {
  PAGE_HEIGHT,
  PAGE_MAX_W,
  PAGE_WIDTH,
  PANEL_WIDGET_KINDS,
  addPanelButton,
  addPanelPage,
  addPanelWidget,
  defaultPanelBook,
  defaultPanelPage,
  duplicatePanelPage,
  duplicatePanelWidget,
  formatPanelValue,
  movePanelWidget,
  panelActionCommand,
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
  it("starts from a fixed 8px canvas with operation buttons", () => {
    const page = defaultPanelPage();
    expect(page.width).toBe(PAGE_WIDTH);
    expect(page.height).toBe(PAGE_HEIGHT);
    expect(
      page.widgets.map((widget) =>
        widget.kind === "button" ? widget.action : widget.kind,
      ),
    ).toEqual(["start", "stop", "clear_trace"]);
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
          action: "start",
          x: 10,
          y: 10,
          w: 20,
          h: 6,
        },
        {
          id: "overlap",
          kind: "button",
          label: "Overlap",
          action: "stop",
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
          action: "stop",
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
          action: "start",
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
    const page = defaultPanelPage();
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
    const added = addPanelButton(page);
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
          action: "start" as const,
          x: 0,
          y: 0,
          w: PAGE_WIDTH,
          h: PAGE_HEIGHT,
        },
      ],
    };
    expect(addPanelButton(full)).toBeNull();
  });

  it("updates labels/actions and removes widgets", () => {
    const page = defaultPanelPage();
    const updated = updatePanelWidget(page, "start", {
      label: "Launch",
      action: "rearm_triggers",
    });
    expect(updated.widgets[0]).toMatchObject({
      label: "Launch",
      action: "rearm_triggers",
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
    const page = defaultPanelPage();
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
    const page = defaultPanelPage();
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
          kind: "numeric",
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
      kind: "numeric",
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
    ).toMatchObject({ kind: "numeric", step: 0.5 });
  });

  it("migrates a single page into a book and manages pages", () => {
    const legacy = defaultPanelPage();
    const migrated = parsePanelBook(undefined, legacy);
    expect(migrated?.pages).toHaveLength(1);
    expect(migrated?.pages[0].name).toBe("Overview");

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

  it("maps page actions to existing commands only", () => {
    expect(panelActionCommand("start")).toEqual({ type: "start" });
    expect(panelActionCommand("pause")).toEqual({ type: "pause", on: true });
    expect(panelActionCommand("resume")).toEqual({ type: "pause", on: false });
    expect(panelActionCommand("clear_trace")).toEqual({ type: "clear_trace" });
    expect(panelActionCommand("rearm_triggers")).toEqual({
      type: "rearm_triggers",
    });
  });
});
