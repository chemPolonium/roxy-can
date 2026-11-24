import { useEffect, useState } from "preact/hooks";
import {
  formatPanelHex,
  parsePanelHex,
  panelFrameProblem,
  type PanelFrameSend,
} from "../panelPage";
import { someIpMemberFor, type PanelFrameDef } from "../panelBinding";
import type { SomeIpFieldDef, SomeIpService } from "../types";
function frameIdText(frame: PanelFrameSend | undefined): string {
  return frame?.protocol === "can"
    ? `0x${frame.id.toString(16).toUpperCase()}`
    : "";
}

function zeroHex(length: number | null): string {
  return length && length > 0
    ? formatPanelHex(new Array(length).fill(0))
    : "";
}

/// Which field the frame-problem text belongs to, so the editor can keep the
/// highlight next to the control instead of only showing a toast.
function panelProblemField(
  problem: string,
): "id" | "slot" | "data" | "someip" | null {
  if (problem.startsWith("ID") || problem.includes("IDs stop")) return "id";
  if (problem.startsWith("Slot")) return "slot";
  if (
    problem.startsWith("Data") ||
    problem.includes("data must") ||
    problem.includes("bytes")
  ) {
    return "data";
  }
  if (
    problem.startsWith("Service") ||
    problem.startsWith("Method") ||
    problem.startsWith("Client") ||
    problem.startsWith("Session") ||
    problem.startsWith("Message type") ||
    problem.startsWith("TCP")
  ) {
    return "someip";
  }
  return null;
}

function SomeIpFieldEditor({
  frame,
  defs,
  invalid,
  onFocus,
  onCommit,
  onProblem,
}: {
  frame: PanelFrameSend;
  defs: SomeIpFieldDef[];
  invalid: boolean;
  onFocus: () => void;
  onCommit: (next: PanelFrameSend) => boolean;
  onProblem: (message: string) => void;
}) {
  const entryOf = (name: string) =>
    (frame.values ?? []).find((entry) => entry.name === name);
  const bus = frame.protocol === "someip" ? frame.bus : 0;
  const service = frame.protocol === "someip" ? frame.service : 0;
  const method = frame.protocol === "someip" ? frame.method : 0;
  const messageType = frame.protocol === "someip" ? frame.messageType : 0;
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  useEffect(() => {
    const next: Record<string, string> = {};
    for (const def of defs) {
      const entry = entryOf(def.name);
      next[def.name] =
        entry?.text !== undefined
          ? entry.text
          : entry
            ? String(entry.value)
            : "";
    }
    setDrafts(next);
    // The frame identity is enough: the picker remounts when the service or
    // method changes, and a successful commit replaces the stored values.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [bus, service, method, messageType, frame.values]);

  const setValue = (def: SomeIpFieldDef, raw: string) => {
    const next = (frame.values ?? []).filter(
      (entry) => entry.name !== def.name,
    );
    if (raw.trim() !== "") {
      if (def.kind === "text") {
        next.push({ name: def.name, value: 0, text: raw });
      } else {
        const value = Number(raw);
        if (!Number.isFinite(value)) {
          onProblem(`${def.name} must be a number`);
          return;
        }
        next.push({ name: def.name, value });
      }
    }
    if (onCommit({ ...frame, mode: next.length ? "physical" : "raw", values: next })) {
      setDrafts((current) => ({ ...current, [def.name]: raw }));
    }
  };

  return (
    <div class={`page-someip-fields ${invalid ? "invalid" : ""}`}>
      <span class="page-frame-hint">
        Fields are listed from the loaded ARXML service description.
      </span>
      {defs.map((def) => {
        const value = drafts[def.name] ?? "";
        const label = `${def.name}${def.unit ? ` (${def.unit})` : ""}`;
        const inputId = `someip-${def.name.replace(/[^a-zA-Z0-9_-]/g, "-")}`;
        return (
          <label key={def.name} title={def.name}>
            {label}
            {def.kind === "text" ? (
              <input
                id={inputId}
                value={value}
                onFocus={onFocus}
                placeholder="text"
                spellcheck={false}
                onInput={(event) =>
                  setDrafts((current) => ({
                    ...current,
                    [def.name]: (event.target as HTMLInputElement).value,
                  }))
                }
                onBlur={(event) => setValue(def, event.currentTarget.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    event.preventDefault();
                    (event.target as HTMLInputElement).blur();
                  }
                }}
              />
            ) : def.labels && def.labels.length > 0 ? (
              <select
                id={inputId}
                value={value}
                onFocus={onFocus}
                onChange={(event) => {
                  setValue(def, event.currentTarget.value);
                }}
              >
                <option value="">—</option>
                {def.labels.map(([code, text]) => (
                  <option value={String(code)} key={code}>
                    {`${code} · ${text}`}
                  </option>
                ))}
              </select>
            ) : (
              <input
                id={inputId}
                type="number"
                value={value}
                onFocus={onFocus}
                placeholder={def.min != null ? String(def.min) : "0"}
                min={def.min ?? undefined}
                max={def.max ?? undefined}
                onInput={(event) =>
                  setDrafts((current) => ({
                    ...current,
                    [def.name]: (event.target as HTMLInputElement).value,
                  }))
                }
                onBlur={(event) => setValue(def, event.currentTarget.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    event.preventDefault();
                    (event.target as HTMLInputElement).blur();
                  }
                }}
              />
            )}
          </label>
        );
      })}
    </div>
  );
}

export function FrameField({
  frame,
  canBuses,
  frBuses,
  ethBuses,
  ethFrames,
  frameDefs,
  someipServices,
  onFocus,
  onChange,
}: {
  frame: PanelFrameSend | undefined;
  canBuses: { index: number; name: string }[];
  frBuses: { index: number; name: string }[];
  ethBuses: { index: number; name: string }[];
  ethFrames: { bus: number; name: string; length: number | null }[];
  frameDefs: PanelFrameDef[];
  someipServices: SomeIpService[];
  onFocus: () => void;
  onChange: (frame: PanelFrameSend) => void;
}) {
  const [idDraft, setIdDraft] = useState(() => frameIdText(frame));
  const [slotDraft, setSlotDraft] = useState(() =>
    frame?.protocol === "fr" ? String(frame.slot) : "",
  );
  const [dataDraft, setDataDraft] = useState(() => frame?.data ?? "");
  const [valueDrafts, setValueDrafts] = useState<Record<string, string>>({});
  const [editing, setEditing] = useState<"id" | "slot" | "data" | null>(null);
  const [error, setError] = useState("");
  const [errorField, setErrorField] = useState<
    "id" | "slot" | "data" | "someip" | null
  >(null);
  const protocol = frame?.protocol ?? "can";
  const someipMember =
    frame?.protocol === "someip"
      ? someIpMemberFor(
          someipServices,
          frame.bus,
          frame.service,
          frame.method,
          frame.messageType,
        )
      : undefined;
  const someipFields =
    (frame?.protocol === "someip" && frame.messageType === 0x80
      ? someipMember?.outputFieldDefs
      : someipMember?.inputFieldDefs) ??
    someipMember?.fieldDefs ??
    [];

  useEffect(() => {
    if (editing !== "id") setIdDraft(frameIdText(frame));
  }, [frame, editing]);
  useEffect(() => {
    if (editing !== "slot") {
      setSlotDraft(frame?.protocol === "fr" ? String(frame.slot) : "");
    }
  }, [frame, editing]);
  useEffect(() => {
    if (editing !== "data") setDataDraft(frame?.data ?? "");
  }, [frame, editing]);
  useEffect(() => {
    const next: Record<string, string> = {};
    for (const entry of frame?.values ?? []) {
      next[entry.name] = String(entry.value);
    }
    setValueDrafts(next);
  }, [frame]);

  const commit = (next: PanelFrameSend): boolean => {
    const problem = panelFrameProblem(next);
    if (problem) {
      setError(problem);
      setErrorField(panelProblemField(problem));
      return false;
    }
    setError("");
    setErrorField(null);
    onChange(next);
    return true;
  };

  /// Changing the addressed frame drops physical values: they belong to the
  /// old signal set. A manual address is no longer a catalog selection.
  const readdress = (next: PanelFrameSend): boolean =>
    commit({
      ...next,
      mode: "raw",
      values: [],
      ...(next.protocol === "can" ? { source: "raw" as const } : {}),
    });

  const defaultFor = (kind: "can" | "fr" | "eth" | "someip"): PanelFrameSend => {
    if (kind === "can") {
      return {
        protocol: "can",
        channel: canBuses[0]?.index ?? 0,
        id: 0x7df,
        extended: false,
        data: "",
      };
    }
    if (kind === "fr") {
      return {
        protocol: "fr",
        bus: frBuses[0]?.index ?? 0,
        slot: 1,
        data: "",
      };
    }
    if (kind === "someip") {
      return {
        protocol: "someip",
        bus: ethBuses[0]?.index ?? 0,
        service: 0x1234,
        method: 0x0001,
        client: 1,
        session: 0,
        messageType: 0x02,
        data: "",
        transport: "udp",
        host: "127.0.0.1",
        port: 30490,
      };
    }
    const bus = ethBuses[0]?.index ?? 0;
    const def = ethFrames.find((entry) => entry.bus === bus);
    return {
      protocol: "eth",
      bus,
      frame: def?.name ?? "",
      data: zeroHex(def?.length ?? null),
    };
  };

  const base = (): PanelFrameSend => frame ?? defaultFor(protocol);
  const ethDef =
    frame?.protocol === "eth"
      ? ethFrames.find(
          (entry) => entry.bus === frame.bus && entry.name === frame.frame,
        )
      : undefined;
  const frameDef = frame
    ? frameDefs.find((entry) => {
        if (entry.protocol !== frame.protocol) return false;
        if (frame.protocol === "can") {
          return (
            entry.bus === frame.channel &&
            entry.id === frame.id &&
            Boolean(entry.extended) === frame.extended
          );
        }
        if (frame.protocol === "fr") {
          return entry.bus === frame.bus && entry.slot === frame.slot;
        }
        if (frame.protocol !== "eth") return false;
        return entry.bus === frame.bus && entry.frame === frame.frame;
      })
    : undefined;
  const source =
    protocol !== "can" ? "catalog" : frameDef ? (frame?.source ?? "catalog") : "raw";
  const catalogOptions = frameDefs.filter(
    (entry): entry is PanelFrameDef & { id: number } =>
      entry.protocol === "can" &&
      entry.id !== undefined &&
      entry.bus === (frame?.protocol === "can" ? frame.channel : 0),
  );
  const catalogValue =
    source === "catalog" && frame?.protocol === "can" && frameDef
      ? `${frame.id}:${frame.extended ? 1 : 0}`
      : "raw";
  const modeEnabled =
    protocol === "can" ? source === "catalog" && frameDef !== undefined : frameDef !== undefined;
  const physical = modeEnabled && frame?.mode === "physical";

  const commitId = () => {
    setEditing(null);
    const raw = idDraft.trim().replace(/^0x/i, "");
    const id = /^[0-9a-fA-F]+$/.test(raw) ? Number.parseInt(raw, 16) : Number.NaN;
    const current = base();
    if (!Number.isInteger(id) || current.protocol !== "can") {
      setError("ID must be hex");
      setErrorField("id");
      setIdDraft(frameIdText(frame));
      return;
    }
    if (!readdress({ ...current, id })) setIdDraft(frameIdText(frame));
  };

  const commitSlot = () => {
    setEditing(null);
    const slot = Number.parseInt(slotDraft.trim(), 10);
    const current = base();
    if (!Number.isInteger(slot) || current.protocol !== "fr") {
      setError("Slot must be a number");
      setErrorField("slot");
      setSlotDraft(frame?.protocol === "fr" ? String(frame.slot) : "");
      return;
    }
    if (!readdress({ ...current, slot })) {
      setSlotDraft(frame?.protocol === "fr" ? String(frame.slot) : "");
    }
  };

  const commitData = () => {
    setEditing(null);
    const bytes = parsePanelHex(dataDraft);
    if (bytes === null) {
      setError("Data must be hex bytes");
      setErrorField("data");
      setDataDraft(frame?.data ?? "");
      return;
    }
    const current = base();
    if (current.protocol === "eth") {
      const def = ethFrames.find(
        (entry) => entry.bus === current.bus && entry.name === current.frame,
      );
      if (def?.length != null && bytes.length !== def.length) {
        setError(`Frame "${current.frame}" carries ${def.length} payload bytes`);
        setErrorField("data");
        setDataDraft(frame?.data ?? "");
        return;
      }
    }
    if (!commit({ ...current, data: formatPanelHex(bytes) })) {
      setDataDraft(frame?.data ?? "");
    }
  };

  const commitValue = (name: string, raw: string) => {
    const current = base();
    const values = (current.values ?? []).filter((entry) => entry.name !== name);
    const trimmed = raw.trim();
    if (trimmed !== "") {
      if (
        trimmed.length >= 2 &&
        ((trimmed.startsWith('"') && trimmed.endsWith('"')) ||
          (trimmed.startsWith("'") && trimmed.endsWith("'")))
      ) {
        values.push({ name, value: 0, text: trimmed.slice(1, -1) });
      } else {
        const value = Number(trimmed);
        if (!Number.isFinite(value)) {
          setError(`Value for ${name} must be a number or "text"`);
          return;
        }
        values.push({ name, value });
      }
    }
    commit({ ...current, mode: "physical", values });
  };

  const dataHint =
    protocol === "can"
      ? source === "catalog"
        ? "Sends once when clicked."
        : "Raw frame. Pick a defined message to edit physical values."
      : protocol === "fr"
        ? "Sends on the FR port when one is attached; otherwise session only."
        : protocol === "someip"
          ? "SOME/IP over UDP, session only — no Ethernet transmit port."
          : `Session only — no Ethernet transmit port.${
              ethDef?.length != null ? ` ${ethDef.length} payload bytes.` : ""
            }`;

  return (
    <div class="page-frame">
      <label>
        Protocol
        <select
          value={protocol}
          onFocus={onFocus}
          onChange={(event) => {
            const next = (event.target as HTMLSelectElement).value;
            if (next !== "can" && next !== "fr" && next !== "eth" && next !== "someip") return;
            setError("");
            onChange(defaultFor(next));
          }}
        >
          <option value="can">CAN</option>
          <option value="fr">FlexRay</option>
          <option value="eth">Ethernet</option>
          <option value="someip">SOME/IP</option>
        </select>
      </label>
      {protocol === "can" ? (
        <>
          <label>
            Bus
            <select
              value={String(
                frame?.protocol === "can" &&
                  canBuses.some((bus) => bus.index === frame.channel)
                  ? frame.channel
                  : (canBuses[0]?.index ?? 0),
              )}
              onFocus={onFocus}
              onChange={(event) => {
                const value = (event.target as HTMLSelectElement).value;
                const current = base();
                if (value === "" || current.protocol !== "can") return;
                readdress({ ...current, channel: Number(value) });
              }}
            >
              {canBuses.map((bus) => (
                <option value={String(bus.index)} key={bus.index}>
                  {bus.name}
                </option>
              ))}
            </select>
          </label>
          <label>
            Message
            <select
              value={catalogValue}
              onFocus={onFocus}
              onChange={(event) => {
                const value = (event.target as HTMLSelectElement).value;
                const current = base();
                if (current.protocol !== "can") return;
                if (value === "raw") {
                  commit({
                    ...current,
                    source: "raw",
                    mode: "raw",
                    values: [],
                  });
                  return;
                }
                const [idText, extendedText] = value.split(":");
                const id = Number(idText);
                const def = catalogOptions.find(
                  (entry) =>
                    entry.id === id &&
                    Boolean(entry.extended) === (extendedText === "1"),
                );
                if (!def) return;
                commit({
                  ...current,
                  source: "catalog",
                  id,
                  extended: Boolean(def.extended),
                  mode: "raw",
                  values: [],
                });
              }}
            >
              <option value="raw">Raw frame (manual ID)</option>
              {catalogOptions.map((entry) => {
                const key = `${entry.id}:${entry.extended ? 1 : 0}`;
                const id = `0x${entry.id.toString(16).toUpperCase()}`;
                return (
                  <option value={key} key={key}>
                    {entry.name ? `${id} · ${entry.name}` : id}
                  </option>
                );
              })}
            </select>
          </label>
          <div class="page-frame-row">
            <label>
              ID (hex)
              <input
                value={idDraft}
                class={errorField === "id" ? "invalid" : undefined}
                aria-invalid={errorField === "id"}
                placeholder="0x7DF"
                spellcheck={false}
                disabled={source === "catalog"}
                title={
                  source === "catalog"
                    ? "Filled by the selected message"
                    : undefined
                }
                onFocus={() => {
                  onFocus();
                  setEditing("id");
                }}
                onInput={(event) =>
                  setIdDraft((event.target as HTMLInputElement).value)
                }
                onBlur={commitId}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    event.preventDefault();
                    (event.target as HTMLInputElement).blur();
                  } else if (event.key === "Escape") {
                    event.preventDefault();
                    setEditing(null);
                    setIdDraft(frameIdText(frame));
                    (event.target as HTMLInputElement).blur();
                  }
                }}
              />
            </label>
            <label class="page-frame-ext">
              <input
                type="checkbox"
                checked={frame?.protocol === "can" && frame.extended}
                disabled={source === "catalog"}
                title={
                  source === "catalog"
                    ? "Filled by the selected message"
                    : undefined
                }
                onFocus={onFocus}
                onChange={(event) => {
                  const current = base();
                  if (current.protocol !== "can") return;
                  readdress({
                    ...current,
                    extended: (event.target as HTMLInputElement).checked,
                  });
                }}
              />
              Extended
            </label>
          </div>
        </>
      ) : protocol === "fr" ? (
        <>
          <label>
            Bus
            <select
              value={String(
                frame?.protocol === "fr" &&
                  frBuses.some((bus) => bus.index === frame.bus)
                  ? frame.bus
                  : (frBuses[0]?.index ?? 0),
              )}
              onFocus={onFocus}
              onChange={(event) => {
                const value = (event.target as HTMLSelectElement).value;
                const current = base();
                if (value === "" || current.protocol !== "fr") return;
                readdress({ ...current, bus: Number(value) });
              }}
            >
              {frBuses.map((bus) => (
                <option value={String(bus.index)} key={bus.index}>
                  {bus.name}
                </option>
              ))}
            </select>
          </label>
          <label>
            Slot
            <input
              type="number"
              min={1}
              max={2047}
              value={slotDraft}
              class={errorField === "slot" ? "invalid" : undefined}
              aria-invalid={errorField === "slot"}
              placeholder="1"
              onFocus={() => {
                onFocus();
                setEditing("slot");
              }}
              onInput={(event) =>
                setSlotDraft((event.target as HTMLInputElement).value)
              }
              onBlur={commitSlot}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  (event.target as HTMLInputElement).blur();
                }
              }}
            />
          </label>
        </>
      ) : protocol === "someip" && frame?.protocol === "someip" ? (
        <>
          <label>
            Bus
            <select
              value={String(frame.bus)}
              onFocus={onFocus}
              onChange={(event) =>
                commit({
                  ...frame,
                  bus: Number((event.target as HTMLSelectElement).value),
                })
              }
            >
              {ethBuses.map((bus) => (
                <option value={String(bus.index)} key={bus.index}>
                  {bus.name}
                </option>
              ))}
            </select>
          </label>
          <label>
            Service
            <input
              type="number"
              value={frame.service}
              onChange={(event) =>
                commit({
                  ...frame,
                  service: Number((event.target as HTMLInputElement).value),
                  values: [],
                  mode: "raw",
                })
              }
            />
          </label>
          <label>
            Method
            <input
              type="number"
              value={frame.method}
              onChange={(event) =>
                commit({
                  ...frame,
                  method: Number((event.target as HTMLInputElement).value),
                  values: [],
                  mode: "raw",
                })
              }
            />
          </label>
          <label>
            Client
            <input
              type="number"
              value={frame.client}
              onChange={(event) =>
                commit({
                  ...frame,
                  client: Number((event.target as HTMLInputElement).value),
                })
              }
            />
          </label>
          <label>
            Session (0 = auto)
            <input
              type="number"
              value={frame.session}
              onChange={(event) =>
                commit({
                  ...frame,
                  session: Number((event.target as HTMLInputElement).value),
                })
              }
            />
          </label>
          <label title="0x00 request, 0x02 notification, 0x80 response">
            Type
            <input
              type="number"
              value={frame.messageType}
              onChange={(event) =>
                commit({
                  ...frame,
                  messageType: Number(
                    (event.target as HTMLInputElement).value,
                  ),
                })
              }
            />
          </label>
          <label>
            Transport
            <select
              value={frame.transport ?? "udp"}
              onChange={(event) =>
                commit({
                  ...frame,
                  transport: (event.target as HTMLSelectElement).value as
                    | "udp"
                    | "tcp",
                })
              }
            >
              <option value="udp">UDP</option>
              <option value="tcp">TCP</option>
            </select>
          </label>
          {(frame.transport ?? "udp") === "tcp" && (
            <>
              <label>
                Host
                <input
                  value={frame.host ?? "127.0.0.1"}
                  onChange={(event) =>
                    commit({
                      ...frame,
                      host: (event.target as HTMLInputElement).value,
                    })
                  }
                />
              </label>
              <label>
                Port
                <input
                  type="number"
                  value={frame.port ?? 30490}
                  onChange={(event) =>
                    commit({
                      ...frame,
                      port: Number((event.target as HTMLInputElement).value),
                    })
                  }
                />
              </label>
            </>
          )}
          {someipFields.length > 0 ? (
            <SomeIpFieldEditor
              frame={frame}
              defs={someipFields}
              invalid={errorField === "someip"}
              onFocus={onFocus}
              onCommit={commit}
              onProblem={(message) => {
                setError(message);
                setErrorField("someip");
              }}
            />
          ) : (
            <label>
              Fields (no service description; name=value)
              <input
                key={`${frame.bus}:${frame.service}:${frame.method}`}
                class={errorField === "someip" ? "invalid" : undefined}
                defaultValue={(frame.values ?? [])
                  .map((entry) =>
                    entry.text !== undefined
                      ? `${entry.name}="${entry.text}"`
                      : `${entry.name}=${entry.value}`,
                  )
                  .join("; ")}
                placeholder="speed=12.5; wheel[0]=1"
                spellcheck={false}
                onBlur={(event) => {
                  const text = (event.target as HTMLInputElement).value.trim();
                  if (text === "") {
                    commit({ ...frame, mode: "raw", values: [] });
                    return;
                  }
                  const values: { name: string; value: number }[] = [];
                  for (const part of text.split(";")) {
                    const [rawName, rawValue] = part.split("=");
                    const name = rawName?.trim() ?? "";
                    const value = Number(rawValue?.trim() ?? "");
                    if (!name || !Number.isFinite(value)) {
                      setError(`Field "${part.trim()}" must be name=number`);
                      setErrorField("someip");
                      return;
                    }
                    values.push({ name, value });
                  }
                  setError("");
                  setErrorField(null);
                  commit({ ...frame, mode: "physical", values });
                }}
              />
            </label>
          )}
        </>
      ) : (
        <>
          <label>
            Bus
            <select
              value={String(
                frame?.protocol === "eth" &&
                  ethBuses.some((bus) => bus.index === frame.bus)
                  ? frame.bus
                  : (ethBuses[0]?.index ?? 0),
              )}
              onFocus={onFocus}
              onChange={(event) => {
                const value = (event.target as HTMLSelectElement).value;
                if (value === "") return;
                const bus = Number(value);
                const def = ethFrames.find((entry) => entry.bus === bus);
                readdress({
                  protocol: "eth",
                  bus,
                  frame: def?.name ?? "",
                  data: zeroHex(def?.length ?? null),
                });
              }}
            >
              {ethBuses.map((bus) => (
                <option value={String(bus.index)} key={bus.index}>
                  {bus.name}
                </option>
              ))}
            </select>
          </label>
          <label>
            Frame
            <select
              value={frame?.protocol === "eth" ? frame.frame : ""}
              onFocus={onFocus}
              onChange={(event) => {
                const name = (event.target as HTMLSelectElement).value;
                if (name === "" || frame?.protocol !== "eth") return;
                const def = ethFrames.find(
                  (entry) => entry.bus === frame.bus && entry.name === name,
                );
                readdress({
                  ...frame,
                  frame: name,
                  data: zeroHex(def?.length ?? null),
                });
              }}
            >
              <option value="">Select frame</option>
              {ethFrames
                .filter(
                  (entry) =>
                    frame?.protocol === "eth" && entry.bus === frame.bus,
                )
                .map((entry) => (
                  <option value={entry.name} key={entry.name}>
                    {entry.name}
                  </option>
                ))}
            </select>
          </label>
        </>
      )}
      {modeEnabled && (
        <label>
          Mode
          <select
            value={physical ? "physical" : "raw"}
            onFocus={onFocus}
            onChange={(event) => {
              const mode =
                (event.target as HTMLSelectElement).value === "physical"
                  ? "physical"
                  : "raw";
              const current = base();
              commit({
                ...current,
                mode,
                ...(mode === "raw" ? { values: [] } : {}),
              });
            }}
          >
            <option value="raw">Raw</option>
            <option value="physical">Physical</option>
          </select>
        </label>
      )}
      {physical && frameDef ? (
        <div class="page-frame-values">
          {frameDef.signals.map((signal) => (
            <label
              key={signal.name}
              title={`${signal.name}${signal.unit ? ` · ${signal.unit}` : ""} · ${signal.min}..${signal.max}`}
            >
              <span>{signal.name}</span>
              <input
                type="number"
                step="any"
                value={valueDrafts[signal.name] ?? ""}
                placeholder={
                  signal.initial !== undefined ? String(signal.initial) : "0"
                }
                onFocus={onFocus}
                onInput={(event) => {
                  const value = (event.target as HTMLInputElement).value;
                  setValueDrafts((drafts) => ({
                    ...drafts,
                    [signal.name]: value,
                  }));
                }}
                onBlur={(event) =>
                  commitValue(signal.name, (event.target as HTMLInputElement).value)
                }
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    event.preventDefault();
                    (event.target as HTMLInputElement).blur();
                  }
                }}
              />
              {signal.unit && <em>{signal.unit}</em>}
            </label>
          ))}
          {frameDef.signals.length === 0 && (
            <span class="page-frame-hint">
              This frame declares no signals; use Raw for its payload.
            </span>
          )}
        </div>
      ) : (
        <label>
          Data (hex)
          <input
            value={dataDraft}
            class={errorField === "data" ? "invalid" : undefined}
            aria-invalid={errorField === "data"}
            placeholder="02 10 03"
            spellcheck={false}
            maxLength={8192}
            onFocus={() => {
              onFocus();
              setEditing("data");
            }}
            onInput={(event) =>
              setDataDraft((event.target as HTMLInputElement).value)
            }
            onBlur={commitData}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                (event.target as HTMLInputElement).blur();
              } else if (event.key === "Escape") {
                event.preventDefault();
                setEditing(null);
                setDataDraft(frame?.data ?? "");
                (event.target as HTMLInputElement).blur();
              }
            }}
          />
        </label>
      )}
      {error && (
        <span class="page-frame-error" role="alert" aria-live="polite">
          {error}
        </span>
      )}
      <span class="page-frame-hint">{dataHint}</span>
    </div>
  );
}


