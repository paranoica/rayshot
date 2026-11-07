use std::path::{Path, PathBuf};

use crate::scene::Tool;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Copy,
    CopyText,
    Save,
    SaveAs,
}

impl Button {
    pub const ALL: [Button; 4] = [Button::Copy, Button::CopyText, Button::Save, Button::SaveAs];

    pub fn name(self) -> &'static str {
        match self {
            Button::Copy => "copy",
            Button::CopyText => "copy-text",
            Button::Save => "save",
            Button::SaveAs => "save-as",
        }
    }

    pub fn from_name(name: &str) -> Option<Button> {
        Button::ALL.into_iter().find(|b| b.name() == name)
    }
}

pub const RECENT_MAX: usize = 5;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Prefs {
    pub hidden: Vec<Tool>,
    pub hidden_buttons: Vec<Button>,
    pub color: Option<egui::Color32>,
    pub width: Option<f32>,
    pub recent: Vec<egui::Color32>,
}

pub fn push_recent(
    recent: &mut Vec<egui::Color32>,
    color: egui::Color32,
    palette: &[egui::Color32],
) {
    if palette.contains(&color) {
        return;
    }
    recent.retain(|c| *c != color);
    recent.insert(0, color);
    recent.truncate(RECENT_MAX);
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
                prefs.hidden_buttons = Vec::new();
                for name in value.split(',').map(str::trim) {
                    if let Some(tool) = Tool::from_name(name) {
                        if !prefs.hidden.contains(&tool) {
                            prefs.hidden.push(tool);
                        }
                    } else if let Some(button) = Button::from_name(name)
                        && !prefs.hidden_buttons.contains(&button)
                    {
                        prefs.hidden_buttons.push(button);
                    }
                }
            }
            "recent" => {
                prefs.recent = value
                    .split(',')
                    .filter_map(|c| parse_color(c.trim()))
                    .take(RECENT_MAX)
                    .collect();
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

pub fn hex(c: egui::Color32) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r(), c.g(), c.b())
}

pub fn parse_color(value: &str) -> Option<egui::Color32> {
    let hex = value.strip_prefix('#').filter(|h| h.len() == 6)?;
    let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
    Some(egui::Color32::from_rgb(byte(0)?, byte(2)?, byte(4)?))
}

pub fn render(prefs: &Prefs) -> String {
    let hidden: Vec<&str> = prefs
        .hidden
        .iter()
        .map(|t| t.name())
        .chain(prefs.hidden_buttons.iter().map(|b| b.name()))
        .collect();
    let mut out = format!("hidden = {}\n", hidden.join(", "));
    if !prefs.recent.is_empty() {
        let recent: Vec<String> = prefs.recent.iter().map(|c| hex(*c)).collect();
        out += &format!("recent = {}\n", recent.join(", "));
    }
    if let Some(c) = prefs.color {
        out += &format!("color = {}\n", hex(c));
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
            hidden_buttons: vec![Button::SaveAs],
            color: Some(Color32::from_rgb(18, 52, 86)),
            width: Some(7.0),
            recent: vec![Color32::from_rgb(1, 2, 3), Color32::from_rgb(250, 100, 0)],
        };
        assert_eq!(parse(&render(&prefs)), prefs);
    }

    #[test]
    fn buttons_and_tools_share_the_hidden_list() {
        let prefs = parse("hidden = copy, pen, save-as\n");
        assert_eq!(prefs.hidden, vec![Tool::Pen]);
        assert_eq!(prefs.hidden_buttons, vec![Button::Copy, Button::SaveAs]);
    }

    #[test]
    fn recent_colours_keep_at_most_five_valid_ones() {
        let prefs = parse("recent = #010203, bad, #0A0B0C, #111111, #222222, #333333, #444444\n");
        assert_eq!(prefs.recent.len(), RECENT_MAX);
        assert_eq!(prefs.recent[0], Color32::from_rgb(1, 2, 3));
        assert_eq!(prefs.recent[1], Color32::from_rgb(10, 11, 12));
    }

    #[test]
    fn a_new_colour_goes_first_and_a_reused_one_moves_up() {
        let palette = [Color32::RED];
        let (a, b, c) = (
            Color32::from_rgb(1, 1, 1),
            Color32::from_rgb(2, 2, 2),
            Color32::from_rgb(3, 3, 3),
        );
        let mut recent = vec![a, b];
        push_recent(&mut recent, c, &palette);
        assert_eq!(recent, vec![c, a, b]);
        push_recent(&mut recent, b, &palette);
        assert_eq!(recent, vec![b, c, a], "no duplicates");
        push_recent(&mut recent, Color32::RED, &palette);
        assert_eq!(
            recent,
            vec![b, c, a],
            "palette colours already have their own spot"
        );
        for i in 10..20 {
            push_recent(&mut recent, Color32::from_rgb(i, i, i), &palette);
        }
        assert_eq!(recent.len(), RECENT_MAX);
        assert_eq!(recent[0], Color32::from_rgb(19, 19, 19));
    }

    #[test]
    fn the_file_is_plain_and_readable() {
        let prefs = Prefs {
            hidden: vec![Tool::Pixelate],
            color: Some(Color32::from_rgb(255, 0, 171)),
            width: Some(4.0),
            ..Prefs::default()
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
