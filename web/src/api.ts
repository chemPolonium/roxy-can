import type {
  Catalog,
  Command,
  DebugCommand,
  DebugView,
  FsList,
  Hardware,
  Script,
  ScriptFunctionHelp,
  Snapshot,
} from "./types";

/// Browser fetch failures surface as bare TypeErrors; translate the common
/// ones so the UI can offer a reason instead of a stack-trace fragment.
export function describeError(error: unknown): string {
  const text = error instanceof Error ? error.message : String(error);
  if (
    error instanceof TypeError ||
    /failed to fetch|networkerror|load failed/i.test(text)
  ) {
    return "Connection to the bus core lost — retrying…";
  }
  if (/^state: \d+$/.test(text)) {
    return "The bus core did not return a snapshot.";
  }
  if (/^command: 503$/.test(text)) {
    return "The bus core is not accepting commands.";
  }
  return text;
}

export async function fetchState(
  epoch: number | null,
  seq: number,
  signalsEpoch: number | null,
  signalsSince: number,
): Promise<Snapshot> {
  const params = new URLSearchParams();
  if (epoch !== null) {
    params.set("epoch", String(epoch));
    params.set("seq", String(seq));
  }
  if (signalsEpoch !== null) {
    params.set("signals_epoch", String(signalsEpoch));
    params.set("signals_since", String(signalsSince));
  }
  const query = params.size > 0 ? `?${params}` : "";
  const response = await fetch(`/api/state${query}`, { cache: "no-store" });
  if (!response.ok) {
    throw new Error(`state: ${response.status}`);
  }
  return (await response.json()) as Snapshot;
}

export async function fetchCatalog(): Promise<Catalog> {
  const response = await fetch("/api/catalog", { cache: "no-store" });
  if (!response.ok) {
    throw new Error(`catalog: ${response.status}`);
  }
  return (await response.json()) as Catalog;
}

export async function fetchHardware(): Promise<Hardware> {
  const response = await fetch("/api/hardware", { cache: "no-store" });
  if (!response.ok) {
    throw new Error(`hardware: ${response.status}`);
  }
  return (await response.json()) as Hardware;
}

export async function listDirectory(path?: string | null): Promise<FsList> {
  const query = path ? `?${new URLSearchParams({ path })}` : "";
  const response = await fetch(`/api/fs/list${query}`, { cache: "no-store" });
  if (!response.ok) {
    const detail = await response.text();
    throw new Error(detail || `list: ${response.status}`);
  }
  return (await response.json()) as FsList;
}

export async function fetchScripts(): Promise<Script[]> {
  const response = await fetch("/api/nodes", { cache: "no-store" });
  if (!response.ok) {
    throw new Error(`nodes: ${response.status}`);
  }
  return (await response.json()) as Script[];
}

let scriptHelp: Promise<ScriptFunctionHelp[]> | null = null;

/// The language's callable names and call shapes, served from the core so the
/// completion list cannot drift from the compiler.
export function fetchScriptHelp(): Promise<ScriptFunctionHelp[]> {
  scriptHelp ??= fetch("/api/script-help", { cache: "no-store" })
    .then(async (response) => {
      if (!response.ok) throw new Error(`script help: ${response.status}`);
      return (await response.json()) as ScriptFunctionHelp[];
    })
    .catch(() => {
      scriptHelp = null;
      return [];
    });
  return scriptHelp;
}

export async function uploadFile(file: File): Promise<string> {
  const query = new URLSearchParams({ name: file.name });
  const response = await fetch(`/api/upload?${query}`, {
    method: "POST",
    body: file,
  });
  if (!response.ok) {
    const detail = await response.text();
    throw new Error(detail || `upload: ${response.status}`);
  }
  const reply = (await response.json()) as { path: string };
  return reply.path;
}

export async function sendCommand(command: Command): Promise<void> {
  const response = await fetch("/api/command", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(command),
  });
  if (!response.ok) {
    throw new Error(`command: ${response.status}`);
  }
}

export async function fetchDebug(): Promise<DebugView> {
  const response = await fetch("/api/debug", { cache: "no-store" });
  if (!response.ok) throw new Error(`debug: ${response.status}`);
  return (await response.json()) as DebugView;
}

export async function sendDebug(command: DebugCommand): Promise<DebugView> {
  const response = await fetch("/api/debug", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(command),
  });
  if (!response.ok) throw new Error(`debug: ${response.status}`);
  return (await response.json()) as DebugView;
}
