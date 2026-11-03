mod anim;
mod capture;
mod export;
mod hotkey;
mod live;
mod overlay;
mod scene;
mod warm;

use anyhow::{Context, Result};

const PROTOCOL_MISMATCH_EXIT: i32 = 3;

fn acquire_single_instance() -> bool {
    use std::os::unix::io::AsRawFd;
    let dir = export::scratch_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("instance.lock");
    let file = match std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
    {
        Ok(f) => f,
        Err(_) => return true,
    };
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc != 0 {
        return false;
    }
    std::mem::forget(file);
    true
}

fn main() -> Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("install-hotkey") => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            let daemon = args.iter().any(|a| a == "--daemon");
            let binding = args
                .iter()
                .find(|a| !a.starts_with("--"))
                .cloned()
                .unwrap_or_else(|| "Print".to_string());
            return hotkey::install(&binding, daemon);
        }
        Some("uninstall-hotkey") => return hotkey::uninstall(),
        Some("warm") => return warm::run(std::env::args().nth(2).as_deref()),
        Some("monitors") => return overlay::list_monitors(),
        Some("daemon") => return hotkey::retire_daemon(),
        Some("serve") => {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .context("failed to build tokio runtime")?;
            return overlay::run_serve(rt.handle().clone());
        }
        Some("overlay") if std::env::args().nth(2).as_deref() == Some("--live") => {
            let dim = |i: usize| -> Result<u32> {
                std::env::args()
                    .nth(i)
                    .context("overlay --live needs <width> <height>")?
                    .parse()
                    .context("overlay --live size must be a number")
            };
            let (width, height) = (dim(3)?, dim(4)?);
            if std::env::args().nth(5).as_deref() == Some("--protocol") {
                let theirs: u32 = std::env::args()
                    .nth(6)
                    .and_then(|v| v.parse().ok())
                    .context("--protocol needs a number")?;
                if theirs != live::PROTOCOL_VERSION {
                    eprintln!(
                        "[rayshot] extension speaks protocol {theirs}, this binary {}",
                        live::PROTOCOL_VERSION
                    );
                    std::process::exit(PROTOCOL_MISMATCH_EXIT);
                }
            }
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .context("failed to build tokio runtime")?;
            return overlay::run_live(width, height, rt.handle().clone());
        }
        // `trigger` was the old daemon hotkey command; it now just captures.
        Some(cmd) if !matches!(cmd, "trigger" | "shot") => {
            anyhow::bail!("unknown command {cmd:?}");
        }
        _ => {}
    }

    let shot = std::env::args().nth(1).as_deref() == Some("shot");
    let interactive = !shot;

    if interactive && !acquire_single_instance() {
        return Ok(());
    }

    export::cleanup_old(&export::scratch_dir(), export::SCRATCH_MAX_AGE);

    let _anim = interactive.then(anim::AnimationGuard::disable);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime")?;

    if shot {
        let frame = rt
            .block_on(capture::capture_frame())
            .context("desktop capture failed")?;
        println!("captured frame: {}x{}", frame.width, frame.height);
        let path = std::env::args()
            .nth(2)
            .unwrap_or_else(|| "/tmp/rayshot-capture.png".to_string());
        let img = image::RgbaImage::from_raw(frame.width, frame.height, frame.rgba)
            .context("frame buffer size mismatch")?;
        img.save(&path).context("failed to save capture")?;
        println!("saved capture to {path}");
        return Ok(());
    }

    overlay::run(rt.handle().clone()).context("overlay failed")?;
    Ok(())
}
