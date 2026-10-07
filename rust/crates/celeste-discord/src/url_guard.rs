//! The public-URL fetch guard, shared by every feature that downloads
//! something a user supplied: the animated-emoji tools and the watched-group
//! import.
//!
//! Port of `HttpURLInputStream.get(url, PUBLIC_ONLY)` in Java — a fetch that
//! refuses anything but HTTP(S), refuses embedded credentials, resolves the
//! host and refuses private, loopback, link-local, carrier-grade-NAT,
//! documentation and other special addresses, re-validates on every
//! redirect, caps the number of redirects, and streams with a hard byte
//! limit so a hostile response can't exhaust memory.
//!
//! This lived inside the animated-emoji command; it is shared here because
//! the second caller (importing watched groups from a URL) needs exactly the
//! same protections, and a second copy would be one to forget to fix.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

pub async fn download_public(mut url: reqwest::Url, limit: usize) -> Result<Vec<u8>, String> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(celeste_core::identity::user_agent())
        .build()
        .map_err(|err| err.to_string())?;
    for _ in 0..=5 {
        validate_public_url(&url).await?;
        let mut response = client
            .get(url.clone())
            .header(reqwest::header::ACCEPT, "image/gif,text/html;q=0.5")
            .send()
            .await
            .map_err(|err| err.to_string())?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| "redirect response omitted Location".to_string())?;
            url = url.join(location).map_err(|err| err.to_string())?;
            continue;
        }
        if !response.status().is_success() {
            return Err(format!("HTTP {}", response.status()));
        }
        if response
            .content_length()
            .is_some_and(|length| length > limit as u64)
        {
            return Err("GIF exceeds maximum allowed size".into());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|err| err.to_string())? {
            if bytes.len().saturating_add(chunk.len()) > limit {
                return Err("GIF exceeds maximum allowed size".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        return Ok(bytes);
    }
    Err("too many redirects".into())
}

pub async fn validate_public_url(url: &reqwest::Url) -> Result<(), String> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err("only HTTP(S) URLs are accepted".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("URLs containing credentials are not accepted".into());
    }
    let host = url
        .host_str()
        .ok_or_else(|| "URL has no host".to_string())?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| "URL has no usable port".to_string())?;
    let addresses: Vec<IpAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|err| format!("could not resolve host: {err}"))?
        .map(|address| address.ip())
        .collect();
    if addresses.is_empty() || addresses.iter().any(|address| !is_public_ip(*address)) {
        return Err("URL resolves to a non-public address".into());
    }
    Ok(())
}

pub fn is_public_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => is_public_v4(ip),
        IpAddr::V6(ip) => is_public_v6(ip),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_broadcast()
        || ip.is_unspecified()
        || ip.is_documentation()
        || octets[0] == 0
        || (octets[0] == 100 && (64..=127).contains(&octets[1]))
        || (octets[0] == 198 && (octets[1] == 18 || octets[1] == 19))
        || octets[0] >= 240)
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    let segments = ip.segments();
    !(ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || (segments[0] & 0xfe00) == 0xfc00
        || (segments[0] & 0xffc0) == 0xfe80
        || (segments[0] == 0x2001 && segments[1] == 0x0db8))
}


/// Convenience for callers that want text (the CSV/JSON watched-group
/// import), with the same guarantees as [`download_public`].
pub async fn download_public_text(url: &str, limit: usize) -> Result<String, String> {
    let parsed = reqwest::Url::parse(url).map_err(|err| format!("not a valid URL: {err}"))?;
    let bytes = download_public(parsed, limit).await?;
    String::from_utf8(bytes).map_err(|_| "response was not valid UTF-8 text".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_private_and_special_addresses() {
        for address in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.1.1",
            "100.64.0.1",
            "::1",
            "fc00::1",
            "fe80::1",
        ] {
            assert!(!is_public_ip(address.parse().unwrap()), "{address}");
        }
        assert!(is_public_ip("1.1.1.1".parse().unwrap()));
        assert!(is_public_ip("2606:4700:4700::1111".parse().unwrap()));
    }
}
