export type Theme = "dark" | "light";

export const THEME_KEY = "roxy-can-web.theme";

export function loadTheme(): Theme {
  try {
    return localStorage.getItem(THEME_KEY) === "light" ? "light" : "dark";
  } catch {
    return "dark";
  }
}

export function saveTheme(theme: Theme): void {
  try {
    localStorage.setItem(THEME_KEY, theme);
  } catch {
    // Private mode can reject storage; the session still switches.
  }
}

export function applyTheme(theme: Theme): void {
  document.documentElement.dataset.theme = theme;
  document
    .querySelector('meta[name="theme-color"]')
    ?.setAttribute("content", theme === "light" ? "#e9edf3" : "#0a0c10");
}

/// Canvas colors cannot inherit CSS variables, so the charts take a palette
/// object and redraw when the theme changes.
export interface Palette {
  signals: string[];
  buses: string[];
  monitors: string[];
  states: string[];
  rateGrid: string;
  rateAxis: string;
  rateTotal: string;
  rateFillTop: string;
  rateFillBottom: string;
  rateEmpty: string;
  sparkBg: string;
  sparkLine: string;
  sparkThreshold: string;
  stateBg: string;
  stateNeutral: string;
  stateLabel: string;
  stateMarker: string;
  signalEmpty: string;
  signalHint: string;
  signalMarker: string;
  signalStripe: string;
  signalLine: string;
  signalName: string;
  signalMeta: string;
  signalNoSamples: string;
  signalMidLine: string;
  signalTimeGrid: string;
  signalAxisLabel: string;
  signalTagBg: string;
  signalLive: string;
  signalHold: string;
}

const DARK: Palette = {
  signals: [
    "#4c9dff",
    "#54c08a",
    "#f2b45e",
    "#c77dff",
    "#ef6b6b",
    "#4ecdc4",
    "#ff9f43",
    "#a3e635",
  ],
  buses: ["#54c08a", "#f2b45e", "#c77dff", "#ff7b72"],
  monitors: ["#4c9dff", "#59c48a", "#e6b455", "#e17171", "#b07cf0", "#57c8d8"],
  states: [
    "#4c9dff",
    "#e6b455",
    "#59c48a",
    "#e17171",
    "#b07cf0",
    "#57c8d8",
    "#d0d7e2",
    "#c67f4a",
  ],
  rateGrid: "#1c2530",
  rateAxis: "#697687",
  rateTotal: "#4c9dff",
  rateFillTop: "rgba(76, 157, 255, 0.28)",
  rateFillBottom: "rgba(76, 157, 255, 0.01)",
  rateEmpty: "#546171",
  sparkBg: "#0b1016",
  sparkLine: "#59b6ff",
  sparkThreshold: "rgb(230 90 90 / 55%)",
  stateBg: "#0a0e13",
  stateNeutral: "#3a4c62",
  stateLabel: "#0d141b",
  stateMarker: "#56b6ff",
  signalEmpty: "#8a97a8",
  signalHint: "#5d6a7a",
  signalMarker: "#e6b455",
  signalStripe: "rgba(255, 255, 255, 0.018)",
  signalLine: "#182029",
  signalName: "#c9d6e5",
  signalMeta: "#7d8999",
  signalNoSamples: "#556273",
  signalMidLine: "#212a35",
  signalTimeGrid: "#1b232d",
  signalAxisLabel: "#5f6d7e",
  signalTagBg: "rgba(10, 12, 16, 0.86)",
  signalLive: "#54c08a",
  signalHold: "#e0a54a",
};

const LIGHT: Palette = {
  signals: [
    "#1f6fd0",
    "#1f8a5b",
    "#b27310",
    "#7b3fbf",
    "#c23838",
    "#168c85",
    "#c25a00",
    "#5b8a00",
  ],
  buses: ["#1f8a5b", "#b27310", "#7b3fbf", "#c23838"],
  monitors: ["#1f6fd0", "#1f8a5b", "#b27310", "#c23838", "#7b3fbf", "#168c85"],
  states: [
    "#1f6fd0",
    "#b27310",
    "#1f8a5b",
    "#c23838",
    "#7b3fbf",
    "#168c85",
    "#64748b",
    "#a15f00",
  ],
  rateGrid: "#e2e8f0",
  rateAxis: "#6b7788",
  rateTotal: "#1f6fd0",
  rateFillTop: "rgba(31, 111, 208, 0.18)",
  rateFillBottom: "rgba(31, 111, 208, 0.01)",
  rateEmpty: "#6b7788",
  sparkBg: "#f5f7fa",
  sparkLine: "#1f6fd0",
  sparkThreshold: "rgb(180 35 24 / 45%)",
  stateBg: "#f1f4f8",
  stateNeutral: "#c4cfdb",
  stateLabel: "#1f2937",
  stateMarker: "#1f6fd0",
  signalEmpty: "#6b7788",
  signalHint: "#8a95a5",
  signalMarker: "#b27310",
  signalStripe: "rgba(15, 23, 42, 0.025)",
  signalLine: "#e6ebf1",
  signalName: "#26313f",
  signalMeta: "#6b7788",
  signalNoSamples: "#8a95a5",
  signalMidLine: "#e6ebf1",
  signalTimeGrid: "#e0e6ee",
  signalAxisLabel: "#7a8796",
  signalTagBg: "rgba(255, 255, 255, 0.92)",
  signalLive: "#1f7a52",
  signalHold: "#a15f00",
};

export function palette(theme: Theme): Palette {
  return theme === "light" ? LIGHT : DARK;
}
