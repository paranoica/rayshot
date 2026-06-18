mod anim;
mod capture;
mod export;
mod hotkey;
mod overlay;
mod scene;
mod screencast;

use anyhow::{Context, Result};

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
        Some("monitors") => return overlay::list_monitors(),
        Some("daemon") => {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .context("failed to build tokio runtime")?;
            return overlay::run_daemon(rt.handle().clone());
        }
        Some("overlay") => {
            let path = std::env::args()
                .nth(2)
                .context("overlay needs a frame file path")?;
            let frame = capture::read_frame_raw(std::path::Path::new(&path))
                .context("failed to read frame file")?;
            let _anim = anim::AnimationGuard::disable();
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .context("failed to build tokio runtime")?;
            return overlay::run_with_frame(frame, rt.handle().clone());
        }
        _ => {}
    }

    if std::env::args().nth(1).as_deref() == Some("close") {
        overlay::close_remote()?;
        return Ok(());
    }

    if std::env::args().nth(1).as_deref() == Some("trigger") && overlay::trigger()? {
        return Ok(());
    }

    let shot_pw = std::env::args().nth(1).as_deref() == Some("shot-pw");
    let shot = std::env::args().nth(1).as_deref() == Some("shot");

    let interactive = !shot && !shot_pw;

    if interactive && !acquire_single_instance() {
        return Ok(());
    }

    export::cleanup_old(&export::scratch_dir(), export::SCRATCH_MAX_AGE);

    let _anim = interactive.then(anim::AnimationGuard::disable);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime")?;

    if shot_pw {
        let t0 = std::time::Instant::now();
        let frame = screencast::capture_once(rt.handle()).context("screencast capture failed")?;
        eprintln!(
            "[rayshot] screencast capture: {:?} ({}x{})",
            t0.elapsed(),
            frame.width,
            frame.height
        );
        let path = std::env::args()
            .nth(2)
            .unwrap_or_else(|| "/tmp/rayshot-pw.png".to_string());
        let img = image::RgbaImage::from_raw(frame.width, frame.height, frame.rgba)
            .context("frame buffer size mismatch")?;
        img.save(&path).context("failed to save capture")?;
        println!("saved screencast capture to {path}");
        return Ok(());
    }

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
