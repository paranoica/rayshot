use std::sync::Arc;

use anyhow::{Context, Result};
use egui_wgpu::{Renderer, RendererOptions, ScreenDescriptor};
use tokio::runtime::Handle;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
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

enum UserEvent {
    Captured(Result<Frame, String>),
    Show {
        id: u64,
        width: u32,
        height: u32,
        monitors: Vec<crate::live::MonitorRect>,
    },
    SessionFrame {
        id: u64,
        result: Result<Frame, String>,
    },
    Cancel {
        id: u64,
    },
    Select {
        id: u64,
        from: (f32, f32),
        to: (f32, f32),
    },
    Quit,
}

pub fn run(rt: Handle) -> Result<()> {
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .context("failed to create winit event loop")?;
    let proxy = event_loop.create_proxy();
    rt.spawn(async move {
        let result = crate::capture::capture_frame()
            .await
            .map_err(|e| format!("{e:?}"));
        let _ = proxy.send_event(UserEvent::Captured(result));
    });
    let mut app = OverlayApp::new(rt);
    event_loop.run_app(&mut app).context("event loop error")?;
    Ok(())
}

pub fn run_live(
    width: u32,
    height: u32,
    monitors: Vec<crate::live::MonitorRect>,
    rt: Handle,
) -> Result<()> {
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .context("failed to create winit event loop")?;
    let proxy = event_loop.create_proxy();
    std::thread::Builder::new()
        .name("rayshot-stdin".into())
        .spawn(move || read_commands(proxy))
        .context("failed to spawn stdin reader")?;
    let mut app = OverlayApp::new(rt);
    app.live = true;
    app.frame_size = Some((width, height));
    app.monitor_rects = monitors;
    event_loop.run_app(&mut app).context("event loop error")?;
    Ok(())
}

pub fn run_serve(rt: Handle) -> Result<()> {
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .context("failed to create winit event loop")?;
    let proxy = event_loop.create_proxy();
    std::thread::Builder::new()
        .name("rayshot-stdin".into())
        .spawn(move || read_commands(proxy))
        .context("failed to spawn stdin reader")?;
    let mut app = OverlayApp::new(rt);
    app.serve = true;
    event_loop.run_app(&mut app).context("event loop error")?;
    Ok(())
}

fn reply(line: &str) {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}").and_then(|_| out.flush());
}

fn read_commands(proxy: winit::event_loop::EventLoopProxy<UserEvent>) {
    use crate::live::Command;
    use std::io::BufRead;
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let event = match crate::live::parse_command(&line) {
            Some(Command::Show {
                id,
                width,
                height,
                monitors,
            }) => UserEvent::Show {
                id,
                width,
                height,
                monitors,
            },
            Some(Command::Frame { id, path }) => {
                let started = std::time::Instant::now();
                let result = crate::capture::read_frame_file(&path).map_err(|e| format!("{e:#}"));
                eprintln!("[rayshot] live frame read {:?}", started.elapsed());
                UserEvent::SessionFrame { id, result }
            }
            Some(Command::Cancel { id }) => UserEvent::Cancel { id },
            Some(Command::Select { id, from, to }) => UserEvent::Select { id, from, to },
            Some(Command::Quit) => break,
            None => {
                eprintln!("[rayshot] ignoring unknown command {line:?}");
                continue;
            }
        };
        if proxy.send_event(event).is_err() {
            return;
        }
    }
    let _ = proxy.send_event(UserEvent::Quit);
}

#[derive(Clone, Copy)]
enum SessionEnd {
    Closed,
    Failed,
}

#[derive(Clone, Copy)]
enum Action {
    Copy,
    CopyText,
    Save,
    SaveAs,
    Close,
}

#[derive(Clone, Debug, PartialEq)]
enum RebindNote {
    Conflict {
        target: crate::keys::Command,
        chord: crate::keys::Chord,
        other: crate::keys::Command,
    },
    Refused {
        target: crate::keys::Command,
        why: String,
    },
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
    replaces: Option<usize>,
}

struct OverlayApp {
    forwarded: bool,
    serve: bool,
    session: Option<u64>,
    live: bool,
    see_through: bool,
    frame_size: Option<(u32, u32)>,
    monitor_rects: Vec<crate::live::MonitorRect>,
    pending_export: Option<Action>,
    frame: Option<Arc<Frame>>,
    pending_frame: Option<Frame>,
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
    straight_from: Option<usize>,
    hidden: Vec<crate::scene::Tool>,
    hidden_buttons: Vec<crate::prefs::Button>,
    recent: Vec<egui::Color32>,
    bindings: crate::keys::Bindings,
    zoom: f32,
    pan_drag: bool,
    sel_zoom: f32,
    sel_offset: egui::Vec2,
    lens_drag: bool,
    rebinding: Option<crate::keys::Command>,
    held_after_capture: Option<KeyCode>,
    rebind_note: Option<RebindNote>,
    show_settings: bool,
    show_colours: bool,
    picker_hsv: Hsv,
    hex_text: String,
    scroll_acc: f32,
    toast: Option<(String, std::time::Instant)>,
    colour_copy: Option<std::thread::JoinHandle<()>>,
    pending: Option<Action>,
    sel_mode: Option<SelMode>,
    sel_ref: egui::Rect,
    text_edit: Option<TextDraft>,
    shape_drag: Option<(usize, Option<crate::scene::Grip>, crate::scene::Shape)>,
    selected: Option<usize>,
    text_sel: Option<(usize, usize, usize)>,
    blurred: Option<Arc<Vec<u8>>>,
}

impl OverlayApp {
    fn new(rt: Handle) -> Self {
        let prefs = crate::prefs::load();
        Self {
            forwarded: false,
            serve: false,
            session: None,
            live: false,
            see_through: false,
            frame_size: None,
            monitor_rects: Vec::new(),
            pending_export: None,
            frame: None,
            pending_frame: None,
            rt,
            shared: None,
            windows: Vec::new(),
            initialized: false,
            selection: None,
            drag_start: None,
            modifiers: winit::keyboard::ModifiersState::empty(),
            scene: crate::scene::Scene::default(),
            tool: crate::scene::Tool::Select,
            color: prefs.color.unwrap_or(egui::Color32::from_rgb(255, 60, 60)),
            stroke_width: prefs.width.unwrap_or(3.0),
            hidden: prefs.hidden,
            hidden_buttons: prefs.hidden_buttons,
            recent: prefs.recent,
            bindings: prefs.keys,
            zoom: 1.0,
            pan_drag: false,
            sel_zoom: 1.0,
            sel_offset: egui::Vec2::ZERO,
            lens_drag: false,
            rebinding: None,
            held_after_capture: None,
            rebind_note: None,
            show_settings: false,
            show_colours: false,
            picker_hsv: Hsv::of(egui::Color32::RED),
            hex_text: String::new(),
            draft: None,
            draft_start: None,
            straight_from: None,
            scroll_acc: 0.0,
            toast: None,
            colour_copy: None,
            pending: None,
            sel_mode: None,
            sel_ref: egui::Rect::ZERO,
            text_edit: None,
            shape_drag: None,
            selected: None,
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

const GRIP_REACH: f32 = 9.0;

fn selection_outline(
    shape: &crate::scene::Shape,
    to_screen: &dyn Fn(egui::Pos2) -> egui::Pos2,
    scale: f32,
) -> Option<egui::Rect> {
    let b = crate::scene::bounds(shape)?;
    Some(
        egui::Rect::from_two_pos(to_screen(b.min), to_screen(b.max))
            .expand(shape_width(shape).unwrap_or(0.0) * scale / 2.0 + 5.0),
    )
}

#[derive(Debug, PartialEq)]
enum TextOutcome {
    Added,
    Kept,
    Removed(usize),
    Nothing,
}

fn apply_text(scene: &mut crate::scene::Scene, d: TextDraft) -> TextOutcome {
    let text = (!d.buf.trim().is_empty()).then_some(crate::scene::Shape::Text {
        pos: d.pos,
        text: d.buf,
        color: d.color,
        size: d.size,
    });
    match (d.replaces, text) {
        (None, Some(text)) => {
            scene.push(text);
            TextOutcome::Added
        }
        (Some(i), Some(text)) if i < scene.shapes().len() => {
            if scene.shapes()[i] != text {
                scene.begin_change();
                scene.set_shape(i, text);
            }
            TextOutcome::Kept
        }
        (Some(i), None) if scene.remove(i) => TextOutcome::Removed(i),
        _ => TextOutcome::Nothing,
    }
}

fn launch_path(
    current: Option<std::path::PathBuf>,
    exists: &dyn Fn(&std::path::Path) -> bool,
) -> std::path::PathBuf {
    current
        .filter(|p| exists(p))
        .unwrap_or_else(|| std::path::PathBuf::from("/proc/self/exe"))
}

fn monitor_region(name: Option<&str>, rects: &[crate::live::MonitorRect]) -> Option<egui::Rect> {
    let m = rects.iter().find(|m| Some(m.name.as_str()) == name)?;
    Some(egui::Rect::from_min_size(
        egui::pos2(m.x as f32, m.y as f32),
        egui::vec2(m.width as f32, m.height as f32),
    ))
}

const ZOOM_STEPS: [f32; 6] = [1.0, 1.5, 2.0, 3.0, 4.0, 5.0];

fn step_zoom(current: f32, up: bool) -> f32 {
    if up {
        ZOOM_STEPS
            .into_iter()
            .find(|z| *z > current + 1e-3)
            .unwrap_or(ZOOM_STEPS[ZOOM_STEPS.len() - 1])
    } else {
        ZOOM_STEPS
            .into_iter()
            .rev()
            .find(|z| *z < current - 1e-3)
            .unwrap_or(ZOOM_STEPS[0])
    }
}

fn view_rect(region: egui::Rect, zoom: f32, center: egui::Pos2) -> egui::Rect {
    let size = region.size() / zoom.max(1.0);
    let half = size / 2.0;
    let c = egui::pos2(
        center.x.clamp(region.min.x + half.x, region.max.x - half.x),
        center.y.clamp(region.min.y + half.y, region.max.y - half.y),
    );
    egui::Rect::from_center_size(c, size)
}

fn zoom_center(region: egui::Rect, view: egui::Rect, anchor: egui::Pos2, zoom: f32) -> egui::Pos2 {
    let t = (anchor - view.min) / view.size();
    let size = region.size() / zoom.max(1.0);
    let min = anchor - t * size;
    min + size / 2.0
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Lens {
    outer: egui::Rect,
    inner: egui::Rect,
    z: f32,
}

impl Lens {
    fn new(outer: egui::Rect, z: f32, offset: egui::Vec2) -> Option<Lens> {
        if z <= 1.0 + 1e-3 {
            return None;
        }
        let size = outer.size() / z;
        let half = size / 2.0;
        let want = outer.center() + offset;
        let c = egui::pos2(
            want.x.clamp(outer.min.x + half.x, outer.max.x - half.x),
            want.y.clamp(outer.min.y + half.y, outer.max.y - half.y),
        );
        Some(Lens {
            outer,
            inner: egui::Rect::from_center_size(c, size),
            z,
        })
    }

    fn into_frame(&self, q: egui::Pos2) -> egui::Pos2 {
        self.inner.min + (q - self.outer.min) / self.z
    }

    fn out_of_frame(&self, f: egui::Pos2) -> egui::Pos2 {
        self.outer.min + (f - self.inner.min) * self.z
    }

    fn offset(&self) -> egui::Vec2 {
        self.inner.center() - self.outer.center()
    }
}

fn export_plan(
    sel: egui::Rect,
    dims: (u32, u32),
    sel_zoom: f32,
    offset: egui::Vec2,
    zoom: f32,
) -> (egui::Rect, (u32, u32)) {
    let (_, _, cw, ch) = crop_box(sel, dims.0, dims.1);
    let area = Lens::new(sel, sel_zoom, offset).map_or(sel, |l| l.inner);
    (area, scaled_size(cw as f32, ch as f32, zoom))
}

fn resized_to(img: image::RgbaImage, size: (u32, u32)) -> image::RgbaImage {
    if img.dimensions() == size {
        return img;
    }
    let filter = if size.0 >= img.width() {
        image::imageops::FilterType::CatmullRom
    } else {
        image::imageops::FilterType::Lanczos3
    };
    image::imageops::resize(&img, size.0, size.1, filter)
}

fn draws_frame(see_through: bool, zoom: f32, has_texture: bool) -> bool {
    has_texture && (!see_through || zoom > 1.0)
}

fn fit(avail: egui::Rect, view: egui::Rect) -> (f32, egui::Rect) {
    let scale = (avail.width() / view.width()).min(avail.height() / view.height());
    let image_rect = egui::Rect::from_center_size(avail.center(), view.size() * scale);
    (scale, image_rect)
}

const EXPORT_MAX_PIXELS: f32 = 40_000_000.0;

fn scaled_size(width: f32, height: f32, scale: f32) -> (u32, u32) {
    let room = (EXPORT_MAX_PIXELS / (width * height).max(1.0)).sqrt();
    if scale > 1.0 && scale > room {
        let side = |v: f32| (v * room.max(1.0)).floor().max(1.0) as u32;
        return (side(width), side(height));
    }
    let side = |v: f32| (v * scale).round().max(1.0) as u32;
    (side(width), side(height))
}

fn crop_box(sel: egui::Rect, width: u32, height: u32) -> (u32, u32, u32, u32) {
    let (fw, fh) = (width as f32, height as f32);
    let x = sel.min.x.floor().clamp(0.0, fw);
    let y = sel.min.y.floor().clamp(0.0, fh);
    let x2 = sel.max.x.ceil().clamp(0.0, fw);
    let y2 = sel.max.y.ceil().clamp(0.0, fh);
    (
        x as u32,
        y as u32,
        (x2 - x).max(0.0) as u32,
        (y2 - y).max(0.0) as u32,
    )
}

fn size_label(width: u32, height: u32, scale: f32) -> String {
    let base = format!("{width} × {height}");
    if (scale - 1.0).abs() < 1e-3 {
        return base;
    }
    let (w, h) = scaled_size(width as f32, height as f32, scale);
    format!("{base} → {w} × {h}")
}

fn spawn_detached(command: &str, file: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    let exe = launch_path(std::env::current_exe().ok(), &|p| p.exists());
    let mut cmd = std::process::Command::new(exe);
    cmd.arg(command)
        .arg(file)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null());
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    cmd.spawn().map(drop)
}

fn shifted_after_removal(idx: usize, removed: usize) -> Option<usize> {
    match idx.cmp(&removed) {
        std::cmp::Ordering::Less => Some(idx),
        std::cmp::Ordering::Equal => None,
        std::cmp::Ordering::Greater => Some(idx - 1),
    }
}

fn grip_cursor(grip: Option<crate::scene::Grip>, dragging: bool) -> egui::CursorIcon {
    match grip {
        Some(crate::scene::Grip::Box(h)) => handle_cursor(h),
        Some(_) => egui::CursorIcon::Crosshair,
        None if dragging => egui::CursorIcon::Grabbing,
        None => egui::CursorIcon::Move,
    }
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
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    _frame_texture: Option<wgpu::Texture>,
    frame_view: Option<wgpu::TextureView>,
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
    frame_texture_id: Option<egui::TextureId>,
    blur_texture_id: Option<egui::TextureId>,
    region: egui::Rect,
    configured: bool,
    suboptimal_reconfigured: bool,
    frames: u64,
    cursor: Option<egui::Pos2>,
    unrendered_input: bool,
    view_center: Option<egui::Pos2>,
}

impl ApplicationHandler<UserEvent> for OverlayApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.initialized {
            return;
        }
        self.initialized = true;
        if let Err(e) = self.ensure_device() {
            eprintln!("[rayshot] failed to init gpu device: {e:?}");
            event_loop.exit();
            return;
        }
        if self.serve {
            reply(&format!("ready {}", crate::live::PROTOCOL_VERSION));
            return;
        }
        if let Err(e) = self.create_windows(event_loop) {
            eprintln!("[rayshot] failed to create windows: {e:?}");
            event_loop.exit();
            return;
        }
        if let Some(frame) = self.pending_frame.take() {
            self.attach_frame(event_loop, frame);
        } else if self.live {
            self.show_live();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        use winit::event_loop::ControlFlow;
        match self.toast.as_ref().map(|(_, at)| *at + TOAST_FOR) {
            Some(end) if end > std::time::Instant::now() => {
                event_loop.set_control_flow(ControlFlow::WaitUntil(end));
            }
            Some(_) => {
                self.toast = None;
                self.request_redraw_all();
                event_loop.set_control_flow(ControlFlow::Wait);
            }
            None => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Show {
                id,
                width,
                height,
                monitors,
            } => {
                self.monitor_rects = monitors;
                self.start_session(event_loop, id, width, height);
            }
            UserEvent::Select { id, from, to } => {
                if self.serve && self.session != Some(id) {
                    return;
                }
                let Some((fw, fh)) = self.frame_dims() else {
                    return;
                };
                let rect = forwarded_selection(from, to, fw as f32, fh as f32);
                (self.selection, self.forwarded) =
                    next_selection(self.selection, self.forwarded, rect);
                self.request_redraw_all();
            }
            UserEvent::Cancel { id } => {
                if self.session == Some(id) {
                    eprintln!("[rayshot] session {id} cancelled");
                    self.close(event_loop);
                }
            }
            UserEvent::SessionFrame { id, result } => {
                if self.serve && self.session != Some(id) {
                    eprintln!(
                        "[rayshot] dropping frame for session {id}, current {:?}",
                        self.session
                    );
                    return;
                }
                self.user_event(event_loop, UserEvent::Captured(result));
            }
            UserEvent::Captured(Ok(frame)) => {
                if self.live && self.frame_size != Some((frame.width, frame.height)) {
                    eprintln!(
                        "[rayshot] live frame is {}x{}, expected {:?}",
                        frame.width, frame.height, self.frame_size
                    );
                    self.fail(event_loop);
                    return;
                }
                if self.initialized && self.shared.is_some() {
                    self.attach_frame(event_loop, frame);
                    match self.pending_export.take() {
                        Some(Action::Copy) => self.finish_and_exit(event_loop),
                        Some(Action::Save) => self.save_and_exit(event_loop),
                        Some(Action::SaveAs) => self.save_as_and_exit(event_loop),
                        Some(Action::CopyText) => self.copy_text_and_exit(event_loop),
                        _ => {}
                    }
                } else {
                    self.pending_frame = Some(frame);
                }
            }
            UserEvent::Captured(Err(e)) => {
                eprintln!("[rayshot] desktop capture failed: {e}");
                if self.live {
                    self.fail(event_loop);
                } else {
                    event_loop.exit();
                }
            }
            UserEvent::Quit => {
                eprintln!("[rayshot] exit: stdin closed");
                self.close(event_loop);
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(idx) = self.windows.iter().position(|w| w.window.id() == id) else {
            return;
        };

        if let WindowEvent::KeyboardInput { event: ke, .. } = &event
            && let PhysicalKey::Code(code) = ke.physical_key
            && self.held_after_capture == Some(code)
        {
            if !ke.state.is_pressed() {
                self.held_after_capture = None;
            } else if ke.repeat {
                return;
            } else {
                self.held_after_capture = None;
            }
        }

        if let WindowEvent::MouseInput {
            state: ElementState::Released,
            ..
        } = event
            && self.windows[idx].unrendered_input
        {
            if let Err(e) = self.render_window(idx) {
                eprintln!("[rayshot] render error: {e:?}");
            }
            self.windows[idx].unrendered_input = false;
        }

        {
            let win = &mut self.windows[idx];
            let resp = win.egui_state.on_window_event(&win.window, &event);
            if resp.repaint {
                win.window.request_redraw();
            }
            if matches!(
                event,
                WindowEvent::CursorMoved { .. }
                    | WindowEvent::MouseInput { .. }
                    | WindowEvent::MouseWheel { .. }
            ) {
                win.unrendered_input = true;
            }
        }

        match event {
            WindowEvent::CloseRequested => {
                eprintln!("[rayshot] exit: CloseRequested");
                self.close(event_loop);
            }
            WindowEvent::ModifiersChanged(m) => {
                self.modifiers = m.state();
                self.request_redraw_all();
            }
            WindowEvent::CursorLeft { .. } => {
                self.windows[idx].cursor = None;
                self.windows[idx].window.request_redraw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                let sf = self.windows[idx].window.scale_factor();
                let lp = position.to_logical::<f64>(sf);
                self.windows[idx].cursor = Some(egui::pos2(lp.x as f32, lp.y as f32));
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state.is_pressed() {
                    let typing_before = self.text_has_keyboard();
                    let drafting_before = self.text_edit.is_some();
                    self.catch_up();
                    if typing_before || self.text_has_keyboard() {
                        self.rebinding = None;
                    }
                    if let Some(action) = self.pending.take() {
                        self.run_action(event_loop, action);
                    } else if let Some(target) = self.rebinding {
                        self.capture_rebind(target, &event);
                    } else if typing_before || self.text_has_keyboard() {
                        self.request_redraw_all();
                    } else if drafting_before || self.text_edit.is_some() {
                        if matches!(event.logical_key, Key::Named(NamedKey::Escape)) {
                            self.commit_text();
                        }
                        self.request_redraw_all();
                    } else {
                        let ctrl = self.modifiers.control_key();
                        let shift = self.modifiers.shift_key();
                        let alt = self.modifiers.alt_key();
                        let code = match event.physical_key {
                            PhysicalKey::Code(c) => Some(c),
                            _ => None,
                        };
                        let is = |c: KeyCode| code == Some(c);
                        use crate::scene::Tool;
                        let dragging = self.shape_drag.is_some();
                        let selected = self
                            .selected
                            .filter(|_| self.tool == Tool::Select && !dragging);
                        let nudge = match event.logical_key {
                            Key::Named(NamedKey::ArrowLeft) => Some(egui::vec2(-1.0, 0.0)),
                            Key::Named(NamedKey::ArrowRight) => Some(egui::vec2(1.0, 0.0)),
                            Key::Named(NamedKey::ArrowUp) => Some(egui::vec2(0.0, -1.0)),
                            Key::Named(NamedKey::ArrowDown) => Some(egui::vec2(0.0, 1.0)),
                            _ => None,
                        };
                        let delete = matches!(
                            event.logical_key,
                            Key::Named(NamedKey::Delete | NamedKey::Backspace)
                        );
                        if self.rebind_note.is_some()
                            && matches!(event.logical_key, Key::Named(NamedKey::Escape))
                        {
                            self.rebind_note = None;
                            self.request_redraw_all();
                        } else if selected.is_some()
                            && (matches!(event.logical_key, Key::Named(NamedKey::Escape))
                                || (!ctrl
                                    && matches!(event.logical_key, Key::Named(NamedKey::Enter))))
                        {
                            self.selected = None;
                            self.scene.break_group();
                            self.request_redraw_all();
                        } else if let (Some(i), true, false) = (selected, delete, ctrl) {
                            self.scene.remove(i);
                            self.selected = None;
                            self.request_redraw_all();
                        } else if let (Some(i), Some(d), false) = (selected, nudge, ctrl) {
                            let step = if shift { 10.0 } else { 1.0 };
                            if let Some(shape) = self.scene.shapes().get(i) {
                                let moved = crate::scene::translated(shape, d * step);
                                self.scene.edit(i, crate::scene::Group::Nudge, moved);
                            }
                            self.request_redraw_all();
                        } else if let (Some(i), true) = (selected, ctrl && is(KeyCode::KeyD)) {
                            if let Some(copy) = self.scene.duplicate(i, DUPLICATE_OFFSET) {
                                self.selected = Some(copy);
                            }
                            self.request_redraw_all();
                        } else if matches!(event.logical_key, Key::Named(NamedKey::Escape)) {
                            eprintln!("[rayshot] exit: Escape (cancelled)");
                            self.close(event_loop);
                        } else if matches!(event.logical_key, Key::Named(NamedKey::Enter)) {
                            self.run_action(event_loop, Action::Copy);
                        } else if dragging
                            && ctrl
                            && (is(KeyCode::KeyZ) || is(KeyCode::KeyY) || is(KeyCode::KeyD))
                        {
                        } else if ctrl && is(KeyCode::KeyZ) && !shift {
                            self.scene.undo();
                            self.selected = None;
                            self.request_redraw_all();
                        } else if ctrl && (is(KeyCode::KeyY) || (is(KeyCode::KeyZ) && shift)) {
                            self.scene.redo();
                            self.selected = None;
                            self.request_redraw_all();
                        } else if let Some(command) =
                            code.and_then(|c| self.bindings.resolve(ctrl, shift, alt, c))
                        {
                            self.run_command(event_loop, command);
                        } else if !ctrl {
                            let digit = [
                                KeyCode::Digit1,
                                KeyCode::Digit2,
                                KeyCode::Digit3,
                                KeyCode::Digit4,
                                KeyCode::Digit5,
                                KeyCode::Digit6,
                                KeyCode::Digit7,
                                KeyCode::Digit8,
                                KeyCode::Digit9,
                            ]
                            .iter()
                            .position(|&k| is(k));
                            if let Some(c) = digit
                                .filter(|_| !shift)
                                .and_then(|d| palette_color(d as u32 + 1))
                            {
                                self.color = c;
                                if let Some(i) = selected {
                                    self.recolor_selected(i);
                                }
                                self.request_redraw_all();
                            }
                        }
                    }
                }
            }
            WindowEvent::Resized(size) => {
                let win = &self.windows[idx];
                if surface_needs_configure(
                    win.configured,
                    (win.config.width, win.config.height),
                    (size.width, size.height),
                ) {
                    if let Some(shared) = self.shared.as_ref() {
                        let win = &mut self.windows[idx];
                        win.config.width = size.width;
                        win.config.height = size.height;
                        win.surface.configure(&shared.device, &win.config);
                        win.configured = true;
                        win.window.request_redraw();
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                if let Err(e) = self.render_window(idx) {
                    eprintln!("[rayshot] render error: {e:?}");
                }
                self.windows[idx].unrendered_input = false;
                if !self.windows.iter().any(|w| w.unrendered_input)
                    && let Some(action) = self.pending.take()
                {
                    self.run_action(event_loop, action);
                }
            }
            _ => {}
        }
    }
}

impl OverlayApp {
    fn ensure_device(&mut self) -> Result<()> {
        if self.shared.is_some() {
            return Ok(());
        }
        let started = std::time::Instant::now();
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = self
            .rt
            .block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
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
        self.shared = Some(Shared {
            instance,
            adapter,
            device,
            queue,
            _frame_texture: None,
            frame_view: None,
            _blur_texture: None,
            blur_view: None,
        });
        eprintln!("[rayshot] gpu device init: {:?}", started.elapsed());
        Ok(())
    }

    fn create_windows(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        let started = std::time::Instant::now();
        let windowed = std::env::var_os("RAYSHOT_WINDOWED").is_some();

        let mut targets: Vec<(Arc<Window>, egui::Rect)> = Vec::new();
        if windowed {
            let attrs = Window::default_attributes()
                .with_title("rayshot")
                .with_visible(false)
                .with_window_level(winit::window::WindowLevel::AlwaysOnTop)
                .with_inner_size(winit::dpi::LogicalSize::new(1600.0, 900.0));
            let window = Arc::new(event_loop.create_window(attrs).context("create window")?);
            targets.push((window, egui::Rect::ZERO));
        } else {
            for monitor in event_loop.available_monitors() {
                let pos = monitor.position();
                let size = monitor.size();
                let region = monitor_region(monitor.name().as_deref(), &self.monitor_rects)
                    .unwrap_or_else(|| {
                        egui::Rect::from_min_size(
                            egui::pos2(pos.x as f32, pos.y as f32),
                            egui::vec2(size.width as f32, size.height as f32),
                        )
                    });
                let attrs = Window::default_attributes()
                    .with_title("rayshot")
                    .with_visible(false)
                    .with_transparent(self.live)
                    .with_decorations(false)
                    .with_fullscreen(Some(Fullscreen::Borderless(Some(monitor))));
                let window = Arc::new(event_loop.create_window(attrs).context("create window")?);
                targets.push((window, region));
            }
        }
        if targets.is_empty() {
            anyhow::bail!("no monitors/windows to display on");
        }

        let shared = self.shared.as_ref().context("device not initialized")?;
        let mut windows = Vec::with_capacity(targets.len());
        let mut see_through = self.live;
        for (window, region) in targets {
            let surface = shared
                .instance
                .create_surface(window.clone())
                .context("failed to create surface")?;
            let size = window.inner_size();
            let mut config = surface
                .get_default_config(&shared.adapter, size.width.max(1), size.height.max(1))
                .context("surface not supported by adapter")?;
            if self.live {
                let modes = surface.get_capabilities(&shared.adapter).alpha_modes;
                match see_through_alpha(&modes) {
                    Some(mode) => config.alpha_mode = mode,
                    None => {
                        eprintln!("[rayshot] no see-through alpha ({modes:?}), drawing the frame");
                        see_through = false;
                    }
                }
            }

            let egui_ctx = egui::Context::default();
            egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
            let mut fonts = egui::FontDefinitions::default();
            egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
            egui_ctx.set_fonts(fonts);
            let egui_state = egui_winit::State::new(
                egui_ctx.clone(),
                egui_ctx.viewport_id(),
                &window,
                Some(window.scale_factor() as f32),
                None,
                Some(shared.device.limits().max_texture_dimension_2d as usize),
            );
            let renderer = Renderer::new(&shared.device, config.format, RendererOptions::default());

            windows.push(WindowState {
                window,
                surface,
                config,
                egui_ctx,
                egui_state,
                renderer,
                frame_texture_id: None,
                blur_texture_id: None,
                region,
                configured: false,
                suboptimal_reconfigured: false,
                frames: 0,
                cursor: None,
                unrendered_input: false,
                view_center: None,
            });
        }
        self.windows = windows;
        self.see_through = see_through;
        eprintln!("[rayshot] windows created: {:?}", started.elapsed());
        Ok(())
    }

    fn frame_dims(&self) -> Option<(u32, u32)> {
        self.frame
            .as_ref()
            .map(|f| (f.width, f.height))
            .or(self.frame_size)
    }

    fn restore_selection(&mut self, fw: f32, fh: f32) {
        if self.selection.is_none() && !self.forwarded {
            self.selection = crate::export::load_last_selection().and_then(|(x, y, w, h)| {
                let r = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h));
                (r.min.x >= 0.0 && r.min.y >= 0.0 && r.max.x <= fw && r.max.y <= fh).then_some(r)
            });
        }
    }

    fn show_live(&mut self) {
        if let Some((w, h)) = self.frame_size {
            self.restore_selection(w as f32, h as f32);
        }
        for win in &self.windows {
            win.window.set_visible(true);
            win.window.request_redraw();
        }
    }

    fn attach_frame(&mut self, event_loop: &ActiveEventLoop, frame: Frame) {
        let attach_started = std::time::Instant::now();
        let frame = Arc::new(frame);
        let (fw, fh) = (frame.width as f32, frame.height as f32);
        self.frame = Some(frame.clone());
        self.restore_selection(fw, fh);

        if std::env::var_os("RAYSHOT_WINDOWED").is_some() {
            let region = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(fw, fh));
            for win in &mut self.windows {
                win.region = region;
            }
        }

        if let Some(shared) = self.shared.as_mut() {
            let extent = wgpu::Extent3d {
                width: frame.width,
                height: frame.height,
                depth_or_array_layers: 1,
            };
            let tex = shared.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("capture frame"),
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
                &frame.rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * frame.width),
                    rows_per_image: Some(frame.height),
                },
                extent,
            );
            let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
            shared._frame_texture = Some(tex);
            shared.frame_view = Some(view);
        }
        if let Some(shared) = self.shared.as_ref() {
            if let Some(view) = shared.frame_view.as_ref() {
                for win in &mut self.windows {
                    win.frame_texture_id = Some(win.renderer.register_native_texture(
                        &shared.device,
                        view,
                        wgpu::FilterMode::Linear,
                    ));
                }
            }
        }

        self.seed_test_env(event_loop, &frame);

        for win in &self.windows {
            win.window.set_visible(true);
            win.window.request_redraw();
        }
        eprintln!("[rayshot] attach frame: {:?}", attach_started.elapsed());
    }

    fn seed_test_env(&mut self, event_loop: &ActiveEventLoop, frame: &Frame) {
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
                        replaces: None,
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
                    &frame.rgba,
                    frame.width,
                    frame.height,
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
                Ok(Some(path)) => eprintln!("[rayshot] test kept a copy at {}", path.display()),
                Ok(None) => eprintln!("[rayshot] test: empty selection"),
                Err(e) => eprintln!("[rayshot] test finish failed: {e:?}"),
            }
            crate::anim::force_restore();
            unsafe { libc::_exit(0) };
        }
        if std::env::var_os("RAYSHOT_TEST_QUIT").is_some() {
            self.close(event_loop);
        }
    }

    fn render_window(&mut self, idx: usize) -> Result<()> {
        if self.shared.is_none() {
            return Ok(());
        }
        if !self.windows[idx].configured {
            return Ok(());
        }
        let Some((frame_wu, frame_hu)) = self.frame_dims() else {
            return Ok(());
        };
        let frame_w = frame_wu as f32;
        let frame_h = frame_hu as f32;
        let live = self.live;
        let see_through = self.see_through;
        let frame_px = self.frame.clone();
        if let Some(frame) = frame_px.as_ref() {
            if matches!(self.tool, crate::scene::Tool::Blur) && self.blurred.is_none() {
                let b = crate::scene::blur_frame(&frame.rgba, frame.width, frame.height);
                self.blurred = Some(Arc::new(b));
            }
        }
        if let (Some(b), Some(shared)) = (self.blurred.as_ref(), self.shared.as_mut()) {
            if shared.blur_view.is_none() {
                let extent = wgpu::Extent3d {
                    width: frame_wu,
                    height: frame_hu,
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
                        bytes_per_row: Some(4 * frame_wu),
                        rows_per_image: Some(frame_hu),
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
        let view = view_rect(
            region,
            self.zoom,
            win.view_center.unwrap_or(region.center()),
        );
        let mut view_center = win.view_center;
        let mut pan_drag = self.pan_drag;
        let mut sel_zoom = self.sel_zoom;
        let mut sel_offset = self.sel_offset;
        let mut lens_drag = self.lens_drag;
        let lens = self
            .selection
            .and_then(|sel| Lens::new(sel, sel_zoom, sel_offset));
        let bounds = if self.zoom > 1.0 {
            view
        } else {
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(f32::MAX, f32::MAX))
        };
        let frame_tex = win.frame_texture_id;
        let see_through = see_through && !draws_frame(see_through, self.zoom, frame_tex.is_some());
        if frame_tex.is_none() && !live {
            return Ok(());
        }
        let blur_tex = win.blur_texture_id.or(frame_tex);
        let raw_cursor = win.cursor;
        let ctx = win.egui_ctx.clone();
        let raw_input = win.egui_state.take_egui_input(&win.window);

        use crate::scene::{Shape, Tool};

        let mut selection = self.selection;
        let mut drag_start = self.drag_start;
        let mut draft = self.draft.clone();
        let mut draft_start = self.draft_start;
        let mut straight_from = self.straight_from;
        let shift = self.modifiers.shift_key();
        let frame_rect =
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(frame_w, frame_h));
        let mut tool = self.tool;
        let mut color = self.color;
        let mut stroke_width = self.stroke_width;
        let mut scroll_acc = self.scroll_acc;
        let mut hidden = self.hidden.clone();
        let mut hidden_buttons = self.hidden_buttons.clone();
        let zoom = self.zoom;
        let frame_dims = self.frame_dims().unwrap_or((u32::MAX, u32::MAX));
        let mut bindings = self.bindings.clone();
        let mut rebinding = self.rebinding;
        let mut rebind_note = self.rebind_note.clone();
        let mut show_settings = self.show_settings;
        let mut show_colours = self.show_colours;
        let mut picker_hsv = self.picker_hsv;
        let mut hex_text = self.hex_text.clone();
        let recent = self.recent.clone();
        let mut picked: Option<egui::Color32> = None;
        let toast = self
            .toast
            .as_ref()
            .filter(|(_, at)| at.elapsed() < TOAST_FOR)
            .map(|(text, _)| text.clone());
        let width_before = stroke_width;
        let shapes = self.scene.shapes();
        let mut committed: Option<Shape> = None;
        let mut clear_scene = false;
        let mut action: Option<Action> = None;
        let mut sel_mode = self.sel_mode;
        let mut sel_ref = self.sel_ref;
        let mut shape_drag = self.shape_drag.clone();
        let mut drag_apply: Option<(usize, Shape)> = None;
        let tool_at_start = self.tool;
        let color_at_start = color;
        let selected_at_start = self.selected.filter(|&i| i < shapes.len());
        let mut selected = selected_at_start;
        let mut wheel_edit: Option<(usize, Shape)> = None;
        let mut size_toast: Option<f32> = None;
        let mut reedit_text: Option<usize> = None;
        let editing_index = self.text_edit.as_ref().and_then(|d| d.replaces);
        let mut start_text: Option<egui::Pos2> = None;
        let mut text_buf = self.text_edit.as_ref().map(|d| d.buf.clone());
        let text_info = self.text_edit.as_ref().map(|d| (d.pos, d.color, d.size));
        let mut text_lost_focus = false;
        let mut text_pos_shift = egui::Vec2::ZERO;
        let text_sel_prev = self.text_sel;
        let mut owner_text_sel: Option<(usize, usize, usize)> = None;

        let full_output = ctx.run_ui(raw_input, |ui| {
            let avail = ui.max_rect();
            if rebinding.is_some()
                && text_info.is_none()
                && let Some(id) = ui.ctx().memory(|m| m.focused())
            {
                ui.ctx().memory_mut(|m| m.surrender_focus(id));
            }

            let (scale, image_rect) = fit(avail, view);

            let plain_to_frame = |p: egui::Pos2| {
                let local = (p - image_rect.min) / scale;
                egui::pos2(
                    (view.min.x + local.x)
                        .clamp(bounds.min.x, bounds.max.x)
                        .clamp(0.0, frame_w),
                    (view.min.y + local.y)
                        .clamp(bounds.min.y, bounds.max.y)
                        .clamp(0.0, frame_h),
                )
            };
            let plain_to_screen = |fp: egui::Pos2| image_rect.min + (fp - view.min) * scale;

            let uv = egui::Rect::from_min_max(
                egui::pos2(view.min.x / frame_w, view.min.y / frame_h),
                egui::pos2(view.max.x / frame_w, view.max.y / frame_h),
            );
            if let (false, Some(texture_id)) = (see_through, frame_tex) {
                ui.painter()
                    .image(texture_id, image_rect, uv, egui::Color32::WHITE);
            }

            let resp = ui.interact(
                image_rect,
                egui::Id::new("rayshot-canvas"),
                egui::Sense::click_and_drag(),
            );
            if resp.drag_started() {
                pan_drag = ui.input(|i| {
                    i.pointer.button_down(egui::PointerButton::Middle) && !i.pointer.primary_down()
                });
            }
            let panning = pan_drag && (resp.drag_started() || resp.dragged() || resp.drag_stopped());
            let cursor_pos = raw_cursor.or_else(|| resp.interact_pointer_pos());
            let press_pos = ui.input(|i| i.pointer.press_origin()).or(cursor_pos);
            let hit = |fp: egui::Pos2| {
                crate::scene::shape_hit(shapes, fp).filter(|&i| Some(i) != editing_index)
            };
            let selected_before = selected.filter(|&i| Some(i) != editing_index);
            let capture_before = selection;
            let on_capture_handle = |p: egui::Pos2| {
                capture_before.is_some_and(|sel| {
                    let s = egui::Rect::from_min_max(plain_to_screen(sel.min), plain_to_screen(sel.max));
                    let hr = 12.0_f32.min(s.width().min(s.height()) / 3.0);
                    hit_handle(s, p, hr).is_some()
                })
            };
            let lens_area = lens.map(|l| {
                egui::Rect::from_min_max(plain_to_screen(l.outer.min), plain_to_screen(l.outer.max))
            });
            let in_lens =
                |p: egui::Pos2| lens_area.is_some_and(|r| r.contains(p)) && !on_capture_handle(p);
            if resp.drag_started() {
                lens_drag = press_pos.is_some_and(in_lens);
            }
            let dragging = resp.drag_started() || resp.dragged() || resp.drag_stopped();
            let lensed = if dragging {
                lens_drag
            } else {
                cursor_pos.is_some_and(in_lens)
            };
            if panning && resp.dragged() {
                if let (true, Some(l)) = (lens_drag, lens) {
                    let shifted = l.offset() - resp.drag_delta() / (scale * l.z);
                    if let Some(moved) = Lens::new(l.outer, l.z, shifted) {
                        sel_offset = moved.offset();
                    }
                } else if zoom > 1.0 {
                    let moved = view.center() - resp.drag_delta() / scale;
                    view_center = Some(view_rect(region, zoom, moved).center());
                }
            }
            let active_lens = lens.filter(|_| lensed);
            let tool_frame = |p: egui::Pos2| {
                let q = plain_to_frame(p);
                active_lens.map_or(q, |l| l.into_frame(q))
            };
            let tool_screen =
                |f: egui::Pos2| plain_to_screen(active_lens.map_or(f, |l| l.out_of_frame(f)));
            let tool_scale = active_lens.map_or(scale, |l| scale * l.z);
            let inside_selected = |p: egui::Pos2| {
                selected_before.filter(|&i| {
                    !on_capture_handle(p)
                        && selection_outline(&shapes[i], &tool_screen, tool_scale)
                            .is_some_and(|r| r.contains(p))
                })
            };
            match tool {
                _ if panning => {}
                Tool::Select => {
                    if resp.clicked()
                        && let Some(p) = cursor_pos
                    {
                        selected = hit(tool_frame(p)).or_else(|| inside_selected(p));
                    }
                    if resp.double_clicked()
                        && let Some(i) = selected
                        && matches!(shapes[i], Shape::Text { .. })
                    {
                        reedit_text = Some(i);
                    }
                    if resp.drag_started() {
                        if let Some(p) = press_pos {
                            let fp = tool_frame(p);
                            drag_start = Some(fp);
                            let grip = selected.and_then(|i| {
                                crate::scene::grips(&shapes[i])
                                    .into_iter()
                                    .find(|(_, at)| (tool_screen(*at) - p).length() <= GRIP_REACH)
                                    .map(|(g, _)| (i, g))
                            });
                            if let Some((i, g)) = grip {
                                shape_drag = Some((i, Some(g), shapes[i].clone()));
                                sel_mode = None;
                            } else if let Some(i) = inside_selected(p).or_else(|| hit(fp)) {
                                selected = Some(i);
                                shape_drag = Some((i, None, shapes[i].clone()));
                                sel_mode = None;
                            } else if let Some(sel) = selection {
                                selected = None;
                                drag_start = Some(plain_to_frame(p));
                                lens_drag = false;
                                let s = egui::Rect::from_min_max(
                                    plain_to_screen(sel.min),
                                    plain_to_screen(sel.max),
                                );
                                let hr = 12.0_f32.min(s.width().min(s.height()) / 3.0);
                                if let Some(h) = hit_handle(s, p, hr) {
                                    sel_mode = Some(SelMode::Resize(h));
                                    sel_ref = sel;
                                } else if sel.contains(plain_to_frame(p)) {
                                    sel_mode = Some(SelMode::Move);
                                    sel_ref = sel;
                                } else {
                                    sel_mode = Some(SelMode::New);
                                    clear_scene = true;
                                    sel_zoom = 1.0;
                                    sel_offset = egui::Vec2::ZERO;
                                }
                            } else {
                                sel_mode = Some(SelMode::New);
                            }
                        }
                    }
                    if resp.dragged() || resp.drag_stopped() {
                        if let (Some(start), Some(p)) = (drag_start, cursor_pos) {
                            let fp = tool_frame(p);
                            if let Some((i, grip, orig)) = &shape_drag {
                                let moved = match grip {
                                    None => crate::scene::translated(orig, fp - start),
                                    Some(g) => reshaped(orig, *g, fp, shift, frame_rect),
                                };
                                drag_apply = Some((*i, moved));
                            } else {
                                let fp = plain_to_frame(p);
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
                        shape_drag = None;
                    }
                    let hover = resp.hover_pos();
                    let grip_under = |p: egui::Pos2| {
                        selected.and_then(|i| {
                            crate::scene::grips(&shapes[i])
                                .into_iter()
                                .find(|(_, at)| (tool_screen(*at) - p).length() <= GRIP_REACH)
                                .map(|(g, _)| g)
                        })
                    };
                    let icon = if let Some((_, grip, _)) = &shape_drag {
                        grip_cursor(*grip, true)
                    } else if let Some(g) = hover.and_then(grip_under) {
                        grip_cursor(Some(g), false)
                    } else if sel_mode.is_none()
                        && hover.is_some_and(|p| {
                            inside_selected(p).is_some() || hit(tool_frame(p)).is_some()
                        })
                    {
                        egui::CursorIcon::Move
                    } else if let Some(sel) = selection {
                        let s =
                            egui::Rect::from_min_max(plain_to_screen(sel.min), plain_to_screen(sel.max));
                        let hr = 12.0_f32.min(s.width().min(s.height()) / 3.0);
                        match sel_mode {
                            Some(SelMode::Resize(h)) => handle_cursor(h),
                            Some(SelMode::Move) => egui::CursorIcon::Grabbing,
                            _ => match resp.hover_pos() {
                                Some(p) if hit_handle(s, p, hr).is_some() => {
                                    handle_cursor(hit_handle(s, p, hr).unwrap())
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
                Tool::Counter => {
                    if (resp.clicked() || resp.drag_stopped())
                        && let Some(p) = cursor_pos
                    {
                        committed = Some(Shape::Counter {
                            center: tool_frame(p),
                            number: crate::scene::next_counter(shapes),
                            color,
                            radius: counter_radius(stroke_width),
                        });
                    }
                }
                Tool::Picker => {
                    if (resp.clicked() || resp.drag_stopped())
                        && let (Some(p), Some(frame)) = (cursor_pos, frame_px.as_deref())
                        && let Some(c) = pixel_color(frame, tool_frame(p))
                    {
                        picked = Some(c);
                    }
                }
                Tool::Text => {
                    if resp.clicked() {
                        if let Some(p) = cursor_pos {
                            start_text = Some(tool_frame(p));
                        }
                    }
                }
                _ => {
                    if resp.drag_started() {
                        if let Some(p) = press_pos {
                            let fp = tool_frame(p);
                            draft_start = Some(fp);
                            straight_from = None;
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
                                    if let Some(frame) = frame_px.as_deref() {
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
                                }
                                Tool::Blur => {
                                    if frame_px.is_some() {
                                        draft = Some(Shape::Blur {
                                            points: vec![fp],
                                            radius: crate::scene::BLUR_BRUSH,
                                        });
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    if resp.dragged() || resp.drag_stopped() {
                        if let Some(p) = cursor_pos {
                            let fp = tool_frame(p);
                            match tool {
                                Tool::Rect => {
                                    if let Some(st) = draft_start {
                                        draft = Some(Shape::Rect {
                                            rect: egui::Rect::from_two_pos(
                                                st,
                                                constrain(tool, st, fp, shift, frame_rect),
                                            ),
                                            color,
                                            width: stroke_width,
                                        });
                                    }
                                }
                                Tool::Ellipse => {
                                    if let Some(st) = draft_start {
                                        draft = Some(Shape::Ellipse {
                                            rect: egui::Rect::from_two_pos(
                                                st,
                                                constrain(tool, st, fp, shift, frame_rect),
                                            ),
                                            color,
                                            width: stroke_width,
                                        });
                                    }
                                }
                                Tool::Line => {
                                    if let Some(st) = draft_start {
                                        draft = Some(Shape::Line {
                                            from: st,
                                            to: constrain(tool, st, fp, shift, frame_rect),
                                            color,
                                            width: stroke_width,
                                        });
                                    }
                                }
                                Tool::Arrow => {
                                    if let Some(st) = draft_start {
                                        draft = Some(Shape::Arrow {
                                            from: st,
                                            to: constrain(tool, st, fp, shift, frame_rect),
                                            color,
                                            width: stroke_width,
                                        });
                                    }
                                }
                                Tool::Pen | Tool::Marker => {
                                    if let Some(Shape::Pen { points, .. }) = &mut draft {
                                        extend_stroke(
                                            tool,
                                            points,
                                            &mut straight_from,
                                            fp,
                                            shift,
                                            frame_rect,
                                        );
                                    }
                                }
                                Tool::Pixelate => {
                                    if let (Some(Shape::Pixelate { cell, cells }), Some(frame)) =
                                        (&mut draft, frame_px.as_deref())
                                    {
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
                                    if let Some(Shape::Blur { points, .. }) = &mut draft {
                                        points.push(fp);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    if resp.drag_stopped() {
                        committed = draft.take();
                        draft_start = None;
                        straight_from = None;
                    }
                }
            }

            let brush_ring = match tool {
                Tool::Pen | Tool::Line => Some((stroke_width * 0.5).max(5.0)),
                Tool::Marker => Some(8.0),
                Tool::Pixelate => Some(crate::scene::PIXEL_BRUSH),
                Tool::Blur => Some(crate::scene::BLUR_BRUSH),
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

            let wheel_target = selected.filter(|_| tool == Tool::Select && shape_drag.is_none());
            if (wheel_target.is_some()
                || matches!(
                    tool,
                    Tool::Pen
                        | Tool::Line
                        | Tool::Arrow
                        | Tool::Rect
                        | Tool::Ellipse
                        | Tool::Counter
                ))
                && resp.hovered()
            {
                let dy: f32 = ui.input(|i| {
                    i.raw
                        .events
                        .iter()
                        .map(|e| match e {
                            egui::Event::MouseWheel { unit, delta, .. } => match unit {
                                egui::MouseWheelUnit::Point => delta.y,
                                egui::MouseWheelUnit::Line => delta.y * SCROLL_STEP,
                                egui::MouseWheelUnit::Page => delta.y * SCROLL_STEP * 3.0,
                            },
                            _ => 0.0,
                        })
                        .sum()
                });
                if dy != 0.0 {
                    match wheel_target.map(|i| (i, &shapes[i])) {
                        Some((i, Shape::Text { size, .. })) => {
                            let new = scrolled_text_size(*size, &mut scroll_acc, dy);
                            if new != *size
                                && let Shape::Text {
                                    pos, text, color, ..
                                } = &shapes[i]
                            {
                                size_toast = Some(new);
                                wheel_edit = Some((
                                    i,
                                    Shape::Text {
                                        pos: *pos,
                                        text: text.clone(),
                                        color: *color,
                                        size: new,
                                    },
                                ));
                            }
                        }
                        Some((i, shape)) => {
                            if let Some(w) = shape_width(shape) {
                                stroke_width = scrolled_width(w, &mut scroll_acc, dy);
                                if stroke_width != w {
                                    wheel_edit = Some((i, with_width(shape, stroke_width)));
                                }
                            }
                        }
                        None => {
                            stroke_width = scrolled_width(stroke_width, &mut scroll_acc, dy);
                        }
                    }
                }
            }

            let painter = ui.painter();
            let dim_alpha = if std::env::var_os("RAYSHOT_NODIM").is_some() {
                0
            } else {
                150
            };
            let dim = egui::Color32::from_black_alpha(dim_alpha);

            let chrome = |p: &egui::Painter, s_true: egui::Rect| {
                let corners = [
                    s_true.left_top(),
                    s_true.right_top(),
                    s_true.right_bottom(),
                    s_true.left_bottom(),
                    s_true.left_top(),
                ];
                p.extend(egui::Shape::dashed_line(
                    &corners,
                    egui::Stroke::new(1.5f32, egui::Color32::WHITE),
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
                    p.rect_filled(hr, 1, egui::Color32::WHITE);
                    p.rect_stroke(
                        hr,
                        1,
                        egui::Stroke::new(1.0f32, egui::Color32::from_gray(70)),
                        egui::StrokeKind::Outside,
                    );
                }
            };

            let visible = selection.and_then(|sel| {
                let v = sel.intersect(region);
                (v.width() > 0.5 && v.height() > 0.5).then_some((sel, v))
            });

            if let Some((sel, vis)) = visible {
                let s = egui::Rect::from_min_max(plain_to_screen(vis.min), plain_to_screen(vis.max));
                let s_true = egui::Rect::from_min_max(plain_to_screen(sel.min), plain_to_screen(sel.max));
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
                chrome(painter, s_true);
                if region.contains(sel.min) {
                    let (_, _, cw, ch) = crop_box(sel, frame_dims.0, frame_dims.1);
                    let label = size_label(cw, ch, zoom);
                    let label = if sel_zoom > 1.0 {
                        format!("{label} · {} %", (sel_zoom * 100.0).round())
                    } else {
                        label
                    };
                    let galley = painter.layout_no_wrap(
                        label,
                        egui::FontId::proportional(12.0),
                        egui::Color32::WHITE,
                    );
                    let pad = egui::vec2(6.0, 3.0);
                    let pill_size = galley.size() + 2.0 * pad;
                    let pill_min = egui::pos2(
                        s_true
                            .left()
                            .min(avail.right() - pill_size.x)
                            .max(avail.left()),
                        (s_true.top() - pill_size.y - 6.0)
                            .min(avail.bottom() - pill_size.y)
                            .max(avail.top()),
                    );
                    let pill = egui::Rect::from_min_size(pill_min, pill_size);
                    painter.rect_filled(pill, 4, egui::Color32::from_black_alpha(190));
                    painter.galley(pill_min + pad, galley, egui::Color32::WHITE);
                }
            } else {
                painter.rect_filled(image_rect, 0, dim);
            }

            let paint_shape = |shape: &Shape| match (blur_tex, shape) {
                (None, Shape::Blur { .. }) => {}
                (tex, _) => crate::scene::paint(
                    painter,
                    shape,
                    &plain_to_screen,
                    scale,
                    tex.unwrap_or_default(),
                    frame_w,
                    frame_h,
                ),
            };
            let shape_now = |i: usize| {
                [&drag_apply, &wheel_edit]
                    .into_iter()
                    .flatten()
                    .find(|(j, _)| *j == i)
                    .map_or(&shapes[i], |(_, s)| s)
            };
            for i in 0..shapes.len() {
                if Some(i) != editing_index {
                    paint_shape(shape_now(i));
                }
            }
            if let Some(d) = &draft {
                paint_shape(d);
            }
            if let Some(c) = &committed {
                paint_shape(c);
            }
            let lens_now = selection.and_then(|sel| Lens::new(sel, sel_zoom, sel_offset));
            if let (Some(l), Some(tex)) = (lens_now, frame_tex) {
                let r = egui::Rect::from_min_max(
                    plain_to_screen(l.outer.min),
                    plain_to_screen(l.outer.max),
                );
                let lp = painter.with_clip_rect(r.intersect(avail));
                let luv = egui::Rect::from_min_max(
                    egui::pos2(l.inner.min.x / frame_w, l.inner.min.y / frame_h),
                    egui::pos2(l.inner.max.x / frame_w, l.inner.max.y / frame_h),
                );
                lp.image(tex, r, luv, egui::Color32::WHITE);
                let ls = |f: egui::Pos2| plain_to_screen(l.out_of_frame(f));
                let paint_lensed = |shape: &Shape| match (blur_tex, shape) {
                    (None, Shape::Blur { .. }) => {}
                    (tex, _) => crate::scene::paint(
                        &lp,
                        shape,
                        &ls,
                        scale * l.z,
                        tex.unwrap_or_default(),
                        frame_w,
                        frame_h,
                    ),
                };
                for i in 0..shapes.len() {
                    if Some(i) != editing_index {
                        paint_lensed(shape_now(i));
                    }
                }
                if let Some(d) = &draft {
                    paint_lensed(d);
                }
                if let Some(c) = &committed {
                    paint_lensed(c);
                }
                chrome(painter, r);
            }
            if let Some(i) = selected.filter(|&i| tool == Tool::Select && Some(i) != editing_index)
            {
                let shape = shape_now(i);
                let owner = lens_now.filter(|l| {
                    selection_outline(shape, &|f: egui::Pos2| f, 1.0)
                        .is_some_and(|r| l.inner.contains(r.center()))
                });
                let os = |f: egui::Pos2| plain_to_screen(owner.map_or(f, |l| l.out_of_frame(f)));
                let oscale = owner.map_or(scale, |l| scale * l.z);
                if let Some(r) = selection_outline(shape, &os, oscale) {
                    let outline = [
                        r.left_top(),
                        r.right_top(),
                        r.right_bottom(),
                        r.left_bottom(),
                        r.left_top(),
                    ];
                    painter.extend(egui::Shape::dashed_line(
                        &outline,
                        egui::Stroke::new(2.5f32, egui::Color32::from_black_alpha(110)),
                        5.0,
                        4.0,
                    ));
                    painter.extend(egui::Shape::dashed_line(
                        &outline,
                        egui::Stroke::new(1.0f32, egui::Color32::WHITE),
                        5.0,
                        4.0,
                    ));
                }
                for (_, at) in crate::scene::grips(shape) {
                    let c = os(at);
                    painter.circle_filled(c, 5.0, egui::Color32::WHITE);
                    painter.circle_stroke(
                        c,
                        5.0,
                        egui::Stroke::new(1.0f32, egui::Color32::from_gray(60)),
                    );
                }
            }
            if let (Some(r), Some(p)) = (brush_ring, raw_cursor) {
                if brush_active {
                    let rs = r * tool_scale;
                    painter.circle_stroke(
                        p,
                        rs,
                        egui::Stroke::new(2.5f32, egui::Color32::from_black_alpha(130)),
                    );
                    painter.circle_stroke(
                        p,
                        rs,
                        egui::Stroke::new(1.2f32, egui::Color32::from_white_alpha(235)),
                    );
                    painter.circle_filled(p, 1.4, egui::Color32::from_white_alpha(235));
                }
            }
            let text_lens = text_info.and_then(|(pos, _, _)| lens_now.filter(|l| l.inner.contains(pos)));
            let text_screen =
                |f: egui::Pos2| plain_to_screen(text_lens.map_or(f, |l| l.out_of_frame(f)));
            let tscale = text_lens.map_or(scale, |l| scale * l.z);
            if let (Some(buf), Some((pos, col, tsize))) = (&text_buf, text_info) {
                if !region.contains(pos) {
                    let font = egui::FontId::proportional(tsize * tscale);
                    let galley = painter.layout_no_wrap(buf.clone(), font, col);
                    let gsize = galley.size();
                    let line_h = gsize.y.max(tsize * tscale);
                    let origin = text_screen(pos);
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
                        egui::Stroke::new(1.0f32, egui::Color32::from_white_alpha(160)),
                        4.0,
                        3.0,
                    ));
                }
            }
            if let (Some(buf), Some((pos, col, tsize))) = (text_buf.as_mut(), text_info) {
                if region.contains(pos) {
                    egui::Area::new(egui::Id::new("rayshot-text-edit"))
                        .fixed_pos(text_screen(pos))
                        .constrain(false)
                        .show(ui.ctx(), |ui| {
                            ui.style_mut().visuals.text_cursor.stroke = egui::Stroke::NONE;
                            ui.style_mut().visuals.selection.bg_fill = egui::Color32::TRANSPARENT;
                            ui.style_mut().visuals.selection.stroke = egui::Stroke::NONE;
                            let font = egui::FontId::proportional(tsize * tscale);
                            let charw = (tsize * tscale * 0.6).max(6.0);
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
                            let line_h = gsize.y.max(tsize * tscale);
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
                                egui::Stroke::new(2.0f32, col),
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
                                egui::Stroke::new(1.0f32, egui::Color32::from_white_alpha(160)),
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
                                egui::Stroke::new(1.0f32, egui::Color32::from_gray(60)),
                                egui::StrokeKind::Outside,
                            );
                            if gh.hovered() || gh.dragged() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
                            }
                            if gh.dragged() {
                                text_pos_shift += gh.drag_delta() / tscale;
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
                if region.contains(sel.max) && sel_mode.is_none() {
                    let s = egui::Rect::from_min_max(plain_to_screen(sel.min), plain_to_screen(sel.max));

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

                    let mut colour_button_y = top;
                    let tools_area = egui::Area::new(tools_id)
                        .fixed_pos(egui::pos2(x, top))
                        .constrain(false)
                        .show(ui.ctx(), |ui| {
                            egui::Frame::popup(ui.style()).show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 3.0;
                                ui.vertical(|ui| {
                                    for t in Tool::ALL.into_iter().filter(|t| !hidden.contains(t)) {
                                        let key = bindings
                                            .chord(crate::keys::Command::Tool(t))
                                            .label();
                                        if tool_button(ui, t.icon(), tool == t)
                                            .on_hover_text(format!("{} ({key})", t.label()))
                                            .clicked()
                                        {
                                            tool = t;
                                        }
                                    }
                                    ui.separator();
                                    let r = colour_button(ui, color, show_colours)
                                        .on_hover_text("Colour (1–9)");
                                    colour_button_y = r.rect.top();
                                    if r.clicked() {
                                        show_colours = !show_colours;
                                        show_settings = false;
                                    }
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
                                    for (button, act) in [
                                        (Button::Copy, Action::Copy),
                                        (Button::CopyText, Action::CopyText),
                                        (Button::Save, Action::Save),
                                        (Button::SaveAs, Action::SaveAs),
                                    ] {
                                        if hidden_buttons.contains(&button) {
                                            continue;
                                        }
                                        let (icon, _) = button_meta(button);
                                        if tool_button(ui, icon, false)
                                            .on_hover_text(button_hint(button, &bindings))
                                            .clicked()
                                        {
                                            action = Some(act);
                                        }
                                    }
                                    if tool_button(ui, p::GEAR_SIX, show_settings)
                                        .on_hover_text("Customise the toolbar")
                                        .clicked()
                                    {
                                        show_settings = !show_settings;
                                        show_colours = false;
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

                    let beside = |w: f32| {
                        let mut sx = x + width + gap;
                        if sx + w > avail.right() - pad {
                            sx = x - gap - w;
                        }
                        sx.max(avail.left() + pad)
                    };
                    let side_width = |id: egui::Id, guess: f32| {
                        egui::AreaState::load(ui.ctx(), id)
                            .and_then(|st| st.size)
                            .map_or(guess, |sz| sz.x)
                    };

                    if show_colours {
                        let colours_id = egui::Id::new("rayshot-colours");
                        let w = side_width(colours_id, 215.0);
                        egui::Area::new(colours_id)
                            .fixed_pos(egui::pos2(beside(w), colour_button_y))
                            .constrain(true)
                            .show(ui.ctx(), |ui| {
                                egui::Frame::popup(ui.style()).show(ui, |ui| {
                                    if colour_panel(
                                        ui,
                                        &mut color,
                                        &recent,
                                        &mut hex_text,
                                        &mut picker_hsv,
                                    ) {
                                        show_colours = false;
                                    }
                                });
                            });
                    }

                    if show_settings {
                        let settings_id = egui::Id::new("rayshot-settings");
                        let w = side_width(settings_id, 260.0);
                        egui::Area::new(settings_id)
                            .fixed_pos(egui::pos2(beside(w), top))
                            .constrain(true)
                            .show(ui.ctx(), |ui| {
                                egui::Frame::popup(ui.style()).show(ui, |ui| {
                                    settings_panel(
                                        ui,
                                        &mut hidden,
                                        &mut hidden_buttons,
                                        &mut bindings,
                                        &mut rebinding,
                                        &mut rebind_note,
                                    );
                                });
                            });
                    }
                }
            }

            let label = |painter: &egui::Painter, at: egui::Pos2, text: &str, swatch: Option<egui::Color32>| {
                let font = egui::FontId::proportional(14.0);
                let galley = painter.layout_no_wrap(text.to_owned(), font, egui::Color32::WHITE);
                let pad = egui::vec2(8.0, 5.0);
                let sw = if swatch.is_some() { 18.0 } else { 0.0 };
                let size = galley.size() + pad * 2.0 + egui::vec2(sw, 0.0);
                let rect = egui::Rect::from_min_size(at, size);
                painter.rect_filled(rect, 6, egui::Color32::from_black_alpha(200));
                if let Some(c) = swatch {
                    let r = egui::Rect::from_min_size(rect.min + pad, egui::vec2(12.0, galley.size().y));
                    painter.rect_filled(r, 2, c);
                }
                painter.galley(rect.min + pad + egui::vec2(sw, 0.0), galley, egui::Color32::WHITE);
            };
            if let Some(p) = raw_cursor {
                let at = p + egui::vec2(18.0, 18.0);
                if let Some(text) = &toast {
                    label(painter, at, text, None);
                } else if tool == Tool::Picker
                    && let Some(c) = frame_px.as_deref().and_then(|f| pixel_color(f, tool_frame(p)))
                {
                    label(painter, at, &hex(c), Some(c));
                }
            }

            if selection.is_none() {
                painter.text(
                    image_rect.center_top() + egui::vec2(0.0, 18.0),
                    egui::Align2::CENTER_TOP,
                    "drag to select · Shift draws straight · wheel sets width · 1-9 colours · Enter copies · Esc cancels",
                    egui::FontId::proportional(15.0),
                    egui::Color32::WHITE,
                );
            }
        });

        self.selection = selection;
        self.drag_start = drag_start;
        self.draft = draft;
        self.draft_start = draft_start;
        self.straight_from = straight_from;
        self.tool = tool;
        self.color = color;
        self.stroke_width = stroke_width;
        self.scroll_acc = scroll_acc;
        self.hidden = hidden;
        self.hidden_buttons = hidden_buttons;
        if !show_settings {
            rebinding = None;
            rebind_note = None;
        }
        self.bindings = bindings;
        self.rebinding = rebinding;
        self.rebind_note = rebind_note;
        self.show_settings = show_settings;
        self.windows[idx].view_center = view_center;
        self.pan_drag = pan_drag;
        self.sel_offset = sel_offset;
        self.sel_zoom = sel_zoom;
        self.lens_drag = lens_drag;
        self.show_colours = show_colours;
        self.picker_hsv = picker_hsv;
        self.hex_text = hex_text;
        if stroke_width != width_before {
            self.toast = Some((format!("{stroke_width:.0} px"), std::time::Instant::now()));
        }
        if let Some(c) = picked {
            let text = hex(c);
            self.toast = Some((format!("Copied {text}"), std::time::Instant::now()));
            self.wait_colour_copy();
            self.colour_copy = Some(std::thread::spawn(move || {
                if let Err(e) = crate::export::copy_text_to_clipboard(&text) {
                    eprintln!("[rayshot] colour copy failed: {e:#}");
                }
            }));
        }
        self.sel_mode = sel_mode;
        self.sel_ref = sel_ref;
        let drag_just_started = self.shape_drag.is_none() && shape_drag.is_some();
        self.shape_drag = shape_drag;
        if drag_just_started {
            self.scene.begin_change();
        }
        if let Some((i, shape)) = drag_apply {
            self.scene.set_shape(i, shape);
        }
        if let Some((i, shape)) = wheel_edit {
            self.scene.edit(i, crate::scene::Group::Width, shape);
        }
        if let Some(size) = size_toast {
            self.toast = Some((format!("{size:.0} px"), std::time::Instant::now()));
        }
        if tool != Tool::Select {
            selected = None;
        }
        if selected != selected_at_start {
            self.scene.break_group();
            self.scroll_acc = 0.0;
            if let Some(shape) = selected.and_then(|i| self.scene.shapes().get(i)) {
                if let Some(c) = crate::scene::colour_of(shape) {
                    self.color = c;
                }
                if let Some(w) = shape_width(shape) {
                    self.stroke_width = w;
                }
            }
        } else if self.color != color_at_start
            && let Some(i) = selected
        {
            self.recolor_selected(i);
        }
        self.selected = selected;

        if let Some(d) = self.text_edit.as_mut() {
            if let Some(buf) = text_buf {
                d.buf = buf;
            }
            d.pos += text_pos_shift;
        }
        let mut removed: Vec<usize> = Vec::new();
        if text_lost_focus {
            removed.extend(self.commit_text());
        }

        let reediting = self
            .text_edit
            .as_ref()
            .is_some_and(|d| d.replaces.is_some());
        if tool != Tool::Text && (!reediting || tool != tool_at_start) {
            removed.extend(self.commit_text());
        }
        if clear_scene {
            self.text_edit = None;
            self.selected = None;
            self.scene = crate::scene::Scene::default();
        }
        if start_text.is_some() || reedit_text.is_some() {
            self.rebinding = None;
        }
        if let Some(pos) = start_text {
            removed.extend(self.commit_text());
            self.text_edit = Some(TextDraft {
                pos,
                buf: String::new(),
                color,
                size: 24.0,
                replaces: None,
            });
        }
        if let Some(i) = reedit_text {
            removed.extend(self.commit_text());
            if let Some(i) = removed
                .iter()
                .try_fold(i, |i, &gone| shifted_after_removal(i, gone))
                && let Some(Shape::Text {
                    pos,
                    text,
                    color,
                    size,
                }) = self.scene.shapes().get(i).cloned()
            {
                self.text_edit = Some(TextDraft {
                    pos,
                    buf: text,
                    color,
                    size,
                    replaces: Some(i),
                });
                self.selected = None;
            }
        }
        if let Some(s) = committed {
            if !matches!(s, Shape::Pixelate { .. } | Shape::Blur { .. }) {
                crate::prefs::push_recent(&mut self.recent, self.color, &PALETTE);
            }
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

        let (surface_texture, suboptimal) = match win.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => (t, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => (t, true),
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
                        load: wgpu::LoadOp::Clear(if see_through {
                            wgpu::Color::TRANSPARENT
                        } else {
                            wgpu::Color::BLACK
                        }),
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
        if reconfigure_after_suboptimal(suboptimal, win.suboptimal_reconfigured) {
            win.suboptimal_reconfigured = true;
            win.surface.configure(&shared.device, &win.config);
            win.window.request_redraw();
        }

        for id in &full_output.textures_delta.free {
            win.renderer.free_texture(id);
        }

        Ok(())
    }

    fn export_area(&self) -> Option<egui::Rect> {
        let sel = self.selection?;
        Some(Lens::new(sel, self.sel_zoom, self.sel_offset).map_or(sel, |l| l.inner))
    }

    fn export_size(&self) -> Option<(u32, u32)> {
        let sel = self.selection?;
        let dims = self.frame_dims()?;
        Some(export_plan(sel, dims, self.sel_zoom, self.sel_offset, self.zoom).1)
    }

    fn cropped_image(&self) -> Option<image::RgbaImage> {
        let sel = self.export_area()?;
        let frame = self.frame.as_ref()?;
        let (x, y, w, h) = crop_box(sel, frame.width, frame.height);
        if w == 0 || h == 0 {
            return None;
        }
        let full = image::RgbaImage::from_raw(frame.width, frame.height, frame.rgba.clone())?;
        Some(image::imageops::crop_imm(&full, x, y, w, h).to_image())
    }

    fn render_selection_to_image(&self, redactions_only: bool) -> Result<Option<image::RgbaImage>> {
        let shared = self.shared.as_ref().context("gpu not initialized")?;
        let frame = self.frame.as_ref().context("frame not captured")?;
        let Some(sel) = self.export_area() else {
            return Ok(None);
        };

        let (x, y, w, h) = crop_box(sel, frame.width, frame.height);
        let (fx, fy) = (x as f32, y as f32);
        let (fx2, fy2) = ((x + w) as f32, (y + h) as f32);
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

        let frame_view = shared
            .frame_view
            .as_ref()
            .context("frame texture not ready")?;
        let ctx = egui::Context::default();
        let mut renderer = Renderer::new(device, format, RendererOptions::default());
        let frame_tex_id =
            renderer.register_native_texture(device, frame_view, wgpu::FilterMode::Linear);
        let blur_tex_id = match shared.blur_view.as_ref() {
            Some(v) => renderer.register_native_texture(device, v, wgpu::FilterMode::Linear),
            None => frame_tex_id,
        };

        let frame_w = frame.width as f32;
        let frame_h = frame.height as f32;
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
            let redaction = |s: &&crate::scene::Shape| {
                matches!(
                    s,
                    crate::scene::Shape::Blur { .. } | crate::scene::Shape::Pixelate { .. }
                )
            };
            for s in shapes.iter().filter(|s| !redactions_only || redaction(s)) {
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

    fn export_image(&self) -> Result<Option<image::RgbaImage>> {
        let Some(size) = self.export_size() else {
            return Ok(None);
        };
        Ok(self
            .render_selection_to_image(false)?
            .map(|img| resized_to(img, size)))
    }

    fn finish(&self) -> Result<Option<std::path::PathBuf>> {
        let render_started = std::time::Instant::now();
        let img = match self.export_image() {
            Ok(Some(img)) => img,
            Ok(None) => return Ok(None),
            Err(e) => {
                eprintln!("[rayshot] annotated export failed ({e:#}); using raw crop");
                match (self.cropped_image(), self.export_size()) {
                    (Some(img), Some(size)) => resized_to(img, size),
                    _ => return Ok(None),
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
        match crate::export::copy_png_to_clipboard(&png) {
            Ok(()) => eprintln!("[rayshot] copied to clipboard"),
            Err(e) => {
                eprintln!("[rayshot] clipboard copy failed: {e:#}");
                // A notification from this process would vanish with it, so the
                // extension tells the user.
                reply(&format!("copy-failed {}", path.display()));
            }
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

    fn commit_text(&mut self) -> Option<usize> {
        let d = self.text_edit.take()?;
        let color = d.color;
        match apply_text(&mut self.scene, d) {
            TextOutcome::Added => {
                crate::prefs::push_recent(&mut self.recent, color, &PALETTE);
                None
            }
            TextOutcome::Removed(i) => {
                self.selected = self.selected.and_then(|s| shifted_after_removal(s, i));
                Some(i)
            }
            TextOutcome::Kept | TextOutcome::Nothing => None,
        }
    }

    fn recolor_selected(&mut self, i: usize) {
        if let Some(shape) = self.scene.shapes().get(i) {
            let new = crate::scene::recolored(shape, self.color);
            if &new != shape {
                self.scene.edit(i, crate::scene::Group::Colour, new);
            }
        }
    }

    fn close(&mut self, _event_loop: &ActiveEventLoop) {
        self.wait_colour_copy();
        if !self.serve || self.session.is_some() {
            crate::prefs::save(&crate::prefs::Prefs {
                hidden: self.hidden.clone(),
                color: Some(self.color),
                width: Some(self.stroke_width),
                hidden_buttons: self.hidden_buttons.clone(),
                recent: self.recent.clone(),
                keys: self.bindings.clone(),
            });
        }
        if let Some(sel) = self.selection {
            if sel.width() > 1.0 && sel.height() > 1.0 {
                crate::export::save_last_selection(sel.min.x, sel.min.y, sel.width(), sel.height());
            }
        }
        if self.serve {
            self.end_session(SessionEnd::Closed);
        }
        crate::anim::restore_detached();
        unsafe { libc::_exit(0) }
    }

    fn apply_prefs(&mut self, prefs: crate::prefs::Prefs) {
        if let Some(c) = prefs.color {
            self.color = c;
        }
        if let Some(w) = prefs.width {
            self.stroke_width = w;
        }
        self.hidden = prefs.hidden;
        self.hidden_buttons = prefs.hidden_buttons;
        self.recent = prefs.recent;
        self.bindings = prefs.keys;
    }

    fn wait_colour_copy(&mut self) {
        if let Some(copy) = self.colour_copy.take() {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(4);
            while !copy.is_finished() && std::time::Instant::now() < until {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    }

    fn start_session(&mut self, event_loop: &ActiveEventLoop, id: u64, width: u32, height: u32) {
        if !self.serve {
            return;
        }
        if self.session.is_some() {
            eprintln!("[rayshot] session {id} refused, this process serves one session");
            reply(&format!("failed {id}"));
            return;
        }
        let started = std::time::Instant::now();
        self.session = Some(id);
        self.apply_prefs(crate::prefs::load());
        self.live = true;
        self.frame_size = Some((width, height));
        if let Err(e) = self.create_windows(event_loop) {
            eprintln!("[rayshot] failed to create windows: {e:?}");
            self.end_session(SessionEnd::Failed);
        }
        self.show_live();
        eprintln!("[rayshot] session {id} shown in {:?}", started.elapsed());
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop) {
        if self.serve {
            self.end_session(SessionEnd::Failed);
        } else {
            self.close(event_loop);
        }
    }

    // One process serves one session: exiting releases every window, surface and
    // GPU resource at once, and the extension keeps the next process warm.
    fn end_session(&mut self, end: SessionEnd) -> ! {
        if let Some(id) = self.session.take() {
            let word = match end {
                SessionEnd::Closed => "closed",
                SessionEnd::Failed => "failed",
            };
            reply(&format!("{word} {id}"));
        }
        let code = match end {
            SessionEnd::Closed => 0,
            SessionEnd::Failed => 1,
        };
        unsafe { libc::_exit(code) }
    }

    fn text_has_keyboard(&self) -> bool {
        self.windows.iter().any(|w| {
            w.egui_ctx
                .memory(|m| m.focused())
                .is_some_and(|id| egui::text_edit::TextEditState::load(&w.egui_ctx, id).is_some())
        })
    }

    fn catch_up(&mut self) {
        for i in 0..self.windows.len() {
            if self.windows[i].unrendered_input {
                if let Err(e) = self.render_window(i) {
                    eprintln!("[rayshot] render error: {e:?}");
                }
                self.windows[i].unrendered_input = false;
            }
        }
    }

    fn run_command(&mut self, event_loop: &ActiveEventLoop, command: crate::keys::Command) {
        use crate::keys::Command;
        match command {
            Command::Tool(t) => {
                self.tool = t;
                if t != crate::scene::Tool::Select {
                    self.selected = None;
                }
                self.request_redraw_all();
            }
            Command::Copy => self.run_action(event_loop, Action::Copy),
            Command::CopyText => self.run_action(event_loop, Action::CopyText),
            Command::Save => self.run_action(event_loop, Action::Save),
            Command::SaveAs => self.run_action(event_loop, Action::SaveAs),
            Command::ZoomIn | Command::ZoomOut | Command::ZoomReset => {
                let text = if self.selection.is_some() {
                    self.sel_zoom = match command {
                        Command::ZoomIn => step_zoom(self.sel_zoom, true),
                        Command::ZoomOut => step_zoom(self.sel_zoom, false),
                        _ => 1.0,
                    };
                    if self.sel_zoom <= 1.0 {
                        self.sel_offset = egui::Vec2::ZERO;
                    }
                    format!("Selection {} %", (self.sel_zoom * 100.0).round())
                } else {
                    "Select an area first".to_string()
                };
                self.toast = Some((text, std::time::Instant::now()));
                self.request_redraw_all();
            }
            Command::ScreenZoomIn | Command::ScreenZoomOut | Command::ScreenZoomReset => {
                let next = match command {
                    Command::ScreenZoomIn => step_zoom(self.zoom, true),
                    Command::ScreenZoomOut => step_zoom(self.zoom, false),
                    _ => 1.0,
                };
                self.set_zoom(next);
                let percent = (self.zoom * 100.0).round();
                self.toast = Some((format!("Screen {percent} %"), std::time::Instant::now()));
                self.request_redraw_all();
            }
        }
    }

    fn set_zoom(&mut self, next: f32) {
        let zoom = self.zoom;
        for win in &mut self.windows {
            let region = win.region;
            let view = view_rect(region, zoom, win.view_center.unwrap_or(region.center()));
            let size = win
                .window
                .inner_size()
                .to_logical::<f32>(win.window.scale_factor());
            let avail =
                egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.width, size.height));
            let (scale, image_rect) = fit(avail, view);
            let anchor = win
                .cursor
                .filter(|p| image_rect.contains(*p))
                .map_or(view.center(), |p| view.min + (p - image_rect.min) / scale);
            win.view_center = (next > 1.0).then(|| zoom_center(region, view, anchor, next));
        }
        self.zoom = next;
    }

    fn capture_rebind(&mut self, target: crate::keys::Command, event: &winit::event::KeyEvent) {
        use crate::keys::{Assign, Chord, Key as Bindable};
        self.request_redraw_all();
        let held = match event.physical_key {
            PhysicalKey::Code(code) => Some(code),
            _ => None,
        };
        match event.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.rebinding = None;
                self.rebind_note = None;
                self.held_after_capture = held;
                return;
            }
            Key::Named(
                NamedKey::Control
                | NamedKey::Shift
                | NamedKey::Alt
                | NamedKey::AltGraph
                | NamedKey::Super
                | NamedKey::Meta
                | NamedKey::Hyper,
            ) => return,
            _ => {}
        }
        let key = match event.physical_key {
            PhysicalKey::Code(code) => Bindable::from_code(code),
            _ => None,
        };
        let Some(key) = key else {
            self.rebind_note = Some(RebindNote::Refused {
                target,
                why: "This key cannot be used here".into(),
            });
            return;
        };
        let chord = Chord {
            ctrl: self.modifiers.control_key(),
            shift: self.modifiers.shift_key(),
            alt: self.modifiers.alt_key(),
            key,
        };
        match self.bindings.assign(target, chord) {
            Assign::Done => {
                self.rebinding = None;
                self.rebind_note = None;
                self.held_after_capture = held;
            }
            Assign::Conflict(other) => {
                self.rebinding = None;
                self.held_after_capture = held;
                self.rebind_note = Some(RebindNote::Conflict {
                    target,
                    chord,
                    other,
                });
            }
            Assign::Reserved(why) => {
                self.rebind_note = Some(RebindNote::Refused {
                    target,
                    why: why.into(),
                });
            }
        }
    }

    fn run_action(&mut self, event_loop: &ActiveEventLoop, action: Action) {
        match action {
            Action::Copy => self.finish_and_exit(event_loop),
            Action::Save => self.save_and_exit(event_loop),
            Action::SaveAs => self.save_as_and_exit(event_loop),
            Action::CopyText => self.copy_text_and_exit(event_loop),
            Action::Close => {
                eprintln!("[rayshot] exit: toolbar close");
                self.close(event_loop);
            }
        }
    }

    fn defer_until_frame(&mut self, action: Action) -> bool {
        if self.frame.is_some() {
            return false;
        }
        eprintln!("[rayshot] export waits for the frame");
        self.pending_export = Some(action);
        true
    }

    fn finish_and_exit(&mut self, event_loop: &ActiveEventLoop) {
        if self.defer_until_frame(Action::Copy) {
            return;
        }
        self.wait_colour_copy();
        match self.finish() {
            Ok(Some(path)) => {
                eprintln!("[rayshot] kept a copy at {}", path.display());
                self.close(event_loop);
            }
            Ok(None) => eprintln!("[rayshot] nothing to copy (no selection)"),
            Err(e) => eprintln!("[rayshot] finish failed: {e:?}"),
        }
    }

    fn save_as_and_exit(&mut self, event_loop: &ActiveEventLoop) {
        if self.defer_until_frame(Action::SaveAs) {
            return;
        }
        let png = match self.export_image() {
            Ok(Some(img)) => crate::export::to_png_bytes(&img),
            Ok(None) => {
                eprintln!("[rayshot] nothing to save (no selection)");
                return;
            }
            Err(e) => {
                eprintln!("[rayshot] save render failed: {e:#}");
                return;
            }
        };
        let path = match png.and_then(|png| crate::export::save_to_scratch(&png)) {
            Ok(path) => path,
            Err(e) => {
                eprintln!("[rayshot] save as failed: {e:#}");
                return;
            }
        };
        match spawn_detached("save-as", &path) {
            Ok(()) => {
                eprintln!(
                    "[rayshot] kept a copy at {}, asking where to save",
                    path.display()
                );
                self.close(event_loop);
            }
            Err(e) => eprintln!("[rayshot] save as failed to start: {e}"),
        }
    }

    fn copy_text_and_exit(&mut self, event_loop: &ActiveEventLoop) {
        if self.defer_until_frame(Action::CopyText) {
            return;
        }
        let img = match self.render_selection_to_image(true) {
            Ok(Some(img)) => img,
            Ok(None) => {
                eprintln!("[rayshot] nothing to read (no selection)");
                return;
            }
            Err(e) => {
                eprintln!("[rayshot] copy text render failed: {e:#}");
                return;
            }
        };
        let path = match crate::export::to_png_bytes(&img)
            .and_then(|png| crate::export::save_to_scratch(&png))
        {
            Ok(path) => path,
            Err(e) => {
                eprintln!("[rayshot] copy text failed: {e:#}");
                return;
            }
        };
        match spawn_detached("ocr", &path) {
            Ok(()) => {
                eprintln!("[rayshot] reading text from {}", path.display());
                self.close(event_loop);
            }
            Err(e) => {
                let _ = std::fs::remove_file(&path);
                eprintln!("[rayshot] copy text failed to start: {e}");
            }
        }
    }

    fn save_and_exit(&mut self, event_loop: &ActiveEventLoop) {
        if self.defer_until_frame(Action::Save) {
            return;
        }
        let img = match self.export_image() {
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
                self.close(event_loop);
            }
            Err(e) => eprintln!("[rayshot] save failed: {e:#}"),
        }
    }
}

fn pixel_color(frame: &Frame, p: egui::Pos2) -> Option<egui::Color32> {
    if p.x < 0.0 || p.y < 0.0 {
        return None;
    }
    let (x, y) = (p.x as usize, p.y as usize);
    if x >= frame.width as usize || y >= frame.height as usize {
        return None;
    }
    let i = (y * frame.width as usize + x) * 4;
    let px = frame.rgba.get(i..i + 3)?;
    Some(egui::Color32::from_rgb(px[0], px[1], px[2]))
}

use crate::prefs::{Button, WIDTH_RANGE, hex};
const TOAST_FOR: std::time::Duration = std::time::Duration::from_millis(1500);

const SCROLL_STEP: f32 = 30.0;

fn scrolled_width(width: f32, acc: &mut f32, delta: f32) -> f32 {
    *acc += delta;
    let steps = (*acc / SCROLL_STEP).trunc();
    *acc -= steps * SCROLL_STEP;
    (width + steps).clamp(WIDTH_RANGE.0, WIDTH_RANGE.1)
}

const PALETTE: [egui::Color32; 9] = [
    egui::Color32::from_rgb(235, 59, 59),
    egui::Color32::from_rgb(245, 140, 30),
    egui::Color32::from_rgb(245, 210, 30),
    egui::Color32::from_rgb(60, 190, 80),
    egui::Color32::from_rgb(40, 200, 220),
    egui::Color32::from_rgb(45, 110, 235),
    egui::Color32::from_rgb(150, 70, 220),
    egui::Color32::WHITE,
    egui::Color32::BLACK,
];

fn palette_color(digit: u32) -> Option<egui::Color32> {
    PALETTE.get((digit as usize).checked_sub(1)?).copied()
}

fn counter_radius(width: f32) -> f32 {
    10.0 + width * 2.0
}

const TEXT_SIZE_RANGE: (f32, f32) = (10.0, 120.0);
const TEXT_SIZE_STEP: f32 = 2.0;
const DUPLICATE_OFFSET: egui::Vec2 = egui::vec2(16.0, 16.0);

fn scrolled_text_size(size: f32, acc: &mut f32, delta: f32) -> f32 {
    *acc += delta;
    let steps = (*acc / SCROLL_STEP).trunc();
    *acc -= steps * SCROLL_STEP;
    (size + steps * TEXT_SIZE_STEP).clamp(TEXT_SIZE_RANGE.0, TEXT_SIZE_RANGE.1)
}

fn is_marker(shape: &crate::scene::Shape) -> bool {
    matches!(shape, crate::scene::Shape::Pen { color, .. } if color.a() < 255)
}

fn shape_width(shape: &crate::scene::Shape) -> Option<f32> {
    use crate::scene::Shape;
    match shape {
        _ if is_marker(shape) => None,
        Shape::Rect { width, .. }
        | Shape::Ellipse { width, .. }
        | Shape::Line { width, .. }
        | Shape::Arrow { width, .. }
        | Shape::Pen { width, .. } => Some(*width),
        Shape::Counter { radius, .. } => Some((radius - counter_radius(0.0)) / 2.0),
        _ => None,
    }
}

fn with_width(shape: &crate::scene::Shape, new: f32) -> crate::scene::Shape {
    use crate::scene::Shape;
    let mut out = shape.clone();
    if is_marker(shape) {
        return out;
    }
    match &mut out {
        Shape::Rect { width, .. }
        | Shape::Ellipse { width, .. }
        | Shape::Line { width, .. }
        | Shape::Arrow { width, .. }
        | Shape::Pen { width, .. } => *width = new,
        Shape::Counter { radius, .. } => *radius = counter_radius(new),
        _ => {}
    }
    out
}

fn reshaped(
    shape: &crate::scene::Shape,
    grip: crate::scene::Grip,
    to: egui::Pos2,
    shift: bool,
    bounds: egui::Rect,
) -> crate::scene::Shape {
    use crate::scene::{Grip, Shape, Tool};
    let mut out = shape.clone();
    match (&mut out, grip) {
        (Shape::Rect { rect, .. } | Shape::Ellipse { rect, .. }, Grip::Box(h)) => {
            let corner = [
                rect.left_top(),
                rect.right_top(),
                rect.right_bottom(),
                rect.left_bottom(),
            ];
            *rect = match h {
                0 | 2 | 4 | 6 if shift => {
                    let anchor = corner[(h / 2 + 2) % 4];
                    egui::Rect::from_two_pos(
                        anchor,
                        constrain(Tool::Rect, anchor, to, true, bounds),
                    )
                }
                _ => resize_rect(*rect, h, to),
            };
        }
        (Shape::Line { from, to: end, .. } | Shape::Arrow { from, to: end, .. }, Grip::From) => {
            *from = constrain(Tool::Line, *end, to, shift, bounds);
        }
        (Shape::Line { from, to: end, .. } | Shape::Arrow { from, to: end, .. }, Grip::To) => {
            *end = constrain(Tool::Line, *from, to, shift, bounds);
        }
        _ => {}
    }
    out
}

fn constrain(
    tool: crate::scene::Tool,
    from: egui::Pos2,
    to: egui::Pos2,
    shift: bool,
    bounds: egui::Rect,
) -> egui::Pos2 {
    use crate::scene::Tool;
    if !shift {
        return to;
    }
    let d = to - from;
    match tool {
        Tool::Rect | Tool::Ellipse => {
            let sign = |d: f32, from: f32, min: f32, max: f32| match d {
                d if d > 0.0 => 1.0,
                d if d < 0.0 => -1.0,
                _ if max - from >= from - min => 1.0,
                _ => -1.0,
            };
            let sx = sign(d.x, from.x, bounds.min.x, bounds.max.x);
            let sy = sign(d.y, from.y, bounds.min.y, bounds.max.y);
            let room_x = if sx > 0.0 {
                bounds.max.x - from.x
            } else {
                from.x - bounds.min.x
            };
            let room_y = if sy > 0.0 {
                bounds.max.y - from.y
            } else {
                from.y - bounds.min.y
            };
            let side = d.x.abs().max(d.y.abs()).min(room_x).min(room_y).max(0.0);
            from + egui::vec2(sx * side, sy * side)
        }
        Tool::Line | Tool::Arrow | Tool::Pen | Tool::Marker => {
            let len = d.length();
            if len == 0.0 {
                return to;
            }
            let step = (d.y.atan2(d.x) / std::f32::consts::FRAC_PI_4).round() as i32;
            let (x, y) = match step.rem_euclid(8) {
                0 => (1.0, 0.0),
                1 => (1.0, 1.0),
                2 => (0.0, 1.0),
                3 => (-1.0, 1.0),
                4 => (-1.0, 0.0),
                5 => (-1.0, -1.0),
                6 => (0.0, -1.0),
                _ => (1.0, -1.0),
            };
            let dir = egui::vec2(x, y).normalized();
            let room = |dir: f32, from: f32, min: f32, max: f32| match dir {
                d if d > 0.0 => (max - from) / d,
                d if d < 0.0 => (min - from) / d,
                _ => f32::INFINITY,
            };
            let len = len
                .min(room(dir.x, from.x, bounds.min.x, bounds.max.x))
                .min(room(dir.y, from.y, bounds.min.y, bounds.max.y))
                .max(0.0);
            (from + dir * len).clamp(bounds.min, bounds.max)
        }
        _ => to,
    }
}

fn extend_stroke(
    tool: crate::scene::Tool,
    points: &mut Vec<egui::Pos2>,
    anchor: &mut Option<usize>,
    to: egui::Pos2,
    shift: bool,
    bounds: egui::Rect,
) {
    if !shift {
        *anchor = None;
        points.push(to);
        return;
    }
    let at = match *anchor {
        Some(at) if at < points.len() => at,
        _ => {
            let Some(last) = points.len().checked_sub(1) else {
                points.push(to);
                return;
            };
            *anchor = Some(last);
            last
        }
    };
    points.truncate(at + 1);
    points.push(constrain(tool, points[at], to, true, bounds));
}

/// The selection a drag made over the freeze before the overlay appeared.
fn forwarded_selection(
    from: (f32, f32),
    to: (f32, f32),
    frame_w: f32,
    frame_h: f32,
) -> Option<egui::Rect> {
    let frame = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(frame_w, frame_h));
    let rect = egui::Rect::from_two_pos(egui::pos2(from.0, from.1), egui::pos2(to.0, to.1))
        .intersect(frame);
    (rect.width() >= MIN_SELECTION && rect.height() >= MIN_SELECTION).then_some(rect)
}

/// What the selection becomes after a forwarded drag update: a drag that
/// shrinks back to a click clears the selection it made.
fn next_selection(
    current: Option<egui::Rect>,
    forwarded: bool,
    rect: Option<egui::Rect>,
) -> (Option<egui::Rect>, bool) {
    match rect {
        Some(r) => (Some(r), true),
        None if forwarded => (None, true),
        None => (current, false),
    }
}

// Once per window: a driver that reports every frame as suboptimal would
// otherwise rebuild the swapchain on each one.
fn reconfigure_after_suboptimal(suboptimal: bool, done_before: bool) -> bool {
    suboptimal && !done_before
}

// Wayland repeats the same size on every configure; rebuilding the swapchain
// for it is wasted work and one more trip through a driver call that has crashed.
fn surface_needs_configure(configured: bool, current: (u32, u32), new: (u32, u32)) -> bool {
    new.0 > 0 && new.1 > 0 && (!configured || current != new)
}

// egui-wgpu blends into premultiplied RGB, so only PreMultiplied composites correctly.
fn see_through_alpha(modes: &[wgpu::CompositeAlphaMode]) -> Option<wgpu::CompositeAlphaMode> {
    let mode = wgpu::CompositeAlphaMode::PreMultiplied;
    modes.contains(&mode).then_some(mode)
}

fn marker_color(c: egui::Color32) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 110)
}

const PALETTE_NAMES: [&str; 9] = [
    "Red", "Orange", "Yellow", "Green", "Cyan", "Blue", "Purple", "White", "Black",
];

fn button_meta(button: Button) -> (&'static str, crate::keys::Command) {
    use crate::keys::Command;
    use egui_phosphor::regular as p;
    match button {
        Button::Copy => (p::COPY, Command::Copy),
        Button::CopyText => (p::TEXT_AA, Command::CopyText),
        Button::Save => (p::FLOPPY_DISK, Command::Save),
        Button::SaveAs => (p::FOLDER_OPEN, Command::SaveAs),
    }
}

fn button_hint(button: Button, bindings: &crate::keys::Bindings) -> String {
    let (_, command) = button_meta(button);
    let key = bindings.chord(command).label();
    match button {
        Button::Copy => format!("{} (Enter, {key})", command.label()),
        _ => format!("{} ({key})", command.label()),
    }
}

fn colour_button(ui: &mut egui::Ui, color: egui::Color32, open: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(30.0, 28.0), egui::Sense::click());
    let painter = ui.painter();
    if open || resp.hovered() {
        painter.rect_filled(rect, 4, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    painter.circle_filled(rect.center(), 9.0, color);
    painter.circle_stroke(
        rect.center(),
        9.0,
        egui::Stroke::new(1.5f32, egui::Color32::from_white_alpha(210)),
    );
    resp
}

fn hex_input(text: &str) -> Option<egui::Color32> {
    let digits = text.trim();
    let digits = digits.strip_prefix('#').unwrap_or(digits);
    crate::prefs::parse_color(&format!("#{digits}"))
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Hsv {
    h: f32,
    s: f32,
    v: f32,
}

impl Hsv {
    fn of(color: egui::Color32) -> Self {
        let unit = |c: u8| c as f32 / 255.0;
        let (h, s, v) =
            egui::ecolor::hsv_from_rgb([unit(color.r()), unit(color.g()), unit(color.b())]);
        Self { h, s, v }
    }

    fn color(self) -> egui::Color32 {
        let [r, g, b] = egui::ecolor::rgb_from_hsv((self.h, self.s, self.v));
        let byte = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
        egui::Color32::from_rgb(byte(r), byte(g), byte(b))
    }
}

fn hsv_color(h: f32, s: f32, v: f32) -> egui::Color32 {
    Hsv { h, s, v }.color()
}

fn sync_hsv(hsv: &mut Hsv, color: egui::Color32) {
    if hsv.color() != color {
        let hue = hsv.h;
        *hsv = Hsv::of(color);
        if hsv.s <= f32::EPSILON || hsv.v <= f32::EPSILON {
            hsv.h = hue;
        }
    }
}

fn paint_grid(
    painter: &egui::Painter,
    rect: egui::Rect,
    cols: usize,
    rows: usize,
    color_at: impl Fn(f32, f32) -> egui::Color32,
) {
    let mut mesh = egui::Mesh::default();
    for j in 0..=rows {
        for i in 0..=cols {
            let (u, v) = (i as f32 / cols as f32, j as f32 / rows as f32);
            mesh.colored_vertex(
                egui::pos2(egui::lerp(rect.x_range(), u), egui::lerp(rect.y_range(), v)),
                color_at(u, v),
            );
        }
    }
    let w = cols as u32 + 1;
    for j in 0..rows as u32 {
        for i in 0..cols as u32 {
            let a = j * w + i;
            mesh.add_triangle(a, a + 1, a + w);
            mesh.add_triangle(a + 1, a + w + 1, a + w);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

fn swatch_row(
    ui: &mut egui::Ui,
    width: f32,
    colors: &[egui::Color32],
    current: egui::Color32,
    hint: impl Fn(usize) -> String,
    id: &str,
) -> Option<egui::Color32> {
    let cell = width / PALETTE.len() as f32;
    let (row, _) = ui.allocate_exact_size(egui::vec2(width, cell), egui::Sense::hover());
    let mut chosen = None;
    for (i, c) in colors.iter().enumerate() {
        let r = egui::Rect::from_min_size(
            egui::pos2(row.left() + cell * i as f32, row.top()),
            egui::vec2(cell, cell),
        );
        let resp = ui
            .interact(r, ui.id().with((id, i)), egui::Sense::click())
            .on_hover_text(hint(i));
        let painter = ui.painter();
        let radius = cell * 0.32;
        if *c == current {
            painter.circle_stroke(
                r.center(),
                radius + 3.0,
                egui::Stroke::new(2.0f32, egui::Color32::WHITE),
            );
        }
        painter.circle_filled(
            r.center(),
            if resp.hovered() { radius + 1.0 } else { radius },
            *c,
        );
        painter.circle_stroke(
            r.center(),
            radius,
            egui::Stroke::new(1.0f32, egui::Color32::from_white_alpha(40)),
        );
        if resp.clicked() {
            chosen = Some(*c);
        }
    }
    chosen
}

fn colour_panel(
    ui: &mut egui::Ui,
    color: &mut egui::Color32,
    recent: &[egui::Color32],
    hex_text: &mut String,
    hsv: &mut Hsv,
) -> bool {
    const WIDTH: f32 = 198.0;
    sync_hsv(hsv, *color);
    ui.spacing_mut().item_spacing = egui::vec2(8.0, 10.0);
    let edge = egui::Stroke::new(1.0f32, egui::Color32::from_white_alpha(30));
    let hex_id = egui::Id::new("rayshot-hex");

    let (sv_rect, sv) =
        ui.allocate_exact_size(egui::vec2(WIDTH, 128.0), egui::Sense::click_and_drag());
    if sv.is_pointer_button_down_on()
        && let Some(p) = sv.interact_pointer_pos()
    {
        ui.memory_mut(|m| m.surrender_focus(hex_id));
        hsv.s = ((p.x - sv_rect.left()) / sv_rect.width()).clamp(0.0, 1.0);
        hsv.v = 1.0 - ((p.y - sv_rect.top()) / sv_rect.height()).clamp(0.0, 1.0);
        *color = hsv.color();
    }
    let h = hsv.h;
    paint_grid(ui.painter(), sv_rect, 24, 12, |s, t| {
        hsv_color(h, s, 1.0 - t)
    });
    ui.painter()
        .rect_stroke(sv_rect, 0, edge, egui::StrokeKind::Inside);
    let knob = egui::pos2(
        egui::lerp(sv_rect.x_range(), hsv.s),
        egui::lerp(sv_rect.y_range(), 1.0 - hsv.v),
    );
    ui.painter().circle_filled(knob, 6.0, *color);
    ui.painter()
        .circle_stroke(knob, 7.0, egui::Stroke::new(2.0f32, egui::Color32::WHITE));
    ui.painter().circle_stroke(
        knob,
        8.5,
        egui::Stroke::new(1.0f32, egui::Color32::from_black_alpha(90)),
    );

    let (hue_rect, hue) =
        ui.allocate_exact_size(egui::vec2(WIDTH, 12.0), egui::Sense::click_and_drag());
    if hue.is_pointer_button_down_on()
        && let Some(p) = hue.interact_pointer_pos()
    {
        ui.memory_mut(|m| m.surrender_focus(hex_id));
        hsv.h = ((p.x - hue_rect.left()) / hue_rect.width()).clamp(0.0, 1.0);
        if hsv.s <= f32::EPSILON {
            hsv.s = 1.0;
        }
        if hsv.v <= f32::EPSILON {
            hsv.v = 1.0;
        }
        *color = hsv.color();
    }
    paint_grid(ui.painter(), hue_rect, 36, 1, |u, _| hsv_color(u, 1.0, 1.0));
    ui.painter()
        .rect_stroke(hue_rect, 0, edge, egui::StrokeKind::Inside);
    let hx = egui::lerp(hue_rect.x_range(), hsv.h);
    let handle =
        egui::Rect::from_center_size(egui::pos2(hx, hue_rect.center().y), egui::vec2(6.0, 18.0));
    ui.painter().rect_filled(handle, 3, egui::Color32::WHITE);
    ui.painter().rect_stroke(
        handle,
        3,
        egui::Stroke::new(1.0f32, egui::Color32::from_black_alpha(120)),
        egui::StrokeKind::Outside,
    );

    ui.horizontal(|ui| {
        let (chip, _) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::hover());
        ui.painter().rect_filled(chip, 6, *color);
        ui.painter()
            .rect_stroke(chip, 6, edge, egui::StrokeKind::Inside);
        if !ui.memory(|m| m.has_focus(hex_id)) {
            *hex_text = hex(*color);
        }
        let resp = ui.add(
            egui::TextEdit::singleline(hex_text)
                .id(hex_id)
                .desired_width(WIDTH - 28.0 - 8.0)
                .char_limit(7)
                .margin(egui::Margin::symmetric(8, 6))
                .font(egui::TextStyle::Monospace),
        );
        if resp.changed()
            && let Some(c) = hex_input(hex_text)
        {
            *color = c;
        }
    });

    let mut chosen = false;
    let names = |i: usize| format!("{}  {}", i + 1, PALETTE_NAMES[i]);
    if let Some(c) = swatch_row(ui, WIDTH, &PALETTE, *color, names, "palette") {
        *color = c;
        chosen = true;
    }
    if !recent.is_empty() {
        ui.add_space(-4.0);
        ui.label(egui::RichText::new("Recent").small().weak());
        ui.add_space(-6.0);
        if let Some(c) = swatch_row(ui, WIDTH, recent, *color, |i| hex(recent[i]), "recent") {
            *color = c;
            chosen = true;
        }
    }
    chosen
}

fn toggle(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let size = egui::vec2(34.0, 18.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, egui::Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool(resp.id, *on);
    let visuals = ui.style().interact_selectable(&resp, *on);
    let radius = rect.height() / 2.0;
    let track = if *on {
        ui.visuals().selection.bg_fill
    } else {
        ui.visuals().widgets.inactive.bg_fill
    };
    ui.painter().rect(
        rect,
        radius,
        track,
        visuals.bg_stroke,
        egui::StrokeKind::Inside,
    );
    let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), t);
    ui.painter().circle_filled(
        egui::pos2(x, rect.center().y),
        radius - 3.0,
        egui::Color32::WHITE,
    );
    resp
}

fn key_cell(
    ui: &mut egui::Ui,
    command: crate::keys::Command,
    bindings: &mut crate::keys::Bindings,
    rebinding: &mut Option<crate::keys::Command>,
    note: &mut Option<RebindNote>,
) {
    ui.horizontal(|ui| {
        let capturing = *rebinding == Some(command);
        let text = if capturing {
            "Press a key…".to_string()
        } else {
            bindings.chord(command).label()
        };
        let label = egui::RichText::new(text).monospace().small();
        if ui
            .add(egui::Button::selectable(capturing, label))
            .on_hover_text("Click, then press the new key. Esc cancels.")
            .clicked()
        {
            *rebinding = (!capturing).then_some(command);
            *note = None;
        }
        if !bindings.is_default(command)
            && ui
                .small_button(egui_phosphor::regular::ARROW_COUNTER_CLOCKWISE)
                .on_hover_text("Back to the default key")
                .clicked()
        {
            bindings.reset(command);
            *rebinding = None;
            *note = None;
        }
    });
}

fn note_row(
    ui: &mut egui::Ui,
    command: crate::keys::Command,
    bindings: &mut crate::keys::Bindings,
    note: &mut Option<RebindNote>,
) {
    let warn = ui.visuals().warn_fg_color;
    match note.clone() {
        Some(RebindNote::Conflict {
            target,
            chord,
            other,
        }) if target == command => {
            ui.label("");
            ui.label(
                egui::RichText::new(format!("{} is used by {}", chord.label(), other.label()))
                    .small()
                    .color(warn),
            );
            ui.horizontal(|ui| {
                if ui.small_button("Swap").clicked() {
                    bindings.swap(target, other);
                    *note = None;
                }
                if ui.small_button("Cancel").clicked() {
                    *note = None;
                }
            });
            ui.label("");
            ui.end_row();
        }
        Some(RebindNote::Refused { target, why }) if target == command => {
            ui.label("");
            ui.label(egui::RichText::new(why).small().color(warn));
            ui.label("");
            ui.label("");
            ui.end_row();
        }
        _ => {}
    }
}

fn settings_panel(
    ui: &mut egui::Ui,
    hidden: &mut Vec<crate::scene::Tool>,
    hidden_buttons: &mut Vec<Button>,
    bindings: &mut crate::keys::Bindings,
    rebinding: &mut Option<crate::keys::Command>,
    note: &mut Option<RebindNote>,
) {
    use crate::keys::Command;
    use crate::scene::Tool;
    use egui_phosphor::regular as p;
    let header = |ui: &mut egui::Ui, title: &str| {
        ui.label("");
        ui.label(egui::RichText::new(title).strong());
        ui.end_row();
    };
    let mut row = |ui: &mut egui::Ui, icon: &str, command: Command, on: Option<&mut bool>| {
        ui.label(egui::RichText::new(icon).size(16.0));
        ui.label(command.label());
        key_cell(ui, command, bindings, rebinding, note);
        let changed = match on {
            Some(on) => toggle(ui, on).changed(),
            None => {
                ui.label("");
                false
            }
        };
        ui.end_row();
        note_row(ui, command, bindings, note);
        changed
    };
    egui::Grid::new("rayshot-settings")
        .num_columns(4)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            header(ui, "Tools");
            for t in Tool::ALL {
                let mut on = !hidden.contains(&t);
                if row(ui, t.icon(), Command::Tool(t), Some(&mut on)) {
                    if on {
                        hidden.retain(|h| *h != t);
                    } else {
                        hidden.push(t);
                    }
                }
            }
            ui.end_row();
            header(ui, "Actions");
            for b in Button::ALL {
                let (icon, command) = button_meta(b);
                let mut on = !hidden_buttons.contains(&b);
                if row(ui, icon, command, Some(&mut on)) {
                    if on {
                        hidden_buttons.retain(|h| *h != b);
                    } else {
                        hidden_buttons.push(b);
                    }
                }
            }
            ui.end_row();
            header(ui, "Other keys");
            row(ui, p::MAGNIFYING_GLASS_PLUS, Command::ZoomIn, None);
            row(ui, p::MAGNIFYING_GLASS_MINUS, Command::ZoomOut, None);
            row(ui, p::ARROWS_OUT_SIMPLE, Command::ZoomReset, None);
            row(ui, p::MAGNIFYING_GLASS_PLUS, Command::ScreenZoomIn, None);
            row(ui, p::MAGNIFYING_GLASS_MINUS, Command::ScreenZoomOut, None);
            row(ui, p::ARROWS_OUT_SIMPLE, Command::ScreenZoomReset, None);
        });
    ui.add_space(6.0);
    let changed = Command::ALL.iter().any(|c| !bindings.is_default(*c));
    if ui
        .add_enabled(changed, egui::Button::new("Reset all keys").small())
        .clicked()
    {
        bindings.reset_all();
        *rebinding = None;
        *note = None;
    }
    ui.add(
        egui::Label::new(
            egui::RichText::new(
                "Esc, Enter, 1–9, Ctrl+Z, Ctrl+Y, Delete, the arrows and Ctrl+D always keep their keys.",
            )
            .small()
            .weak(),
        )
        .wrap(),
    );
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

#[cfg(test)]
mod tests {
    use super::reconfigure_after_suboptimal;
    use super::see_through_alpha;
    use super::surface_needs_configure;

    use super::{
        Frame, PALETTE, WIDTH_RANGE, counter_radius, hex, palette_color, pixel_color,
        scrolled_width,
    };

    #[test]
    fn the_picker_reads_the_frame_pixel_under_the_cursor() {
        let mut rgba = vec![0u8; 3 * 2 * 4];
        rgba[(1 * 3 + 2) * 4..(1 * 3 + 2) * 4 + 4].copy_from_slice(&[18, 52, 86, 255]);
        let frame = Frame {
            width: 3,
            height: 2,
            rgba,
        };
        assert_eq!(
            pixel_color(&frame, pos2(2.6, 1.2)),
            Some(egui::Color32::from_rgb(18, 52, 86))
        );
        assert_eq!(
            pixel_color(&frame, pos2(0.0, 0.0)),
            Some(egui::Color32::from_rgb(0, 0, 0))
        );
        assert_eq!(
            pixel_color(&frame, pos2(3.0, 0.0)),
            None,
            "right of the frame"
        );
        assert_eq!(pixel_color(&frame, pos2(0.0, 2.0)), None, "below the frame");
        assert_eq!(
            pixel_color(&frame, pos2(-0.5, 0.0)),
            None,
            "left of the frame"
        );
    }

    #[test]
    fn colours_are_written_as_hex() {
        assert_eq!(hex(egui::Color32::from_rgb(18, 52, 86)), "#123456");
        assert_eq!(hex(egui::Color32::from_rgb(255, 0, 171)), "#FF00AB");
        assert_eq!(hex(egui::Color32::BLACK), "#000000");
    }

    #[test]
    fn the_wheel_changes_width_one_step_per_notch() {
        let mut acc = 0.0;
        assert_eq!(scrolled_width(3.0, &mut acc, 50.0), 4.0, "up thickens");
        assert_eq!(scrolled_width(4.0, &mut acc, -50.0), 3.0, "down thins");
        assert_eq!(
            scrolled_width(3.0, &mut acc, 10.0),
            3.0,
            "a small touchpad move waits"
        );
        assert_eq!(scrolled_width(3.0, &mut acc, 10.0), 3.0);
        assert_eq!(scrolled_width(3.0, &mut acc, 15.0), 4.0, "until it adds up");
    }

    #[test]
    fn width_stays_in_range() {
        let mut acc = 0.0;
        assert_eq!(
            scrolled_width(WIDTH_RANGE.1, &mut acc, 500.0),
            WIDTH_RANGE.1
        );
        let mut acc = 0.0;
        assert_eq!(
            scrolled_width(WIDTH_RANGE.0, &mut acc, -500.0),
            WIDTH_RANGE.0
        );
        let mut acc = 0.0;
        assert_eq!(scrolled_width(3.0, &mut acc, 0.0), 3.0);
    }

    fn frame_bounds() -> egui::Rect {
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1000.0, 800.0))
    }

    fn boxed(rect: egui::Rect) -> crate::scene::Shape {
        crate::scene::Shape::Rect {
            rect,
            color: egui::Color32::RED,
            width: 3.0,
        }
    }

    #[test]
    fn a_corner_grip_resizes_from_the_opposite_corner() {
        use crate::scene::Grip;
        let base = egui::Rect::from_min_max(egui::pos2(100.0, 100.0), egui::pos2(200.0, 150.0));
        let r = super::reshaped(
            &boxed(base),
            Grip::Box(4),
            egui::pos2(260.0, 180.0),
            false,
            frame_bounds(),
        );
        assert_eq!(
            r,
            boxed(egui::Rect::from_min_max(
                egui::pos2(100.0, 100.0),
                egui::pos2(260.0, 180.0)
            ))
        );
        let r = super::reshaped(
            &boxed(base),
            Grip::Box(1),
            egui::pos2(500.0, 60.0),
            false,
            frame_bounds(),
        );
        assert_eq!(
            r,
            boxed(egui::Rect::from_min_max(
                egui::pos2(100.0, 60.0),
                egui::pos2(200.0, 150.0)
            )),
            "an edge grip moves one side only"
        );
        let r = super::reshaped(
            &boxed(base),
            Grip::Box(0),
            egui::pos2(300.0, 300.0),
            false,
            frame_bounds(),
        );
        assert_eq!(
            r,
            boxed(egui::Rect::from_min_max(
                egui::pos2(200.0, 150.0),
                egui::pos2(300.0, 300.0)
            )),
            "dragging past the far corner flips"
        );
    }

    #[test]
    fn shift_on_a_corner_grip_keeps_the_box_square() {
        use crate::scene::Grip;
        let base = egui::Rect::from_min_max(egui::pos2(100.0, 100.0), egui::pos2(200.0, 150.0));
        let r = super::reshaped(
            &boxed(base),
            Grip::Box(4),
            egui::pos2(260.0, 180.0),
            true,
            frame_bounds(),
        );
        assert_eq!(
            r,
            boxed(egui::Rect::from_min_max(
                egui::pos2(100.0, 100.0),
                egui::pos2(260.0, 260.0)
            ))
        );
        let e = crate::scene::Shape::Ellipse {
            rect: base,
            color: egui::Color32::RED,
            width: 3.0,
        };
        match super::reshaped(
            &e,
            Grip::Box(0),
            egui::pos2(20.0, 90.0),
            true,
            frame_bounds(),
        ) {
            crate::scene::Shape::Ellipse { rect, .. } => {
                assert_eq!(rect.max, egui::pos2(200.0, 150.0));
                assert_eq!(rect.width(), rect.height());
            }
            _ => panic!("not an ellipse"),
        }
    }

    #[test]
    fn line_grips_move_one_end_and_snap_with_shift() {
        use crate::scene::{Grip, Shape};
        let line = Shape::Arrow {
            from: egui::pos2(100.0, 100.0),
            to: egui::pos2(200.0, 100.0),
            color: egui::Color32::RED,
            width: 3.0,
        };
        match super::reshaped(
            &line,
            Grip::To,
            egui::pos2(300.0, 140.0),
            false,
            frame_bounds(),
        ) {
            Shape::Arrow { from, to, .. } => {
                assert_eq!(from, egui::pos2(100.0, 100.0));
                assert_eq!(to, egui::pos2(300.0, 140.0));
            }
            _ => panic!("not an arrow"),
        }
        match super::reshaped(
            &line,
            Grip::From,
            egui::pos2(195.0, 20.0),
            true,
            frame_bounds(),
        ) {
            Shape::Arrow { from, to, .. } => {
                assert_eq!(to, egui::pos2(200.0, 100.0));
                assert!((from.x - 200.0).abs() < 0.01, "{from:?}");
                assert!(from.y < 100.0);
            }
            _ => panic!("not an arrow"),
        }
        let pen = Shape::Pen {
            points: vec![egui::pos2(1.0, 1.0), egui::pos2(5.0, 5.0)],
            color: egui::Color32::RED,
            width: 3.0,
        };
        assert_eq!(
            super::reshaped(
                &pen,
                Grip::To,
                egui::pos2(50.0, 50.0),
                false,
                frame_bounds()
            ),
            pen
        );
    }

    #[test]
    fn width_goes_where_each_shape_keeps_it() {
        use crate::scene::Shape;
        let base = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(10.0, 10.0));
        assert_eq!(super::shape_width(&boxed(base)), Some(3.0));
        match super::with_width(&boxed(base), 9.0) {
            Shape::Rect { width, .. } => assert_eq!(width, 9.0),
            _ => panic!("not a rect"),
        }
        let c = Shape::Counter {
            center: egui::pos2(0.0, 0.0),
            number: 1,
            color: egui::Color32::RED,
            radius: super::counter_radius(4.0),
        };
        assert_eq!(super::shape_width(&c), Some(4.0));
        match super::with_width(&c, 7.0) {
            Shape::Counter { radius, .. } => assert_eq!(radius, super::counter_radius(7.0)),
            _ => panic!("not a counter"),
        }
        let marker = Shape::Pen {
            points: vec![egui::pos2(0.0, 0.0)],
            color: super::marker_color(egui::Color32::RED),
            width: 16.0,
        };
        assert_eq!(super::shape_width(&marker), None);
        assert_eq!(super::with_width(&marker, 3.0), marker);
        let text = Shape::Text {
            pos: egui::pos2(0.0, 0.0),
            text: "hi".into(),
            color: egui::Color32::RED,
            size: 24.0,
        };
        assert_eq!(super::shape_width(&text), None);
        assert_eq!(super::with_width(&text, 3.0), text);
    }

    #[test]
    fn the_wheel_steps_text_size_within_range() {
        let mut acc = 0.0;
        assert_eq!(
            super::scrolled_text_size(24.0, &mut acc, super::SCROLL_STEP),
            26.0
        );
        let mut acc = 0.0;
        assert_eq!(
            super::scrolled_text_size(24.0, &mut acc, -super::SCROLL_STEP),
            22.0
        );
        let mut acc = 0.0;
        assert_eq!(
            super::scrolled_text_size(24.0, &mut acc, super::SCROLL_STEP / 2.0),
            24.0
        );
        assert_eq!(
            super::scrolled_text_size(24.0, &mut acc, super::SCROLL_STEP / 2.0),
            26.0,
            "small scrolls add up"
        );
        let mut acc = 0.0;
        assert_eq!(
            super::scrolled_text_size(super::TEXT_SIZE_RANGE.1, &mut acc, 500.0),
            super::TEXT_SIZE_RANGE.1
        );
        let mut acc = 0.0;
        assert_eq!(
            super::scrolled_text_size(super::TEXT_SIZE_RANGE.0, &mut acc, -500.0),
            super::TEXT_SIZE_RANGE.0
        );
    }

    fn text_shape(text: &str) -> crate::scene::Shape {
        crate::scene::Shape::Text {
            pos: egui::pos2(10.0, 10.0),
            text: text.into(),
            color: egui::Color32::RED,
            size: 24.0,
        }
    }

    fn draft(text: &str, replaces: Option<usize>) -> super::TextDraft {
        super::TextDraft {
            pos: egui::pos2(10.0, 10.0),
            buf: text.into(),
            color: egui::Color32::RED,
            size: 24.0,
            replaces,
        }
    }

    #[test]
    fn the_selection_outline_covers_the_inside_of_a_shape() {
        let base = egui::Rect::from_min_max(egui::pos2(100.0, 100.0), egui::pos2(200.0, 150.0));
        let to_screen = |p: egui::Pos2| egui::pos2(p.x * 2.0, p.y * 2.0);
        let r = super::selection_outline(&boxed(base), &to_screen, 2.0).unwrap();
        assert!(r.contains(egui::pos2(300.0, 250.0)), "middle of the box");
        assert!(
            r.contains(egui::pos2(200.0 - 7.0, 200.0 - 7.0)),
            "just outside the line"
        );
        assert!(!r.contains(egui::pos2(200.0 - 20.0, 250.0)));
        assert!(!r.contains(egui::pos2(420.0, 250.0)));
        let blur = crate::scene::Shape::Blur {
            points: vec![egui::pos2(1.0, 1.0)],
            radius: 16.0,
        };
        assert_eq!(super::selection_outline(&blur, &to_screen, 2.0), None);
    }

    #[test]
    fn helpers_start_from_the_installed_binary_unless_it_was_replaced() {
        let exists = |p: &std::path::Path| p == std::path::Path::new("/usr/bin/rayshot");
        assert_eq!(
            super::launch_path(Some("/usr/bin/rayshot".into()), &exists),
            std::path::PathBuf::from("/usr/bin/rayshot")
        );
        assert_eq!(
            super::launch_path(Some("/usr/bin/rayshot (deleted)".into()), &exists),
            std::path::PathBuf::from("/proc/self/exe")
        );
        assert_eq!(
            super::launch_path(None, &exists),
            std::path::PathBuf::from("/proc/self/exe")
        );
    }

    #[test]
    fn a_window_takes_its_region_from_the_monitor_with_its_name() {
        let rects = vec![
            crate::live::MonitorRect {
                name: "DP-1".into(),
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
            crate::live::MonitorRect {
                name: "DP-2".into(),
                x: 1920,
                y: 0,
                width: 3840,
                height: 2160,
            },
        ];
        assert_eq!(
            super::monitor_region(Some("DP-2"), &rects),
            Some(egui::Rect::from_min_size(
                egui::pos2(1920.0, 0.0),
                egui::vec2(3840.0, 2160.0)
            ))
        );
        assert_eq!(super::monitor_region(Some("HDMI-1"), &rects), None);
        assert_eq!(super::monitor_region(None, &rects), None);
        assert_eq!(super::monitor_region(Some("DP-1"), &[]), None);
    }

    fn toggle_lefts(shape: &egui::Shape, out: &mut Vec<f32>) {
        match shape {
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| toggle_lefts(s, out)),
            egui::Shape::Rect(r)
                if (r.rect.width() - 34.0).abs() < 0.5
                    && (r.rect.height() - 18.0).abs() < 0.5
                    && r.corner_radius.nw == 9 =>
            {
                out.push(r.rect.left())
            }
            _ => {}
        }
    }

    #[test]
    fn every_toggle_in_the_toolbar_settings_lines_up() {
        let ctx = egui::Context::default();
        let mut hidden = Vec::new();
        let mut hidden_buttons = Vec::new();
        let mut bindings = crate::keys::Bindings::defaults();
        let mut rebinding = None;
        let mut note = None;
        let mut lefts = Vec::new();
        for _ in 0..2 {
            let out = ctx.run_ui(egui::RawInput::default(), |ui| {
                super::settings_panel(
                    ui,
                    &mut hidden,
                    &mut hidden_buttons,
                    &mut bindings,
                    &mut rebinding,
                    &mut note,
                );
            });
            lefts = Vec::new();
            for clipped in &out.shapes {
                toggle_lefts(&clipped.shape, &mut lefts);
            }
        }
        let rows = crate::scene::Tool::ALL.len() + crate::prefs::Button::ALL.len();
        assert_eq!(lefts.len(), rows, "one toggle per row");
        let first = lefts[0];
        assert!(
            lefts.iter().all(|x| (x - first).abs() < 0.5),
            "toggles at {lefts:?}"
        );
    }

    #[test]
    fn zoom_steps_through_the_agreed_sizes() {
        let mut z = 1.0;
        let mut ups = Vec::new();
        for _ in 0..6 {
            z = super::step_zoom(z, true);
            ups.push(z);
        }
        assert_eq!(ups, vec![1.5, 2.0, 3.0, 4.0, 5.0, 5.0]);
        let mut downs = Vec::new();
        for _ in 0..6 {
            z = super::step_zoom(z, false);
            downs.push(z);
        }
        assert_eq!(downs, vec![4.0, 3.0, 2.0, 1.5, 1.0, 1.0]);
        assert_eq!(super::step_zoom(1.2, true), 1.5);
        assert_eq!(super::step_zoom(1.2, false), 1.0);
    }

    #[test]
    fn the_zoomed_view_stays_inside_the_monitor() {
        let region = egui::Rect::from_min_size(egui::pos2(1920.0, 0.0), egui::vec2(1920.0, 1080.0));
        assert_eq!(
            super::view_rect(region, 2.0, egui::pos2(2880.0, 540.0)),
            egui::Rect::from_min_max(egui::pos2(2400.0, 270.0), egui::pos2(3360.0, 810.0))
        );
        assert_eq!(
            super::view_rect(region, 2.0, egui::pos2(1930.0, 10.0)),
            egui::Rect::from_min_max(egui::pos2(1920.0, 0.0), egui::pos2(2880.0, 540.0)),
            "pushed back from the top left corner"
        );
        assert_eq!(
            super::view_rect(region, 4.0, egui::pos2(9000.0, 9000.0)),
            egui::Rect::from_min_max(egui::pos2(3360.0, 810.0), egui::pos2(3840.0, 1080.0)),
            "pushed back from the bottom right corner"
        );
        assert_eq!(super::view_rect(region, 1.0, egui::pos2(0.0, 0.0)), region);
    }

    #[test]
    fn zooming_keeps_the_point_under_the_cursor_in_place() {
        let region = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1920.0, 1080.0));
        let anchor = egui::pos2(480.0, 270.0);
        let center = super::zoom_center(region, region, anchor, 2.0);
        let view = super::view_rect(region, 2.0, center);
        assert_eq!(view.min, egui::pos2(240.0, 135.0));
        assert_eq!(view.size(), egui::vec2(960.0, 540.0));
        let center = super::zoom_center(region, view, anchor, 4.0);
        let deeper = super::view_rect(region, 4.0, center);
        assert_eq!(deeper.min, egui::pos2(360.0, 202.5));
        let back = super::zoom_center(region, deeper, anchor, 1.0);
        assert_eq!(super::view_rect(region, 1.0, back), region);
    }

    #[test]
    fn the_view_fills_the_window_at_the_zoomed_scale() {
        let avail = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1920.0, 1080.0));
        let view = egui::Rect::from_min_max(egui::pos2(480.0, 270.0), egui::pos2(1440.0, 810.0));
        let (scale, image_rect) = super::fit(avail, view);
        assert_eq!(scale, 2.0);
        assert_eq!(image_rect, avail);
        let hidpi = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(3840.0, 2160.0));
        let (scale, image_rect) = super::fit(avail, hidpi);
        assert_eq!(scale, 0.5, "frame pixels of a 2x monitor are half a point");
        assert_eq!(image_rect, avail);
    }

    #[test]
    fn the_crop_covers_every_pixel_the_selection_touches() {
        let sel = egui::Rect::from_min_max(egui::pos2(10.4, 0.0), egui::pos2(110.6, 10.0));
        assert_eq!(super::crop_box(sel, 1920, 1080), (10, 0, 101, 10));
        let past = egui::Rect::from_min_max(egui::pos2(-5.0, 1070.2), egui::pos2(30.0, 1200.0));
        assert_eq!(super::crop_box(past, 1920, 1080), (0, 1070, 30, 10));
        let outside = egui::Rect::from_min_max(egui::pos2(1930.0, 5.0), egui::pos2(1990.0, 50.0));
        assert_eq!(super::crop_box(outside, 1920, 1080).2, 0);
    }

    #[test]
    fn the_lens_shows_the_middle_of_the_selection_magnified() {
        let outer = egui::Rect::from_min_max(egui::pos2(100.0, 100.0), egui::pos2(500.0, 400.0));
        assert!(super::Lens::new(outer, 1.0, egui::Vec2::ZERO).is_none());
        let lens = super::Lens::new(outer, 2.0, egui::Vec2::ZERO).unwrap();
        assert_eq!(
            lens.inner,
            egui::Rect::from_min_max(egui::pos2(200.0, 175.0), egui::pos2(400.0, 325.0))
        );
        assert_eq!(lens.into_frame(outer.min), lens.inner.min);
        assert_eq!(lens.into_frame(outer.max), lens.inner.max);
        assert_eq!(
            lens.into_frame(egui::pos2(300.0, 250.0)),
            egui::pos2(300.0, 250.0)
        );
        assert_eq!(
            lens.out_of_frame(egui::pos2(250.0, 200.0)),
            egui::pos2(200.0, 150.0)
        );
        let q = egui::pos2(137.0, 333.0);
        assert_eq!(lens.out_of_frame(lens.into_frame(q)), q);
    }

    #[test]
    fn the_lens_can_be_moved_but_not_past_the_selection() {
        let outer = egui::Rect::from_min_max(egui::pos2(100.0, 100.0), egui::pos2(500.0, 400.0));
        let moved = super::Lens::new(outer, 2.0, egui::vec2(-50.0, 30.0)).unwrap();
        assert_eq!(moved.inner.center(), egui::pos2(250.0, 280.0));
        let pushed = super::Lens::new(outer, 4.0, egui::vec2(1000.0, -1000.0)).unwrap();
        assert_eq!(
            pushed.inner,
            egui::Rect::from_min_max(egui::pos2(400.0, 100.0), egui::pos2(500.0, 175.0))
        );
        assert_eq!(pushed.offset(), egui::vec2(150.0, -112.5));
    }

    #[test]
    fn a_zoomed_selection_exports_its_middle_at_the_selection_size() {
        let sel = egui::Rect::from_min_max(egui::pos2(100.0, 100.0), egui::pos2(500.0, 300.0));
        let plain = super::export_plan(sel, (1920, 1080), 1.0, egui::Vec2::ZERO, 1.0);
        assert_eq!(plain, (sel, (400, 200)));
        let lens = super::export_plan(sel, (1920, 1080), 2.0, egui::Vec2::ZERO, 1.0);
        assert_eq!(
            lens,
            (
                egui::Rect::from_min_max(egui::pos2(200.0, 150.0), egui::pos2(400.0, 250.0)),
                (400, 200)
            )
        );
        let both = super::export_plan(sel, (1920, 1080), 2.0, egui::Vec2::ZERO, 2.0);
        assert_eq!(
            both.1,
            (800, 400),
            "a zoomed screen still doubles the picture"
        );
        let moved = super::export_plan(sel, (1920, 1080), 4.0, egui::vec2(-1000.0, 0.0), 1.0);
        assert_eq!(moved.0.min, egui::pos2(100.0, 175.0));
    }

    #[test]
    fn a_zoomed_view_draws_the_frame_itself() {
        assert!(
            !super::draws_frame(true, 1.0, true),
            "the shell freeze shows through"
        );
        assert!(super::draws_frame(true, 2.0, true));
        assert!(!super::draws_frame(true, 2.0, false), "nothing to draw yet");
        assert!(super::draws_frame(false, 1.0, true));
        assert!(!super::draws_frame(false, 1.0, false));
    }

    #[test]
    fn a_huge_zoomed_export_is_capped() {
        assert_eq!(super::scaled_size(60.0, 24.0, 4.0), (240, 96));
        let (w, h) = super::scaled_size(3840.0, 2160.0, 4.0);
        assert!(w as u64 * h as u64 <= 40_000_000, "{w}x{h}");
        assert!(w > 3840 * 2, "still zoomed: {w}x{h}");
        assert_eq!(w * 9 / 16, h, "keeps the shape: {w}x{h}");
        assert_eq!(
            super::size_label(3840, 2160, 4.0),
            format!("3840 × 2160 → {w} × {h}")
        );
    }

    #[test]
    fn the_size_label_shows_the_exported_size() {
        assert_eq!(super::size_label(1920, 1080, 1.0), "1920 × 1080");
        assert_eq!(
            super::size_label(1920, 1080, 0.5),
            "1920 × 1080 → 960 × 540"
        );
        assert_eq!(super::size_label(101, 33, 1.5), "101 × 33 → 152 × 50");
        assert_eq!(super::size_label(3, 3, 0.25), "3 × 3 → 1 × 1");
    }

    #[test]
    fn the_export_is_resized_to_the_planned_size() {
        let img = image::RgbaImage::from_pixel(100, 50, image::Rgba([10, 20, 30, 255]));
        assert_eq!(
            super::resized_to(img.clone(), (50, 25)).dimensions(),
            (50, 25)
        );
        assert_eq!(
            super::resized_to(img.clone(), (200, 100)).dimensions(),
            (200, 100)
        );
        let same = super::resized_to(img.clone(), (100, 50));
        assert_eq!(same, img);
        let half = super::resized_to(img, (50, 25));
        assert_eq!(
            half.get_pixel(25, 12).0,
            [10, 20, 30, 255],
            "a flat colour stays flat"
        );
    }

    #[test]
    fn indices_after_a_removal_shift_down() {
        assert_eq!(super::shifted_after_removal(0, 1), Some(0));
        assert_eq!(super::shifted_after_removal(1, 1), None);
        assert_eq!(super::shifted_after_removal(2, 1), Some(1));
        assert_eq!(super::shifted_after_removal(0, 0), None);
    }

    #[test]
    fn a_new_text_is_added_and_a_blank_one_is_dropped() {
        use super::TextOutcome;
        let mut scene = crate::scene::Scene::default();
        assert_eq!(
            super::apply_text(&mut scene, draft("hi", None)),
            TextOutcome::Added
        );
        assert_eq!(scene.shapes(), &[text_shape("hi")]);
        assert_eq!(
            super::apply_text(&mut scene, draft("   ", None)),
            TextOutcome::Nothing
        );
        assert_eq!(scene.shapes().len(), 1);
    }

    #[test]
    fn a_reedited_text_is_replaced_in_place_in_one_step() {
        use super::TextOutcome;
        let mut scene = crate::scene::Scene::default();
        scene.push(text_shape("one"));
        scene.push(text_shape("two"));
        assert_eq!(
            super::apply_text(&mut scene, draft("uno", Some(0))),
            TextOutcome::Kept
        );
        assert_eq!(scene.shapes(), &[text_shape("uno"), text_shape("two")]);
        scene.undo();
        assert_eq!(scene.shapes(), &[text_shape("one"), text_shape("two")]);
    }

    #[test]
    fn an_unchanged_reedit_adds_no_undo_step() {
        use super::TextOutcome;
        let mut scene = crate::scene::Scene::default();
        scene.push(text_shape("one"));
        assert_eq!(
            super::apply_text(&mut scene, draft("one", Some(0))),
            TextOutcome::Kept
        );
        scene.undo();
        assert!(scene.shapes().is_empty());
    }

    #[test]
    fn emptying_a_reedited_text_removes_it() {
        use super::TextOutcome;
        let mut scene = crate::scene::Scene::default();
        scene.push(text_shape("one"));
        scene.push(text_shape("two"));
        assert_eq!(
            super::apply_text(&mut scene, draft("", Some(0))),
            TextOutcome::Removed(0)
        );
        assert_eq!(scene.shapes(), &[text_shape("two")]);
        scene.undo();
        assert_eq!(scene.shapes().len(), 2);
        assert_eq!(
            super::apply_text(&mut scene, draft("", Some(7))),
            TextOutcome::Nothing
        );
    }

    #[test]
    fn dragging_the_square_takes_the_focus_from_the_hex_field() {
        let ctx = egui::Context::default();
        let hex_id = egui::Id::new("rayshot-hex");
        let mut color = egui::Color32::RED;
        let mut hex_text = super::hex(color);
        let mut hsv = super::Hsv::of(color);
        let mut frame = |events: Vec<egui::Event>| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 400.0),
                )),
                events,
                ..Default::default()
            };
            let _ = ctx.run_ui(input, |ui| {
                egui::Area::new(egui::Id::new("panel"))
                    .fixed_pos(egui::Pos2::ZERO)
                    .show(ui.ctx(), |ui| {
                        super::colour_panel(ui, &mut color, &[], &mut hex_text, &mut hsv);
                    });
            });
        };
        frame(vec![]);
        ctx.memory_mut(|m| m.request_focus(hex_id));
        frame(vec![]);
        assert!(ctx.memory(|m| m.has_focus(hex_id)));
        let press = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let at = egui::pos2(20.0, 100.0);
        frame(vec![egui::Event::PointerMoved(at), press(at, true)]);
        let to = egui::pos2(60.0, 110.0);
        frame(vec![egui::Event::PointerMoved(to)]);
        frame(vec![press(to, false)]);
        frame(vec![]);
        assert_ne!(color, egui::Color32::RED);
        assert!(!ctx.memory(|m| m.has_focus(hex_id)));
        assert_eq!(hex_text, super::hex(color));
    }

    #[test]
    fn the_picker_square_is_plain_srgb_hsv() {
        let grey = super::hsv_color(0.0, 0.0, 0.5);
        assert!((127..=128).contains(&grey.r()), "{grey:?}");
        assert_eq!(grey.r(), grey.g());
        assert_eq!(
            super::hsv_color(1.0 / 6.0, 1.0, 1.0),
            egui::Color32::from_rgb(255, 255, 0)
        );
        assert_eq!(
            super::hsv_color(0.0, 0.5, 1.0),
            egui::Color32::from_rgb(255, 128, 128)
        );
    }

    #[test]
    fn the_picker_keeps_every_colour_it_is_given() {
        let mut hsv = super::Hsv::of(egui::Color32::RED);
        for r in (0..=255).step_by(15) {
            for g in (0..=255).step_by(15) {
                for b in (0..=255).step_by(15) {
                    let c = egui::Color32::from_rgb(r, g, b);
                    super::sync_hsv(&mut hsv, c);
                    assert_eq!(hsv.color(), c);
                }
            }
        }
    }

    #[test]
    fn the_picker_keeps_its_hue_through_grey_and_black() {
        let mut hsv = super::Hsv::of(egui::Color32::from_rgb(0, 0, 255));
        let hue = hsv.h;
        super::sync_hsv(&mut hsv, egui::Color32::from_rgb(90, 90, 90));
        assert_eq!(hsv.h, hue);
        super::sync_hsv(&mut hsv, egui::Color32::BLACK);
        assert_eq!(hsv.h, hue);
        super::sync_hsv(&mut hsv, egui::Color32::from_rgb(255, 0, 0));
        assert_ne!(hsv.h, hue);
    }

    #[test]
    fn the_hex_field_takes_what_the_picker_copies() {
        let green = Some(egui::Color32::from_rgb(0, 255, 0));
        assert_eq!(super::hex_input("00FF00"), green);
        assert_eq!(super::hex_input("#00FF00"), green, "pasted from the picker");
        assert_eq!(super::hex_input("00ff00 "), green);
        assert_eq!(super::hex_input("00FF0"), None, "still typing");
        assert_eq!(super::hex_input("##00FF00"), None);
    }

    #[test]
    fn digits_pick_the_palette() {
        assert_eq!(palette_color(1), Some(PALETTE[0]));
        assert_eq!(palette_color(9), Some(PALETTE[8]));
        assert_eq!(palette_color(0), None);
        assert_eq!(palette_color(10), None);
    }

    #[test]
    fn markers_grow_with_the_width() {
        assert!(
            counter_radius(1.0) >= 10.0,
            "readable at the thinnest width"
        );
        assert!(counter_radius(3.0) < counter_radius(10.0));
        assert!(counter_radius(20.0) <= 60.0);
    }

    #[test]
    fn shift_makes_an_ellipse_a_circle() {
        let o = pos2(10.0, 10.0);
        assert!(close(
            constrain(Tool::Ellipse, o, pos2(30.0, 14.0), true, ANY),
            pos2(30.0, 30.0)
        ));
    }

    use super::constrain;
    use super::extend_stroke;
    use super::forwarded_selection;
    use crate::scene::Tool;
    use egui::pos2;

    const ANY: egui::Rect = egui::Rect::EVERYTHING;

    fn close(a: egui::Pos2, b: egui::Pos2) -> bool {
        (a - b).length() < 1e-3
    }

    #[test]
    fn shift_snaps_lines_and_arrows_to_the_nearest_45_degrees() {
        let o = pos2(100.0, 100.0);
        for tool in [Tool::Line, Tool::Arrow, Tool::Pen, Tool::Marker] {
            let east = constrain(tool, o, pos2(150.0, 104.0), true, ANY);
            assert_eq!(east.y, 100.0, "horizontal stays on the row");
            assert!(east.x > 100.0);
            let south = constrain(tool, o, pos2(97.0, 160.0), true, ANY);
            assert_eq!(south.x, 100.0, "vertical stays on the column");
            assert!(south.y > 100.0);
            let west = constrain(tool, o, pos2(40.0, 98.0), true, ANY);
            assert_eq!(west.y, 100.0);
            assert!(west.x < 100.0);
            let north = constrain(tool, o, pos2(102.0, 30.0), true, ANY);
            assert_eq!(north.x, 100.0);
            assert!(north.y < 100.0);
            let diagonal = constrain(tool, o, pos2(140.0, 64.0), true, ANY);
            assert!(
                ((diagonal.x - 100.0) - (100.0 - diagonal.y)).abs() < 1e-3,
                "45 degrees up and right"
            );
        }
    }

    #[test]
    fn snapping_keeps_the_length_of_the_drag() {
        let o = pos2(0.0, 0.0);
        let to = pos2(30.0, 40.0);
        assert!(((constrain(Tool::Line, o, to, true, ANY) - o).length() - 50.0).abs() < 1e-3);
    }

    #[test]
    fn the_split_between_directions_is_at_22_5_degrees() {
        let o = pos2(0.0, 0.0);
        let below = 21.0_f32.to_radians();
        let above = 24.0_f32.to_radians();
        let flat = constrain(
            Tool::Line,
            o,
            pos2(below.cos() * 100.0, below.sin() * 100.0),
            true,
            ANY,
        );
        assert_eq!(flat.y, 0.0);
        let tilted = constrain(
            Tool::Line,
            o,
            pos2(above.cos() * 100.0, above.sin() * 100.0),
            true,
            ANY,
        );
        assert!((tilted.x - tilted.y).abs() < 1e-3);
    }

    #[test]
    fn shift_makes_a_rectangle_square_in_the_drag_direction() {
        let o = pos2(10.0, 10.0);
        assert!(close(
            constrain(Tool::Rect, o, pos2(30.0, 14.0), true, ANY),
            pos2(30.0, 30.0)
        ));
        assert!(close(
            constrain(Tool::Rect, o, pos2(4.0, 40.0), true, ANY),
            pos2(-20.0, 40.0)
        ));
        assert!(close(
            constrain(Tool::Rect, o, pos2(-10.0, -2.0), true, ANY),
            pos2(-10.0, -10.0)
        ));
        assert!(close(
            constrain(Tool::Rect, o, pos2(10.0, 25.0), true, ANY),
            pos2(25.0, 25.0)
        ));
    }

    #[test]
    fn a_zero_length_drag_stays_put() {
        let o = pos2(5.0, 5.0);
        for tool in [Tool::Line, Tool::Arrow, Tool::Rect, Tool::Pen, Tool::Marker] {
            assert_eq!(constrain(tool, o, o, true, ANY), o);
        }
    }

    #[test]
    fn a_snapped_end_stays_inside_the_frame() {
        let frame = egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1920.0, 1080.0));
        let square = constrain(
            Tool::Rect,
            pos2(100.0, 1000.0),
            pos2(600.0, 1080.0),
            true,
            frame,
        );
        assert!(close(square, pos2(180.0, 1080.0)), "{square:?}");
        let end = constrain(
            Tool::Line,
            pos2(1900.0, 500.0),
            pos2(1920.0, 540.0),
            true,
            frame,
        );
        assert!(end.x <= 1920.0 && end.y <= 1080.0, "{end:?}");
        assert!(
            ((end.x - 1900.0) - (end.y - 500.0)).abs() < 1e-3,
            "still on the diagonal: {end:?}"
        );
        assert!(end.x > 1910.0, "shortened only as far as the edge: {end:?}");
        let along_edge = constrain(
            Tool::Rect,
            pos2(100.0, 1080.0),
            pos2(300.0, 1080.0),
            true,
            frame,
        );
        assert!(
            close(along_edge, pos2(300.0, 880.0)),
            "grows into the frame: {along_edge:?}"
        );
    }

    #[test]
    fn shift_mid_stroke_keeps_what_was_drawn() {
        let frame = ANY;
        let mut points = vec![pos2(0.0, 0.0), pos2(5.0, 3.0), pos2(9.0, 8.0)];
        let mut anchor = None;
        extend_stroke(
            Tool::Pen,
            &mut points,
            &mut anchor,
            pos2(60.0, 10.0),
            true,
            frame,
        );
        assert_eq!(
            &points[..3],
            &[pos2(0.0, 0.0), pos2(5.0, 3.0), pos2(9.0, 8.0)]
        );
        assert_eq!(points.len(), 4);
        assert_eq!(points[3].y, 8.0, "straight from the last point");
        extend_stroke(
            Tool::Pen,
            &mut points,
            &mut anchor,
            pos2(9.0, 70.0),
            true,
            frame,
        );
        assert_eq!(points.len(), 4, "the straight part follows the cursor");
        assert_eq!(points[3].x, 9.0);
        extend_stroke(
            Tool::Pen,
            &mut points,
            &mut anchor,
            pos2(20.0, 75.0),
            false,
            frame,
        );
        assert_eq!(points.len(), 5, "freehand again after Shift");
        assert_eq!(points[4], pos2(20.0, 75.0));
        assert_eq!(anchor, None);
    }

    #[test]
    fn shift_from_the_start_draws_one_straight_stroke() {
        let mut points = vec![pos2(10.0, 10.0)];
        let mut anchor = None;
        extend_stroke(
            Tool::Marker,
            &mut points,
            &mut anchor,
            pos2(200.0, 14.0),
            true,
            ANY,
        );
        extend_stroke(
            Tool::Marker,
            &mut points,
            &mut anchor,
            pos2(300.0, 6.0),
            true,
            ANY,
        );
        assert_eq!(points.len(), 2);
        assert_eq!(points[0], pos2(10.0, 10.0));
        assert_eq!(points[1].y, 10.0);
    }

    #[test]
    fn without_shift_or_for_other_tools_nothing_changes() {
        let o = pos2(100.0, 100.0);
        let to = pos2(137.0, 121.0);
        for tool in [
            Tool::Line,
            Tool::Arrow,
            Tool::Rect,
            Tool::Pen,
            Tool::Marker,
            Tool::Blur,
            Tool::Pixelate,
        ] {
            assert_eq!(constrain(tool, o, to, false, ANY), to);
        }
        for tool in [Tool::Blur, Tool::Pixelate, Tool::Select, Tool::Text] {
            assert_eq!(constrain(tool, o, to, true, ANY), to);
        }
    }
    use wgpu::CompositeAlphaMode::{Auto, Inherit, Opaque, PostMultiplied, PreMultiplied};

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Option<egui::Rect> {
        Some(egui::Rect::from_min_max(
            egui::pos2(x0, y0),
            egui::pos2(x1, y1),
        ))
    }

    #[test]
    fn a_suboptimal_frame_reconfigures_the_surface_once() {
        assert!(reconfigure_after_suboptimal(true, false));
        assert!(!reconfigure_after_suboptimal(true, true));
    }

    #[test]
    fn a_good_frame_never_reconfigures() {
        assert!(!reconfigure_after_suboptimal(false, false));
        assert!(!reconfigure_after_suboptimal(false, true));
    }

    #[test]
    fn first_resize_configures_the_surface_even_at_the_initial_size() {
        assert!(surface_needs_configure(false, (1920, 1080), (1920, 1080)));
    }

    #[test]
    fn repeated_resize_to_the_same_size_skips_reconfigure() {
        assert!(!surface_needs_configure(true, (1920, 1080), (1920, 1080)));
    }

    #[test]
    fn resize_to_a_new_size_reconfigures() {
        assert!(surface_needs_configure(true, (1920, 1080), (1920, 1079)));
        assert!(surface_needs_configure(true, (1920, 1080), (3840, 1080)));
    }

    #[test]
    fn zero_sized_resize_never_configures() {
        assert!(!surface_needs_configure(false, (1920, 1080), (0, 1080)));
        assert!(!surface_needs_configure(true, (1920, 1080), (1920, 0)));
    }

    #[test]
    fn forwarded_drag_becomes_a_selection_in_any_direction() {
        assert_eq!(
            forwarded_selection((10.0, 20.0), (300.0, 400.0), 3840.0, 1080.0),
            rect(10.0, 20.0, 300.0, 400.0)
        );
        assert_eq!(
            forwarded_selection((300.0, 400.0), (10.0, 20.0), 3840.0, 1080.0),
            rect(10.0, 20.0, 300.0, 400.0)
        );
    }

    #[test]
    fn forwarded_drag_is_clamped_to_the_frame() {
        assert_eq!(
            forwarded_selection((-50.0, -5.0), (5000.0, 2000.0), 3840.0, 1080.0),
            rect(0.0, 0.0, 3840.0, 1080.0)
        );
    }

    #[test]
    fn minimum_selection_size_is_inclusive() {
        assert!(forwarded_selection((0.0, 0.0), (5.0, 5.0), 100.0, 100.0).is_some());
        assert!(forwarded_selection((0.0, 0.0), (4.9, 5.0), 100.0, 100.0).is_none());
        assert!(forwarded_selection((0.0, 0.0), (5.0, 4.9), 100.0, 100.0).is_none());
    }

    #[test]
    fn forwarded_rect_replaces_the_selection() {
        let old = rect(1.0, 1.0, 50.0, 50.0);
        let new = rect(10.0, 10.0, 90.0, 90.0);
        assert_eq!(super::next_selection(old, false, new), (new, true));
        assert_eq!(super::next_selection(old, true, new), (new, true));
    }

    #[test]
    fn shrinking_a_forwarded_drag_clears_it() {
        let made = rect(10.0, 10.0, 90.0, 90.0);
        assert_eq!(super::next_selection(made, true, None), (None, true));
    }

    #[test]
    fn a_click_before_any_drag_keeps_the_remembered_selection() {
        let remembered = rect(1.0, 1.0, 50.0, 50.0);
        assert_eq!(
            super::next_selection(remembered, false, None),
            (remembered, false)
        );
        assert_eq!(super::next_selection(None, false, None), (None, false));
    }

    #[test]
    fn a_click_or_sliver_is_not_a_selection() {
        assert_eq!(
            forwarded_selection((10.0, 10.0), (10.0, 10.0), 3840.0, 1080.0),
            None
        );
        assert_eq!(
            forwarded_selection((10.0, 10.0), (12.0, 400.0), 3840.0, 1080.0),
            None
        );
        assert_eq!(
            forwarded_selection((10.0, 10.0), (400.0, 13.0), 3840.0, 1080.0),
            None
        );
        assert_eq!(
            forwarded_selection((-50.0, 10.0), (-10.0, 400.0), 3840.0, 1080.0),
            None
        );
    }

    #[test]
    fn prefers_premultiplied() {
        assert_eq!(
            see_through_alpha(&[Opaque, PreMultiplied]),
            Some(PreMultiplied)
        );
        assert_eq!(
            see_through_alpha(&[PostMultiplied, Opaque, PreMultiplied]),
            Some(PreMultiplied)
        );
    }

    #[test]
    fn postmultiplied_alone_is_not_see_through() {
        assert_eq!(see_through_alpha(&[Opaque, PostMultiplied]), None);
    }

    #[test]
    fn opaque_only_is_not_see_through() {
        assert_eq!(see_through_alpha(&[Opaque]), None);
        assert_eq!(see_through_alpha(&[Opaque, Inherit, Auto]), None);
        assert_eq!(see_through_alpha(&[]), None);
    }
}
