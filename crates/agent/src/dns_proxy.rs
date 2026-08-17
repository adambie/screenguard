use anyhow::Result;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::RwLock;
use tokio::time::timeout;

const DNS_TIMEOUT: Duration = Duration::from_secs(5);

/// Handle to a running per-UID DNS proxy. Aborts listener tasks on drop.
pub struct ProxyHandle {
    blocklist: Arc<RwLock<Vec<String>>>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl ProxyHandle {
    /// Replace the blocklist without restarting the proxy.
    pub async fn update_blocklist(&self, domains: Vec<String>) {
        *self.blocklist.write().await = domains;
    }
}

impl Drop for ProxyHandle {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// Spawn UDP + TCP DNS proxy listeners for a single managed UID.
///
/// All DNS queries from that UID are redirected here by nftables. Blocked
/// domains get an NXDOMAIN response; everything else is forwarded to the
/// upstream resolver.
pub fn spawn(uid: u32, port: u16, blocked_domains: Vec<String>) -> Result<ProxyHandle> {
    let upstream = read_upstream_resolver();
    tracing::debug!("DNS proxy for UID {uid} on port {port}, upstream {upstream}");

    let blocklist = Arc::new(RwLock::new(blocked_domains));
    let mut tasks = Vec::new();

    let bl = blocklist.clone();
    tasks.push(tokio::spawn(async move {
        if let Err(e) = run_udp(port, bl, upstream).await {
            tracing::error!("DNS proxy UDP stopped (uid {uid}): {e}");
        }
    }));

    let bl = blocklist.clone();
    tasks.push(tokio::spawn(async move {
        if let Err(e) = run_tcp(port, bl, upstream).await {
            tracing::error!("DNS proxy TCP stopped (uid {uid}): {e}");
        }
    }));

    Ok(ProxyHandle { blocklist, tasks })
}

async fn run_udp(
    port: u16,
    blocklist: Arc<RwLock<Vec<String>>>,
    upstream: SocketAddr,
) -> Result<()> {
    let socket = Arc::new(UdpSocket::bind(format!("127.0.0.1:{port}")).await?);
    tracing::info!("DNS proxy UDP listening on 127.0.0.1:{port}");

    let mut buf = [0u8; 4096];
    loop {
        let (len, peer) = socket.recv_from(&mut buf).await?;
        let query = buf[..len].to_vec();
        let blocked_snapshot = blocklist.read().await.clone();
        let sock = socket.clone();
        tokio::spawn(async move {
            if let Some(resp) = handle_query(&query, &blocked_snapshot, upstream).await {
                let _ = sock.send_to(&resp, peer).await;
            }
        });
    }
}

async fn run_tcp(
    port: u16,
    blocklist: Arc<RwLock<Vec<String>>>,
    upstream: SocketAddr,
) -> Result<()> {
    let listener = TcpListener::bind(format!("127.0.0.1:{port}")).await?;
    tracing::info!("DNS proxy TCP listening on 127.0.0.1:{port}");

    loop {
        let (stream, _peer) = listener.accept().await?;
        let bl = blocklist.clone();
        tokio::spawn(handle_tcp_connection(stream, bl, upstream));
    }
}

async fn handle_tcp_connection(
    mut stream: tokio::net::TcpStream,
    blocklist: Arc<RwLock<Vec<String>>>,
    upstream: SocketAddr,
) {
    // DNS over TCP: each message is prefixed with a 2-byte big-endian length.
    loop {
        let mut len_buf = [0u8; 2];
        if stream.read_exact(&mut len_buf).await.is_err() {
            break;
        }
        let len = u16::from_be_bytes(len_buf) as usize;
        if len == 0 {
            break;
        }

        let mut query = vec![0u8; len];
        if stream.read_exact(&mut query).await.is_err() {
            break;
        }

        let blocked_snapshot = blocklist.read().await.clone();
        let resp = handle_query(&query, &blocked_snapshot, upstream).await;

        match resp {
            Some(resp) => {
                let len_prefix = (resp.len() as u16).to_be_bytes();
                if stream.write_all(&len_prefix).await.is_err() {
                    break;
                }
                if stream.write_all(&resp).await.is_err() {
                    break;
                }
            }
            None => break,
        }
    }
}

async fn handle_query(
    query: &[u8],
    blocked: &[String],
    upstream: SocketAddr,
) -> Option<Vec<u8>> {
    if query.len() < 12 {
        return None;
    }
    match parse_qname(query) {
        Some(ref qname) if is_blocked(qname, blocked) => {
            tracing::info!("DNS: blocked {qname}");
            Some(make_nxdomain(query))
        }
        _ => forward_udp(query, upstream).await,
    }
}

async fn forward_udp(query: &[u8], upstream: SocketAddr) -> Option<Vec<u8>> {
    let sock = UdpSocket::bind("0.0.0.0:0").await.ok()?;
    sock.connect(upstream).await.ok()?;

    timeout(DNS_TIMEOUT, sock.send(query)).await.ok()?.ok()?;

    let mut buf = vec![0u8; 4096];
    match timeout(DNS_TIMEOUT, sock.recv(&mut buf)).await {
        Ok(Ok(n)) => Some(buf[..n].to_vec()),
        _ => None,
    }
}

/// Extract the queried domain name from a DNS query packet, returned as a
/// lowercase dotted string (e.g. `"www.youtube.com"`).
pub fn parse_qname(query: &[u8]) -> Option<String> {
    if query.len() < 13 {
        return None;
    }
    let mut labels: Vec<String> = Vec::new();
    let mut pos = 12; // first byte after the 12-byte DNS header
    loop {
        let len = *query.get(pos)? as usize;
        if len == 0 {
            break;
        }
        // Compression pointers (0xC0 prefix) should not appear in client queries.
        if len & 0xC0 == 0xC0 {
            return None;
        }
        pos += 1;
        let label_bytes = query.get(pos..pos + len)?;
        labels.push(String::from_utf8_lossy(label_bytes).to_lowercase());
        pos += len;
    }
    if labels.is_empty() {
        None
    } else {
        Some(labels.join("."))
    }
}

/// Build an NXDOMAIN response from a DNS query packet by flipping QR=1,
/// setting RCODE=3, and zeroing the answer/authority/additional counts.
pub fn make_nxdomain(query: &[u8]) -> Vec<u8> {
    let mut resp = query.to_vec();
    resp[2] |= 0x80; // QR = 1 (response)
    resp[3] = 0x03;  // RA=0, RCODE=3 (NXDOMAIN)
    resp[6] = 0;
    resp[7] = 0; // ANCOUNT = 0
    resp[8] = 0;
    resp[9] = 0; // NSCOUNT = 0
    resp[10] = 0;
    resp[11] = 0; // ARCOUNT = 0
    resp
}

/// Return true if `qname` matches a blocked domain exactly or as a subdomain.
///
/// `youtube.com` in the blocklist blocks `youtube.com` and `www.youtube.com`
/// but not `fakeyoutube.com`.
pub fn is_blocked(qname: &str, blocked: &[String]) -> bool {
    let name = qname.trim_end_matches('.');
    blocked.iter().any(|b| {
        let b = b.trim_end_matches('.');
        name == b || name.ends_with(&format!(".{b}"))
    })
}

/// Read the first non-loopback nameserver from `/etc/resolv.conf`.
///
/// Loopback addresses (e.g. 127.0.0.53 from systemd-resolved) are skipped:
/// forwarding through the local stub could interfere with nftables redirect
/// rules on some configurations. Falls back to 8.8.8.8 if none found.
fn read_upstream_resolver() -> SocketAddr {
    let content = std::fs::read_to_string("/etc/resolv.conf").unwrap_or_default();
    for line in content.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("nameserver") else {
            continue;
        };
        let Some(addr_str) = rest.split_whitespace().next() else {
            continue;
        };
        let Ok(addr) = addr_str.parse::<IpAddr>() else {
            continue;
        };
        if addr.is_loopback() {
            continue;
        }
        return SocketAddr::new(addr, 53);
    }
    let fallback: SocketAddr = "8.8.8.8:53".parse().unwrap();
    tracing::warn!("No non-loopback nameserver in /etc/resolv.conf, using {fallback}");
    fallback
}

#[cfg(test)]
mod tests {
    use super::{is_blocked, make_nxdomain, parse_qname};

    /// Build a minimal DNS A-record query for the given dotted domain name.
    fn make_query(domain: &str) -> Vec<u8> {
        let mut pkt = vec![
            0xAB, 0xCD, // Transaction ID
            0x01, 0x00, // Flags: RD=1
            0x00, 0x01, // QDCOUNT = 1
            0x00, 0x00, // ANCOUNT = 0
            0x00, 0x00, // NSCOUNT = 0
            0x00, 0x00, // ARCOUNT = 0
        ];
        for label in domain.split('.') {
            pkt.push(label.len() as u8);
            pkt.extend_from_slice(label.as_bytes());
        }
        pkt.push(0x00); // end of QNAME
        pkt.extend_from_slice(&[0x00, 0x01]); // QTYPE = A
        pkt.extend_from_slice(&[0x00, 0x01]); // QCLASS = IN
        pkt
    }

    // --- parse_qname ---

    #[test]
    fn parse_qname_single_label() {
        let pkt = make_query("localhost");
        assert_eq!(parse_qname(&pkt), Some("localhost".to_string()));
    }

    #[test]
    fn parse_qname_two_labels() {
        let pkt = make_query("example.com");
        assert_eq!(parse_qname(&pkt), Some("example.com".to_string()));
    }

    #[test]
    fn parse_qname_subdomain() {
        let pkt = make_query("www.youtube.com");
        assert_eq!(parse_qname(&pkt), Some("www.youtube.com".to_string()));
    }

    #[test]
    fn parse_qname_uppercased_lowercased() {
        // make_query encodes label bytes as-is, so uppercase ASCII is preserved
        // in the wire format — parse_qname must lowercase them.
        let pkt = make_query("Example.COM");
        assert_eq!(parse_qname(&pkt), Some("example.com".to_string()));
    }

    #[test]
    fn parse_qname_too_short_returns_none() {
        assert_eq!(parse_qname(&[0u8; 5]), None);
    }

    #[test]
    fn parse_qname_truncated_label_returns_none() {
        let mut pkt = make_query("example.com");
        // Corrupt: set the first label length to 99 (longer than the packet)
        pkt[12] = 99;
        assert_eq!(parse_qname(&pkt), None);
    }

    // --- make_nxdomain ---

    #[test]
    fn make_nxdomain_preserves_transaction_id() {
        let query = make_query("example.com");
        let resp = make_nxdomain(&query);
        assert_eq!(&resp[0..2], &[0xAB, 0xCD]);
    }

    #[test]
    fn make_nxdomain_sets_qr_bit() {
        let query = make_query("example.com");
        let resp = make_nxdomain(&query);
        assert!(resp[2] & 0x80 != 0, "QR bit must be set");
    }

    #[test]
    fn make_nxdomain_sets_rcode_3() {
        let query = make_query("example.com");
        let resp = make_nxdomain(&query);
        assert_eq!(resp[3] & 0x0F, 3, "RCODE must be 3 (NXDOMAIN)");
    }

    #[test]
    fn make_nxdomain_zeroes_counts() {
        let query = make_query("example.com");
        let resp = make_nxdomain(&query);
        assert_eq!(&resp[6..12], &[0, 0, 0, 0, 0, 0], "answer/authority/additional must be zero");
    }

    #[test]
    fn make_nxdomain_preserves_question_section() {
        let query = make_query("example.com");
        let resp = make_nxdomain(&query);
        // Question section starts at byte 12; must be unchanged.
        assert_eq!(&resp[12..], &query[12..]);
    }

    // --- is_blocked ---

    #[test]
    fn is_blocked_exact_match() {
        assert!(is_blocked("youtube.com", &["youtube.com".to_string()]));
    }

    #[test]
    fn is_blocked_subdomain() {
        assert!(is_blocked("www.youtube.com", &["youtube.com".to_string()]));
        assert!(is_blocked("a.b.youtube.com", &["youtube.com".to_string()]));
    }

    #[test]
    fn is_blocked_sibling_domain_not_matched() {
        // "eviltube.com" must not match "youtube.com"
        assert!(!is_blocked("eviltube.com", &["youtube.com".to_string()]));
    }

    #[test]
    fn is_blocked_unrelated_domain() {
        assert!(!is_blocked("google.com", &["youtube.com".to_string()]));
    }

    #[test]
    fn is_blocked_trailing_dot_normalized() {
        assert!(is_blocked("youtube.com.", &["youtube.com".to_string()]));
        assert!(is_blocked("www.youtube.com.", &["youtube.com.".to_string()]));
    }

    #[test]
    fn is_blocked_empty_list() {
        assert!(!is_blocked("youtube.com", &[]));
    }

    #[test]
    fn is_blocked_multiple_domains_in_list() {
        let list = vec!["youtube.com".to_string(), "tiktok.com".to_string()];
        assert!(is_blocked("www.tiktok.com", &list));
        assert!(!is_blocked("google.com", &list));
    }
}
