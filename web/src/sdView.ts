import type { SomeIpEvent } from "./types";

export type SdServiceState = "offered" | "stopped" | "expired";

export interface SdServiceStatus {
  bus: number;
  service: number;
  instance: number;
  state: SdServiceState;
  major: number;
  minor: number;
  ttl: number;
  endpoints: string[];
  lastOfferTus: number;
  lastSeenTus: number;
  offers: number;
  stops: number;
  subscribers: number;
  eventgroups: number[];
}

/// Availability is evaluated against the newest message in the capture (or
/// an explicit clock): this is an offline trace, not a live TTL timer. An
/// offer with TTL 0xFFFFFF never expires; a later stop-offer wins over its
/// own offer, while a newer offer re-opens the service.
export function sdServiceStatuses(
  rows: SomeIpEvent[],
  nowTus?: number,
): SdServiceStatus[] {
  const ordered = [...rows].sort((a, b) => a.tUs - b.tUs);
  const map = new Map<string, SdServiceStatus>();
  for (const row of ordered) {
    if (!row.sd) continue;
    for (const entry of row.sd) {
      if (entry.kind !== 0x01 && entry.kind !== 0x06) {
        continue;
      }
      const key = `${row.bus}:${entry.service}:${entry.instance}`;
      let status = map.get(key);
      if (!status) {
        status = {
          bus: row.bus,
          service: entry.service,
          instance: entry.instance,
          state: "stopped",
          major: entry.major,
          minor: entry.minor,
          ttl: entry.ttl,
          endpoints: [],
          lastOfferTus: 0,
          lastSeenTus: row.tUs,
          offers: 0,
          stops: 0,
          subscribers: 0,
          eventgroups: [],
        };
        map.set(key, status);
      }
      status.lastSeenTus = row.tUs;
      if (entry.kind === 0x01 && entry.ttl > 0) {
        status.state = "offered";
        status.major = entry.major;
        status.minor = entry.minor;
        status.ttl = entry.ttl;
        status.lastOfferTus = row.tUs;
        status.offers += 1;
        for (const endpoint of entry.endpoints) {
          const text = `${endpoint.address}:${endpoint.port}/${
            endpoint.protocol === 0x11
              ? "UDP"
              : endpoint.protocol === 0x06
                ? "TCP"
                : endpoint.protocol
          }`;
          if (!status.endpoints.includes(text)) status.endpoints.push(text);
        }
      } else if (entry.kind === 0x01) {
        status.state = "stopped";
        status.stops += 1;
      } else if (entry.ttl > 0) {
        status.subscribers += 1;
        if (
          entry.eventgroup != null &&
          !status.eventgroups.includes(entry.eventgroup)
        ) {
          status.eventgroups.push(entry.eventgroup);
        }
      }
    }
  }
  const now =
    nowTus ?? ordered.reduce((latest, row) => Math.max(latest, row.tUs), 0);
  for (const status of map.values()) {
    if (
      status.state === "offered" &&
      status.ttl !== 0xffffff &&
      now > status.lastOfferTus + status.ttl * 1_000_000
    ) {
      status.state = "expired";
    }
  }
  return [...map.values()].sort(
    (a, b) =>
      a.bus - b.bus || a.service - b.service || a.instance - b.instance,
  );
}
