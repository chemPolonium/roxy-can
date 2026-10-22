import type { Command, Snapshot } from "./types";

export async function fetchState(
  epoch: number | null,
  seq: number,
): Promise<Snapshot> {
  const query = epoch === null ? "" : `?epoch=${epoch}&seq=${seq}`;
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
