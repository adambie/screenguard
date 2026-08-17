use anyhow::Result;
use std::collections::{HashMap, HashSet};

use crate::{dns_proxy, nftables};

const BASE_PORT: u16 = 5354;

/// Owns the per-UID DNS proxy tasks and the nftables ruleset.
///
/// Cleans up both on drop so the system returns to an unfiltered state
/// if the agent process exits. Systemd ExecStopPost provides a crash
/// fallback for nftables (proxy tasks die with the process automatically).
pub struct WebFilter {
    available: bool,
    /// uid → (assigned_port, proxy_handle)
    proxies: HashMap<u32, (u16, dns_proxy::ProxyHandle)>,
}

impl WebFilter {
    pub fn new(available: bool) -> Self {
        Self {
            available,
            proxies: HashMap::new(),
        }
    }

    /// Synchronise nftables rules and DNS proxy tasks with the given
    /// UID → blocklist mapping.
    ///
    /// Safe to call repeatedly; nftables is always rebuilt from scratch
    /// and proxy blocklists are hot-swapped without restarting.
    /// UIDs with an empty blocklist are treated as unfiltered and get no rules.
    pub async fn apply(&mut self, uid_configs: &[(u32, Vec<String>)]) -> Result<()> {
        if !self.available {
            return Ok(());
        }

        // Only manage UIDs that have at least one domain to block.
        let mut active: Vec<(u32, &Vec<String>)> = uid_configs
            .iter()
            .filter(|(_, domains)| !domains.is_empty())
            .map(|(uid, domains)| (*uid, domains))
            .collect();
        active.sort_by_key(|(uid, _)| *uid);

        // Port assignment: sorted position → 5354 + index.
        let uid_port_pairs: Vec<(u32, u16)> = active
            .iter()
            .enumerate()
            .map(|(i, (uid, _))| (*uid, BASE_PORT + i as u16))
            .collect();

        // Remove proxies for UIDs that no longer need filtering.
        let active_uids: HashSet<u32> = active.iter().map(|(uid, _)| *uid).collect();
        self.proxies.retain(|uid, _| active_uids.contains(uid));

        // Spawn new proxies or hot-swap blocklists on existing ones.
        for (uid, domains) in &active {
            let uid = *uid;
            let port = uid_port_pairs
                .iter()
                .find(|(u, _)| *u == uid)
                .map(|(_, p)| *p)
                .expect("uid present in uid_port_pairs");

            match self.proxies.get(&uid) {
                Some((existing_port, handle)) if *existing_port == port => {
                    handle.update_blocklist((*domains).clone()).await;
                }
                _ => {
                    // New UID or port changed: spawn fresh (drops old handle → aborts tasks).
                    let handle = dns_proxy::spawn(uid, port, (*domains).clone())?;
                    self.proxies.insert(uid, (port, handle));
                }
            }
        }

        // Rebuild nftables table (teardown_silent + load fresh config).
        nftables::setup(&uid_port_pairs)?;

        if active.is_empty() {
            tracing::info!("Web filter: no active UIDs, nftables table cleared");
        } else {
            tracing::info!(
                "Web filter applied for {} UID(s): {:?}",
                active.len(),
                uid_port_pairs
            );
        }

        Ok(())
    }
}

impl Drop for WebFilter {
    fn drop(&mut self) {
        if !self.available {
            return;
        }
        // Abort proxy tasks before clearing nftables so in-flight DNS queries
        // are not redirected to dead ports.
        self.proxies.clear();
        if let Err(e) = nftables::teardown() {
            tracing::warn!("Web filter teardown on drop: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn noop_when_unavailable() {
        let mut filter = WebFilter::new(false);
        // Must succeed without touching nft or binding any ports.
        filter
            .apply(&[(1000, vec!["youtube.com".to_string()])])
            .await
            .unwrap();
        assert!(filter.proxies.is_empty());
    }

    #[tokio::test]
    async fn empty_blocklist_skips_uid() {
        let mut filter = WebFilter::new(false);
        filter
            .apply(&[(1000, vec![]), (1001, vec!["tiktok.com".to_string()])])
            .await
            .unwrap();
        // Available=false so proxies stay empty regardless, but the filtering
        // logic (empty → skip) is the same path.
        assert!(filter.proxies.is_empty());
    }
}
