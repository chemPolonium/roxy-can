export interface State {
  measuring: boolean;
  tracePaused: boolean;
  recording: boolean;
  recordFile: string;
  recordFilter: string;
  mode: string;
  markers: number[];
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
  replaySpeed: number;
  recentProjects: string[];
  recentLogs: string[];
  workspace: unknown;
  workspaceRev: number;
  reference: {
    name: string;
    signals: {
      key: string;
      name: string;
      bus: string;
      unit: string;
      points: [number, number][];
    }[];
  } | null;
  referenceRev: number;
  derived: DerivedDef[];
  replayMap: ReplayMap;
  buses: Bus[];
  arxmlScan?: ArxmlScan;
  realBus: boolean;
  hwRefresh: number;
  tx: TxEntry[];
  frTx: FrTxEntry[];
  blocks: ReplayBlock[];
  triggers: TriggerEntry[];
  triggerLimits: TriggerLimits;
  sysvars: SysVar[];
  monitor: MonitorRow[];
  data: DataRow[];
  stateRows: StateRow[];
  stateWindowS: number;
  scripts: ScriptInfo[];
  busStats: BusStats[];
  specRows: SpecRow[];
  specTolerance: number;
  specGrace: number;
  specPeriodic: number;
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
  /** Read-only: subscribe and show at cursors without drawing a lane. */
  read: boolean;
  /** `[seconds, value]`, oldest first. */
  points: [number, number][];
}

export interface CatalogSignal {
  key: string;
  name: string;
  unit: string;
  min: number;
  max: number;
  /// Declared start value in physical units (CAN only).
  initial?: number;
  /// `CM_ SG_` description, when the database carries one.
  comment?: string;
  /// I-SIGNAL-GROUP name, when the model declares one.
  group?: string;
}

export interface CatalogMessage {
  id: number;
  extended: boolean;
  name: string;
  frame?: string;
  /// `CM_ BO_` description, when the database carries one.
  comment?: string;
  /// CAN FD, declared by `VFrameFormat` or implied by a DLC above 8 bytes.
  fd?: boolean;
  /// Ethernet payload bytes after the project's L2-header convention.
  length?: number;
  signals: CatalogSignal[];
}

export interface CatalogChannel {
  kind: string;
  index: number;
  name: string;
  /// Cluster names the description carries, for the named FR/ETH spellings.
  aliases?: string[];
  messages: CatalogMessage[];
}

export interface Catalog {
  channels: CatalogChannel[];
}

export interface ReplayMapRow {
  raw: number;
  count: number;
  target: number | null;
}

export interface ReplayMap {
  can: ReplayMapRow[];
  fr: ReplayMapRow[];
  eth: ReplayMapRow[];
}

export interface BusDescription {
  kind: "dbc" | "arxml" | "fibex" | "manual" | string;
  path: string;
  cluster?: string;
}

export interface ArxmlScan {
  path: string;
  clusters: { kind: "can" | "flexray" | "ethernet" | string; name: string }[];
}

export interface CanBusDetail {
  bitrateKbps: number;
  fdDataKbps: number;
  nodes: BusNode[];
  attached: HwAttachment | null;
}

export interface FlexRayBusDetail {
  cluster: string;
  attached: boolean;
  channel: number;
}

export interface EthernetBusDetail {
  clusters: number;
}

export interface HwAttachment {
  driver: string;
  adapter: number;
  kbps: number;
  fd: boolean;
  canTx: boolean;
  txFail: { reason: string; count: number } | null;
}

export interface BusNode {
  name: string;
  simulated: boolean;
  entries: number;
}

export interface Bus {
  index: number;
  id: number;
  kind: "can" | "flexray" | "ethernet";
  name: string;
  frames: number;
  source: string;
  descriptions: BusDescription[];
  can?: CanBusDetail;
  flexray?: FlexRayBusDetail;
  ethernet?: EthernetBusDetail;
}

export interface HwChannel {
  driver: string;
  index: number;
  name: string;
  detail: string | null;
}

export interface Hardware {
  channels: HwChannel[];
  frChannels: HwChannel[];
  error: string;
  frError: string;
}

export interface ReplayBlock {
  id: number;
  name: string;
  channel: number;
  path: string;
  nodeFilter: string | null;
  ids: [number, boolean][];
  enabled: boolean;
  frames: number;
  error: string | null;
}

export interface FrTxEntry {
  bus: number;
  busName: string;
  slot: number;
  name: string;
  node: string;
  active: boolean;
  cycleUs: number;
  dataText: string;
  sentText: string;
  muted: boolean;
  undescribed: boolean;
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
  derivedId: string;
  actionKind: string;
  sendCh: number;
  sendId: number;
}

export interface ScriptFunctionHelp {
  name: string;
  usage: string;
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
  kind: string;
  ch: number;
  id: number;
  ext: boolean;
  frame?: string;
  name: string;
  label: string;
  bus: string;
  value: number;
  hasValue: boolean;
  unit: string;
  digits: number;
  ruleOn: boolean;
  rising: boolean;
  threshold: number;
  color: number;
}

export interface DataRow {
  kind: string;
  ch: number;
  id: number;
  ext: boolean;
  frame?: string;
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

export interface StateRule {
  cuts: number[];
  names: string[];
}

export interface DerivedSource {
  name: string;
  key: string;
}

export interface DerivedDef {
  id: string;
  name: string;
  unit: string;
  expression: string;
  cuts: number[];
  names: string[];
  sources?: DerivedSource[];
  error?: string;
}

export interface StateRow {
  kind: string;
  ch: number;
  id: number;
  ext: boolean;
  frame?: string;
  name: string;
  bus: string;
  current: string;
  bands: StateBand[];
  rule: StateRule | null;
}

export interface ScriptInfo {
  id: number;
  name: string;
  channel: number;
  attachedBus: number | null;
  attachedNode: string | null;
  enabled: boolean;
  running: boolean;
  errored: boolean;
  filePath: string;
  logLen: number;
  lastLog: string;
  panelActions: string[];
}

export interface ScriptIssue {
  line: number;
  col: number | null;
  message: string;
}

export interface ScriptTest {
  name: string;
  runs: number;
  failures: number;
  line: number;
  timeS: number;
}

export interface Script extends ScriptInfo {
  source: string;
  tests: ScriptTest[];
  log: string[];
  issue: ScriptIssue | null;
}

export interface BusStatRow {
  label: string;
  current: string;
  min: string;
  max: string;
  avg: string;
}

export interface BusStats {
  name: string;
  kbps: number;
  loadNow: number;
  loadHistory: number[];
  rows: BusStatRow[];
}

export interface SpecRow {
  name: string;
  addr: string;
  bus: string;
  rule: string;
  declared: string;
  measured: string;
  count: number;
  first: string;
  last: string;
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
  derivedId: string;
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
  brs: boolean;
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
}

export interface Frame {
  kind: string;
  addr: string;
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
  brs: boolean;
}

export interface MessageSignal {
  name: string;
  value: string;
  raw: number;
  unit: string;
  label?: string | null;
}

export interface MessageE2e {
  profile: number;
  dataId?: number | null;
  counter?: number | null;
  frames: number;
  repeats: number;
  gaps: number;
  jumps: number;
}

export interface Message {
  key: string;
  kind: string;
  addr: string;
  bus: number;
  busName: string;
  id: number;
  extended: boolean;
  name: string;
  count: number;
  cycleMs: number;
  len: number;
  data: string;
  signalCount: number;
  signals?: MessageSignal[];
  e2e?: MessageE2e;
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
  | { type: "replay_step"; delta_us: number }
  | { type: "replay_play_to"; target_us: number }
  | { type: "replay_play_for"; duration_us: number }
  | { type: "set_replay_speed"; speed: number }
  | {
      type: "set_replay_channel";
      kind: string;
      raw: number;
      target: number | null;
    }
  | { type: "record_toggle"; name: string | null; format: number | null }
  | { type: "set_record_filter"; text: string }
  | {
      type: "add_script";
      name: string;
      channel: number;
      attached_bus: number | null;
      attached_node: string | null;
    }
  | { type: "set_script_name"; id: number; name: string }
  | { type: "set_script_source"; id: number; source: string }
  | { type: "set_script_enabled"; id: number; on: boolean }
  | { type: "remove_script"; id: number }
  | { type: "save_script_file"; path: string; source: string }
  | { type: "set_spec_tolerance"; value: number }
  | { type: "set_spec_grace"; value: number }
  | { type: "clear_spec" }
  | {
      type: "export";
      kind: string;
      path: string;
      from_us?: number;
      to_us?: number;
    }
  | { type: "clear_trace" }
  | {
      type: "run_panel_action";
      name: string;
      /// Handed to a handler that declares a parameter.
      value?: number;
    }
  | { type: "panel_write_signal"; source: string; value: number }
  | {
      type: "panel_write_signals";
      writes: { source: string; value: number }[];
    }
  | {
      type: "panel_send_frame";
      channel: number;
      id: number;
      extended: boolean;
      data: number[];
      physical?: boolean;
      signals?: Record<string, number>;
    }
  | {
      type: "panel_send_fr_frame";
      bus: number;
      slot: number;
      data: number[];
      physical?: boolean;
      signals?: Record<string, number>;
    }
  | {
      type: "panel_send_eth_frame";
      bus: number;
      frame: string;
      data: number[];
      physical?: boolean;
      signals?: Record<string, number>;
    }
  | { type: "set_message_expanded"; key: string; on: boolean }
  | { type: "open_project"; path: string }
  | { type: "pick_project" }
  | { type: "save_project_as" }
  | { type: "create_sample_project" }
  | { type: "set_run_mode"; mode: "simulation" | "replay" }
  | { type: "open_log"; path: string; play?: boolean }
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
  | {
      type: "add_monitor";
      kind: string;
      ch: number;
      id: number;
      ext: boolean;
      frame?: string;
      name: string;
    }
  | {
      type: "remove_monitor";
      kind: string;
      ch: number;
      id: number;
      ext: boolean;
      frame?: string;
      name: string;
    }
  | { type: "clear_monitor" }
  | {
      type: "edit_monitor";
      kind: string;
      ch: number;
      id: number;
      ext: boolean;
      frame?: string;
      name: string;
      digits?: number;
      rule_on?: boolean;
      rising?: boolean;
      threshold?: number;
      color?: number;
    }
  | {
      type: "add_data";
      kind: string;
      ch: number;
      id: number;
      ext: boolean;
      frame?: string;
      name: string;
    }
  | {
      type: "remove_data";
      kind: string;
      ch: number;
      id: number;
      ext: boolean;
      frame?: string;
      name: string;
    }
  | { type: "clear_data" }
  | {
      type: "set_data_visible";
      kind: string;
      ch: number;
      id: number;
      ext: boolean;
      frame?: string;
      name: string;
      visible: boolean;
    }
  | {
      type: "add_state";
      kind: string;
      ch: number;
      id: number;
      ext: boolean;
      frame?: string;
      name: string;
    }
  | {
      type: "remove_state";
      kind: string;
      ch: number;
      id: number;
      ext: boolean;
      frame?: string;
      name: string;
    }
  | { type: "clear_state" }
  | { type: "set_state_window"; seconds: number }
  | {
      type: "set_state_rule";
      kind: string;
      ch: number;
      id: number;
      ext: boolean;
      frame?: string;
      name: string;
      cuts: number[];
      names: string[];
    }
  | {
      type: "clear_state_rule";
      kind: string;
      ch: number;
      id: number;
      ext: boolean;
      frame?: string;
      name: string;
    }
  | {
      type: "select_signal";
      ch: number;
      id: number;
      ext: boolean;
      name: string;
      read?: boolean;
    }
  | { type: "unselect_signal"; ch: number; id: number; ext: boolean; name: string }
  | {
      type: "select_fr_signal";
      bus: number;
      slot: number;
      name: string;
      read?: boolean;
    }
  | { type: "unselect_fr_signal"; bus: number; slot: number; name: string }
  | {
      type: "select_eth_signal";
      bus: number;
      frame: string;
      name: string;
      read?: boolean;
    }
  | { type: "unselect_eth_signal"; bus: number; frame: string; name: string }
  | {
      type: "select_derived_signal";
      id: string;
      name: string;
      read?: boolean;
    }
  | { type: "unselect_derived_signal"; id: string; name: string }
  | { type: "move_signal"; key: string; to: number }
  | {
      type: "chart_signal";
      key: string;
      action: "monitor" | "data" | "state" | "remove";
    }
  | { type: "attach_hardware"; bus: number; driver: string; index: number }
  | { type: "detach_hardware"; bus: number }
  | { type: "attach_fr_hardware"; bus: number; index: number }
  | { type: "detach_fr_hardware"; bus: number }
  | { type: "refresh_hardware" }
  | { type: "set_bus_mode"; real: boolean }
  | { type: "add_bus"; kind: "can" | "flexray" | "ethernet" }
  | { type: "remove_bus"; ch: number }
  | { type: "rename_bus"; ch: number; name: string }
  | {
      type: "set_bus_rates";
      ch: number;
      bitrate_kbps: number;
      fd_data_kbps: number;
    }
  | { type: "scan_arxml"; path: string }
  | {
      type: "add_bus_description";
      bus: number;
      kind: string;
      path: string;
      cluster?: string;
    }
  | {
      type: "remove_bus_description";
      bus: number;
      kind: string;
      path: string;
      cluster?: string;
    }
  | { type: "set_node_role"; bus: number; node: string; simulated: boolean }
  | { type: "remove_fr_bus"; bus: number }
  | { type: "remove_eth_bus"; bus: number }
  | { type: "set_entry_active"; ch: number; id: number; on: boolean }
  | { type: "set_entry_cycle"; ch: number; id: number; cycle_us: number }
  | { type: "set_entry_fd"; ch: number; id: number; fd: boolean }
  | { type: "set_entry_brs"; ch: number; id: number; brs: boolean }
  | { type: "set_entry_hex"; ch: number; id: number; text: string }
  | { type: "set_entry_source"; ch: number; id: number; src: TxSource }
  | { type: "clear_entry_source"; ch: number; id: number; name: string }
  | {
      type: "add_replay_block";
      channel: number;
      name: string;
      path: string;
      node: string | null;
    }
  | {
      type: "set_replay_block";
      id: number;
      channel: number;
      name: string;
      path: string;
      node: string | null;
      ids: [number, boolean][];
    }
  | { type: "set_replay_block_enabled"; id: number; on: boolean }
  | { type: "remove_replay_block"; id: number }
  | { type: "add_fr_entry"; bus: number; slot: number }
  | { type: "remove_fr_entry"; bus: number; slot: number }
  | { type: "set_fr_entry_active"; bus: number; slot: number; on: boolean }
  | { type: "set_fr_entry_cycle"; bus: number; slot: number; cycle_us: number }
  | { type: "set_fr_entry_hex"; bus: number; slot: number; text: string }
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
      derived_id: string;
      action: string;
      send_ch: number;
      send_id: number;
    }
  | {
      type: "set_trigger_limits";
      pre_frames: number;
      post_frames: number;
      marker_cap: number;
    }
  | { type: "set_web_workspace"; rev: number; workspace: unknown }
  | { type: "load_reference"; path: string }
  | { type: "clear_reference" }
  | { type: "set_derived"; defs: DerivedDef[] };

export interface DebugValue {
  name: string;
  value: string;
}

export interface DebugFrame {
  function: string;
  line: number;
}

export interface DebugView {
  enabled: boolean;
  node: number | null;
  paused: boolean;
  line: number | null;
  function: string | null;
  breakpoints: number[];
  locals: DebugValue[];
  globals: DebugValue[];
  stack: DebugFrame[];
  note: string | null;
}

export type DebugCommand =
  | { action: "arm"; node_id: number; lines: number[] }
  | { action: "set_breakpoints"; lines: number[] }
  | { action: "continue" }
  | { action: "step" }
  | { action: "pause" }
  | { action: "stop" };
