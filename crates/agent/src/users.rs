use anyhow::Result;
use common::models::LocalUser;
use std::collections::HashMap;

const SKIP_SHELLS: &[&str] = &["/usr/sbin/nologin", "/bin/nologin", "/bin/false", "/sbin/nologin"];

/// Parse /etc/passwd and return all real local users.
pub fn scan_local_users(min_uid: u32) -> Result<Vec<LocalUser>> {
    let content = std::fs::read_to_string("/etc/passwd")?;
    let users = parse_passwd(&content, min_uid);
    Ok(users)
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
            display_name,
        });
    }
    users
}

/// Computes the diff between a previous user snapshot and the current scan.
/// Returns (current_list, added, removed_uids).
pub fn diff_users(
    previous: &HashMap<u32, LocalUser>,
    current: &[LocalUser],
) -> (Vec<LocalUser>, Vec<u32>) {
    let current_map: HashMap<u32, &LocalUser> =
        current.iter().map(|u| (u.local_uid, u)).collect();

    let added: Vec<LocalUser> = current
        .iter()
        .filter(|u| !previous.contains_key(&u.local_uid))
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
