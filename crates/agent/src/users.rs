use anyhow::Result;
use common::models::LocalUser;
use std::collections::HashMap;

const SKIP_SHELLS: &[&str] = &["/usr/sbin/nologin", "/bin/nologin", "/bin/false", "/sbin/nologin"];

/// Parse /etc/passwd and return all real local users.
pub fn scan_local_users(min_uid: u32) -> Result<Vec<LocalUser>> {
    let content = std::fs::read_to_string("/etc/passwd")?;
    let mut users = parse_passwd(&content, min_uid);
    // Only root can read the sudoers files; elsewhere (tests, dev runs) leave it unknown.
    if let Ok(groups) = std::fs::read_to_string("/etc/group") {
        let sudoers = read_sudoers();
        let primary = parse_primary_gids(&content);
        for u in &mut users {
            u.can_admin = sudoers
                .as_deref()
                .map(|s| can_admin(&u.username, primary.get(&u.username).copied(), &groups, s));
        }
    }
    Ok(users)
}

/// Groups whose members may administer the system on common distributions.
const ADMIN_GROUPS: &[&str] = &["wheel", "sudo", "admin"];

/// `/etc/sudoers` plus `/etc/sudoers.d/*` (skipping names sudo itself ignores:
/// with a `.` or ending in `~`). `None` if the main file is unreadable.
fn read_sudoers() -> Option<String> {
    let mut all = std::fs::read_to_string("/etc/sudoers").ok()?;
    if let Ok(rd) = std::fs::read_dir("/etc/sudoers.d") {
        let mut files: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        files.sort();
        for f in files {
            let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.contains('.') || name.ends_with('~') {
                continue;
            }
            if let Ok(t) = std::fs::read_to_string(&f) {
                all.push('\n');
                all.push_str(&t);
            }
        }
    }
    Some(all)
}

fn parse_primary_gids(passwd: &str) -> HashMap<String, u32> {
    passwd
        .lines()
        .filter_map(|l| {
            let p: Vec<&str> = l.split(':').collect();
            Some((p.first()?.to_string(), p.get(3)?.parse().ok()?))
        })
        .collect()
}

/// Members of a group by name (supplementary members, plus users whose primary gid it is).
fn in_group(user: &str, primary_gid: Option<u32>, group_file: &str, group: &str) -> bool {
    group_file.lines().any(|l| {
        let p: Vec<&str> = l.split(':').collect();
        p.len() >= 4
            && p[0] == group
            && (p[3].split(',').any(|m| m.trim() == user) || p[2].parse::<u32>().ok().is_some_and(|g| Some(g) == primary_gid))
    })
}

/// Can `user` become administrator: in an admin group, or named — directly or
/// through `%group` — by a sudoers rule. Any rule counts (it runs something as root).
fn can_admin(user: &str, primary_gid: Option<u32>, group_file: &str, sudoers: &str) -> bool {
    if ADMIN_GROUPS.iter().any(|g| in_group(user, primary_gid, group_file, g)) {
        return true;
    }
    sudoers.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with('@')).any(|l| {
        let who = l.split_whitespace().next().unwrap_or("");
        if who == "Defaults" || who.starts_with("Defaults") {
            return false;
        }
        who == user || who.strip_prefix('%').is_some_and(|g| in_group(user, primary_gid, group_file, g))
    })
}

fn parse_passwd(content: &str, min_uid: u32) -> Vec<LocalUser> {
    let mut users = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // username:password:uid:gid:gecos:home:shell
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() < 7 {
            continue;
        }
        let username = parts[0];
        let uid: u32 = match parts[2].parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let gecos = parts[4]; // display name (GECOS field, first comma-delimited value)
        let shell = parts[6];

        if uid < min_uid {
            continue;
        }
        if username == "nobody" {
            continue;
        }
        if SKIP_SHELLS.contains(&shell) {
            continue;
        }

        let display_name = gecos.split(',').next().unwrap_or(username).trim();
        let display_name = if display_name.is_empty() {
            username.to_string()
        } else {
            display_name.to_string()
        };

        users.push(LocalUser {
            local_uid: uid,
            username: username.to_string(),
            display_name, can_admin: None
        });
    }
    users
}

/// Home directories by uid, from `/etc/passwd`.
pub fn home_dirs() -> HashMap<u32, std::path::PathBuf> {
    std::fs::read_to_string("/etc/passwd").map(|c| parse_home_dirs(&c)).unwrap_or_default()
}

fn parse_home_dirs(content: &str) -> HashMap<u32, std::path::PathBuf> {
    content
        .lines()
        .filter_map(|l| {
            let parts: Vec<&str> = l.trim().split(':').collect();
            (parts.len() >= 7).then(|| Some((parts[2].parse().ok()?, parts[5].into()))).flatten()
        })
        .collect()
}

/// Computes the diff between a previous user snapshot and the current scan.
/// Returns (current_list, added, removed_uids).
pub fn diff_users(
    previous: &HashMap<u32, LocalUser>,
    current: &[LocalUser],
) -> (Vec<LocalUser>, Vec<u32>) {
    let current_map: HashMap<u32, &LocalUser> =
        current.iter().map(|u| (u.local_uid, u)).collect();

    // new accounts, and accounts whose admin rights changed (removed from wheel…)
    let added: Vec<LocalUser> = current
        .iter()
        .filter(|u| previous.get(&u.local_uid).is_none_or(|p| p.can_admin != u.can_admin))
        .cloned()
        .collect();

    let removed_uids: Vec<u32> = previous
        .keys()
        .filter(|uid| !current_map.contains_key(uid))
        .copied()
        .collect();

    (added, removed_uids)
}

pub fn users_to_map(users: &[LocalUser]) -> HashMap<u32, LocalUser> {
    users.iter().map(|u| (u.local_uid, u.clone())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_admin_rights() {
        let groups = "root:x:0:\nwheel:x:998:adam,dad\nsudo:x:27:\nkids:x:1500:zuzia,jacek\nadmin:x:1600:\nzuzia:x:1001:\nboss:x:1700:\n";
        let sudoers = "Defaults env_reset\n# kid:\n#includedir /etc/sudoers.d\n@includedir /etc/sudoers.d\nroot ALL=(ALL) ALL\n%wheel ALL=(ALL) ALL\njacek ALL=(ALL) NOPASSWD: /usr/bin/pacman\n%kidsadmins ALL=(ALL) ALL\n";
        assert!(can_admin("adam", Some(1000), groups, sudoers)); // wheel member
        assert!(can_admin("jacek", Some(1002), groups, sudoers)); // own sudoers rule
        assert!(!can_admin("zuzia", Some(1001), groups, sudoers)); // just "kids"
        assert!(can_admin("boss", Some(1600), groups, sudoers)); // primary group is admin
        assert!(!can_admin("kid", Some(1003), groups, sudoers)); // only in a comment
        let with_group_rule = format!("{sudoers}%kids ALL=(ALL) ALL\n");
        assert!(can_admin("zuzia", Some(1001), groups, &with_group_rule));
    }

    #[test]
    fn admin_change_resends_the_user() {
        let u = |admin| LocalUser { local_uid: 1001, username: "zuzia".into(), display_name: "Zuzia".into(), can_admin: admin };
        let prev = users_to_map(&[u(Some(true))]);
        assert_eq!(diff_users(&prev, &[u(Some(true))]).0.len(), 0);
        assert_eq!(diff_users(&prev, &[u(Some(false))]).0.len(), 1);
    }

    #[test]
    fn parses_home_dirs() {
        let homes = parse_home_dirs("root:x:0:0:root:/root:/bin/bash\nkid:x:1001:1001:Kid:/home/kid:/bin/bash\nbroken\n");
        assert_eq!(homes[&1001], std::path::PathBuf::from("/home/kid"));
        assert_eq!(homes.len(), 2);
    }

    #[test]
    fn parse_passwd_filters_correctly() {
        let content = "\
root:x:0:0:root:/root:/bin/bash
daemon:x:1:1:Daemon:/usr/sbin:/usr/sbin/nologin
nobody:x:65534:65534:nobody:/nonexistent:/usr/sbin/nologin
tom:x:1000:1000:Tom Smith,,,:/home/tom:/bin/bash
alice:x:1001:1001:Alice:/home/alice:/bin/zsh
svc:x:999:999:Service:/srv:/bin/false
";
        let users = parse_passwd(content, 1000);
        assert_eq!(users.len(), 2);
        assert_eq!(users[0].username, "tom");
        assert_eq!(users[0].display_name, "Tom Smith");
        assert_eq!(users[1].username, "alice");
        assert_eq!(users[1].display_name, "Alice");
    }
}
