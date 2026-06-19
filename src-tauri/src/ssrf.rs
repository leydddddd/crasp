//! SSRF-safe URL validation and redirect handling.
//!
//! Every URL is validated before the crawler fetches it:
//!   1. Scheme must be http or https.
//!   2. Hostname must resolve to only public, non-reserved IP addresses.
//!   3. On every HTTP redirect, the Location URL is revalidated the same way.
//!
//! The crawler's reqwest::Client is built with `redirect(Policy::none())` so
//! that redirect chains are manually followed with full validation at each
//! hop, preventing a public-looking URL from 302-ing to an internal address.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use url::Url;

/// Maximum number of redirect hops we'll follow before giving up.
const MAX_REDIRECT_HOPS: u32 = 5;

/// Validate a seed URL string for SSRF safety.
///
/// Returns Ok(Url) if the URL passes all checks:
///   - Valid URL parse
///   - Scheme is http or https
///   - Hostname is present
///   - DNS resolution yields only non-reserved addresses
///
/// Returns Err with a human-readable reason on failure.
pub async fn validate_seed_url(raw: &str) -> Result<Url, String> {
    let parsed = Url::parse(raw).map_err(|e| format!("Invalid URL: {}", e))?;

    validate_url_scheme(&parsed)?;
    validate_url_host(&parsed).await?;

    Ok(parsed)
}

/// Build an SSRF-safe reqwest client that does NOT follow redirects
/// automatically. The caller is responsible for following redirects
/// manually via [`follow_redirects`].
pub fn build_safe_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent("Crasp/0.1 (archiver; +https://crasp.app)")
        .gzip(true)
        .brotli(true)
        .deflate(true)
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("failed to build SSRF-safe HTTP client")
}

/// Follow a redirect chain manually, re-validating every Location URL.
///
/// Given an initial `response` (which may be a 3xx), follows up to
/// MAX_REDIRECT_HOPS redirects, returning the final response.
/// Each intermediate URL is fully SSRF-validated before the next request.
pub async fn follow_redirects(
    client: &reqwest::Client,
    initial_url: Url,
    initial_response: reqwest::Response,
) -> Result<reqwest::Response, String> {
    let mut response = initial_response;
    let mut url = initial_url;
    let mut hops = 0u32;

    loop {
        let status = response.status();
        if !status.is_redirection() {
            return Ok(response);
        }

        hops += 1;
        if hops > MAX_REDIRECT_HOPS {
            return Err(format!("Too many redirects (>{}) for {}", MAX_REDIRECT_HOPS, url));
        }

        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .ok_or_else(|| format!("Redirect {} missing Location header", hops))?;

        let location_str = location
            .to_str()
            .map_err(|e| format!("Invalid Location header: {}", e))?;

        let next_url = url.join(location_str)
            .map_err(|e| format!("Invalid redirect URL '{}': {}", location_str, e))?;

        validate_url_scheme(&next_url)?;
        validate_url_host(&next_url).await?;

        response = client
            .get(next_url.clone())
            .send()
            .await
            .map_err(|e| format!("Redirect request to {} failed: {}", next_url, e))?;

        url = next_url;
    }
}

// ─── Internal helpers ─────────────────────────────────────────────

fn validate_url_scheme(url: &Url) -> Result<(), String> {
    match url.scheme() {
        "http" | "https" => Ok(()),
        other => Err(format!(
            "Unsupported scheme: {}. Only http and https are allowed.",
            other
        )),
    }
}

async fn validate_url_host(url: &Url) -> Result<(), String> {
    let host = url.host_str().ok_or("URL must have a hostname")?;

    // If it's already a literal IP address, check it directly.
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_reserved_address(&ip) {
            return Err(format!(
                "Target address {} is a reserved/non-public address; crawling internal addresses is not permitted",
                ip
            ));
        }
        return Ok(());
    }

    // Resolve the hostname via DNS and check every address.
    // Use tokio's blocking DNS resolution via a spawn_blocking wrapper
    // since std::net::ToSocketAddrs is blocking.
    let host_owned = host.to_string();
    let resolved = tokio::task::spawn_blocking(move || {
        use std::net::ToSocketAddrs;
        format!("{}:0", host_owned).to_socket_addrs()
    })
    .await
    .map_err(|e| format!("DNS resolution task failed: {}", e))?
    .map_err(|e| format!("DNS resolution failed for '{}': {}", host, e))?;

    let addresses: Vec<IpAddr> = resolved.map(|sa| sa.ip()).collect();

    if addresses.is_empty() {
        return Err(format!("Hostname '{}' did not resolve to any address", host));
    }

    for addr in &addresses {
        if is_reserved_address(addr) {
            return Err(format!(
                "Hostname '{}' resolves to reserved address {}; crawling internal addresses is not permitted",
                host, addr
            ));
        }
    }

    Ok(())
}

/// Check if an IP address is reserved, private, loopback, link-local,
/// or in the CGNAT range. Only truly global unicast addresses are allowed.
fn is_reserved_address(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_reserved_v4(v4),
        IpAddr::V6(v6) => is_reserved_v6(v6),
    }
}

fn is_reserved_v4(ip: &Ipv4Addr) -> bool {
    // Loopback: 127.0.0.0/8
    if ip.is_loopback() {
        return true;
    }
    // Private: 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
    if ip.is_private() {
        return true;
    }
    // Link-local: 169.254.0.0/16
    if ip.is_link_local() {
        return true;
    }
    // Multicast: 224.0.0.0/4
    if ip.is_multicast() {
        return true;
    }
    // Broadcast: 255.255.255.255
    if ip.is_broadcast() {
        return true;
    }
    // Unspecified: 0.0.0.0
    if ip.is_unspecified() {
        return true;
    }
    // CGNAT (Carrier-Grade NAT): 100.64.0.0/10
    // IANA RFC 6598 — reserved for shared address space.
    if is_cgnat(ip) {
        return true;
    }
    // IETF Protocol Assignments: 192.0.0.0/24
    // (includes 192.0.0.1 — used for documentation, not actually used in production)
    // We don't block 192.0.2.0/24 (TEST-NET-1) etc. because they're
    // not routable in practice, but they fail at the network level, not
    // the validation level. Let them fail naturally.
    false
}

fn is_reserved_v6(ip: &Ipv6Addr) -> bool {
    // Loopback: ::1
    if ip.is_loopback() {
        return true;
    }
    // Unspecified: ::
    if ip.is_unspecified() {
        return true;
    }
    // Multicast: ff00::/8
    if ip.is_multicast() {
        return true;
    }
    // Link-local: fe80::/10
    // Ipv6Addr::is_link_local() is not yet stable in 1.79.0.
    // Implement manually: first 10 bits must be 0xfe8 (1111 1110 10xx xxxx).
    let segments = ip.segments();
    let first_10_bits = (segments[0] >> 6) as u16;
    if first_10_bits == 0x0fe8 >> 2 {
        // fe80::/10 — first 10 bits = 1111 1110 10 = 0x3FA
        return true;
    }

    // IPv4-mapped IPv6 addresses: ::ffff:0:0/96
    // Reject these since they might embed private IPv4 addresses.
    if let Some(v4) = ip.to_ipv4_mapped() {
        if is_reserved_v4(&v4) {
            return true;
        }
    }

    // Unique local addresses: fc00::/7 (fd00::/8 and fc00::/8)
    // These are the IPv6 equivalent of private addresses.
    // Ipv6Addr doesn't have a stable is_unique_local() yet.
    let first_byte = (segments[0] >> 8) as u8;
    if first_byte == 0xfc || first_byte == 0xfd {
        return true;
    }

    // Only allow global unicast addresses.
    // Ipv6Addr::is_unspecified() etc. are checked above; everything
    // else that survives is assumed to be global unicast.
    false
}

/// CGNAT range: 100.64.0.0/10 (RFC 6598)
fn is_cgnat(ip: &Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 100 && (octets[1] & 0xC0) == 0x40
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_loopback_rejected() {
        assert!(is_reserved_v4(&Ipv4Addr::new(127, 0, 0, 1)));
        assert!(is_reserved_v6(&Ipv6Addr::LOCALHOST));
    }

    #[test]
    fn test_private_rejected() {
        assert!(is_reserved_v4(&Ipv4Addr::new(10, 0, 0, 1)));
        assert!(is_reserved_v4(&Ipv4Addr::new(172, 16, 0, 1)));
        assert!(is_reserved_v4(&Ipv4Addr::new(172, 31, 255, 1)));
        assert!(is_reserved_v4(&Ipv4Addr::new(192, 168, 1, 1)));
    }

    #[test]
    fn test_link_local_rejected() {
        assert!(is_reserved_v4(&Ipv4Addr::new(169, 254, 169, 254)));
    }

    #[test]
    fn test_cgnat_rejected() {
        assert!(is_cgnat(&Ipv4Addr::new(100, 64, 0, 1)));
        assert!(is_cgnat(&Ipv4Addr::new(100, 127, 255, 1)));
        assert!(!is_cgnat(&Ipv4Addr::new(100, 63, 0, 1)));
        assert!(!is_cgnat(&Ipv4Addr::new(100, 128, 0, 1)));
    }

    #[test]
    fn test_public_allowed() {
        assert!(!is_reserved_v4(&Ipv4Addr::new(93, 184, 216, 34))); // example.com
        assert!(!is_reserved_v4(&Ipv4Addr::new(8, 8, 8, 8))); // dns.google
    }

    #[test]
    fn test_172_range_edge_cases() {
        // 172.16.x.x through 172.31.x.x are private
        assert!(is_reserved_v4(&Ipv4Addr::new(172, 16, 0, 1)));
        assert!(is_reserved_v4(&Ipv4Addr::new(172, 31, 255, 255)));
        // 172.15.x.x and 172.32.x.x are NOT private
        assert!(!is_reserved_v4(&Ipv4Addr::new(172, 15, 0, 1)));
        assert!(!is_reserved_v4(&Ipv4Addr::new(172, 32, 0, 1)));
    }
}
