use anyhow::{Context, Result, anyhow};
use ashpd::desktop::screenshot::Screenshot;

pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
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
    let data = std::fs::read(path).with_context(|| format!("read frame {}", path.display()))?;
    let _ = std::fs::remove_file(path);
    if !data.starts_with(&STRIDED_MAGIC) {
        return Err(anyhow!("not an RSF1 frame file"));
    }
    decode_strided(&data)
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
    fn rejects_png_frames_now_that_only_rsf1_is_written() {
        let path = temp_path("frame.png");
        image::RgbaImage::from_raw(2, 1, PIXELS.to_vec())
            .unwrap()
            .save_with_format(&path, image::ImageFormat::Png)
            .unwrap();
        assert!(read_frame_file(&path).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn rejects_the_old_daemon_raw_format() {
        let path = temp_path("frame.raw");
        let mut data = Vec::new();
        data.extend_from_slice(&2u32.to_le_bytes());
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&PIXELS);
        std::fs::write(&path, data).unwrap();
        assert!(read_frame_file(&path).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn rejects_file_shorter_than_magic() {
        let path = temp_path("tiny.rsf");
        std::fs::write(&path, [b'R', b'S', b'F']).unwrap();
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
