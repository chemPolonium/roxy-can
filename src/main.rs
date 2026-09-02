#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod aggregate;
mod app;
mod can;
mod channel;
mod config;
mod dbc;
mod decode;
mod export;
mod generator;
mod load;
mod log;
mod observe;
mod project;
mod recorder;
mod sim;
mod source;
mod spec;
mod trigger;
mod ui;
mod workspace;

use std::sync::Arc;
use std::time::Instant;

use imgui_wgpu::{Renderer, RendererConfig};
use imgui_winit_support::{HiDpiMode, WinitPlatform};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::{ElementState, Event, Ime, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

static IME_REQ: std::sync::Mutex<(bool, f32, f32, f32)> =
    std::sync::Mutex::new((false, 0.0, 0.0, 0.0));

/// Redraw cadence. A bus tool idles most of the time; redrawing at 30 fps
/// keeps the UI responsive while the event loop sleeps between frames
/// instead of spinning a render at the display's refresh rate.
const TARGET_FPS: u64 = 30;
const FRAME_DT: std::time::Duration = std::time::Duration::from_nanos(1_000_000_000 / TARGET_FPS);

/// Pending global shortcut: 1=start/stop, 2=record, 3=export, 4=open DBC,
/// 5=play/pause, 6=slower, 7=faster, 8=jump to the live edge.
pub static CMD: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

unsafe extern "C" fn ime_data_callback(
    _viewport: *mut imgui::sys::ImGuiViewport,
    data: *mut imgui::sys::ImGuiPlatformImeData,
) {
    let d = unsafe { &*data };
    *IME_REQ.lock().unwrap() = (d.WantVisible, d.InputPos.x, d.InputPos.y, d.InputLineHeight);
}

struct State {
    context: imgui::Context,
    platform: WinitPlatform,
    renderer: Renderer,
    device: wgpu::Device,
    queue: wgpu::Queue,
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    surface_desc: wgpu::SurfaceConfiguration,
    app: app::App,
    last_frame: Instant,
    last_cursor: Option<imgui::MouseCursor>,
    ime_pos: Option<(bool, f32, f32, f32)>,
    ctrl: bool,
    shift: bool,
    last_title: String,
    last_autosave: Instant,
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
        let size = window.inner_size();
        let dpi = window.scale_factor();
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

        let mut context = imgui::Context::create();
        let mut platform = WinitPlatform::new(&mut context);
        platform.attach_window(context.io_mut(), &window, HiDpiMode::Default);
        // Layouts live inside .rxproj files; no standalone ini file.
        context.set_ini_filename(None);
        context.io_mut().config_flags |= imgui::ConfigFlags::DOCKING_ENABLE;
        context.io_mut().config_windows_move_from_title_bar_only = true;
        context.io_mut().set_platform_ime_data_fn = Some(ime_data_callback);
        let font_size = 13.0 * dpi as f32;
        let font_config = imgui::FontConfig {
            oversample_h: 1,
            pixel_snap_h: true,
            size_pixels: font_size,
            ..Default::default()
        };
        const INCONSOLATA_FONT: &[u8] = include_bytes!("../fonts/Inconsolata-Regular.ttf");
        let mut sources = vec![imgui::FontSource::TtfData {
            data: INCONSOLATA_FONT,
            size_pixels: font_size,
            config: Some(font_config.clone()),
        }];
        // Merge Chinese glyphs into the same font (base glyphs come from
        // Inconsolata). GlyphOffset nudges CJK glyphs down: their fonts have
        // tall ascents and otherwise render too high inside widgets.
        for path in [
            "C:\\Windows\\Fonts\\msyh.ttc",
            "C:\\Windows\\Fonts\\msyh.ttf",
            "C:\\Windows\\Fonts\\simhei.ttf",
            "C:\\Windows\\Fonts\\simsun.ttc",
        ] {
            if let Ok(bytes) = std::fs::read(path) {
                let data: &'static [u8] = Box::leak(bytes.into_boxed_slice());
                sources.push(imgui::FontSource::TtfData {
                    data,
                    size_pixels: font_size,
                    config: Some(imgui::FontConfig {
                        glyph_ranges: imgui::FontGlyphRanges::chinese_full(),
                        ..font_config.clone()
                    }),
                });
                break;
            }
        }
        context.fonts().add_font(&sources);
        context.io_mut().font_global_scale = (1.0 / dpi) as f32;

        let renderer = Renderer::new(
            &mut context,
            &device,
            &queue,
            RendererConfig {
                texture_format: surface_desc.format,
                ..Default::default()
            },
        );

        let mut app = app::App::new();
        app.startup_workspace();
        // The default layout for New Project is whatever imgui persisted
        // last; empty when there is no ini yet.
        app.default_layout = std::fs::read_to_string("roxy-can.ini").unwrap_or_default();

        State {
            context,
            platform,
            renderer,
            device,
            queue,
            window,
            surface,
            surface_desc,
            app,
            last_frame: Instant::now(),
            last_cursor: None,
            ime_pos: None,
            ctrl: false,
            shift: false,
            last_title: String::new(),
            last_autosave: Instant::now(),
            next_frame: Instant::now(),
        }
    }

    fn frame(&mut self) {
        let now = Instant::now();
        self.context
            .io_mut()
            .update_delta_time(now - self.last_frame);
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
            .prepare_frame(self.context.io_mut(), &self.window)
            .expect("prepare_frame failed");

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

        let ui = self.context.frame();

        self.app.update();
        ui::render(&mut self.app, ui);

        let req = *IME_REQ.lock().unwrap();
        if self.ime_pos != Some(req) {
            self.ime_pos = Some(req);
            if req.0 {
                self.window.set_ime_cursor_area(
                    LogicalPosition::new(req.1 as f64, req.2 as f64),
                    LogicalSize::new(20.0, req.3.max(16.0) as f64),
                );
            }
        }

        if self.last_cursor != ui.mouse_cursor() {
            self.last_cursor = ui.mouse_cursor();
            self.platform.prepare_render(ui, &self.window);
        }

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
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
        self.renderer
            .render(self.context.render(), &self.queue, &self.device, &mut rpass)
            .expect("imgui render failed");
        drop(rpass);
        self.queue.submit(Some(encoder.finish()));
        frame.present();

        // Next redraw one frame budget after this one started; if the frame
        // overran the budget already, draw again immediately rather than
        // adding delay on top.
        self.next_frame = (self.last_frame + FRAME_DT).max(Instant::now());
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

    fn window_event(&mut self, el: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
        let st = self.state.as_mut().unwrap();
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
            WindowEvent::Ime(Ime::Commit(text)) => {
                for ch in text.chars() {
                    st.context.io_mut().add_input_character(ch);
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
        st.platform.handle_event::<()>(
            st.context.io_mut(),
            &st.window,
            &Event::WindowEvent { window_id, event },
        );
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
            st.platform
                .handle_event::<()>(st.context.io_mut(), &st.window, &Event::AboutToWait);
        }
    }

    fn exiting(&mut self, _el: &ActiveEventLoop) {
        if let Some(st) = &mut self.state {
            if let Some(p) = st.app.project_path.clone() {
                st.app.save_project(Some(p));
            }
            st.app.write_meta();
            // Clean exit: the crash cache is no longer needed.
            let _ = std::fs::remove_file(config::AUTOSAVE_PATH);
        }
    }
}

fn main() {
    let event_loop = EventLoop::new().unwrap();
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run_app(&mut Program::default()).unwrap();
}
