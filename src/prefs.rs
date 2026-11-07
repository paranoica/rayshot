use std::path::{Path, PathBuf};

use crate::scene::Tool;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Prefs {
    pub hidden: Vec<Tool>,
    pub color: Option<egui::Color32>,
    pub width: Option<f32>,
}

pub const WIDTH_RANGE: (f32, f32) = (1.0, 20.0);

pub fn parse(text: &str) -> Prefs {
    let mut prefs = Prefs::default();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "hidden" => {
                prefs.hidden = Vec::new();
                for tool in value.split(',').filter_map(|n| Tool::from_name(n.trim())) {
                    if !prefs.hidden.contains(&tool) {
                        prefs.hidden.push(tool);
                    }
                }
            }
            "color" => prefs.color = parse_color(value),
            "width" => {
                prefs.width = value
                    .parse::<f32>()
                    .ok()
                    .filter(|w| w.is_finite())
                    .map(|w| w.clamp(WIDTH_RANGE.0, WIDTH_RANGE.1));
            }
            _ => {}
        }
    }
    prefs
}

fn parse_color(value: &str) -> Option<egui::Color32> {
    let hex = value.strip_prefix('#').filter(|h| h.len() == 6)?;
    let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
    Some(egui::Color32::from_rgb(byte(0)?, byte(2)?, byte(4)?))
}

pub fn render(prefs: &Prefs) -> String {
    let hidden: Vec<&str> = prefs.hidden.iter().map(|t| t.name()).collect();
    let mut out = format!("hidden = {}\n", hidden.join(", "));
    if let Some(c) = prefs.color {
        out += &format!("color = #{:02X}{:02X}{:02X}\n", c.r(), c.g(), c.b());
    }
    if let Some(w) = prefs.width {
        out += &format!("width = {w}\n");
    }
    out
}

fn path_from(xdg_config: Option<&Path>, home: Option<&Path>) -> Option<PathBuf> {
    let base = match xdg_config.filter(|p| p.is_absolute()) {
        Some(dir) => dir.to_path_buf(),
        None => home.filter(|h| h.is_absolute())?.join(".config"),
    };
    Some(base.join("rayshot").join("editor.conf"))
}

fn path() -> Option<PathBuf> {
    let env = |name: &str| std::env::var_os(name).map(PathBuf::from);
    path_from(env("XDG_CONFIG_HOME").as_deref(), env("HOME").as_deref())
}

pub fn load() -> Prefs {
    path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|text| parse(&text))
        .unwrap_or_default()
}

pub fn save(prefs: &Prefs) {
    let Some(path) = path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("conf.tmp");
    if std::fs::write(&tmp, render(prefs)).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Color32;

    #[test]
    fn a_saved_file_reads_back_the_same() {
        let prefs = Prefs {
            hidden: vec![Tool::Blur, Tool::Counter],
            color: Some(Color32::from_rgb(18, 52, 86)),
            width: Some(7.0),
        };
        assert_eq!(parse(&render(&prefs)), prefs);
    }

    #[test]
    fn the_file_is_plain_and_readable() {
        let prefs = Prefs {
            hidden: vec![Tool::Pixelate],
            color: Some(Color32::from_rgb(255, 0, 171)),
            width: Some(4.0),
        };
        let text = render(&prefs);
        assert!(text.contains("hidden = pixelate"), "{text}");
        assert!(text.contains("color = #FF00AB"), "{text}");
        assert!(text.contains("width = 4"), "{text}");
    }

    #[test]
    fn an_empty_or_missing_file_gives_defaults() {
        assert_eq!(parse(""), Prefs::default());
    }

    #[test]
    fn hand_edits_and_garbage_are_tolerated() {
        let text = "# my settings\n  hidden=  blur ,  nonsense,pen \ncolor = red\nwidth = 99\nfoo = bar\nbroken line\n";
        let prefs = parse(text);
        assert_eq!(prefs.hidden, vec![Tool::Blur, Tool::Pen]);
        assert_eq!(prefs.color, None, "only #RRGGBB is understood");
        assert_eq!(prefs.width, Some(20.0), "clamped to the allowed range");
    }

    #[test]
    fn widths_below_the_range_are_raised_and_nonsense_ignored() {
        assert_eq!(parse("width = 0\n").width, Some(1.0));
        assert_eq!(parse("width = wide\n").width, None);
        assert_eq!(parse("width = NaN\n").width, None);
    }

    #[test]
    fn the_file_lives_in_the_config_dir() {
        assert_eq!(
            path_from(Some(Path::new("/cfg")), Some(Path::new("/home/u"))),
            Some(PathBuf::from("/cfg/rayshot/editor.conf"))
        );
        assert_eq!(
            path_from(None, Some(Path::new("/home/u"))),
            Some(PathBuf::from("/home/u/.config/rayshot/editor.conf"))
        );
        assert_eq!(
            path_from(Some(Path::new("relative")), Some(Path::new("/home/u"))),
            Some(PathBuf::from("/home/u/.config/rayshot/editor.conf")),
            "a relative XDG_CONFIG_HOME is ignored"
        );
        assert_eq!(path_from(None, None), None);
        assert_eq!(path_from(None, Some(Path::new(""))), None);
    }
}
