use crate::model::{Protocol, ProxyNode};
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine;
use percent_encoding::percent_decode_str;
use std::collections::{HashMap, HashSet};
use url::{Host, Url};

pub fn decode_base64_flexible(input: &str) -> Option<String> {
    let trimmed = input.trim().replace(['\n', '\r'], "");

    let bytes = STANDARD
        .decode(&trimmed)
        .or_else(|_| STANDARD_NO_PAD.decode(&trimmed))
        .or_else(|_| URL_SAFE.decode(&trimmed))
        .or_else(|_| URL_SAFE_NO_PAD.decode(&trimmed))
        .ok()?;

    String::from_utf8(bytes).ok()
}

pub fn parse_link(link: &str, subscription_id: Option<String>) -> Option<ProxyNode> {
    let trimmed = link.trim();
    if trimmed.starts_with("vless://") {
        parse_vless(trimmed, subscription_id)
    } else if trimmed.starts_with("vmess://") {
        parse_vmess(trimmed, subscription_id)
    } else if trimmed.starts_with("trojan://") {
        parse_trojan(trimmed, subscription_id)
    } else if trimmed.starts_with("ss://") {
        parse_shadowsocks(trimmed, subscription_id)
    } else {
        None
    }
}

pub fn parse_vless(link: &str, sub_id: Option<String>) -> Option<ProxyNode> {
    let parsed = Url::parse(link).ok()?;
    let secret = percent_decode(parsed.username());
    let server = url_host(&parsed)?;
    let port = parsed.port()?;
    let name = parsed
        .fragment()
        .map(percent_decode)
        .unwrap_or_else(|| format!("{server}:{port}"));

    let query: HashMap<String, String> = parsed.query_pairs().into_owned().collect();

    let security = query.get("security").cloned().unwrap_or_else(|| "none".to_string());
    let network = query.get("type").cloned().unwrap_or_else(|| "tcp".to_string());
    let sni = query.get("sni").cloned().or_else(|| query.get("peer").cloned());
    let pbk = query.get("pbk").cloned();
    let sid = query.get("sid").cloned();
    let spider_x = query.get("spx").cloned();
    let flow = query.get("flow").cloned();
    let fingerprint = query.get("fp").cloned();
    let path = query.get("path").cloned();
    let host = query.get("host").cloned();
    let service_name = query.get("serviceName").cloned();

    Some(ProxyNode {
        id: stable_id(link),
        name,
        protocol: Protocol::Vless,
        server,
        port,
        secret,
        cipher: None,
        network,
        path,
        host,
        service_name,
        security,
        sni,
        alpn: query.get("alpn").map(|a| split_list(a)),
        fingerprint,
        pbk,
        sid,
        spider_x,
        flow,
        raw_link: link.to_string(),
        subscription_id: sub_id,
        ping_ms: None,
    })
}

pub fn parse_trojan(link: &str, sub_id: Option<String>) -> Option<ProxyNode> {
    let parsed = Url::parse(link).ok()?;
    let secret = percent_decode(parsed.username());
    let server = url_host(&parsed)?;
    let port = parsed.port()?;
    let name = parsed
        .fragment()
        .map(percent_decode)
        .unwrap_or_else(|| format!("{server}:{port}"));

    let query: HashMap<String, String> = parsed.query_pairs().into_owned().collect();

    let security = query.get("security").cloned().unwrap_or_else(|| "tls".to_string());
    let network = query.get("type").cloned().unwrap_or_else(|| "tcp".to_string());
    let sni = query.get("sni").cloned().or_else(|| query.get("peer").cloned());
    let fingerprint = query.get("fp").cloned();
    let path = query.get("path").cloned();
    let host = query.get("host").cloned();

    Some(ProxyNode {
        id: stable_id(link),
        name,
        protocol: Protocol::Trojan,
        server,
        port,
        secret,
        cipher: None,
        network,
        path,
        host,
        service_name: query.get("serviceName").cloned(),
        security,
        sni,
        alpn: query.get("alpn").map(|a| split_list(a)),
        fingerprint,
        pbk: query.get("pbk").cloned(),
        sid: query.get("sid").cloned(),
        spider_x: query.get("spx").cloned(),
        flow: None,
        raw_link: link.to_string(),
        subscription_id: sub_id,
        ping_ms: None,
    })
}

pub fn parse_vmess(link: &str, sub_id: Option<String>) -> Option<ProxyNode> {
    let b64_data = link.trim_start_matches("vmess://");
    let decoded = decode_base64_flexible(b64_data)?;
    let v: serde_json::Value = serde_json::from_str(&decoded).ok()?;

    let str_field = |key: &str| v[key].as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from);

    let server = str_field("add")?;
    // Many panels emit the port as a string ("443") rather than a number.
    let port = v["port"]
        .as_u64()
        .or_else(|| v["port"].as_str()?.trim().parse().ok())
        .and_then(|p| u16::try_from(p).ok())?;
    let secret = str_field("id")?;
    let name = str_field("ps").unwrap_or_else(|| format!("{server}:{port}"));
    let network = str_field("net").unwrap_or_else(|| "tcp".to_string());
    let path = str_field("path");
    let host = str_field("host");
    let security = match v["tls"].as_str().unwrap_or("") {
        "tls" => "tls",
        "reality" => "reality",
        _ => "none",
    }
    .to_string();
    let sni = str_field("sni").or_else(|| host.clone());
    // v2rayN-style links carry the gRPC service name in `path`.
    let service_name = if network == "grpc" { path.clone() } else { None };

    Some(ProxyNode {
        id: stable_id(link),
        name,
        protocol: Protocol::Vmess,
        server,
        port,
        secret,
        cipher: Some("auto".to_string()),
        network,
        path,
        host,
        service_name,
        security,
        sni,
        alpn: str_field("alpn").map(|a| split_list(&a)),
        fingerprint: str_field("fp"),
        pbk: str_field("pbk"),
        sid: str_field("sid"),
        spider_x: str_field("spx"),
        flow: None,
        raw_link: link.to_string(),
        subscription_id: sub_id,
        ping_ms: None,
    })
}

pub fn parse_shadowsocks(link: &str, sub_id: Option<String>) -> Option<ProxyNode> {
    let without_prefix = link.trim_start_matches("ss://");
    let (info_part, fragment) = match without_prefix.split_once('#') {
        Some((left, right)) => (left, Some(percent_decode(right))),
        None => (without_prefix, None),
    };

    let (cipher, secret, server, port) = if let Some((cred_part, server_part)) = info_part.rsplit_once('@') {
        // SIP002: userinfo is base64(method:password) or, for 2022 ciphers,
        // percent-encoded plain text.
        let cred = decode_base64_flexible(cred_part).unwrap_or_else(|| percent_decode(cred_part));
        let (cipher, pass) = cred.split_once(':')?;
        let (srv, prt) = split_host_port(server_part)?;
        (cipher.to_string(), pass.to_string(), srv, prt)
    } else {
        let encoded = info_part.split(['/', '?']).next()?;
        let decoded = decode_base64_flexible(encoded)?;
        let (cred_part, server_part) = decoded.rsplit_once('@')?;
        let (cipher, pass) = cred_part.split_once(':')?;
        let (srv, prt) = split_host_port(server_part)?;
        (cipher.to_string(), pass.to_string(), srv, prt)
    };

    let name = fragment.unwrap_or_else(|| format!("{server}:{port}"));

    Some(ProxyNode {
        id: stable_id(link),
        name,
        protocol: Protocol::Shadowsocks,
        server,
        port,
        secret,
        cipher: Some(cipher),
        network: "tcp".to_string(),
        path: None,
        host: None,
        service_name: None,
        security: "none".to_string(),
        sni: None,
        alpn: None,
        fingerprint: None,
        pbk: None,
        sid: None,
        spider_x: None,
        flow: None,
        raw_link: link.to_string(),
        subscription_id: sub_id,
        ping_ms: None,
    })
}

pub fn parse_subscription_text(text: &str, sub_id: &str) -> Vec<ProxyNode> {
    let decoded = decode_base64_flexible(text).unwrap_or_else(|| text.to_string());
    let mut seen = HashSet::new();
    decoded
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter_map(|l| parse_link(l, Some(sub_id.to_string())))
        .filter(|n| seen.insert(n.id.clone()))
        .collect()
}

fn percent_decode(s: &str) -> String {
    percent_decode_str(s).decode_utf8_lossy().into_owned()
}

/// IPv6 literals come back from `Url` in brackets, which neither Xray nor
/// socket resolution accept.
fn url_host(url: &Url) -> Option<String> {
    Some(match url.host()? {
        Host::Ipv6(ip) => ip.to_string(),
        Host::Ipv4(ip) => ip.to_string(),
        Host::Domain(d) => percent_decode(d),
    })
}

/// Parses `host:port`, tolerating `[v6]:port` and SIP002's trailing
/// `/?plugin=...` section.
fn split_host_port(s: &str) -> Option<(String, u16)> {
    let s = s.split(['/', '?']).next()?;
    let (host, port) = s.rsplit_once(':')?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.is_empty() {
        return None;
    }
    Some((host.to_string(), port.trim().parse().ok()?))
}

fn split_list(s: &str) -> Vec<String> {
    s.split(',').map(str::trim).filter(|x| !x.is_empty()).map(String::from).collect()
}

/// FNV-1a. Node ids are persisted (e.g. the active node), so they must not
/// depend on `DefaultHasher`, whose algorithm may change between Rust releases.
fn stable_id(input: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in input.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trojan_password_is_percent_decoded() {
        let n = parse_link("trojan://p%40ss%3Aw0rd@example.com:443?sni=a.example#T", None).expect("parse");
        assert_eq!(n.secret, "p@ss:w0rd");
        assert_eq!(n.sni.as_deref(), Some("a.example"));
    }

    #[test]
    fn ipv6_hosts_lose_their_brackets() {
        let n = parse_link("vless://uuid@[2001:db8::1]:443?type=ws#v6", None).expect("parse");
        assert_eq!(n.server, "2001:db8::1");
        let t = parse_link("trojan://pw@[2001:db8::2]:8443#v6", None).expect("parse");
        assert_eq!(t.server, "2001:db8::2");
    }

    #[test]
    fn names_keep_ampersands_and_plus_signs() {
        let n = parse_link("vless://u@h.example:443#Tom%20%26%20Jerry+1", None).expect("parse");
        assert_eq!(n.name, "Tom & Jerry+1");
    }

    #[test]
    fn vmess_accepts_string_ports_and_grpc_service_names() {
        let json = r#"{"v":"2","ps":"VM","add":"vm.example","port":"8443","id":"uuid","net":"grpc","path":"svc","tls":"tls","alpn":"h2,http/1.1"}"#;
        let link = format!("vmess://{}", STANDARD.encode(json));
        let n = parse_link(&link, None).expect("parse");
        assert_eq!(n.port, 8443);
        assert_eq!(n.service_name.as_deref(), Some("svc"));
        assert_eq!(n.security, "tls");
        assert_eq!(n.alpn, Some(vec!["h2".to_string(), "http/1.1".to_string()]));
    }

    #[test]
    fn vmess_rejects_out_of_range_ports() {
        let json = r#"{"add":"vm.example","port":70000,"id":"uuid"}"#;
        assert!(parse_link(&format!("vmess://{}", STANDARD.encode(json)), None).is_none());
    }

    #[test]
    fn shadowsocks_sip002_with_plugin_suffix() {
        let userinfo = URL_SAFE_NO_PAD.encode("aes-256-gcm:secret");
        let n = parse_link(&format!("ss://{userinfo}@ss.example:8388/?plugin=obfs-local#SS%201"), None).expect("parse");
        assert_eq!((n.server.as_str(), n.port), ("ss.example", 8388));
        assert_eq!(n.cipher.as_deref(), Some("aes-256-gcm"));
        assert_eq!(n.secret, "secret");
        assert_eq!(n.name, "SS 1");
    }

    #[test]
    fn shadowsocks_2022_plain_userinfo_and_ipv6() {
        let n = parse_link("ss://2022-blake3-aes-128-gcm:YWJj%3D@[2001:db8::3]:443#x", None).expect("parse");
        assert_eq!(n.cipher.as_deref(), Some("2022-blake3-aes-128-gcm"));
        assert_eq!(n.secret, "YWJj=");
        assert_eq!((n.server.as_str(), n.port), ("2001:db8::3", 443));
    }

    #[test]
    fn shadowsocks_legacy_fully_encoded_form() {
        let body = STANDARD.encode("chacha20-ietf-poly1305:pw@1.2.3.4:8388");
        let n = parse_link(&format!("ss://{body}"), None).expect("parse");
        assert_eq!((n.server.as_str(), n.port), ("1.2.3.4", 8388));
        assert_eq!(n.name, "1.2.3.4:8388");
    }

    #[test]
    fn ids_are_stable_and_distinct() {
        assert_eq!(stable_id("vless://a"), stable_id("vless://a"));
        assert_ne!(stable_id("vless://a"), stable_id("vless://b"));
        assert_eq!(stable_id(""), "cbf29ce484222325");
    }

    #[test]
    fn subscription_duplicates_are_dropped() {
        let text = "vless://u@a.example:443#A\nvless://u@a.example:443#A\n\nvless://u@b.example:443#B";
        assert_eq!(parse_subscription_text(text, "s").len(), 2);
    }
}
