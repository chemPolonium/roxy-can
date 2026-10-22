import type { Command, Snapshot } from "./types";

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
