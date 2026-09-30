use crate::model::{AppConfig, ProxyNode, RouteRule, Subscription};
use crate::parser::{parse_link, parse_subscription_text};
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

    // Create a default routes.json if not present
    let routes_file = get_config_dir().join("routes.json");
    if !routes_file.exists() {
        let sample_routes = serde_json::json!({
            "description": "Custom routing rules for OmaXray. Rules here are prepended to Xray routing.",
            "direct": {
                "domains": [
                    "domain:local",
                    "domain:internal"
                ],
                "ips": [
                    "geoip:private"
                ]
            },
            "proxy": {
                "domains": [],
                "ips": []
            },
            "block": {
                "domains": [
                    "geosite:category-ads-all"
                ],
                "ips": []
            }
        });
        if let Ok(content) = serde_json::to_string_pretty(&sample_routes) {
            let _ = fs::write(&routes_file, content);
        }
    }

    Ok(())
}

pub fn load_config() -> AppConfig {
    let config_file = get_config_dir().join("config.json");
    if config_file.exists()
        && let Ok(content) = fs::read_to_string(&config_file)
            && let Ok(cfg) = serde_json::from_str::<AppConfig>(&content) {
                return cfg;
            }
    AppConfig::default()
}

pub fn save_config(cfg: &AppConfig) -> std::io::Result<()> {
    ensure_directories()?;
    let config_file = get_config_dir().join("config.json");
    let content = serde_json::to_string_pretty(cfg)?;
    fs::write(config_file, content)
}

pub fn add_single_node(cfg: &mut AppConfig, link: &str) -> Result<ProxyNode, String> {
    let node = parse_link(link, None).ok_or_else(|| "Failed to parse proxy link. Ensure it is a valid vless://, vmess://, trojan://, or ss:// URI.".to_string())?;

    // Remove existing node with same ID if any
    cfg.nodes.retain(|n| n.id != node.id);
    cfg.nodes.push(node.clone());

    if cfg.active_node_id.is_none() {
        cfg.active_node_id = Some(node.id.clone());
    }

    save_config(cfg).map_err(|e| e.to_string())?;
    Ok(node)
}

pub fn add_subscription(cfg: &mut AppConfig, url: &str, name_opt: Option<&str>) -> Result<Subscription, String> {
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

    for sub in &mut cfg.subscriptions {
        match fetch_subscription(&sub.url) {
            Ok(body) => {
                let parsed = parse_subscription_text(&body, &sub.id);
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

    let _ = save_config(cfg);
    results
}

fn fetch_subscription(url: &str) -> Result<String, String> {
    let resp = ureq::get(url)
        .header("User-Agent", "xrs/0.1.0 (v2rayN; Clash)")
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
