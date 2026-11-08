use crate::scene::Tool;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    Tool(Tool),
    Copy,
    CopyText,
    Save,
    SaveAs,
    ZoomIn,
    ZoomOut,
    ZoomReset,
}

impl Command {
    pub const ALL: [Command; 19] = [
        Command::Tool(Tool::ALL[0]),
        Command::Tool(Tool::ALL[1]),
        Command::Tool(Tool::ALL[2]),
        Command::Tool(Tool::ALL[3]),
        Command::Tool(Tool::ALL[4]),
        Command::Tool(Tool::ALL[5]),
        Command::Tool(Tool::ALL[6]),
        Command::Tool(Tool::ALL[7]),
        Command::Tool(Tool::ALL[8]),
        Command::Tool(Tool::ALL[9]),
        Command::Tool(Tool::ALL[10]),
        Command::Tool(Tool::ALL[11]),
        Command::Copy,
        Command::CopyText,
        Command::Save,
        Command::SaveAs,
        Command::ZoomIn,
        Command::ZoomOut,
        Command::ZoomReset,
    ];

    pub fn name(self) -> String {
        match self {
            Command::Tool(t) => t.name().to_string(),
            Command::Copy => "copy".into(),
            Command::CopyText => "copy-text".into(),
            Command::Save => "save".into(),
            Command::SaveAs => "save-as".into(),
            Command::ZoomIn => "zoom-in".into(),
            Command::ZoomOut => "zoom-out".into(),
            Command::ZoomReset => "zoom-reset".into(),
        }
    }

    pub fn from_name(name: &str) -> Option<Command> {
        Command::ALL.into_iter().find(|c| c.name() == name)
    }

    pub fn label(self) -> &'static str {
        match self {
            Command::Tool(t) => t.label(),
            Command::Copy => "Copy",
            Command::CopyText => "Copy text",
            Command::Save => "Save",
            Command::SaveAs => "Save as…",
            Command::ZoomIn => "Zoom in",
            Command::ZoomOut => "Zoom out",
            Command::ZoomReset => "Actual size",
        }
    }

    fn default_chord(self) -> Chord {
        let text = match self {
            Command::Tool(t) => t.key(),
            Command::Copy => "Ctrl+C",
            Command::CopyText => "Ctrl+Shift+C",
            Command::Save => "Ctrl+S",
            Command::SaveAs => "Ctrl+Shift+S",
            Command::ZoomIn => "Ctrl+=",
            Command::ZoomOut => "Ctrl+-",
            Command::ZoomReset => "Ctrl+0",
        };
        Chord::parse(text).expect("default chords parse")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Letter(u8),
    Digit(u8),
    F(u8),
    Equal,
    Minus,
    BracketLeft,
    BracketRight,
    Semicolon,
    Quote,
    Comma,
    Period,
    Slash,
    Backslash,
    Backquote,
    Space,
}

const SYMBOLS: [(Key, &str); 11] = [
    (Key::Equal, "="),
    (Key::Minus, "-"),
    (Key::BracketLeft, "["),
    (Key::BracketRight, "]"),
    (Key::Semicolon, ";"),
    (Key::Quote, "'"),
    (Key::Comma, ","),
    (Key::Period, "."),
    (Key::Slash, "/"),
    (Key::Backslash, "\\"),
    (Key::Backquote, "`"),
];

impl Key {
    fn text(self) -> String {
        match self {
            Key::Letter(c) | Key::Digit(c) => (c as char).to_string(),
            Key::F(n) => format!("F{n}"),
            Key::Space => "Space".into(),
            other => SYMBOLS
                .iter()
                .find(|(k, _)| *k == other)
                .map_or(String::new(), |(_, t)| t.to_string()),
        }
    }

    fn parse(text: &str) -> Option<Key> {
        let upper = text.to_ascii_uppercase();
        let bytes = upper.as_bytes();
        match bytes {
            [c] if c.is_ascii_uppercase() => return Some(Key::Letter(*c)),
            [c] if c.is_ascii_digit() => return Some(Key::Digit(*c)),
            _ => {}
        }
        if upper == "SPACE" {
            return Some(Key::Space);
        }
        if let Some(n) = upper.strip_prefix('F').and_then(|n| n.parse::<u8>().ok()) {
            return (1..=12).contains(&n).then_some(Key::F(n));
        }
        SYMBOLS.iter().find(|(_, t)| *t == text).map(|(k, _)| *k)
    }

    pub fn from_code(code: winit::keyboard::KeyCode) -> Option<Key> {
        use winit::keyboard::KeyCode as K;
        let letters = [
            K::KeyA,
            K::KeyB,
            K::KeyC,
            K::KeyD,
            K::KeyE,
            K::KeyF,
            K::KeyG,
            K::KeyH,
            K::KeyI,
            K::KeyJ,
            K::KeyK,
            K::KeyL,
            K::KeyM,
            K::KeyN,
            K::KeyO,
            K::KeyP,
            K::KeyQ,
            K::KeyR,
            K::KeyS,
            K::KeyT,
            K::KeyU,
            K::KeyV,
            K::KeyW,
            K::KeyX,
            K::KeyY,
            K::KeyZ,
        ];
        if let Some(i) = letters.iter().position(|k| *k == code) {
            return Some(Key::Letter(b'A' + i as u8));
        }
        let digits = [
            K::Digit0,
            K::Digit1,
            K::Digit2,
            K::Digit3,
            K::Digit4,
            K::Digit5,
            K::Digit6,
            K::Digit7,
            K::Digit8,
            K::Digit9,
        ];
        if let Some(i) = digits.iter().position(|k| *k == code) {
            return Some(Key::Digit(b'0' + i as u8));
        }
        let fkeys = [
            K::F1,
            K::F2,
            K::F3,
            K::F4,
            K::F5,
            K::F6,
            K::F7,
            K::F8,
            K::F9,
            K::F10,
            K::F11,
            K::F12,
        ];
        if let Some(i) = fkeys.iter().position(|k| *k == code) {
            return Some(Key::F(i as u8 + 1));
        }
        Some(match code {
            K::Equal => Key::Equal,
            K::Minus => Key::Minus,
            K::BracketLeft => Key::BracketLeft,
            K::BracketRight => Key::BracketRight,
            K::Semicolon => Key::Semicolon,
            K::Quote => Key::Quote,
            K::Comma => Key::Comma,
            K::Period => Key::Period,
            K::Slash => Key::Slash,
            K::Backslash => Key::Backslash,
            K::Backquote => Key::Backquote,
            K::Space => Key::Space,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Chord {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: Key,
}

impl Chord {
    pub fn parse(text: &str) -> Option<Chord> {
        let mut parts: Vec<&str> = text.split('+').collect();
        let key = Key::parse(parts.pop()?)?;
        let mut chord = Chord {
            ctrl: false,
            shift: false,
            alt: false,
            key,
        };
        for m in parts {
            match m.to_ascii_lowercase().as_str() {
                "ctrl" => chord.ctrl = true,
                "shift" => chord.shift = true,
                "alt" => chord.alt = true,
                _ => return None,
            }
        }
        Some(chord)
    }

    fn parts(self) -> Vec<String> {
        let mut out = Vec::new();
        if self.ctrl {
            out.push("Ctrl".to_string());
        }
        if self.shift {
            out.push("Shift".to_string());
        }
        if self.alt {
            out.push("Alt".to_string());
        }
        out.push(self.key.text());
        out
    }

    pub fn text(self) -> String {
        self.parts().join("+")
    }

    pub fn label(self) -> String {
        self.parts().join(" ")
    }
}

pub fn reserved(chord: Chord) -> Option<&'static str> {
    let plain = !chord.ctrl && !chord.shift && !chord.alt;
    match chord.key {
        Key::Digit(d) if plain && d != b'0' => Some("1–9 pick colours"),
        Key::Letter(b'Z') if chord.ctrl => Some("Ctrl+Z undoes and redoes"),
        Key::Letter(b'Y') if chord.ctrl => Some("Ctrl+Y redoes"),
        Key::Letter(b'D') if chord.ctrl => Some("Ctrl+D duplicates a shape"),
        _ => None,
    }
}

#[derive(Debug, PartialEq)]
pub enum Assign {
    Done,
    Conflict(Command),
    Reserved(&'static str),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Bindings(Vec<(Command, Chord)>);

impl Default for Bindings {
    fn default() -> Self {
        Bindings::defaults()
    }
}

impl Bindings {
    pub fn defaults() -> Bindings {
        Bindings(
            Command::ALL
                .iter()
                .map(|c| (*c, c.default_chord()))
                .collect(),
        )
    }

    pub fn chord(&self, command: Command) -> Chord {
        self.0
            .iter()
            .find(|(c, _)| *c == command)
            .map_or_else(|| command.default_chord(), |(_, k)| *k)
    }

    pub fn command(&self, chord: Chord) -> Option<Command> {
        self.0.iter().find(|(_, k)| *k == chord).map(|(c, _)| *c)
    }

    pub fn resolve(
        &self,
        ctrl: bool,
        shift: bool,
        alt: bool,
        code: winit::keyboard::KeyCode,
    ) -> Option<Command> {
        use winit::keyboard::KeyCode;
        let key = match code {
            KeyCode::NumpadAdd => Key::Equal,
            KeyCode::NumpadSubtract => Key::Minus,
            _ => Key::from_code(code)?,
        };
        let exact = Chord {
            ctrl,
            shift,
            alt,
            key,
        };
        if let Some(command) = self.command(exact) {
            return Some(command);
        }
        let unshifted = self.command(Chord {
            shift: false,
            ..exact
        })?;
        (shift
            && matches!(
                unshifted,
                Command::Tool(_) | Command::ZoomIn | Command::ZoomOut
            ))
        .then_some(unshifted)
    }

    fn set(&mut self, command: Command, chord: Chord) {
        if let Some(slot) = self.0.iter_mut().find(|(c, _)| *c == command) {
            slot.1 = chord;
        }
    }

    pub fn assign(&mut self, command: Command, chord: Chord) -> Assign {
        if let Some(why) = reserved(chord) {
            return Assign::Reserved(why);
        }
        match self.command(chord) {
            Some(other) if other != command => Assign::Conflict(other),
            _ => {
                self.set(command, chord);
                Assign::Done
            }
        }
    }

    pub fn swap(&mut self, a: Command, b: Command) {
        let (ka, kb) = (self.chord(a), self.chord(b));
        self.set(a, kb);
        self.set(b, ka);
    }

    pub fn is_default(&self, command: Command) -> bool {
        self.chord(command) == command.default_chord()
    }

    pub fn reset(&mut self, command: Command) {
        let wanted = command.default_chord();
        if let Some(other) = self.command(wanted).filter(|o| *o != command) {
            self.swap(command, other);
        } else {
            self.set(command, wanted);
        }
    }

    pub fn reset_all(&mut self) {
        *self = Bindings::defaults();
    }

    pub fn to_pref(&self) -> String {
        Command::ALL
            .iter()
            .filter(|c| !self.is_default(**c))
            .map(|c| format!("{}:{}", c.name(), self.chord(*c).text()))
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn from_pref(text: &str) -> Bindings {
        let mut entries: Vec<(Command, Chord)> = Vec::new();
        for token in text.split_whitespace() {
            let Some((name, key)) = token.split_once(':') else {
                continue;
            };
            let (Some(command), Some(chord)) = (Command::from_name(name), Chord::parse(key)) else {
                continue;
            };
            if reserved(chord).is_none() {
                entries.retain(|(c, _)| *c != command);
                entries.push((command, chord));
            }
        }
        let mut out = Bindings::defaults();
        for (command, chord) in &entries {
            out.set(*command, *chord);
        }
        for _ in 0..Command::ALL.len() * 2 {
            let clash = Command::ALL.iter().find_map(|a| {
                let others: Vec<Command> = Command::ALL
                    .iter()
                    .filter(|b| *b != a && out.chord(**b) == out.chord(*a))
                    .copied()
                    .collect();
                (!others.is_empty()).then(|| (*a, others))
            });
            let Some((a, others)) = clash else {
                return out;
            };
            let rank = |c: Command| entries.iter().position(|(e, _)| *e == c);
            let mut group = others;
            group.push(a);
            let loser = group
                .iter()
                .copied()
                .filter(|c| rank(*c).is_some())
                .max_by_key(|c| rank(*c))
                .unwrap_or(a);
            out.set(loser, loser.default_chord());
            entries.retain(|(c, _)| *c != loser);
        }
        Bindings::defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(text: &str) -> Chord {
        Chord::parse(text).unwrap()
    }

    #[test]
    fn defaults_match_the_keys_the_readme_lists() {
        let b = Bindings::defaults();
        assert_eq!(b.chord(Command::Tool(Tool::Select)), chord("S"));
        assert_eq!(b.chord(Command::Tool(Tool::Picker)), chord("I"));
        assert_eq!(b.chord(Command::Copy), chord("Ctrl+C"));
        assert_eq!(b.chord(Command::CopyText), chord("Ctrl+Shift+C"));
        assert_eq!(b.chord(Command::Save), chord("Ctrl+S"));
        assert_eq!(b.chord(Command::SaveAs), chord("Ctrl+Shift+S"));
        assert_eq!(b.chord(Command::ZoomIn), chord("Ctrl+="));
        assert_eq!(b.chord(Command::ZoomOut), chord("Ctrl+-"));
        assert_eq!(b.chord(Command::ZoomReset), chord("Ctrl+0"));
    }

    #[test]
    fn every_default_is_unique_and_allowed() {
        let b = Bindings::defaults();
        for (i, a) in Command::ALL.iter().enumerate() {
            let ca = b.chord(*a);
            assert_eq!(reserved(ca), None, "{a:?}");
            for c in &Command::ALL[i + 1..] {
                assert_ne!(ca, b.chord(*c), "{a:?} and {c:?}");
            }
        }
    }

    #[test]
    fn chords_read_and_write_the_same_text() {
        for text in [
            "S",
            "Ctrl+C",
            "Ctrl+Shift+S",
            "Ctrl+=",
            "Ctrl+-",
            "Alt+F5",
            "Shift+[",
            "Space",
        ] {
            assert_eq!(chord(text).text(), text);
        }
        assert_eq!(chord("ctrl+shift+s"), chord("Ctrl+Shift+S"));
        assert_eq!(chord("Shift+Ctrl+S"), chord("Ctrl+Shift+S"));
        assert_eq!(chord("Ctrl+Shift+S").label(), "Ctrl Shift S");
        for bad in [
            "",
            "Ctrl",
            "Ctrl+",
            "Hyper+S",
            "Ctrl+SS",
            "Ctrl+Enter",
            "F13",
            "+",
        ] {
            assert_eq!(Chord::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_editor_s_fixed_keys_cannot_be_taken() {
        for text in [
            "1",
            "5",
            "9",
            "Ctrl+Z",
            "Ctrl+Shift+Z",
            "Ctrl+Alt+Z",
            "Ctrl+Alt+Shift+Z",
            "Ctrl+Y",
            "Ctrl+Shift+Y",
            "Ctrl+Alt+Y",
            "Ctrl+D",
            "Ctrl+Shift+D",
            "Ctrl+Alt+D",
        ] {
            assert!(reserved(chord(text)).is_some(), "{text}");
        }
        for text in ["0", "Ctrl+1", "Shift+5", "Q", "Ctrl+Q", "Alt+Z"] {
            assert_eq!(reserved(chord(text)), None, "{text}");
        }
    }

    #[test]
    fn a_free_chord_is_assigned_and_looked_up() {
        let mut b = Bindings::defaults();
        assert_eq!(b.assign(Command::Tool(Tool::Pen), chord("Q")), Assign::Done);
        assert_eq!(b.chord(Command::Tool(Tool::Pen)), chord("Q"));
        assert_eq!(b.command(chord("Q")), Some(Command::Tool(Tool::Pen)));
        assert_eq!(b.command(chord("P")), None, "the old key is free again");
    }

    #[test]
    fn a_taken_chord_is_reported_and_nothing_changes() {
        let mut b = Bindings::defaults();
        assert_eq!(
            b.assign(Command::Tool(Tool::Pen), chord("Ctrl+S")),
            Assign::Conflict(Command::Save)
        );
        assert_eq!(b.chord(Command::Tool(Tool::Pen)), chord("P"));
        assert_eq!(b.chord(Command::Save), chord("Ctrl+S"));
        assert_eq!(
            b.assign(Command::Tool(Tool::Pen), chord("P")),
            Assign::Done,
            "its own key is not a conflict"
        );
    }

    #[test]
    fn a_reserved_chord_is_refused() {
        let mut b = Bindings::defaults();
        assert!(matches!(
            b.assign(Command::Copy, chord("Ctrl+Z")),
            Assign::Reserved(_)
        ));
        assert_eq!(b.chord(Command::Copy), chord("Ctrl+C"));
    }

    #[test]
    fn swapping_trades_the_two_keys() {
        let mut b = Bindings::defaults();
        b.swap(Command::Tool(Tool::Pen), Command::Save);
        assert_eq!(b.chord(Command::Tool(Tool::Pen)), chord("Ctrl+S"));
        assert_eq!(b.chord(Command::Save), chord("P"));
    }

    #[test]
    fn reset_brings_back_the_default() {
        let mut b = Bindings::defaults();
        b.assign(Command::Tool(Tool::Pen), chord("Q"));
        b.assign(Command::Copy, chord("Ctrl+K"));
        assert!(!b.is_default(Command::Copy));
        b.reset(Command::Copy);
        assert!(b.is_default(Command::Copy));
        assert_eq!(b.chord(Command::Tool(Tool::Pen)), chord("Q"));
        b.reset_all();
        assert_eq!(b, Bindings::defaults());
    }

    #[test]
    fn reset_of_a_key_taken_since_swaps_it_back() {
        let mut b = Bindings::defaults();
        b.assign(Command::Tool(Tool::Pen), chord("Q"));
        b.assign(Command::Tool(Tool::Line), chord("P"));
        b.reset(Command::Tool(Tool::Pen));
        assert_eq!(b.chord(Command::Tool(Tool::Pen)), chord("P"));
        assert_eq!(b.chord(Command::Tool(Tool::Line)), chord("Q"));
    }

    #[test]
    fn only_changed_keys_are_written() {
        let mut b = Bindings::defaults();
        assert_eq!(b.to_pref(), "");
        b.assign(Command::Tool(Tool::Pen), chord("Q"));
        b.assign(Command::SaveAs, chord("Ctrl+Alt+S"));
        assert_eq!(b.to_pref(), "pen:Q save-as:Ctrl+Alt+S");
        assert_eq!(Bindings::from_pref(&b.to_pref()), b);
    }

    #[test]
    fn a_damaged_pref_keeps_the_good_entries() {
        let b =
            Bindings::from_pref("pen:Q bogus:K copy:Ctrl+Z line: rectangle:Q save:Ctrl+K nonsense");
        assert_eq!(b.chord(Command::Tool(Tool::Pen)), chord("Q"));
        assert_eq!(
            b.chord(Command::Copy),
            chord("Ctrl+C"),
            "reserved is ignored"
        );
        assert_eq!(b.chord(Command::Tool(Tool::Line)), chord("L"));
        assert_eq!(
            b.chord(Command::Tool(Tool::Rect)),
            chord("R"),
            "a clash with pen is ignored"
        );
        assert_eq!(b.chord(Command::Save), chord("Ctrl+K"));
    }

    #[test]
    fn command_names_round_trip() {
        for c in Command::ALL {
            assert_eq!(Command::from_name(&c.name()), Some(c));
        }
        assert_eq!(Command::from_name("nope"), None);
    }

    #[test]
    fn a_saved_swap_survives_loading() {
        let b = Bindings::from_pref("pen:R rectangle:P");
        assert_eq!(b.chord(Command::Tool(Tool::Pen)), chord("R"));
        assert_eq!(b.chord(Command::Tool(Tool::Rect)), chord("P"));
    }

    #[test]
    fn an_entry_on_an_untouched_default_is_dropped() {
        let b = Bindings::from_pref("pen:R");
        assert_eq!(b.chord(Command::Tool(Tool::Rect)), chord("R"));
        assert_eq!(b.chord(Command::Tool(Tool::Pen)), chord("P"));
    }

    #[test]
    fn the_last_of_repeated_entries_wins() {
        let b = Bindings::from_pref("pen:Q pen:W");
        assert_eq!(b.chord(Command::Tool(Tool::Pen)), chord("W"));
        assert_eq!(b.command(chord("Q")), None);
    }

    #[test]
    fn plus_and_the_keypad_zoom_the_export() {
        use winit::keyboard::KeyCode;
        let b = Bindings::defaults();
        assert_eq!(
            b.resolve(true, true, false, KeyCode::Equal),
            Some(Command::ZoomIn),
            "Ctrl+Shift+= is Ctrl++"
        );
        assert_eq!(
            b.resolve(true, false, false, KeyCode::NumpadAdd),
            Some(Command::ZoomIn)
        );
        assert_eq!(
            b.resolve(true, false, false, KeyCode::NumpadSubtract),
            Some(Command::ZoomOut)
        );
        assert_eq!(b.resolve(false, false, false, KeyCode::NumpadAdd), None);
    }

    #[test]
    fn shift_still_picks_a_tool_but_not_an_action() {
        use winit::keyboard::KeyCode;
        let b = Bindings::defaults();
        assert_eq!(
            b.resolve(false, true, false, KeyCode::KeyR),
            Some(Command::Tool(Tool::Rect))
        );
        assert_eq!(
            b.resolve(true, false, false, KeyCode::KeyS),
            Some(Command::Save)
        );
        assert_eq!(
            b.resolve(true, true, false, KeyCode::KeyS),
            Some(Command::SaveAs)
        );
        assert_eq!(b.resolve(false, true, false, KeyCode::KeyZ), None);
        assert_eq!(b.resolve(false, false, true, KeyCode::KeyR), None);
        let mut rebound = Bindings::defaults();
        assert_eq!(rebound.assign(Command::Copy, chord("Ctrl+K")), Assign::Done);
        assert_eq!(rebound.resolve(true, true, false, KeyCode::KeyK), None);
    }
}
