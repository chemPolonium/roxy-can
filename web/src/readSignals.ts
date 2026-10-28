import type {
  Catalog,
  CatalogChannel,
  CatalogMessage,
  CatalogSignal,
  Command,
} from "./types";

/// A signal kept only for cursor readout. The structured fields let a
/// project restore the read set without needing the catalog loaded first.
export interface ReadSignalSpec {
  key: string;
  kind: "can" | "fr" | "eth";
  bus: number;
  id?: number;
  extended?: boolean;
  frame?: string;
  name: string;
}

export function parseReadSignals(raw: unknown): ReadSignalSpec[] {
  if (!Array.isArray(raw)) return [];
  const out: ReadSignalSpec[] = [];
  const seen = new Set<string>();
  for (const item of raw) {
    if (!item || typeof item !== "object") continue;
    const saved = item as Partial<ReadSignalSpec>;
    const kind = saved.kind;
    if (kind !== "can" && kind !== "fr" && kind !== "eth") continue;
    const bus = saved.bus;
    if (!Number.isInteger(bus) || bus! < 0 || bus! > 255) continue;
    if (typeof saved.name !== "string" || !saved.name) continue;
    if (kind !== "eth" && (!Number.isInteger(saved.id) || saved.id! < 0)) {
      continue;
    }
    if (kind === "eth" && (typeof saved.frame !== "string" || !saved.frame)) {
      continue;
    }
    const key =
      typeof saved.key === "string" && saved.key
        ? saved.key
        : `${kind}:${bus}:${saved.id ?? saved.frame}:${saved.name}`;
    if (seen.has(key)) continue;
    seen.add(key);
    out.push({
      key,
      kind,
      bus: bus!,
      name: saved.name,
      ...(kind !== "eth"
        ? {
            id: saved.id,
            ...(kind === "can" ? { extended: !!saved.extended } : {}),
          }
        : { frame: saved.frame }),
    });
  }
  return out;
}

export function readSpecCommand(spec: ReadSignalSpec): Command {
  if (spec.kind === "fr") {
    return {
      type: "select_fr_signal",
      bus: spec.bus,
      slot: spec.id ?? 0,
      name: spec.name,
      read: true,
    };
  }
  if (spec.kind === "eth") {
    return {
      type: "select_eth_signal",
      bus: spec.bus,
      frame: spec.frame ?? spec.name,
      name: spec.name,
      read: true,
    };
  }
  return {
    type: "select_signal",
    ch: spec.bus,
    id: spec.id ?? 0,
    ext: !!spec.extended,
    name: spec.name,
    read: true,
  };
}

/// The catalog entry behind a series key, for keys produced outside the picker
/// (Panel setpoints store only the key).
export function findCatalogSignal(
  catalog: Catalog | null,
  key: string,
): { channel: CatalogChannel; message: CatalogMessage; signal: CatalogSignal } | null {
  for (const channel of catalog?.channels ?? []) {
    for (const message of channel.messages) {
      const signal = message.signals.find((entry) => entry.key === key);
      if (signal) return { channel, message, signal };
    }
  }
  return null;
}

export function catalogSignalSpec(
  channel: CatalogChannel,
  message: CatalogMessage,
  signal: CatalogSignal,
): ReadSignalSpec {
  const kind =
    channel.kind === "fr" || channel.kind === "eth" ? channel.kind : "can";
  if (kind === "eth") {
    return {
      key: signal.key,
      kind,
      bus: channel.index,
      frame: message.frame ?? message.name,
      name: signal.name,
    };
  }
  return {
    key: signal.key,
    kind,
    bus: channel.index,
    id: message.id,
    ...(kind === "can" ? { extended: message.extended } : {}),
    name: signal.name,
  };
}

/// `read = null` unselects; true/false select as read-only or plotted.
export function catalogSignalCommand(
  channel: CatalogChannel,
  message: CatalogMessage,
  signal: CatalogSignal,
  read: boolean | null,
): Command {
  const kind =
    channel.kind === "fr" || channel.kind === "eth" ? channel.kind : "can";
  if (read === null) {
    if (kind === "fr") {
      return {
        type: "unselect_fr_signal",
        bus: channel.index,
        slot: message.id,
        name: signal.name,
      };
    }
    if (kind === "eth") {
      return {
        type: "unselect_eth_signal",
        bus: channel.index,
        frame: message.frame ?? message.name,
        name: signal.name,
      };
    }
    return {
      type: "unselect_signal",
      ch: channel.index,
      id: message.id,
      ext: message.extended,
      name: signal.name,
    };
  }
  if (kind === "fr") {
    return {
      type: "select_fr_signal",
      bus: channel.index,
      slot: message.id,
      name: signal.name,
      read,
    };
  }
  if (kind === "eth") {
    return {
      type: "select_eth_signal",
      bus: channel.index,
      frame: message.frame ?? message.name,
      name: signal.name,
      read,
    };
  }
  return {
    type: "select_signal",
    ch: channel.index,
    id: message.id,
    ext: message.extended,
    name: signal.name,
    read,
  };
}
