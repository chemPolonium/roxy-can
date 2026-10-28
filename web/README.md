# roxy-can web UI

The UI for roxy-can. It follows the lightweight shape of roxy-arxml: a Rust
`axum` host, a Preact + TypeScript frontend built by Vite, no component
library. There is no second desktop UI.

## Run

From the repository root:

```bash
cd web
npm install
npm run build
cd ..
cargo run                                       # demo simulation
cargo run -- --web --project <path.rxproj>      # project, stopped
cargo run -- --web --replay <path.asc|path.blf> # load and start a replay
```

With neither flag the generator demo starts automatically. `--project` loads
the project but leaves it stopped; `--replay` loads the log and starts
playback. A project and a replay can be combined. `--profile <name>` overlays
`profiles/<name>.toml` from the project directory onto its roles and hardware
mapping, exactly like the headless CLI. `--port <n>` changes the
port (default 2010). A bare launch opens the browser and prints the serving URL with the
version; press Ctrl+C to stop. `--web` remains an explicit synonym for the
default and keeps scripted runs silent (`ROXY_WEB_NO_OPEN=1` does the same).
The headless CLI modes (`--replay`/`--project` without `--web`, `--convert`,
`--check-script`) keep their old meaning.

The toolbar's **Session…** dialog can switch sessions without restarting the
server: enter a host path, pick a recent project/log, or choose a file with
the browser's picker. Chosen files are uploaded to the host's per-user
application data folder (`%LOCALAPPDATA%\roxy-can\web-uploads` on Windows,
256 MiB cap) and the returned path is opened, so a saved project keeps its
picked DBCs. Opening a project or log stops
the current measurement and clears the trace; a log starts playing
immediately. A loaded replay also gets a second, slim bar with the playhead,
a speed ladder (0.5x/1x/2x/4x/Fast), a forward **Step** (10 ms–5 s), **Play
for** (1–60 s) and **Play to** an absolute log time; both timed plays resume
playback and pause at the target.
**Save** writes the open project; **Save as** writes a copy to a typed path,
and a bare file name plus **Save & download** hands that copy to the browser.
The dialog marks unsaved changes next to the current name. The
**Recording** section starts/stops an ASC or BLF capture with a typed file
name and an optional whitelist (`100, 3F4x, FR0:13`); relative names land in
`Record/` beside the project, and arming while stopped creates the file on
the next Start. Once stopped, **Download** fetches the last recording
through the browser. The live badge in the top bar turns red while recording.
Recording names still take host paths; opening projects, logs, DBCs and
ARXML descriptions can use the browser picker, and bare export/save names
download through the browser.

For hot reload, keep the server running and start Vite in a second terminal:

```bash
cd web
npm run dev                 # http://127.0.0.1:5173, proxies /api to :2010
```

## What this slice proves

- The bus core is real: `web.rs` runs `App::headless()` on a manual drive in
  its own thread, seeds a simulation from the bundled DBCs, and streams a
  small JSON snapshot. It is not mock data.
- A fixed shell with one main table and two resizable right-hand panes covers
  the common "watch Trace + Messages + a curve" flow without window
  management. Pane sizes persist in `localStorage`.
- Trace uses a hand-rolled fixed-row-height virtualizer (no table library):
  50,000 buffered rows render as roughly 30 DOM rows between two spacer
  rows. The server seeds the buffer once and then sends only new frames, so
  the 100 ms poll stays small as the table grows. A scrolled-away reader keeps
  the same frame in view when rows are prepended; the live edge follows new
  frames. Click a row to select it, move with ↑/↓, and copy with Ctrl+C.
  Right-click a row to filter its ID, set cursor A/B for a Δt readout (the
  row's left edge marks each cursor), or copy it. The search box also
  matches the address column (`0x100`, `slot 13`, EtherType). Click a column
  header to sort ascending, again for descending, and a third time to restore
  the live newest-first order.
- A project or log can be loaded from the Session dialog by host path or
  from recents. The header, mode chip and status bar carry the session name
  and replay playhead; switching sessions resets the trace and curve buffers.
  A loaded project starts clean, plain **Save** rewrites it in place, and
  **Save as** targets a new path — both use the current project format.
- **Buses…** lists each bus with its DBCs and bit rates, probes Kvaser/Vector
  adapters on demand, attaches/detaches a selected channel at the bus's
  configured rates, and toggles the Simulated/Real bus mode. Buses can be
  added, removed, renamed, re-rated and given a new DBC by host path; removing
  one remaps the web signal keys with the core's channel indices. Each DBC
  node appears as a chip with its generator-entry count; ticking it declares
  the node Simulated (its DBC traffic joins the generator), unticking takes it
  offline. **FlexRay** and **Ethernet** sections load a FIBEX/ARXML cluster or
  an AUTOSAR matrix (host path or browser picker) and list the resulting bus,
  cluster and frame count; loading or removing a description stops the
  measurement. **Import ARXML…** adds every CAN, FlexRay and Ethernet cluster
  in one file as its own bus. A loaded FlexRay bus can attach a Vector FlexRay
  port in receive-only mode and detach it again. The **Replay channel map**
  lists the CAN, FlexRay and Ethernet channels the current log carries, their frame
  counts, and maps each to a project bus, as-is, or No replay; choices apply
  immediately and are saved with the project. It uses the same core commands
  as the core node/Buses models.
- **Generator…** edits the real TxView and FlexRay Tx lists: per-entry On,
  cycle, FD, BRS and hex (CAN), add-from-DBC/remove, a per-bus “Simulate nodes”
  action, and FR slots added by number to a loaded cluster description. The
  Write tab shows the bus Write ring (command results, node errors, trigger
  events). **Sources** opens the CAN waveform editor per entry: add/remove a
  DBC signal and edit its shape (Ramp, Sine, Step, Random, Triangle, Counter),
  Lo/Hi, period, phase, Step sequence and Random redraw interval. A
  **Replay blocks** section injects a recorded ASC/BLF onto a chosen bus,
  optionally limited to one DBC node; blocks toggle and remove in place.
- **Triggers…** shows the live rule list with condition summary, action,
  fired count and last-fire time. Default rules can be added by kind,
  enabled/disabled, removed and re-armed. The editor covers every condition
  kind (CAN signal/ID/error/timeout, FR frame/signal, sysvar), all five
  actions including Send-to-generator-entry, and the pre/post/marker limits.
  Insert-marker actions draw amber dashed lines in the Signals chart.
- The **Data** tab mirrors the first Data window: per selected signal it
  shows the live value (or enum label), unit, raw wire value, min/avg/max and
  a bar scaled by the database's declared range (observed range as fallback).
  Rows can be hidden without unsubscribing, removed, or cleared; selection
  uses the same catalog as Signals and the Monitor, including FlexRay and
  Ethernet rows.
- The **State** tab shows logic-analyzer bands for the first State Tracker
  window. Values are quantized to six significant digits, sampled into one
  band per pixel column, and drawn on a canvas over a 5 s–30 min trailing
  window (persisted in the project). Few states get distinct high-contrast
  colors and in-band labels; an analog sweep collapses to one neutral band.
  **bands** opens the Value Definition editor: ascending cut points, a name
  per band and Default to return to quantized values.
- **Scripts…** (from the Buses dialog) manages simulation node scripts:
  create an unbound script or bind it to a DBC node, edit the C-like source
  in a plain textarea, enable/disable it, apply while measuring (the node
  recompiles and restarts), see its log and compile errors, insert a starter
  template, load a local `.rxcan` file into the draft, save the source to a
  host `.rxcan` path, or remove it. Nodes run through the same
  `AddNode`/`SetNodeSource` commands as the core node model. The editor
  colors comments, strings, numbers, keywords and calls, completes
  context-aware names, shows call signatures and inline compile errors,
  and supports find/replace. F9 toggles line breakpoints; a paused node
  shows locals, globals and its call stack. `on stop` plus
  `test("name", condition)` leaves a pass/fail list in the editor and a
  `[tests]` verdict in Write.
- **SysVars…** manages system variables through the real core: add/edit
  definitions (namespace, name, init, clamped min/max, unit, comment), edit
  live values in place (writes clamp), rename by deleting the old key, and
  delete. The core applies the same validation rules on the host;
  definitions are saved with the project. Values reset to Init when a
  measurement starts, and scripts reach them through `sys_get` / `sys_set`.
- The lower panel has **Statistics / Bus / Spec / Write**. Messages shares
  the main pane with Trace through the **Trace / Messages** switch, and both
  the switch and the selected view are saved in the web workspace.
  Messages and Statistics include CAN, FlexRay and Ethernet rows when the
  session carries them: FR addresses read `slot N` and use the loaded
  cluster description, Ethernet rows use EtherType and the matched flow name.
  A Messages row with decoded signals expands from its chevron into signal
  children: the name is indented in the Name column, the physical value sits
  in Cycle, and its unit or enum label sits in Data; only open rows are
  decoded, so the collapsed list stays cheap.
  Both tabs have a search box matching bus, name or address.
  The topbar **Standard / Panel** switch opens the **Panel** view: a fixed
  canvas with an always-visible 8px grid and a page tab bar. Each page keeps
  its own canvas size and controls; **Edit** adds pages (`+`), duplicates or
  deletes the active page and renames it by double-clicking the tab. Switching
  pages clears the selection, undo history and clipboard but stays in Edit.
  **Edit** adds controls from the **+ Add** menu (button, value, bar, gauge,
  lamp, toggle, setpoint), drags and resizes
  them with live grid snapping (per-kind minimum sizes), refuses overlaps, and
  changes the canvas width/height in 8px steps; shrinking is refused while any
  control would fall outside. The inspector is generated from each kind's field
  schema and stays visible for the whole Edit session: a selected control shows
  its properties, a blank click shows the **Panel** properties (canvas W×H and
  control count), so changing the selection never shifts the canvas. It edits a
  button command or **Script action** name with an optional argument
  (`on panel_action("…", value)` in every running node), a value source (signal
  key, `derived:Name` or `sysvar:ns::name`) with decimals, a bar/gauge range, a
  lamp source with its lit threshold and colour, a toggle's sysvar, or a
  **Setpoint**: choose **Target** = *System variable* (name, decimals, step;
  committed on Enter/blur and clamped into the variable's min/max) or *Signal*
  (a CAN, FlexRay or Ethernet catalog signal, only while measuring; clamped to
  the declared range, or unbounded when the description names none). A CAN
  write goes into a one-shot frame and is held by dropping that signal's
  waveform source; a FlexRay write encodes the occupant slot and injects one
  session frame; an Ethernet write encodes the frame's PDU payload and injects
  one session TX frame with the matrix VLAN, its EtherType (or `0x88B5`) and
  synthetic locally-administered MACs. Neither has a transmit port, so the
  tooltip marks both `session only`. Derived signals are refused with a
  reason. Both targets take an
  optional script action fired with the
  committed value. The signal target also reads the signal back: it subscribes
  it as a Read signal, shows the live bus value, and releases that subscription
  when the control is retargeted or deleted (user plot/read selections are
  never touched). Pages saved with the old Numeric/Setpoint kinds migrate to
  the matching target on load. Value, bar and gauge tiles
  can carry **colour
  bands**: ascending `≤` limits coloured Neutral/OK/Warning/Critical plus an
  optional `above` band; only the number or fill is tinted, and values outside
  every band keep the normal colour. Editing has
  Ctrl+C/Ctrl+V copy-paste, Ctrl+Z / Ctrl+Shift+Z undo-redo, Delete and a
  Duplicate action. Controls expose availability: disabled buttons and
  toggles carry the reason in their tooltip (not measuring, no running script
  handler, unknown sysvar), and unresolved sources show a placeholder.
  **Export** downloads the current page as a versioned `.rxpanel` file without
  widget ids; **Import** reads one in the browser and appends it as a new page,
  rejecting unknown types, out-of-bounds or overlapping controls instead of
  dropping them.
  The canvas scrolls when the viewport is smaller; it never scales or
  reflows. **Fullscreen**
  covers the app chrome but keeps the browser window, centers the fixed canvas,
  and exits with its button or Esc; the page is saved with the web workspace.
  Statistics shows aggregate count, min/avg/max/jitter and share; Jitter
  folds away below 1280 px and the row tooltip keeps the full values. Bus
  shows per-bus load, send distance, bursts and per-class frame rates with a
  one-minute load sparkline. Write filters by category and text. Spec reads
  the project's tolerance/grace, the
  periodic-message count and the latched Unknown id / DLC / Cycle / Dropped
  verdicts, with per-rule filters, Clear and CSV export.
- **Export…** sits in the Trace header and the lower panel: it writes the
  trace as ASC (or PCAP when the path ends in `.pcap`), and Messages,
  Statistics, Data, State, the Signals chart or the Write log as CSV/TXT.
  A bare file name is written to the host's web download folder and
  **Export & download** hands it to the browser; a path with a directory part
  writes there instead. Exports use the first observer window of that kind,
  which is All buses by default.
- Trace/Messages tables, bus and text filtering, the extra **Filters** dialog
  (direction, decoded-only, payload substring, time range, only/exclude hex
  ids), a canvas chart
  with total and per-bus frame rates plus dashed bus-load lines,
  Start/Stop/Pause/Clear trace and the same core commands as the rest of the
  app.
  Trace merges CAN, FlexRay (`slot N` addresses) and Ethernet rows from the
  loaded session; the status bar reports the browser's own UI frame rate.
- The Signals tab plots real DBC signals from the loaded project, FlexRay
  signals when a cluster description is loaded (slots appear as `Slot N`
  instead of hex ids), and Ethernet signals when a matrix is loaded (frames
  are addressed by name). **Pick signals…** opens a searchable
  bus → message → signal catalog fetched from the backend. The demo starts
  with eight auto-selected generator sources,
  while real projects start from their first Signals chart and save the
  selection back there, so lanes survive save/reload. The core samples at up
  to 200 Hz; each series keeps 12k points (one minute at that rate), and the
  canvas reduces them to one min/max span per pixel column. Wheel zoom and
  drag pan freeze the view; double-click returns to live. With eight full
  series and continuous wheel zoom, 1024x768 and 1200x800 both hold ~60 UI
  fps. Each row has a **Plot / Read** mode switch (click the active segment
  to clear; Ctrl-click the name toggles Read): Read keeps the signal sampled
  and listed in the cursor table without drawing a lane. The table is always
  present: without a cursor it shows **Live values** under a `Now` column;
  one cursor switches to **Cursor values**, and A/B adds Δ and interval
  stats. The read set is saved in the web workspace, and an empty plot still
  accepts a cursor when Read signals exist. Fullscreen moves this table into
  a left rail with names and values adjacent; the chart takes the remaining
  width.
- The **Monitor** tab watches selected CAN, FlexRay or Ethernet signals as
  large live values with an optional coloring rule (`>=`/`<=` a threshold), decimal places and a
  rule color; rows are project state, so the rule survives a save. Selection
  reuses the same searchable catalog, and Clear unsubscribes every row. The
  row layout stacks controls under the value at narrow widths.
- Shortcuts: F9 start/stop, Ctrl+S save, Ctrl+Shift+S Session, Ctrl+O
  Session, Ctrl+E export, `?` for the help/about dialog (ignored while
  typing). The **?** button at the right of the stats strip opens the same
  dialog.
- The layout has been checked at 1024x768, 1200x800 and 1440x900.

## Not here yet

Ethernet hardware attachment and generation (descriptions, replay rows,
read-only Trace/Messages/Statistics and the FR generator are supported).
The web trace buffer caps at 50,000 rows, matching the source ring; raising
it needs a different storage layout than per-row JSON strings. Snapshots are
polled at 10 Hz over a 120-sample window; SSE is the obvious next step.

## Performance testing

CANoe's Trace buffer is commonly around 100k events by default and can be
raised into the millions. The web spike caps at 50k, the same order of
magnitude and the core trace ring's limit.

The demo generates 9 messages at 20 Hz. To fill 50k rows quickly, shorten
the generator period through the environment:

```bash
ROXY_WEB_CYCLE_US=5000 ./target/debug/roxy-can.exe --web
```

At 5 ms the demo runs around 1,800 frames/s, so the 50k buffer fills in
under 30 seconds. The same period drives the eight signal curves at 200 Hz,
so each series reaches its 12k-point cap in about a minute. The page keeps
a UI frame-rate readout in the status bar.
The Vite build is embedded into the release binary, so the exe runs on its
own. Debug builds read `web/dist` from disk instead, so frontend edits need no
Rust rebuild.
