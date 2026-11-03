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

pub const STRIDED_MAGIC: [u8; 4] = *b"RSF1";

fn decode_strided(data: &[u8]) -> Result<Frame> {
    let field = |i: usize| -> Result<u32> {
        let b = data
            .get(i..i + 4)
            .context("strided frame header cut short")?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let (width, height, stride) = (field(4)?, field(8)?, field(12)?);
    if width == 0 || height == 0 {
        return Err(anyhow!("strided frame has zero size"));
    }
    let row = (width as usize)
        .checked_mul(4)
        .context("strided frame too large")?;
    let stride = stride as usize;
    if stride < row {
        return Err(anyhow!(
            "strided frame stride {stride} below row size {row}"
        ));
    }
    // The last row may be unpadded, as GdkPixbuf leaves it.
    let needed = (height as usize - 1)
        .checked_mul(stride)
        .and_then(|n| n.checked_add(row))
        .context("strided frame too large")?;
    let body = &data[16..];
    if body.len() < needed {
        return Err(anyhow!(
            "strided frame truncated: {} < {needed}",
            body.len()
        ));
    }
    let mut rgba = Vec::with_capacity(row * height as usize);
    for y in 0..height as usize {
        rgba.extend_from_slice(&body[y * stride..y * stride + row]);
    }
    Ok(Frame {
        width,
        height,
        rgba,
    })
}

pub fn read_frame_file(path: &std::path::Path) -> Result<Frame> {
    const PNG_MAGIC: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let data = std::fs::read(path).with_context(|| format!("read frame {}", path.display()))?;
    let _ = std::fs::remove_file(path);
    if data.len() >= 4 && data[..4] == STRIDED_MAGIC {
        return decode_strided(&data);
    }
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

    fn strided(w: u32, h: u32, stride: u32, body: &[u8]) -> Vec<u8> {
        let mut d = STRIDED_MAGIC.to_vec();
        d.extend_from_slice(&w.to_le_bytes());
        d.extend_from_slice(&h.to_le_bytes());
        d.extend_from_slice(&stride.to_le_bytes());
        d.extend_from_slice(body);
        d
    }

    const ROW0: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];
    const ROW1: [u8; 8] = [9, 10, 11, 12, 13, 14, 15, 16];
    const PAD: [u8; 4] = [0xEE; 4];

    fn write_tmp(name: &str, data: &[u8]) -> std::path::PathBuf {
        let path = temp_path(name);
        std::fs::write(&path, data).unwrap();
        path
    }

    #[test]
    fn reads_strided_frame_with_unpadded_last_row() {
        let body = [&ROW0[..], &PAD, &ROW1].concat();
        let path = write_tmp("strided-last.rsf", &strided(2, 2, 12, &body));
        let frame = read_frame_file(&path).unwrap();
        assert_eq!((frame.width, frame.height), (2, 2));
        assert_eq!(frame.rgba, [ROW0, ROW1].concat());
        assert!(!path.exists());
    }

    #[test]
    fn reads_strided_frame_with_padded_last_row() {
        let body = [&ROW0[..], &PAD, &ROW1, &PAD].concat();
        let path = write_tmp("strided-full.rsf", &strided(2, 2, 12, &body));
        let frame = read_frame_file(&path).unwrap();
        assert_eq!(frame.rgba, [ROW0, ROW1].concat());
    }

    #[test]
    fn reads_strided_frame_without_padding() {
        let body = [ROW0, ROW1].concat();
        let path = write_tmp("strided-tight.rsf", &strided(2, 2, 8, &body));
        let frame = read_frame_file(&path).unwrap();
        assert_eq!(frame.rgba, body);
    }

    #[test]
    fn rejects_strided_stride_smaller_than_row() {
        let body = [ROW0, ROW1].concat();
        let path = write_tmp("strided-narrow.rsf", &strided(2, 2, 4, &body));
        assert!(read_frame_file(&path).is_err());
    }

    #[test]
    fn rejects_truncated_strided_frame() {
        let body = [&ROW0[..], &PAD, &ROW1[..7]].concat();
        let path = write_tmp("strided-short.rsf", &strided(2, 2, 12, &body));
        assert!(read_frame_file(&path).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn rejects_strided_zero_size() {
        let path = write_tmp("strided-zero.rsf", &strided(0, 2, 0, &[]));
        assert!(read_frame_file(&path).is_err());
    }

    #[test]
    fn rejects_strided_header_cut_short() {
        let mut d = strided(2, 2, 8, &[]);
        d.truncate(10);
        let path = write_tmp("strided-hdr.rsf", &d);
        assert!(read_frame_file(&path).is_err());
    }

    #[test]
    fn rejects_strided_overflowing_dimensions_without_panic() {
        let path = write_tmp(
            "strided-huge.rsf",
            &strided(u32::MAX, u32::MAX, u32::MAX, &ROW0),
        );
        assert!(read_frame_file(&path).is_err());
    }

    #[test]
    fn rejects_empty_file() {
        let path = temp_path("empty.raw");
        std::fs::write(&path, []).unwrap();
        assert!(read_frame_file(&path).is_err());
    }
}
