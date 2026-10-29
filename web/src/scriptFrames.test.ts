import { describe, expect, it } from "vitest";
import { frameOrigins, parseFramePath } from "./scriptFrames";

describe("frame origins", () => {
  it("normalizes every frame(...) path spelling", () => {
    expect(parseFramePath("CAN1.EngineStatus")).toBe("CAN1.EngineStatus");
    expect(parseFramePath("EngineStatus")).toBe("EngineStatus");
    expect(parseFramePath('CAN1["Frame Name"]')).toBe("CAN1.Frame Name");
    expect(parseFramePath('"Frame Name"')).toBe("Frame Name");
    expect(parseFramePath("CAN1.A.B")).toBeNull();
  });

  it("follows let, re-assignment and direct arguments", () => {
    const origins = frameOrigins(`
      on start {
        let a = frame(CAN1.EngineStatus);
        b = frame(CAN1["Frame Name"]);
        send(a);
      }
      fn wrap(cmd) { send_frame(cmd); }
      on start { wrap(frame(EngineStatus)); }
    `);
    expect(origins.get("a")).toBe("CAN1.EngineStatus");
    expect(origins.get("b")).toBe("CAN1.Frame Name");
    expect(origins.get("cmd")).toBe("EngineStatus");
  });

  it("keeps ambiguous parameters silent", () => {
    const origins = frameOrigins(`
      fn send(cmd) { send_frame(cmd); }
      on start {
        send(frame(CAN1.EngineStatus));
        send(frame(CAN1.BrakeInfo));
      }
    `);
    expect(origins.has("cmd")).toBe(false);
  });

  it("propagates a frame through a forwarding helper", () => {
    const origins = frameOrigins(`
      fn outer(item) { inner(item); }
      fn inner(cmd) { send_frame(cmd); }
      on start { outer(frame(CAN1.EngineStatus)); }
    `);
    expect(origins.get("item")).toBe("CAN1.EngineStatus");
    expect(origins.get("cmd")).toBe("CAN1.EngineStatus");
  });
});
