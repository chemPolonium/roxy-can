import type { TraceColumnPrefs } from "../traceLayout";
import type { FilterRule, FilterGroup } from "../filterTree";
import type { Palette } from "../theme";
import type { Message, DataRow, SeriesPoint, StatsRow, WriteLine } from "../types";
import type { PanelPage } from "../panelPage";
import type { MutableRef, StateUpdater } from "preact/hooks";
import type { ComponentChildren, JSX } from "preact";
import type {
  Catalog,
  CatalogChannel,
  CatalogMessage,
  CatalogSignal,
  Command,
  Frame,
  SignalSeries,
  State,
} from "../types";
import { triggerDownload } from "../download";
import {
  SIGNAL_FILTER_SPECS,
} from "../signalFilter";
import { filterActive } from "../filterTree";
import type { FilterNode } from "../filterTree";
import type { SavedFilter } from "../filterLibrary";
import { FilterBar } from "../components/FilterBar";
import {
  MESSAGE_FILTER_SPECS,
} from "../messageFilter";
import { STATS_FILTER_SPECS } from "../statsFilter";
import { Activity } from "../components/Activity";
import { ArmedClearButton } from "../components/ArmedClearButton";
import { BusStatsPanel } from "../components/BusStatsPanel";
import { CursorAnalysis } from "../components/CursorAnalysis";
import { ColumnsMenu } from "../components/ColumnsMenu";
import { DataPanel } from "../components/DataPanel";
import { DbcExplorer } from "../components/DbcExplorer";
import { MessageTable } from "../components/MessageTable";
import { MonitorPanel } from "../components/MonitorPanel";
import { PanelFilterInput } from "../components/PanelFilterInput";
import { RateChart } from "../components/RateChart";
import {
  SignalChart,
  cursorLabels,
  type ChartCursor,
  type ChartSignalAction,
  type SignalView,
} from "../components/SignalChart";
import { SpecPanel } from "../components/SpecPanel";
import { SomeIpPanel } from "../components/SomeIpPanel";
import type { ChartYMode } from "../chartScale";
import { StatePanel } from "../components/StatePanel";
import { StatsTable } from "../components/StatsTable";
import { TraceTable } from "../components/TraceTable";
import type { TraceJump } from "../components/TraceTable";
import {
  DockSlotSurface,
  SlotHeader,
  SlotMaximizeButton,
  SlotMenu,
} from "../components/DockSlot";
import { UiIcon } from "../components/icons";
import {
  assignPanel,
  moveSlot,
  panelsFor,
  type DockState,
  type PanelId,
} from "../dockLayout";
import { TRACE_FILTER_SPECS } from "../traceFilter";
import type { MonitorRow, StateRow } from "../types";
import {
  type Theme,
} from "../theme";
import {
  addPanelPage,
  addPanelPageFromFile,
  duplicatePanelPage,
  exportPanelFile,
  importPanelFile,
  panelFileName,
  parsePanelHex,
  removePanelPage,
  renamePanelPage,
  selectPanelPage,
  type PanelBook,
  type PanelButton,
  type PanelFrameSend,
} from "../panelPage";

import {
  clock,
  panelMeta,
  type DockDrag,
  type ReferenceRun,
  type ShellMode,
} from "./support";

/// Everything App owns that the extracted panel renderers read.
export interface PanelRenderContext {
  state: State | null;
  emptyWorkspace: JSX.Element;
  theme: "dark" | "light";
  setDock: (value: StateUpdater<DockState>) => void;
  maximizedSlot: number | null;
  setMaximizedSlot: (value: StateUpdater<number | null>) => void;
  dock: DockState;
  setTheme: (value: StateUpdater<Theme>) => void;
  bus: string;
  query: string;
  traceEpoch: number;
  visibleRows: Frame[];
  traceFilterKey: string;
  traceFilter: FilterRule | FilterGroup;
  traceColumns: TraceColumnPrefs;
  setTraceColumns: (value: StateUpdater<TraceColumnPrefs>) => void;
  traceJump: TraceJump | null;
  setTraceJump: (value: StateUpdater<TraceJump | null>) => void;
  traceRows: Frame[];
  explorerOpen: boolean;
  openExplorer: () => void;
  openExport: (kind: string) => void;
  canControl: boolean;
  run: (command: Command) => Promise<boolean>;
  setBus: (value: StateUpdater<string>) => void;
  setQuery: (value: StateUpdater<string>) => void;
  setTraceFilter: (value: StateUpdater<FilterNode>) => void;
  filterLibrary: SavedFilter[];
  setFilterLibrary: (value: StateUpdater<SavedFilter[]>) => void;
  traceAtTop: boolean;
  traceNewCount: number;
  traceTopRef: MutableRef<(() => void) | null>;
  placeChartCursor: (tUs: number) => void;
  setChartCursorAt: (slot: "A" | "B", tUs: number) => void;
  addFrameSignals: (frame: Frame, target: "chart" | "monitor") => Promise<void>;
  goToTimeUs: (tUs: number, explainBehind?: boolean) => void;
  triggerFromTrace: (frame: Frame) => void;
  setTraceAtTop: (value: StateUpdater<boolean>) => void;
  setTraceNewCount: (value: StateUpdater<number>) => void;
  catalog: Catalog | null;
  catalogLoading: boolean;
  catalogError: string;
  selectedKeys: Set<string>;
  monitorKeys: Set<string>;
  addExplorerChart: (channel: CatalogChannel, message: CatalogMessage, signal: CatalogSignal) => void;
  addExplorerMonitor: (channel: CatalogChannel, message: CatalogMessage, signal: CatalogSignal) => void;
  setExplorerOpen: (value: StateUpdater<boolean>) => void;
  mainMessages: Message[];
  messageQuery: string;
  setMessageQuery: (value: StateUpdater<string>) => void;
  messageFilter: FilterRule | FilterGroup;
  setMessageFilter: (value: StateUpdater<FilterNode>) => void;
  showToast: (message: string) => void;
  allMessagesExpanded: boolean;
  setAllMessagesExpanded: (value: StateUpdater<boolean>) => void;
  messageColumns: unknown;
  setMessageColumns: (value: unknown) => void;
  chartPalette: Palette;
  latest: SeriesPoint | undefined;
  chartCursors: ChartCursor[];
  signalsQuery: string;
  signalsFilter: FilterRule | FilterGroup;
  chartSeries: SignalSeries[];
  visibleChartSeries: SignalSeries[];
  setSignalsQuery: (value: StateUpdater<string>) => void;
  setSignalsFilter: (value: StateUpdater<FilterNode>) => void;
  readSeries: SignalSeries[];
  addChartCursorCenter: () => void;
  clearChartCursors: () => void;
  signalOverlay: boolean;
  setSignalOverlay: (value: StateUpdater<boolean>) => void;
  setReferenceOpen: (value: StateUpdater<boolean>) => void;
  reference: ReferenceRun | null;
  chartExportRef: MutableRef<(() => void) | null>;
  openPicker: () => void;
  moveSignal: (key: string, to: number) => void;
  signalView: SignalView;
  setSignalView: (value: StateUpdater<SignalView>) => void;
  addChartCursor: (t: number) => number;
  moveChartCursor: (id: number, t: number) => void;
  chartSignalAction: (key: string, action: ChartSignalAction) => void;
  triggerFromSignal: (key: string) => void;
  measuring: boolean;
  chartYModes: { [x: string]: ChartYMode; };
  setSignalYMode: (key: string, mode: ChartYMode) => void;
  signalRanges: { [x: string]: { min: number; max: number; }; };
  visibleReadSeries: SignalSeries[];
  readRange: { first: number; last: number; } | null;
  cursorSeries: SignalSeries[];
  downloadChartCsv: (fromS: number, toS: number) => void;
  exportRange: { fromUs: number; toUs: number; } | null;
  setExportKind: (value: StateUpdater<string>) => void;
  setExportOpen: (value: StateUpdater<boolean>) => void;
  monitorQuery: string;
  monitorFilter: FilterRule | FilterGroup;
  monitorRows: MonitorRow[];
  visibleMonitorRows: MonitorRow[];
  setMonitorQuery: (value: StateUpdater<string>) => void;
  setMonitorFilter: (value: StateUpdater<FilterNode>) => void;
  openMonitorPicker: () => void;
  setMonitorStyle: (value: StateUpdater<MonitorRow | null>) => void;
  removeDerived: (key?: string | undefined) => void;
  dataQuery: string;
  dataFilter: FilterRule | FilterGroup;
  dataRows: DataRow[];
  visibleDataRows: DataRow[];
  setDataQuery: (value: StateUpdater<string>) => void;
  setDataFilter: (value: StateUpdater<FilterNode>) => void;
  openDataPicker: () => void;
  stateQuery: string;
  stateFilter: FilterRule | FilterGroup;
  stateRows: StateRow[];
  visibleStateRows: StateRow[];
  setStateQuery: (value: StateUpdater<string>) => void;
  setStateFilter: (value: StateUpdater<FilterNode>) => void;
  openStatePicker: () => void;
  stateRowRule: (row: StateRow) => void;
  stateRowRemove: (row: StateRow) => void;
  lowerStats: StatsRow[];
  panelQuery: string;
  statsFilter: FilterRule | FilterGroup;
  setPanelQuery: (value: StateUpdater<string>) => void;
  setStatsFilter: (value: StateUpdater<FilterNode>) => void;
  statsColumns: unknown;
  setStatsColumns: (value: unknown) => void;
  writeRows: WriteLine[];
  writeKind: string;
  setWriteKind: (value: StateUpdater<string>) => void;
  writeQuery: string;
  setWriteQuery: (value: StateUpdater<string>) => void;
  revealPanel: (panel: PanelId, preferredSlot?: number) => void;
  setFocusedSlot: (value: StateUpdater<number>) => void;
  dropCatalogSignal: (event: DragEvent, target: PanelId) => void;
  setDockDrag: (value: StateUpdater<DockDrag>) => void;
  sideRef: { current: HTMLDivElement | null; };
  setShellMode: (value: StateUpdater<ShellMode>) => void;
  setPanelEditing: (value: StateUpdater<boolean>) => void;
  updatePanelBook: (book: PanelBook) => void;
  panelBook: PanelBook;
  activePage: PanelPage;
  setPageConfirmDelete: (value: StateUpdater<string | null>) => void;
  shellMode: "standard" | "panel";
  setPageMenuMode: (value: StateUpdater<"menu" | "move">) => void;
  setPageMenu: (value: StateUpdater<{ id: string; x: number; y: number; } | null>) => void;
  pageRename: { id: string; name: string; } | null;
  setPageRename: (value: StateUpdater<{ id: string; name: string; } | null>) => void;
  pageConfirmDelete: string | null;
}

/// Panel headers, dock shell and Panel-page actions, split out of app.tsx.
/// The context is built by App from its own local state and callbacks.
export function createPanelRenderers(ctx: PanelRenderContext) {
  const {
    state,
    emptyWorkspace,
    theme,
    setDock,
    maximizedSlot,
    setMaximizedSlot,
    dock,
    setTheme,
    bus,
    query,
    traceEpoch,
    visibleRows,
    traceFilterKey,
    traceFilter,
    traceColumns,
    setTraceColumns,
    traceJump,
    setTraceJump,
    traceRows,
    explorerOpen,
    openExplorer,
    openExport,
    canControl,
    run,
    setBus,
    setQuery,
    setTraceFilter,
    filterLibrary,
    setFilterLibrary,
    traceAtTop,
    traceNewCount,
    traceTopRef,
    placeChartCursor,
    setChartCursorAt,
    addFrameSignals,
    goToTimeUs,
    triggerFromTrace,
    setTraceAtTop,
    setTraceNewCount,
    catalog,
    catalogLoading,
    catalogError,
    selectedKeys,
    monitorKeys,
    addExplorerChart,
    addExplorerMonitor,
    setExplorerOpen,
    mainMessages,
    messageQuery,
    setMessageQuery,
    messageFilter,
    setMessageFilter,
    showToast,
    allMessagesExpanded,
    setAllMessagesExpanded,
    messageColumns,
    setMessageColumns,
    chartPalette,
    latest,
    chartCursors,
    signalsQuery,
    signalsFilter,
    chartSeries,
    visibleChartSeries,
    setSignalsQuery,
    setSignalsFilter,
    readSeries,
    addChartCursorCenter,
    clearChartCursors,
    signalOverlay,
    setSignalOverlay,
    setReferenceOpen,
    reference,
    chartExportRef,
    openPicker,
    moveSignal,
    signalView,
    setSignalView,
    addChartCursor,
    moveChartCursor,
    chartSignalAction,
    triggerFromSignal,
    measuring,
    chartYModes,
    setSignalYMode,
    signalRanges,
    visibleReadSeries,
    readRange,
    cursorSeries,
    downloadChartCsv,
    exportRange,
    setExportKind,
    setExportOpen,
    monitorQuery,
    monitorFilter,
    monitorRows,
    visibleMonitorRows,
    setMonitorQuery,
    setMonitorFilter,
    openMonitorPicker,
    setMonitorStyle,
    removeDerived,
    dataQuery,
    dataFilter,
    dataRows,
    visibleDataRows,
    setDataQuery,
    setDataFilter,
    openDataPicker,
    stateQuery,
    stateFilter,
    stateRows,
    visibleStateRows,
    setStateQuery,
    setStateFilter,
    openStatePicker,
    stateRowRule,
    stateRowRemove,
    lowerStats,
    panelQuery,
    statsFilter,
    setPanelQuery,
    setStatsFilter,
    statsColumns,
    setStatsColumns,
    writeRows,
    writeKind,
    setWriteKind,
    writeQuery,
    setWriteQuery,
    revealPanel,
    setFocusedSlot,
    dropCatalogSignal,
    setDockDrag,
    sideRef,
    setShellMode,
    setPanelEditing,
    updatePanelBook,
    panelBook,
    activePage,
    setPageConfirmDelete,
    shellMode,
    setPageMenuMode,
    setPageMenu,
    pageRename,
    setPageRename,
    pageConfirmDelete,
  } = ctx;

  const renderRate = () =>
    state && state.buses.length === 0 ? (
      emptyWorkspace
    ) : (
      <RateChart
        points={state?.series ?? []}
        channels={state?.channels ?? []}
        theme={theme}
      />
    );

  const slotPick = (slot: number, panel: PanelId) => {
    setDock((old) => assignPanel(old, slot, panel));
  };

  const slotMove = (slot: number, to: number) => {
    setDock((old) => moveSlot(old, slot, to));
    if (maximizedSlot === slot) setMaximizedSlot(to);
  };

  const toggleSlotMaximize = (slot: number) => {
    setMaximizedSlot((old) => (old === slot ? null : slot));
  };

  const slotMenu = (slot: number, panel: PanelId) => (
    <SlotMenu
      dock={dock}
      slot={slot}
      panel={panel}
      onPick={(next) => slotPick(slot, next)}
      onMove={(to) => slotMove(slot, to)}
    />
  );

  const slotMax = (slot: number) => (
    <SlotMaximizeButton
      maximized={maximizedSlot === slot}
      onClick={() => toggleSlotMaximize(slot)}
      theme={theme}
      onTheme={() => setTheme(theme === "dark" ? "light" : "dark")}
    />
  );

  const slotHeader = (
    slot: number,
    panel: PanelId,
    props: {
      leading?: ComponentChildren;
      meta?: ComponentChildren;
      actions?: ComponentChildren;
    },
  ) => (
    <SlotHeader
      dock={dock}
      slot={slot}
      panel={panel}
      onPick={(next) => slotPick(slot, next)}
      onMove={(to) => slotMove(slot, to)}
      onMaximize={() => toggleSlotMaximize(slot)}
      maximized={maximizedSlot === slot}
      theme={theme}
      onTheme={() => setTheme(theme === "dark" ? "light" : "dark")}
      {...props}
    />
  );

  const renderTracePanel = (slot: number) => (
    <>
      <TraceTable
        key={`${bus}:${traceEpoch}`}
        rows={visibleRows}
        filterKey={traceFilterKey}
        filtered={Boolean(
          query.trim() ||
            bus !== "all" ||
            filterActive(traceFilter),
        )}
        columns={traceColumns}
        onColumnsChange={setTraceColumns}
        jump={traceJump}
        onJumpDone={() => setTraceJump(null)}
        leading={slotMenu(slot, "trace")}
        meta={panelMeta("frame", visibleRows.length, traceRows.length)}
        actions={
          <>
            <button
              type="button"
              class={`mini icon-btn ${explorerOpen ? "on" : ""}`}
              title="Browse DBC signals and add them to the chart or Monitor"
              aria-label="DBC signals"
              onClick={openExplorer}
            >
              <UiIcon name="book" />
            </button>
            <button
              type="button"
              class="mini icon-btn"
              onClick={() => openExport("trace")}
              title="Export the trace to an ASC or PCAP host file"
              aria-label="Export trace"
            >
              <UiIcon name="download" />
            </button>
            <ArmedClearButton
              disabled={!canControl || traceRows.length === 0}
              title="Clear the visible trace; measurement keeps running"
              confirmTitle="Click again to clear the visible trace"
              label="Clear trace"
              confirmLabel="Confirm clear trace"
              onClear={() => void run({ type: "clear_trace" })}
            />
            <ColumnsMenu
              columns={traceColumns}
              onChange={setTraceColumns}
            />
            <select
              value={bus}
              onChange={(e) => {
                setBus((e.target as HTMLSelectElement).value);
              }}
            >
              <option value="all">All buses</option>
              {state?.buses.map((entry) => (
                <option key={`${entry.kind}:${entry.index}`} value={entry.name}>
                  {entry.name}
                </option>
              ))}
            </select>
            <FilterBar
              query={query}
              onQuery={setQuery}
              filter={traceFilter}
              onFilter={setTraceFilter}
              fields={TRACE_FILTER_SPECS}
              label="Filter trace"
              placeholder="Filter id, name, data…"
              scope="trace"
              savedFilters={filterLibrary}
              onSavedFilters={setFilterLibrary}
            />
            <button
              type="button"
              class="mini icon-btn"
              disabled={traceAtTop || traceRows.length === 0}
              title={
                traceNewCount > 0
                  ? `${traceNewCount} new frames · jump to the newest`
                  : "Jump to the newest frames at the top"
              }
              aria-label="Back to top"
              onClick={() => traceTopRef.current?.()}
            >
              <UiIcon name="arrow-up" />
              {traceNewCount > 0 && (
                <span class="mini-count">
                  {traceNewCount > 999 ? "999+" : traceNewCount}
                </span>
              )}
            </button>
          </>
        }
        trailing={slotMax(slot)}
        onQuery={setQuery}
        empty={state && state.buses.length === 0 ? emptyWorkspace : undefined}
        onChartCursor={placeChartCursor}
        onSetChartCursor={setChartCursorAt}
        onAddToChart={(frame) => void addFrameSignals(frame, "chart")}
        onWatch={(frame) => void addFrameSignals(frame, "monitor")}
        onShowTime={(tUs) => goToTimeUs(tUs, false)}
        onTrigger={triggerFromTrace}
        scrollToTopRef={traceTopRef}
        onAtTopChange={setTraceAtTop}
        onNewCountChange={setTraceNewCount}
      />
      {explorerOpen && (
        <DbcExplorer
          catalog={catalog}
          loading={catalogLoading}
          error={catalogError}
          charted={selectedKeys}
          watched={monitorKeys}
          onAddChart={addExplorerChart}
          onAddMonitor={addExplorerMonitor}
          onClose={() => setExplorerOpen(false)}
        />
      )}
    </>
  );

  const renderMessagesPanel = (slot: number) => (
    <>
      {slotHeader(slot, "messages", {
        meta: panelMeta("message", mainMessages.length),
        actions: (
          <>
            <select
              value={bus}
              title="Bus filter; shared with Trace"
              onChange={(event) =>
                setBus((event.target as HTMLSelectElement).value)
              }
            >
              <option value="all">All buses</option>
              {state?.buses.map((entry) => (
                <option key={`${entry.kind}:${entry.index}`} value={entry.name}>
                  {entry.name}
                </option>
              ))}
            </select>
            <FilterBar
              query={messageQuery}
              onQuery={setMessageQuery}
              filter={messageFilter}
              onFilter={setMessageFilter}
              fields={MESSAGE_FILTER_SPECS}
              label="Filter messages"
              placeholder="Filter messages…"
              scope="message"
              savedFilters={filterLibrary}
              onSavedFilters={setFilterLibrary}
            />
            <button
              type="button"
              class="mini icon-btn"
              disabled={mainMessages.length === 0}
              title="Copy the visible messages as tab-separated rows"
              aria-label="Copy messages"
              onClick={() => {
                const text = mainMessages
                  .map(
                    (message) =>
                      `${message.busName}\t${message.addr || `0x${message.id.toString(16).toUpperCase()}`}\t${message.name}\t${message.cycleMs.toFixed(1)}\t${message.data}`,
                  )
                  .join("\n");
                void navigator.clipboard
                  .writeText(text)
                  .then(() =>
                    showToast(`Copied ${mainMessages.length} messages`),
                  )
                  .catch(() => showToast("Clipboard is unavailable"));
              }}
            >
              <UiIcon name="clipboard" />
            </button>
            <ArmedClearButton
              className="message-clear"
              disabled={mainMessages.length === 0}
              title="Reset message counters; Trace data is kept"
              confirmTitle="Click again to reset message counters"
              label="Reset message counters"
              confirmLabel="Confirm reset message counters"
              onClear={() => void run({ type: "clear_messages" })}
            />
            <button
              type="button"
              class="mini icon-btn"
              disabled={mainMessages.length === 0}
              title={
                allMessagesExpanded
                  ? "Collapse all decoded signal rows"
                  : "Expand all decoded signal rows"
              }
              aria-label={allMessagesExpanded ? "Collapse all" : "Expand all"}
              onClick={() => {
                const next = !allMessagesExpanded;
                setAllMessagesExpanded(next);
                void run({ type: "set_all_messages_expanded", on: next });
              }}
            >
              <UiIcon
                name={allMessagesExpanded ? "chevrons-up" : "chevrons-down"}
              />
            </button>
            <button
              type="button"
              class="mini icon-btn"
              onClick={() => openExport("messages")}
              title="Export messages to a host CSV file"
              aria-label="Export messages"
            >
              <UiIcon name="download" />
            </button>
          </>
        ),
      })}
      <MessageTable
        key={state?.log ?? state?.projectPath ?? "untitled"}
        rows={mainMessages}
        filtered={messageQuery.trim() !== "" || filterActive(messageFilter)}
        columnWidths={messageColumns}
        onColumnWidthsChange={setMessageColumns}
        empty={state && state.buses.length === 0 ? emptyWorkspace : undefined}
        onExpand={(key, on) =>
          void run({ type: "set_message_expanded", key, on })
        }
      />
    </>
  );

  const renderRatePanel = (slot: number) => (
    <>
      {slotHeader(slot, "rate", {
        actions: (
          <strong class="readout">
            {state ? state.fps.toFixed(0) : "—"} f/s
          </strong>
        ),
      })}
      {(!state || state.buses.length > 0) && (
        <div class="chart-legend">
          <span class="legend-key">
            <i style={{ background: chartPalette.rateTotal }} />
            Total <b>{state ? state.fps.toFixed(0) : "—"}</b> f/s
          </span>
          {state?.channels.map((channel, index) => (
            <span class="legend-key" key={channel.name}>
              <i
                style={{
                  background:
                    chartPalette.buses[index % chartPalette.buses.length],
                }}
              />
              {channel.name}{" "}
              <b>{(latest?.busRates[index] ?? 0).toFixed(0)}</b> f/s
              <em>{(latest?.busLoads[index] ?? 0).toFixed(1)}%</em>
            </span>
          ))}
        </div>
      )}
      {renderRate()}
    </>
  );

  const renderSignalsPanel = (slot: number) => {
    const labels = cursorLabels(chartCursors);
    const filtered =
      signalsQuery.trim() !== "" || filterActive(signalsFilter);
    const cursorA =
      labels.indexOf("A") >= 0 ? chartCursors[labels.indexOf("A")] : undefined;
    const cursorB =
      labels.indexOf("B") >= 0 ? chartCursors[labels.indexOf("B")] : undefined;
    return (
      <>
        {slotHeader(slot, "signals", {
          meta: filtered
            ? panelMeta("lane", visibleChartSeries.length, chartSeries.length)
            : undefined,
          actions: (
            <>
              <FilterBar
                query={signalsQuery}
                onQuery={setSignalsQuery}
                filter={signalsFilter}
                onFilter={setSignalsFilter}
                fields={SIGNAL_FILTER_SPECS}
                label="Filter chart signals"
                placeholder="Filter signals…"
                scope="signal"
                savedFilters={filterLibrary}
                onSavedFilters={setFilterLibrary}
              />
              <button
                type="button"
                class="mini icon-btn"
                disabled={chartSeries.length === 0 && readSeries.length === 0}
                title="Add a time cursor at the middle of the visible data"
                aria-label="Add cursor"
                onClick={addChartCursorCenter}
              >
                <UiIcon name="cursor" />
              </button>
              {chartCursors.length > 0 && (
                <ArmedClearButton
                  title="Remove all time cursors"
                  confirmTitle="Click again to remove all time cursors"
                  label="Clear cursors"
                  confirmLabel="Confirm clear cursors"
                  onClear={clearChartCursors}
                />
              )}
              <button
                type="button"
                class={`mini icon-btn ${signalOverlay ? "on" : ""}`}
                title={
                  signalOverlay
                    ? "Back to one lane per signal"
                    : "Overlay all signals on one normalized plot"
                }
                aria-label="Overlay signals"
                onClick={() => setSignalOverlay((value) => !value)}
              >
                <UiIcon name="layers" />
              </button>
              {cursorA && cursorB && (
                <span
                  class="meta cursor-delta"
                  title="Time between cursors A and B"
                >
                  Δt {Math.abs(cursorB.t - cursorA.t).toFixed(3)}s
                </span>
              )}
              <button
                type="button"
                class="mini icon-btn"
                title="Overlay an aligned CSV from a previous run"
                aria-label="Reference CSV"
                onClick={() => setReferenceOpen(true)}
              >
                <UiIcon name="file" />
              </button>
              {reference && (
                <span
                  class="reference-chip"
                  title={`Reference ${reference.name}; its time + ${reference.offsetS.toFixed(3)} s is matched to the live time`}
                >
                  REF {reference.offsetS >= 0 ? "+" : ""}
                  {reference.offsetS.toFixed(3)}s
                </span>
              )}
              <button
                type="button"
                class="mini icon-btn"
                disabled={chartSeries.length === 0 && readSeries.length === 0}
                title="Save the current chart view as a PNG image"
                aria-label="Export chart image"
                onClick={() => chartExportRef.current?.()}
              >
                <UiIcon name="camera" />
              </button>
              <button
                type="button"
                class="mini icon-btn"
                title="Choose signals to plot or read at cursors"
                aria-label="Add signals"
                onClick={() => void openPicker()}
              >
                <UiIcon name="plus-signal" />
              </button>
            </>
          ),
        })}
        <div class="signal-stack">
          {state && state.buses.length === 0 ? (
            emptyWorkspace
          ) : (
            <SignalChart
              series={visibleChartSeries}
              markers={state?.markers ?? []}
              theme={theme}
              onReorder={filtered ? () => {} : moveSignal}
              view={signalView}
              onView={(update) => setSignalView(update)}
              cursors={chartCursors}
              onAddCursor={addChartCursor}
              onMoveCursor={moveChartCursor}
              onClearCursors={clearChartCursors}
              onSignalAction={chartSignalAction}
              onTriggerSignal={triggerFromSignal}
              fullscreen={maximizedSlot === slot}
              measuring={measuring}
              paused={state?.tracePaused ?? false}
              reference={reference}
              onOpenReference={() => setReferenceOpen(true)}
              overlay={signalOverlay}
              onToggleOverlay={() => setSignalOverlay((value) => !value)}
              yModes={chartYModes}
              onYMode={setSignalYMode}
              ranges={signalRanges}
              hasReadout={visibleReadSeries.length > 0}
              readRange={readRange}
              readCount={visibleReadSeries.length}
              filtered={filtered}
              showCursorTools={false}
              exportRef={chartExportRef}
            />
          )}
          {maximizedSlot === slot && (
            <CursorAnalysis
              series={cursorSeries}
              cursors={chartCursors}
              onDownloadRange={downloadChartCsv}
              onExportRange={
                exportRange
                  ? () => {
                      setExportKind("signals");
                      setExportOpen(true);
                    }
                  : undefined
              }
            />
          )}
        </div>
      </>
    );
  };
  const renderMonitorPanel = (slot: number) => (
    <>
      {slotHeader(slot, "monitor", {
        meta:
          monitorQuery.trim() || filterActive(monitorFilter)
            ? panelMeta("row", visibleMonitorRows.length, monitorRows.length)
            : undefined,
        actions: (
          <>
            <FilterBar
              query={monitorQuery}
              onQuery={setMonitorQuery}
              filter={monitorFilter}
              onFilter={setMonitorFilter}
              fields={SIGNAL_FILTER_SPECS}
              label="Filter monitor signals"
              placeholder="Filter signals…"
              scope="signal"
              savedFilters={filterLibrary}
              onSavedFilters={setFilterLibrary}
            />
            <button
              type="button"
              class="mini"
              title={
                (state?.monitorColumns ?? 1) >= 2
                  ? "Switch Monitor to one column"
                  : "Switch Monitor to two columns"
              }
              aria-label="Monitor columns"
              onClick={() =>
                void run({
                  type: "set_monitor_columns",
                  columns: (state?.monitorColumns ?? 1) >= 2 ? 1 : 2,
                })
              }
            >
              {(state?.monitorColumns ?? 1) >= 2 ? "2 col" : "1 col"}
            </button>
            <button
              type="button"
              class="mini icon-btn"
              title="Choose the signals to watch"
              aria-label="Add signals"
              onClick={() => void openMonitorPicker()}
            >
              <UiIcon name="plus-signal" />
            </button>
            <ArmedClearButton
              disabled={!(state?.monitor.length ?? 0)}
              title="Clear watched signals"
              confirmTitle="Click again to clear all watched signals"
              label="Clear watched signals"
              confirmLabel="Confirm clear watched signals"
              onClear={() => void run({ type: "clear_monitor" })}
            />
          </>
        ),
      })}
      <MonitorPanel
        rows={visibleMonitorRows}
        filtered={monitorQuery.trim() !== "" || filterActive(monitorFilter)}
        empty={state && state.buses.length === 0 ? emptyWorkspace : undefined}
        columns={state?.monitorColumns ?? 1}
        theme={theme}
        onMoveRow={(key, to) =>
          void run({ type: "move_monitor_row", key, to })
        }
        onConfigure={setMonitorStyle}
        onRemove={(row) =>
          row.kind === "derived"
            ? removeDerived(row.frame)
            : void run({
                type: "remove_monitor",
                kind: row.kind,
                ch: row.ch,
                id: row.id,
                ext: row.ext,
                frame: row.frame,
                name: row.name,
              })
        }
      />
    </>
  );

  const renderDataPanel = (slot: number) => (
    <>
      {slotHeader(slot, "data", {
        meta:
          dataQuery.trim() || filterActive(dataFilter)
            ? panelMeta("row", visibleDataRows.length, dataRows.length)
            : undefined,
        actions: (
          <>
            <FilterBar
              query={dataQuery}
              onQuery={setDataQuery}
              filter={dataFilter}
              onFilter={setDataFilter}
              fields={SIGNAL_FILTER_SPECS}
              label="Filter data signals"
              placeholder="Filter signals…"
              scope="signal"
              savedFilters={filterLibrary}
              onSavedFilters={setFilterLibrary}
            />
            <button
              type="button"
              class="mini icon-btn"
              title="Choose the signals to track"
              aria-label="Add signals"
              onClick={() => void openDataPicker()}
            >
              <UiIcon name="plus-signal" />
            </button>
            <ArmedClearButton
              disabled={!(state?.data.length ?? 0)}
              title="Clear tracked signals"
              confirmTitle="Click again to clear all tracked signals"
              label="Clear tracked signals"
              confirmLabel="Confirm clear tracked signals"
              onClear={() => void run({ type: "clear_data" })}
            />
          </>
        ),
      })}
      <DataPanel
        rows={visibleDataRows}
        filtered={dataQuery.trim() !== "" || filterActive(dataFilter)}
        empty={state && state.buses.length === 0 ? emptyWorkspace : undefined}
        onMoveRow={(key, to) => void run({ type: "move_data_row", key, to })}
        onVisible={(row, visible) =>
          row.kind === "derived"
            ? showToast("Derived rows are always visible")
            : void run({
                type: "set_data_visible",
                kind: row.kind,
                ch: row.ch,
                id: row.id,
                ext: row.ext,
                frame: row.frame,
                name: row.name,
                visible,
              })
        }
        onRemove={(row) =>
          row.kind === "derived"
            ? removeDerived(row.frame)
            : void run({
                type: "remove_data",
                kind: row.kind,
                ch: row.ch,
                id: row.id,
                ext: row.ext,
                frame: row.frame,
                name: row.name,
              })
        }
      />
    </>
  );

  const renderStatePanel = (slot: number) => (
    <>
      {slotHeader(slot, "state", {
        meta:
          stateQuery.trim() || filterActive(stateFilter)
            ? panelMeta("row", visibleStateRows.length, stateRows.length)
            : undefined,
        actions: (
          <>
            <FilterBar
              query={stateQuery}
              onQuery={setStateQuery}
              filter={stateFilter}
              onFilter={setStateFilter}
              fields={SIGNAL_FILTER_SPECS}
              label="Filter state signals"
              placeholder="Filter signals…"
              scope="signal"
              savedFilters={filterLibrary}
              onSavedFilters={setFilterLibrary}
            />
            <button
              type="button"
              class="mini icon-btn"
              title="Choose the signals to show as bands"
              aria-label="Add signals"
              onClick={() => void openStatePicker()}
            >
              <UiIcon name="plus-signal" />
            </button>
            <ArmedClearButton
              disabled={!(state?.stateRows.length ?? 0)}
              title="Clear state bands"
              confirmTitle="Click again to clear all state bands"
              label="Clear state bands"
              confirmLabel="Confirm clear state bands"
              onClear={() => void run({ type: "clear_state" })}
            />
          </>
        ),
      })}
      <StatePanel
        rows={visibleStateRows}
        filtered={stateQuery.trim() !== "" || filterActive(stateFilter)}
        empty={state && state.buses.length === 0 ? emptyWorkspace : undefined}
        windowS={state?.stateWindowS ?? 20}
        theme={theme}
        onWindow={(seconds) => void run({ type: "set_state_window", seconds })}
        onRule={stateRowRule}
        onRemove={stateRowRemove}
      />
    </>
  );

  const renderStatisticsPanel = (slot: number) => (
    <>
      {slotHeader(slot, "statistics", {
        meta: panelMeta(
          "row",
          lowerStats.length,
          panelQuery.trim() || filterActive(statsFilter)
            ? state?.stats.length ?? 0
            : undefined,
        ),
        actions: (
          <>
            <FilterBar
              query={panelQuery}
              onQuery={setPanelQuery}
              filter={statsFilter}
              onFilter={setStatsFilter}
              fields={STATS_FILTER_SPECS}
              label="Filter message statistics"
              placeholder="Filter stats…"
              scope="stats"
              savedFilters={filterLibrary}
              onSavedFilters={setFilterLibrary}
            />
            <button
              type="button"
              class="mini icon-btn lower-export"
              onClick={() => openExport("stats")}
              title="Export this panel to a host file"
              aria-label="Export statistics"
            >
              <UiIcon name="download" />
            </button>
          </>
        ),
      })}
      <div class="panel-list">
        <StatsTable
          rows={lowerStats}
          filtered={panelQuery.trim() !== "" || filterActive(statsFilter)}
          empty={state && state.buses.length === 0 ? emptyWorkspace : undefined}
          columnWidths={statsColumns}
          onColumnWidthsChange={setStatsColumns}
        />
      </div>
    </>
  );

  const renderBusPanel = (slot: number) => (
    <>
      {slotHeader(slot, "bus", {
        meta: panelMeta("bus", state?.busStats.length ?? 0, undefined, "buses"),
      })}
      <BusStatsPanel
        buses={state?.busStats ?? []}
        theme={theme}
        empty={state && state.buses.length === 0 ? emptyWorkspace : undefined}
      />
    </>
  );

  const renderSpecPanel = (slot: number) => (
    <>
      {slotHeader(slot, "spec", {
        meta: panelMeta("violation", state?.specRows.length ?? 0),
      })}
      <SpecPanel
        rows={state && state.buses.length > 0 ? state.specRows : []}
        tolerance={state?.specTolerance ?? 5}
        grace={state?.specGrace ?? 3}
        periodic={state?.specPeriodic ?? 0}
        onTolerance={(value) =>
          void run({ type: "set_spec_tolerance", value })
        }
        onGrace={(value) => void run({ type: "set_spec_grace", value })}
        onClear={() => void run({ type: "clear_spec" })}
        onExport={() => openExport("spec")}
        empty={state && state.buses.length === 0 ? emptyWorkspace : undefined}
      />
    </>
  );

  const renderSomeIpPanel = (slot: number) => (
    <>
      {slotHeader(slot, "someip", {
        meta: panelMeta("message", state?.someip?.length ?? 0),
      })}
      <SomeIpPanel
        rows={state?.someip ?? []}
        services={state?.someipServices ?? []}
        subscriptions={state?.someipSubscriptions ?? []}
        issues={state?.someipIssues ?? []}
        empty={state && state.buses.length === 0 ? emptyWorkspace : undefined}
      />
    </>
  );

  const renderWritePanel = (slot: number) => (
    <>
      {slotHeader(slot, "write", {
        meta: panelMeta("line", writeRows.length),
        actions: (
          <button
            type="button"
            class="mini icon-btn lower-export"
            onClick={() => openExport("write")}
            title="Export this panel to a host file"
            aria-label="Export log"
          >
            <UiIcon name="download" />
          </button>
        ),
      })}
      {state && state.buses.length === 0 ? (
        emptyWorkspace
      ) : (
        <div class="panel-list">
          <div class="panel-filter-row">
          <select
            value={writeKind}
            onChange={(event) =>
              setWriteKind((event.target as HTMLSelectElement).value)
            }
          >
            <option value="all">All lines</option>
            <option value="script">Script</option>
            <option value="info">Info</option>
            <option value="warn">Warn</option>
            <option value="error">Error</option>
          </select>
          <PanelFilterInput
            value={writeQuery}
            placeholder="Filter text…"
            label="Filter log text"
            onInput={setWriteQuery}
          />
        </div>
        <Activity
          rows={writeRows}
          clock={clock}
          filtered={writeKind !== "all" || writeQuery.trim() !== ""}
          onFind={(query) => {
            setQuery(query);
            revealPanel("trace");
          }}
        />
        </div>
      )}
    </>
  );

  const renderDockPanel = (panel: PanelId, slot: number) => {
    switch (panel) {
      case "trace":
        return renderTracePanel(slot);
      case "messages":
        return renderMessagesPanel(slot);
      case "rate":
        return renderRatePanel(slot);
      case "signals":
        return renderSignalsPanel(slot);
      case "monitor":
        return renderMonitorPanel(slot);
      case "data":
        return renderDataPanel(slot);
      case "state":
        return renderStatePanel(slot);
      case "statistics":
        return renderStatisticsPanel(slot);
      case "bus":
        return renderBusPanel(slot);
      case "spec":
        return renderSpecPanel(slot);
      case "someip":
        return renderSomeIpPanel(slot);
      case "write":
        return renderWritePanel(slot);
      default:
        return renderMessagesPanel(slot);
    }
  };

  const renderSlot = (slot: number, style?: JSX.CSSProperties) => {
    const panel = panelsFor(dock)[slot] ?? "messages";
    const acceptsDrop =
      panel === "signals" || panel === "monitor" || panel === "data";
    return (
      <DockSlotSurface
        panel={panel}
        slot={slot}
        maximized={maximizedSlot === slot}
        style={style}
        onFocus={() => setFocusedSlot(slot)}
        onDragOver={
          acceptsDrop
            ? (event) => {
                if (
                  event.dataTransfer?.types.includes(
                    "application/x-roxy-signal",
                  )
                ) {
                  event.preventDefault();
                }
              }
            : undefined
        }
        onDrop={
          acceptsDrop ? (event) => dropCatalogSignal(event, panel) : undefined
        }
      >
        {renderDockPanel(panel, slot)}
      </DockSlotSurface>
    );
  };

  const renderDock = () => {
    const ratio = dock.ratios[dock.layout];
    if (dock.layout === "focus") {
      return <div class="dock-body focus">{renderSlot(0)}</div>;
    }
    if (dock.layout === "split") {
      return (
        <div class="dock-body split">
          {renderSlot(0, { flex: `${ratio.x} 1 0` })}
          <div
            class="splitter vertical dock-splitter"
            title="Drag to resize the pages"
            onPointerDown={(event) => {
              event.preventDefault();
              setDockDrag("x");
            }}
          />
          {renderSlot(1, { flex: `${1 - ratio.x} 1 0` })}
        </div>
      );
    }
    if (dock.layout === "main-side") {
      return (
        <div class="dock-body main-side">
          {renderSlot(0, { flex: `${ratio.x} 1 0` })}
          <div
            class="splitter vertical dock-splitter"
            title="Drag to resize the main page and the side column"
            onPointerDown={(event) => {
              event.preventDefault();
              setDockDrag("x");
            }}
          />
          <div
            class="dock-side"
            ref={sideRef}
            style={{ flex: `${1 - ratio.x} 1 0` }}
          >
            {renderSlot(1, { flex: `${ratio.y} 1 0` })}
            <div
              class="splitter horizontal dock-splitter"
              title="Drag to resize the two side pages"
              onPointerDown={(event) => {
                event.preventDefault();
                setDockDrag("y");
              }}
            />
            {renderSlot(2, { flex: `${1 - ratio.y} 1 0` })}
          </div>
        </div>
      );
    }
    return (
      <div
        class="dock-body quad"
        style={{
          gridTemplateColumns: `${ratio.x}fr 3px ${1 - ratio.x}fr`,
          gridTemplateRows: `${ratio.y}fr 3px ${1 - ratio.y}fr`,
        }}
      >
        {renderSlot(0, { gridColumn: "1", gridRow: "1" })}
        {renderSlot(1, { gridColumn: "3", gridRow: "1" })}
        <div
          class="splitter vertical dock-splitter quad-v"
          title="Drag to resize the two columns"
          onPointerDown={(event) => {
            event.preventDefault();
            setDockDrag("x");
          }}
        />
        <div
          class="splitter horizontal dock-splitter quad-h"
          title="Drag to resize the two rows"
          onPointerDown={(event) => {
            event.preventDefault();
            setDockDrag("y");
          }}
        />
        {renderSlot(2, { gridColumn: "1", gridRow: "3" })}
        {renderSlot(3, { gridColumn: "3", gridRow: "3" })}
      </div>
    );
  };

  const showStandard = () => {
    setShellMode("standard");
    setPanelEditing(false);
    setMaximizedSlot(null);
  };

  const showPanelPage = (id: string) => {
    updatePanelBook(selectPanelPage(panelBook, id));
    setShellMode("panel");
    setMaximizedSlot(null);
    setExplorerOpen(false);
  };

  const newPanelPage = () => {
    const added = addPanelPage(panelBook);
    updatePanelBook(added.book);
    setShellMode("panel");
    setPanelEditing(true);
    setMaximizedSlot(null);
    setExplorerOpen(false);
  };

  const openPageMenuAt = (id: string, x: number, y: number) => {
    if (shellMode !== "panel" || activePage.id !== id) showPanelPage(id);
    setPageConfirmDelete(null);
    setPageMenuMode("menu");
    setPageMenu({
      id,
      x: Math.max(8, Math.min(x, window.innerWidth - 230)),
      y: Math.max(8, Math.min(y, window.innerHeight - 260)),
    });
  };

  const openPageMenu = (event: MouseEvent, id: string) => {
    event.preventDefault();
    openPageMenuAt(id, event.clientX, event.clientY);
  };

  const commitPageRename = () => {
    if (!pageRename) return;
    updatePanelBook(renamePanelPage(panelBook, pageRename.id, pageRename.name));
    setPageRename(null);
  };

  const duplicatePage = (id: string) => {
    const copy = duplicatePanelPage(panelBook, id);
    if (!copy) {
      showToast("Cannot duplicate this page");
      return;
    }
    updatePanelBook(copy.book);
    setShellMode("panel");
    setPanelEditing(true);
    setPageMenu(null);
  };

  const exportPage = (id: string) => {
    const page = panelBook.pages.find((entry) => entry.id === id);
    if (!page) return;
    const name = panelFileName(page.name);
    const blob = new Blob([exportPanelFile(page)], {
      type: "application/json",
    });
    const url = URL.createObjectURL(blob);
    triggerDownload(url, name);
    window.setTimeout(() => URL.revokeObjectURL(url), 1_000);
    setPageMenu(null);
    showToast(`Exported ${name}`);
  };

  const importPanelPage = async (file: File) => {
    let raw: unknown;
    try {
      raw = JSON.parse(await file.text());
    } catch {
      showToast("That Panel file is not valid JSON");
      return;
    }
    const result = importPanelFile(raw);
    if ("error" in result) {
      showToast(result.error);
      return;
    }
    const added = addPanelPageFromFile(panelBook, result.page);
    updatePanelBook(added.book);
    setShellMode("panel");
    setPanelEditing(true);
    showToast(`Imported ${added.page.name}`);
  };

  const deletePage = (id: string) => {
    const page = panelBook.pages.find((entry) => entry.id === id);
    if (!page) return;
    if (page.widgets.length > 0 && pageConfirmDelete !== id) {
      setPageConfirmDelete(id);
      return;
    }
    updatePanelBook(removePanelPage(panelBook, id));
    setPageMenu(null);
    setPageConfirmDelete(null);
  };

  // The physical mode ships chosen signal values; the core encodes them and
  // keeps the database's start value for everything else.
  const panelPhysical = (frame: PanelFrameSend) => {
    if (frame.mode !== "physical") return {};
    const signals: Record<string, number> = {};
    for (const entry of frame.values ?? []) signals[entry.name] = entry.value;
    return { physical: true, signals };
  };

  const runPanelAction = (widget: PanelButton) => {
    if (widget.action === "send") {
      const frame = widget.frame;
      const data = frame ? parsePanelHex(frame.data) : null;
      if (!frame || data === null) {
        showToast("This button has no valid message");
        return;
      }
      if (frame.protocol === "can") {
        void run({
          type: "panel_send_frame",
          channel: frame.channel,
          id: frame.id,
          extended: frame.extended,
          data,
          ...panelPhysical(frame),
        });
      } else if (frame.protocol === "fr") {
        void run({
          type: "panel_send_fr_frame",
          bus: frame.bus,
          slot: frame.slot,
          data,
          ...panelPhysical(frame),
        });
      } else if (frame.protocol === "someip") {
        void run({
          type: "panel_send_someip",
          bus: frame.bus,
          service: frame.service,
          method: frame.method,
          client: frame.client,
          session: frame.session,
          message_type: frame.messageType,
          data,
          transport: frame.transport ?? "udp",
          host: frame.host ?? "127.0.0.1",
          port: frame.port ?? 30490,
          values: frame.mode === "physical" && frame.values?.length
            ? Object.fromEntries(
                frame.values
                  .filter((entry) => entry.text === undefined)
                  .map((entry) => [entry.name, entry.value]),
              )
            : undefined,
          texts: frame.mode === "physical" && frame.values?.length
            ? Object.fromEntries(
                frame.values
                  .filter((entry) => entry.text !== undefined)
                  .map((entry) => [entry.name, entry.text ?? ""]),
              )
            : undefined,
        });
      } else {
        void run({
          type: "panel_send_eth_frame",
          bus: frame.bus,
          frame: frame.frame,
          data,
          ...panelPhysical(frame),
        });
      }
      return;
    }
    if (widget.action === "script") {
      const name = widget.script?.trim() ?? "";
      if (!name) {
        showToast("This button has no script action");
        return;
      }
      void run({ type: "run_panel_action", name, value: widget.argument });
      return;
    }
  };

  return {
    renderRate,
    slotMenu,
    slotHeader,
    renderTracePanel,
    renderMessagesPanel,
    renderRatePanel,
    renderSignalsPanel,
    renderMonitorPanel,
    renderDataPanel,
    renderStatePanel,
    renderStatisticsPanel,
    renderBusPanel,
    renderSpecPanel,
    renderSomeIpPanel,
    renderWritePanel,
    renderDockPanel,
    renderSlot,
    renderDock,
    showStandard,
    showPanelPage,
    newPanelPage,
    openPageMenuAt,
    openPageMenu,
    commitPageRename,
    duplicatePage,
    exportPage,
    importPanelPage,
    deletePage,
    runPanelAction,
  };
}
