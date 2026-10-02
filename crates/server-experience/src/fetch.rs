//! Host-only HTTPS fetches with pinned public DNS answers and no redirects.

use anyhow::{Result, ensure};
use reqwest::{
    Client, StatusCode,
    header::{CONTENT_ENCODING, CONTENT_RANGE, RANGE},
};
use std::{
    collections::BTreeSet,
    net::{IpAddr, SocketAddr},
    time::Duration,
};
use url::Url;

/// Validates an exact URL against the approved origin set, without doing I/O.
pub fn approved_url(text: &str, origins: &BTreeSet<String>) -> Result<Url> {
    ensure!(text.len() <= crate::policy::MAX_URL_BYTES, "URL too long");
    let url = Url::parse(text)?;
    ensure!(
        url.scheme() == "https" && url.host_str().is_some(),
        "HTTPS required"
    );
    ensure!(
        url.username().is_empty() && url.password().is_none() && url.fragment().is_none(),
        "URL credentials or fragment denied"
    );
    ensure!(
        origins.contains(&url.origin().ascii_serialization()),
        "origin not approved"
    );
    if let Some(url::Host::Ipv4(ip)) = url.host() {
        ensure!(public_address(IpAddr::V4(ip)), "private address denied");
    }
    if let Some(url::Host::Ipv6(ip)) = url.host() {
        ensure!(public_address(IpAddr::V6(ip)), "private address denied");
    }
    Ok(url)
}

/// Conservatively denies special-use networks, including mapped IPv4 addresses.
pub fn public_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_broadcast()
                && !ip.is_documentation()
                && a != 0
                && a < 224
                && !(a == 100 && (64..=127).contains(&b))
                && !(a == 192 && b == 0)
                && !(a == 198 && (b == 18 || b == 19))
                && !(a == 192 && b == 88 && c == 99)
        }
        IpAddr::V6(ip) => {
            // Only global unicast; exclude transition, documentation and special registries.
            let s = ip.segments();
            s[0] & 0xe000 == 0x2000
                && s[0] != 0x2002
                && !(s[0] == 0x2001 && (s[1] < 0x0200 || s[1] == 0x0db8))
                && !(s[0] == 0x3fff && s[1] < 0x1000)
        }
    }
}

/// Fetches one immutable object or exact byte range; dropping the future cancels it.
pub async fn fetch(
    text: &str,
    origins: &BTreeSet<String>,
    expected_bytes: usize,
    range: Option<(u64, u64, u64)>,
) -> Result<Vec<u8>> {
    ensure!(
        expected_bytes > 0 && expected_bytes <= crate::policy::MAX_BUNDLE_BYTES,
        "fetch size denied"
    );
    let url = approved_url(text, origins)?;
    let host = url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("missing host"))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| anyhow::anyhow!("missing port"))?;
    let addresses: Vec<SocketAddr> = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::lookup_host((host.trim_matches(['[', ']']), port)),
    )
    .await??
    .collect();
    ensure!(
        !addresses.is_empty() && addresses.iter().all(|a| public_address(a.ip())),
        "DNS address denied"
    );
    let client = Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30))
        .resolve_to_addrs(host, &addresses)
        .build()?;
    let mut request = client
        .get(url.clone())
        .header("accept-encoding", "identity");
    if let Some((start, end, total)) = range {
        ensure!(
            start <= end && end < total && end - start + 1 == expected_bytes as u64,
            "invalid range"
        );
        request = request.header(RANGE, format!("bytes={start}-{end}"));
    }
    let mut response = request.send().await?;
    ensure!(
        response
            .remote_addr()
            .is_some_and(|a| addresses.contains(&a)),
        "connected address changed"
    );
    ensure!(
        response
            .headers()
            .get(CONTENT_ENCODING)
            .is_none_or(|v| v == "identity"),
        "encoded body denied"
    );
    if let Some((start, end, total)) = range {
        ensure!(
            response.status() == StatusCode::PARTIAL_CONTENT,
            "range unsupported"
        );
        let expected = format!("bytes {start}-{end}/{total}");
        ensure!(
            response
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                == Some(expected.as_str()),
            "range mismatch"
        );
    } else {
        ensure!(response.status() == StatusCode::OK, "download failed");
    }
    ensure!(
        response
            .content_length()
            .is_none_or(|size| size == expected_bytes as u64),
        "download length mismatch"
    );
    let mut bytes = Vec::with_capacity(expected_bytes);
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            chunk.len() <= expected_bytes - bytes.len(),
            "download exceeds limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    ensure!(bytes.len() == expected_bytes, "truncated download");
    Ok(bytes)
}
