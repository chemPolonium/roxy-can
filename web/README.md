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
cargo run -- --web          # http://127.0.0.1:2010
```

`--port <n>` changes the port. For hot reload, keep the server running and
start Vite in a second terminal:

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
- Trace/Messages tables, a canvas chart with total and per-bus frame rates
  plus dashed bus-load lines, bus filtering, text filtering, and
  Start/Stop/Pause/Clear trace are wired to the same core commands as the
  desktop UI. The status bar reports the browser's own UI frame rate.
- The layout has been checked at 1024x768, 1200x800 and 1440x900.

## Not here yet

Project open/save, replay, hardware channels, signal plots, node scripts,
docking/desktops, and virtualization for the full 50k trace ring. Snapshots
are polled at 10 Hz over a 120-sample window; SSE is the obvious next step
if the frontend is adopted.
The Vite build is served from disk (`web/dist`) rather than embedded, so the
trial avoids a Rust rebuild for every frontend change.
