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
  buses: Bus[];
  realBus: boolean;
  hwRefresh: number;
  tx: TxEntry[];
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

export interface HwAttachment {
  driver: string;
  adapter: number;
  kbps: number;
  fd: boolean;
  canTx: boolean;
  txFail: { reason: string; count: number } | null;
}

export interface Bus {
  index: number;
  name: string;
  dbcPaths: string[];
  bitrateKbps: number;
  fdDataKbps: number;
  attached: HwAttachment | null;
}

export interface HwChannel {
  driver: string;
  index: number;
  name: string;
  detail: string | null;
}

export interface Hardware {
  channels: HwChannel[];
  error: string;
}

export interface TxEntry {
  channel: number;
  bus: string;
  id: number;
  name: string;
  node: string;
  active: boolean;
  gateOpen: boolean;
  fd: boolean;
  cycleUs: number;
  dataText: string;
  sentText: string;
  muted: boolean;
  sources: number;
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
  | { type: "unselect_signal"; ch: number; id: number; ext: boolean; name: string }
  | { type: "attach_hardware"; bus: number; driver: string; index: number }
  | { type: "detach_hardware"; bus: number }
  | { type: "refresh_hardware" }
  | { type: "set_bus_mode"; real: boolean }
  | { type: "add_bus" }
  | { type: "remove_bus"; ch: number }
  | { type: "rename_bus"; ch: number; name: string }
  | {
      type: "set_bus_rates";
      ch: number;
      bitrate_kbps: number;
      fd_data_kbps: number;
    }
  | { type: "load_dbc"; ch: number; path: string }
  | { type: "set_entry_active"; ch: number; id: number; on: boolean }
  | { type: "set_entry_cycle"; ch: number; id: number; cycle_us: number }
  | { type: "set_entry_fd"; ch: number; id: number; fd: boolean }
  | { type: "set_entry_hex"; ch: number; id: number; text: string }
  | { type: "add_entry"; ch: number; id: number }
  | { type: "remove_entry"; ch: number; id: number }
  | { type: "simulate_bus_nodes"; ch: number };
