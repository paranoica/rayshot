use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use image::RgbaImage;

pub const SCRATCH_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

pub fn scratch_dir() -> PathBuf {
    if let Some(rt) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(rt).join("rayshot");
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join(".cache/rayshot/scratch")
}

fn last_selection_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join(".cache/rayshot/last_selection")
}

fn remember_selection() -> bool {
    std::env::var_os("RAYSHOT_NO_REMEMBER_SELECTION").is_none()
}

pub fn save_last_selection(x: f32, y: f32, w: f32, h: f32) {
    save_last_selection_to(remember_selection(), &last_selection_path(), x, y, w, h);
}

fn save_last_selection_to(remember: bool, path: &Path, x: f32, y: f32, w: f32, h: f32) {
    if !remember {
        return;
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, format!("{x},{y},{w},{h}"));
}

pub fn load_last_selection() -> Option<(f32, f32, f32, f32)> {
    load_last_selection_from(remember_selection(), &last_selection_path())
}

fn load_last_selection_from(remember: bool, path: &Path) -> Option<(f32, f32, f32, f32)> {
    if !remember {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    let v: Vec<f32> = text
        .trim()
        .split(',')
        .filter_map(|s| s.parse().ok())
        .collect();
    match v.as_slice() {
        [x, y, w, h] if *w > 0.0 && *h > 0.0 => Some((*x, *y, *w, *h)),
        _ => None,
    }
}

pub fn cleanup_old(dir: &Path, max_age: Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        if let Ok(modified) = meta.modified() {
            if now.duration_since(modified).is_ok_and(|age| age > max_age) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

fn timestamped_filename() -> String {
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        libc::localtime_r(&secs, &mut tm);
    }
    format!(
        "rayshot-{:02}-{:02}-{:04}-{:02}-{:02}-{:02}.png",
        tm.tm_mday,
        tm.tm_mon + 1,
        tm.tm_year + 1900,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
    )
}

fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let base = dir.join(name);
    if !base.exists() {
        return base;
    }
    let stem = name.strip_suffix(".png").unwrap_or(name);
    for n in 2..1000 {
        let p = dir.join(format!("{stem}-{n}.png"));
        if !p.exists() {
            return p;
        }
    }
    base
}

pub fn to_png_bytes(img: &RgbaImage) -> Result<Vec<u8>> {
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png)
        .context("PNG encode failed")?;
    Ok(buf.into_inner())
}

pub fn save_to_scratch(png: &[u8]) -> Result<PathBuf> {
    let dir = scratch_dir();
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let path = unique_path(&dir, &timestamped_filename());
    std::fs::write(&path, png).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

pub fn save_to_pictures(png: &[u8]) -> Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    let pictures = home.join("Pictures");
    let dir = if pictures.is_dir() {
        pictures
    } else {
        PathBuf::from("/tmp")
    };
    let path = unique_path(&dir, &timestamped_filename());
    std::fs::write(&path, png).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

pub fn copy_png_to_clipboard(png: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new("wl-copy")
        .args(["--type", "image/png"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to spawn wl-copy (is wl-clipboard installed?)")?;
    child
        .stdin
        .take()
        .context("wl-copy stdin unavailable")?
        .write_all(png)
        .context("failed to write image to wl-copy")?;
    let status = child.wait().context("failed to wait for wl-copy")?;
    if !status.success() {
        anyhow::bail!("wl-copy exited with {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn temp_file() -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("rayshot-sel-test-{}-{n}", std::process::id()))
    }

    #[test]
    fn load_returns_saved_selection_when_remembered() {
        let path = temp_file();
        save_last_selection_to(true, &path, 10.0, 20.0, 30.0, 40.0);
        assert_eq!(
            load_last_selection_from(true, &path),
            Some((10.0, 20.0, 30.0, 40.0))
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_skips_restore_when_disabled_even_if_file_exists() {
        let path = temp_file();
        save_last_selection_to(true, &path, 10.0, 20.0, 30.0, 40.0);
        assert_eq!(load_last_selection_from(false, &path), None);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn save_writes_nothing_when_disabled() {
        let path = temp_file();
        save_last_selection_to(false, &path, 10.0, 20.0, 30.0, 40.0);
        assert!(!path.exists());
    }

    #[test]
    fn load_rejects_zero_or_negative_size() {
        let path = temp_file();
        std::fs::write(&path, "10,20,0,40").unwrap();
        assert_eq!(load_last_selection_from(true, &path), None);
        std::fs::write(&path, "10,20,30,0").unwrap();
        assert_eq!(load_last_selection_from(true, &path), None);
        std::fs::write(&path, "10,20,-5,40").unwrap();
        assert_eq!(load_last_selection_from(true, &path), None);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_rejects_malformed() {
        let path = temp_file();
        std::fs::write(&path, "").unwrap();
        assert_eq!(load_last_selection_from(true, &path), None);
        std::fs::write(&path, "10,20,30").unwrap();
        assert_eq!(load_last_selection_from(true, &path), None);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_missing_file_is_none() {
        let path = temp_file();
        let _ = std::fs::remove_file(&path);
        assert_eq!(load_last_selection_from(true, &path), None);
    }
}
