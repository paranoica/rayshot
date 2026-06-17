use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};

const MEDIA_KEYS: &str = "org.gnome.settings-daemon.plugins.media-keys";
const KB_PATH: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/rayshot/";
const SHELL_KB: &str = "org.gnome.shell.keybindings";

fn kb_schema_path() -> String {
    format!("{MEDIA_KEYS}.custom-keybinding:{KB_PATH}")
}

fn print_backup_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join(".config/rayshot/print_backup")
}

fn gset(args: &[&str]) {
    let _ = Command::new("gsettings").args(args).status();
}

fn gget(schema: &str, key: &str) -> Option<String> {
    Command::new("gsettings")
        .args(["get", schema, key])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
}

fn parse_paths(s: &str) -> Vec<String> {
    s.split('\'')
        .enumerate()
        .filter(|(i, _)| i % 2 == 1)
        .map(|(_, p)| p.to_string())
        .collect()
}

fn format_paths(paths: &[String]) -> String {
    if paths.is_empty() {
        "@as []".to_string()
    } else {
        let inner = paths
            .iter()
            .map(|p| format!("'{p}'"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("[{inner}]")
    }
}

fn autostart_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join(".config/autostart/rayshot-daemon.desktop")
}

fn write_autostart(exe: &str) {
    let path = autostart_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let content = format!(
        "[Desktop Entry]\nType=Application\nName=rayshot daemon\nComment=rayshot instant screenshot daemon\nExec={exe} daemon\nX-GNOME-Autostart-enabled=true\nNoDisplay=true\n"
    );
    let _ = std::fs::write(path, content);
}

fn start_daemon(exe: &str) {
    let _ = Command::new("setsid")
        .arg(exe)
        .arg("daemon")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

pub fn install(binding: &str, daemon: bool) -> Result<()> {
    let exe = std::env::current_exe()
        .context("cannot find rayshot's own path")?
        .to_string_lossy()
        .to_string();

    if binding.eq_ignore_ascii_case("Print") {
        if let Some(cur) = gget(SHELL_KB, "show-screenshot-ui") {
            if let Some(dir) = print_backup_path().parent() {
                std::fs::create_dir_all(dir).ok();
            }
            if !print_backup_path().exists() {
                std::fs::write(print_backup_path(), &cur).ok();
            }
        }
        gset(&["set", SHELL_KB, "show-screenshot-ui", "@as []"]);
    }

    let command = if daemon {
        format!("{exe} trigger")
    } else {
        exe.clone()
    };

    let schema_path = kb_schema_path();
    gset(&["set", &schema_path, "name", "rayshot"]);
    gset(&["set", &schema_path, "command", &command]);
    gset(&["set", &schema_path, "binding", binding]);

    let list = gget(MEDIA_KEYS, "custom-keybindings").unwrap_or_default();
    let mut paths = parse_paths(&list);
    if !paths.iter().any(|p| p == KB_PATH) {
        paths.push(KB_PATH.to_string());
    }
    gset(&[
        "set",
        MEDIA_KEYS,
        "custom-keybindings",
        &format_paths(&paths),
    ]);

    if daemon {
        write_autostart(&exe);
        start_daemon(&exe);
        println!("rayshot hotkey installed (daemon mode): {binding} -> {command}");
        println!("daemon autostart enabled and started");
    } else {
        let _ = std::fs::remove_file(autostart_path());
        println!("rayshot hotkey installed: {binding} -> {command}");
    }
    Ok(())
}

pub fn uninstall() -> Result<()> {
    let list = gget(MEDIA_KEYS, "custom-keybindings").unwrap_or_default();
    let paths: Vec<String> = parse_paths(&list)
        .into_iter()
        .filter(|p| p != KB_PATH)
        .collect();
    gset(&[
        "set",
        MEDIA_KEYS,
        "custom-keybindings",
        &format_paths(&paths),
    ]);

    let restore = std::fs::read_to_string(print_backup_path())
        .ok()
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "['Print']".to_string());
    gset(&["set", SHELL_KB, "show-screenshot-ui", &restore]);
    let _ = std::fs::remove_file(print_backup_path());
    let _ = std::fs::remove_file(autostart_path());

    println!("rayshot hotkey removed; GNOME Print restored to {restore}");
    println!("daemon autostart disabled (running daemon, if any, stays until logout)");
    Ok(())
}
