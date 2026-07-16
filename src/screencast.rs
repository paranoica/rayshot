use std::cell::RefCell;
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use pipewire as pw;
use pw::stream::StreamState;
use pw::{properties::properties, spa};
use spa::pod::Pod;
use tokio::runtime::Handle;

use crate::capture::Frame;

struct StreamInfo {
    node_id: u32,
    pos: (i32, i32),
}

struct Portal {
    fd: OwnedFd,
    streams: Vec<StreamInfo>,
}

struct Slot {
    raw: Vec<u8>,
    w: u32,
    h: u32,
    stride: usize,
    format: spa::param::video::VideoFormat,
    pos: (i32, i32),
    present: bool,
}

struct Captured {
    pixels: Vec<u8>,
    w: u32,
    h: u32,
    pos: (i32, i32),
}

pub struct ScreencastSession {
    latest: Arc<Mutex<Vec<Slot>>>,
    states: Arc<Mutex<Vec<StreamState>>>,
    quit_tx: pw::channel::Sender<()>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for ScreencastSession {
    fn drop(&mut self) {
        let _ = self.quit_tx.send(());
        if let Some(h) = self.thread.take() {
            let _ = h.join();
        }
    }
}

impl ScreencastSession {
    pub fn start(rt: &Handle) -> Result<Self> {
        let portal = match rt.block_on(async {
            tokio::time::timeout(Duration::from_secs(60), open_screencast()).await
        }) {
            Ok(Ok(p)) => p,
            Ok(Err(e)) => return Err(e).context("screencast portal handshake failed"),
            Err(_) => return Err(anyhow!("screencast portal handshake timed out")),
        };
        let latest: Vec<Slot> = portal
            .streams
            .iter()
            .map(|s| Slot {
                raw: Vec::new(),
                w: 0,
                h: 0,
                stride: 0,
                format: spa::param::video::VideoFormat::BGRx,
                pos: s.pos,
                present: false,
            })
            .collect();
        let latest = Arc::new(Mutex::new(latest));
        let states: Vec<StreamState> = portal
            .streams
            .iter()
            .map(|_| StreamState::Unconnected)
            .collect();
        let states = Arc::new(Mutex::new(states));
        let shared = latest.clone();
        let shared_states = states.clone();
        let fd = portal.fd;
        let infos = portal.streams;
        let (quit_tx, quit_rx) = pw::channel::channel::<()>();
        let thread = std::thread::Builder::new()
            .name("rayshot-pw".into())
            .spawn(move || {
                if let Err(e) = run_pw_loop(fd, infos, shared, shared_states, quit_rx) {
                    eprintln!("[rayshot] screencast pipewire loop failed: {e:?}");
                }
            })
            .context("spawn pipewire thread")?;
        Ok(Self {
            latest,
            states,
            quit_tx,
            thread: Some(thread),
        })
    }

    pub fn is_healthy(&self) -> bool {
        self.states
            .lock()
            .map(|s| !s.is_empty() && s.iter().all(|st| *st == StreamState::Streaming))
            .unwrap_or(false)
    }

    fn grab_inner(&self, timeout: Duration) -> Result<Frame> {
        let start = Instant::now();
        loop {
            if self.latest.lock().unwrap().iter().all(|s| s.present) {
                break;
            }
            if start.elapsed() > timeout {
                return Err(anyhow!("timed out waiting for screencast frames"));
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let slots = self.latest.lock().unwrap();
        let captured: Vec<Captured> = slots
            .iter()
            .map(|s| Captured {
                pixels: to_rgba(&s.raw, s.w, s.h, s.stride, s.format),
                w: s.w,
                h: s.h,
                pos: s.pos,
            })
            .collect();
        composite(&captured)
    }

    pub fn wait_ready(&self, timeout: Duration) -> bool {
        self.grab_inner(timeout).is_ok()
    }

    pub fn grab(&self) -> Result<Frame> {
        self.grab_inner(Duration::from_millis(700))
    }
}

pub fn capture_once(rt: &Handle) -> Result<Frame> {
    let session = ScreencastSession::start(rt)?;
    if !session.wait_ready(Duration::from_secs(5)) {
        return Err(anyhow!("timed out waiting for screencast frames"));
    }
    session.grab()
}

fn token_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join(".config/rayshot/screencast.token")
}

async fn open_screencast() -> Result<Portal> {
    use ashpd::desktop::PersistMode;
    use ashpd::desktop::screencast::{
        CursorMode, OpenPipeWireRemoteOptions, Screencast, SelectSourcesOptions, SourceType,
        StartCastOptions,
    };

    let proxy = Screencast::new().await?;
    let session = proxy.create_session(Default::default()).await?;

    let saved_token = std::fs::read_to_string(token_path())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    proxy
        .select_sources(
            &session,
            SelectSourcesOptions::default()
                .set_multiple(true)
                .set_cursor_mode(CursorMode::Hidden)
                .set_sources(ashpd::enumflags2::BitFlags::from(SourceType::Monitor))
                .set_persist_mode(PersistMode::Application)
                .set_restore_token(saved_token.as_deref()),
        )
        .await?
        .response()?;

    let streams = proxy
        .start(&session, None, StartCastOptions::default())
        .await?
        .response()?;

    if let Some(token) = streams.restore_token() {
        if let Some(dir) = token_path().parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(token_path(), token);
    }

    let infos: Vec<StreamInfo> = streams
        .streams()
        .iter()
        .map(|s| StreamInfo {
            node_id: s.pipe_wire_node_id(),
            pos: s.position().unwrap_or((0, 0)),
        })
        .collect();
    if infos.is_empty() {
        return Err(anyhow!("portal returned no screencast streams"));
    }

    let fd = proxy
        .open_pipe_wire_remote(&session, OpenPipeWireRemoteOptions::default())
        .await?;

    std::mem::forget(session);
    std::mem::forget(proxy);

    Ok(Portal { fd, streams: infos })
}

struct FmtData {
    format: spa::param::video::VideoInfoRaw,
}

fn run_pw_loop(
    fd: OwnedFd,
    infos: Vec<StreamInfo>,
    latest: Arc<Mutex<Vec<Slot>>>,
    states: Arc<Mutex<Vec<StreamState>>>,
    quit_rx: pw::channel::Receiver<()>,
) -> Result<()> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None).context("pipewire main loop")?;
    let context = pw::context::ContextRc::new(&mainloop, None).context("pipewire context")?;
    let core = context
        .connect_fd_rc(fd, None)
        .context("pipewire connect_fd")?;

    let _quit = quit_rx.attach(mainloop.loop_(), {
        let mainloop = mainloop.clone();
        move |_| mainloop.quit()
    });

    let mut streams = Vec::new();
    let mut listeners = Vec::new();

    for (idx, info) in infos.iter().enumerate() {
        let stream = pw::stream::StreamRc::new(
            core.clone(),
            "rayshot-capture",
            properties! {
                *pw::keys::MEDIA_TYPE => "Video",
                *pw::keys::MEDIA_CATEGORY => "Capture",
                *pw::keys::MEDIA_ROLE => "Screen",
            },
        )
        .context("pipewire stream")?;

        let listener = stream
            .add_local_listener_with_user_data(FmtData {
                format: Default::default(),
            })
            .param_changed(|_, ud, id, param| {
                let Some(param) = param else {
                    return;
                };
                if id != pw::spa::param::ParamType::Format.as_raw() {
                    return;
                }
                let Ok((media_type, media_subtype)) =
                    pw::spa::param::format_utils::parse_format(param)
                else {
                    return;
                };
                if media_type != pw::spa::param::format::MediaType::Video
                    || media_subtype != pw::spa::param::format::MediaSubtype::Raw
                {
                    return;
                }
                let _ = ud.format.parse(param);
            })
            .state_changed({
                let states = states.clone();
                move |_, _, _old, new| {
                    if let Ok(mut st) = states.lock() {
                        if idx < st.len() {
                            st[idx] = new;
                        }
                    }
                }
            })
            .process({
                let latest = latest.clone();
                let last_copy = std::cell::Cell::new(
                    Instant::now()
                        .checked_sub(Duration::from_secs(1))
                        .unwrap_or_else(Instant::now),
                );
                move |stream, ud| {
                    let Some(mut buffer) = stream.dequeue_buffer() else {
                        return;
                    };
                    let now = Instant::now();
                    if now.duration_since(last_copy.get()) < Duration::from_millis(66) {
                        return;
                    }
                    let datas = buffer.datas_mut();
                    if datas.is_empty() {
                        return;
                    }
                    let w = ud.format.size().width;
                    let h = ud.format.size().height;
                    if w == 0 || h == 0 {
                        return;
                    }
                    let stride = datas[0].chunk().stride().max(0) as usize;
                    let row = stride.max(w as usize * 4);
                    let Some(src) = datas[0].data() else {
                        return;
                    };
                    if src.is_empty() {
                        return;
                    }
                    let len = (row * h as usize).min(src.len());
                    let mut slots = latest.lock().unwrap();
                    let s = &mut slots[idx];
                    s.raw.clear();
                    s.raw.extend_from_slice(&src[..len]);
                    s.w = w;
                    s.h = h;
                    s.stride = row;
                    s.format = ud.format.format();
                    s.present = true;
                    last_copy.set(now);
                }
            })
            .register()
            .context("register pipewire listener")?;

        let obj = pw::spa::pod::object!(
            pw::spa::utils::SpaTypes::ObjectParamFormat,
            pw::spa::param::ParamType::EnumFormat,
            pw::spa::pod::property!(
                pw::spa::param::format::FormatProperties::MediaType,
                Id,
                pw::spa::param::format::MediaType::Video
            ),
            pw::spa::pod::property!(
                pw::spa::param::format::FormatProperties::MediaSubtype,
                Id,
                pw::spa::param::format::MediaSubtype::Raw
            ),
            pw::spa::pod::property!(
                pw::spa::param::format::FormatProperties::VideoFormat,
                Choice,
                Enum,
                Id,
                pw::spa::param::video::VideoFormat::BGRx,
                pw::spa::param::video::VideoFormat::BGRx,
                pw::spa::param::video::VideoFormat::RGBx,
                pw::spa::param::video::VideoFormat::BGRA,
                pw::spa::param::video::VideoFormat::RGBA,
            ),
            pw::spa::pod::property!(
                pw::spa::param::format::FormatProperties::VideoSize,
                Choice,
                Range,
                Rectangle,
                pw::spa::utils::Rectangle {
                    width: 1920,
                    height: 1080
                },
                pw::spa::utils::Rectangle {
                    width: 1,
                    height: 1
                },
                pw::spa::utils::Rectangle {
                    width: 8192,
                    height: 8192
                }
            ),
        );
        let values: Vec<u8> = pw::spa::pod::serialize::PodSerializer::serialize(
            std::io::Cursor::new(Vec::new()),
            &pw::spa::pod::Value::Object(obj),
        )
        .map_err(|e| anyhow!("serialize pod: {e:?}"))?
        .0
        .into_inner();

        let buffers = pw::spa::pod::Value::Object(pw::spa::pod::Object {
            type_: pw::spa::sys::SPA_TYPE_OBJECT_ParamBuffers,
            id: pw::spa::sys::SPA_PARAM_Buffers,
            properties: vec![pw::spa::pod::Property::new(
                pw::spa::sys::SPA_PARAM_BUFFERS_buffers,
                pw::spa::pod::Value::Int(4),
            )],
        });
        let bvalues: Vec<u8> = pw::spa::pod::serialize::PodSerializer::serialize(
            std::io::Cursor::new(Vec::new()),
            &buffers,
        )
        .map_err(|e| anyhow!("serialize buffers pod: {e:?}"))?
        .0
        .into_inner();

        let mut params = [
            Pod::from_bytes(&values).ok_or_else(|| anyhow!("pod from_bytes"))?,
            Pod::from_bytes(&bvalues).ok_or_else(|| anyhow!("buffers pod from_bytes"))?,
        ];

        stream
            .connect(
                spa::utils::Direction::Input,
                Some(info.node_id),
                pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
                &mut params,
            )
            .context("connect pipewire stream")?;

        streams.push(stream);
        listeners.push(listener);
    }

    let _keep = Rc::new(RefCell::new((streams, listeners)));
    mainloop.run();
    Ok(())
}

fn to_rgba(
    src: &[u8],
    w: u32,
    h: u32,
    row_bytes: usize,
    format: spa::param::video::VideoFormat,
) -> Vec<u8> {
    use spa::param::video::VideoFormat as F;
    let (w, h) = (w as usize, h as usize);
    let mut out = vec![0u8; w * h * 4];
    let swap = matches!(format, F::BGRx | F::BGRA);
    let has_alpha = matches!(format, F::RGBA | F::BGRA);
    for y in 0..h {
        let start = y * row_bytes;
        if start >= src.len() {
            break;
        }
        let row = &src[start..];
        let o = y * w * 4;
        for x in 0..w {
            let i = x * 4;
            if i + 3 >= row.len() {
                break;
            }
            let (r, g, b) = if swap {
                (row[i + 2], row[i + 1], row[i])
            } else {
                (row[i], row[i + 1], row[i + 2])
            };
            out[o + x * 4] = r;
            out[o + x * 4 + 1] = g;
            out[o + x * 4 + 2] = b;
            out[o + x * 4 + 3] = if has_alpha { row[i + 3] } else { 255 };
        }
    }
    out
}

fn composite(slots: &[Captured]) -> Result<Frame> {
    if slots.iter().any(|c| c.w == 0 || c.h == 0) {
        return Err(anyhow!("screencast stream produced an empty frame"));
    }
    let min_x = slots.iter().map(|c| c.pos.0).min().unwrap_or(0);
    let min_y = slots.iter().map(|c| c.pos.1).min().unwrap_or(0);
    let max_x = slots.iter().map(|c| c.pos.0 + c.w as i32).max().unwrap_or(0);
    let max_y = slots.iter().map(|c| c.pos.1 + c.h as i32).max().unwrap_or(0);
    let fw = (max_x - min_x).max(1) as u32;
    let fh = (max_y - min_y).max(1) as u32;
    let mut rgba = vec![0u8; fw as usize * fh as usize * 4];

    for c in slots.iter() {
        let ox = (c.pos.0 - min_x) as usize;
        let oy = (c.pos.1 - min_y) as usize;
        for y in 0..c.h as usize {
            let dst_y = oy + y;
            if dst_y >= fh as usize {
                break;
            }
            let src_row = &c.pixels[y * c.w as usize * 4..];
            let dst = (dst_y * fw as usize + ox) * 4;
            let copy_w = (c.w as usize).min(fw as usize - ox) * 4;
            rgba[dst..dst + copy_w].copy_from_slice(&src_row[..copy_w]);
        }
    }

    Ok(Frame {
        width: fw,
        height: fh,
        rgba,
    })
}
