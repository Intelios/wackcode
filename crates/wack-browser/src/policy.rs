//! Which addresses the agent may open in the browser: the public web and this Mac.
//!
//! Page content can steer the model, so without a check a prompt injection could send the
//! signed-in browser to the user's router, a NAS or a cloud metadata endpoint. The user may type
//! any address themselves; this applies only to navigations the agent asks for, and to the
//! redirects and script navigations that follow while the agent is driving a tab.
//!
//! The rule (docs/security.md, "Network"):
//! - http and https only (plus `about:blank`), never a URL with credentials in it;
//! - public unicast addresses, plus loopback and `*.localhost` so the agent can preview the
//!   project's dev servers;
//! - every other private, link-local, carrier-grade NAT, multicast or reserved address is refused,
//!   judged on the addresses DNS returns, not on the name.
//!
//! It reuses `wack_web::guard`'s classification, which refuses loopback; this module carves
//! loopback back out and nothing else.

use std::net::IpAddr;

use url::{Host, Url};
use wack_web::guard::{is_blocked_address, is_blocked_hostname};

/// True when the agent may connect to `address`.
pub fn allows_address(address: IpAddr) -> bool {
    is_loopback(address) || !is_blocked_address(address)
}

fn is_loopback(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => {
            v6.is_loopback() || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())
        }
    }
}

/// `localhost` and `*.localhost` always mean this Mac (RFC 6761), so they need no lookup.
fn is_local_name(name: &str) -> bool {
    let name = name.trim_end_matches('.').to_ascii_lowercase();
    name == "localhost" || name.ends_with(".localhost")
}

fn refused(host: &str) -> String {
    format!(
        "{host} is on a private network. The agent may open public web pages and this Mac (localhost), \
         not other local addresses; the user can open it themselves."
    )
}

/// What a navigation needs before it may go ahead.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Allowed without a lookup (an IP literal, a `localhost` name, `about:blank`).
    Allowed(Url),
    /// Allowed only if every address `host` resolves to is allowed.
    NeedsLookup(Url, String),
}

/// The checks that need no DNS: scheme, credentials, names, IP literals.
pub fn check(raw: &str) -> Result<Verdict, String> {
    let trimmed = raw.trim();
    let url = Url::parse(trimmed).map_err(|_| format!("Not a valid URL: {trimmed}"))?;
    if url.as_str() == "about:blank" {
        return Ok(Verdict::Allowed(url));
    }
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(format!(
            "The agent can open http and https pages, not {} URLs.",
            url.scheme()
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("URLs with a user name or password in them are not opened. Sign in on the page instead.".into());
    }
    match url.host() {
        None => Err(format!("Not a valid URL: {trimmed}")),
        Some(Host::Domain(name)) if is_local_name(name) => Ok(Verdict::Allowed(url.clone())),
        Some(Host::Domain(name)) if is_blocked_hostname(name) => Err(refused(name)),
        Some(Host::Domain(name)) => {
            let name = name.to_string();
            Ok(Verdict::NeedsLookup(url, name))
        }
        Some(Host::Ipv4(address)) if address.is_unspecified() => Err(
            "0.0.0.0 is not an address a browser can open. Use http://localhost with the same port.".into(),
        ),
        Some(Host::Ipv4(address)) if !allows_address(address.into()) => Err(refused(&address.to_string())),
        Some(Host::Ipv6(address)) if !allows_address(address.into()) => Err(refused(&address.to_string())),
        Some(_) => Ok(Verdict::Allowed(url)),
    }
}

/// Judges the addresses a host resolved to: all must be allowed, and there must be one.
pub fn judge_addresses(host: &str, addresses: &[IpAddr]) -> Result<(), String> {
    if addresses.is_empty() {
        return Err(format!("Could not find {host}."));
    }
    if addresses.iter().all(|address| allows_address(*address)) {
        Ok(())
    } else {
        Err(refused(host))
    }
}

/// The whole check for a URL the agent asked to open, DNS included.
pub async fn check_resolved(raw: &str) -> Result<Url, String> {
    match check(raw)? {
        Verdict::Allowed(url) => Ok(url),
        Verdict::NeedsLookup(url, host) => {
            let addresses: Vec<IpAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await
                .map_err(|_| format!("Could not find {host}."))?
                .map(|socket| socket.ip())
                .collect();
            judge_addresses(&host, &addresses)?;
            Ok(url)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowed(raw: &str) -> bool {
        matches!(check(raw), Ok(Verdict::Allowed(_)))
    }

    #[test]
    fn this_mac_is_allowed() {
        assert!(allowed("http://localhost:5173/"));
        assert!(allowed("http://app.localhost:3000/login"));
        assert!(allowed("http://127.0.0.1:8080"));
        assert!(allowed("http://127.1.2.3"));
        assert!(allowed("http://[::1]:4000/"));
        assert!(allowed("about:blank"));
        assert!(allows_address("::ffff:127.0.0.1".parse().unwrap()));
    }

    #[test]
    fn the_local_network_is_refused() {
        for raw in [
            "http://192.168.1.1/",
            "http://10.0.0.2:8080",
            "http://172.16.4.4",
            "http://169.254.169.254/latest/meta-data",
            "http://100.64.0.1",
            "http://[fe80::1]/",
            "http://[fc00::1]/",
            "http://printer.local/",
            "http://nas.home.arpa/",
            "http://metadata.internal/",
        ] {
            let error = check(raw).expect_err(raw);
            assert!(error.contains("private network"), "{raw}: {error}");
        }
        let error = check("http://0.0.0.0:3000").unwrap_err();
        assert!(error.contains("localhost"), "{error}");
    }

    #[test]
    fn schemes_and_credentials() {
        assert!(
            check("file:///etc/passwd")
                .unwrap_err()
                .contains("not file URLs")
        );
        assert!(
            check("chrome://settings")
                .unwrap_err()
                .contains("not chrome URLs")
        );
        assert!(
            check("javascript:alert(1)")
                .unwrap_err()
                .contains("not javascript URLs")
        );
        assert!(check("data:text/html,hi").is_err());
        assert!(
            check("https://user:pw@example.com/")
                .unwrap_err()
                .contains("password")
        );
        assert!(
            check("not a url")
                .unwrap_err()
                .starts_with("Not a valid URL")
        );
    }

    #[test]
    fn names_need_a_lookup_and_every_address_must_pass() {
        assert!(
            matches!(check("https://example.com/a"), Ok(Verdict::NeedsLookup(_, host)) if host == "example.com")
        );
        let public: IpAddr = "93.184.216.34".parse().unwrap();
        let private: IpAddr = "192.168.0.10".parse().unwrap();
        let loopback: IpAddr = "127.0.0.1".parse().unwrap();
        assert!(judge_addresses("example.com", &[public]).is_ok());
        assert!(judge_addresses("dev.test", &[loopback]).is_ok());
        assert!(judge_addresses("rebind.example", &[public, private]).is_err());
        assert!(
            judge_addresses("nowhere.example", &[])
                .unwrap_err()
                .starts_with("Could not find")
        );
    }
}
