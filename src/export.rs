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
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // Only screenshot copies: lock files and frames share this folder.
        if !(meta.is_file() && name.starts_with("rayshot-") && name.ends_with(".png")) {
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

/// The folder from `user-dirs.dirs`, which is localized (`~/Изображения`,
/// `~/Bilder`). Values are double-quoted shell strings; "$HOME" alone means
/// the user turned the folder off.
fn configured_pictures_dir(home: &Path, user_dirs: &str) -> Option<PathBuf> {
    let raw = user_dirs
        .lines()
        .rev()
        .find_map(|line| line.trim().strip_prefix("XDG_PICTURES_DIR="))?;
    let mut chars = raw.strip_prefix('"')?.chars();
    let mut value = String::new();
    loop {
        match chars.next()? {
            '"' => break,
            '\\' => value.push(chars.next()?),
            c => value.push(c),
        }
    }
    let dir = match value.strip_prefix("$HOME/") {
        Some(rest) => home.join(rest.trim_matches('/')),
        None if value.starts_with('/') => PathBuf::from(&value),
        None => return None,
    };
    let usable = dir != home
        && dir.parent().is_some()
        && !dir
            .components()
            .any(|c| c == std::path::Component::ParentDir);
    usable.then_some(dir)
}

/// Configured folder, then ~/Pictures, then /tmp. Only a folder inside the
/// home is created; elsewhere it could land on an unmounted drive.
fn pictures_dir(
    home: Option<&Path>,
    user_dirs: Option<&str>,
    exists: &dyn Fn(&Path) -> bool,
    create: &dyn Fn(&Path) -> bool,
) -> PathBuf {
    let Some(home) = home.filter(|h| h.is_absolute() && h.parent().is_some()) else {
        return PathBuf::from("/tmp");
    };
    if let Some(dir) = user_dirs.and_then(|d| configured_pictures_dir(home, d))
        && (exists(&dir) || (dir.starts_with(home) && create(&dir)))
    {
        return dir;
    }
    let pictures = home.join("Pictures");
    if exists(&pictures) {
        pictures
    } else {
        PathBuf::from("/tmp")
    }
}

pub fn save_to_pictures(png: &[u8]) -> Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|h| h.is_absolute());
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| home.as_ref().map(|h| h.join(".config")));
    let user_dirs = config.and_then(|c| std::fs::read_to_string(c.join("user-dirs.dirs")).ok());
    let dir = pictures_dir(
        home.as_deref(),
        user_dirs.as_deref(),
        &|p| p.is_dir(),
        &|p| std::fs::create_dir_all(p).is_ok(),
    );
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
    // Without a data-control protocol wl-copy needs the keyboard focus, and
    // an unbounded wait would leave the overlay and the freeze up for good.
    let status = wait_bounded(&mut child, std::time::Duration::from_secs(3))
        .context("wl-copy did not finish")?;
    if !status.success() {
        anyhow::bail!("wl-copy exited with {status}");
    }
    Ok(())
}

fn wait_bounded(
    child: &mut std::process::Child,
    timeout: std::time::Duration,
) -> Result<std::process::ExitStatus> {
    let deadline = std::time::Instant::now() + timeout;
    let err = loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if std::time::Instant::now() < deadline => {}
            Ok(None) => break anyhow::anyhow!("gave up after {timeout:?}"),
            Err(e) => break e.into(),
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    let _ = child.kill();
    let _ = child.wait();
    Err(err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{Duration, Instant};

    #[test]
    fn bounded_wait_returns_the_status_of_a_quick_child() {
        let mut child = std::process::Command::new("sh")
            .args(["-c", "exit 3"])
            .spawn()
            .unwrap();
        let status = wait_bounded(&mut child, Duration::from_secs(5)).unwrap();
        assert_eq!(status.code(), Some(3));
    }

    #[test]
    fn bounded_wait_gives_up_on_a_hung_child_and_kills_it() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let started = Instant::now();
        assert!(wait_bounded(&mut child, Duration::from_millis(200)).is_err());
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(
            child.try_wait().unwrap().is_some(),
            "hung child left running"
        );
    }

    fn file_aged(dir: &Path, name: &str, age: Duration) -> PathBuf {
        let path = dir.join(name);
        let file = std::fs::File::create(&path).unwrap();
        file.set_modified(std::time::SystemTime::now() - age)
            .unwrap();
        path
    }

    #[test]
    fn cleanup_removes_only_old_screenshots() {
        let dir = temp_file();
        std::fs::create_dir_all(&dir).unwrap();
        let hour = Duration::from_secs(3600);
        let old_shot = file_aged(&dir, "rayshot-01-11-2026-10-00-00.png", hour);
        let old_dup = file_aged(&dir, "rayshot-01-11-2026-10-00-00-2.png", hour);
        let new_shot = file_aged(&dir, "rayshot-04-11-2026-10-00-00.png", Duration::ZERO);
        let lock = file_aged(&dir, "instance.lock", hour);
        let frame = file_aged(&dir, "frame-123.rsf", hour);
        let other = file_aged(&dir, "notes.png", hour);

        cleanup_old(&dir, Duration::from_secs(60));

        assert!(!old_shot.exists() && !old_dup.exists());
        assert!(new_shot.exists(), "fresh screenshot removed");
        assert!(lock.exists(), "a held lock file must survive");
        assert!(frame.exists(), "frames belong to the extension's sweep");
        assert!(other.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    const HOME: &str = "/home/u";

    fn pick(user_dirs: Option<&str>, existing: &[&str], creatable: &[&str]) -> PathBuf {
        pictures_dir(
            Some(Path::new(HOME)),
            user_dirs,
            &|p| existing.iter().any(|e| p == Path::new(e)),
            &|p| creatable.iter().any(|c| p == Path::new(c)),
        )
    }

    #[test]
    fn pictures_go_to_the_localized_folder_the_desktop_configured() {
        let dirs =
            "XDG_DESKTOP_DIR=\"$HOME/Рабочий стол\"\nXDG_PICTURES_DIR=\"$HOME/Изображения\"\n";
        assert_eq!(
            pick(Some(dirs), &["/home/u/Изображения"], &[]),
            PathBuf::from("/home/u/Изображения")
        );
    }

    #[test]
    fn a_missing_configured_folder_in_home_is_created() {
        let dirs = "XDG_PICTURES_DIR=\"$HOME/Изображения\"\n";
        assert_eq!(
            pick(Some(dirs), &[], &["/home/u/Изображения"]),
            PathBuf::from("/home/u/Изображения")
        );
    }

    #[test]
    fn a_configured_folder_that_cannot_be_made_falls_back_to_pictures_first() {
        let dirs = "XDG_PICTURES_DIR=\"$HOME/Изображения\"\n";
        assert_eq!(
            pick(Some(dirs), &["/home/u/Pictures"], &[]),
            PathBuf::from("/home/u/Pictures")
        );
    }

    #[test]
    fn an_absolute_folder_is_used_only_if_it_exists() {
        let dirs = "XDG_PICTURES_DIR=\"/data/pics\"\n";
        assert_eq!(
            pick(Some(dirs), &["/data/pics"], &[]),
            PathBuf::from("/data/pics")
        );
        assert_eq!(
            pick(Some(dirs), &[], &["/data/pics"]),
            PathBuf::from("/tmp")
        );
    }

    #[test]
    fn the_value_is_read_as_a_quoted_shell_string() {
        let cases = [
            ("XDG_PICTURES_DIR=\"$HOME/Pics\"  # mine\n", "/home/u/Pics"),
            (
                "XDG_PICTURES_DIR=\"$HOME/My \\$tuff\"\n",
                "/home/u/My $tuff",
            ),
            ("XDG_PICTURES_DIR=\"$HOME/Fotos\"\r\n", "/home/u/Fotos"),
            (
                "XDG_PICTURES_DIR=\"$HOME/Old\"\nXDG_PICTURES_DIR=\"$HOME/New\"\n",
                "/home/u/New",
            ),
            (
                "# XDG_PICTURES_DIR=\"$HOME/Wrong\"\n  XDG_PICTURES_DIR=\"$HOME/Fotos\"\n",
                "/home/u/Fotos",
            ),
        ];
        for (dirs, want) in cases {
            assert_eq!(
                pick(Some(dirs), &[want], &[]),
                PathBuf::from(want),
                "{dirs:?}"
            );
        }
    }

    #[test]
    fn without_a_usable_setting_pictures_then_tmp() {
        for dirs in [
            None,
            Some("XDG_DOWNLOAD_DIR=\"$HOME/Downloads\"\n"),
            Some("XDG_PICTURES_DIR=\"$HOME/\"\n"),
            Some("XDG_PICTURES_DIR=\"$HOME\"\n"),
            Some("XDG_PICTURES_DIR=\"Pics\"\n"),
            Some("XDG_PICTURES_DIR=\"\"\n"),
            Some("XDG_PICTURES_DIR=$HOME/Unquoted\n"),
            Some("XDG_PICTURES_DIR=\"$HOME/..\"\n"),
            Some("XDG_PICTURES_DIR=\"/\"\n"),
        ] {
            assert_eq!(
                pick(dirs, &["/home/u/Pictures"], &["/home/u/Unquoted"]),
                PathBuf::from("/home/u/Pictures"),
                "{dirs:?}"
            );
            assert_eq!(pick(dirs, &[], &[]), PathBuf::from("/tmp"), "{dirs:?}");
        }
    }

    #[test]
    fn without_a_home_only_tmp_is_safe() {
        let dirs = Some("XDG_PICTURES_DIR=\"/data/pics\"\n");
        for home in [
            None,
            Some(Path::new("")),
            Some(Path::new("relative")),
            Some(Path::new("/")),
        ] {
            assert_eq!(
                pictures_dir(home, dirs, &|_| true, &|_| true),
                PathBuf::from("/tmp")
            );
        }
    }

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
