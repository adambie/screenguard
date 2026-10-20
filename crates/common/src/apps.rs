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

/// How a group rule matches an app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleKind {
    /// Exact app id, e.g. `desktop:firefox`.
    AppId,
    /// App id prefix, e.g. `steam:` for every Steam game.
    Prefix,
    /// freedesktop category, e.g. `Game`, `WebBrowser` (case-sensitive, as in the spec).
    Category,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct GroupRule {
    pub kind: RuleKind,
    pub value: String,
    /// Takes matching apps *out* of the group (e.g. one browser from "Web browsers").
    #[serde(default)]
    pub exclude: bool,
}

impl GroupRule {
    fn hits(&self, app_id: &str, categories: &[String]) -> bool {
        match self.kind {
            RuleKind::AppId => app_id == self.value,
            RuleKind::Prefix => !self.value.is_empty() && app_id.starts_with(&self.value),
            RuleKind::Category => categories.contains(&self.value),
        }
    }
}

/// An app is in a group if any include rule matches and no exclude rule does.
/// Shared by server (reports) and agent (enforcement) so they always agree.
pub fn group_matches(rules: &[GroupRule], app_id: &str, categories: &[String]) -> bool {
    rules.iter().any(|r| !r.exclude && r.hits(app_id, categories))
        && !rules.iter().any(|r| r.exclude && r.hits(app_id, categories))
}

/// The one group an app belongs to. `groups` is in display order (predefined
/// groups first, then by name). A group that names the app explicitly wins;
/// otherwise the first group whose rules (category, prefix) match. Exclusions
/// always apply. Shared by server and agent so reports and limits agree.
pub fn assign_group<'a>(groups: &[(&'a str, &[GroupRule])], app_id: &str, categories: &[String]) -> Option<&'a str> {
    let explicit = |rules: &[GroupRule]| {
        rules.iter().any(|r| r.kind == RuleKind::AppId && !r.exclude && r.value == app_id)
            && !rules.iter().any(|r| r.exclude && r.hits(app_id, categories))
    };
    groups
        .iter()
        .find(|(_, rules)| explicit(rules))
        .or_else(|| groups.iter().find(|(_, rules)| group_matches(rules, app_id, categories)))
        .map(|(id, _)| *id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_group_per_app() {
        let games = vec![rule(RuleKind::Category, "Game", false), rule(RuleKind::Prefix, "steam:", false)];
        let browsers = vec![rule(RuleKind::Category, "WebBrowser", false)];
        let school = vec![rule(RuleKind::AppId, "desktop:firefox", false), rule(RuleKind::AppId, "steam:570", false)];
        let groups = [("games", games.as_slice()), ("browsers", browsers.as_slice()), ("school", school.as_slice())];
        let cats = |c: &[&str]| c.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // explicit entry beats a category / prefix match in an earlier group
        assert_eq!(assign_group(&groups, "desktop:firefox", &cats(&["WebBrowser"])), Some("school"));
        assert_eq!(assign_group(&groups, "steam:570", &[]), Some("school"));
        // matching both categories: the first group in order
        assert_eq!(assign_group(&groups, "desktop:browser-game", &cats(&["Game", "WebBrowser"])), Some("games"));
        assert_eq!(assign_group(&groups, "desktop:chromium", &cats(&["WebBrowser"])), Some("browsers"));
        assert_eq!(assign_group(&groups, "desktop:foot", &[]), None);
        // an exclusion in the explicit group falls back to category matching
        let school_ex = vec![rule(RuleKind::AppId, "desktop:firefox", false), rule(RuleKind::AppId, "desktop:firefox", true)];
        let groups = [("browsers", browsers.as_slice()), ("school", school_ex.as_slice())];
        assert_eq!(assign_group(&groups, "desktop:firefox", &cats(&["WebBrowser"])), Some("browsers"));
    }

    fn rule(kind: RuleKind, value: &str, exclude: bool) -> GroupRule {
        GroupRule { kind, value: value.into(), exclude }
    }

    #[test]
    fn group_rules() {
        let games = [rule(RuleKind::Category, "Game", false), rule(RuleKind::Prefix, "steam:", false), rule(RuleKind::Prefix, "minecraft:", false)];
        let browsers = [rule(RuleKind::Category, "WebBrowser", false), rule(RuleKind::AppId, "desktop:org.kde.falkon", true)];
        let cats = |c: &[&str]| c.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(group_matches(&games, "desktop:supertux", &cats(&["Game", "ArcadeGame"])));
        assert!(group_matches(&games, "steam:570", &[]));
        assert!(group_matches(&games, "minecraft:java", &[]));
        assert!(!group_matches(&games, "desktop:firefox", &cats(&["Network", "WebBrowser"])));
        assert!(!group_matches(&games, "desktop:game-editor", &cats(&["game"]))); // case-sensitive
        assert!(group_matches(&browsers, "desktop:firefox", &cats(&["Network", "WebBrowser"])));
        assert!(!group_matches(&browsers, "desktop:org.kde.falkon", &cats(&["WebBrowser"]))); // excluded
        assert!(!group_matches(&[], "desktop:firefox", &[]));
        assert!(!group_matches(&[rule(RuleKind::Prefix, "", false)], "desktop:x", &[])); // empty prefix matches nothing
        // only exclusions: nothing is in the group
        assert!(!group_matches(&[rule(RuleKind::AppId, "desktop:x", true)], "desktop:y", &[]));
        let json = serde_json::to_string(&browsers[1]).unwrap();
        assert_eq!(json, r#"{"kind":"app_id","value":"desktop:org.kde.falkon","exclude":true}"#);
        let back: GroupRule = serde_json::from_str(r#"{"kind":"category","value":"Game"}"#).unwrap();
        assert!(!back.exclude);
    }

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
