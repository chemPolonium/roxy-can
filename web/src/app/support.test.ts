import { describe, expect, it } from "vitest";
import { globalShortcutsBlocked, panelMeta } from "./support";

describe("global shortcut context", () => {
  it("ignores fields, contenteditable and open overlays", () => {
    expect(globalShortcutsBlocked(null, false)).toBe(false);
    expect(
      globalShortcutsBlocked({ tagName: "DIV", isContentEditable: false }, false),
    ).toBe(false);
    expect(
      globalShortcutsBlocked({ tagName: "INPUT", isContentEditable: false }, false),
    ).toBe(true);
    expect(
      globalShortcutsBlocked({ tagName: "TEXTAREA", isContentEditable: false }, false),
    ).toBe(true);
    expect(
      globalShortcutsBlocked({ tagName: "SELECT", isContentEditable: false }, false),
    ).toBe(true);
    expect(
      globalShortcutsBlocked({ tagName: "DIV", isContentEditable: true }, false),
    ).toBe(true);
    expect(globalShortcutsBlocked(null, true)).toBe(true);
  });
});

describe("panel header counts", () => {
  it("uses one noun form for zero, one and many", () => {
    expect(panelMeta("message", 0)).toBe("0 messages");
    expect(panelMeta("message", 1)).toBe("1 message");
    expect(panelMeta("message", 9)).toBe("9 messages");
  });

  it("shows filtered totals as count / total with thousands separators", () => {
    expect(panelMeta("frame", 7076, 7076)).toBe("7,076 / 7,076 frames");
    expect(panelMeta("lane", 3, 8)).toBe("3 / 8 lanes");
    expect(panelMeta("bus", 2, undefined, "buses")).toBe("2 buses");
  });
});
