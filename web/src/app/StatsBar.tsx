//! The statistics strip and the replay transport row above the workspace.

import type { State } from "../types";
import { UiIcon } from "../components/icons";
import { baseName, clamp, seconds } from "./support";

export interface StatsBarProps {
  state: State | null;
  error: string;
  recordActive: boolean;
  recordArmed: boolean;
  replaying: boolean;
  attachedBuses: number;
  busLive: boolean;
}

export function StatsBar({
  state,
  error,
  recordActive,
  recordArmed,
  replaying,
  attachedBuses,
  busLive,
}: StatsBarProps) {
  return (
    <section class="stats">
      <div class="stat">
        <span>Frames</span>
        <strong>{state?.frames.toLocaleString() ?? "—"}</strong>
      </div>
      <div class="stat">
        <span>Frame rate</span>
        <strong>{state ? `${state.fps.toFixed(0)} f/s` : "—"}</strong>
      </div>
      <div class="stat">
        <span>Trace ring</span>
        <strong>{state?.traceLen.toLocaleString() ?? "—"}</strong>
      </div>
      <div class="stat">
        <span>Sim time</span>
        <strong>{state ? `${seconds(state.simTimeUs, 2)} s` : "—"}</strong>
      </div>
      <div
        class="stat"
        title={
          recordActive
            ? `Recording to ${
                state?.recordFile ? baseName(state.recordFile) : "a new file"
              }`
            : recordArmed
              ? "Recording is armed; Start opens a new file"
              : "Recording is off"
        }
      >
        <span>Record</span>
        <strong class={recordActive ? "rec" : recordArmed ? "armed" : ""}>
          {!state ? "—" : recordActive ? "REC" : recordArmed ? "Armed" : "Off"}
        </strong>
      </div>
      <div class="stat">
        <span>Bus</span>
        <strong>
          {!state
            ? "—"
            : replaying
              ? "—"
              : attachedBuses === 0
                ? "Simulated"
                : busLive
                  ? `Real · ${attachedBuses}`
                  : `Simulated · ${attachedBuses}`}
        </strong>
      </div>
      {error && <div class="error-pill">{error}</div>}
    </section>
  );
}

export interface ReplayBarProps {
  state: State;
  replayText: string;
  replayDropped: number;
  replayStepMs: number;
  replayForS: number;
  replayTo: string;
  prevTriggerUs: number | null;
  nextTriggerUs: number | null;
  onSpeed: (speed: number) => void;
  onStepChange: (ms: number) => void;
  onForChange: (seconds: number) => void;
  onStepForward: () => void;
  onPlayFor: () => void;
  onPlayTo: (seconds: number) => void;
  onToChange: (text: string) => void;
  onOpenChannels: () => void;
  onViewTime: (us: number) => void;
  onGoTo: (us: number) => void;
}

export function ReplayBar({
  state,
  replayText,
  replayDropped,
  replayStepMs,
  replayForS,
  replayTo,
  prevTriggerUs,
  nextTriggerUs,
  onSpeed,
  onStepChange,
  onForChange,
  onStepForward,
  onPlayFor,
  onPlayTo,
  onToChange,
  onOpenChannels,
  onViewTime,
  onGoTo,
}: ReplayBarProps) {
  return (
    <section class="replaybar">
      <span class="replay-mode">Replay</span>
      <span class="replay-pos">{replayText || "no position"}</span>
      <button
        type="button"
        class="mini replay-channels"
        title="Map this recording's channel numbers onto project buses"
        aria-label={
          replayDropped > 0 ? `Channels, ${replayDropped} not replayed` : "Channels"
        }
        onClick={onOpenChannels}
      >
        Channels
        {replayDropped > 0 && <em>{replayDropped} not replayed</em>}
      </button>
      <div class="replay-nav">
        <label>
          Speed
          <select
            value={state.replaySpeed >= 1e8 ? "fast" : String(state.replaySpeed)}
            onChange={(event) => {
              const value = (event.target as HTMLSelectElement).value;
              onSpeed(value === "fast" ? 1e9 : Number(value));
            }}
          >
            {[0.5, 1, 2, 4].map((speed) => (
              <option key={speed} value={speed}>
                {speed}x
              </option>
            ))}
            <option value="fast">Fast</option>
          </select>
        </label>
        <label>
          Step
          <select
            value={replayStepMs}
            onChange={(event) =>
              onStepChange(Number((event.target as HTMLSelectElement).value))
            }
          >
            {[10, 50, 100, 500, 1000, 5000].map((ms) => (
              <option key={ms} value={ms}>
                {ms >= 1000 ? `${ms / 1000} s` : `${ms} ms`}
              </option>
            ))}
          </select>
        </label>
        <button
          type="button"
          class="icon-btn"
          disabled={!state.measuring}
          title="Advance the replay by the step size (forward only)"
          aria-label="Advance one step"
          onClick={onStepForward}
        >
          <UiIcon name="step-forward" />
        </button>
        {state.markers.length > 0 && (
          <>
            <button
              type="button"
              class="icon-btn"
              disabled={prevTriggerUs === null}
              title="Show the previous trigger in Trace and the chart; playback keeps running"
              aria-label="Previous trigger"
              onClick={() => prevTriggerUs !== null && onViewTime(prevTriggerUs)}
            >
              <UiIcon name="arrow-left" />
            </button>
            <button
              type="button"
              class="icon-btn"
              disabled={nextTriggerUs === null}
              title="Show the next trigger in Trace and the chart; playback keeps running"
              aria-label="Next trigger"
              onClick={() => nextTriggerUs !== null && onViewTime(nextTriggerUs)}
            >
              <UiIcon name="arrow-right" />
            </button>
          </>
        )}
        <label>
          Play for
          <select
            value={replayForS}
            onChange={(event) =>
              onForChange(Number((event.target as HTMLSelectElement).value))
            }
          >
            {[1, 5, 10, 30, 60].map((s) => (
              <option key={s} value={s}>
                {s} s
              </option>
            ))}
          </select>
        </label>
        <button
          type="button"
          disabled={!state.measuring}
          title={`Play ${replayForS} s, then pause`}
          onClick={onPlayFor}
        >
          Play {replayForS} s
        </button>
        <form
          class="replay-to"
          onSubmit={(event) => {
            event.preventDefault();
            const secs = Number(replayTo);
            if (Number.isFinite(secs) && secs >= 0) onPlayTo(secs);
          }}
        >
          <label>
            to
            <input
              value={replayTo}
              placeholder="s"
              inputMode="decimal"
              onInput={(event) =>
                onToChange((event.target as HTMLInputElement).value)
              }
            />
          </label>
          <button
            type="submit"
            disabled={
              !state.measuring ||
              !replayTo.trim() ||
              !Number.isFinite(Number(replayTo))
            }
            title="Play forward to this absolute log time, then pause"
          >
            Go
          </button>
        </form>
      </div>
      {state.replayPosUs != null &&
        state.replayTotalUs != null &&
        state.replayTotalUs > 0 && (
          <div class="replay-track-row">
            <div
              class="replay-track"
              title="Click ahead to play there; clicking behind only moves the view"
              onClick={(event) => {
                const rect = event.currentTarget.getBoundingClientRect();
                const frac = clamp(
                  (event.clientX - rect.left) / rect.width,
                  0,
                  1,
                );
                onGoTo(Math.round(frac * state.replayTotalUs!));
              }}
            >
              <span
                class="replay-fill"
                style={{
                  width: `${(state.replayPosUs / state.replayTotalUs) * 100}%`,
                }}
              />
              {state.markers.map((marker) => (
                <span
                  key={marker}
                  class="replay-mark"
                  style={{
                    left: `${clamp(
                      ((marker * 1e6) / state.replayTotalUs!) * 100,
                      0,
                      100,
                    )}%`,
                  }}
                />
              ))}
              <span
                class="replay-cursor"
                style={{
                  left: `${(state.replayPosUs / state.replayTotalUs) * 100}%`,
                }}
              />
            </div>
            <span class="replay-hint">forward only · pauses at the target</span>
          </div>
        )}
    </section>
  );
}
