use std::path::Path;

use anyhow::{Context, Result};
use image::{GrayImage, RgbaImage};

const UPSCALE_STEPS: [(u32, u32); 2] = [(1000, 3), (1600, 2)];
const PREVIEW_CHARS: usize = 60;
const PREFERRED: [&str; 2] = ["eng", "rus"];
const TESSERACT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

pub fn owned_scratch(path: &Path, scratch: &Path) -> bool {
    path.parent() == Some(scratch)
        && path.extension().is_some_and(|e| e == "png")
        && path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("rayshot-"))
}

pub fn upscale(width: u32, height: u32) -> u32 {
    let side = width.max(height);
    UPSCALE_STEPS
        .into_iter()
        .find(|(limit, _)| side <= *limit)
        .map_or(1, |(_, factor)| factor)
}

pub fn prepare(img: &RgbaImage) -> GrayImage {
    let mut gray = image::imageops::grayscale(img);
    let (w, h) = gray.dimensions();
    let total: u64 = gray.pixels().map(|p| p.0[0] as u64).sum();
    if w > 0 && h > 0 && total / (w as u64 * h as u64) < 128 {
        image::imageops::invert(&mut gray);
    }
    let k = upscale(w, h);
    if k > 1 {
        gray =
            image::imageops::resize(&gray, w * k, h * k, image::imageops::FilterType::CatmullRom);
    }
    gray
}

pub fn languages(list: &str) -> Option<String> {
    let installed: Vec<&str> = list
        .lines()
        .skip(1)
        .map(str::trim)
        .filter(|l| !l.is_empty() && *l != "osd")
        .collect();
    let preferred: Vec<&str> = PREFERRED
        .into_iter()
        .filter(|l| installed.contains(l))
        .collect();
    if preferred.is_empty() {
        installed.first().map(|l| l.to_string())
    } else {
        Some(preferred.join("+"))
    }
}

pub fn tidy(text: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for line in text.split('\n') {
        let line = line.trim_end_matches(|c: char| c.is_whitespace() || c == '\u{c}');
        let line = if line.trim_matches('\u{c}').trim().is_empty() {
            ""
        } else {
            line
        };
        if line.is_empty() && out.last().is_none_or(|l| l.is_empty()) {
            continue;
        }
        out.push(line);
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out.join("\n")
}

pub fn preview(text: &str) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let first = lines.first().map_or("", |l| l.trim());
    let mut shown: String = first.chars().take(PREVIEW_CHARS).collect();
    if first.chars().count() > PREVIEW_CHARS {
        shown.push('…');
    }
    if lines.len() > 1 {
        shown.push_str(&format!("  ({} lines)", lines.len()));
    }
    shown
}

enum Recognized {
    Text(String),
    NoTesseract,
    NoLanguage,
}

const INSTALL_HINT: &str = "Copying text needs tesseract and its language data: tesseract-data-eng and tesseract-data-rus on Arch, tesseract-ocr and tesseract-ocr-rus on Debian and Ubuntu, tesseract and tesseract-langpack-rus on Fedora.";

fn recognize(png: &Path) -> Result<Recognized> {
    let list = match std::process::Command::new("tesseract")
        .arg("--list-langs")
        .output()
    {
        Ok(out) => out,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Recognized::NoTesseract),
        Err(e) => return Err(e).context("failed to run tesseract"),
    };
    if !list.status.success() {
        anyhow::bail!("tesseract --list-langs exited with {}", list.status);
    }
    let Some(langs) = languages(&String::from_utf8_lossy(&list.stdout)) else {
        return Ok(Recognized::NoLanguage);
    };

    let img = image::open(png)
        .with_context(|| format!("read {}", png.display()))?
        .to_rgba8();
    let dpi = (96 * upscale(img.width(), img.height())).to_string();
    let prepared = png.with_extension("ocr.png");
    prepare(&img)
        .save(&prepared)
        .with_context(|| format!("write {}", prepared.display()))?;
    let out = read_text(&prepared, &langs, &dpi);
    let _ = std::fs::remove_file(&prepared);
    Ok(Recognized::Text(tidy(&out?)))
}

fn read_text(image: &Path, langs: &str, dpi: &str) -> Result<String> {
    use std::io::Read;
    let mut child = std::process::Command::new("tesseract")
        .arg(image)
        .arg("stdout")
        .args(["-l", langs])
        .args(["--dpi", dpi])
        .args(["-c", "preserve_interword_spaces=1"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("failed to run tesseract")?;
    let mut stdout = child
        .stdout
        .take()
        .context("tesseract stdout unavailable")?;
    let mut stderr = child
        .stderr
        .take()
        .context("tesseract stderr unavailable")?;
    let out = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let err = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        buf
    });
    let status = crate::export::wait_bounded(&mut child, TESSERACT_TIMEOUT)
        .context("tesseract took too long")?;
    let out = out.join().unwrap_or_default();
    let err = err.join().unwrap_or_default();
    if !status.success() {
        let err = String::from_utf8_lossy(&err);
        anyhow::bail!(
            "tesseract exited with {status}: {}",
            err.lines().last().unwrap_or("").trim()
        );
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

pub fn run(png: &Path, print: bool) -> bool {
    let png = match std::path::absolute(png) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[rayshot] ocr: bad path {}: {e}", png.display());
            return false;
        }
    };
    let result = recognize(&png);
    if !print && owned_scratch(&png, &crate::export::scratch_dir()) {
        let _ = std::fs::remove_file(&png);
    }
    let notice = |summary: &str, body: &str| {
        if !print {
            crate::hotkey::desktop_notice(summary, body);
        }
    };
    match result {
        Ok(Recognized::Text(text)) if print => {
            println!("{text}");
            true
        }
        Ok(Recognized::Text(text)) if text.is_empty() => {
            eprintln!("[rayshot] ocr: no text");
            notice("No text found", "rayshot found no text in the selection.");
            false
        }
        Ok(Recognized::Text(text)) => match crate::export::copy_text_to_clipboard(&text) {
            Ok(()) => {
                eprintln!("[rayshot] ocr: copied {} chars", text.chars().count());
                notice("Text copied", &preview(&text));
                true
            }
            Err(e) => {
                eprintln!("[rayshot] ocr: copy failed: {e:#}");
                notice("rayshot could not copy the text", &format!("{e:#}"));
                false
            }
        },
        Ok(Recognized::NoTesseract) => {
            eprintln!("[rayshot] ocr: tesseract is not installed");
            notice("Install tesseract to copy text", INSTALL_HINT);
            false
        }
        Ok(Recognized::NoLanguage) => {
            eprintln!("[rayshot] ocr: tesseract has no language data");
            notice("Install tesseract language data", INSTALL_HINT);
            false
        }
        Err(e) => {
            eprintln!("[rayshot] ocr failed: {e:#}");
            notice("rayshot could not read the text", &format!("{e:#}"));
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_our_own_scratch_files_are_deleted() {
        let scratch = Path::new("/run/user/1000/rayshot");
        assert!(owned_scratch(
            &scratch.join("rayshot-07-11-2026.png"),
            scratch
        ));
        assert!(!owned_scratch(&scratch.join("notes.png"), scratch));
        assert!(!owned_scratch(&scratch.join("sub/rayshot-1.png"), scratch));
        assert!(!owned_scratch(
            Path::new("/home/u/Pictures/rayshot-1.png"),
            scratch
        ));
        assert!(!owned_scratch(Path::new("rayshot-1.png"), scratch));
        assert!(!owned_scratch(&scratch.join("rayshot-1.txt"), scratch));
    }

    fn page(width: u32, height: u32, bg: u8, ink: u8) -> RgbaImage {
        let mut img = RgbaImage::from_pixel(width, height, image::Rgba([bg, bg, bg, 255]));
        for x in width / 4..width / 2 {
            img.put_pixel(x, height / 2, image::Rgba([ink, ink, ink, 255]));
        }
        img
    }

    #[test]
    fn light_text_on_dark_is_turned_dark_on_light() {
        let out = prepare(&page(200, 40, 20, 240));
        assert!(out.get_pixel(0, 0).0[0] > 200, "background");
        let (w, h) = out.dimensions();
        assert!(out.get_pixel(w * 3 / 8, h / 2).0[0] < 80, "ink");
    }

    #[test]
    fn dark_text_on_light_keeps_its_polarity() {
        let out = prepare(&page(200, 40, 250, 10));
        assert!(out.get_pixel(0, 0).0[0] > 200);
        let (w, h) = out.dimensions();
        assert!(out.get_pixel(w * 3 / 8, h / 2).0[0] < 80);
    }

    #[test]
    fn small_crops_are_upscaled_more_than_big_ones() {
        assert_eq!(upscale(1, 1), 3);
        assert_eq!(upscale(1000, 40), 3);
        assert_eq!(upscale(40, 1001), 2);
        assert_eq!(upscale(1600, 900), 2);
        assert_eq!(upscale(1601, 900), 1);
        assert_eq!(upscale(1920, 1080), 1);
        assert_eq!(prepare(&page(200, 40, 250, 10)).dimensions(), (600, 120));
        assert_eq!(prepare(&page(1920, 40, 250, 10)).dimensions(), (1920, 40));
    }

    const LIST: &str =
        "List of available languages in \"/usr/share/tessdata/\" (3):\nrus\nosd\neng\n";

    #[test]
    fn english_and_russian_are_used_when_installed() {
        assert_eq!(languages(LIST).as_deref(), Some("eng+rus"));
        assert_eq!(
            languages("List of available languages (2):\neng\nosd\n").as_deref(),
            Some("eng")
        );
        assert_eq!(
            languages("List of available languages (1):\nrus\n").as_deref(),
            Some("rus")
        );
    }

    #[test]
    fn another_language_is_used_when_neither_is_installed() {
        assert_eq!(
            languages("List of available languages (3):\nosd\ndeu\nfra\n").as_deref(),
            Some("deu")
        );
    }

    #[test]
    fn no_usable_language_means_none() {
        assert_eq!(languages("List of available languages (1):\nosd\n"), None);
        assert_eq!(languages("List of available languages (0):\n"), None);
        assert_eq!(languages(""), None);
    }

    #[test]
    fn recognised_text_is_tidied() {
        assert_eq!(
            tidy("\n\n  fn main() {   \n\n\n\n      ok\t\n}\n\u{c}\n"),
            "  fn main() {\n\n      ok\n}"
        );
        assert_eq!(tidy(" \n\u{c}\n"), "");
    }

    #[test]
    fn the_notice_shows_the_first_line_and_the_count() {
        assert_eq!(preview("hello\nworld"), "hello  (2 lines)");
        assert_eq!(preview("\n  one line  "), "one line");
        let long = "x".repeat(80);
        assert_eq!(preview(&long), format!("{}…", "x".repeat(PREVIEW_CHARS)));
    }
}
