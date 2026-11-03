use std::path::PathBuf;

#[derive(Debug, PartialEq)]
pub enum Command {
    Frame(PathBuf),
    Quit,
}

pub fn parse_command(line: &str) -> Option<Command> {
    let line = line.trim_end_matches(['\n', '\r']);
    if line == "quit" {
        return Some(Command::Quit);
    }
    let path = line.strip_prefix("frame ")?;
    (!path.trim().is_empty()).then(|| Command::Frame(PathBuf::from(path)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frame_with_path() {
        assert_eq!(
            parse_command("frame /run/user/1000/rayshot/f.rsf\n"),
            Some(Command::Frame(PathBuf::from(
                "/run/user/1000/rayshot/f.rsf"
            )))
        );
    }

    #[test]
    fn keeps_spaces_inside_path() {
        assert_eq!(
            parse_command("frame /tmp/a b/c.rsf"),
            Some(Command::Frame(PathBuf::from("/tmp/a b/c.rsf")))
        );
    }

    #[test]
    fn strips_crlf() {
        assert_eq!(parse_command("quit\r\n"), Some(Command::Quit));
    }

    #[test]
    fn rejects_frame_without_path() {
        assert_eq!(parse_command("frame"), None);
        assert_eq!(parse_command("frame "), None);
        assert_eq!(parse_command("frame \n"), None);
    }

    #[test]
    fn rejects_unknown_and_empty() {
        assert_eq!(parse_command(""), None);
        assert_eq!(parse_command("\n"), None);
        assert_eq!(parse_command("hello"), None);
        assert_eq!(parse_command("FRAME /x"), None);
        assert_eq!(parse_command("quitx"), None);
        assert_eq!(parse_command("framex /x"), None);
    }
}
