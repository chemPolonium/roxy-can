import { describe, expect, it } from "vitest";
import type { ReplayMap, ReplayMapRow } from "./types";
import {
  replayChoice,
  replayMapTotals,
  replayRowView,
  replaySelectChoice,
  replaySelectValue,
} from "./replayTargets";

const row = (raw: number, patch: Partial<ReplayMapRow> = {}): ReplayMapRow => ({
  raw,
  count: 1,
  target: raw,
  configured: false,
  ...patch,
});

const map = (can: ReplayMapRow[], fr: ReplayMapRow[] = [], eth: ReplayMapRow[] = []): ReplayMap => ({
  can,
  fr,
  eth,
});

describe("replay channel choices", () => {
  it("keeps an unconfigured row at as-is even when no bus answers", () => {
    const view = replayRowView(row(3, { target: null }));
    expect(view.choice).toBe("same");
    expect(view.unmatched).toBe(true);
    expect(replaySelectValue(view.choice)).toBe("same");
  });

  it("separates an explicit no-replay from an unmapped missing bus", () => {
    expect(replayChoice(row(3, { target: null, configured: true }))).toBe("drop");
    expect(replayChoice(row(3, { target: null, configured: false }))).toBe("same");
  });

  it("keeps the chosen bus index", () => {
    const view = replayRowView(row(1, { target: 2, configured: true }));
    expect(view.choice).toBe(2);
    expect(view.unmatched).toBe(false);
    expect(replaySelectValue(view.choice)).toBe("b2");
    expect(replaySelectChoice("b2")).toBe(2);
  });

  it("round-trips the select values, including two-digit buses", () => {
    for (const value of ["same", "drop", "b0", "b12"]) {
      expect(replaySelectValue(replaySelectChoice(value))).toBe(value);
    }
    expect(replaySelectChoice("garbage")).toBe("same");
  });

  it("marks saved rows this recording does not carry", () => {
    const view = replayRowView(row(4, { count: 0, configured: true }));
    expect(view.absent).toBe(true);
    expect(view.count).toBe(0);
  });
});

describe("replay map totals", () => {
  it("counts routed channels and the frames that land nowhere", () => {
    const totals = replayMapTotals(
      map(
        [
          row(0, { count: 12 }),
          row(1, { count: 7, target: null, configured: true }),
          row(4, { count: 5, target: null }),
        ],
        [row(2, { count: 3, target: 0, configured: true })],
      ),
    );
    expect(totals).toEqual({
      channels: 4,
      routed: 2,
      dropped: 2,
      droppedFrames: 12,
      absent: 0,
    });
  });

  it("does not warn about saved rows with no frames", () => {
    const totals = replayMapTotals(
      map([row(6, { count: 0, target: null, configured: true })]),
    );
    expect(totals.dropped).toBe(0);
    expect(totals.droppedFrames).toBe(0);
    expect(totals.absent).toBe(1);
    expect(totals.routed).toBe(0);
  });
});
