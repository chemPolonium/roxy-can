import type { Command, State } from "./types";

export async function fetchState(): Promise<State> {
  const response = await fetch("/api/state", { cache: "no-store" });
  if (!response.ok) {
    throw new Error(`state: ${response.status}`);
  }
  return (await response.json()) as State;
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
