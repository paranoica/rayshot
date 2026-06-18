use anyhow::{Context, Result, anyhow};
use ashpd::desktop::screenshot::Screenshot;

pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub fn write_frame_raw(frame: &Frame, path: &std::path::Path) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let mut buf = Vec::with_capacity(8 + frame.rgba.len());
    buf.extend_from_slice(&frame.width.to_le_bytes());
    buf.extend_from_slice(&frame.height.to_le_bytes());
    buf.extend_from_slice(&frame.rgba);
    std::fs::write(path, buf).with_context(|| format!("write frame {}", path.display()))?;
    Ok(())
}

pub fn read_frame_raw(path: &std::path::Path) -> Result<Frame> {
    let data = std::fs::read(path).with_context(|| format!("read frame {}", path.display()))?;
    let _ = std::fs::remove_file(path);
    if data.len() < 8 {
        return Err(anyhow!("frame file too small"));
    }
    let width = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    let height = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    let rgba = data[8..].to_vec();
    if rgba.len() != width as usize * height as usize * 4 {
        return Err(anyhow!("frame file size mismatch"));
    }
    Ok(Frame {
        width,
        height,
        rgba,
    })
}

pub async fn capture_frame() -> Result<Frame> {
    let t0 = std::time::Instant::now();
    let response = Screenshot::request()
        .interactive(false)
        .modal(false)
        .send()
        .await
        .context("portal Screenshot request failed")?
        .response()
        .context("portal returned an error response")?;
    let t_portal = t0.elapsed();

    let path = url::Url::parse(response.uri().as_str())
        .context("portal uri is not a valid url")?
        .to_file_path()
        .map_err(|_| anyhow!("portal uri is not a local file path"))?;

    let t1 = std::time::Instant::now();
    let img = image::open(&path)
        .context("failed to decode the screenshot file")?
        .to_rgba8();
    let t_decode = t1.elapsed();
    let _ = std::fs::remove_file(&path);
    let (width, height) = img.dimensions();

    eprintln!(
        "[rayshot] capture: portal {:?}, decode {:?}",
        t_portal, t_decode
    );

    Ok(Frame {
        width,
        height,
        rgba: img.into_raw(),
    })
}
