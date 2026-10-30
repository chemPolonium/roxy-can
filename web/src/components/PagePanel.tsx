import { useEffect, useRef, useState } from "preact/hooks";
import {
  PAGE_GRID,
  PAGE_MAX_H,
  PAGE_MAX_W,
  PAGE_MIN_H,
  PAGE_MIN_W,
  PANEL_BAND_COLORS,
  PANEL_MAX_BANDS,
  PANEL_MAX_WRITES,
  PANEL_WIDGET_KINDS,
  activePanelPage,
  addPanelPage,
  addPanelPageFromFile,
  addPanelWidget,
  duplicatePanelPage,
  duplicatePanelWidget,
  exportPanelFile,
  formatPanelValue,
  importPanelFile,
  movePanelWidget,
  panelFileName,
  panelFrameProblem,
  panelSetpointTarget,
  parsePanelHex,
  formatPanelHex,
  panelWidgetSpec,
  pastePanelWidget,
  removePanelPage,
  removePanelWidget,
  renamePanelPage,
  resizePanelPage,
  selectPanelPage,
  updatePanelPage,
  updatePanelWidget,
  widgetMinSize,
  type PanelBand,
  type PanelBandColor,
  type PanelBook,
  type PanelGauge,
  type PanelPage,
  type PanelWidget,
  type PanelWidgetKind,
  type PanelWidgetPatch,
  type PanelToggle,
  type PanelButton,
  type PanelSetpoint,
  type PanelWrite,
  type PanelFrameSend,
} from "../panelPage";
import {
  clampPanelNumber,
  filterPanelOptions,
  findPanelSysVar,
  panelGaugeArc,
  panelLampLit,
  panelMeterRatio,
  panelValueBand,
  panelWidgetCapability,
  resolvePanelSource,
  type PanelActionContext,
  type PanelFrameDef,
  type PanelView,
} from "../panelBinding";
import { FilePick } from "./FilePick";
import { copyText } from "../clipboard";

interface DragState {
  kind: "move" | "resize";
  id: string;
  pointerX: number;
  pointerY: number;
  x: number;
  y: number;
  w: number;
  h: number;
}

interface History {
  past: PanelPage[];
  future: PanelPage[];
}

type PageMenu =
  | { kind: "widget"; x: number; y: number; id: string }
  | { kind: "canvas"; x: number; y: number }
  | { kind: "page"; x: number; y: number; id: string };

/// The signal a read-only or setpoint cell watches, for "Copy signal key".
function panelSignalKey(widget: PanelWidget): string {
  if (
    widget.kind === "value" ||
    widget.kind === "lamp" ||
    widget.kind === "bar" ||
    widget.kind === "gauge"
  ) {
    return widget.source.trim();
  }
  if (widget.kind === "setpoint" && panelSetpointTarget(widget) === "signal") {
    return widget.source.trim();
  }
  return "";
}

const EMPTY_HISTORY: History = { past: [], future: [] };

function SysVarSetpointControl({
  widget,
  value,
  min,
  max,
  unit,
  disabled,
  reason,
  title,
  onCommit,
}: {
  widget: PanelSetpoint;
  value: number | null;
  min: number | null;
  max: number | null;
  unit: string;
  disabled: boolean;
  reason: string;
  title?: string;
  onCommit: (value: number) => void;
}) {
  const [draft, setDraft] = useState("");
  const [editing, setEditing] = useState(false);
  useEffect(() => {
    if (!editing) {
      setDraft(value === null ? "" : value.toFixed(widget.digits));
    }
  }, [value, editing, widget.digits]);
  const commit = () => {
    setEditing(false);
    const parsed = draft.trim() === "" ? Number.NaN : Number(draft);
    if (!Number.isFinite(parsed)) {
      setDraft(value === null ? "" : value.toFixed(widget.digits));
      return;
    }
    let next = clampPanelNumber(parsed, min, max);
    onCommit(next);
  };
  return (
    <div class="page-setpoint" title={disabled ? reason : title ?? widget.sysvar}>
      {widget.label && <span class="page-setpoint-label">{widget.label}</span>}
      <span class="page-setpoint-row">
        <input
          type="number"
          value={draft}
          step={widget.step}
          disabled={disabled}
          onFocus={() => setEditing(true)}
          onInput={(event) => setDraft((event.target as HTMLInputElement).value)}
          onBlur={commit}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              (event.target as HTMLInputElement).blur();
            } else if (event.key === "Escape") {
              setDraft(value === null ? "" : value.toFixed(widget.digits));
              (event.target as HTMLInputElement).blur();
            }
          }}
        />
        {unit && <em>{unit}</em>}
      </span>
    </div>
  );
}

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

function FrameField({
  frame,
  canBuses,
  frBuses,
  ethBuses,
  ethFrames,
  frameDefs,
  onFocus,
  onChange,
}: {
  frame: PanelFrameSend | undefined;
  canBuses: { index: number; name: string }[];
  frBuses: { index: number; name: string }[];
  ethBuses: { index: number; name: string }[];
  ethFrames: { bus: number; name: string; length: number | null }[];
  frameDefs: PanelFrameDef[];
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
  const protocol = frame?.protocol ?? "can";

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
      return false;
    }
    setError("");
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

  const defaultFor = (kind: "can" | "fr" | "eth"): PanelFrameSend => {
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
      const value = Number(trimmed);
      if (!Number.isFinite(value)) {
        setError(`Value for ${name} must be a number`);
        return;
      }
      values.push({ name, value });
    }
    commit({ ...current, mode: "physical", values });
  };

  const dataHint =
    protocol === "can"
      ? source === "catalog"
        ? "Sends once when clicked."
        : "Raw frame. Pick a defined message to edit physical values."
      : protocol === "fr"
        ? "Session only — the FR port stays receive-only."
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
            if (next !== "can" && next !== "fr" && next !== "eth") return;
            setError("");
            onChange(defaultFor(next));
          }}
        >
          <option value="can">CAN</option>
          <option value="fr">FlexRay</option>
          <option value="eth">Ethernet</option>
        </select>
      </label>
      {protocol === "can" ? (
        <>
          <label>
            Bus
            <select
              value={
                frame?.protocol === "can" ? String(frame.channel) : ""
              }
              onFocus={onFocus}
              onChange={(event) => {
                const value = (event.target as HTMLSelectElement).value;
                const current = base();
                if (value === "" || current.protocol !== "can") return;
                readdress({ ...current, channel: Number(value) });
              }}
            >
              {canBuses.map((bus) => (
                <option value={bus.index} key={bus.index}>
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
              value={frame?.protocol === "fr" ? String(frame.bus) : ""}
              onFocus={onFocus}
              onChange={(event) => {
                const value = (event.target as HTMLSelectElement).value;
                const current = base();
                if (value === "" || current.protocol !== "fr") return;
                readdress({ ...current, bus: Number(value) });
              }}
            >
              {frBuses.map((bus) => (
                <option value={bus.index} key={bus.index}>
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
      ) : (
        <>
          <label>
            Bus
            <select
              value={frame?.protocol === "eth" ? String(frame.bus) : ""}
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
                <option value={bus.index} key={bus.index}>
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
      {error && <span class="page-frame-error">{error}</span>}
      <span class="page-frame-hint">{dataHint}</span>
    </div>
  );
}

function SetpointControl({
  widget,
  value,
  min,
  max,
  unit,
  disabled,
  reason,
  onWrite,
}: {
  widget: PanelSetpoint;
  /// Live value read back from the bus; falls back to the last write.
  value: number | null;
  min: number | null;
  max: number | null;
  unit: string;
  disabled: boolean;
  reason: string;
  onWrite: (value: number) => void;
}) {
  const [sent, setSent] = useState<number | null>(null);
  useEffect(() => {
    setSent(null);
  }, [widget.source]);
  const shape: PanelSetpoint = {
    id: widget.id,
    kind: "setpoint",
    label: widget.label,
    target: "signal",
    sysvar: "",
    source: widget.source,
    digits: widget.digits,
    step: widget.step,
    x: widget.x,
    y: widget.y,
    w: widget.w,
    h: widget.h,
  };
  const title = [
    widget.source,
    value !== null ? `value ${formatPanelValue(value, widget.digits)}` : "",
    sent !== null ? `written ${formatPanelValue(sent, widget.digits)}` : "",
    widget.source.startsWith("fr:") ||
    widget.source.startsWith("eth:") ||
    widget.source.startsWith("sig:fr:") ||
    widget.source.startsWith("sig:eth:")
      ? "session only"
      : "",
  ]
    .filter(Boolean)
    .join(" · ");
  return (
    <SysVarSetpointControl
      widget={shape}
      value={value ?? sent}
      min={min}
      max={max}
      unit={unit}
      disabled={disabled}
      reason={reason}
      title={title}
      onCommit={(next) => {
        setSent(next);
        onWrite(next);
      }}
    />
  );
}

function DraftNumberInput({
  value,
  label,
  placeholder,
  onFocus,
  onCommit,
}: {
  value: number;
  label?: string;
  placeholder?: string;
  onFocus: () => void;
  onCommit: (value: number) => void;
}) {
  const [draft, setDraft] = useState(() => String(value));
  const [editing, setEditing] = useState(false);
  useEffect(() => {
    if (!editing) setDraft(String(value));
  }, [value, editing]);
  const commit = () => {
    setEditing(false);
    const parsed = Number(draft);
    if (draft.trim() === "" || !Number.isFinite(parsed)) {
      setDraft(String(value));
      return;
    }
    onCommit(parsed);
  };
  return (
    <input
      class={label}
      type="number"
      step="any"
      value={draft}
      placeholder={placeholder}
      onFocus={() => {
        setEditing(true);
        onFocus();
      }}
      onInput={(event) => setDraft((event.target as HTMLInputElement).value)}
      onBlur={commit}
      onKeyDown={(event) => {
        if (event.key === "Enter") {
          (event.target as HTMLInputElement).blur();
        } else if (event.key === "Escape") {
          setDraft(String(value));
          (event.target as HTMLInputElement).blur();
        }
      }}
    />
  );
}

function BandsField({
  label,
  bands,
  onFocus,
  onChange,
}: {
  label: string;
  bands: PanelBand[];
  onFocus: () => void;
  onChange: (bands: PanelBand[]) => void;
}) {
  const hasAbove = bands.some((band) => band.upTo === null);
  const full = bands.length >= PANEL_MAX_BANDS;
  const addBand = () => {
    const top = bands.reduce<number | null>(
      (limit, band) =>
        band.upTo === null ? limit : limit === null ? band.upTo : Math.max(limit, band.upTo),
      null,
    );
    onChange([...bands, { upTo: top === null ? 1 : top + 1, color: "ok" }]);
  };
  const setBand = (index: number, patch: Partial<PanelBand>) =>
    onChange(
      bands.map((band, at) => (at === index ? { ...band, ...patch } : band)),
    );
  return (
    <div class="page-bands">
      <div class="page-bands-title">
        <span>{label}</span>
        <span>
          {bands.length}/{PANEL_MAX_BANDS}
        </span>
      </div>
      {bands.length === 0 && (
        <span class="page-bands-empty">
          No bands: the value keeps its normal colour.
        </span>
      )}
      {bands.map((band, index) => (
        <div class="page-band-row" key={index}>
          {band.upTo === null ? (
            <span class="page-band-above">above</span>
          ) : (
            <span class="page-band-up">
              ≤
              <DraftNumberInput
                value={band.upTo}
                label="page-band-limit"
                onFocus={onFocus}
                onCommit={(upTo) => setBand(index, { upTo })}
              />
            </span>
          )}
          <select
            class="page-band-color"
            value={band.color}
            onFocus={onFocus}
            onChange={(event) =>
              setBand(index, {
                color: (event.target as HTMLSelectElement)
                  .value as PanelBandColor,
              })
            }
          >
            {PANEL_BAND_COLORS.map((option) => (
              <option value={option.value} key={option.value}>
                {option.label}
              </option>
            ))}
          </select>
          <button
            type="button"
            class="mini page-band-remove"
            title="Remove band"
            aria-label="Remove band"
            onClick={() => onChange(bands.filter((_, at) => at !== index))}
          >
            <svg viewBox="0 0 16 16" aria-hidden="true">
              <path d="M4.5 4.5l7 7M11.5 4.5l-7 7" />
            </svg>
          </button>
        </div>
      ))}
      <div class="page-band-add">
        <button
          type="button"
          class="mini"
          disabled={full}
          title="Colour values up to a threshold"
          onClick={addBand}
        >
          + Band
        </button>
        <button
          type="button"
          class="mini"
          disabled={full || hasAbove}
          title="Colour everything above the last threshold"
          onClick={() => onChange([...bands, { upTo: null, color: "crit" }])}
        >
          + Above
        </button>
      </div>
    </div>
  );
}

function meterTone(value: number | null, bands?: PanelBand[]): string {
  const band = panelValueBand(value, bands);
  return band && band !== "neutral" ? `band-${band}` : "meter-accent";
}

function GaugeView({
  widget,
  value,
  unit,
}: {
  widget: PanelGauge;
  value: number | null;
  unit: string;
}) {
  const ratio = panelMeterRatio(value, widget.min, widget.max);
  return (
    <svg
      class="page-gauge-svg"
      viewBox="0 0 100 64"
      preserveAspectRatio="xMidYMid meet"
      aria-hidden="true"
    >
      <path class="page-gauge-track" d="M6 50 A44 44 0 0 1 94 50" />
      {ratio !== null && ratio > 0 && (
        <path
          class={`page-gauge-fill ${meterTone(value, widget.bands)}`}
          d={panelGaugeArc(ratio)}
        />
      )}
      <text class="page-gauge-value" x="50" y="45">
        {formatPanelValue(value, widget.digits)}
      </text>
      {unit && (
        <text class="page-gauge-unit" x="50" y="57">
          {unit}
        </text>
      )}
      <text class="page-gauge-range" x="6" y="62">
        {formatPanelValue(widget.min, 0)}
      </text>
      <text class="page-gauge-range end" x="94" y="62">
        {formatPanelValue(widget.max, 0)}
      </text>
    </svg>
  );
}

function SuggestionInput({
  value,
  options,
  placeholder,
  maxLength,
  onFocus,
  onCommit,
}: {
  value: string;
  options: string[];
  placeholder?: string;
  maxLength?: number;
  onFocus: () => void;
  onCommit: (value: string) => void;
}) {
  const input = useRef<HTMLInputElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState<string | null>(null);
  const [active, setActive] = useState(-1);
  const [anchor, setAnchor] = useState<{
    left: number;
    top: number;
    width: number;
    maxHeight: number;
  } | null>(null);
  // Focusing an already bound field lists everything; typing narrows it.
  const list = filterPanelOptions(options, query ?? "");

  const show = () => {
    const box = input.current?.getBoundingClientRect();
    if (!box) return;
    setAnchor({
      left: box.left,
      top: box.bottom + 2,
      width: box.width,
      maxHeight: Math.max(60, window.innerHeight - box.bottom - 12),
    });
    setOpen(true);
  };

  useEffect(() => {
    if (!open) return;
    const inside = (node: Node | null) =>
      !!node && (input.current?.contains(node) || menu.current?.contains(node));
    const closeOnPointer = (event: PointerEvent) => {
      if (!inside(event.target as Node)) setOpen(false);
    };
    const closeOnScroll = (event: Event) => {
      if (inside(event.target as Node)) return;
      setOpen(false);
    };
    const closeOnResize = () => setOpen(false);
    window.addEventListener("pointerdown", closeOnPointer);
    window.addEventListener("scroll", closeOnScroll, true);
    window.addEventListener("resize", closeOnResize);
    return () => {
      window.removeEventListener("pointerdown", closeOnPointer);
      window.removeEventListener("scroll", closeOnScroll, true);
      window.removeEventListener("resize", closeOnResize);
    };
  }, [open]);

  const pick = (option: string) => {
    onCommit(option);
    setQuery(null);
    setActive(-1);
    setOpen(false);
  };

  return (
    <div class="page-suggest">
      <input
        ref={input}
        value={value}
        placeholder={placeholder}
        maxLength={maxLength}
        spellcheck={false}
        onFocus={() => {
          onFocus();
          setQuery(null);
          setActive(-1);
          show();
        }}
        onBlur={(event) => {
          const next = event.relatedTarget as Node | null;
          if (next && menu.current?.contains(next)) return;
          setOpen(false);
        }}
        onInput={(event) => {
          const next = (event.target as HTMLInputElement).value;
          setQuery(next);
          setActive(-1);
          show();
          onCommit(next);
        }}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown" || event.key === "ArrowUp") {
            event.preventDefault();
            if (!open) {
              setQuery(null);
              show();
              return;
            }
            const step = event.key === "ArrowDown" ? 1 : -1;
            setActive((old) => {
              if (list.length === 0) return -1;
              const next = old + step;
              return next < 0 ? list.length - 1 : next >= list.length ? 0 : next;
            });
            return;
          }
          if (event.key === "Enter") {
            event.preventDefault();
            if (open && active >= 0 && list[active]) {
              pick(list[active]);
            } else {
              setOpen(false);
            }
            return;
          }
          if (event.key === "Escape" && open) {
            event.preventDefault();
            // Keep the editor's own Esc handler from clearing the selection.
            event.stopPropagation();
            setOpen(false);
            setActive(-1);
          }
        }}
      />
      {open && anchor && list.length > 0 && (
        <div
          ref={menu}
          class="page-suggest-menu"
          role="listbox"
          style={{
            left: `${anchor.left}px`,
            top: `${anchor.top}px`,
            width: `${anchor.width}px`,
            maxHeight: `${Math.min(184, anchor.maxHeight)}px`,
          }}
        >
          {list.map((option, index) => (
            <button
              type="button"
              role="option"
              aria-selected={index === active}
              class={index === active ? "active" : ""}
              key={option}
              title={option}
              onMouseDown={(event) => event.preventDefault()}
              onClick={() => pick(option)}
            >
              {option}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

export function PagePanel({
  book,
  revision,
  toast,
  sourceOptions,
  sysvarOptions,
  actionNames,
  view,
  actionContext,
  onBook,
  onRun,
  onToggle,
  onSetNumber,
  onWrite,
  onNotice,
}: {
  book: PanelBook;
  revision: number;
  toast?: string;
  sourceOptions: string[];
  sysvarOptions: string[];
  actionNames: string[];
  view: PanelView;
  actionContext: PanelActionContext;
  onBook: (book: PanelBook) => void;
  onRun: (widget: PanelButton) => void;
  onToggle: (widget: PanelToggle, on: boolean) => void;
  onSetNumber: (widget: PanelSetpoint, value: number) => void;
  onWrite: (widget: PanelSetpoint, value: number) => void;
  onNotice: (message: string) => void;
}) {
  const page = activePanelPage(book);
  const onPage = (next: PanelPage) => onBook(updatePanelPage(book, next));
  const [editing, setEditing] = useState(false);
  const [fullscreen, setFullscreen] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<{ id: string; name: string } | null>(
    null,
  );
  const renameCanceled = useRef(false);
  const [drag, setDrag] = useState<DragState | null>(null);
  const [history, setHistory] = useState<History>(EMPTY_HISTORY);
  const [clipboard, setClipboard] = useState<PanelWidget | null>(null);
  const [sizeW, setSizeW] = useState(String(page.width * PAGE_GRID));
  const [sizeH, setSizeH] = useState(String(page.height * PAGE_GRID));
  const [addOpen, setAddOpen] = useState(false);
  const addTrigger = useRef<HTMLButtonElement>(null);
  const addMenu = useRef<HTMLDivElement>(null);
  const [menu, setMenu] = useState<PageMenu | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    setSizeW(String(page.width * PAGE_GRID));
    setSizeH(String(page.height * PAGE_GRID));
  }, [page.width, page.height]);

  useEffect(() => {
    setHistory(EMPTY_HISTORY);
    setClipboard(null);
    setSelected(null);
    setRenaming(null);
  }, [revision, page.id]);

  useEffect(() => {
    if (!editing) setSelected(null);
  }, [editing]);

  useEffect(() => {
    if (!editing) setAddOpen(false);
  }, [editing]);

  useEffect(() => {
    if (!addOpen) return;
    addMenu.current?.querySelector<HTMLButtonElement>("button")?.focus();
  }, [addOpen]);

  useEffect(() => {
    if (!menu) return;
    menuRef.current
      ?.querySelector<HTMLButtonElement>("button:not(:disabled)")
      ?.focus();
    const close = (event: PointerEvent) => {
      if (!menuRef.current?.contains(event.target as Node)) setMenu(null);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      // Capture phase: the editor's Esc handler must not leave Edit too.
      event.stopImmediatePropagation();
      setMenu(null);
    };
    window.addEventListener("pointerdown", close);
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("pointerdown", close);
      window.removeEventListener("keydown", onKey, true);
    };
  }, [menu]);

  useEffect(() => {
    if (!addOpen) return;
    const close = (event: PointerEvent) => {
      if (!(event.target as HTMLElement).closest(".page-add")) {
        setAddOpen(false);
      }
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      // Capture phase: the editor's own Esc handler must not leave Edit too.
      event.stopImmediatePropagation();
      setAddOpen(false);
      addTrigger.current?.focus();
    };
    window.addEventListener("pointerdown", close);
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("pointerdown", close);
      window.removeEventListener("keydown", onKey, true);
    };
  }, [addOpen]);

  useEffect(() => {
    if (!editing) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      if (selected) {
        setSelected(null);
      } else {
        setEditing(false);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [editing, selected]);

  useEffect(() => {
    if (!fullscreen || editing) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      if (document.querySelector(".modal-backdrop")) return;
      setFullscreen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [fullscreen, editing]);

  useEffect(() => {
    if (!drag) return;
    const min = widgetMinSize(
      page.widgets.find((widget) => widget.id === drag.id)?.kind ?? "button",
    );
    const move = (event: PointerEvent) => {
      const dx = Math.round((event.clientX - drag.pointerX) / PAGE_GRID);
      const dy = Math.round((event.clientY - drag.pointerY) / PAGE_GRID);
      const next =
        drag.kind === "move"
          ? movePanelWidget(
              page,
              drag.id,
              drag.x + dx,
              drag.y + dy,
              drag.w,
              drag.h,
            )
          : movePanelWidget(
              page,
              drag.id,
              drag.x,
              drag.y,
              Math.max(min.w, drag.w + dx),
              Math.max(min.h, drag.h + dy),
            );
      if (next) onPage(next);
    };
    const up = () => setDrag(null);
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
    };
  }, [drag, page, onPage]);

  const selectedWidget = page.widgets.find((widget) => widget.id === selected);

  const pushHistory = () => {
    setHistory((old) => ({
      past: [...old.past.slice(-49), page],
      future: [],
    }));
  };

  const commit = (next: PanelPage) => {
    if (next === page) return;
    pushHistory();
    onPage(next);
  };

  const undo = () => {
    if (history.past.length === 0) return;
    const previous = history.past[history.past.length - 1];
    setHistory({
      past: history.past.slice(0, -1),
      future: [page, ...history.future].slice(0, 50),
    });
    onPage(previous);
    setSelected(null);
  };

  const redo = () => {
    if (history.future.length === 0) return;
    const next = history.future[0];
    setHistory({
      past: [...history.past.slice(-49), page],
      future: history.future.slice(1),
    });
    onPage(next);
    setSelected(null);
  };

  const removeWidget = (id: string) => {
    commit(removePanelWidget(page, id));
    setSelected(null);
  };

  const duplicateWidget = (id: string) => {
    const copy = duplicatePanelWidget(page, id);
    if (!copy) {
      onNotice("No free space for a copy");
      return;
    }
    commit(copy.page);
    setSelected(copy.id);
  };

  const copyWidget = (id: string) => {
    const widget = page.widgets.find((entry) => entry.id === id);
    if (widget) setClipboard({ ...widget });
  };

  const copySignalKey = (id: string) => {
    const widget = page.widgets.find((entry) => entry.id === id);
    const key = widget ? panelSignalKey(widget) : "";
    if (!key) return;
    void copyText(key).then((copied) =>
      onNotice(copied ? `Copied ${key}` : "Could not copy the signal key"),
    );
  };

  const removeSelected = () => {
    if (selectedWidget) removeWidget(selectedWidget.id);
  };

  const duplicateSelected = () => {
    if (selectedWidget) duplicateWidget(selectedWidget.id);
  };

  const copySelected = () => {
    if (selectedWidget) copyWidget(selectedWidget.id);
  };

  const pasteClipboard = () => {
    if (!clipboard) return;
    const copy = pastePanelWidget(page, clipboard);
    if (!copy) {
      onNotice("No free space for a paste");
      return;
    }
    commit(copy.page);
    setSelected(copy.id);
  };

  useEffect(() => {
    if (!editing) return;
    const onKey = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      if (
        target &&
        (target.tagName === "INPUT" ||
          target.tagName === "TEXTAREA" ||
          target.tagName === "SELECT" ||
          target.isContentEditable)
      ) {
        return;
      }
      if (event.key === "Delete" || event.key === "Backspace") {
        if (!selectedWidget) return;
        event.preventDefault();
        removeSelected();
        return;
      }
      if (!(event.ctrlKey || event.metaKey)) return;
      const key = event.key.toLowerCase();
      if (key === "c") {
        if (!selectedWidget) return;
        event.preventDefault();
        copySelected();
      } else if (key === "v") {
        event.preventDefault();
        pasteClipboard();
      } else if (key === "z" && !event.shiftKey) {
        event.preventDefault();
        undo();
      } else if (key === "y" || (key === "z" && event.shiftKey)) {
        event.preventDefault();
        redo();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [editing, selectedWidget, clipboard, history, page]);

  const startDrag = (
    event: PointerEvent,
    id: string,
    kind: "move" | "resize",
  ) => {
    if (!editing || event.button !== 0) return;
    const widget = page.widgets.find((entry) => entry.id === id);
    if (!widget) return;
    event.preventDefault();
    event.stopPropagation();
    pushHistory();
    setSelected(id);
    setDrag({
      kind,
      id,
      pointerX: event.clientX,
      pointerY: event.clientY,
      x: widget.x,
      y: widget.y,
      w: widget.w,
      h: widget.h,
    });
  };

  const clearPageUi = () => {
    setSelected(null);
    setHistory(EMPTY_HISTORY);
    setClipboard(null);
    setRenaming(null);
  };

  const switchPage = (id: string) => {
    if (drag || id === page.id) return;
    clearPageUi();
    onBook(selectPanelPage(book, id));
  };

  const newPage = () => {
    const added = addPanelPage(book);
    clearPageUi();
    onBook(added.book);
  };

  const copyPageById = (id: string) => {
    const copy = duplicatePanelPage(book, id);
    if (!copy) {
      onNotice("Cannot duplicate this page");
      return;
    }
    clearPageUi();
    onBook(copy.book);
  };

  const deletePageById = (id: string) => {
    if (book.pages.length <= 1) {
      onNotice("Keep at least one page");
      return;
    }
    clearPageUi();
    onBook(removePanelPage(book, id));
  };

  const copyPage = () => copyPageById(page.id);

  const deletePage = () => deletePageById(page.id);

  const commitRename = () => {
    if (!renaming) return;
    if (renameCanceled.current) {
      renameCanceled.current = false;
      setRenaming(null);
      return;
    }
    onBook(renamePanelPage(book, renaming.id, renaming.name));
    setRenaming(null);
  };

  const add = (kind: PanelWidgetKind) => {
    const added = addPanelWidget(page, kind);
    if (!added) {
      onNotice("No free space on this page");
      return;
    }
    commit(added.page);
    setSelected(added.id);
  };

  const exportPageById = (id: string) => {
    const target = book.pages.find((entry) => entry.id === id) ?? page;
    const name = panelFileName(target.name);
    const blob = new Blob([exportPanelFile(target)], {
      type: "application/json",
    });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = name;
    link.click();
    window.setTimeout(() => URL.revokeObjectURL(url), 0);
    onNotice(`Exported ${name}`);
  };

  const exportPage = () => exportPageById(page.id);

  const importPage = (text: string) => {
    let raw: unknown;
    try {
      raw = JSON.parse(text);
    } catch {
      onNotice("That file is not valid JSON");
      return;
    }
    const result = importPanelFile(raw);
    if ("error" in result) {
      onNotice(result.error);
      return;
    }
    const added = addPanelPageFromFile(book, result.page);
    clearPageUi();
    onBook(added.book);
    onNotice(`Imported ${added.page.name}`);
  };

  const resetSize = () => {
    setSizeW(String(page.width * PAGE_GRID));
    setSizeH(String(page.height * PAGE_GRID));
  };

  const applySize = () => {
    const w = Number(sizeW);
    const h = Number(sizeH);
    if (!Number.isFinite(w) || !Number.isFinite(h)) {
      onNotice("Canvas width and height must be numbers");
      resetSize();
      return;
    }
    const next = resizePanelPage(
      page,
      Math.round(w / PAGE_GRID),
      Math.round(h / PAGE_GRID),
    );
    if (!next) {
      onNotice("Move controls inside the new canvas first");
      resetSize();
      return;
    }
    commit(next);
    setSizeW(String(next.width * PAGE_GRID));
    setSizeH(String(next.height * PAGE_GRID));
  };

  const menuWidget =
    menu?.kind === "widget"
      ? page.widgets.find((widget) => widget.id === menu.id)
      : undefined;
  const menuSignal = menuWidget ? panelSignalKey(menuWidget) : "";
  // Flip above the click when the menu would run off the bottom; the canvas
  // menu is the tallest (Paste, seven controls, Undo/Redo).
  const menuHeight =
    menu?.kind === "canvas" ? 330 : menu?.kind === "widget" ? 150 : 150;
  const menuTop = menu
    ? menu.y + menuHeight > window.innerHeight - 8
      ? Math.max(8, menu.y - menuHeight)
      : menu.y
    : 0;
  const menuLeft = menu ? Math.min(menu.x, window.innerWidth - 200) : 0;

  const capabilityOf = (widget: PanelWidget) =>
    panelWidgetCapability(widget, view, actionContext);

  const control = (widget: PanelWidget) => {
    const capability = capabilityOf(widget);
    if (widget.kind === "value") {
      const resolved = resolvePanelSource(widget.source, view);
      const band = capability.enabled
        ? panelValueBand(resolved.value, widget.bands)
        : null;
      return (
        <div
          class={`page-value ${capability.enabled ? "" : "missing"}`}
          title={capability.enabled ? widget.source : capability.reason}
        >
          <span class="page-value-head">
            {widget.label && (
              <span class="page-value-label">{widget.label}</span>
            )}
            {resolved.unit && <em>{resolved.unit}</em>}
          </span>
          <span class="page-value-line">
            <strong
              class={band && band !== "neutral" ? `band-${band}` : undefined}
            >
              {formatPanelValue(resolved.value, widget.digits)}
            </strong>
          </span>
        </div>
      );
    }
    if (widget.kind === "setpoint") {
      if (panelSetpointTarget(widget) === "signal") {
        const target = (view.writeTargets ?? []).find(
          (entry) => entry.key === widget.source,
        );
        const resolved = resolvePanelSource(widget.source, view);
        return (
          <SetpointControl
            widget={widget}
            value={resolved.value}
            min={target?.min ?? null}
            max={target?.max ?? null}
            unit={target?.unit ?? resolved.unit}
            disabled={editing || !capability.enabled}
            reason={capability.reason}
            onWrite={(value) => {
              if (!editing && capability.enabled) onWrite(widget, value);
            }}
          />
        );
      }
      const variable = findPanelSysVar(widget.sysvar, view);
      const disabled = editing || !capability.enabled;
      return (
        <SysVarSetpointControl
          widget={widget}
          value={variable?.value ?? null}
          min={variable?.min ?? null}
          max={variable?.max ?? null}
          unit={variable?.unit ?? ""}
          disabled={disabled}
          reason={capability.reason}
          onCommit={(value) => {
            if (!editing && capability.enabled) onSetNumber(widget, value);
          }}
        />
      );
    }
    if (widget.kind === "toggle") {
      const variable = findPanelSysVar(widget.sysvar, view);
      const on = (variable?.value ?? 0) !== 0;
      return (
        <button
          type="button"
          class={`page-toggle ${on ? "on" : ""} ${
            capability.enabled ? "" : "disabled"
          }`}
          title={capability.enabled ? widget.sysvar : capability.reason}
          onClick={() => {
            if (!editing && capability.enabled) onToggle(widget, !on);
          }}
        >
          <span class="page-toggle-label">
            {widget.label || widget.sysvar || "Toggle"}
          </span>
          <span class="page-toggle-state">{on ? "ON" : "OFF"}</span>
        </button>
      );
    }
    if (widget.kind === "lamp") {
      const resolved = resolvePanelSource(widget.source, view);
      const lit = capability.enabled && panelLampLit(widget, resolved.value);
      return (
        <div
          class={`page-lamp tone-${widget.color} ${lit ? "on" : ""} ${
            capability.enabled ? "" : "missing"
          }`}
          title={
            capability.enabled
              ? `${widget.source} · ${formatPanelValue(resolved.value, 1)}`
              : capability.reason
          }
        >
          <span class="page-lamp-dot" />
          <span class="page-lamp-label">
            {widget.label || widget.source || "Lamp"}
          </span>
        </div>
      );
    }
    if (widget.kind === "bar") {
      const resolved = resolvePanelSource(widget.source, view);
      const ratio = capability.enabled
        ? panelMeterRatio(resolved.value, widget.min, widget.max)
        : null;
      return (
        <div
          class={`page-bar ${capability.enabled ? "" : "missing"}`}
          title={
            capability.enabled
              ? `${widget.source} · ${formatPanelValue(
                  widget.min,
                  0,
                )}..${formatPanelValue(widget.max, 0)}`
              : capability.reason
          }
        >
          <span class="page-bar-head">
            <span class="page-bar-label">{widget.label || "Bar"}</span>
            <span class="page-bar-value">
              {formatPanelValue(resolved.value, widget.digits)}
              {resolved.unit && <em>{resolved.unit}</em>}
            </span>
          </span>
          <span class="page-bar-track">
            <span
              class={`page-bar-fill ${meterTone(resolved.value, widget.bands)}`}
              style={{ width: `${(ratio ?? 0) * 100}%` }}
            />
          </span>
        </div>
      );
    }
    if (widget.kind === "gauge") {
      const resolved = resolvePanelSource(widget.source, view);
      return (
        <div
          class={`page-gauge ${capability.enabled ? "" : "missing"}`}
          title={capability.enabled ? widget.source : capability.reason}
        >
          <span class="page-gauge-label">{widget.label || "Gauge"}</span>
          <GaugeView
            widget={widget}
            value={resolved.value}
            unit={resolved.unit}
          />
        </div>
      );
    }
    return (
      <button
        type="button"
        class={`page-button ${capability.enabled ? "" : "disabled"}`}
        title={capability.enabled ? widget.label : capability.reason}
        onClick={() => {
          if (!editing && capability.enabled) onRun(widget);
        }}
      >
        {widget.label ||
          (widget.action === "none"
            ? "Preset"
            : widget.action === "send"
              ? "Send"
              : widget.script || "Script") ||
          "Button"}
      </button>
    );
  };

  const field = (widget: PanelWidget, key: keyof PanelWidgetPatch) =>
    panelWidgetSpec(widget.kind).fieldValue(widget, key);

  const inspectorField = (widget: PanelWidget, key: string) => {
    const spec = panelWidgetSpec(widget.kind);
    const fieldSpec = spec.fields.find((field) => field.key === key);
    if (!fieldSpec) return null;
    if (key === "script" && field(widget, "action") !== "script") return null;
    if (key === "argument" && field(widget, "action") !== "script") return null;
    if (key === "writes" && field(widget, "action") !== "none") return null;
    if (key === "frame" && field(widget, "action") !== "send") return null;
    if (
      widget.kind === "setpoint" &&
      key === "sysvar" &&
      field(widget, "target") === "signal"
    ) {
      return null;
    }
    if (
      widget.kind === "setpoint" &&
      key === "source" &&
      field(widget, "target") !== "signal"
    ) {
      return null;
    }
    const value = field(widget, fieldSpec.key);
    const update = (patch: PanelWidgetPatch) =>
      commit(updatePanelWidget(page, widget.id, patch));
    if (fieldSpec.kind === "writes") {
      const writes: PanelWrite[] =
        widget.kind === "button" ? (widget.writes ?? []) : [];
      return (
        <div class="page-writes">
          <span class="page-writes-label">{fieldSpec.label}</span>
          {writes.map((write, index) => (
            <div class="page-write-row" key={index}>
              <SuggestionInput
                value={write.source}
                options={(view.writeTargets ?? []).map((target) => target.key)}
                placeholder="channel:id:signal"
                maxLength={120}
                onFocus={pushHistory}
                onCommit={(next) =>
                  update({
                    writes: writes.map((entry, at) =>
                      at === index ? { ...entry, source: next } : entry,
                    ),
                  })
                }
              />
              <span class="page-write-value">
                Value
                <DraftNumberInput
                  value={write.value}
                  placeholder="0"
                  onFocus={pushHistory}
                  onCommit={(next) =>
                    update({
                      writes: writes.map((entry, at) =>
                        at === index ? { ...entry, value: next } : entry,
                      ),
                    })
                  }
                />
              </span>
              <button
                type="button"
                class="mini page-write-remove"
                title="Remove signal write"
                aria-label="Remove signal write"
                onClick={() =>
                  update({
                    writes: writes.filter((_, at) => at !== index),
                  })
                }
              >
                <svg viewBox="0 0 16 16" aria-hidden="true">
                  <path d="M4.5 4.5l7 7M11.5 4.5l-7 7" />
                </svg>
              </button>
            </div>
          ))}
          <button
            type="button"
            class="mini page-write-add"
            disabled={writes.length >= PANEL_MAX_WRITES}
            onClick={() =>
              update({ writes: [...writes, { source: "", value: 0 }] })
            }
          >
            + Signal
          </button>
        </div>
      );
    }
    if (fieldSpec.kind === "frame") {
      return (
        <FrameField
          frame={widget.kind === "button" ? widget.frame : undefined}
          canBuses={view.frameBuses ?? []}
          frBuses={view.frBuses ?? []}
          ethBuses={view.ethBuses ?? []}
          ethFrames={view.ethFrames ?? []}
          frameDefs={view.frameDefs ?? []}
          onFocus={pushHistory}
          onChange={(frame) => update({ frame })}
        />
      );
    }
    if (fieldSpec.kind === "bands") {
      return (
        <BandsField
          label={fieldSpec.label}
          bands={
            widget.kind === "value" ||
            widget.kind === "bar" ||
            widget.kind === "gauge"
              ? widget.bands ?? []
              : []
          }
          onFocus={pushHistory}
          onChange={(bands) => update({ bands })}
        />
      );
    }
    if (fieldSpec.kind === "select") {
      return (
        <label>
          {fieldSpec.label}
          <select
            value={String(value)}
            onFocus={pushHistory}
            onChange={(event) =>
              update({
                [fieldSpec.key]: (event.target as HTMLSelectElement)
                  .value as never,
              } as PanelWidgetPatch)
            }
          >
            {fieldSpec.options?.map((option) => (
              <option value={option.value} key={option.value}>
                {option.label}
              </option>
            ))}
          </select>
        </label>
      );
    }
    if (fieldSpec.kind === "digits") {
      return (
        <label>
          {fieldSpec.label}
          <select
            value={String(value)}
            onFocus={pushHistory}
            onChange={(event) =>
              update({
                [fieldSpec.key]: Number(
                  (event.target as HTMLSelectElement).value,
                ),
              } as PanelWidgetPatch)
            }
          >
            {[0, 1, 2, 3].map((digits) => (
              <option value={digits} key={digits}>
                {digits}
              </option>
            ))}
          </select>
        </label>
      );
    }
    if (fieldSpec.kind === "number") {
      const numeric = Number(value);
      return (
        <label>
          {fieldSpec.label}
          <DraftNumberInput
            value={Number.isFinite(numeric) ? numeric : 0}
            placeholder={fieldSpec.placeholder}
            onFocus={pushHistory}
            onCommit={(next) =>
              update({ [fieldSpec.key]: next } as PanelWidgetPatch)
            }
          />
        </label>
      );
    }
    if (fieldSpec.kind === "list") {
      return (
        <label>
          {fieldSpec.label}
          <SuggestionInput
            value={String(value)}
            options={
              fieldSpec.listId === "page-sysvars"
                ? sysvarOptions
                : fieldSpec.listId === "page-actions"
                  ? actionNames
                  : fieldSpec.listId === "page-write"
                    ? (view.writeTargets ?? []).map((target) => target.key)
                    : sourceOptions
            }
            placeholder={fieldSpec.placeholder}
            maxLength={fieldSpec.maxLength}
            onFocus={pushHistory}
            onCommit={(next) =>
              update({ [fieldSpec.key]: next } as PanelWidgetPatch)
            }
          />
        </label>
      );
    }
    return (
      <label>
        {fieldSpec.label}
        <input
          value={String(value)}
          maxLength={fieldSpec.maxLength}
          placeholder={fieldSpec.placeholder}
          onFocus={pushHistory}
          onInput={(event) =>
            update({
              [fieldSpec.key]: (event.target as HTMLInputElement).value,
            } as PanelWidgetPatch)
          }
        />
      </label>
    );
  };

  return (
    <section
      class={`panel page-panel ${fullscreen ? "page-fullscreen" : ""}`}
    >
      <div class={`panel-head ${editing ? "editing" : ""}`}>
        <div class="panel-title">Panel</div>
        <span class="meta">
          {page.width * PAGE_GRID}×{page.height * PAGE_GRID} · {PAGE_GRID}px grid
        </span>
        <div class="page-tabs">
          <div class="page-tab-list" role="tablist" aria-label="Panel pages">
            {book.pages.map((entry) =>
              renaming?.id === entry.id ? (
                <input
                  class="page-tab-rename"
                  autoFocus
                  value={renaming.name}
                  maxLength={40}
                  onInput={(event) =>
                    setRenaming({
                      id: entry.id,
                      name: (event.target as HTMLInputElement).value,
                    })
                  }
                  onBlur={commitRename}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") {
                      event.preventDefault();
                      commitRename();
                    } else if (event.key === "Escape") {
                      event.preventDefault();
                      event.stopPropagation();
                      renameCanceled.current = true;
                      setRenaming(null);
                    }
                  }}
                />
              ) : (
                <button
                  type="button"
                  role="tab"
                  aria-selected={entry.id === page.id}
                  class={`page-tab ${entry.id === page.id ? "on" : ""}`}
                  key={entry.id}
                  title={editing ? "Double-click to rename" : entry.name}
                  onClick={() => switchPage(entry.id)}
                  onDblClick={() => {
                    if (editing) {
                      renameCanceled.current = false;
                      setRenaming({ id: entry.id, name: entry.name });
                    }
                  }}
                  onContextMenu={(event) => {
                    if (!editing) return;
                    event.preventDefault();
                    if (entry.id !== page.id) switchPage(entry.id);
                    setMenu({
                      kind: "page",
                      id: entry.id,
                      x: event.clientX,
                      y: event.clientY,
                    });
                  }}
                >
                  {entry.name}
                </button>
              ),
            )}
          </div>
          {editing && (
            <div class="page-tab-actions">
              <button
                type="button"
                class="mini"
                title="New page"
                onClick={newPage}
              >
                +
              </button>
              <button
                type="button"
                class="mini"
                title="Duplicate this page"
                onClick={copyPage}
              >
                Duplicate
              </button>
              <button
                type="button"
                class="mini"
                title="Delete this page"
                disabled={book.pages.length <= 1}
                onClick={deletePage}
              >
                Delete
              </button>
              <FilePick
                label="Import"
                accept=".rxpanel,.json,application/json"
                onText={importPage}
                onError={(message) => onNotice(message)}
              />
              <button
                type="button"
                class="mini"
                title="Download this page as a .rxpanel file"
                onClick={exportPage}
              >
                Export
              </button>
            </div>
          )}
        </div>
        <div class="panel-tools">
          {editing && (
            <>
              <div
                class="page-add"
                onBlur={(event) => {
                  const next = event.relatedTarget as Node | null;
                  if (!next || !(event.currentTarget as HTMLElement).contains(next)) {
                    setAddOpen(false);
                  }
                }}
              >
                <button
                  ref={addTrigger}
                  type="button"
                  class="mini"
                  aria-haspopup="menu"
                  aria-expanded={addOpen}
                  title="Add a control"
                  onClick={() => setAddOpen((open) => !open)}
                >
                  + Add
                </button>
                {addOpen && (
                  <div
                    ref={addMenu}
                    class="page-add-menu"
                    role="menu"
                    onKeyDown={(event) => {
                      const items = [
                        ...(addMenu.current?.querySelectorAll("button") ?? []),
                      ];
                      const index = items.indexOf(
                        document.activeElement as HTMLButtonElement,
                      );
                      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
                        event.preventDefault();
                        const step = event.key === "ArrowDown" ? 1 : -1;
                        const next =
                          index < 0
                            ? 0
                            : (index + step + items.length) % items.length;
                        items[next]?.focus();
                      } else if (event.key === "Home" || event.key === "End") {
                        event.preventDefault();
                        items[event.key === "Home" ? 0 : items.length - 1]?.focus();
                      } else if (event.key === "Tab") {
                        setAddOpen(false);
                      }
                    }}
                  >
                    {PANEL_WIDGET_KINDS.map((entry) => (
                      <button
                        type="button"
                        role="menuitem"
                        key={entry.kind}
                        onClick={() => {
                          setAddOpen(false);
                          add(entry.kind);
                          addTrigger.current?.focus();
                        }}
                      >
                        {entry.label}
                      </button>
                    ))}
                  </div>
                )}
              </div>
              <button
                type="button"
                class="mini"
                disabled={history.past.length === 0}
                title="Undo (Ctrl+Z)"
                onClick={undo}
              >
                Undo
              </button>
              <button
                type="button"
                class="mini"
                disabled={history.future.length === 0}
                title="Redo (Ctrl+Shift+Z / Ctrl+Y)"
                onClick={redo}
              >
                Redo
              </button>
            </>
          )}
          <button
            type="button"
            class={`mini ${editing ? "on" : ""}`}
            onClick={() => setEditing((old) => !old)}
          >
            {editing ? "Done" : "Edit"}
          </button>
          <button
            type="button"
            class={`mini ${fullscreen ? "on" : ""}`}
            title={fullscreen ? "Exit web fullscreen" : "Fill the browser window"}
            onClick={() => setFullscreen((old) => !old)}
          >
            {fullscreen ? "Exit" : "Fullscreen"}
          </button>
        </div>
      </div>
      <div class="page-body">
        <div
          class="page-viewport"
          onPointerDown={() => editing && setSelected(null)}
        >
          <div
            class={`page-canvas ${editing ? "editing" : ""}`}
            style={{
              width: `${page.width * PAGE_GRID}px`,
              height: `${page.height * PAGE_GRID}px`,
            }}
            onPointerDown={(event) => {
              if (event.target === event.currentTarget && editing) {
                setSelected(null);
              }
            }}
            onContextMenu={(event) => {
              if (!editing) return;
              event.preventDefault();
              setSelected(null);
              setMenu({
                kind: "canvas",
                x: event.clientX,
                y: event.clientY,
              });
            }}
          >
            {page.widgets.length === 0 && (
              <div class="page-empty">
                <strong>No controls yet</strong>
                <span>Use Edit, then + Button, + Value or + Toggle.</span>
              </div>
            )}
            {page.widgets.map((widget) => (
              <div
                key={widget.id}
                class={`page-widget ${editing ? "editing" : ""} ${
                  selected === widget.id ? "selected" : ""
                }`}
                style={{
                  left: `${widget.x * PAGE_GRID}px`,
                  top: `${widget.y * PAGE_GRID}px`,
                  width: `${widget.w * PAGE_GRID}px`,
                  height: `${widget.h * PAGE_GRID}px`,
                }}
                onPointerDown={(event) => {
                  if ((event.target as HTMLElement).closest(".page-resize")) {
                    startDrag(event, widget.id, "resize");
                  } else {
                    startDrag(event, widget.id, "move");
                  }
                }}
                onContextMenu={(event) => {
                  if (!editing) return;
                  event.preventDefault();
                  event.stopPropagation();
                  setSelected(widget.id);
                  setMenu({
                    kind: "widget",
                    id: widget.id,
                    x: event.clientX,
                    y: event.clientY,
                  });
                }}
              >
                {control(widget)}
                {editing && selected === widget.id && (
                  <span
                    class="page-resize"
                    title="Resize on the 8px grid"
                  />
                )}
              </div>
            ))}
          </div>
        </div>
        {editing && (
          <aside class="page-inspector">
            {selectedWidget ? (
              <>
                <div class="page-inspector-title">
                  {panelWidgetSpec(selectedWidget.kind).label}
                </div>
            {panelWidgetSpec(selectedWidget.kind).fields.map((spec) =>
              inspectorField(selectedWidget, spec.key),
            )}
            <div class="page-inspector-meta">
              {selectedWidget.w}×{selectedWidget.h} cells · x{" "}
              {selectedWidget.x} · y {selectedWidget.y}
            </div>
            <div class="page-inspector-actions">
              <button
                type="button"
                class="mini"
                title="Duplicate (Ctrl+C / Ctrl+V)"
                onClick={duplicateSelected}
              >
                Duplicate
              </button>
              <button type="button" class="mini" onClick={removeSelected}>
                Delete
              </button>
            </div>
              </>
            ) : (
              <>
                <div class="page-inspector-title">Panel</div>
                <label>
                  Width (px)
                  <input
                    type="number"
                    step={PAGE_GRID}
                    min={PAGE_MIN_W * PAGE_GRID}
                    max={PAGE_MAX_W * PAGE_GRID}
                    value={sizeW}
                    onInput={(event) =>
                      setSizeW((event.target as HTMLInputElement).value)
                    }
                    onKeyDown={(event) => {
                      if (event.key === "Enter") {
                        event.preventDefault();
                        applySize();
                      } else if (event.key === "Escape") {
                        event.preventDefault();
                        event.stopPropagation();
                        resetSize();
                        (event.target as HTMLInputElement).blur();
                      }
                    }}
                  />
                </label>
                <label>
                  Height (px)
                  <input
                    type="number"
                    step={PAGE_GRID}
                    min={PAGE_MIN_H * PAGE_GRID}
                    max={PAGE_MAX_H * PAGE_GRID}
                    value={sizeH}
                    onInput={(event) =>
                      setSizeH((event.target as HTMLInputElement).value)
                    }
                    onKeyDown={(event) => {
                      if (event.key === "Enter") {
                        event.preventDefault();
                        applySize();
                      } else if (event.key === "Escape") {
                        event.preventDefault();
                        event.stopPropagation();
                        resetSize();
                        (event.target as HTMLInputElement).blur();
                      }
                    }}
                  />
                </label>
                <button type="button" class="mini" onClick={applySize}>
                  Apply size
                </button>
                <div class="page-inspector-meta">
                  {page.widgets.length} control
                  {page.widgets.length === 1 ? "" : "s"} · {page.width}×
                  {page.height} cells · {PAGE_GRID}px grid
                </div>
              </>
            )}
          </aside>
        )}
      </div>
      {menu && (
        <div
          ref={menuRef}
          class="trace-menu page-menu"
          role="menu"
          style={{
            left: `${menuLeft}px`,
            top: `${menuTop}px`,
            maxHeight: `calc(100vh - ${menuTop + 8}px)`,
          }}
          onPointerDown={(event) => event.stopPropagation()}
          onKeyDown={(event) => {
            const items = [
              ...(menuRef.current?.querySelectorAll<HTMLButtonElement>(
                "button:not(:disabled)",
              ) ?? []),
            ];
            const index = items.indexOf(
              document.activeElement as HTMLButtonElement,
            );
            if (event.key === "ArrowDown" || event.key === "ArrowUp") {
              event.preventDefault();
              const step = event.key === "ArrowDown" ? 1 : -1;
              items[
                index < 0
                  ? 0
                  : (index + step + items.length) % items.length
              ]?.focus();
            } else if (event.key === "Home" || event.key === "End") {
              event.preventDefault();
              items[event.key === "Home" ? 0 : items.length - 1]?.focus();
            } else if (event.key === "Tab") {
              setMenu(null);
            }
          }}
        >
          {menu.kind === "widget" && (
            <>
              <button
                type="button"
                role="menuitem"
                onClick={() => {
                  setMenu(null);
                  duplicateWidget(menu.id);
                }}
              >
                Duplicate
              </button>
              <button
                type="button"
                role="menuitem"
                onClick={() => {
                  setMenu(null);
                  copyWidget(menu.id);
                }}
              >
                Copy
              </button>
              {menuSignal && (
                <button
                  type="button"
                  role="menuitem"
                  onClick={() => {
                    setMenu(null);
                    copySignalKey(menu.id);
                  }}
                >
                  Copy signal key
                </button>
              )}
              <div class="page-menu-sep" role="separator" />
              <button
                type="button"
                role="menuitem"
                onClick={() => {
                  setMenu(null);
                  removeWidget(menu.id);
                }}
              >
                Delete
              </button>
            </>
          )}
          {menu.kind === "canvas" && (
            <>
              <button
                type="button"
                role="menuitem"
                disabled={!clipboard}
                onClick={() => {
                  setMenu(null);
                  pasteClipboard();
                }}
              >
                Paste
              </button>
              <div class="page-menu-sep" role="separator" />
              {PANEL_WIDGET_KINDS.map((entry) => (
                <button
                  type="button"
                  role="menuitem"
                  key={entry.kind}
                  onClick={() => {
                    setMenu(null);
                    add(entry.kind);
                  }}
                >
                  Add {entry.label}
                </button>
              ))}
              <div class="page-menu-sep" role="separator" />
              <button
                type="button"
                role="menuitem"
                disabled={history.past.length === 0}
                onClick={() => {
                  setMenu(null);
                  undo();
                }}
              >
                Undo
              </button>
              <button
                type="button"
                role="menuitem"
                disabled={history.future.length === 0}
                onClick={() => {
                  setMenu(null);
                  redo();
                }}
              >
                Redo
              </button>
            </>
          )}
          {menu.kind === "page" && (
            <>
              <button
                type="button"
                role="menuitem"
                onClick={() => {
                  const target = book.pages.find((entry) => entry.id === menu.id);
                  setMenu(null);
                  if (!target) return;
                  renameCanceled.current = false;
                  setRenaming({ id: target.id, name: target.name });
                }}
              >
                Rename
              </button>
              <button
                type="button"
                role="menuitem"
                onClick={() => {
                  setMenu(null);
                  copyPageById(menu.id);
                }}
              >
                Duplicate page
              </button>
              <button
                type="button"
                role="menuitem"
                onClick={() => {
                  setMenu(null);
                  exportPageById(menu.id);
                }}
              >
                Export page
              </button>
              <div class="page-menu-sep" role="separator" />
              <button
                type="button"
                role="menuitem"
                disabled={book.pages.length <= 1}
                onClick={() => {
                  setMenu(null);
                  deletePageById(menu.id);
                }}
              >
                Delete page
              </button>
            </>
          )}
        </div>
      )}
      {fullscreen && toast && (
        <div class="page-toast" role="status">
          {toast}
        </div>
      )}
    </section>
  );
}