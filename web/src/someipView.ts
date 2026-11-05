import type { SomeIpEvent } from "./types";

export interface SomeIpService {
  bus: number;
  service: number;
  messages: number;
  firstTus: number;
  lastTus: number;
  /// last - first, in microseconds.
  spanUs: number;
  /// Mean gap between consecutive messages, in microseconds.
  periodUs: number;
  /// max gap - min gap, in microseconds.
  jitterUs: number;
  methods: number[];
  types: number[];
  endpoints: string[];
}

/// Groups events into service/instance conversations. SD itself is the
/// directory, not a service conversation, so its 0xFFFF pseudo-service is
/// excluded; the endpoints an SD offer names are attached to the matching
/// service when both sides were captured.
export function groupSomeIpServices(rows: SomeIpEvent[]): SomeIpService[] {
  const map = new Map<string, SomeIpService>();
  for (const row of rows) {
    if (row.service === 0xffff) continue;
    const key = `${row.bus}:${row.service}`;
    let entry = map.get(key);
    if (!entry) {
      entry = {
        bus: row.bus,
        service: row.service,
        messages: 0,
        firstTus: row.tUs,
        lastTus: row.tUs,
        spanUs: 0,
        periodUs: 0,
        jitterUs: 0,
        methods: [],
        types: [],
        endpoints: [],
      };
      map.set(key, entry);
    }
    entry.messages += 1;
    entry.firstTus = Math.min(entry.firstTus, row.tUs);
    entry.lastTus = Math.max(entry.lastTus, row.tUs);
    if (!entry.methods.includes(row.method)) entry.methods.push(row.method);
    if (!entry.types.includes(row.messageType)) entry.types.push(row.messageType);
  }
  for (const row of rows) {
    if (!row.sd) continue;
    for (const sd of row.sd) {
      for (const endpoint of sd.endpoints) {
        const target = map.get(`${row.bus}:${sd.service}`);
        if (!target) continue;
        const text = `${endpoint.address}:${endpoint.port}/${
          endpoint.protocol === 0x11 ? "UDP" : endpoint.protocol === 0x06 ? "TCP" : endpoint.protocol
        }`;
        if (!target.endpoints.includes(text)) target.endpoints.push(text);
      }
    }
  }
  for (const service of map.values()) {
    const times = rows
      .filter(
        (row) => row.bus === service.bus && row.service === service.service,
      )
      .map((row) => row.tUs)
      .sort((a, b) => a - b);
    service.spanUs = service.lastTus - service.firstTus;
    if (times.length >= 2) {
      const gaps: number[] = [];
      for (let index = 1; index < times.length; index += 1) {
        gaps.push(times[index] - times[index - 1]);
      }
      service.periodUs = Math.round(
        gaps.reduce((sum, gap) => sum + gap, 0) / gaps.length,
      );
      service.jitterUs = Math.max(...gaps) - Math.min(...gaps);
    }
  }
  return [...map.values()].sort(
    (a, b) => a.bus - b.bus || a.service - b.service,
  );
}
