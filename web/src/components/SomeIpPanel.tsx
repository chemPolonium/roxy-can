import { Fragment } from "preact";
import { useMemo, useState } from "preact/hooks";
import type { SomeIpEvent, SomeIpService, SomeIpSubscription } from "../types";
import { groupSomeIpServices } from "../someipView";
import { copyText } from "../clipboard";
import { sdServiceStatuses } from "../sdView";
import { UiIcon } from "./icons";

const SD_KINDS: Record<number, string> = {
  0x00: "Find",
  0x06: "Subscribe",
  0x07: "SubscribeAck",
};

function sdText(row: SomeIpEvent): { label: string; title: string } {
  if (!row.sd || row.sd.length === 0) return { label: "—", title: "" };
  const first = row.sd[0];
  const kind =
    first.kind === 0x01
      ? first.ttl > 0
        ? "Offer"
        : "StopOffer"
      : (SD_KINDS[first.kind] ?? `0x${first.kind.toString(16)}`);
  return {
    label: `${kind} 0x${first.service.toString(16).toUpperCase()}${
      row.sd.length > 1 ? ` +${row.sd.length - 1}` : ""
    }`,
    title: row.sd
      .map(
        (entry) =>
          `${
            entry.kind === 0x01
              ? entry.ttl > 0
                ? "Offer"
                : "StopOffer"
              : (SD_KINDS[entry.kind] ?? `0x${entry.kind.toString(16)}`)
          } service 0x${entry.service
            .toString(16)
            .toUpperCase()} instance 0x${entry.instance
            .toString(16)
            .toUpperCase()} v${entry.major}.${entry.minor} TTL ${entry.ttl}${
            entry.eventgroup != null
              ? ` eventgroup 0x${entry.eventgroup.toString(16).toUpperCase()}`
              : ""
          }${
            entry.endpoints.length > 0
              ? ` @ ${entry.endpoints
                  .map(
                    (endpoint) =>
                      `${endpoint.address}:${endpoint.port}/${endpoint.protocol === 0x11 ? "UDP" : endpoint.protocol === 0x06 ? "TCP" : endpoint.protocol}`,
                  )
                  .join(", ")}`
              : ""
          }${
            entry.configuration.length > 0
              ? ` config ${entry.configuration.join(", ")}`
              : ""
          }`,
      )
      .join("\n"),
  };
}

const MESSAGE_TYPES: Record<number, string> = {
  0x00: "REQUEST",
  0x01: "REQUEST_NR",
  0x02: "NOTIFICATION",
  0x20: "TP_REQUEST",
  0x21: "TP_REQUEST_NR",
  0x22: "TP_NOTIFICATION",
  0x80: "RESPONSE",
  0x81: "ERROR",
  0xa0: "TP_RESPONSE",
  0xa1: "TP_ERROR",
};

function hex(value: number, width = 4): string {
  return `0x${value.toString(16).toUpperCase().padStart(width, "0")}`;
}

function timeText(us: number): string {
  if (us <= 0) return "—";
  return us >= 1_000_000
    ? `${(us / 1e6).toFixed(3)} s`
    : `${(us / 1_000).toFixed(2)} ms`;
}

export function SomeIpPanel({
  rows,
  services = [],
  subscriptions = [],
  issues = [],
}: {
  rows: SomeIpEvent[];
  services?: SomeIpService[];
  subscriptions?: SomeIpSubscription[];
  issues?: string[];
}) {
  const [query, setQuery] = useState("");
  const [view, setView] = useState<
    "events" | "services" | "availability" | "database"
  >("events");
  const [selected, setSelected] = useState<string | null>(null);
  const trafficServices = useMemo(() => groupSomeIpServices(rows), [rows]);
  const [dbQuery, setDbQuery] = useState("");
  const dbServices = useMemo(() => {
    const needle = dbQuery.trim().toLowerCase();
    if (!needle) return services;
    return services.filter(
      (service) =>
        service.name.toLowerCase().includes(needle) ||
        service.busName.toLowerCase().includes(needle) ||
        hex(service.id).toLowerCase().includes(needle) ||
        service.instances.some(
          (instance) =>
            String(instance.id).includes(needle) ||
            instance.endpoint.toLowerCase().includes(needle) ||
            instance.eventgroups.some((group) => String(group).includes(needle)),
        ),
    );
  }, [dbQuery, services]);
  const availability = useMemo(() => sdServiceStatuses(rows), [rows]);
  const visible = useMemo(() => {
    const needle = query.trim().toLowerCase();
    const filtered = needle
      ? rows.filter((row) =>
          [
            row.src,
            row.dst,
            hex(row.service),
            hex(row.method),
            MESSAGE_TYPES[row.messageType] ?? "",
            ...(row.sd ?? []).flatMap((entry) => [
              String(entry.instance),
              entry.eventgroup == null ? "" : String(entry.eventgroup),
            ]),
          ]
            .join(" ")
            .toLowerCase()
            .includes(needle),
        )
      : rows;
    return [...filtered].reverse();
  }, [rows, query]);

  return (
    <div class="panel-list">
      <div class="panel-filter-row">
        <button
          type="button"
          class={view === "events" ? "mini on" : "mini"}
          onClick={() => setView("events")}
        >
          Events
        </button>
        <button
          type="button"
          class={view === "services" ? "mini on" : "mini"}
          onClick={() => setView("services")}
        >
          Services
        </button>
        <button
          type="button"
          class={view === "availability" ? "mini on" : "mini"}
          onClick={() => setView("availability")}
        >
          Availability
        </button>
        <button
          type="button"
          class={view === "database" ? "mini on" : "mini"}
          onClick={() => setView("database")}
        >
          Database
        </button>
        <input
          class="panel-filter"
          type="search"
          value={query}
          placeholder="Filter service, method, instance, eventgroup…"
          onInput={(event) =>
            setQuery((event.target as HTMLInputElement).value)
          }
        />
        {query && (
          <button
            type="button"
            class="mini icon-btn"
            title="Clear the filter"
            aria-label="Clear filter"
            onClick={() => setQuery("")}
          >
            <UiIcon name="close" />
          </button>
        )}
      </div>
      {issues.length > 0 && (
        <div class="someip-issues" title={issues.join("\n")}>
          {issues.length} ARXML type issue{issues.length === 1 ? "" : "s"}:{" "}
          {issues[0]}
          {issues.length > 1 ? ` +${issues.length - 1}` : ""}
        </div>
      )}
      {view === "availability" ? (
        availability.length === 0 ? (
          <div class="empty">
            <strong>No service offers</strong>
            <span>SD offer/stop-offer entries define availability.</span>
          </div>
        ) : (
          <div class="table-scroll">
            <table class="data-table someip-table">
              <thead>
                <tr>
                  <th>Bus</th>
                  <th>Service</th>
                  <th>Instance</th>
                  <th>State</th>
                  <th>Version</th>
                  <th>TTL</th>
                  <th>Endpoints</th>
                  <th>Subscribers</th>
                  <th>Last seen</th>
                </tr>
              </thead>
              <tbody>
                {availability.map((entry) => (
                  <tr key={`${entry.bus}:${entry.service}:${entry.instance}`}>
                    <td>ETH{entry.bus}</td>
                    <td class="mono">{hex(entry.service)}</td>
                    <td class="mono">{hex(entry.instance)}</td>
                    <td>
                      <span class={`sd-state ${entry.state}`}>
                        {entry.state === "offered"
                          ? "Offered"
                          : entry.state === "stopped"
                            ? "Stopped"
                            : "Expired"}
                      </span>
                    </td>
                    <td class="mono">
                      {entry.major}.{entry.minor}
                    </td>
                    <td class="mono">
                      {entry.ttl === 0xffffff ? "∞" : `${entry.ttl} s`}
                    </td>
                    <td class="mono" title={entry.endpoints.join("\n")}>
                      {entry.endpoints[0] ?? "—"}
                      {entry.endpoints.length > 1
                        ? ` +${entry.endpoints.length - 1}`
                        : ""}
                    </td>
                    <td
                      class="mono"
                      title={entry.eventgroups
                        .map((group) => `eventgroup 0x${group.toString(16)}`)
                        .join("\n")}
                    >
                      {entry.subscribers}
                    </td>
                    <td class="mono">
                      {(entry.lastSeenTus / 1e6).toFixed(3)} s
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )
      ) : view === "database" ? (
        services.length === 0 ? (
          <div class="empty">
            <strong>No ARXML service objects</strong>
            <span>Import an Ethernet ARXML with SERVICE-INTERFACE deployments.</span>
          </div>
        ) : (
          <div class="table-scroll">
            <div class="panel-filter-row">
              <input
                class="panel-filter"
                type="search"
                value={dbQuery}
                placeholder="Filter service, instance, eventgroup, endpoint…"
                onInput={(event) =>
                  setDbQuery((event.target as HTMLInputElement).value)
                }
              />
              <span class="muted">
                {dbServices.length} / {services.length}
              </span>
            </div>
            <table class="data-table someip-table">
              <thead>
                <tr>
                  <th>Bus</th>
                  <th>Service</th>
                  <th>Name</th>
                  <th>Instances</th>
                  <th>Methods / Events</th>
                  <th>Source</th>
                </tr>
              </thead>
              <tbody>
                {dbServices.map((service) => (
                  <tr key={`${service.bus}:${service.id}`}>
                    <td>ETH{service.bus}</td>
                    <td class="mono">{hex(service.id)}</td>
                    <td title={service.busName}>{service.name}</td>
                    <td class="mono">
                      {service.instances.length === 0
                        ? "—"
                        : service.instances
                            .map(
                              (instance) =>
                                `${instance.id}${instance.eventgroups.length ? ` [${instance.eventgroups.join(",")}]` : ""}${instance.endpoint ? ` @${instance.endpoint}` : ""}`,
                            )
                            .join(" ")}
                    </td>
                    <td class="mono" title={[...service.methods, ...service.events].flatMap((m) => m.fields).join("\n")}>
                      {service.methods.length} / {service.events.length}
                    </td>
                    <td title={service.source}>{service.source ? "ARXML" : "—"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            {subscriptions.length > 0 && (
              <table class="data-table someip-table">
                <thead>
                  <tr>
                    <th>Bus</th>
                    <th>Service</th>
                    <th>Group</th>
                    <th>Subscriber</th>
                    <th>TTL</th>
                  </tr>
                </thead>
                <tbody>
                  {subscriptions.map((subscription) => (
                    <tr
                      key={`${subscription.bus}:${subscription.service}:${subscription.instance}:${subscription.eventgroup}:${subscription.ip}:${subscription.port}`}
                    >
                      <td>ETH{subscription.bus}</td>
                      <td
                        class="mono"
                        title={`instance ${subscription.instance}`}
                      >
                        {hex(subscription.service)}
                      </td>
                      <td class="mono">{subscription.eventgroup}</td>
                      <td class="mono">
                        {subscription.ip}:{subscription.port}
                      </td>
                      <td class="mono">{subscription.ttl}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </div>
        )
      ) : view === "services" ? (
        trafficServices.length === 0 ? (
          <div class="empty">
            <strong>No SOME/IP services</strong>
            <span>Only method/event traffic is counted; SD is the directory.</span>
          </div>
        ) : (
          <div class="table-scroll">
            <table class="data-table someip-table">
              <thead>
                <tr>
                  <th>Bus</th>
                  <th>Service</th>
                  <th>Messages</th>
                  <th>Methods</th>
                  <th>Types</th>
                  <th>Endpoints</th>
                  <th>Period</th>
                  <th>Jitter</th>
                  <th>Span</th>
                </tr>
              </thead>
              <tbody>
                {trafficServices.map((service) => (
                  <tr
                    key={`${service.bus}:${service.service}`}
                    class="someip-service-row"
                    onClick={() => {
                      setQuery(hex(service.service));
                      setView("events");
                    }}
                  >
                    <td>ETH{service.bus}</td>
                    <td class="mono">{hex(service.service)}</td>
                    <td class="mono">{service.messages}</td>
                    <td
                      class="mono"
                      title={service.methods.map((method) => hex(method)).join(", ")}
                    >
                      {service.methods.slice(0, 3).map((method) => hex(method)).join(" ")}
                      {service.methods.length > 3
                        ? ` +${service.methods.length - 3}`
                        : ""}
                    </td>
                    <td>
                      {service.types
                        .map((type) => MESSAGE_TYPES[type] ?? hex(type, 2))
                        .join(" ")}
                    </td>
                    <td class="mono" title={service.endpoints.join("\n")}>
                      {service.endpoints[0] ?? "—"}
                      {service.endpoints.length > 1
                        ? ` +${service.endpoints.length - 1}`
                        : ""}
                    </td>
                    <td class="mono">{timeText(service.periodUs)}</td>
                    <td class="mono">{timeText(service.jitterUs)}</td>
                    <td
                      class="mono"
                      title={`${timeText(service.firstTus)} → ${timeText(service.lastTus)}`}
                    >
                      {timeText(service.spanUs)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )
      ) : visible.length === 0 ? (
        <div class="empty">
          <strong>No SOME/IP messages</strong>
          <span>
            Load a PCAP/PCAPNG with SOME/IP over UDP (SD included) and replay
            it.
          </span>
        </div>
      ) : (
        <div class="table-scroll">
          <table class="data-table someip-table someip-events-table">
            <thead>
              <tr>
                <th>Time</th>
                <th>Service</th>
                <th>Method</th>
                <th>Type</th>
                <th>Fields</th>
              </tr>
            </thead>
            <tbody>
              {visible.map((row, index) => {
                const key = `${row.tUs}-${row.bus}-${index}`;
                const sd = sdText(row);
                const open = selected === key;
                return (
                  <Fragment key={key}>
                    <tr
                      class={`someip-event-row ${open ? "open" : ""}`}
                      onClick={() => setSelected(open ? null : key)}
                    >
                      <td class="mono">{(row.tUs / 1e6).toFixed(3)} s</td>
                      <td class="mono">{hex(row.service)}</td>
                      <td class="mono">{hex(row.method)}</td>
                      <td title={`Message type ${hex(row.messageType, 2)}`}>
                        {MESSAGE_TYPES[row.messageType] ?? hex(row.messageType, 2)}
                      </td>
                      <td
                        class="mono"
                        title={row.decoded
                          .map((field) => `${field.name} = ${field.text}`)
                          .join("\n")}
                      >
                        {row.decoded.length === 0
                          ? "—"
                          : row.decoded
                              .slice(0, 2)
                              .map((field) => `${field.name}=${field.text}`)
                              .join(" ")}
                        {row.decoded.length > 2
                          ? ` +${row.decoded.length - 2}`
                          : ""}
                      </td>
                    </tr>
                    {open && (
                      <tr class="someip-detail-row">
                        <td colSpan={5}>
                          <div class="someip-detail-grid">
                            <span>
                              <b>Bus</b> ETH{row.bus} · {row.transport} ·{" "}
                              {row.tx ? "Tx" : "Rx"}
                            </span>
                            <span>
                              <b>Client / session</b> {row.client} / {row.session}
                            </span>
                            <span>
                              <b>Return</b> {hex(row.returnCode, 2)}
                            </span>
                            <span>
                              <b>Endpoint</b> {row.src}:{row.srcPort} → {row.dst}:
                              {row.dstPort}
                            </span>
                            <span>
                              <b>SD</b> {sd.label}
                            </span>
                            <span>
                              <b>Payload</b>{" "}
                              {row.payload.length === 0
                                ? "—"
                                : row.payload
                                    .slice(0, 32)
                                    .map((byte) =>
                                      byte.toString(16).padStart(2, "0").toUpperCase(),
                                    )
                                    .join(" ")}
                              {row.payload.length > 32
                                ? ` … +${row.payload.length - 32} B`
                                : ""}
                              {row.payload.length > 0 && (
                                <button
                                  type="button"
                                  class="mini"
                                  onClick={(event) => {
                                    event.stopPropagation();
                                    void copyText(
                                      row.payload
                                        .map((byte) =>
                                          byte
                                            .toString(16)
                                            .padStart(2, "0")
                                            .toUpperCase(),
                                        )
                                        .join(" "),
                                    );
                                  }}
                                >
                                  Copy
                                </button>
                              )}
                            </span>
                          </div>
                          {row.decodeError && (
                            <div class="someip-detail-error">
                              {row.decodeError}
                            </div>
                          )}
                          {row.decoded.length > 0 && (
                            <div class="someip-detail-fields">
                              {row.decoded.map((field) => (
                                <span key={field.name}>
                                  {field.name} = {field.text}
                                </span>
                              ))}
                            </div>
                          )}
                        </td>
                      </tr>
                    )}
                  </Fragment>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
