use crate::engine::StreamEngine;
use crate::llm::LlmCurator;
use crate::models::Track;
use crate::sources::SourceRegistry;
use crate::store::Store;
use crate::taste::TasteEngine;
use axum::http::{header, HeaderMap};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct NowPlayingEntry {
    /// `None` acts as a short-lived tombstone for a client that just paused or closed,
    /// so an in-flight older `play` request from the same client cannot resurrect it.
    pub track: Option<Track>,
    pub started_at: i64,
    pub last_seen_at: i64,
    pub client_id: Option<String>,
    pub seq: u64,
}

/// (user_id -> active playback presence)
pub type NowPlayingMap = Arc<Mutex<HashMap<i64, NowPlayingEntry>>>;

/// In-flight background "New Releases" refresh handles keyed by user ID.
pub type NewReleasesInflight = Arc<Mutex<HashMap<i64, tokio::sync::watch::Receiver<bool>>>>;

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub sources: SourceRegistry,
    pub engine: Arc<StreamEngine>,
    pub taste: Arc<TasteEngine>,
    pub llm: Option<Arc<LlmCurator>>,
    pub now_playing: NowPlayingMap,
    pub new_releases_inflight: NewReleasesInflight,
}

/// Internal header carrying the TCP peer address, stamped by `main::stamp_peer_ip`
/// (any copy sent by a client is stripped first).
pub const PEER_IP_HEADER: &str = "x-beatlab-peer-ip";

fn peer_ip(headers: &HeaderMap) -> Option<std::net::IpAddr> {
    headers.get(PEER_IP_HEADER)?.to_str().ok()?.parse().ok()
}

/// Whether forwarded-IP headers from this peer can be believed. A peer on loopback or a
/// private network is assumed to be a reverse proxy (nginx, Caddy, Docker, Fly's 6PN);
/// a public peer is talking to us directly, so its `X-Forwarded-For` is attacker-controlled.
/// Override with `BEATLAB_TRUST_PROXY=always|never`.
fn trust_forwarded_headers(peer: Option<std::net::IpAddr>) -> bool {
    use std::net::IpAddr;
    static MODE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let mode = MODE.get_or_init(|| {
        std::env::var("BEATLAB_TRUST_PROXY")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
    });
    match mode.as_str() {
        "always" | "true" | "1" => return true,
        "never" | "false" | "0" => return false,
        _ => {}
    }
    let Some(peer) = peer else {
        // No peer recorded (tests, or a server started without connect info): legacy behaviour.
        return true;
    };
    match peer {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return v4.is_loopback() || v4.is_private() || v4.is_link_local();
            }
            let first = v6.segments()[0];
            v6.is_loopback() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
    }
}

fn header_ip(headers: &HeaderMap, names: &[&str]) -> Option<String> {
    for name in names {
        if let Some(v) = headers.get(*name).and_then(|v| v.to_str().ok()) {
            let first = v.split(',').next().unwrap_or("").trim();
            if !first.is_empty() {
                return Some(first.chars().take(64).collect());
            }
        }
    }
    None
}

/// Best-effort client IP. Behind a trusted reverse proxy this comes from the proxy's
/// forwarding headers; otherwise it is the socket peer itself.
pub fn client_ip(headers: &HeaderMap) -> String {
    let peer = peer_ip(headers);
    if trust_forwarded_headers(peer) {
        if let Some(ip) = header_ip(
            headers,
            &["x-forwarded-for", "x-real-ip", "cf-connecting-ip", "fly-client-ip"],
        ) {
            return ip;
        }
    }
    peer.map(|p| p.to_string()).unwrap_or_default()
}

/// Client IP used for rate limiting. Behind a trusted proxy, prefers the header the
/// hosting proxy overwrites (Fly.io's `Fly-Client-IP`), which a client cannot forge the
/// way it can prepend entries to `X-Forwarded-For`. Directly-connected clients are keyed
/// by their socket address, so spoofed headers can't dodge the limiter.
pub fn rate_limit_ip(headers: &HeaderMap) -> String {
    if trust_forwarded_headers(peer_ip(headers)) {
        if let Some(ip) = header_ip(headers, &["fly-client-ip"]) {
            return ip;
        }
    }
    client_ip(headers)
}

pub fn user_agent(headers: &HeaderMap) -> String {
    headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .chars()
        .take(256)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwarded_headers_only_trusted_from_proxy_peers() {
        let mk = |peer: Option<&str>, xff: &str| {
            let mut h = HeaderMap::new();
            if let Some(p) = peer {
                h.insert(PEER_IP_HEADER, p.parse().unwrap());
            }
            h.insert("x-forwarded-for", xff.parse().unwrap());
            h.insert("fly-client-ip", "203.0.113.9".parse().unwrap());
            h
        };
        // Public peer talking directly: spoofed headers are ignored.
        let direct = mk(Some("198.51.100.7"), "1.2.3.4");
        assert_eq!(client_ip(&direct), "198.51.100.7");
        assert_eq!(rate_limit_ip(&direct), "198.51.100.7");
        // Reverse proxy on loopback / private / IPv6 ULA: headers are believed.
        for proxy in [
            "127.0.0.1",
            "10.0.0.2",
            "172.17.0.1",
            "fdaa:0:1::3",
            "::ffff:192.168.1.5",
        ] {
            let h = mk(Some(proxy), "1.2.3.4, 10.0.0.1");
            assert_eq!(client_ip(&h), "1.2.3.4", "proxy {proxy}");
            assert_eq!(rate_limit_ip(&h), "203.0.113.9", "proxy {proxy}");
        }
        // No proxy headers from a LAN client: fall back to the peer address instead of "".
        let mut lan = HeaderMap::new();
        lan.insert(PEER_IP_HEADER, "192.168.1.20".parse().unwrap());
        assert_eq!(rate_limit_ip(&lan), "192.168.1.20");
    }
}
