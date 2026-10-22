export interface State {
  measuring: boolean;
  tracePaused: boolean;
  mode: string;
  frames: number;
  fps: number;
  traceLen: number;
  simTimeUs: number;
  status: string;
  channels: Channel[];
  trace: Frame[];
  messages: Message[];
  write: WriteLine[];
  series: number[];
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
  | { type: "clear_trace" };
