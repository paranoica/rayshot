mod anim;
mod capture;
mod export;
mod hotkey;
mod overlay;
mod scene;

use anyhow::{Context, Result};

fn main() -> Result<()> {
    // Hotkey setup subcommands (no capture/overlay needed).
    match std::env::args().nth(1).as_deref() {
        Some("install-hotkey") => {
            let binding = std::env::args()
                .nth(2)
                .unwrap_or_else(|| "Print".to_string());
            return hotkey::install(&binding);
        }
        Some("uninstall-hotkey") => return hotkey::uninstall(),
        Some("monitors") => return overlay::list_monitors(),
        _ => {}
    }
    let shot = std::env::args().nth(1).as_deref() == Some("shot");

    export::cleanup_old(&export::scratch_dir(), export::SCRATCH_MAX_AGE);

    let _anim = (!shot).then(anim::AnimationGuard::disable);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime")?;

    let frame = rt
        .block_on(capture::capture_frame())
        .context("desktop capture failed")?;
    println!("captured frame: {}x{}", frame.width, frame.height);

    if shot {
        let path = std::env::args()
            .nth(2)
            .unwrap_or_else(|| "/tmp/rayshot-capture.png".to_string());
        let img = image::RgbaImage::from_raw(frame.width, frame.height, frame.rgba)
            .context("frame buffer size mismatch")?;
        img.save(&path).context("failed to save capture")?;
        println!("saved capture to {path}");
        return Ok(());
    }

    overlay::run(frame, rt.handle().clone()).context("overlay failed")?;
    Ok(())
}
