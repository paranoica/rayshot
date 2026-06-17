use anyhow::{Context, Result, anyhow};
use ashpd::desktop::screenshot::Screenshot;

pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
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
