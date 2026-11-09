use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::process::Command;

pub const SCHEMA: &str = "org.gnome.shell.extensions.rayshot";
const KEY: &str = "warm-overlay";

pub fn schema_dir() -> Result<PathBuf> {
    let env_dir = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
    let user = env_dir("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    let system = env_dir("XDG_DATA_DIRS")
        .map(|v| std::env::split_paths(&v).collect::<Vec<_>>())
        .unwrap_or_else(|| vec!["/usr/local/share".into(), "/usr/share".into()]);
    user.into_iter()
        .chain(system)
        .map(|d| d.join("gnome-shell/extensions/rayshot@paranoica/schemas"))
        .find(|d| d.join("gschemas.compiled").exists())
        .context("the rayshot GNOME extension is not installed")
}

pub fn gsettings(dir: &PathBuf, args: &[&str]) -> Result<String> {
    let out = Command::new("gsettings")
        .arg("--schemadir")
        .arg(dir)
        .args(args)
        .output()
        .context("failed to run gsettings")?;
    if !out.status.success() {
        bail!(
            "gsettings failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn run(arg: Option<&str>) -> Result<()> {
    let dir = schema_dir()?;
    match arg {
        Some("on") => gsettings(&dir, &["set", SCHEMA, KEY, "true"]).map(drop)?,
        Some("off") => gsettings(&dir, &["set", SCHEMA, KEY, "false"]).map(drop)?,
        Some("status") | None => {}
        Some(other) => bail!("usage: rayshot warm [on|off|status], got {other:?}"),
    }
    let state = parse_bool(&gsettings(&dir, &["get", SCHEMA, KEY])?)
        .context("unexpected gsettings output")?;
    println!("warm overlay: {}", if state { "on" } else { "off" });
    Ok(())
}

pub fn parse_bool(output: &str) -> Option<bool> {
    match output.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_gsettings_booleans() {
        assert_eq!(parse_bool("true\n"), Some(true));
        assert_eq!(parse_bool("false\n"), Some(false));
        assert_eq!(parse_bool("  true "), Some(true));
    }

    #[test]
    fn rejects_anything_else() {
        assert_eq!(parse_bool(""), None);
        assert_eq!(parse_bool("\n"), None);
        assert_eq!(parse_bool("TRUE"), None);
        assert_eq!(parse_bool("yes"), None);
        assert_eq!(parse_bool("true false"), None);
        assert_eq!(parse_bool("No such key “warm-overlay”"), None);
    }
}
