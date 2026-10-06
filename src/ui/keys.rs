//! Rebindable keyboard shortcuts. Bindings are stored as text ("Ctrl+Shift+K"); only the ones you
//! changed are saved, so new defaults reach everyone else.
use eframe::egui::{Key, Modifiers};
use std::collections::BTreeMap;

/// (action, what it does, default binding)
pub const ACTIONS: [(&str, &str, &str); 24] = [
    ("play", "Play / pause", "Space"),
    ("fwd", "Seek forward 5 s", "Right"),
    ("back", "Seek back 5 s", "Left"),
    ("fwd_long", "Seek forward 30 s", "Shift+Right"),
    ("back_long", "Seek back 30 s", "Shift+Left"),
    ("next", "Next song", "Ctrl+Right"),
    ("prev", "Previous song", "Ctrl+Left"),
    ("vol_up", "Volume up", "Up"),
    ("vol_down", "Volume down", "Down"),
    ("mute", "Mute", "M"),
    ("shuffle", "Shuffle", "S"),
    ("repeat", "Repeat mode", "R"),
    ("like", "Like current song", "L"),
    ("vis", "Cycle visualizer", "V"),
    ("palette", "Command palette: search & do anything", "Ctrl+K"),
    ("search", "Search library", "Ctrl+F"),
    ("layout", "Edit layout", "Ctrl+E"),
    ("mini", "Mini player", "Ctrl+M"),
    ("settings", "Settings", "Ctrl+Comma"),
    ("import", "Import music", "Ctrl+I"),
    ("rescan", "Rescan library", ""),
    ("undo", "Undo the last removal / move / delete / merge", "Ctrl+Z"),
    ("private", "Private listening on / off", ""),
    ("nowplaying", "Full-screen now playing (Esc leaves)", "F11"),
];

/// These work even while typing in a text box (they use Ctrl and don't edit text).
pub const GLOBAL: [&str; 6] = ["palette", "search", "layout", "mini", "settings", "import"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Combo {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: Key,
}

impl Combo {
    pub fn parse(s: &str) -> Option<Combo> {
        let mut c = Combo { ctrl: false, shift: false, alt: false, key: Key::Space };
        let mut key = None;
        for part in s.split('+').map(str::trim) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "cmd" | "control" => c.ctrl = true,
                "shift" => c.shift = true,
                "alt" | "option" => c.alt = true,
                "" => {}
                _ => key = Some(Key::from_name(part)?),
            }
        }
        c.key = key?;
        Some(c)
    }
    pub fn from_event(key: Key, m: Modifiers) -> Combo {
        Combo { ctrl: m.command, shift: m.shift, alt: m.alt, key }
    }
    pub fn text(&self) -> String {
        let mut s = String::new();
        for (on, name) in [(self.ctrl, "Ctrl+"), (self.alt, "Alt+"), (self.shift, "Shift+")] {
            if on {
                s.push_str(name);
            }
        }
        s.push_str(self.key.name());
        s
    }
}

pub fn default_of(action: &str) -> &'static str {
    ACTIONS.iter().find(|a| a.0 == action).map(|a| a.2).unwrap_or("")
}

/// The binding in use for an action (your change, or the default).
pub fn binding(map: &BTreeMap<String, String>, action: &str) -> Option<Combo> {
    Combo::parse(map.get(action).map(|s| s.as_str()).unwrap_or(default_of(action)))
}

/// Actions whose bindings clash, as pairs.
pub fn conflicts(map: &BTreeMap<String, String>) -> Vec<(&'static str, &'static str)> {
    let b: Vec<(&str, Option<Combo>)> = ACTIONS.iter().map(|a| (a.0, binding(map, a.0))).collect();
    let mut out = Vec::new();
    for (i, (a, ca)) in b.iter().enumerate() {
        for (bb, cb) in &b[i + 1..] {
            if ca.is_some() && ca == cb {
                out.push((*a, *bb));
            }
        }
    }
    out
}

/// Which actions a key press triggers.
pub fn matching(map: &BTreeMap<String, String>, pressed: Combo) -> Vec<&'static str> {
    ACTIONS.iter().map(|a| a.0).filter(|a| binding(map, a) == Some(pressed)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_prints() {
        for (_, _, d) in ACTIONS {
            if !d.is_empty() {
                assert_eq!(Combo::parse(d).unwrap().text(), d);
            }
        }
        assert_eq!(Combo::parse("ctrl + shift + k").unwrap(), Combo { ctrl: true, shift: true, alt: false, key: Key::K });
        assert!(Combo::parse("Ctrl+").is_none() && Combo::parse("Banana").is_none());
    }

    #[test]
    fn detects_conflicts() {
        let mut m = BTreeMap::new();
        assert!(conflicts(&m).is_empty(), "defaults clash: {:?}", conflicts(&m));
        m.insert("like".to_string(), "M".to_string());
        assert_eq!(conflicts(&m), vec![("mute", "like")]);
        m.insert("mute".to_string(), String::new()); // unbound: no clash
        assert!(conflicts(&m).is_empty());
        m.insert("rescan".to_string(), "Ctrl+K".to_string());
        assert_eq!(conflicts(&m), vec![("palette", "rescan")]);
        let k = Combo::parse("Ctrl+K").unwrap();
        assert_eq!(matching(&m, k), vec!["palette", "rescan"]);
        assert_eq!(matching(&m, Combo::parse("M").unwrap()), vec!["like"]);
    }
}
