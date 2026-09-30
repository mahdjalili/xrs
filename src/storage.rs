use crate::model::{AppConfig, ProxyNode, RouteRule, Subscription};
use crate::parser::{parse_link, parse_subscription_text};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn get_config_dir() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let new_dir = home.join(".config/xrs");
    let old_dir = home.join(".config/omaxray");
    if !new_dir.exists() && old_dir.exists() {
        let _ = fs::rename(&old_dir, &new_dir);
    }
    new_dir
}

pub fn get_data_dir() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let new_dir = home.join(".local/share/xrs");
    let old_dir = home.join(".local/share/omaxray");
    if !new_dir.exists() && old_dir.exists() {
        let _ = fs::rename(&old_dir, &new_dir);
    }
    new_dir
}

pub fn ensure_directories() -> std::io::Result<()> {
    fs::create_dir_all(get_config_dir())?;
    fs::create_dir_all(get_data_dir())?;

    // Earlier releases shipped a sample that blocked ads unconditionally,
    // overriding the AdBlock toggle; replace it only if the user never edited it.
    let routes_file = get_config_dir().join("routes.json");
    let is_legacy_sample = fs::read_to_string(&routes_file)
        .ok()
        .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
        .is_some_and(|v| v == legacy_sample_routes());
    if (!routes_file.exists() || is_legacy_sample)
        && let Ok(content) = serde_json::to_string_pretty(&sample_routes())
    {
        let _ = fs::write(&routes_file, content);
    }

    Ok(())
}

fn sample_routes() -> serde_json::Value {
    serde_json::json!({
        "description": "Custom routing rules for xrs. Rules here are prepended to Xray routing. Ad blocking is controlled by the AdBlock rule in the TUI.",
        "direct": {
            "domains": ["domain:local", "domain:internal"],
            "ips": ["geoip:private"]
        },
        "proxy": { "domains": [], "ips": [] },
        "block": { "domains": [], "ips": [] }
    })
}

fn legacy_sample_routes() -> serde_json::Value {
    serde_json::json!({
        "description": "Custom routing rules for OmaXray. Rules here are prepended to Xray routing.",
        "direct": {
            "domains": ["domain:local", "domain:internal"],
            "ips": ["geoip:private"]
        },
        "proxy": { "domains": [], "ips": [] },
        "block": { "domains": ["geosite:category-ads-all"], "ips": [] }
    })
}

pub fn load_config() -> AppConfig {
    let config_file = get_config_dir().join("config.json");
    let Ok(content) = fs::read_to_string(&config_file) else {
        return AppConfig::default();
    };
    match serde_json::from_str::<AppConfig>(&content) {
        Ok(cfg) => cfg,
        Err(e) => {
            // The next save would replace the unreadable file with defaults,
            // wiping every server and subscription, so keep a copy first.
            let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
            let backup = config_file.with_extension(format!("json.broken-{ts}"));
            let _ = fs::copy(&config_file, &backup);
            tracing::error!("Config {} is unreadable ({e}); saved a copy to {}", config_file.display(), backup.display());
            AppConfig::default()
        }
    }
}

/// Writes atomically (temp file + rename) so a crash or a concurrent reader
/// never sees a truncated file. The config holds proxy credentials, so it is
/// only readable by the owner.
pub fn save_config(cfg: &AppConfig) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    ensure_directories()?;
    let config_file = get_config_dir().join("config.json");
    let tmp = config_file.with_extension(format!("json.tmp-{}", std::process::id()));
    let content = serde_json::to_string_pretty(cfg)?;
    let write = || -> std::io::Result<()> {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(content.as_bytes())?;
        f.sync_all()?;
        fs::rename(&tmp, &config_file)
    };
    write().inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

pub fn add_single_node(cfg: &mut AppConfig, link: &str) -> Result<ProxyNode, String> {
    let node = parse_link(link, None).ok_or_else(|| "Failed to parse proxy link. Ensure it is a valid vless://, vmess://, trojan://, or ss:// URI.".to_string())?;

    cfg.nodes.retain(|n| n.id != node.id && n.raw_link != node.raw_link);
    cfg.nodes.push(node.clone());

    if cfg.active_node_id.is_none() {
        cfg.active_node_id = Some(node.id.clone());
    }

    save_config(cfg).map_err(|e| e.to_string())?;
    Ok(node)
}

pub fn add_subscription(cfg: &mut AppConfig, url: &str, name_opt: Option<&str>) -> Result<Subscription, String> {
    if cfg.subscriptions.iter().any(|s| s.url.trim() == url.trim()) {
        return Err("This subscription URL is already added. Use update to refresh it.".to_string());
    }
    let id = format!("{:x}", SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis());
    let name = name_opt.unwrap_or("Subscription").to_string();

    let fetched = fetch_subscription(url)?;
    let parsed_nodes = parse_subscription_text(&fetched, &id);
    let count = parsed_nodes.len();

    let sub = Subscription {
        id: id.clone(),
        name,
        url: url.to_string(),
        updated_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
        node_count: count,
    };

    cfg.subscriptions.push(sub.clone());
    cfg.nodes.extend(parsed_nodes);

    if cfg.active_node_id.is_none() && !cfg.nodes.is_empty() {
        cfg.active_node_id = Some(cfg.nodes[0].id.clone());
    }

    save_config(cfg).map_err(|e| e.to_string())?;
    Ok(sub)
}

pub fn update_all_subscriptions(cfg: &mut AppConfig) -> Vec<(String, Result<usize, String>)> {
    let mut results = Vec::new();
    let previous_active = cfg
        .active_node_id
        .as_ref()
        .and_then(|id| cfg.nodes.iter().find(|n| &n.id == id))
        .cloned();
    let old_pings: HashMap<String, u64> = cfg
        .nodes
        .iter()
        .filter_map(|n| n.ping_ms.map(|p| (n.raw_link.clone(), p)))
        .collect();

    for sub in &mut cfg.subscriptions {
        match fetch_subscription(&sub.url) {
            Ok(body) => {
                let mut parsed = parse_subscription_text(&body, &sub.id);
                for node in &mut parsed {
                    node.ping_ms = old_pings.get(&node.raw_link).copied();
                }
                let count = parsed.len();
                sub.node_count = count;
                sub.updated_at = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();

                // Remove existing nodes for this subscription
                cfg.nodes.retain(|n| n.subscription_id.as_deref() != Some(&sub.id));
                cfg.nodes.extend(parsed);
                results.push((sub.name.clone(), Ok(count)));
            }
            Err(e) => {
                results.push((sub.name.clone(), Err(e)));
            }
        }
    }

    if let Some(id) = previous_active.and_then(|prev| relocate_node(&cfg.nodes, &prev)) {
        cfg.active_node_id = Some(id);
    }

    let _ = save_config(cfg);
    results
}

/// Providers rotate link parameters (paths, SNI, keys) on refresh, which
/// changes node ids. Find the same logical server so the user stays on it.
fn relocate_node(nodes: &[ProxyNode], prev: &ProxyNode) -> Option<String> {
    let by = |pred: &dyn Fn(&ProxyNode) -> bool| nodes.iter().find(|n| pred(n)).map(|n| n.id.clone());
    by(&|n| n.id == prev.id)
        .or_else(|| by(&|n| n.raw_link == prev.raw_link))
        .or_else(|| by(&|n| n.name == prev.name && n.server == prev.server && n.port == prev.port))
        .or_else(|| by(&|n| n.name == prev.name))
}

fn fetch_subscription(url: &str) -> Result<String, String> {
    let resp = ureq::get(url)
        .header("User-Agent", concat!("xrs/", env!("CARGO_PKG_VERSION"), " (v2rayN; Clash)"))
        .call()
        .map_err(|e| format!("Failed to download subscription: {e}"))?;

    let body = resp
        .into_body()
        .read_to_string()
        .map_err(|e| format!("Failed to read response: {e}"))?;

    Ok(body)
}

pub fn add_route_rule(cfg: &mut AppConfig, rule: RouteRule) -> Result<(), String> {
    cfg.routing.rules.retain(|r| r.id != rule.id);
    cfg.routing.rules.push(rule);
    save_config(cfg).map_err(|e| e.to_string())
}

pub fn toggle_route_rule(cfg: &mut AppConfig, id_or_name: &str) -> Result<bool, String> {
    let lower = id_or_name.to_lowercase();
    let rule = cfg
        .routing
        .rules
        .iter_mut()
        .find(|r| r.id.to_lowercase() == lower || r.name.to_lowercase().contains(&lower))
        .ok_or_else(|| format!("Rule '{id_or_name}' not found"))?;

    rule.enabled = !rule.enabled;
    let new_state = rule.enabled;
    save_config(cfg).map_err(|e| e.to_string())?;
    Ok(new_state)
}

#[allow(dead_code)]
pub fn remove_route_rule(cfg: &mut AppConfig, id: &str) -> Result<bool, String> {
    let len_before = cfg.routing.rules.len();
    cfg.routing.rules.retain(|r| r.id != id);
    if cfg.routing.rules.len() < len_before {
        save_config(cfg).map_err(|e| e.to_string())?;
        Ok(true)
    } else {
        Ok(false)
    }
}

pub fn setup_iran_rule_preset(cfg: &mut AppConfig) -> Result<(), String> {
    let rule = RouteRule {
        id: "iran_bypass".to_string(),
        name: "Iran Bypass (chocolate4u)".to_string(),
        enabled: true,
        description: "Bypasses domestic Iranian domains, *.ir, and IP ranges".to_string(),
        direct_domains: vec!["geosite:ir".to_string(), "regexp:.*\\.ir$".to_string()],
        direct_ips: vec!["geoip:ir".to_string(), "geoip:private".to_string()],
        block_domains: Vec::new(),
        block_ips: Vec::new(),
        proxy_domains: Vec::new(),
        proxy_ips: Vec::new(),
    };
    add_route_rule(cfg, rule)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_link;

    fn node(link: &str) -> ProxyNode {
        parse_link(link, Some("s".into())).expect("valid link")
    }

    #[test]
    fn relocates_active_node_after_provider_rotates_params() {
        let prev = node("vless://u@fi.example.net:443?path=%2Fold&type=ws#FI1");
        let fresh = vec![
            node("vless://u@de.example.net:443?type=ws#DE1"),
            node("vless://u@fi.example.net:443?path=%2Fnew&type=ws#FI1"),
        ];
        assert_ne!(prev.id, fresh[1].id);
        assert_eq!(relocate_node(&fresh, &prev), Some(fresh[1].id.clone()));
    }

    #[test]
    fn relocation_gives_up_when_server_is_gone() {
        let prev = node("vless://u@fi.example.net:443#FI1");
        let fresh = vec![node("vless://u@de.example.net:443#DE1")];
        assert_eq!(relocate_node(&fresh, &prev), None);
    }

    #[test]
    fn partial_config_keeps_nodes_and_defaults_the_rest() {
        let cfg: AppConfig = serde_json::from_str(r#"{"nodes": [], "active_node_id": "x"}"#).expect("parse");
        assert_eq!(cfg.active_node_id.as_deref(), Some("x"));
        assert_eq!(cfg.inbounds.socks_port, 10808);
        assert!(!cfg.routing.rules.is_empty());
    }
}
