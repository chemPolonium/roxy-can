# roxy-can web spike

Experimental web frontend for roxy-can, on the `web-spike` branch. It follows
the lightweight shape of roxy-arxml: a Rust `axum` host, a Preact + TypeScript
frontend built by Vite, no component library.

**This is not the supported UI.** The Dear ImGui desktop app remains the
product. The spike exists to measure the alternative before deciding.

## Run

From the repository root (the demo loads `assets/sample.dbc` and
`assets/motbus.dbc` by relative path):

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
server: enter a host path or pick a recent project/log. Opening a project or
log stops the current measurement and clears the trace; a log starts playing
immediately. Paths are resolved on the host (the page cannot open a native
file dialog).

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
- **Buses…** lists each bus with its DBCs and bit rates, probes Kvaser/Vector
  adapters on demand, attaches/detaches a selected channel at the bus's
  configured rates, and toggles the Simulated/Real bus mode. Buses can be
  added, removed, renamed, re-rated and given a new DBC by host path; removing
  one remaps the web signal keys with the core's channel indices. It uses the
  same core commands as the desktop Buses window.
- **Generator…** edits the real TxView list: per-entry On, cycle, FD, hex
  payload, add-from-DBC/remove, and a per-bus “Simulate nodes” action. The
  Write tab shows the bus Write ring (command results, node errors, trigger
  events). Signal-source editing remains desktop-only for now.
- **Triggers…** shows the live rule list with condition summary, action,
  fired count and last-fire time. Default rules can be added by kind,
  enabled/disabled, removed and re-armed. Condition/action editing and the
  pre/post-trigger limits are the next slice.
- Trace/Messages tables, a canvas chart with total and per-bus frame rates
  plus dashed bus-load lines, bus filtering, text filtering, and
  Start/Stop/Pause/Clear trace are wired to the same core commands as the
  desktop UI. The status bar reports the browser's own UI frame rate.
- The Signals tab plots real DBC signals from the loaded project. **Pick
  signals…** opens a searchable bus → message → signal catalog fetched from
  the backend; the selection is session-only. The demo starts with eight
  auto-selected generator sources, while real projects start empty. The
  core samples at up to 200 Hz; each series keeps 12k points (one minute at
  that rate), and the canvas reduces them to one min/max span per pixel
  column. Wheel zoom and drag pan freeze the view; double-click returns to
  live. With eight full series and continuous wheel zoom, 1024x768 and
  1200x800 both hold ~60 UI fps.
- The layout has been checked at 1024x768, 1200x800 and 1440x900.

## Not here yet

Saving projects, a native browser file picker (opening and DBC loading use
host paths and recents; the CLI flags also work), FlexRay/Ethernet
attachment, generator signal sources, trigger condition/action editing,
pre/post-trigger limits, system variables, the monitor, node scripts, and
docking/desktops. Hardware is CAN attach/detach only for now; signal
selection is CAN-only and session-only.
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
