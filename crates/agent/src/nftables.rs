use anyhow::{bail, Result};
use std::io::Write;
use std::process::{Command, Stdio};

// Known DoH provider IPs to block per managed UID, preventing encrypted DNS bypass.
const DOH_IPV4: &[&str] = &[
    "1.1.1.1", "1.0.0.1",               // Cloudflare
    "8.8.8.8", "8.8.4.4",               // Google
    "9.9.9.9", "149.112.112.112",        // Quad9
    "45.90.28.0", "45.90.30.0",          // NextDNS
    "94.140.14.14", "94.140.15.15",      // AdGuard
    "208.67.222.222", "208.67.220.220",  // OpenDNS
    "194.242.2.2", "194.242.2.3",        // Mullvad
];

const DOH_IPV6: &[&str] = &[
    "2606:4700:4700::1111", "2606:4700:4700::1001", // Cloudflare
    "2001:4860:4860::8888", "2001:4860:4860::8844", // Google
    "2620:fe::fe", "2620:fe::9",                    // Quad9
];

pub fn is_available() -> bool {
    Command::new("nft")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Full (re)setup of the screenguard nftables table.
///
/// Tears down any existing table first, then loads a fresh config for all
/// provided UID→proxy-port pairs. If `uid_port_pairs` is empty the table is
/// removed and nothing is installed.
pub fn setup(uid_port_pairs: &[(u32, u16)]) -> Result<()> {
    teardown_silent();

    if uid_port_pairs.is_empty() {
        return Ok(());
    }

    let config = build_ruleset(uid_port_pairs);
    tracing::debug!("Loading nftables ruleset:\n{config}");
    run_nft(&config)
}

/// Remove the screenguard table. Safe to call when the table does not exist.
pub fn teardown() -> Result<()> {
    let out = Command::new("nft")
        .args(["delete", "table", "inet", "screenguard"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        // "No such file or directory" means the table was never created — not an error.
        if !stderr.contains("No such file") && !stderr.contains("ENOENT") && !stderr.contains("does not exist") {
            bail!("nft delete table failed: {stderr}");
        }
    }
    Ok(())
}

fn teardown_silent() {
    let _ = Command::new("nft")
        .args(["delete", "table", "inet", "screenguard"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

pub fn build_ruleset(uid_port_pairs: &[(u32, u16)]) -> String {
    let mut s = String::from("table inet screenguard {\n");

    s.push_str("    set doh_ipv4 {\n        type ipv4_addr;\n        elements = { ");
    s.push_str(&DOH_IPV4.join(", "));
    s.push_str(" }\n    }\n");

    s.push_str("    set doh_ipv6 {\n        type ipv6_addr;\n        elements = { ");
    s.push_str(&DOH_IPV6.join(", "));
    s.push_str(" }\n    }\n");

    // NAT chain: redirect DNS queries from managed UIDs to their per-UID proxy port.
    // DNAT to 127.0.0.1 explicitly rather than using `redirect` — `redirect` in the
    // output chain sends to the outbound interface's primary IP, which breaks when the
    // system resolver is a non-loopback address (e.g. a LAN nameserver at 192.168.x.x).
    s.push_str("    chain dns_redirect {\n");
    s.push_str("        type nat hook output priority -100; policy accept;\n");
    for (uid, port) in uid_port_pairs {
        s.push_str(&format!("        meta skuid {uid} udp dport 53 dnat to 127.0.0.1:{port}\n"));
        s.push_str(&format!("        meta skuid {uid} tcp dport 53 dnat to 127.0.0.1:{port}\n"));
    }
    s.push_str("    }\n");

    // Filter chain: drop connections to known DoH endpoints for managed UIDs.
    s.push_str("    chain doh_block {\n");
    s.push_str("        type filter hook output priority 0; policy accept;\n");
    for (uid, _) in uid_port_pairs {
        s.push_str(&format!("        meta skuid {uid} ip  daddr @doh_ipv4 drop\n"));
        s.push_str(&format!("        meta skuid {uid} ip6 daddr @doh_ipv6 drop\n"));
    }
    s.push_str("    }\n");

    s.push_str("}\n");
    s
}

fn run_nft(config: &str) -> Result<()> {
    let mut child = Command::new("nft")
        .args(["-f", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(config.as_bytes())?;
    }

    let out = child.wait_with_output()?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        bail!("nft load failed: {stderr}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{build_ruleset, DOH_IPV4, DOH_IPV6};

    #[test]
    fn ruleset_contains_uid_and_port() {
        let rules = build_ruleset(&[(1000, 5354), (1001, 5355)]);
        assert!(rules.contains("meta skuid 1000 udp dport 53 dnat to 127.0.0.1:5354"));
        assert!(rules.contains("meta skuid 1000 tcp dport 53 dnat to 127.0.0.1:5354"));
        assert!(rules.contains("meta skuid 1001 udp dport 53 dnat to 127.0.0.1:5355"));
        assert!(rules.contains("meta skuid 1001 tcp dport 53 dnat to 127.0.0.1:5355"));
    }

    #[test]
    fn ruleset_contains_doh_block_rules() {
        let rules = build_ruleset(&[(1000, 5354)]);
        assert!(rules.contains("meta skuid 1000 ip  daddr @doh_ipv4 drop"));
        assert!(rules.contains("meta skuid 1000 ip6 daddr @doh_ipv6 drop"));
    }

    #[test]
    fn ruleset_references_doh_sets() {
        let rules = build_ruleset(&[(1000, 5354)]);
        // Sets must be declared before chains reference them.
        let doh_set_pos = rules.find("set doh_ipv4").unwrap();
        let chain_pos = rules.find("chain dns_redirect").unwrap();
        assert!(doh_set_pos < chain_pos);
    }

    #[test]
    fn ruleset_contains_known_doh_ips() {
        let rules = build_ruleset(&[(1000, 5354)]);
        for ip in DOH_IPV4 {
            assert!(rules.contains(ip), "missing DoH IPv4: {ip}");
        }
        for ip in DOH_IPV6 {
            assert!(rules.contains(ip), "missing DoH IPv6: {ip}");
        }
    }

    #[test]
    fn ruleset_has_correct_chain_hooks() {
        let rules = build_ruleset(&[(1000, 5354)]);
        assert!(rules.contains("type nat hook output priority -100"));
        assert!(rules.contains("type filter hook output priority 0"));
    }

    #[test]
    fn multiple_uids_each_get_own_rules() {
        let rules = build_ruleset(&[(1000, 5354), (1001, 5355), (1002, 5356)]);
        for (uid, port) in [(1000, 5354), (1001, 5355), (1002, 5356)] {
            assert!(rules.contains(&format!("meta skuid {uid} udp dport 53 dnat to 127.0.0.1:{port}")));
            assert!(rules.contains(&format!("meta skuid {uid} ip  daddr @doh_ipv4 drop")));
        }
    }

    #[test]
    fn ruleset_is_valid_table_block() {
        let rules = build_ruleset(&[(1000, 5354)]);
        assert!(rules.trim_start().starts_with("table inet screenguard {"));
        assert!(rules.trim_end().ends_with('}'));
    }
}
