import type { Bus, TriggerEdit, TriggerEntry } from "./types";

/// Triggers shared on their own, without the project around them. Bus
/// references travel by name: a project's channel numbers are local.
export const TRIGGER_FILE_FORMAT = "roxy-can-triggers";
export const TRIGGER_FILE_VERSION = 1;

export const TRIGGER_KINDS = [
  "signal",
  "id",
  "error",
  "timeout",
  "sysvar",
  "derived",
  "fr_frame",
  "fr_signal",
  "payload",
  "eth_flow",
  "fr_timeout",
  "e2e",
  "someip",
] as const;

export const TRIGGER_ACTIONS = [
  "start_recording",
  "stop_recording",
  "send",
  "send_fr",
  "send_eth",
  "clear_trace",
  "insert_marker",
  "set_sysvar",
  "script",
] as const;

export interface TriggerFileRule {
  kind: string;
  action: string;
  enabled: boolean;
  /// Condition bus, by name. Empty for global rules (sysvar/derived).
  busName: string;
  /// Send target bus, by name. Only for send/send_fr.
  sendBusName: string;
  channel: number;
  id: number;
  ext: boolean;
  signal: string;
  threshold: number;
  rising: boolean;
  slot: number;
  key: string;
  derivedId: string;
  sendChannel: number;
  sendId: number;
  sysVarKey: string;
  sysVarValue: number | null;
  scriptName: string;
  scriptValue: number | null;
  sendFrame: string;
  sendData: string;
  someipMethod: number;
  someipClient: number;
  someipSession: number;
  someipType: number;
  someipValues: Record<string, number>;
  condSomeipMethod: number | null;
  condSomeipType: number | null;
  offset: number;
  mask: number;
  value: number;
  vlan: number | null;
  cooldownMs: number;
  maxFires: number;
  preFrames: number | null;
  postFrames: number | null;
}

export interface TriggerFile {
  format: string;
  version: number;
  triggers: TriggerFileRule[];
}

const GLOBAL_KINDS = new Set(["sysvar", "derived"]);
const BUS_KINDS = new Set([
  "fr_frame",
  "fr_signal",
  "fr_timeout",
  "eth_flow",
  "someip",
]);

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function busNameOf(buses: Bus[], index: number): string {
  return buses.find((bus) => bus.index === index)?.name ?? "";
}

function conditionBus(kind: string, trigger: TriggerEntry): number {
  if (GLOBAL_KINDS.has(kind)) return 0;
  return BUS_KINDS.has(kind) ? trigger.bus : trigger.ch;
}

export function buildTriggerFile(
  triggers: TriggerEntry[],
  buses: Bus[],
): TriggerFile {
  return {
    format: TRIGGER_FILE_FORMAT,
    version: TRIGGER_FILE_VERSION,
    triggers: triggers.map((trigger) => {
      const busIndex = conditionBus(trigger.kind, trigger);
      return {
        kind: trigger.kind,
        action: trigger.actionKind,
        enabled: trigger.enabled,
        busName: GLOBAL_KINDS.has(trigger.kind)
          ? ""
          : busNameOf(buses, busIndex),
        sendBusName:
          trigger.actionKind === "send" ||
          trigger.actionKind === "send_fr" ||
          trigger.actionKind === "send_eth" ||
          trigger.actionKind === "send_someip"
            ? busNameOf(buses, trigger.sendCh)
            : "",
        channel: busIndex,
        id: trigger.id,
        ext: trigger.ext,
        signal: trigger.signal,
        threshold: trigger.threshold,
        rising: trigger.rising,
        slot: trigger.slot,
        key: trigger.varKey,
        derivedId: trigger.derivedId,
        sendChannel: trigger.sendCh,
        sendId: trigger.sendId,
        sysVarKey: trigger.sysVarKey,
        sysVarValue: trigger.sysVarValue ?? null,
        scriptName: trigger.scriptName,
        scriptValue: trigger.scriptValue,
        sendFrame: trigger.sendFrame,
        sendData: trigger.sendData,
        someipMethod: trigger.someipMethod ?? 0,
        someipClient: trigger.someipClient ?? 0,
        someipSession: trigger.someipSession ?? 0,
        someipType: trigger.someipType ?? 0,
        someipValues: trigger.someipValues ?? {},
        condSomeipMethod: trigger.condSomeipMethod ?? null,
        condSomeipType: trigger.condSomeipType ?? null,
        offset: trigger.offset ?? 0,
        mask: trigger.mask ?? 0xff,
        value: trigger.value ?? 0,
        vlan: trigger.vlan,
        cooldownMs: trigger.cooldownMs,
        maxFires: trigger.maxFires,
        preFrames: trigger.preFrames,
        postFrames: trigger.postFrames,
      };
    }),
  };
}

function finiteNumber(
  value: unknown,
  fallback: number,
  min: number,
  max: number,
): number | null {
  if (value === undefined || value === null) return fallback;
  if (typeof value !== "number" || !Number.isFinite(value)) return null;
  if (value < min || value > max) return null;
  return Math.round(value);
}

function optionalCount(value: unknown): number | null {
  if (value === undefined || value === null) return null;
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0) {
    return null;
  }
  return Math.round(value);
}

function text(value: unknown, fallback = ""): string | null {
  if (value === undefined || value === null) return fallback;
  return typeof value === "string" ? value : null;
}

/// Strict shape check; returns the first problem rather than importing a
/// half-understood rule.
export function parseTriggerFile(
  raw: unknown,
): { file: TriggerFile } | { error: string } {
  if (!isRecord(raw)) return { error: "The file is not a JSON object" };
  if (raw.format !== TRIGGER_FILE_FORMAT) {
    return { error: "This is not a roxy-can trigger file" };
  }
  if (raw.version !== TRIGGER_FILE_VERSION) {
    return {
      error: `Unsupported trigger file version ${String(raw.version)}`,
    };
  }
  if (!Array.isArray(raw.triggers) || raw.triggers.length === 0) {
    return { error: "The file contains no trigger rules" };
  }
  if (raw.triggers.length > 500) {
    return { error: "Too many rules in one file (500 max)" };
  }
  const triggers: TriggerFileRule[] = [];
  for (let index = 0; index < raw.triggers.length; index += 1) {
    const entry = raw.triggers[index];
    if (!isRecord(entry)) {
      return { error: `Rule ${index + 1} is not an object` };
    }
    const kind = text(entry.kind);
    const action = text(entry.action);
    if (!kind || !TRIGGER_KINDS.includes(kind as never)) {
      return { error: `Rule ${index + 1} has an unknown condition` };
    }
    if (!action || !TRIGGER_ACTIONS.includes(action as never)) {
      return { error: `Rule ${index + 1} has an unknown action` };
    }
    const mask = finiteNumber(entry.mask, 0xff, 0, 0xff);
    const offset = finiteNumber(entry.offset, 0, 0, 255);
    const value = finiteNumber(entry.value, 0, 0, 0xff);
    const vlan = optionalCount(entry.vlan);
    if (vlan !== null && vlan > 4095) {
      return { error: `Rule ${index + 1} has an invalid VLAN` };
    }
    const channel = finiteNumber(entry.channel, 0, 0, 255);
    const sendChannel = finiteNumber(entry.sendChannel, 0, 0, 255);
    const id = finiteNumber(entry.id, 0, 0, 0x1fffffff);
    const sendId = finiteNumber(entry.sendId, 0, 0, 0x1fffffff);
    const slot = finiteNumber(entry.slot, 0, 0, 0xffff);
    const threshold = finiteNumber(entry.threshold, 0, -1e15, 1e15);
    const cooldownMs = finiteNumber(entry.cooldownMs, 0, 0, 86_400_000);
    const maxFires = finiteNumber(entry.maxFires, 0, 0, 1_000_000);
    const busName = text(entry.busName);
    const sendBusName = text(entry.sendBusName);
    const signal = text(entry.signal);
    const key = text(entry.key);
    const derivedId = text(entry.derivedId);
    const sysVarKey = text(entry.sysVarKey);
    const scriptName = text(entry.scriptName);
    const sendFrame = text(entry.sendFrame);
    const sendData = text(entry.sendData);
    const someipMethod = finiteNumber(entry.someipMethod, 0, 0, 0xffff);
    const someipClient = finiteNumber(entry.someipClient, 0, 0, 0xffff);
    const someipSession = finiteNumber(entry.someipSession, 0, 0, 0xffff);
    const someipType = finiteNumber(entry.someipType, 0, 0, 0xff);
    const someipValues: Record<string, number> = {};
    if (isRecord(entry.someipValues)) {
      for (const [name, raw] of Object.entries(entry.someipValues)) {
        if (typeof raw !== "number" || !Number.isFinite(raw)) {
          return { error: `Rule ${index + 1} has an invalid SOME/IP value` };
        }
        someipValues[name] = raw;
      }
    }
    const condSomeipMethod =
      entry.condSomeipMethod === null || entry.condSomeipMethod === undefined
        ? null
        : finiteNumber(entry.condSomeipMethod, 0, 0, 0xffff);
    const condSomeipType =
      entry.condSomeipType === null || entry.condSomeipType === undefined
        ? null
        : finiteNumber(entry.condSomeipType, 0, 0, 0xff);
    const sysVarValue =
      entry.sysVarValue === null || entry.sysVarValue === undefined
        ? null
        : finiteNumber(entry.sysVarValue, 0, -1e15, 1e15);
    const scriptValue =
      entry.scriptValue === null || entry.scriptValue === undefined
        ? null
        : finiteNumber(entry.scriptValue, 0, -1e15, 1e15);
    const preFrames = optionalCount(entry.preFrames);
    const postFrames = optionalCount(entry.postFrames);
    if (
      mask === null ||
      offset === null ||
      value === null ||
      channel === null ||
      sendChannel === null ||
      id === null ||
      sendId === null ||
      slot === null ||
      threshold === null ||
      cooldownMs === null ||
      maxFires === null ||
      busName === null ||
      sendBusName === null ||
      signal === null ||
      key === null ||
      derivedId === null ||
      sysVarKey === null ||
      scriptName === null ||
      sendFrame === null ||
      sendData === null ||
      someipMethod === null ||
      someipClient === null ||
      someipSession === null ||
      someipType === null ||
      (entry.sysVarValue != null && sysVarValue === null) ||
      (entry.scriptValue != null && scriptValue === null)
    ) {
      return { error: `Rule ${index + 1} has an invalid field` };
    }
    triggers.push({
      kind,
      action,
      enabled: entry.enabled !== false,
      busName,
      sendBusName,
      channel,
      id,
      ext: entry.ext === true,
      signal,
      threshold,
      rising: entry.rising === true,
      slot,
      key,
      derivedId,
      sendChannel,
      sendId,
      sysVarKey,
      sysVarValue,
      scriptName,
      scriptValue,
      sendFrame,
      sendData,
      someipMethod,
      someipClient,
      someipSession,
      someipType,
      someipValues,
      condSomeipMethod,
      condSomeipType,
      offset,
      mask,
      value,
      vlan,
      cooldownMs,
      maxFires,
      preFrames,
      postFrames,
    });
  }
  return { file: { format: TRIGGER_FILE_FORMAT, version: TRIGGER_FILE_VERSION, triggers } };
}

/// The resolved file rule as the existing edit command sees it. Both the
/// CAN channel and the FR/ETH bus are set to the same resolved index; each
/// condition reads only the one it uses.
export function triggerEditOf(
  rule: TriggerFileRule,
  index: number,
): TriggerEdit {
  return {
    index,
    kind: rule.kind,
    ch: rule.channel,
    id: rule.id,
    ext: rule.ext,
    signal: rule.signal,
    threshold: rule.threshold,
    rising: rule.rising,
    bus: rule.channel,
    slot: rule.slot,
    key: rule.key,
    derivedId: rule.derivedId,
    action: rule.action,
    sendCh: rule.sendChannel,
    sendId: rule.sendId,
    cooldownMs: rule.cooldownMs,
    maxFires: rule.maxFires,
    sysVarKey: rule.sysVarKey,
    sysVarValue: rule.sysVarValue,
    scriptName: rule.scriptName,
    scriptValue: rule.scriptValue,
    sendFrame: rule.sendFrame,
    sendData: rule.sendData,
    someipMethod: rule.someipMethod,
    someipClient: rule.someipClient,
    someipSession: rule.someipSession,
    someipType: rule.someipType,
    someipValues: rule.someipValues,
    condSomeipMethod: rule.condSomeipMethod ?? undefined,
    condSomeipType: rule.condSomeipType ?? undefined,
    offset: rule.offset,
    mask: rule.mask,
    value: rule.value,
    vlan: rule.vlan,
    preFrames: rule.preFrames,
    postFrames: rule.postFrames,
  };
}

/// Maps the file's names back onto this project's channel numbers.
export function resolveTriggerFile(
  file: TriggerFile,
  buses: Bus[],
): { rules: TriggerFileRule[]; skipped: string[] } {
  const rules: TriggerFileRule[] = [];
  const skipped: string[] = [];
  const find = (name: string): number | null | "ambiguous" => {
    const matches = buses.filter((bus) => bus.name === name);
    if (matches.length === 1) return matches[0].index;
    if (matches.length > 1) return "ambiguous";
    return null;
  };
  for (const rule of file.triggers) {
    let channel = rule.channel;
    if (!GLOBAL_KINDS.has(rule.kind) && rule.busName.trim() !== "") {
      const found = find(rule.busName);
      if (found === "ambiguous") {
        skipped.push(`${rule.busName}: more than one bus has that name`);
        continue;
      }
      if (found === null) {
        skipped.push(`${rule.busName}: no such bus in this project`);
        continue;
      }
      channel = found;
    }
    let sendChannel = rule.sendChannel;
    if (
      rule.action === "send" ||
      rule.action === "send_fr" ||
      rule.action === "send_eth" ||
      rule.action === "send_someip"
    ) {
      if (rule.sendBusName.trim() !== "") {
        const found = find(rule.sendBusName);
        if (found === "ambiguous") {
          skipped.push(`${rule.sendBusName}: more than one bus has that name`);
          continue;
        }
        if (found === null) {
          skipped.push(`${rule.sendBusName}: no such send bus in this project`);
          continue;
        }
        sendChannel = found;
      }
    }
    rules.push({ ...rule, channel, sendChannel });
  }
  return { rules, skipped };
}
