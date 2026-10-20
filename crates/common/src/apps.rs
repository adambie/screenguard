//! App ids for per-app usage (docs/feature-app-usage-limits.md).
//!
//! An app id is `<kind>:<name>`, e.g. `desktop:org.kde.dolphin`, `flatpak:org.mozilla.firefox`,
//! `steam:570`. On the wire it stays a plain string so an older server can drop ids of
//! a kind it doesn't know instead of rejecting the whole message.

use std::fmt;

/// Longest accepted app id, prefix included.
pub const MAX_APP_ID_LEN: usize = 255;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AppKind {
    /// `.desktop` file id (without `.desktop`).
    Desktop,
    Flatpak,
    Snap,
    /// Steam AppId.
    Steam,
    /// `java` — Minecraft Java edition.
    Minecraft,
    /// Lower-cased `.exe` basename run under Wine/Proton.
    Wine,
    /// Not an app: executable basename, kept only in the hidden "other" bucket.
    Exe,
}

impl AppKind {
    pub fn prefix(self) -> &'static str {
        match self {
            AppKind::Desktop => "desktop",
            AppKind::Flatpak => "flatpak",
            AppKind::Snap => "snap",
            AppKind::Steam => "steam",
            AppKind::Minecraft => "minecraft",
            AppKind::Wine => "wine",
            AppKind::Exe => "exe",
        }
    }

    fn from_prefix(s: &str) -> Option<Self> {
        Some(match s {
            "desktop" => AppKind::Desktop,
            "flatpak" => AppKind::Flatpak,
            "snap" => AppKind::Snap,
            "steam" => AppKind::Steam,
            "minecraft" => AppKind::Minecraft,
            "wine" => AppKind::Wine,
            "exe" => AppKind::Exe,
            _ => return None,
        })
    }

    fn valid_name(self, name: &str) -> bool {
        let reverse_dns = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-');
        match self {
            AppKind::Flatpak => name.chars().all(reverse_dns),
            AppKind::Snap => name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
            AppKind::Steam => name.chars().all(|c| c.is_ascii_digit()),
            AppKind::Minecraft => name.chars().all(|c| c.is_ascii_lowercase()),
            // desktop ids are usually reverse-DNS, but Wine creates e.g.
            // `wine-Programs-Game Name.desktop`
            AppKind::Desktop | AppKind::Wine | AppKind::Exe => {
                name.chars().all(|c| !c.is_control() && c != '/')
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AppId {
    pub kind: AppKind,
    pub name: String,
}

impl AppId {
    /// Builds an id, returning `None` if it would not parse back.
    pub fn new(kind: AppKind, name: impl Into<String>) -> Option<Self> {
        let id = AppId { kind, name: name.into() };
        Self::parse(&id.to_string()).map(|_| id)
    }

    pub fn parse(s: &str) -> Option<Self> {
        if s.len() > MAX_APP_ID_LEN {
            return None;
        }
        let (prefix, name) = s.split_once(':')?;
        let kind = AppKind::from_prefix(prefix)?;
        if name.is_empty() || !kind.valid_name(name) {
            return None;
        }
        Some(AppId { kind, name: name.to_string() })
    }

    /// Everything except `exe:` is an app that counts towards usage reports.
    pub fn is_app(&self) -> bool {
        self.kind != AppKind::Exe
    }
}

impl fmt::Display for AppId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.kind.prefix(), self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_each_kind() {
        for (s, kind, name) in [
            ("desktop:org.kde.dolphin", AppKind::Desktop, "org.kde.dolphin"),
            ("desktop:foot", AppKind::Desktop, "foot"),
            ("desktop:kde4-kate", AppKind::Desktop, "kde4-kate"),
            ("desktop:wine-Programs-Game Name", AppKind::Desktop, "wine-Programs-Game Name"),
            ("flatpak:org.mozilla.firefox", AppKind::Flatpak, "org.mozilla.firefox"),
            ("snap:spotify", AppKind::Snap, "spotify"),
            ("steam:570", AppKind::Steam, "570"),
            ("minecraft:java", AppKind::Minecraft, "java"),
            ("wine:game launcher.exe", AppKind::Wine, "game launcher.exe"),
            ("exe:python3.14", AppKind::Exe, "python3.14"),
        ] {
            let id = AppId::parse(s).unwrap_or_else(|| panic!("{s} should parse"));
            assert_eq!((id.kind, id.name.as_str()), (kind, name));
            assert_eq!(id.to_string(), s);
        }
    }

    #[test]
    fn rejects_malformed() {
        for s in [
            "",
            "desktop",
            "desktop:",
            ":foo",
            "unknown:foo",
            "Desktop:foo",
            "desktop:../etc/passwd",
            "flatpak:has space",
            "steam:57a",
            "snap:Spotify",
            "minecraft:Java1",
            "exe:usr/bin/foo",
            "exe:bad\nname",
        ] {
            assert!(AppId::parse(s).is_none(), "{s:?} should be rejected");
        }
        let long = format!("desktop:{}", "a".repeat(MAX_APP_ID_LEN));
        assert!(AppId::parse(&long).is_none());
        let max = format!("desktop:{}", "a".repeat(MAX_APP_ID_LEN - "desktop:".len()));
        assert!(AppId::parse(&max).is_some());
    }

    #[test]
    fn new_validates_and_is_app() {
        assert!(AppId::new(AppKind::Steam, "abc").is_none());
        assert!(AppId::new(AppKind::Steam, "570").unwrap().is_app());
        assert!(!AppId::new(AppKind::Exe, "bash").unwrap().is_app());
    }
}
