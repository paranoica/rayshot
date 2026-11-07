use std::path::PathBuf;

/// Bumped whenever the extension and the binary would misread each other.
pub const PROTOCOL_VERSION: u32 = 2;

#[derive(Clone, Debug, PartialEq)]
pub struct MonitorRect {
    pub name: String,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, PartialEq)]
pub enum Command {
    Show {
        id: u64,
        width: u32,
        height: u32,
        monitors: Vec<MonitorRect>,
    },
    Frame {
        id: u64,
        path: PathBuf,
    },
    Cancel {
        id: u64,
    },
    Select {
        id: u64,
        from: (f32, f32),
        to: (f32, f32),
    },
    Quit,
}

fn parse_monitor(token: &str) -> Option<MonitorRect> {
    let (name, rect) = token.rsplit_once(':')?;
    if name.is_empty() {
        return None;
    }
    let mut parts = rect.split(',').map(|v| v.parse::<u32>().ok());
    let (x, y, width, height) = (
        parts.next()??,
        parts.next()??,
        parts.next()??,
        parts.next()??,
    );
    (parts.next().is_none() && width > 0 && height > 0).then(|| MonitorRect {
        name: name.to_string(),
        x,
        y,
        width,
        height,
    })
}

pub fn parse_monitors(tokens: &[String]) -> Vec<MonitorRect> {
    tokens.iter().filter_map(|t| parse_monitor(t)).collect()
}

fn parse_select(rest: &str) -> Option<Command> {
    let mut parts = rest.split(' ');
    let id = parts.next()?.parse().ok()?;
    let mut coord = || parts.next()?.parse::<f32>().ok().filter(|v| v.is_finite());
    let from = (coord()?, coord()?);
    let to = (coord()?, coord()?);
    parts
        .next()
        .is_none()
        .then_some(Command::Select { id, from, to })
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
    if let Some(rest) = line.strip_prefix("select ") {
        return parse_select(rest);
    }
    if let Some(id) = line.strip_prefix("cancel ") {
        return id.parse().ok().map(|id| Command::Cancel { id });
    }
    let rest = line.strip_prefix("show ")?;
    let mut parts = rest.split(' ');
    let id = parts.next()?.parse().ok()?;
    let width = parts.next()?.parse().ok().filter(|&w: &u32| w > 0)?;
    let height = parts.next()?.parse().ok().filter(|&h: &u32| h > 0)?;
    let monitors = parts.filter_map(parse_monitor).collect();
    Some(Command::Show {
        id,
        width,
        height,
        monitors,
    })
}

/// A crash is reported to the extension only once the kernel has finished the
/// core dump, which for a GPU process takes long enough to hold the freeze up.
pub fn skip_core_dumps() {
    let mut lim = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    if unsafe { libc::getrlimit(libc::RLIMIT_CORE, &mut lim) } == 0 {
        lim.rlim_cur = 0;
        unsafe { libc::setrlimit(libc::RLIMIT_CORE, &lim) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core_limit() -> (u64, u64) {
        let mut lim = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        assert_eq!(unsafe { libc::getrlimit(libc::RLIMIT_CORE, &mut lim) }, 0);
        (lim.rlim_cur as u64, lim.rlim_max as u64)
    }

    #[test]
    fn skipping_core_dumps_zeroes_the_soft_limit_only() {
        let (_, hard) = core_limit();
        let raised = libc::rlimit {
            rlim_cur: hard as libc::rlim_t,
            rlim_max: hard as libc::rlim_t,
        };
        assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_CORE, &raised) }, 0);
        if hard == 0 {
            return;
        }
        assert_ne!(core_limit().0, 0, "precondition: dumps enabled");
        super::skip_core_dumps();
        assert_eq!(core_limit(), (0, hard));
    }

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
                height: 1080,
                monitors: vec![]
            })
        );
        assert_eq!(
            parse_command(&format!("show {} 1 1", u64::MAX)),
            Some(Command::Show {
                id: u64::MAX,
                width: 1,
                height: 1,
                monitors: vec![]
            })
        );
    }

    fn monitor(name: &str, x: u32, y: u32, width: u32, height: u32) -> MonitorRect {
        MonitorRect {
            name: name.into(),
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn show_carries_where_each_monitor_sits_in_the_frame() {
        assert_eq!(
            parse_command("show 3 5760 2160 eDP-1:0,0,1920,1080 HDMI-A-1:1920,0,3840,2160\n"),
            Some(Command::Show {
                id: 3,
                width: 5760,
                height: 2160,
                monitors: vec![
                    monitor("eDP-1", 0, 0, 1920, 1080),
                    monitor("HDMI-A-1", 1920, 0, 3840, 2160)
                ]
            })
        );
    }

    #[test]
    fn a_malformed_monitor_is_dropped_but_the_capture_goes_on() {
        let bare = Some(Command::Show {
            id: 3,
            width: 1920,
            height: 1080,
            monitors: vec![],
        });
        for token in [
            "DP-1:0,0,1920",
            "DP-1:0,0,1920,1080,5",
            ":0,0,1920,1080",
            "DP-1",
            "5",
            "DP-1:0,0,0,1080",
            "DP-1:-5,0,1920,1080",
            "DP-1:a,0,1920,1080",
        ] {
            assert_eq!(
                parse_command(&format!("show 3 1920 1080 {token}")),
                bare,
                "{token}"
            );
        }
        assert_eq!(
            parse_command("show 3 3840 1080 DP-1 DP-2:1920,0,1920,1080"),
            Some(Command::Show {
                id: 3,
                width: 3840,
                height: 1080,
                monitors: vec![monitor("DP-2", 1920, 0, 1920, 1080)]
            })
        );
    }

    #[test]
    fn one_shot_arguments_carry_the_monitors_too() {
        let args: Vec<String> = ["DP-1:0,0,1920,1080", "DP-2:1920,0,1080,1920"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            parse_monitors(&args),
            vec![
                monitor("DP-1", 0, 0, 1920, 1080),
                monitor("DP-2", 1920, 0, 1080, 1920)
            ]
        );
        assert_eq!(parse_monitors(&[]), vec![]);
        assert_eq!(parse_monitors(&["DP-1:0,0".to_string()]), vec![]);
    }

    #[test]
    fn rejects_bad_show() {
        assert_eq!(parse_command("show 3 0 1080"), None);
        assert_eq!(parse_command("show 3 1920 0"), None);
        assert_eq!(parse_command("show 3 1920"), None);
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

    fn select(id: u64, from: (f32, f32), to: (f32, f32)) -> Option<Command> {
        Some(Command::Select { id, from, to })
    }

    #[test]
    fn parses_select() {
        assert_eq!(
            parse_command("select 4 10 20 300.5 400\n"),
            select(4, (10.0, 20.0), (300.5, 400.0))
        );
        assert_eq!(
            parse_command("select 4 -1920 0 5 6"),
            select(4, (-1920.0, 0.0), (5.0, 6.0))
        );
    }

    #[test]
    fn rejects_bad_select() {
        assert_eq!(parse_command("select 4 10 20 30"), None);
        assert_eq!(parse_command("select 4 10 20 30 40 9"), None);
        assert_eq!(parse_command("select 4 a 20 30 40"), None);
        assert_eq!(parse_command("select x 10 20 30 40"), None);
        assert_eq!(parse_command("select 4 nan 20 30 40"), None);
        assert_eq!(parse_command("select 4 10 inf 30 40"), None);
        assert_eq!(parse_command("select"), None);
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
