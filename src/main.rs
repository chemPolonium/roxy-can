mod app;
mod can;
mod dbc;
mod decode;
mod log;
mod source;
mod ui;

use std::sync::Arc;
use std::time::Instant;

use imgui_wgpu::{Renderer, RendererConfig};
use imgui_winit_support::{HiDpiMode, WinitPlatform};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::{Event, Ime, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

static IME_REQ: std::sync::Mutex<(bool, f32, f32, f32)> =
    std::sync::Mutex::new((false, 0.0, 0.0, 0.0));

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
        let hidpi = window.scale_factor();
        let surface = instance.create_surface(window.clone()).unwrap();

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .unwrap();
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .unwrap();

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
        context.set_ini_filename(None);
        context.io_mut().config_flags |= imgui::ConfigFlags::DOCKING_ENABLE;
        context.io_mut().config_windows_move_from_title_bar_only = true;
        context.io_mut().set_platform_ime_data_fn = Some(ime_data_callback);
        let font_size = (13.0 * hidpi) as f32;
        context.io_mut().font_global_scale = (1.0 / hidpi) as f32;
        let mut sources = Vec::new();
        let mut mono_loaded = false;
        for path in [
            "C:\\Windows\\Fonts\\consola.ttf",
            "C:\\Windows\\Fonts\\cour.ttf",
        ] {
            if let Ok(bytes) = std::fs::read(path) {
                let data: &'static [u8] = Box::leak(bytes.into_boxed_slice());
                sources.push(imgui::FontSource::TtfData {
                    data,
                    size_pixels: font_size,
                    config: Some(imgui::FontConfig {
                        size_pixels: font_size,
                        ..Default::default()
                    }),
                });
                mono_loaded = true;
                break;
            }
        }
        if !mono_loaded {
            sources.push(imgui::FontSource::DefaultFontData {
                config: Some(imgui::FontConfig {
                    size_pixels: font_size,
                    ..Default::default()
                }),
            });
        }
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
                        size_pixels: font_size,
                        glyph_ranges: imgui::FontGlyphRanges::chinese_simplified_common(),
                        ..Default::default()
                    }),
                });
                break;
            }
        }
        context.fonts().add_font(&sources);

        let renderer = Renderer::new(
            &mut context,
            &device,
            &queue,
            RendererConfig {
                texture_format: surface_desc.format,
                ..Default::default()
            },
        );

        State {
            context,
            platform,
            renderer,
            device,
            queue,
            window,
            surface,
            surface_desc,
            app: app::App::new(),
            last_frame: Instant::now(),
            last_cursor: None,
            ime_pos: None,
        }
    }

    fn frame(&mut self) {
        let now = Instant::now();
        self.context.io_mut().update_delta_time(now - self.last_frame);
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
        let ui = self.context.frame();

        self.app.update();
        ui::render(&mut self.app, ui);

        let req = *IME_REQ.lock().unwrap();
        if self.ime_pos != Some(req) {
            self.ime_pos = Some(req);
            if req.0 {
                let _ = self.window.set_ime_cursor_area(
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
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(size) => {
                st.surface_desc.width = size.width.max(1);
                st.surface_desc.height = size.height.max(1);
                st.surface.configure(&st.device, &st.surface_desc);
            }
            WindowEvent::RedrawRequested => st.frame(),
            WindowEvent::Ime(Ime::Commit(text)) => {
                for ch in text.chars() {
                    st.context.io_mut().add_input_character(ch);
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

    fn about_to_wait(&mut self, _el: &ActiveEventLoop) {
        if let Some(st) = &mut self.state {
            st.window.request_redraw();
            st.platform.handle_event::<()>(
                st.context.io_mut(),
                &st.window,
                &Event::AboutToWait,
            );
        }
    }
}

fn main() {
    let event_loop = EventLoop::new().unwrap();
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run_app(&mut Program::default()).unwrap();
}
