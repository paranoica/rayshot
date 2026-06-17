use std::cell::RefCell;
use std::os::fd::OwnedFd;
use std::rc::Rc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use pipewire as pw;
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

pub fn capture_once(rt: &Handle) -> Result<Frame> {
    let portal = rt
        .block_on(open_screencast())
        .context("screencast portal handshake failed")?;
    pw_capture(portal)
}

async fn open_screencast() -> Result<Portal> {
    use ashpd::desktop::PersistMode;
    use ashpd::desktop::screencast::{
        CursorMode, OpenPipeWireRemoteOptions, Screencast, SelectSourcesOptions, SourceType,
        StartCastOptions,
    };

    let proxy = Screencast::new().await?;
    let session = proxy.create_session(Default::default()).await?;

    proxy
        .select_sources(
            &session,
            SelectSourcesOptions::default()
                .set_multiple(true)
                .set_cursor_mode(CursorMode::Hidden)
                .set_sources(ashpd::enumflags2::BitFlags::from(SourceType::Monitor))
                .set_persist_mode(PersistMode::Application),
        )
        .await?
        .response()?;

    let streams = proxy
        .start(&session, None, StartCastOptions::default())
        .await?
        .response()?;

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

struct Captured {
    pixels: Vec<u8>,
    w: u32,
    h: u32,
    pos: (i32, i32),
    done: bool,
}

struct FmtData {
    format: spa::param::video::VideoInfoRaw,
}

fn pw_capture(portal: Portal) -> Result<Frame> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None).context("pipewire main loop")?;
    let context = pw::context::ContextRc::new(&mainloop, None).context("pipewire context")?;
    let core = context
        .connect_fd_rc(portal.fd, None)
        .context("pipewire connect_fd")?;

    let collector = Rc::new(RefCell::new(
        portal
            .streams
            .iter()
            .map(|s| Captured {
                pixels: Vec::new(),
                w: 0,
                h: 0,
                pos: s.pos,
                done: false,
            })
            .collect::<Vec<_>>(),
    ));
    let total = portal.streams.len();

    let mut streams = Vec::new();
    let mut listeners = Vec::new();

    for (idx, info) in portal.streams.iter().enumerate() {
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
            .process({
                let collector = collector.clone();
                let mainloop = mainloop.clone();
                move |stream, ud| {
                    let Some(mut buffer) = stream.dequeue_buffer() else {
                        return;
                    };
                    {
                        let mut slots = collector.borrow_mut();
                        if slots[idx].done {
                            return;
                        }
                        let datas = buffer.datas_mut();
                        if datas.is_empty() {
                            return;
                        }
                        let w = ud.format.size().width;
                        let h = ud.format.size().height;
                        let stride = datas[0].chunk().stride().max(0) as usize;
                        let Some(src) = datas[0].data() else {
                            return;
                        };
                        if w == 0 || h == 0 || src.is_empty() {
                            return;
                        }
                        let row_bytes = stride.max(w as usize * 4);
                        let rgba = to_rgba(src, w, h, row_bytes, ud.format.format());
                        slots[idx] = Captured {
                            pixels: rgba,
                            w,
                            h,
                            pos: slots[idx].pos,
                            done: true,
                        };
                    }
                    let done = collector.borrow().iter().filter(|c| c.done).count();
                    if done >= total {
                        mainloop.quit();
                    }
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
        let mut params = [Pod::from_bytes(&values).ok_or_else(|| anyhow!("pod from_bytes"))?];

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

    let timed_out = Rc::new(RefCell::new(false));
    let timer = mainloop.loop_().add_timer({
        let mainloop = mainloop.clone();
        let timed_out = timed_out.clone();
        move |_| {
            *timed_out.borrow_mut() = true;
            mainloop.quit();
        }
    });
    timer
        .update_timer(Some(Duration::from_secs(3)), None)
        .into_result()
        .ok();

    mainloop.run();

    if *timed_out.borrow() {
        return Err(anyhow!("timed out waiting for screencast frames"));
    }

    composite(collector)
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
        let row = &src[y * row_bytes..];
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

fn composite(collector: Rc<RefCell<Vec<Captured>>>) -> Result<Frame> {
    let slots = collector.borrow();
    if slots.iter().any(|c| !c.done) {
        return Err(anyhow!("not all screencast streams produced a frame"));
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
