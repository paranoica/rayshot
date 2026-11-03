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

pub fn read_frame_file(path: &std::path::Path) -> Result<Frame> {
    const PNG_MAGIC: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let data = std::fs::read(path).with_context(|| format!("read frame {}", path.display()))?;
    let _ = std::fs::remove_file(path);
    if data.len() >= 8 && data[..8] == PNG_MAGIC {
        let img = image::load_from_memory(&data)
            .context("failed to decode png frame")?
            .into_rgba8();
        let (width, height) = img.dimensions();
        return Ok(Frame {
            width,
            height,
            rgba: img.into_raw(),
        });
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("rayshot-test-{}-{name}", std::process::id()))
    }

    const PIXELS: [u8; 8] = [255, 0, 0, 255, 0, 128, 255, 64];

    #[test]
    fn reads_png_frame_and_removes_file() {
        let path = temp_path("frame.png");
        image::RgbaImage::from_raw(2, 1, PIXELS.to_vec())
            .unwrap()
            .save_with_format(&path, image::ImageFormat::Png)
            .unwrap();
        let frame = read_frame_file(&path).unwrap();
        assert_eq!((frame.width, frame.height), (2, 1));
        assert_eq!(frame.rgba, PIXELS);
        assert!(!path.exists());
    }

    #[test]
    fn reads_raw_frame_and_removes_file() {
        let path = temp_path("frame.raw");
        let src = Frame {
            width: 2,
            height: 1,
            rgba: PIXELS.to_vec(),
        };
        write_frame_raw(&src, &path).unwrap();
        let frame = read_frame_file(&path).unwrap();
        assert_eq!((frame.width, frame.height), (2, 1));
        assert_eq!(frame.rgba, PIXELS);
        assert!(!path.exists());
    }

    #[test]
    fn rejects_corrupt_png_instead_of_reading_it_as_raw() {
        let path = temp_path("corrupt.png");
        let mut data = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        data.extend_from_slice(&[0u8; 32]);
        std::fs::write(&path, data).unwrap();
        let Err(err) = read_frame_file(&path) else {
            panic!("corrupt png was accepted");
        };
        assert!(format!("{err:#}").contains("decode png"), "{err:#}");
        assert!(!path.exists());
    }

    #[test]
    fn rejects_raw_size_mismatch() {
        let path = temp_path("short.raw");
        let mut data = Vec::new();
        data.extend_from_slice(&2u32.to_le_bytes());
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&[0u8; 7]);
        std::fs::write(&path, data).unwrap();
        assert!(read_frame_file(&path).is_err());
    }

    #[test]
    fn rejects_file_shorter_than_header() {
        let path = temp_path("tiny.raw");
        std::fs::write(&path, [1u8, 2, 3]).unwrap();
        assert!(read_frame_file(&path).is_err());
    }

    #[test]
    fn rejects_empty_file() {
        let path = temp_path("empty.raw");
        std::fs::write(&path, []).unwrap();
        assert!(read_frame_file(&path).is_err());
    }
}
