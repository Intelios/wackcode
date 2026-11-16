//! Which addresses the web tools may reach: public unicast only. Page content can steer the model,
//! so without this a prompt injection could aim `web_fetch` at the user's router, a local dev
//! server or a cloud metadata endpoint.
//!
//! The check runs on the addresses the socket connects to, never on a host name alone:
//! [`PublicOnly`] is the HTTP client's DNS resolver, so it refuses the whole host if any address it
//! resolves to is private and hands the connection exactly the addresses it checked. That leaves no
//! gap between checking and connecting for DNS rebinding to use. A URL whose host is an IP literal
//! skips DNS, so [`check_url`] judges it before the request. Every redirect hop repeats both.
//!
//! Ported from version 2's `address.ts`, the stricter of its two lists.

use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::Arc,
};

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use url::{Host, Url};

/// Which addresses a client may connect to. Production always uses [`Policy::public`]; tests
/// widen it to reach a server on loopback.
#[derive(Clone)]
pub struct Policy {
    allowed: Arc<dyn Fn(IpAddr) -> bool + Send + Sync>,
}

impl Policy {
    pub fn public() -> Self {
        Self { allowed: Arc::new(|address| !is_blocked_address(address)) }
    }

    #[cfg(test)]
    pub fn loopback_for_tests() -> Self {
        Self { allowed: Arc::new(|address| address.is_loopback()) }
    }

    pub fn allows(&self, address: IpAddr) -> bool {
        (self.allowed)(address)
    }
}

/// The sentence for a refused host, safe to show the model and the user.
pub fn private_message(host: &str) -> String {
    format!("{host} is a local or private network address. web_fetch only reads public web pages.")
}

/// Validates a URL the model or a redirect supplied: http or https, no credentials, no local
/// name, and an IP literal only when it is public. Hosts that need DNS are judged at connect time.
pub fn check_url(raw: &str, policy: &Policy) -> Result<Url, String> {
    let url = Url::parse(raw.trim()).map_err(|_| format!("Not a valid URL: {}", raw.trim()))?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(format!("Only http and https URLs can be fetched, not {}.", url.scheme()));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("URLs with embedded credentials are not fetched.".into());
    }
    match url.host() {
        None => return Err(format!("Not a valid URL: {}", raw.trim())),
        Some(Host::Domain(name)) if is_blocked_hostname(name) => return Err(private_message(name)),
        Some(Host::Ipv4(address)) if !policy.allows(address.into()) => return Err(private_message(&address.to_string())),
        Some(Host::Ipv6(address)) if !policy.allows(address.into()) => return Err(private_message(&address.to_string())),
        Some(_) => {}
    }
    Ok(url)
}

/// [`check_url`] plus the DNS check, without fetching: for a URL handed to someone else to read (a
/// search provider's fetch endpoint), so a private address never leaves the Mac.
pub async fn check_public_url(raw: &str, policy: &Policy) -> Result<Url, String> {
    let url = check_url(raw, policy)?;
    if let Some(Host::Domain(name)) = url.host() {
        resolve_checked(name, policy).await.map_err(|error| error.0)?;
    }
    Ok(url)
}

/// The HTTP client's resolver: every address of a host, all of them public, or an error.
pub struct PublicOnly(pub Policy);

impl Resolve for PublicOnly {
    fn resolve(&self, name: Name) -> Resolving {
        let policy = self.0.clone();
        Box::pin(async move {
            let found = resolve_checked(name.as_str(), &policy).await?;
            Ok(Box::new(found.into_iter()) as Addrs)
        })
    }
}

/// An error from the resolver, carried through reqwest so the fetch can say what happened.
#[derive(Debug)]
pub struct Refused(pub String);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refused {}

async fn resolve_checked(name: &str, policy: &Policy) -> Result<Vec<SocketAddr>, Refused> {
    if is_blocked_hostname(name) {
        return Err(Refused(private_message(name)));
    }
    let found: Vec<SocketAddr> = tokio::net::lookup_host((name, 0))
        .await
        .map_err(|_| Refused(format!("Could not find {name}.")))?
        .collect();
    if found.is_empty() || found.iter().any(|address| !policy.allows(address.ip())) {
        return Err(Refused(private_message(name)));
    }
    Ok(found)
}

/// Names that always mean this machine or the local network, refused before any lookup.
pub fn is_blocked_hostname(hostname: &str) -> bool {
    let name = hostname.trim_end_matches('.').to_ascii_lowercase();
    name == "localhost"
        || name.ends_with(".localhost")
        || name.ends_with(".local")
        || name.ends_with(".internal")
        || name.ends_with(".home.arpa")
}

/// True unless `address` is public unicast.
pub fn is_blocked_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => is_blocked_v4(v4),
        IpAddr::V6(v6) => is_blocked_v6(v6),
    }
}

/// `(first address, prefix length)`.
const BLOCKED_V4: [([u8; 4], u32); 15] = [
    ([0, 0, 0, 0], 8),       // "this" network
    ([10, 0, 0, 0], 8),      // private
    ([100, 64, 0, 0], 10),   // carrier-grade NAT
    ([127, 0, 0, 0], 8),     // loopback
    ([169, 254, 0, 0], 16),  // link-local, including cloud metadata
    ([172, 16, 0, 0], 12),   // private
    ([192, 0, 0, 0], 24),    // IETF protocol assignments
    ([192, 0, 2, 0], 24),    // documentation
    ([192, 88, 99, 0], 24),  // 6to4 relay anycast
    ([192, 168, 0, 0], 16),  // private
    ([198, 18, 0, 0], 15),   // benchmarking
    ([198, 51, 100, 0], 24), // documentation
    ([203, 0, 113, 0], 24),  // documentation
    ([224, 0, 0, 0], 4),     // multicast
    ([240, 0, 0, 0], 4),     // reserved, including broadcast
];

fn is_blocked_v4(address: Ipv4Addr) -> bool {
    let value = u32::from(address);
    BLOCKED_V4.iter().any(|(base, bits)| {
        let mask = u32::MAX << (32 - bits);
        value & mask == u32::from_be_bytes(*base)
    })
}

fn embedded_v4(high: u16, low: u16) -> Ipv4Addr {
    Ipv4Addr::from((u32::from(high) << 16) | u32::from(low))
}

fn is_blocked_v6(address: Ipv6Addr) -> bool {
    let [w0, w1, w2, w3, w4, w5, w6, w7] = address.segments();
    let zero_prefix = w0 == 0 && w1 == 0 && w2 == 0 && w3 == 0 && w4 == 0;
    // IPv4-mapped (::ffff:a.b.c.d): judge the IPv4 address inside. The rest of ::/96 is
    // unspecified, loopback or the deprecated IPv4-compatible form, none of them public.
    if zero_prefix && w5 == 0xffff {
        return is_blocked_v4(embedded_v4(w6, w7));
    }
    if zero_prefix && w5 == 0 {
        return true;
    }
    // NAT64's well-known prefix 64:ff9b::/96 carries an IPv4 address in its last 32 bits.
    if w0 == 0x64 && w1 == 0xff9b && w2 == 0 && w3 == 0 && w4 == 0 && w5 == 0 {
        return is_blocked_v4(embedded_v4(w6, w7));
    }
    // 6to4 (2002::/16) carries an IPv4 address in bits 16 to 47.
    if w0 == 0x2002 {
        return is_blocked_v4(embedded_v4(w1, w2));
    }
    // Only global unicast (2000::/3) is public: that excludes loopback, unique-local, link-local,
    // multicast, discard and local-use NAT64.
    if w0 & 0xe000 != 0x2000 {
        return true;
    }
    // Teredo (2001::/32) tunnels to any IPv4 host; 2001:db8::/32 and 3fff::/20 are documentation.
    if w0 == 0x2001 && (w1 == 0 || w1 == 0xdb8) {
        return true;
    }
    w0 == 0x3fff && w1 < 0x1000
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocked(text: &str) -> bool {
        let bare = text.trim_start_matches('[').trim_end_matches(']');
        let bare = bare.split('%').next().unwrap_or(bare);
        bare.parse::<IpAddr>().map(is_blocked_address).unwrap_or(true)
    }

    #[test]
    fn blocks_everything_that_is_not_public_unicast() {
        for address in [
            "127.0.0.1", "127.8.9.10", "10.1.2.3", "172.16.0.1", "172.31.255.255", "192.168.1.1", "169.254.169.254",
            "100.64.0.1", "0.0.0.0", "224.0.0.1", "255.255.255.255", "198.18.0.1", "192.88.99.1",
            "::", "::1", "fe80::1", "fe80::1%en0", "fc00::1", "fd12:3456::1", "ff02::1",
            "::ffff:127.0.0.1", "::ffff:7f00:1", "::ffff:10.0.0.1", "::127.0.0.1", "64:ff9b::a00:1", "64:ff9b:1::1",
            "2002:c0a8:101::1", "2001:db8::1", "2001::1", "3fff::1", "not an ip", "",
        ] {
            assert!(blocked(address), "{address} should be blocked");
        }
    }

    #[test]
    fn allows_public_addresses() {
        for address in [
            "93.184.215.14", "1.1.1.1", "172.32.0.1", "8.8.8.8", "2606:4700::1111", "::ffff:8.8.8.8",
            "64:ff9b::808:808", "2002:808:808::1", "[2606:4700::1111]",
        ] {
            assert!(!blocked(address), "{address} should be allowed");
        }
    }

    #[test]
    fn refuses_local_host_names_before_any_lookup() {
        for name in ["localhost", "LOCALHOST.", "app.localhost", "printer.local", "db.internal", "nas.home.arpa"] {
            assert!(is_blocked_hostname(name), "{name}");
        }
        assert!(!is_blocked_hostname("example.com"));
        assert!(!is_blocked_hostname("localhost.example.com"));
    }

    #[test]
    fn accepts_only_plain_public_http_urls() {
        let policy = Policy::public();
        assert_eq!(check_url("https://example.com/a?b", &policy).unwrap().as_str(), "https://example.com/a?b");
        assert!(check_url("file:///etc/passwd", &policy).unwrap_err().contains("Only http and https"));
        assert!(check_url("ftp://example.com", &policy).unwrap_err().contains("Only http and https"));
        assert!(check_url("https://user:pass@example.com", &policy).unwrap_err().contains("credentials"));
        assert!(check_url("not a url", &policy).unwrap_err().contains("Not a valid URL"));
        assert!(check_url("http://localhost:3000", &policy).unwrap_err().contains("private network"));
        assert!(check_url("http://10.0.0.1/admin", &policy).unwrap_err().contains("private network"));
        assert!(check_url("http://[::1]/", &policy).unwrap_err().contains("private network"));
        assert!(check_url("http://[::ffff:169.254.169.254]/", &policy).unwrap_err().contains("private network"));
        assert!(check_url("https://93.184.215.14/x", &policy).is_ok());
    }

    #[tokio::test]
    async fn the_resolver_refuses_a_name_that_points_inside() {
        let policy = Policy::public();
        assert!(check_public_url("http://localhost/", &policy).await.is_err());
        // A name that resolves to loopback is caught by address, after the lookup.
        let refused = resolve_checked("127.0.0.1", &policy).await.unwrap_err();
        assert!(refused.0.contains("private network"), "{refused}");
        assert!(resolve_checked("127.0.0.1", &Policy::loopback_for_tests()).await.is_ok());
    }
}
