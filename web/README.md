# roxy-can web spike

Experimental web frontend for roxy-can, merged into `main`. It follows
the lightweight shape of roxy-arxml: a Rust `axum` host, a Preact + TypeScript
frontend built by Vite, no component library.

**The Dear ImGui desktop app remains the supported UI until web parity.** The
spike exists to measure the alternative before deciding; features are being
ported panel by panel.

## Run

From the repository root:

```bash
cd web
npm install
npm run build
cd ..
cargo run -- --web                              # demo simulation
cargo run -- --web --project <path.rxproj>      # project, stopped
cargo run -- --web --replay <path.asc|path.blf> # load and start a replay
```

With neither flag the generator demo starts automatically. `--project` loads
the project but leaves it stopped; `--replay` loads the log and starts
playback. A project and a replay can be combined. `--port <n>` changes the
port (default 2010).

The toolbar's **Session…** dialog can switch sessions without restarting the
server: enter a host path, pick a recent project/log, or choose a file with
the browser's picker. Chosen files are uploaded to the host's OS temp folder
(256 MiB cap) and the returned path is opened. Opening a project or log stops
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
the next Start. The live badge in the top bar turns red while recording.
Saving keeps
the project's desktop dock layout and Graphics-window settings untouched: the
web plot asks the core for its sampling stride instead of editing desktop
config. Recording names still take host paths; opening projects, logs, DBCs
and ARXML descriptions can use the browser picker, and bare export/save
names download through the browser.

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
  the 100 ms poll stays small as the table grows.
- A project or log can be loaded from the Session dialog by host path or
  from recents. The header, mode chip and status bar carry the session name
  and replay playhead; switching sessions resets the trace and curve buffers.
  A loaded project starts clean, plain **Save** rewrites it in place, and
  **Save as** targets a new path — both reuse the desktop project format,
  including its layout and desktops.
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
  measurement. A loaded FlexRay bus can attach a Vector FlexRay port in
  receive-only mode and detach it again. It uses the same core commands as
  the desktop Buses/Network windows.
- **Generator…** edits the real TxView and FlexRay Tx lists: per-entry On,
  cycle, FD and hex (CAN), add-from-DBC/remove, a per-bus “Simulate nodes”
  action, and FR slots added by number to a loaded cluster description. The
  Write tab shows the bus Write ring (command results, node errors, trigger
  events). **Sources** opens the CAN waveform editor per entry: add/remove a
  DBC signal and edit its shape (Ramp, Sine, Step, Random, Triangle, Counter),
  Lo/Hi, period, phase, Step sequence and Random redraw interval.
- **Triggers…** shows the live rule list with condition summary, action,
  fired count and last-fire time. Default rules can be added by kind,
  enabled/disabled, removed and re-armed. The editor covers every condition
  kind (CAN signal/ID/error/timeout, FR frame/signal, sysvar), all five
  actions including Send-to-generator-entry, and the pre/post/marker limits.
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
  template, save the source to a host `.rxcan` path, or remove it. Nodes run
  through the same `AddNode`/`SetNodeSource` commands as the desktop editor;
  syntax highlighting and file loading are desktop-only for now.
- **SysVars…** manages system variables through the real core: add/edit
  definitions (namespace, name, init, clamped min/max, unit, comment), edit
  live values in place (writes clamp), rename by deleting the old key, and
  delete. The same validation rules as the desktop editor run on the host;
  definitions are saved with the project. Values reset to Init when a
  measurement starts, and scripts reach them through `sys_get` / `sys_set`.
- The lower panel has **Messages / Statistics / Bus / Spec / Write**.
  Messages and Statistics include CAN, FlexRay and Ethernet rows when the
  session carries them: FR addresses read `slot N` and use the loaded
  cluster description, Ethernet rows use EtherType and the matched flow name.
  Statistics shows aggregate count, min/avg/max/jitter and share; Jitter
  folds away below 1280 px and the row tooltip keeps the full values. Bus
  shows per-bus load, send distance, bursts and per-class frame rates with a
  one-minute load sparkline. Spec reads the project's tolerance/grace, the
  periodic-message count and the latched Unknown id / DLC / Cycle / Dropped
  verdicts, with per-rule filters, Clear and CSV export.
- **Export…** sits in the Trace header and the lower panel: it writes the
  trace as ASC (or PCAP when the path ends in `.pcap`), and Messages,
  Statistics, Data, State, the Signals chart or the Write log as CSV/TXT.
  A bare file name is written to the host's web download folder and
  **Export & download** hands it to the browser; a path with a directory part
  writes there instead. Exports use the first desktop window of that kind,
  which is All buses by default.
- Trace/Messages tables, bus and text filtering, the extra **Filters** dialog
  (direction, decoded-only, payload substring, time range, only/exclude hex
  ids), a canvas chart
  with total and per-bus frame rates plus dashed bus-load lines,
  Start/Stop/Pause/Clear trace and the same core commands as the desktop UI.
  Trace merges CAN, FlexRay (`slot N` addresses) and Ethernet rows from the
  loaded session; the status bar reports the browser's own UI frame rate.
- The Signals tab plots real DBC signals from the loaded project, FlexRay
  signals when a cluster description is loaded (slots appear as `Slot N`
  instead of hex ids), and Ethernet signals when a matrix is loaded (frames
  are addressed by name). **Pick signals…** opens a searchable
  bus → message → signal catalog fetched from the backend. The demo starts
  with eight auto-selected generator sources,
  while real projects start from their first Graphics window and save the
  selection back there, so lanes survive save/reload. The core samples at up
  to 200 Hz; each series keeps 12k points (one minute at that rate), and the
  canvas reduces them to one min/max span per pixel column. Wheel zoom and
  drag pan freeze the view; double-click returns to live. With eight full
  series and continuous wheel zoom, 1024x768 and 1200x800 both hold ~60 UI
  fps.
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
read-only Trace/Messages/Statistics and the FR generator are supported),
docking/desktops, and syntax highlighting or `.rxcan` loading in the web
script editor.
The web trace buffer caps at 50,000 rows, matching the source ring; raising
it needs a different storage layout than per-row JSON strings. Scroll
anchoring while new rows arrive is left to the browser. Snapshots are polled
at 10 Hz over a 120-sample window; SSE is the obvious next step.

## Performance testing

CANoe's Trace buffer is commonly around 100k events by default and can be
raised into the millions. The web spike caps at 50k, the same order of
magnitude and the desktop source ring's limit.

The demo generates 9 messages at 20 Hz. To fill 50k rows quickly, shorten
the generator period through the environment:

```bash
ROXY_WEB_CYCLE_US=5000 ./target/debug/roxy-can.exe --web
```

At 5 ms the demo runs around 1,800 frames/s, so the 50k buffer fills in
under 30 seconds. The same period drives the eight signal curves at 200 Hz,
so each series reaches its 12k-point cap in about a minute. The page keeps
a UI frame-rate readout in the status bar.
The Vite build is served from disk (`web/dist`) rather than embedded, so the
trial avoids a Rust rebuild for every frontend change.
