use std::sync::Arc;

use anyhow::{Context, Result};
use egui_wgpu::{Renderer, RendererOptions, ScreenDescriptor};
use tokio::runtime::Handle;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};
use winit::window::{Fullscreen, Window, WindowId};

use crate::capture::Frame;

pub fn list_monitors() -> Result<()> {
    struct Probe;
    impl ApplicationHandler for Probe {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            for (i, m) in event_loop.available_monitors().enumerate() {
                let pos = m.position();
                let size = m.size();
                eprintln!(
                    "[monitor {i}] name={:?} pos=({},{}) size={}x{} scale={}",
                    m.name(),
                    pos.x,
                    pos.y,
                    size.width,
                    size.height,
                    m.scale_factor(),
                );
            }
            event_loop.exit();
        }
        fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
    }
    let event_loop = EventLoop::new().context("failed to create event loop")?;
    event_loop.run_app(&mut Probe).context("probe run failed")?;
    Ok(())
}

pub fn run(frame: Frame, rt: Handle) -> Result<()> {
    let event_loop = EventLoop::new().context("failed to create winit event loop")?;
    let mut app = OverlayApp::new(frame, rt);
    event_loop.run_app(&mut app).context("event loop error")?;
    Ok(())
}

#[derive(Clone, Copy)]
enum Action {
    Copy,
    Save,
    Close,
}

#[derive(Clone, Copy, PartialEq)]
enum SelMode {
    New,
    Move,
    Resize(usize),
}

struct TextDraft {
    pos: egui::Pos2,
    buf: String,
    color: egui::Color32,
    size: f32,
}

struct OverlayApp {
    frame: Arc<Frame>,
    rt: Handle,
    shared: Option<Shared>,
    windows: Vec<WindowState>,
    initialized: bool,
    selection: Option<egui::Rect>,
    drag_start: Option<egui::Pos2>,
    modifiers: winit::keyboard::ModifiersState,
    scene: crate::scene::Scene,
    tool: crate::scene::Tool,
    color: egui::Color32,
    stroke_width: f32,
    draft: Option<crate::scene::Shape>,
    draft_start: Option<egui::Pos2>,
    pending: Option<Action>,
    sel_mode: Option<SelMode>,
    sel_ref: egui::Rect,
    text_edit: Option<TextDraft>,
    move_shape: Option<(usize, crate::scene::Shape)>,
    text_sel: Option<(usize, usize, usize)>,
    blurred: Option<Arc<Vec<u8>>>,
}

impl OverlayApp {
    fn new(frame: Frame, rt: Handle) -> Self {
        let (fw, fh) = (frame.width as f32, frame.height as f32);
        let selection = crate::export::load_last_selection().and_then(|(x, y, w, h)| {
            let r = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h));
            (r.min.x >= 0.0 && r.min.y >= 0.0 && r.max.x <= fw && r.max.y <= fh).then_some(r)
        });
        Self {
            frame: Arc::new(frame),
            rt,
            shared: None,
            windows: Vec::new(),
            initialized: false,
            selection,
            drag_start: None,
            modifiers: winit::keyboard::ModifiersState::empty(),
            scene: crate::scene::Scene::default(),
            tool: crate::scene::Tool::Select,
            color: egui::Color32::from_rgb(255, 60, 60),
            stroke_width: 3.0,
            draft: None,
            draft_start: None,
            pending: None,
            sel_mode: None,
            sel_ref: egui::Rect::ZERO,
            text_edit: None,
            move_shape: None,
            text_sel: None,
            blurred: None,
        }
    }
}

const MIN_SELECTION: f32 = 5.0;

fn hit_handle(s: egui::Rect, p: egui::Pos2, radius: f32) -> Option<usize> {
    let pts = [
        s.left_top(),
        s.center_top(),
        s.right_top(),
        s.right_center(),
        s.right_bottom(),
        s.center_bottom(),
        s.left_bottom(),
        s.left_center(),
    ];
    pts.iter().position(|&c| (c - p).length() <= radius)
}

fn handle_cursor(h: usize) -> egui::CursorIcon {
    use egui::CursorIcon;
    match h {
        0 | 4 => CursorIcon::ResizeNwSe,
        2 | 6 => CursorIcon::ResizeNeSw,
        1 | 5 => CursorIcon::ResizeVertical,
        3 | 7 => CursorIcon::ResizeHorizontal,
        _ => CursorIcon::Default,
    }
}

fn resize_rect(base: egui::Rect, handle: usize, fp: egui::Pos2) -> egui::Rect {
    let (mut l, mut r, mut t, mut b) = (base.left(), base.right(), base.top(), base.bottom());
    match handle {
        0 => {
            l = fp.x;
            t = fp.y;
        }
        1 => t = fp.y,
        2 => {
            r = fp.x;
            t = fp.y;
        }
        3 => r = fp.x,
        4 => {
            r = fp.x;
            b = fp.y;
        }
        5 => b = fp.y,
        6 => {
            l = fp.x;
            b = fp.y;
        }
        7 => l = fp.x,
        _ => {}
    }
    egui::Rect::from_two_pos(egui::pos2(l, t), egui::pos2(r, b))
}

struct Shared {
    device: wgpu::Device,
    queue: wgpu::Queue,
    _frame_texture: wgpu::Texture,
    _frame_view: wgpu::TextureView,
    _blur_texture: Option<wgpu::Texture>,
    blur_view: Option<wgpu::TextureView>,
}

struct WindowState {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    renderer: Renderer,
    frame_texture_id: egui::TextureId,
    blur_texture_id: Option<egui::TextureId>,
    region: egui::Rect,
    frames: u64,
    cursor: Option<egui::Pos2>,
}

impl ApplicationHandler for OverlayApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.initialized {
            return;
        }
        self.initialized = true;
        if let Err(e) = self.init(event_loop) {
            eprintln!("[rayshot] failed to initialize overlay: {e:?}");
            event_loop.exit();
            return;
        }

        if let Some(spec) = std::env::var_os("RAYSHOT_SHOW_SEL") {
            self.selection = parse_crop(&spec.to_string_lossy());
        }
        if let Some(spec) = std::env::var_os("RAYSHOT_TEST_TEXT") {
            let s = spec.to_string_lossy();
            let mut it = s.splitn(3, ',');
            if let (Some(x), Some(y), Some(t)) = (it.next(), it.next(), it.next()) {
                if let (Ok(x), Ok(y)) = (x.trim().parse::<f32>(), y.trim().parse::<f32>()) {
                    self.tool = crate::scene::Tool::Text;
                    self.text_edit = Some(TextDraft {
                        pos: egui::pos2(x, y),
                        buf: t.to_string(),
                        color: self.color,
                        size: 24.0,
                    });
                }
            }
        }
        if std::env::var_os("RAYSHOT_DEMO").is_some() {
            use crate::scene::Shape;
            let red = egui::Color32::from_rgb(255, 60, 60);
            let yellow = egui::Color32::from_rgb(255, 200, 40);
            let green = egui::Color32::from_rgb(60, 200, 90);
            self.scene.push(Shape::Rect {
                rect: egui::Rect::from_min_max(egui::pos2(400.0, 300.0), egui::pos2(800.0, 600.0)),
                color: red,
                width: 3.0,
            });
            self.scene.push(Shape::Arrow {
                from: egui::pos2(900.0, 400.0),
                to: egui::pos2(1250.0, 700.0),
                color: yellow,
                width: 4.0,
            });
            self.scene.push(Shape::Line {
                from: egui::pos2(300.0, 720.0),
                to: egui::pos2(760.0, 760.0),
                color: green,
                width: 3.0,
            });
            self.scene.push(Shape::Text {
                pos: egui::pos2(420.0, 240.0),
                text: "rayshot text!".to_string(),
                color: yellow,
                size: 28.0,
            });
            let mut bcells = Vec::new();
            for i in 0..30 {
                crate::scene::add_brush_cells(
                    &mut bcells,
                    &self.frame.rgba,
                    self.frame.width,
                    self.frame.height,
                    egui::pos2(950.0 + i as f32 * 12.0, 300.0),
                    crate::scene::PIXEL_CELL,
                    crate::scene::PIXEL_BRUSH,
                    crate::scene::PIXEL_SAMPLE,
                );
            }
            self.scene.push(Shape::Pixelate {
                cell: crate::scene::PIXEL_CELL,
                cells: bcells,
            });
        }
        if let Some(spec) = std::env::var_os("RAYSHOT_TEST_CROP") {
            self.selection = parse_crop(&spec.to_string_lossy());
            match self.finish() {
                Ok(Some(path)) => eprintln!("[rayshot] test saved + copied {}", path.display()),
                Ok(None) => eprintln!("[rayshot] test: empty selection"),
                Err(e) => eprintln!("[rayshot] test finish failed: {e:?}"),
            }
            crate::anim::force_restore();
            unsafe { libc::_exit(0) };
        }

        if std::env::var_os("RAYSHOT_TEST_QUIT").is_some() {
            self.hide_and_exit(event_loop);
        }

        for win in &self.windows {
            win.window.request_redraw();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(idx) = self.windows.iter().position(|w| w.window.id() == id) else {
            return;
        };

        {
            let win = &mut self.windows[idx];
            let resp = win.egui_state.on_window_event(&win.window, &event);
            if resp.repaint {
                win.window.request_redraw();
            }
        }

        match event {
            WindowEvent::CloseRequested => {
                eprintln!("[rayshot] exit: CloseRequested");
                self.hide_and_exit(event_loop);
            }
            WindowEvent::ModifiersChanged(m) => {
                self.modifiers = m.state();
            }
            WindowEvent::CursorMoved { position, .. } => {
                let sf = self.windows[idx].window.scale_factor();
                let lp = position.to_logical::<f64>(sf);
                self.windows[idx].cursor = Some(egui::pos2(lp.x as f32, lp.y as f32));
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state.is_pressed() {
                    if self.text_edit.is_some() {
                        if matches!(event.logical_key, Key::Named(NamedKey::Escape)) {
                            self.commit_text();
                        }
                        self.request_redraw_all();
                    } else {
                        let ctrl = self.modifiers.control_key();
                        let shift = self.modifiers.shift_key();
                        let code = match event.physical_key {
                            PhysicalKey::Code(c) => Some(c),
                            _ => None,
                        };
                        let is = |c: KeyCode| code == Some(c);
                        use crate::scene::Tool;
                        if matches!(event.logical_key, Key::Named(NamedKey::Escape)) {
                            eprintln!("[rayshot] exit: Escape (cancelled)");
                            self.hide_and_exit(event_loop);
                        } else if matches!(event.logical_key, Key::Named(NamedKey::Enter)) {
                            self.finish_and_exit(event_loop);
                        } else if ctrl && is(KeyCode::KeyC) {
                            self.finish_and_exit(event_loop);
                        } else if ctrl && is(KeyCode::KeyZ) && !shift {
                            self.scene.undo();
                            self.request_redraw_all();
                        } else if ctrl && (is(KeyCode::KeyY) || (is(KeyCode::KeyZ) && shift)) {
                            self.scene.redo();
                            self.request_redraw_all();
                        } else if !ctrl {
                            let new_tool = if is(KeyCode::KeyS) || is(KeyCode::KeyV) {
                                Some(Tool::Select)
                            } else if is(KeyCode::KeyR) {
                                Some(Tool::Rect)
                            } else if is(KeyCode::KeyA) {
                                Some(Tool::Arrow)
                            } else if is(KeyCode::KeyL) {
                                Some(Tool::Line)
                            } else if is(KeyCode::KeyP) {
                                Some(Tool::Pen)
                            } else if is(KeyCode::KeyM) {
                                Some(Tool::Marker)
                            } else if is(KeyCode::KeyT) {
                                Some(Tool::Text)
                            } else if is(KeyCode::KeyX) {
                                Some(Tool::Pixelate)
                            } else if is(KeyCode::KeyB) {
                                Some(Tool::Blur)
                            } else {
                                None
                            };
                            if let Some(t) = new_tool {
                                self.tool = t;
                                self.request_redraw_all();
                            }
                        }
                    }
                }
            }
            WindowEvent::Resized(size) => {
                if size.width > 0 && size.height > 0 {
                    if let Some(shared) = self.shared.as_ref() {
                        let win = &mut self.windows[idx];
                        win.config.width = size.width;
                        win.config.height = size.height;
                        win.surface.configure(&shared.device, &win.config);
                        win.window.request_redraw();
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                if let Err(e) = self.render_window(idx) {
                    eprintln!("[rayshot] render error: {e:?}");
                }
                match self.pending.take() {
                    Some(Action::Copy) => self.finish_and_exit(event_loop),
                    Some(Action::Save) => self.save_and_exit(event_loop),
                    Some(Action::Close) => {
                        eprintln!("[rayshot] exit: toolbar close");
                        self.hide_and_exit(event_loop);
                    }
                    None => {}
                }
            }
            _ => {}
        }
    }
}

impl OverlayApp {
    fn init(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        let init_started = std::time::Instant::now();
        let windowed = std::env::var_os("RAYSHOT_WINDOWED").is_some();
        let frame_w = self.frame.width as f32;
        let frame_h = self.frame.height as f32;

        let mut targets: Vec<(Arc<Window>, egui::Rect)> = Vec::new();

        if windowed {
            let attrs = Window::default_attributes()
                .with_title("rayshot")
                .with_window_level(winit::window::WindowLevel::AlwaysOnTop)
                .with_inner_size(winit::dpi::LogicalSize::new(1600.0, 900.0));
            let window = Arc::new(event_loop.create_window(attrs).context("create window")?);
            let region =
                egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(frame_w, frame_h));
            targets.push((window, region));
        } else {
            for monitor in event_loop.available_monitors() {
                let pos = monitor.position();
                let size = monitor.size();
                let region = egui::Rect::from_min_size(
                    egui::pos2(pos.x as f32, pos.y as f32),
                    egui::vec2(size.width as f32, size.height as f32),
                );
                let attrs = Window::default_attributes()
                    .with_title("rayshot")
                    .with_fullscreen(Some(Fullscreen::Borderless(Some(monitor))));
                let window = Arc::new(event_loop.create_window(attrs).context("create window")?);
                targets.push((window, region));
            }
        }
        if targets.is_empty() {
            anyhow::bail!("no monitors/windows to display on");
        }
        eprintln!("[rayshot] creating {} overlay window(s)", targets.len());

        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());

        let mut surfaces: Vec<wgpu::Surface<'static>> = Vec::with_capacity(targets.len());
        for (window, _) in &targets {
            surfaces.push(
                instance
                    .create_surface(window.clone())
                    .context("failed to create surface")?,
            );
        }

        let adapter = self
            .rt
            .block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surfaces[0]),
                force_fallback_adapter: false,
            }))
            .context("no suitable GPU adapter found")?;
        let (device, queue) = self
            .rt
            .block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("rayshot device"),
                ..Default::default()
            }))
            .context("failed to request GPU device")?;

        let extent = wgpu::Extent3d {
            width: self.frame.width,
            height: self.frame.height,
            depth_or_array_layers: 1,
        };
        let frame_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("capture frame"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &frame_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &self.frame.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * self.frame.width),
                rows_per_image: Some(self.frame.height),
            },
            extent,
        );
        let frame_view = frame_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut windows = Vec::with_capacity(targets.len());
        for ((window, region), surface) in targets.into_iter().zip(surfaces.into_iter()) {
            let size = window.inner_size();
            let config = surface
                .get_default_config(&adapter, size.width.max(1), size.height.max(1))
                .context("surface not supported by adapter")?;
            surface.configure(&device, &config);

            let egui_ctx = egui::Context::default();
            let mut fonts = egui::FontDefinitions::default();
            egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
            egui_ctx.set_fonts(fonts);
            let egui_state = egui_winit::State::new(
                egui_ctx.clone(),
                egui_ctx.viewport_id(),
                &window,
                Some(window.scale_factor() as f32),
                None,
                Some(device.limits().max_texture_dimension_2d as usize),
            );
            let mut renderer = Renderer::new(&device, config.format, RendererOptions::default());
            let frame_texture_id =
                renderer.register_native_texture(&device, &frame_view, wgpu::FilterMode::Linear);

            windows.push(WindowState {
                window,
                surface,
                config,
                egui_ctx,
                egui_state,
                renderer,
                frame_texture_id,
                blur_texture_id: None,
                region,
                frames: 0,
                cursor: None,
            });
        }
        eprintln!("[rayshot] surface format: {:?}", windows[0].config.format);

        self.shared = Some(Shared {
            device,
            queue,
            _frame_texture: frame_texture,
            _frame_view: frame_view,
            _blur_texture: None,
            blur_view: None,
        });
        self.windows = windows;
        eprintln!("[rayshot] gpu/window init: {:?}", init_started.elapsed());
        Ok(())
    }

    fn render_window(&mut self, idx: usize) -> Result<()> {
        if self.shared.is_none() {
            return Ok(());
        }
        let frame_w = self.frame.width as f32;
        let frame_h = self.frame.height as f32;
        let frame = self.frame.clone();
        if matches!(self.tool, crate::scene::Tool::Blur) && self.blurred.is_none() {
            let b = crate::scene::blur_frame(&self.frame.rgba, self.frame.width, self.frame.height);
            self.blurred = Some(Arc::new(b));
        }
        if let (Some(b), Some(shared)) = (self.blurred.as_ref(), self.shared.as_mut()) {
            if shared.blur_view.is_none() {
                let extent = wgpu::Extent3d {
                    width: self.frame.width,
                    height: self.frame.height,
                    depth_or_array_layers: 1,
                };
                let tex = shared.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("blur frame"),
                    size: extent,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                shared.queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &tex,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    b.as_slice(),
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(4 * self.frame.width),
                        rows_per_image: Some(self.frame.height),
                    },
                    extent,
                );
                let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
                shared._blur_texture = Some(tex);
                shared.blur_view = Some(view);
            }
        }
        let win = &mut self.windows[idx];
        if win.blur_texture_id.is_none() {
            if let Some(shared) = self.shared.as_ref() {
                if let Some(view) = shared.blur_view.as_ref() {
                    win.blur_texture_id = Some(win.renderer.register_native_texture(
                        &shared.device,
                        view,
                        wgpu::FilterMode::Linear,
                    ));
                }
            }
        }

        win.frames += 1;
        if win.frames <= 2 {
            eprintln!("[rayshot] window {idx} render #{}", win.frames);
        }

        let region = win.region;
        let texture_id = win.frame_texture_id;
        let blur_tex = win.blur_texture_id.unwrap_or(texture_id);
        let raw_cursor = win.cursor;
        let ctx = win.egui_ctx.clone();
        let raw_input = win.egui_state.take_egui_input(&win.window);

        use crate::scene::{Shape, Tool};

        let mut selection = self.selection;
        let mut drag_start = self.drag_start;
        let mut draft = self.draft.clone();
        let mut draft_start = self.draft_start;
        let mut tool = self.tool;
        let mut color = self.color;
        let stroke_width = self.stroke_width;
        let shapes = self.scene.shapes();
        let mut committed: Option<Shape> = None;
        let mut clear_scene = false;
        let mut action: Option<Action> = None;
        let mut sel_mode = self.sel_mode;
        let mut sel_ref = self.sel_ref;
        let mut moving_shape = self.move_shape.clone();
        let mut move_apply: Option<(usize, Shape)> = None;
        let mut start_text: Option<egui::Pos2> = None;
        let mut text_buf = self.text_edit.as_ref().map(|d| d.buf.clone());
        let text_info = self.text_edit.as_ref().map(|d| (d.pos, d.color, d.size));
        let mut text_lost_focus = false;
        let mut text_pos_shift = egui::Vec2::ZERO;
        let text_sel_prev = self.text_sel;
        let mut owner_text_sel: Option<(usize, usize, usize)> = None;

        let full_output = ctx.run_ui(raw_input, |ui| {
            let avail = ui.max_rect();

            let scale = (avail.width() / region.width()).min(avail.height() / region.height());
            let draw_size = egui::vec2(region.width() * scale, region.height() * scale);
            let image_rect = egui::Rect::from_center_size(avail.center(), draw_size);

            let to_frame = |p: egui::Pos2| {
                let local = (p - image_rect.min) / scale;
                egui::pos2(
                    (region.min.x + local.x).clamp(0.0, frame_w),
                    (region.min.y + local.y).clamp(0.0, frame_h),
                )
            };
            let to_screen = |fp: egui::Pos2| image_rect.min + (fp - region.min) * scale;

            let uv = egui::Rect::from_min_max(
                egui::pos2(region.min.x / frame_w, region.min.y / frame_h),
                egui::pos2(region.max.x / frame_w, region.max.y / frame_h),
            );
            ui.painter()
                .image(texture_id, image_rect, uv, egui::Color32::WHITE);

            let resp = ui.interact(
                image_rect,
                egui::Id::new("rayshot-canvas"),
                egui::Sense::click_and_drag(),
            );
            let cursor_pos = raw_cursor.or_else(|| resp.interact_pointer_pos());
            match tool {
                Tool::Select => {
                    if resp.drag_started() {
                        if let Some(p) = cursor_pos {
                            let fp = to_frame(p);
                            drag_start = Some(fp);
                            if let Some(i) = crate::scene::shape_hit(shapes, fp) {
                                moving_shape = Some((i, shapes[i].clone()));
                                sel_mode = None;
                            } else if let Some(sel) = selection {
                                let s = egui::Rect::from_min_max(
                                    to_screen(sel.min),
                                    to_screen(sel.max),
                                );
                                if let Some(h) = hit_handle(s, p, 12.0) {
                                    sel_mode = Some(SelMode::Resize(h));
                                    sel_ref = sel;
                                } else if sel.contains(fp) {
                                    sel_mode = Some(SelMode::Move);
                                    sel_ref = sel;
                                } else {
                                    sel_mode = Some(SelMode::New);
                                    clear_scene = true;
                                }
                            } else {
                                sel_mode = Some(SelMode::New);
                            }
                        }
                    }
                    if resp.dragged() {
                        if let (Some(start), Some(p)) = (drag_start, cursor_pos) {
                            let fp = to_frame(p);
                            if let Some((i, orig)) = &moving_shape {
                                move_apply = Some((*i, crate::scene::translated(orig, fp - start)));
                            } else {
                                match sel_mode {
                                    Some(SelMode::New) => {
                                        selection = Some(egui::Rect::from_two_pos(start, fp));
                                    }
                                    Some(SelMode::Move) => {
                                        let mut m = sel_ref.translate(fp - start);
                                        let dx =
                                            (-m.left()).max(0.0) - (m.right() - frame_w).max(0.0);
                                        let dy =
                                            (-m.top()).max(0.0) - (m.bottom() - frame_h).max(0.0);
                                        m = m.translate(egui::vec2(dx, dy));
                                        selection = Some(m);
                                    }
                                    Some(SelMode::Resize(h)) => {
                                        let r = resize_rect(sel_ref, h, fp);
                                        if r.width() >= MIN_SELECTION && r.height() >= MIN_SELECTION
                                        {
                                            selection = Some(r);
                                        }
                                    }
                                    None => {}
                                }
                            }
                        }
                    }
                    if resp.drag_stopped() {
                        if sel_mode == Some(SelMode::New) {
                            if let Some(s) = selection {
                                if s.width() < MIN_SELECTION || s.height() < MIN_SELECTION {
                                    selection = None;
                                }
                            }
                        }
                        sel_mode = None;
                        moving_shape = None;
                    }
                    let icon = if let Some(sel) = selection {
                        let s = egui::Rect::from_min_max(to_screen(sel.min), to_screen(sel.max));
                        match sel_mode {
                            Some(SelMode::Resize(h)) => handle_cursor(h),
                            Some(SelMode::Move) => egui::CursorIcon::Grabbing,
                            _ => match resp.hover_pos() {
                                Some(p) if hit_handle(s, p, 12.0).is_some() => {
                                    handle_cursor(hit_handle(s, p, 12.0).unwrap())
                                }
                                Some(p) if s.contains(p) => egui::CursorIcon::Move,
                                _ => egui::CursorIcon::Crosshair,
                            },
                        }
                    } else {
                        egui::CursorIcon::Crosshair
                    };
                    ui.ctx().set_cursor_icon(icon);
                }
                Tool::Text => {
                    if resp.clicked() {
                        if let Some(p) = cursor_pos {
                            start_text = Some(to_frame(p));
                        }
                    }
                }
                _ => {
                    if resp.drag_started() {
                        if let Some(p) = cursor_pos {
                            let fp = to_frame(p);
                            draft_start = Some(fp);
                            match tool {
                                Tool::Pen => {
                                    draft = Some(Shape::Pen {
                                        points: vec![fp],
                                        color,
                                        width: stroke_width,
                                    });
                                }
                                Tool::Marker => {
                                    draft = Some(Shape::Pen {
                                        points: vec![fp],
                                        color: marker_color(color),
                                        width: 16.0,
                                    });
                                }
                                Tool::Pixelate => {
                                    let (cell, brush, sample) = brush_params(tool);
                                    let mut cells = Vec::new();
                                    crate::scene::add_brush_cells(
                                        &mut cells,
                                        &frame.rgba,
                                        frame.width,
                                        frame.height,
                                        fp,
                                        cell,
                                        brush,
                                        sample,
                                    );
                                    draft = Some(Shape::Pixelate { cell, cells });
                                }
                                _ => {}
                            }
                        }
                    }
                    if resp.dragged() {
                        if let Some(p) = cursor_pos {
                            let fp = to_frame(p);
                            match tool {
                                Tool::Rect => {
                                    if let Some(st) = draft_start {
                                        draft = Some(Shape::Rect {
                                            rect: egui::Rect::from_two_pos(st, fp),
                                            color,
                                            width: stroke_width,
                                        });
                                    }
                                }
                                Tool::Line => {
                                    if let Some(st) = draft_start {
                                        draft = Some(Shape::Line {
                                            from: st,
                                            to: fp,
                                            color,
                                            width: stroke_width,
                                        });
                                    }
                                }
                                Tool::Arrow => {
                                    if let Some(st) = draft_start {
                                        draft = Some(Shape::Arrow {
                                            from: st,
                                            to: fp,
                                            color,
                                            width: stroke_width,
                                        });
                                    }
                                }
                                Tool::Pen | Tool::Marker => {
                                    if let Some(Shape::Pen { points, .. }) = &mut draft {
                                        points.push(fp);
                                    }
                                }
                                Tool::Pixelate => {
                                    if let Some(Shape::Pixelate { cell, cells }) = &mut draft {
                                        let (_, brush, sample) = brush_params(tool);
                                        crate::scene::add_brush_cells(
                                            cells,
                                            &frame.rgba,
                                            frame.width,
                                            frame.height,
                                            fp,
                                            *cell,
                                            brush,
                                            sample,
                                        );
                                    }
                                }
                                Tool::Blur => {
                                    if let Some(st) = draft_start {
                                        draft = Some(Shape::Blur {
                                            rect: egui::Rect::from_two_pos(st, fp),
                                        });
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    if resp.drag_stopped() {
                        committed = draft.take();
                        draft_start = None;
                    }
                }
            }

            let brush_ring = match tool {
                Tool::Pen | Tool::Line => Some((stroke_width * 0.5).max(5.0)),
                Tool::Marker => Some(8.0),
                Tool::Pixelate => Some(crate::scene::PIXEL_BRUSH),
                _ => None,
            };
            let brush_active = resp.hovered() || resp.dragged();
            match tool {
                Tool::Select => {}
                Tool::Text => ui.ctx().set_cursor_icon(egui::CursorIcon::Text),
                _ if brush_ring.is_some() && brush_active => {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::None);
                }
                _ => ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair),
            }

            let painter = ui.painter();
            let dim_alpha = if std::env::var_os("RAYSHOT_NODIM").is_some() {
                0
            } else {
                150
            };
            let dim = egui::Color32::from_black_alpha(dim_alpha);

            let visible = selection.and_then(|sel| {
                let v = sel.intersect(region);
                (v.width() > 0.5 && v.height() > 0.5).then_some((sel, v))
            });

            if let Some((sel, vis)) = visible {
                let s = egui::Rect::from_min_max(to_screen(vis.min), to_screen(vis.max));
                let s_true = egui::Rect::from_min_max(to_screen(sel.min), to_screen(sel.max));
                painter.rect_filled(
                    egui::Rect::from_min_max(
                        image_rect.min,
                        egui::pos2(image_rect.right(), s.top()),
                    ),
                    0,
                    dim,
                );
                painter.rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(image_rect.left(), s.bottom()),
                        image_rect.max,
                    ),
                    0,
                    dim,
                );
                painter.rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(image_rect.left(), s.top()),
                        egui::pos2(s.left(), s.bottom()),
                    ),
                    0,
                    dim,
                );
                painter.rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(s.right(), s.top()),
                        egui::pos2(image_rect.right(), s.bottom()),
                    ),
                    0,
                    dim,
                );
                let corners = [
                    s_true.left_top(),
                    s_true.right_top(),
                    s_true.right_bottom(),
                    s_true.left_bottom(),
                    s_true.left_top(),
                ];
                painter.extend(egui::Shape::dashed_line(
                    &corners,
                    egui::Stroke::new(1.5, egui::Color32::WHITE),
                    6.0,
                    4.0,
                ));
                let hsz = 7.0;
                for c in [
                    s_true.left_top(),
                    s_true.center_top(),
                    s_true.right_top(),
                    s_true.right_center(),
                    s_true.right_bottom(),
                    s_true.center_bottom(),
                    s_true.left_bottom(),
                    s_true.left_center(),
                ] {
                    let hr = egui::Rect::from_center_size(c, egui::vec2(hsz, hsz));
                    painter.rect_filled(hr, 1, egui::Color32::WHITE);
                    painter.rect_stroke(
                        hr,
                        1,
                        egui::Stroke::new(1.0, egui::Color32::from_gray(70)),
                        egui::StrokeKind::Outside,
                    );
                }
                if region.contains(sel.min) {
                    let label = format!(
                        "{} × {}",
                        sel.width().round() as i32,
                        sel.height().round() as i32
                    );
                    let galley = painter.layout_no_wrap(
                        label,
                        egui::FontId::proportional(12.0),
                        egui::Color32::WHITE,
                    );
                    let pad = egui::vec2(6.0, 3.0);
                    let pill_min = egui::pos2(
                        s_true.left(),
                        s_true.top() - galley.size().y - 2.0 * pad.y - 6.0,
                    );
                    let pill = egui::Rect::from_min_size(pill_min, galley.size() + 2.0 * pad);
                    painter.rect_filled(pill, 4, egui::Color32::from_black_alpha(190));
                    painter.galley(pill_min + pad, galley, egui::Color32::WHITE);
                }
            } else {
                painter.rect_filled(image_rect, 0, dim);
            }

            for shape in shapes {
                crate::scene::paint(
                    painter, shape, &to_screen, scale, blur_tex, frame_w, frame_h,
                );
            }
            if let Some(d) = &draft {
                crate::scene::paint(painter, d, &to_screen, scale, blur_tex, frame_w, frame_h);
            }
            if let Some(c) = &committed {
                crate::scene::paint(painter, c, &to_screen, scale, blur_tex, frame_w, frame_h);
            }
            if let (Some(r), Some(p)) = (brush_ring, raw_cursor) {
                if brush_active {
                    let rs = r * scale;
                    painter.circle_stroke(
                        p,
                        rs,
                        egui::Stroke::new(2.5, egui::Color32::from_black_alpha(130)),
                    );
                    painter.circle_stroke(
                        p,
                        rs,
                        egui::Stroke::new(1.2, egui::Color32::from_white_alpha(235)),
                    );
                    painter.circle_filled(p, 1.4, egui::Color32::from_white_alpha(235));
                }
            }
            if let (Some(buf), Some((pos, col, tsize))) = (&text_buf, text_info) {
                if !region.contains(pos) {
                    let font = egui::FontId::proportional(tsize * scale);
                    let galley = painter.layout_no_wrap(buf.clone(), font, col);
                    let gsize = galley.size();
                    let line_h = gsize.y.max(tsize * scale);
                    let origin = to_screen(pos);
                    if let Some((s, e, _)) = text_sel_prev {
                        if s != e {
                            let x0 = galley.pos_from_cursor(egui::text::CCursor::new(s)).min.x;
                            let x1 = galley.pos_from_cursor(egui::text::CCursor::new(e)).min.x;
                            painter.rect_filled(
                                egui::Rect::from_min_max(
                                    origin + egui::vec2(x0, 0.0),
                                    origin + egui::vec2(x1, line_h),
                                ),
                                1,
                                egui::Color32::from_rgba_unmultiplied(80, 140, 230, 120),
                            );
                        }
                    }
                    painter.galley(origin, galley, col);
                    let bb = egui::Rect::from_min_size(
                        origin,
                        egui::vec2((gsize.x + 8.0).max(16.0), line_h),
                    )
                    .expand(3.0);
                    painter.extend(egui::Shape::dashed_line(
                        &[
                            bb.left_top(),
                            bb.right_top(),
                            bb.right_bottom(),
                            bb.left_bottom(),
                            bb.left_top(),
                        ],
                        egui::Stroke::new(1.0, egui::Color32::from_white_alpha(160)),
                        4.0,
                        3.0,
                    ));
                }
            }
            if let (Some(buf), Some((pos, col, tsize))) = (text_buf.as_mut(), text_info) {
                if region.contains(pos) {
                    egui::Area::new(egui::Id::new("rayshot-text-edit"))
                        .fixed_pos(to_screen(pos))
                        .constrain(false)
                        .show(ui.ctx(), |ui| {
                            ui.style_mut().visuals.text_cursor.stroke = egui::Stroke::NONE;
                            ui.style_mut().visuals.selection.bg_fill = egui::Color32::TRANSPARENT;
                            ui.style_mut().visuals.selection.stroke = egui::Stroke::NONE;
                            let font = egui::FontId::proportional(tsize * scale);
                            let charw = (tsize * scale * 0.6).max(6.0);
                            let tw_before = ui
                                .painter()
                                .layout_no_wrap(buf.clone(), font.clone(), col)
                                .size()
                                .x;
                            let output = egui::TextEdit::singleline(buf)
                                .frame(egui::Frame::NONE)
                                .desired_width(tw_before + charw * 2.0 + 10.0)
                                .font(font.clone())
                                .text_color(egui::Color32::TRANSPARENT)
                                .show(ui);
                            let r = output.response.clone();
                            let cursor_range = output.cursor_range;
                            owner_text_sel = cursor_range.map(|cr| {
                                let rg = cr.as_sorted_char_range();
                                (rg.start, rg.end, cr.primary.index)
                            });

                            let galley = ui.painter().layout_no_wrap(buf.clone(), font, col);
                            let gsize = galley.size();
                            let line_h = gsize.y.max(tsize * scale);
                            let origin = r.rect.min;

                            if let Some(cr) = cursor_range {
                                let range = cr.as_sorted_char_range();
                                if range.start != range.end {
                                    let x0 = galley
                                        .pos_from_cursor(egui::text::CCursor::new(range.start))
                                        .min
                                        .x;
                                    let x1 = galley
                                        .pos_from_cursor(egui::text::CCursor::new(range.end))
                                        .min
                                        .x;
                                    let sel = egui::Rect::from_min_max(
                                        egui::pos2(origin.x + x0, origin.y),
                                        egui::pos2(origin.x + x1, origin.y + line_h),
                                    );
                                    ui.painter().rect_filled(
                                        sel,
                                        1,
                                        egui::Color32::from_rgba_unmultiplied(80, 140, 230, 120),
                                    );
                                }
                            }
                            ui.painter().galley(origin, galley.clone(), col);
                            let caret_idx = cursor_range
                                .map(|cr| cr.primary.index)
                                .unwrap_or_else(|| buf.chars().count());
                            let caret_x = origin.x
                                + galley
                                    .pos_from_cursor(egui::text::CCursor::new(caret_idx))
                                    .min
                                    .x;
                            ui.painter().line_segment(
                                [
                                    egui::pos2(caret_x, origin.y),
                                    egui::pos2(caret_x, origin.y + line_h),
                                ],
                                egui::Stroke::new(2.0, col),
                            );
                            let bb = egui::Rect::from_min_size(
                                r.rect.min,
                                egui::vec2((gsize.x + 8.0).max(16.0), line_h),
                            )
                            .expand(3.0);
                            let pts = [
                                bb.left_top(),
                                bb.right_top(),
                                bb.right_bottom(),
                                bb.left_bottom(),
                                bb.left_top(),
                            ];
                            ui.painter().extend(egui::Shape::dashed_line(
                                &pts,
                                egui::Stroke::new(1.0, egui::Color32::from_white_alpha(160)),
                                4.0,
                                3.0,
                            ));
                            let grip =
                                egui::Rect::from_center_size(bb.left_top(), egui::vec2(12.0, 12.0));
                            let gh = ui.interact(
                                grip,
                                egui::Id::new("rayshot-text-grip"),
                                egui::Sense::drag(),
                            );
                            ui.painter()
                                .rect_filled(grip, 2, egui::Color32::from_white_alpha(210));
                            ui.painter().rect_stroke(
                                grip,
                                2,
                                egui::Stroke::new(1.0, egui::Color32::from_gray(60)),
                                egui::StrokeKind::Outside,
                            );
                            if gh.hovered() || gh.dragged() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
                            }
                            if gh.dragged() {
                                text_pos_shift += gh.drag_delta() / scale;
                            }
                            if !r.has_focus() && !r.lost_focus() && !gh.dragged() {
                                r.request_focus();
                            }
                            if r.lost_focus() {
                                text_lost_focus = true;
                            }
                        });
                }
            }

            if let Some((sel, _vis)) = visible {
                if region.contains(sel.max) {
                    let s = egui::Rect::from_min_max(to_screen(sel.min), to_screen(sel.max));

                    let tools_id = egui::Id::new("rayshot-tools");
                    let actions_id = egui::Id::new("rayshot-actions");
                    let tools_sz = egui::AreaState::load(ui.ctx(), tools_id)
                        .and_then(|st| st.size)
                        .unwrap_or(egui::vec2(46.0, 300.0));
                    let actions_sz = egui::AreaState::load(ui.ctx(), actions_id)
                        .and_then(|st| st.size)
                        .unwrap_or(egui::vec2(46.0, 110.0));
                    let gap = 6.0;
                    let width = tools_sz.x.max(actions_sz.x);
                    let total_h = tools_sz.y + gap + actions_sz.y;
                    let pad = 8.0;

                    let mut x = s.right() + 10.0;
                    if x + width > avail.right() - pad {
                        x = s.left() - 10.0 - width;
                    }
                    if x < avail.left() + pad {
                        x = (avail.right() - pad - width).max(avail.left() + pad);
                    }

                    let mut top = s.top();
                    if top + total_h > avail.bottom() - pad {
                        top = avail.bottom() - pad - total_h;
                    }
                    if top < avail.top() + pad {
                        top = avail.top() + pad;
                    }

                    let tools_area = egui::Area::new(tools_id)
                        .fixed_pos(egui::pos2(x, top))
                        .constrain(false)
                        .show(ui.ctx(), |ui| {
                            egui::Frame::popup(ui.style()).show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 3.0;
                                ui.vertical(|ui| {
                                    for t in [
                                        Tool::Pen,
                                        Tool::Line,
                                        Tool::Arrow,
                                        Tool::Rect,
                                        Tool::Marker,
                                        Tool::Text,
                                        Tool::Pixelate,
                                        Tool::Blur,
                                        Tool::Select,
                                    ] {
                                        if tool_button(ui, t.icon(), tool == t)
                                            .on_hover_text(t.tooltip())
                                            .clicked()
                                        {
                                            tool = t;
                                        }
                                    }
                                    ui.separator();
                                    ui.scope(|ui| {
                                        ui.spacing_mut().interact_size = egui::vec2(30.0, 28.0);
                                        ui.color_edit_button_srgba(&mut color)
                                            .on_hover_text("Colour");
                                    });
                                });
                            });
                        });

                    let actions_pos = tools_area.response.rect.left_bottom() + egui::vec2(0.0, gap);
                    egui::Area::new(actions_id)
                        .fixed_pos(actions_pos)
                        .constrain(false)
                        .show(ui.ctx(), |ui| {
                            egui::Frame::popup(ui.style()).show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 3.0;
                                ui.vertical(|ui| {
                                    use egui_phosphor::regular as p;
                                    if tool_button(ui, p::COPY, false)
                                        .on_hover_text("Copy to clipboard (Ctrl+C)")
                                        .clicked()
                                    {
                                        action = Some(Action::Copy);
                                    }
                                    if tool_button(ui, p::FLOPPY_DISK, false)
                                        .on_hover_text("Save to ~/Pictures")
                                        .clicked()
                                    {
                                        action = Some(Action::Save);
                                    }
                                    if tool_button(ui, p::X, false)
                                        .on_hover_text("Cancel (Esc)")
                                        .clicked()
                                    {
                                        action = Some(Action::Close);
                                    }
                                });
                            });
                        });
                }
            }

            if selection.is_none() {
                painter.text(
                    image_rect.center_top() + egui::vec2(0.0, 18.0),
                    egui::Align2::CENTER_TOP,
                    "drag to select · tools: P L A R M S · Enter/Ctrl+C copy · Esc cancel",
                    egui::FontId::proportional(15.0),
                    egui::Color32::WHITE,
                );
            }
        });

        self.selection = selection;
        self.drag_start = drag_start;
        self.draft = draft;
        self.draft_start = draft_start;
        self.tool = tool;
        self.color = color;
        self.sel_mode = sel_mode;
        self.sel_ref = sel_ref;
        let move_just_started = self.move_shape.is_none() && moving_shape.is_some();
        self.move_shape = moving_shape;
        if move_just_started {
            self.scene.begin_change();
        }
        if let Some((i, shape)) = move_apply {
            self.scene.set_shape(i, shape);
        }

        if let Some(d) = self.text_edit.as_mut() {
            if let Some(buf) = text_buf {
                d.buf = buf;
            }
            d.pos += text_pos_shift;
        }
        if text_lost_focus {
            self.commit_text();
        }

        if tool != crate::scene::Tool::Text {
            self.commit_text();
        }
        if clear_scene {
            self.text_edit = None;
            self.scene = crate::scene::Scene::default();
        }
        if let Some(pos) = start_text {
            self.commit_text();
            self.text_edit = Some(TextDraft {
                pos,
                buf: String::new(),
                color,
                size: 24.0,
            });
        }
        if let Some(s) = committed {
            self.scene.push(s);
        }
        if action.is_some() {
            self.pending = action;
        }

        match self.text_edit.as_ref() {
            Some(d) if region.contains(d.pos) => self.text_sel = owner_text_sel,
            None => self.text_sel = None,
            _ => {}
        }

        if self.windows.len() > 1 {
            for (i, w) in self.windows.iter().enumerate() {
                if i != idx {
                    w.window.request_redraw();
                }
            }
        }

        let shared = self.shared.as_ref().expect("shared gpu initialized");
        let win = &mut self.windows[idx];
        win.egui_state
            .handle_platform_output(&win.window, full_output.platform_output);

        let paint_jobs = ctx.tessellate(full_output.shapes, full_output.pixels_per_point);
        let screen_descriptor = ScreenDescriptor {
            size_in_pixels: [win.config.width, win.config.height],
            pixels_per_point: full_output.pixels_per_point,
        };

        for (id, delta) in &full_output.textures_delta.set {
            win.renderer
                .update_texture(&shared.device, &shared.queue, *id, delta);
        }

        let surface_texture = match win.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                win.surface.configure(&shared.device, &win.config);
                win.window.request_redraw();
                return Ok(());
            }
            _ => {
                win.window.request_redraw();
                return Ok(());
            }
        };
        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = shared
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("rayshot encoder"),
            });

        let cmd_bufs = win.renderer.update_buffers(
            &shared.device,
            &shared.queue,
            &mut encoder,
            &paint_jobs,
            &screen_descriptor,
        );

        {
            let rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("rayshot pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let mut rpass = rpass.forget_lifetime();
            win.renderer
                .render(&mut rpass, &paint_jobs, &screen_descriptor);
        }

        shared.queue.submit(
            cmd_bufs
                .into_iter()
                .chain(std::iter::once(encoder.finish())),
        );
        surface_texture.present();

        for id in &full_output.textures_delta.free {
            win.renderer.free_texture(id);
        }

        Ok(())
    }

    fn cropped_image(&self) -> Option<image::RgbaImage> {
        let sel = self.selection?;
        let x = sel.min.x.floor().clamp(0.0, self.frame.width as f32) as u32;
        let y = sel.min.y.floor().clamp(0.0, self.frame.height as f32) as u32;
        let x2 = sel.max.x.ceil().clamp(0.0, self.frame.width as f32) as u32;
        let y2 = sel.max.y.ceil().clamp(0.0, self.frame.height as f32) as u32;
        if x2 <= x || y2 <= y {
            return None;
        }
        let full = image::RgbaImage::from_raw(
            self.frame.width,
            self.frame.height,
            self.frame.rgba.clone(),
        )?;
        Some(image::imageops::crop_imm(&full, x, y, x2 - x, y2 - y).to_image())
    }

    fn render_selection_to_image(&self) -> Result<Option<image::RgbaImage>> {
        let shared = self.shared.as_ref().context("gpu not initialized")?;
        let Some(sel) = self.selection else {
            return Ok(None);
        };

        let fx = sel.min.x.floor().clamp(0.0, self.frame.width as f32);
        let fy = sel.min.y.floor().clamp(0.0, self.frame.height as f32);
        let fx2 = sel.max.x.ceil().clamp(0.0, self.frame.width as f32);
        let fy2 = sel.max.y.ceil().clamp(0.0, self.frame.height as f32);
        let (w, h) = ((fx2 - fx) as u32, (fy2 - fy) as u32);
        if w == 0 || h == 0 {
            return Ok(None);
        }

        let device = &shared.device;
        let queue = &shared.queue;
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let extent = wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        };

        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("export target"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());

        let ctx = egui::Context::default();
        let mut renderer = Renderer::new(device, format, RendererOptions::default());
        let frame_tex_id =
            renderer.register_native_texture(device, &shared._frame_view, wgpu::FilterMode::Linear);
        let blur_tex_id = match shared.blur_view.as_ref() {
            Some(v) => renderer.register_native_texture(device, v, wgpu::FilterMode::Linear),
            None => frame_tex_id,
        };

        let frame_w = self.frame.width as f32;
        let frame_h = self.frame.height as f32;
        let sel_min = egui::pos2(fx, fy);
        let shapes = self.scene.shapes();

        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(w as f32, h as f32),
            )),
            ..Default::default()
        };
        let full_output = ctx.run_ui(raw_input, |ui| {
            let rect =
                egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(w as f32, h as f32));
            let uv = egui::Rect::from_min_max(
                egui::pos2(fx / frame_w, fy / frame_h),
                egui::pos2(fx2 / frame_w, fy2 / frame_h),
            );
            ui.painter()
                .image(frame_tex_id, rect, uv, egui::Color32::WHITE);
            let to_screen = |p: egui::Pos2| egui::pos2(p.x - sel_min.x, p.y - sel_min.y);
            for s in shapes {
                crate::scene::paint(
                    ui.painter(),
                    s,
                    &to_screen,
                    1.0,
                    blur_tex_id,
                    frame_w,
                    frame_h,
                );
            }
        });

        let paint_jobs = ctx.tessellate(full_output.shapes, 1.0);
        let screen_descriptor = ScreenDescriptor {
            size_in_pixels: [w, h],
            pixels_per_point: 1.0,
        };
        for (id, delta) in &full_output.textures_delta.set {
            renderer.update_texture(device, queue, *id, delta);
        }

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("export"),
        });
        let cmd_bufs =
            renderer.update_buffers(device, queue, &mut encoder, &paint_jobs, &screen_descriptor);
        {
            let rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("export pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let mut rpass = rpass.forget_lifetime();
            renderer.render(&mut rpass, &paint_jobs, &screen_descriptor);
        }

        let unpadded = w * 4;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded = unpadded.div_ceil(align) * align;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("export readback"),
            size: (padded * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(h),
                },
            },
            extent,
        );
        queue.submit(
            cmd_bufs
                .into_iter()
                .chain(std::iter::once(encoder.finish())),
        );

        let (tx, rx) = std::sync::mpsc::channel();
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .context("device poll failed")?;
        rx.recv()
            .context("readback channel closed")?
            .context("buffer map failed")?;

        let data = buffer.slice(..).get_mapped_range();
        let mut out = Vec::with_capacity((unpadded * h) as usize);
        for row in 0..h {
            let start = (row * padded) as usize;
            out.extend_from_slice(&data[start..start + unpadded as usize]);
        }
        drop(data);
        buffer.unmap();

        image::RgbaImage::from_raw(w, h, out)
            .map(Some)
            .context("export image build failed")
    }

    fn finish(&self) -> Result<Option<std::path::PathBuf>> {
        let render_started = std::time::Instant::now();
        let img = match self.render_selection_to_image() {
            Ok(Some(img)) => img,
            Ok(None) => return Ok(None),
            Err(e) => {
                eprintln!("[rayshot] annotated export failed ({e:#}); using raw crop");
                match self.cropped_image() {
                    Some(img) => img,
                    None => return Ok(None),
                }
            }
        };
        let t_render = render_started.elapsed();
        let t1 = std::time::Instant::now();
        let png = crate::export::to_png_bytes(&img)?;
        let t_encode = t1.elapsed();
        let t2 = std::time::Instant::now();
        let path = crate::export::save_to_scratch(&png)?;
        let t_save = t2.elapsed();
        let t3 = std::time::Instant::now();
        if let Err(e) = crate::export::copy_png_to_clipboard(&png) {
            eprintln!("[rayshot] clipboard copy failed: {e:#}");
        }
        eprintln!(
            "[rayshot] finish: render {:?}, encode {:?}, save {:?}, clip {:?}",
            t_render,
            t_encode,
            t_save,
            t3.elapsed()
        );
        Ok(Some(path))
    }

    fn request_redraw_all(&self) {
        for w in &self.windows {
            w.window.request_redraw();
        }
    }

    fn commit_text(&mut self) {
        if let Some(d) = self.text_edit.take() {
            if !d.buf.trim().is_empty() {
                self.scene.push(crate::scene::Shape::Text {
                    pos: d.pos,
                    text: d.buf,
                    color: d.color,
                    size: d.size,
                });
            }
        }
    }

    fn hide_and_exit(&self, _event_loop: &ActiveEventLoop) -> ! {
        if let Some(sel) = self.selection {
            if sel.width() > 1.0 && sel.height() > 1.0 {
                crate::export::save_last_selection(sel.min.x, sel.min.y, sel.width(), sel.height());
            }
        }
        crate::anim::restore_detached();
        unsafe { libc::_exit(0) }
    }

    fn finish_and_exit(&self, event_loop: &ActiveEventLoop) {
        match self.finish() {
            Ok(Some(path)) => {
                eprintln!("[rayshot] copied to clipboard + saved {}", path.display());
                self.hide_and_exit(event_loop);
            }
            Ok(None) => eprintln!("[rayshot] nothing to copy (no selection)"),
            Err(e) => eprintln!("[rayshot] finish failed: {e:?}"),
        }
    }

    fn save_and_exit(&self, event_loop: &ActiveEventLoop) {
        let img = match self.render_selection_to_image() {
            Ok(Some(img)) => img,
            Ok(None) => {
                eprintln!("[rayshot] nothing to save (no selection)");
                return;
            }
            Err(e) => {
                eprintln!("[rayshot] save render failed: {e:#}");
                return;
            }
        };
        match crate::export::to_png_bytes(&img)
            .and_then(|png| crate::export::save_to_pictures(&png))
        {
            Ok(path) => {
                eprintln!("[rayshot] saved {}", path.display());
                self.hide_and_exit(event_loop);
            }
            Err(e) => eprintln!("[rayshot] save failed: {e:#}"),
        }
    }
}

fn marker_color(c: egui::Color32) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 110)
}

fn tool_button(ui: &mut egui::Ui, glyph: &str, selected: bool) -> egui::Response {
    let text = egui::RichText::new(glyph).size(18.0);
    ui.add_sized(
        egui::vec2(30.0, 28.0),
        egui::Button::selectable(selected, text),
    )
}

fn brush_params(tool: crate::scene::Tool) -> (f32, f32, f32) {
    use crate::scene;
    match tool {
        scene::Tool::Pixelate => (scene::PIXEL_CELL, scene::PIXEL_BRUSH, scene::PIXEL_SAMPLE),
        _ => (scene::BLUR_CELL, scene::BLUR_BRUSH, scene::BLUR_SAMPLE),
    }
}

fn parse_crop(spec: &str) -> Option<egui::Rect> {
    let parts: Vec<f32> = spec
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    if parts.len() != 4 {
        return None;
    }
    Some(egui::Rect::from_min_size(
        egui::pos2(parts[0], parts[1]),
        egui::vec2(parts[2], parts[3]),
    ))
}
