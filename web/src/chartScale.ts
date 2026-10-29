/// How the Signals chart scales each lane's value axis.
export type ChartYMode = "auto" | "memory" | "dbc";

/// What the Memory mode has seen so far, per signal.
export interface YMemory {
  min: number;
  max: number;
  lastT: number;
}

export interface YRange {
  min: number;
  max: number;
}

/// Folds every sample newer than the remembered tail into the range. The
/// range only grows: values already seen stay on the plot for the run.
export function extendYMemory(
  current: YMemory | undefined,
  points: [number, number][],
): YMemory | undefined {
  let memory = current;
  for (const [t, value] of points) {
    if (memory && t <= memory.lastT) continue;
    if (!memory) {
      memory = { min: value, max: value, lastT: t };
      continue;
    }
    memory.min = Math.min(memory.min, value);
    memory.max = Math.max(memory.max, value);
    memory.lastT = t;
  }
  return memory;
}

/// The range one lane should draw with. Auto follows the visible window;
/// Memory follows every value seen in the run; DBC follows the declared
/// communication range and falls back to the visible window without one.
export function chartYRange(
  mode: ChartYMode,
  view: YRange,
  declared: YRange | undefined,
  memory: YMemory | undefined,
): YRange {
  if (mode === "memory" && memory) {
    return { min: memory.min, max: memory.max };
  }
  if (mode === "dbc" && declared && declared.max > declared.min) {
    return declared;
  }
  return view;
}
