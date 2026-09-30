use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Protocol {
    Vless,
    Vmess,
    Trojan,
    Shadowsocks,
    Socks5,
    Http,
}

impl std::fmt::Display for Protocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Protocol::Vless => write!(f, "VLESS"),
            Protocol::Vmess => write!(f, "VMESS"),
            Protocol::Trojan => write!(f, "TROJAN"),
            Protocol::Shadowsocks => write!(f, "SS"),
            Protocol::Socks5 => write!(f, "SOCKS5"),
            Protocol::Http => write!(f, "HTTP"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyNode {
    pub id: String,
    pub name: String,
    pub protocol: Protocol,
    pub server: String,
    pub port: u16,
    pub secret: String, // uuid, password, or key
    pub cipher: Option<String>, // For shadowsocks

    pub network: String, // tcp, ws, grpc, splithttp, h2
    pub path: Option<String>,
    pub host: Option<String>,
    pub service_name: Option<String>,

    pub security: String, // none, tls, reality
    pub sni: Option<String>,
    pub alpn: Option<Vec<String>>,
    pub fingerprint: Option<String>,
    pub pbk: Option<String>, // Reality public key
    pub sid: Option<String>, // Reality short id
    pub spider_x: Option<String>,
    pub flow: Option<String>, // xtls-rprx-vision

    pub raw_link: String,
    pub subscription_id: Option<String>, // None for manual/single configs
    pub ping_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub id: String,
    pub name: String,
    pub url: String,
    pub updated_at: u64,
    pub node_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteRule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub direct_domains: Vec<String>,
    #[serde(default)]
    pub direct_ips: Vec<String>,
    #[serde(default)]
    pub block_domains: Vec<String>,
    #[serde(default)]
    pub block_ips: Vec<String>,
    #[serde(default)]
    pub proxy_domains: Vec<String>,
    #[serde(default)]
    pub proxy_ips: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct InboundConfig {
    pub socks_port: u16,
    pub http_port: u16,
    pub listen: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TunConfig {
    pub enabled: bool,
    pub name: String,
    pub mtu: u32,
    pub auto_route: bool,
    pub stack: String, // "system" or "gvisor"
}

impl Default for TunConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            name: "xrs-tun".to_string(),
            mtu: 1500,
            auto_route: true,
            stack: "system".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RoutingConfig {
    pub domain_strategy: String, // "IPIfNonMatch", "AsIs"
    pub rules: Vec<RouteRule>,
}

/// Every section falls back to its default when missing so configs written by
/// older or newer versions still load instead of being reset.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AppConfig {
    pub inbounds: InboundConfig,
    pub tun: TunConfig,
    pub routing: RoutingConfig,
    pub active_node_id: Option<String>,
    pub subscriptions: Vec<Subscription>,
    pub nodes: Vec<ProxyNode>,
}

impl Default for InboundConfig {
    fn default() -> Self {
        Self {
            socks_port: 10808,
            http_port: 10809,
            listen: "127.0.0.1".to_string(),
        }
    }
}

impl Default for RoutingConfig {
    fn default() -> Self {
        Self {
            domain_strategy: "IPIfNonMatch".to_string(),
            rules: vec![
                RouteRule {
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
                },
                RouteRule {
                    id: "adblock".to_string(),
                    name: "AdBlock & Malware".to_string(),
                    enabled: true,
                    description: "Blocks advertising, tracking, phishing, and malware domains".to_string(),
                    direct_domains: Vec::new(),
                    direct_ips: Vec::new(),
                    block_domains: vec![
                        "geosite:category-ads-all".to_string(),
                        "geosite:malware".to_string(),
                        "geosite:phishing".to_string(),
                        "geosite:cryptominers".to_string(),
                    ],
                    block_ips: vec!["geoip:malware".to_string(), "geoip:phishing".to_string()],
                    proxy_domains: Vec::new(),
                    proxy_ips: Vec::new(),
                },
            ],
        }
    }
}

