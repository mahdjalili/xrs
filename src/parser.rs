use crate::model::{Protocol, ProxyNode};
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine;
use std::collections::HashMap;
use url::Url;

pub fn decode_base64_flexible(input: &str) -> Option<String> {
    let trimmed = input.trim().replace(['\n', '\r'], "");
    
    // Try standard engines
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
    let secret = parsed.username().to_string();
    let server = parsed.host_str()?.to_string();
    let port = parsed.port()?;
    let name = parsed
        .fragment()
        .map(urlencoding_decode)
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

    let id = format!("{:x}", md5_hash(link));

    Some(ProxyNode {
        id,
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
        alpn: query.get("alpn").map(|a| a.split(',').map(String::from).collect()),
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
    let secret = parsed.username().to_string();
    let server = parsed.host_str()?.to_string();
    let port = parsed.port()?;
    let name = parsed
        .fragment()
        .map(urlencoding_decode)
        .unwrap_or_else(|| format!("{server}:{port}"));

    let query: HashMap<String, String> = parsed.query_pairs().into_owned().collect();

    let security = query.get("security").cloned().unwrap_or_else(|| "tls".to_string());
    let network = query.get("type").cloned().unwrap_or_else(|| "tcp".to_string());
    let sni = query.get("sni").cloned().or_else(|| query.get("peer").cloned());
    let fingerprint = query.get("fp").cloned();
    let path = query.get("path").cloned();
    let host = query.get("host").cloned();

    let id = format!("{:x}", md5_hash(link));

    Some(ProxyNode {
        id,
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
        alpn: query.get("alpn").map(|a| a.split(',').map(String::from).collect()),
        fingerprint,
        pbk: None,
        sid: None,
        spider_x: None,
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

    let server = v["add"].as_str()?.to_string();
    let port = v["port"].as_u64()? as u16;
    let secret = v["id"].as_str()?.to_string();
    let name = v["ps"].as_str().map(String::from).unwrap_or_else(|| format!("{server}:{port}"));
    let network = v["net"].as_str().unwrap_or("tcp").to_string();
    let path = v["path"].as_str().map(String::from);
    let host = v["host"].as_str().map(String::from);
    let tls = v["tls"].as_str().unwrap_or("");
    let security = if tls == "tls" { "tls".to_string() } else { "none".to_string() };
    let sni = v["sni"].as_str().map(String::from).or_else(|| host.clone());
    let fingerprint = v["fp"].as_str().map(String::from);

    let id = format!("{:x}", md5_hash(link));

    Some(ProxyNode {
        id,
        name,
        protocol: Protocol::Vmess,
        server,
        port,
        secret,
        cipher: Some("auto".to_string()),
        network,
        path,
        host,
        service_name: None,
        security,
        sni,
        alpn: None,
        fingerprint,
        pbk: None,
        sid: None,
        spider_x: None,
        flow: None,
        raw_link: link.to_string(),
        subscription_id: sub_id,
        ping_ms: None,
    })
}

pub fn parse_shadowsocks(link: &str, sub_id: Option<String>) -> Option<ProxyNode> {
    let without_prefix = link.trim_start_matches("ss://");
    let (info_part, name) = if let Some(idx) = without_prefix.find('#') {
        let (left, right) = without_prefix.split_at(idx);
        (left, urlencoding_decode(&right[1..]))
    } else {
        (without_prefix, "Shadowsocks".to_string())
    };

    let (cipher, secret, server, port) = if let Some(at_idx) = info_part.find('@') {
        let (cred_part, server_part) = info_part.split_at(at_idx);
        let cred_decoded = decode_base64_flexible(cred_part).unwrap_or_else(|| cred_part.to_string());
        let (cipher, pass) = cred_decoded.split_once(':')?;
        let (srv, prt) = server_part[1..].split_once(':')?;
        (cipher.to_string(), pass.to_string(), srv.to_string(), prt.parse::<u16>().ok()?)
    } else {
        let decoded = decode_base64_flexible(info_part)?;
        let (cred_part, server_part) = decoded.split_once('@')?;
        let (cipher, pass) = cred_part.split_once(':')?;
        let (srv, prt) = server_part.split_once(':')?;
        (cipher.to_string(), pass.to_string(), srv.to_string(), prt.parse::<u16>().ok()?)
    };

    let id = format!("{:x}", md5_hash(link));

    Some(ProxyNode {
        id,
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
    let mut nodes = Vec::new();

    for line in decoded.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(node) = parse_link(trimmed, Some(sub_id.to_string())) {
            nodes.push(node);
        }
    }
    nodes
}

fn urlencoding_decode(s: &str) -> String {
    url::form_urlencoded::parse(s.as_bytes())
        .map(|(k, v)| if v.is_empty() { k.into_owned() } else { format!("{k}={v}") })
        .collect::<Vec<_>>()
        .join("")
}

fn md5_hash(input: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    input.hash(&mut hasher);
    hasher.finish()
}
