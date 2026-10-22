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
  projectPath: string;
  dirty: boolean;
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
  triggers: TriggerEntry[];
  triggerLimits: TriggerLimits;
  sysvars: SysVar[];
  monitor: MonitorRow[];
  data: DataRow[];
  stateRows: StateRow[];
  stateWindowS: number;
  stats: StatsRow[];
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

export interface TriggerEntry {
  summary: string;
  action: string;
  kind: string;
  enabled: boolean;
  fired: number;
  level: boolean;
  lastFireUs: number;
  ch: number;
  id: number;
  ext: boolean;
  signal: string;
  threshold: number;
  rising: boolean;
  bus: number;
  slot: number;
  varKey: string;
  actionKind: string;
  sendCh: number;
  sendId: number;
}

export interface SysVar {
  namespace: string;
  name: string;
  value: number;
  init: number;
  min: number | null;
  max: number | null;
  unit: string;
  comment: string;
}

export interface MonitorRow {
  ch: number;
  id: number;
  ext: boolean;
  name: string;
  label: string;
  bus: string;
  value: number;
  unit: string;
  digits: number;
  ruleOn: boolean;
  rising: boolean;
  threshold: number;
  color: number;
}

export interface DataRow {
  ch: number;
  id: number;
  ext: boolean;
  name: string;
  bus: string;
  value: string;
  unit: string;
  raw: string;
  min: string;
  avg: string;
  max: string;
  frac: number;
  visible: boolean;
}

export interface StateBand {
  start: number;
  end: number;
  label: string;
  color: number;
}

export interface StateRow {
  ch: number;
  id: number;
  ext: boolean;
  name: string;
  bus: string;
  current: string;
  bands: StateBand[];
}

export interface StatsRow {  label: string;
  bus: string;
  count: number;
  minMs: string;
  avgMs: string;
  maxMs: string;
  jitterMs: string;
  share: string;
  len: number;
}

export interface TriggerLimits {
  preFrames: number;
  postFrames: number;
  markerCap: number;
}

export interface TriggerEdit {
  index: number;
  kind: string;
  ch: number;
  id: number;
  ext: boolean;
  signal: string;
  threshold: number;
  rising: boolean;
  bus: number;
  slot: number;
  key: string;
  action: string;
  sendCh: number;
  sendId: number;
}

export interface TxSource {
  name: string;
  kind: string;
  lo: number;
  hi: number;
  periodUs: number;
  phaseUs: number;
  seq: number[];
  seed: number;
  redrawUs: number;
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
  srcs: TxSource[];
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
  | { type: "save_project"; path: string | null }
  | { type: "set_sys_var"; namespace: string; name: string; value: number }
  | {
      type: "define_sys_var";
      namespace: string;
      name: string;
      init: number;
      min: number | null;
      max: number | null;
      unit: string;
      comment: string;
      old_namespace: string | null;
      old_name: string | null;
    }
  | { type: "delete_sys_var"; namespace: string; name: string }
  | { type: "add_monitor"; ch: number; id: number; ext: boolean; name: string }
  | { type: "remove_monitor"; ch: number; id: number; ext: boolean; name: string }
  | { type: "clear_monitor" }
  | {
      type: "edit_monitor";
      ch: number;
      id: number;
      ext: boolean;
      name: string;
      digits?: number;
      rule_on?: boolean;
      rising?: boolean;
      threshold?: number;
      color?: number;
    }
  | { type: "add_data"; ch: number; id: number; ext: boolean; name: string }
  | { type: "remove_data"; ch: number; id: number; ext: boolean; name: string }
  | { type: "clear_data" }
  | {
      type: "set_data_visible";
      ch: number;
      id: number;
      ext: boolean;
      name: string;
      visible: boolean;
    }
  | { type: "add_state"; ch: number; id: number; ext: boolean; name: string }
  | { type: "remove_state"; ch: number; id: number; ext: boolean; name: string }
  | { type: "clear_state" }
  | { type: "set_state_window"; seconds: number }
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
  | { type: "set_entry_source"; ch: number; id: number; src: TxSource }
  | { type: "clear_entry_source"; ch: number; id: number; name: string }
  | { type: "add_entry"; ch: number; id: number }
  | { type: "remove_entry"; ch: number; id: number }
  | { type: "simulate_bus_nodes"; ch: number }
  | { type: "add_trigger"; kind: string }
  | { type: "set_trigger_enabled"; index: number; on: boolean }
  | { type: "remove_trigger"; index: number }
  | { type: "rearm_triggers" }
  | {
      type: "edit_trigger";
      index: number;
      kind: string;
      ch: number;
      id: number;
      ext: boolean;
      signal: string;
      threshold: number;
      rising: boolean;
      bus: number;
      slot: number;
      key: string;
      action: string;
      send_ch: number;
      send_id: number;
    }
  | {
      type: "set_trigger_limits";
      pre_frames: number;
      post_frames: number;
      marker_cap: number;
    };
