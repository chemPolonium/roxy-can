export interface State {
  measuring: boolean;
  tracePaused: boolean;
  mode: string;
  frames: number;
  fps: number;
  traceLen: number;
  simTimeUs: number;
  status: string;
  project: string;
  log: string;
  logInfo: string;
  runMode: string;
  replayPosUs: number | null;
  replayTotalUs: number | null;
  recentProjects: string[];
  recentLogs: string[];
  channels: Channel[];
  messages: Message[];
  write: WriteLine[];
  series: SeriesPoint[];
}

export interface SignalSeries {
  key: string;
  name: string;
  bus: string;
  unit: string;
  color: number;
  /** `[seconds, value]`, oldest first. */
  points: [number, number][];
}

export interface CatalogSignal {
  key: string;
  name: string;
  unit: string;
  min: number;
  max: number;
}

export interface CatalogMessage {
  id: number;
  extended: boolean;
  name: string;
  signals: CatalogSignal[];
}

export interface CatalogChannel {
  name: string;
  messages: CatalogMessage[];
}

export interface Catalog {
  channels: CatalogChannel[];
}

/** Light state plus the incremental trace/signal hand-off. */
export interface Snapshot extends State {
  traceEpoch: number;
  traceSeq: number;
  traceSeed?: Frame[];
  traceDelta?: Frame[];
  signalsEpoch: number;
  signalsMaxT: number;
  signalsSeed?: SignalSeries[];
  signalsDelta?: SignalSeries[];
}

export interface SeriesPoint {
  total: number;
  busRates: number[];
  busLoads: number[];
}

export interface Channel {
  name: string;
  bitrateKbps: number;
  fdDataKbps: number;
  database: boolean;
}

export interface Frame {
  tUs: number;
  bus: number;
  busName: string;
  id: number;
  extended: boolean;
  name: string;
  dir: "Rx" | "Tx";
  len: number;
  data: string;
  fd: boolean;
}

export interface Message {
  bus: number;
  busName: string;
  id: number;
  extended: boolean;
  name: string;
  count: number;
  cycleMs: number;
  len: number;
  data: string;
}

export interface WriteLine {
  wallUs: number;
  kind: "script" | "info" | "warn" | "error";
  text: string;
}

export type Command =
  | { type: "start" }
  | { type: "stop" }
  | { type: "pause"; on: boolean }
  | { type: "clear_trace" }
  | { type: "open_project"; path: string }
  | { type: "open_log"; path: string }
  | { type: "select_signal"; ch: number; id: number; ext: boolean; name: string }
  | { type: "unselect_signal"; ch: number; id: number; ext: boolean; name: string };
