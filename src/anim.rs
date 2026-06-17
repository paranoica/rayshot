use std::path::PathBuf;
use std::process::{Command, Stdio};

const SETTINGS: &[(&str, &str)] = &[
    ("org.gnome.desktop.interface", "enable-animations"),
    ("org.gnome.desktop.sound", "event-sounds"),
];

pub struct AnimationGuard {
    restore: bool,
}

impl AnimationGuard {
    pub fn disable() -> Self {
        if std::env::var_os("RAYSHOT_KEEP_ANIMATIONS").is_some() {
            return Self { restore: false };
        }
        force_restore();

        let mut changed: Vec<String> = Vec::new();
        for (schema, key) in SETTINGS {
            if get_bool(schema, key) {
                set_bool(schema, key, false);
                changed.push(format!("{schema} {key}"));
            }
        }
        if changed.is_empty() {
            return Self { restore: false };
        }
        if let Some(dir) = marker_path().parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(marker_path(), changed.join("\n"));
        install_signal_restore();
        Self { restore: true }
    }
}

impl Drop for AnimationGuard {
    fn drop(&mut self) {
        if self.restore {
            force_restore();
        }
    }
}

fn marker_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join(".cache/rayshot/anim_restore")
}

fn get_bool(schema: &str, key: &str) -> bool {
    Command::new("gsettings")
        .args(["get", schema, key])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim() == "true")
        .unwrap_or(false)
}

fn set_bool(schema: &str, key: &str, val: bool) {
    let _ = Command::new("gsettings")
        .args(["set", schema, key, if val { "true" } else { "false" }])
        .status();
}

pub fn force_restore() {
    if let Ok(content) = std::fs::read_to_string(marker_path()) {
        for line in content.lines() {
            if let Some((schema, key)) = line.split_once(' ') {
                set_bool(schema, key, true);
            }
        }
        let _ = std::fs::remove_file(marker_path());
    }
}

pub fn restore_detached() {
    let Ok(content) = std::fs::read_to_string(marker_path()) else {
        return;
    };
    let mut script = String::from("sleep 0.25");
    for line in content.lines() {
        if let Some((schema, key)) = line.split_once(' ') {
            script.push_str(&format!("; gsettings set {schema} {key} true"));
        }
    }
    script.push_str(&format!("; rm -f '{}'", marker_path().display()));
    let _ = Command::new("sh")
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

fn install_signal_restore() {
    use signal_hook::consts::{SIGINT, SIGTERM};
    use signal_hook::iterator::Signals;
    if let Ok(mut signals) = Signals::new([SIGINT, SIGTERM]) {
        std::thread::spawn(move || {
            if signals.forever().next().is_some() {
                force_restore();
                unsafe { libc::_exit(130) };
            }
        });
    }
}
