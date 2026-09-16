#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod aggregate;
mod app;
mod block;
mod bus;
mod can;
mod channel;
mod cli;
mod clipboard;
mod config;
mod core_loop;
mod dbc;
mod decode;
mod export;
mod generator;
mod hw;
mod load;
mod log;
mod node;
mod observe;
mod profile;
mod project;
mod recorder;
mod script;
mod sim;
mod source;
mod spec;
mod trace;
mod trigger;
mod ui;
mod workspace;

/// 主线阶段 2 验收：不开 UI 跑完整仿真 + 录制 + 导出。
#[cfg(test)]
#[path = "headless_tests.rs"]
mod headless_tests;

/// 无头 UI 冒烟测试床：不接渲染器逐帧跑真实绘制路径。
#[cfg(test)]
mod ui_tests;

use std::sync::Arc;
use std::time::Instant;

use dear_imgui_rs::{ConfigFlags, Context, StyleColor};
use dear_imgui_wgpu::{FramebufferExtent, WgpuInitInfo, WgpuRenderer};
use dear_imgui_winit::{HiDpiMode, WinitPlatform};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

/// Redraw cadence. The event loop sleeps between frames instead of spinning
/// a render at the display's refresh rate; 60 keeps motion smooth, while
/// text content throttles itself separately (see `App::text_fresh`).
const TARGET_FPS: u64 = 60;
const FRAME_DT: std::time::Duration = std::time::Duration::from_nanos(1_000_000_000 / TARGET_FPS);

/// Pending global shortcut: 1=start/stop, 2=record, 3=export, 4=open DBC,
/// 5=play/pause, 6=slower, 7=faster, 8=jump to the live edge.
pub static CMD: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

struct State {
    // Field order is drop order: the app (and its CTE text editors) must
    // die before the ImGui context they are bound to.
    app: app::App,
    platform: WinitPlatform,
    renderer: WgpuRenderer,
    context: Context,
    device: wgpu::Device,
    queue: wgpu::Queue,
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    surface_desc: wgpu::SurfaceConfiguration,
    last_frame: Instant,
    last_title: String,
    last_autosave: Instant,
    ctrl: bool,
    shift: bool,
    /// When the next redraw is due; the event loop sleeps until then.
    next_frame: Instant,
}

impl State {
    fn new(event_loop: &ActiveEventLoop) -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..wgpu::InstanceDescriptor::new_with_display_handle(Box::new(
                event_loop.owned_display_handle(),
            ))
        });

        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("roxy-can")
                        .with_inner_size(LogicalSize::new(1280.0, 800.0)),
                )
                .unwrap(),
        );
        window.set_ime_allowed(true);
        // Windows cascades default placement toward the bottom-right of the
        // monitor, where the taskbar and screen edge crowd it out. Bring the
        // window to the center of whatever monitor the OS dropped it on.
        if let Some(monitor) = window.current_monitor() {
            window.set_outer_position(centered_position(
                window.outer_size(),
                monitor.position(),
                monitor.size(),
            ));
        }
        let size = window.inner_size();
        let surface = instance.create_surface(window.clone()).unwrap();

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .unwrap();
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();

        let surface_desc = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: wgpu::TextureFormat::Bgra8UnormSrgb,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![wgpu::TextureFormat::Bgra8Unorm],
        };
        surface.configure(&device, &surface_desc);

        let mut context = Context::create();
        // Copy/paste everywhere (input text, trace Copy row) routes through
        // the context's clipboard backend; the OS one is on us.
        context.set_clipboard_backend(clipboard::Clipboard);
        // Layouts live inside .rxproj files; no standalone ini file.
        context.set_ini_filename(None::<String>).unwrap();
        let mut config_flags = context.io().config_flags();
        config_flags.insert(ConfigFlags::DOCKING_ENABLE);
        context.io_mut().set_config_flags(config_flags);
        // Windows only move from their title bar: dragging anywhere else
        // (panels, lists, the signal checkboxes) must never carry a window.
        context
            .io_mut()
            .set_config_windows_move_from_title_bar_only(true);
        context.style_mut().set_frame_padding([4.0, 1.0]);
        // Opaque windows. The stock 94% window/popup alpha ghosts the
        // content behind them through the editor's backgroundless child
        // windows; the tool reads as a docked instrument panel, not an
        // overlay.
        {
            let style = context.style_mut();
            let mut window_bg = style.color(StyleColor::WindowBg);
            window_bg[3] = 1.0;
            style.set_color(StyleColor::WindowBg, window_bg);
            let mut popup_bg = style.color(StyleColor::PopupBg);
            popup_bg[3] = 1.0;
            style.set_color(StyleColor::PopupBg, popup_bg);
        }
        // ImGui 1.92 rasterizes glyphs on demand at the platform-reported
        // DPI, so fonts load at their logical reference size -- no baked
        // glyph ranges, no global scale hack. The CJK fallback merges into
        // the same font (base glyphs come from Inconsolata); a .ttc loads
        // as its first face.
        const INCONSOLATA_FONT: &[u8] = include_bytes!("../fonts/Inconsolata-Regular.ttf");
        let mut sources = vec![
            // # Safety: embedded font bytes are a complete TTF.
            unsafe {
                dear_imgui_rs::FontSource::ttf_data_with_size(INCONSOLATA_FONT, 13.0)
                    .with_config(
                        dear_imgui_rs::FontConfig::new()
                            .pixel_snap_h(true)
                            .oversample_h(1),
                    )
            },
        ];
        for path in [
            "C:\\Windows\\Fonts\\msyh.ttc",
            "C:\\Windows\\Fonts\\msyh.ttf",
            "C:\\Windows\\Fonts\\simhei.ttf",
            "C:\\Windows\\Fonts\\simsun.ttc",
        ] {
            if let Ok(bytes) = std::fs::read(path) {
                let data: &'static [u8] = Box::leak(bytes.into_boxed_slice());
                // # Safety: font data outlives the atlas and is a complete font.
                sources.push(unsafe {
                    dear_imgui_rs::FontSource::ttf_data_with_size(data, 13.0)
                });
                break;
            }
        }
        context.font_atlas().add_font(&sources);

        let init_info = WgpuInitInfo::new(device.clone(), queue.clone(), surface_desc.format);
        let mut renderer = WgpuRenderer::new(init_info, &mut context).unwrap();
        renderer.set_gamma_mode(dear_imgui_wgpu::GammaMode::Auto);
        let mut platform = WinitPlatform::new(&mut context).unwrap();
        platform
            .attach_window(window.clone(), HiDpiMode::Default, &mut context)
            .unwrap();

        let mut app = app::App::new();
        app.startup_workspace();
        // The default layout for New Project is whatever imgui persisted
        // last; empty when there is no ini yet.
        app.default_layout =
            std::fs::read_to_string(config::state_path("roxy-can.ini")).unwrap_or_default();

        State {
            app,
            platform,
            renderer,
            context,
            device,
            queue,
            window,
            surface,
            surface_desc,
            last_frame: Instant::now(),
            last_title: String::new(),
            last_autosave: Instant::now(),
            ctrl: false,
            shift: false,
            next_frame: Instant::now(),
        }
    }

    fn frame(&mut self) {
        let now = Instant::now();
        self.context
            .io_mut()
            .set_delta_time((now - self.last_frame).as_secs_f32());
        self.last_frame = now;

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) => f,
            wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.surface_desc);
                return;
            }
            _ => return,
        };

        self.platform
            .prepare_frame(&mut self.context, &self.window)
            .unwrap();

        // Project layouts are applied between imgui frames; the captured
        // text is embedded when the project is saved.
        if let Some(l) = self.app.pending_layout.take() {
            self.context.load_ini_settings(&l);
        }
        self.app.layout_cache.clear();
        self.context.save_ini_settings(&mut self.app.layout_cache);
        if self.last_autosave.elapsed() >= std::time::Duration::from_secs(30) {
            self.app.write_autosave();
            self.last_autosave = Instant::now();
        }

        let title = format!("{} - roxy-can", self.app.display_name());
        if self.last_title != title {
            self.window.set_title(&title);
            self.last_title = title.clone();
        }

        // CTE text editors bind to the context, which the frame's `Ui`
        // borrows exclusively; create the ones the UI requested last frame
        // out here, before the borrow starts.
        if !self.app.pending_editors.is_empty() {
            let wanted: Vec<u64> = std::mem::take(&mut self.app.pending_editors);
            // Autocomplete vocabulary is snapshotted per editor before the
            // entry borrow starts.
            let vocab: std::collections::HashMap<u64, Vec<String>> = wanted
                .iter()
                .map(|&id| {
                    let ch = self
                        .app
                        .snap
                        .nodes
                        .iter()
                        .find(|n| n.id == id)
                        .map(|n| n.channel)
                        .unwrap_or(0);
                    (id, ui::script_editor::autocomplete_vocabulary(&self.app, ch))
                })
                .collect();
            for id in wanted {
                self.app.editors.entry(id).or_insert_with(|| {
                    let mut editor = dear_imgui_cte::TextEditor::create(&self.context);
                    ui::script_editor::configure_new_editor(&mut editor);
                    ui::script_editor::install_autocomplete(
                        &mut editor,
                        vocab.get(&id).cloned().unwrap_or_default(),
                    );
                    editor
                });
            }
        }

        let ui = self.context.frame();

        self.app.update();
        ui::render(&mut self.app, ui);

        self.platform.prepare_render(ui, &self.window).unwrap();

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.06,
                            g: 0.06,
                            b: 0.08,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let pending = self
                .context
                .try_render(self.renderer.renderer_consumer().unwrap())
                .unwrap();
            self.renderer
                .render(
                    pending,
                    &mut rpass,
                    FramebufferExtent::from_texture(&frame.texture),
                )
                .unwrap();
        }
        self.queue.submit(Some(encoder.finish()));
        frame.present();

        // Next redraw one frame budget after this one started; if the frame
        // overran the budget already, draw again immediately rather than
        // adding delay on top.
        self.next_frame = (self.last_frame + FRAME_DT).max(Instant::now());
    }

    /// Releases render-loop resources that must die before the context.
    /// Safe to call twice.
    fn shutdown(&mut self) {
        self.app.editors.clear();
        let _ = self.renderer.shutdown(&mut self.context);
        let _ = self.platform.shutdown(&mut self.context);
    }
}

#[derive(Default)]
struct Program {
    state: Option<State>,
}

impl ApplicationHandler for Program {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.state = Some(State::new(event_loop));
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
        let st = self.state.as_mut().unwrap();
        // Platform first: it consumes keyboard/mouse/IME into ImGui input.
        st.platform
            .handle_window_event(&mut st.context, &st.window, &event)
            .unwrap();
        match &event {
            WindowEvent::CloseRequested => {
                st.app.request_quit();
                if st.app.quit {
                    el.exit();
                }
            }
            WindowEvent::DroppedFile(path) => st.app.open_dropped(path),
            WindowEvent::Resized(size) => {
                st.surface_desc.width = size.width.max(1);
                st.surface_desc.height = size.height.max(1);
                st.surface.configure(&st.device, &st.surface_desc);
            }
            WindowEvent::RedrawRequested => {
                st.frame();
                if st.app.quit {
                    el.exit();
                }
            }
            WindowEvent::ModifiersChanged(m) => {
                st.ctrl = m.state().control_key();
                st.shift = m.state().shift_key();
            }
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed && !event.repeat =>
            {
                let code = match (&event.logical_key, st.ctrl) {
                    (Key::Named(NamedKey::F9), _) => 1,
                    (Key::Character(c), true) => {
                        match (c.to_ascii_lowercase().as_str(), st.shift) {
                            ("r", false) => 2,
                            ("e", false) => 3,
                            ("o", false) => 4,
                            ("s", false) => 9,
                            ("s", true) => 10,
                            ("n", false) => 11,
                            ("o", true) => 12,
                            _ => 0,
                        }
                    }
                    (Key::Named(NamedKey::Space), false) => 5,
                    (Key::Character(c), false) => match c.as_str() {
                        "-" => 6,
                        "+" | "=" => 7,
                        _ => 0,
                    },
                    (Key::Named(NamedKey::Home), _) => 8,
                    _ => 0,
                };
                if code != 0 {
                    CMD.store(code, std::sync::atomic::Ordering::Relaxed);
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if let Some(st) = &mut self.state {
            // Redraw on the frame cadence; between frames the event loop
            // sleeps on the deadline and still dispatches OS events (input
            // stays low-latency, only the drawing is throttled).
            if Instant::now() >= st.next_frame {
                st.window.request_redraw();
            }
            el.set_control_flow(ControlFlow::WaitUntil(st.next_frame));
        }
    }

    fn exiting(&mut self, _el: &ActiveEventLoop) {
        if let Some(st) = &mut self.state {
            if let Some(p) = st.app.project_path.clone() {
                st.app.save_project(Some(p));
            }
            st.app.write_meta();
            // Clean exit: the crash cache is no longer needed.
            let _ = std::fs::remove_file(config::state_path(config::AUTOSAVE_PATH));
            st.shutdown();
        }
    }
}

/// The top-left position (physical pixels) that centers a window of `win`
/// inside the monitor at `monitor_pos` spanning `monitor`, clamped to the
/// monitor's top-left corner so a window larger than the monitor still
/// starts fully reachable.
fn centered_position(
    win: winit::dpi::PhysicalSize<u32>,
    monitor_pos: winit::dpi::PhysicalPosition<i32>,
    monitor: winit::dpi::PhysicalSize<u32>,
) -> winit::dpi::PhysicalPosition<i32> {
    let slack_w = (monitor.width as i32 - win.width as i32).max(0) / 2;
    let slack_h = (monitor.height as i32 - win.height as i32).max(0) / 2;
    winit::dpi::PhysicalPosition::new(monitor_pos.x + slack_w, monitor_pos.y + slack_h)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match cli::parse_args(&args) {
        Ok(cli::Cli::Gui) => {}
        Ok(cli::Cli::Help(text)) => {
            cli::attach_parent_console();
            println!("{text}");
            return;
        }
        Ok(cli::Cli::Run(opts)) => {
            cli::attach_parent_console();
            match cli::run(&opts) {
                Ok(report) => println!("{report}"),
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
            // Printed the report; do not fall through into the window.
            return;
        }
        Ok(cli::Cli::CheckScripts(check)) => {
            cli::attach_parent_console();
            match cli::check_scripts(&check) {
                Ok(report) => println!("{report}"),
                Err(report) => {
                    eprintln!("{report}");
                    std::process::exit(2);
                }
            }
            return;
        }
        Ok(cli::Cli::Convert(input, output)) => {
            cli::attach_parent_console();
            match cli::convert_log(&input, &output) {
                Ok(report) => println!("{report}"),
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
            return;
        }
        Ok(cli::Cli::KvaserProbe) => {
            cli::attach_parent_console();
            match cli::kvaser_probe() {
                Ok(report) => println!("{report}"),
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
            return;
        }
        Ok(cli::Cli::VectorProbe) => {
            cli::attach_parent_console();
            match cli::vector_probe() {
                Ok(report) => println!("{report}"),
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
            return;
        }
        Err(msg) => {
            cli::attach_parent_console();
            eprintln!("{msg}\n\n{}", cli::usage());
            std::process::exit(2);
        }
    }
    let event_loop = EventLoop::new().unwrap();
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run_app(&mut Program::default()).unwrap();
}

#[cfg(test)]
mod window_tests {
    use super::centered_position;
    use winit::dpi::{PhysicalPosition, PhysicalSize};

    #[test]
    fn a_window_lands_in_the_middle_of_its_monitor() {
        let pos = centered_position(
            PhysicalSize::new(1280, 800),
            PhysicalPosition::new(0, 0),
            PhysicalSize::new(1920, 1080),
        );
        assert_eq!(pos, PhysicalPosition::new(320, 140));
    }

    #[test]
    fn the_monitor_origin_counts() {
        // A monitor left of / above the primary keeps the window on itself.
        let pos = centered_position(
            PhysicalSize::new(1280, 800),
            PhysicalPosition::new(-2560, -1440),
            PhysicalSize::new(2560, 1440),
        );
        assert_eq!(pos, PhysicalPosition::new(-1920, -1120));
    }

    #[test]
    fn a_window_bigger_than_the_monitor_parks_at_its_corner() {
        let pos = centered_position(
            PhysicalSize::new(4000, 2000),
            PhysicalPosition::new(0, 0),
            PhysicalSize::new(1920, 1080),
        );
        assert_eq!(pos, PhysicalPosition::new(0, 0));
    }
}
