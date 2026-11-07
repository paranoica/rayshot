use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};

const MEDIA_KEYS: &str = "org.gnome.settings-daemon.plugins.media-keys";
const KB_PATH: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/rayshot/";
const SHELL_KB: &str = "org.gnome.shell.keybindings";
const EXTENSION: &str = "rayshot@paranoica";

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

fn print_backup_path() -> PathBuf {
    home().join(".config/rayshot/print_backup")
}

fn autostart_path() -> PathBuf {
    home().join(".config/autostart/rayshot-daemon.desktop")
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

/// The custom-keybindings list without the old daemon hotkey, or None when
/// it is not there and nothing needs to change.
fn without_rayshot_binding(list: &str) -> Option<String> {
    let paths = parse_paths(list);
    if !paths.iter().any(|p| p == KB_PATH) {
        return None;
    }
    let kept: Vec<String> = paths.into_iter().filter(|p| p != KB_PATH).collect();
    Some(format_paths(&kept))
}

/// A one-element GVariant string array for gsettings.
fn gvariant_strv(item: &str) -> String {
    let escaped = item.replace('\\', "\\\\").replace('\'', "\\'");
    format!("['{escaped}']")
}

/// A key name is required after any `<Modifier>` prefixes.
fn valid_binding(binding: &str) -> bool {
    let key = binding.rsplit('>').next().unwrap_or_default();
    let balanced = binding.matches('<').count() == binding.matches('>').count();
    !key.is_empty()
        && !key.contains('<')
        && balanced
        && !binding.eq_ignore_ascii_case("disabled")
        && !binding.chars().any(char::is_whitespace)
}

/// Pids of old `rayshot daemon` processes, never including `me`.
fn daemon_pids(procs: &[(u32, Vec<String>)], me: u32) -> Vec<u32> {
    procs
        .iter()
        .filter(|(pid, argv)| {
            *pid != me
                && argv.get(1).map(String::as_str) == Some("daemon")
                && argv
                    .first()
                    .and_then(|a| std::path::Path::new(a).file_name())
                    .is_some_and(|n| n == "rayshot")
        })
        .map(|(pid, _)| *pid)
        .collect()
}

fn own_processes() -> Vec<(u32, Vec<String>)> {
    use std::os::unix::fs::MetadataExt;
    let uid = unsafe { libc::getuid() };
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    dir.filter_map(|e| {
        let e = e.ok()?;
        let pid: u32 = e.file_name().to_str()?.parse().ok()?;
        if e.metadata().ok()?.uid() != uid {
            return None;
        }
        let raw = std::fs::read(e.path().join("cmdline")).ok()?;
        let argv = raw
            .split(|b| *b == 0)
            .filter(|a| !a.is_empty())
            .map(|a| String::from_utf8_lossy(a).into_owned())
            .collect();
        Some((pid, argv))
    })
    .collect()
}

/// Removes everything the pre-extension setup left behind: the media-keys
/// hotkey, the daemon autostart entry and any daemon still running.
fn retire_old_setup() {
    let _ = std::fs::remove_file(autostart_path());
    for pid in daemon_pids(&own_processes(), std::process::id()) {
        if unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) } == 0 {
            println!("stopped the old rayshot daemon (pid {pid})");
        }
    }
    if !is_gnome() {
        return;
    }
    let list = gget(MEDIA_KEYS, "custom-keybindings").unwrap_or_default();
    if let Some(kept) = without_rayshot_binding(&list) {
        gset(&["set", MEDIA_KEYS, "custom-keybindings", &kept]);
        let schema = format!("{MEDIA_KEYS}.custom-keybinding:{KB_PATH}");
        for key in ["name", "command", "binding"] {
            gset(&["reset", &schema, key]);
        }
        println!("removed the old rayshot media-keys shortcut");
    }
}

fn gnome_extensions(args: &[&str]) -> bool {
    Command::new("gnome-extensions")
        .args(args)
        .status()
        .is_ok_and(|s| s.success())
}

fn extension_listed(filter: &[&str]) -> bool {
    Command::new("gnome-extensions")
        .arg("list")
        .args(filter)
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .is_some_and(|out| out.lines().any(|l| l.trim() == EXTENSION))
}

/// Enables the extension for the next login when the running shell has not
/// scanned it yet, which `gnome-extensions enable` cannot do.
fn enable_on_next_login() {
    let list = gget("org.gnome.shell", "enabled-extensions").unwrap_or_default();
    let mut uuids = parse_paths(&list);
    if !uuids.iter().any(|u| u == EXTENSION) {
        uuids.push(EXTENSION.to_string());
        gset(&[
            "set",
            "org.gnome.shell",
            "enabled-extensions",
            &format_paths(&uuids),
        ]);
    }
}

fn current_desktop() -> String {
    std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default()
}

fn is_gnome() -> bool {
    current_desktop().to_ascii_uppercase().contains("GNOME")
}

/// Runs once at login for users of the old daemon, from its autostart entry,
/// which is removed here; nobody reads stderr there, so they get a notice.
pub fn retire_daemon() -> Result<()> {
    let had_autostart = std::fs::remove_file(autostart_path()).is_ok();
    eprintln!("rayshot: the background daemon was removed in favour of the GNOME extension.");
    eprintln!("rayshot: run `rayshot install-hotkey Print` once to finish the switch.");
    if !had_autostart {
        return Ok(());
    }
    desktop_notice(
        "rayshot was updated",
        "Run \"rayshot install-hotkey Print\" once to finish the update.",
    );
    Ok(())
}

pub fn gvariant_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub fn desktop_notice(summary: &str, body: &str) {
    let _ = Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "org.freedesktop.Notifications",
            "--object-path",
            "/org/freedesktop/Notifications",
            "--method",
            "org.freedesktop.Notifications.Notify",
            "--",
        ])
        .args([
            gvariant_string("rayshot"),
            "uint32 0".to_string(),
            gvariant_string(""),
            gvariant_string(summary),
            gvariant_string(body),
            "@as []".to_string(),
            "@a{sv} {}".to_string(),
            "int32 0".to_string(),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

pub fn install(binding: &str, daemon: bool) -> Result<()> {
    let exe = std::env::current_exe()
        .context("cannot find rayshot's own path")?
        .to_string_lossy()
        .to_string();
    if daemon {
        println!("rayshot: --daemon is no longer needed and is ignored.");
    }

    if !is_gnome() {
        retire_old_setup();
        let desktop = current_desktop();
        let desktop = if desktop.is_empty() {
            "unknown".to_string()
        } else {
            desktop
        };
        println!("rayshot: automatic hotkey setup is GNOME-only (detected: {desktop}).");
        println!("Bind a shortcut to this command in your desktop's keyboard settings:");
        println!("    {exe}");
        println!(
            "(rayshot captures via the screenshot portal; needs a portal backend and wl-clipboard.)"
        );
        return Ok(());
    }

    // Everything that can fail runs before the old hotkey is taken away.
    if !valid_binding(binding) {
        bail!("{binding:?} is not a key binding, try Print or <Control>Print");
    }
    let dir = crate::warm::schema_dir()?;
    if !extension_listed(&[]) {
        enable_on_next_login();
        println!(
            "rayshot: the GNOME extension is installed but GNOME Shell has not loaded it yet."
        );
        println!(
            "Log out and back in, then run `rayshot install-hotkey {binding}` again to finish."
        );
        return Ok(());
    }
    if !gnome_extensions(&["enable", EXTENSION]) {
        bail!("could not enable the {EXTENSION} GNOME extension");
    }
    // GNOME Shell enables an extension from an idle callback, not at once.
    let active = (0..20).any(|attempt| {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        extension_listed(&["--active"])
    });
    if !active {
        if gget("org.gnome.shell", "disable-user-extensions").as_deref() == Some("true") {
            bail!("user extensions are turned off in GNOME; turn them on in the Extensions app");
        }
        println!("rayshot: the GNOME extension is enabled but not running.");
        println!(
            "Log out and back in, then run `rayshot install-hotkey {binding}` again to finish."
        );
        println!("If it still is not running, check `gnome-extensions info {EXTENSION}`.");
        return Ok(());
    }
    crate::warm::gsettings(
        &dir,
        &[
            "set",
            crate::warm::SCHEMA,
            "capture",
            &gvariant_strv(binding),
        ],
    )?;

    retire_old_setup();
    if binding.eq_ignore_ascii_case("Print") {
        if let Some(cur) = gget(SHELL_KB, "show-screenshot-ui") {
            if let Some(parent) = print_backup_path().parent() {
                std::fs::create_dir_all(parent).ok();
            }
            if !print_backup_path().exists() {
                std::fs::write(print_backup_path(), &cur).ok();
            }
        }
        gset(&["set", SHELL_KB, "show-screenshot-ui", "@as []"]);
    }
    println!("rayshot hotkey installed: {binding} opens rayshot through the GNOME extension");
    Ok(())
}

pub fn uninstall() -> Result<()> {
    retire_old_setup();
    if !is_gnome() {
        println!("rayshot: nothing to remove automatically (GNOME-only).");
        println!("Remove the shortcut you bound manually in your desktop's settings.");
        return Ok(());
    }
    gnome_extensions(&["disable", EXTENSION]);
    let restore = std::fs::read_to_string(print_backup_path())
        .ok()
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "['Print']".to_string());
    gset(&["set", SHELL_KB, "show-screenshot-ui", &restore]);
    let _ = std::fs::remove_file(print_backup_path());
    println!(
        "rayshot hotkey removed and the extension disabled; GNOME Print restored to {restore}"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notice_text_goes_to_gdbus_as_a_plain_string() {
        assert_eq!(gvariant_string("hello"), "\"hello\"");
        assert_eq!(gvariant_string("- item"), "\"- item\"");
        assert_eq!(gvariant_string("'quoted'"), "\"'quoted'\"");
        assert_eq!(gvariant_string("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(gvariant_string("one\ntwo\tx"), "\"one\\ntwo\\tx\"");
        assert_eq!(gvariant_string(""), "\"\"");
    }

    const OTHER: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/";

    #[test]
    fn drops_only_the_rayshot_binding() {
        let list = format!("['{OTHER}', '{KB_PATH}']");
        assert_eq!(without_rayshot_binding(&list), Some(format!("['{OTHER}']")));
    }

    #[test]
    fn empties_a_list_that_held_only_rayshot() {
        let list = format!("['{KB_PATH}']");
        assert_eq!(without_rayshot_binding(&list), Some("@as []".to_string()));
    }

    #[test]
    fn leaves_lists_without_rayshot_alone() {
        assert_eq!(without_rayshot_binding(&format!("['{OTHER}']")), None);
        assert_eq!(without_rayshot_binding("@as []"), None);
        assert_eq!(without_rayshot_binding(""), None);
    }

    fn proc(pid: u32, argv: &[&str]) -> (u32, Vec<String>) {
        (pid, argv.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn finds_old_daemons_by_name_and_subcommand() {
        let procs = [
            proc(10, &["/usr/bin/rayshot", "daemon"]),
            proc(
                11,
                &["/home/u/projects/rayshot/target/release/rayshot", "daemon"],
            ),
            proc(12, &["rayshot", "serve"]),
            proc(13, &["/usr/bin/other", "daemon"]),
            proc(14, &["/usr/bin/rayshot-helper", "daemon"]),
            proc(15, &["/usr/bin/rayshot"]),
        ];
        assert_eq!(daemon_pids(&procs, 1), vec![10, 11]);
    }

    #[test]
    fn quotes_plain_bindings() {
        assert_eq!(gvariant_strv("Print"), "['Print']");
        assert_eq!(gvariant_strv("<Control>Print"), "['<Control>Print']");
    }

    #[test]
    fn escapes_quotes_and_backslashes() {
        // one quote becomes backslash-quote, one backslash becomes two
        assert_eq!(gvariant_strv("a'b"), "['a\\'b']");
        assert_eq!(gvariant_strv("a\\"), "['a\\\\']");
        assert_eq!(gvariant_strv("\\'"), "['\\\\\\'']");
    }

    #[test]
    fn accepts_real_bindings() {
        assert!(valid_binding("Print"));
        assert!(valid_binding("<Control>Print"));
        assert!(valid_binding("<Shift><Super>s"));
    }

    #[test]
    fn rejects_empty_or_spaced_bindings() {
        assert!(!valid_binding(""));
        assert!(!valid_binding("   "));
        assert!(!valid_binding("Print Screen"));
        assert!(!valid_binding("<Control>"));
    }

    #[test]
    fn rejects_disabled_and_unclosed_modifiers() {
        assert!(!valid_binding("disabled"));
        assert!(!valid_binding("Disabled"));
        assert!(!valid_binding("<Control"));
        assert!(!valid_binding("<Control>Print>"));
    }

    #[test]
    fn never_lists_itself() {
        let procs = [proc(42, &["/usr/bin/rayshot", "daemon"])];
        assert_eq!(daemon_pids(&procs, 42), Vec::<u32>::new());
    }
}
