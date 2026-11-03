use std::path::PathBuf;

#[derive(Debug, PartialEq)]
pub enum Command {
    Show { id: u64, width: u32, height: u32 },
    Frame { id: u64, path: PathBuf },
    Cancel { id: u64 },
    Quit,
}

pub fn parse_command(line: &str) -> Option<Command> {
    let line = line.trim_end_matches(['\n', '\r']);
    if line == "quit" {
        return Some(Command::Quit);
    }
    if let Some(rest) = line.strip_prefix("frame ") {
        let (id, path) = rest.split_once(' ')?;
        let id = id.parse().ok()?;
        return (!path.trim().is_empty()).then(|| Command::Frame {
            id,
            path: PathBuf::from(path),
        });
    }
    if let Some(id) = line.strip_prefix("cancel ") {
        return id.parse().ok().map(|id| Command::Cancel { id });
    }
    let rest = line.strip_prefix("show ")?;
    let mut parts = rest.split(' ');
    let id = parts.next()?.parse().ok()?;
    let width = parts.next()?.parse().ok().filter(|&w: &u32| w > 0)?;
    let height = parts.next()?.parse().ok().filter(|&h: &u32| h > 0)?;
    parts
        .next()
        .is_none()
        .then_some(Command::Show { id, width, height })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(id: u64, path: &str) -> Option<Command> {
        Some(Command::Frame {
            id,
            path: PathBuf::from(path),
        })
    }

    #[test]
    fn parses_frame_with_session_and_path() {
        assert_eq!(
            parse_command("frame 7 /run/user/1000/rayshot/f.rsf\n"),
            frame(7, "/run/user/1000/rayshot/f.rsf")
        );
    }

    #[test]
    fn keeps_spaces_inside_frame_path() {
        assert_eq!(
            parse_command("frame 1 /tmp/a b/c.rsf"),
            frame(1, "/tmp/a b/c.rsf")
        );
    }

    #[test]
    fn rejects_frame_without_session_or_path() {
        assert_eq!(parse_command("frame /x.rsf"), None);
        assert_eq!(parse_command("frame 7"), None);
        assert_eq!(parse_command("frame 7 "), None);
        assert_eq!(parse_command("frame"), None);
        assert_eq!(parse_command("frame -1 /x"), None);
        assert_eq!(parse_command("frame x /x"), None);
    }

    #[test]
    fn parses_show() {
        assert_eq!(
            parse_command("show 3 3840 1080\r\n"),
            Some(Command::Show {
                id: 3,
                width: 3840,
                height: 1080
            })
        );
        assert_eq!(
            parse_command(&format!("show {} 1 1", u64::MAX)),
            Some(Command::Show {
                id: u64::MAX,
                width: 1,
                height: 1
            })
        );
    }

    #[test]
    fn rejects_bad_show() {
        assert_eq!(parse_command("show 3 0 1080"), None);
        assert_eq!(parse_command("show 3 1920 0"), None);
        assert_eq!(parse_command("show 3 1920"), None);
        assert_eq!(parse_command("show 3 1920 1080 5"), None);
        assert_eq!(parse_command("show 3 a b"), None);
        assert_eq!(parse_command("show 1920 1080"), None);
        assert_eq!(parse_command("show -1 2 2"), None);
        assert_eq!(parse_command("show 3 4294967296 1"), None);
    }

    #[test]
    fn parses_cancel() {
        assert_eq!(parse_command("cancel 9\n"), Some(Command::Cancel { id: 9 }));
    }

    #[test]
    fn rejects_bad_cancel() {
        assert_eq!(parse_command("cancel"), None);
        assert_eq!(parse_command("cancel "), None);
        assert_eq!(parse_command("cancel x"), None);
        assert_eq!(parse_command("cancel 9 9"), None);
        assert_eq!(parse_command("cancel -9"), None);
    }

    #[test]
    fn parses_quit() {
        assert_eq!(parse_command("quit\r\n"), Some(Command::Quit));
    }

    #[test]
    fn rejects_unknown_and_empty() {
        assert_eq!(parse_command(""), None);
        assert_eq!(parse_command("\n"), None);
        assert_eq!(parse_command("hello"), None);
        assert_eq!(parse_command("FRAME 1 /x"), None);
        assert_eq!(parse_command("quitx"), None);
        assert_eq!(parse_command("showx 1 2 3"), None);
    }
}
