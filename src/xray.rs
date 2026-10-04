use crate::model::{AppConfig, Protocol, ProxyNode};
use crate::storage::{get_config_dir, get_data_dir};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

pub fn find_xray_binary() -> Option<PathBuf> {
    // Fixed locations first, in a deterministic order, so the daemon (run by
    // the systemd user service with a minimal PATH) and an interactive
    // terminal resolve the SAME core. A context-dependent match (PATH before
    // fixed paths) let the two disagree, which re-fired the TUN setup — and
    // its auth prompts — separately per context.
    let local = get_data_dir().join("xray");
    if local.is_file() {
        return Some(local);
    }

    if let Some(home) = std::env::home_dir() {
        let user_local = home.join(".local/bin/xray");
        if user_local.is_file() {
            return Some(user_local);
        }
    }

    for p in ["/usr/local/bin/xray", "/usr/bin/xray"] {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Some(pb);
        }
    }

    // Context-dependent (differs under the systemd user service).
    find_in_path("xray")
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(|dir| dir.join(name))
        .find(|p| fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0))
}

pub fn generate_xray_config(cfg: &AppConfig, node: &ProxyNode) -> Value {
    let mut inbounds = vec![
        json!({
            "tag": "socks-in",
            "port": cfg.inbounds.socks_port,
            "listen": cfg.inbounds.listen,
            "protocol": "socks",
            "settings": {
                "auth": "noauth",
                "udp": true
            },
            "sniffing": {
                "enabled": true,
                "destOverride": ["http", "tls"]
            }
        }),
        json!({
            "tag": "http-in",
            "port": cfg.inbounds.http_port,
            "listen": cfg.inbounds.listen,
            "protocol": "http",
            "settings": {
                "allowTransparent": false
            }
        }),
    ];

    // TUN Mode Inbound
    if cfg.tun.enabled {
        inbounds.push(json!({
            "tag": "tun-in",
            "port": 0,
            "protocol": "tun",
            "settings": {
                "name": cfg.tun.name,
                "MTU": cfg.tun.mtu
            },
            "sniffing": {
                "enabled": true,
                "destOverride": ["http", "tls"]
            }
        }));
    }

    let mut outbounds = Vec::new();

    // 1. Main Proxy Outbound
    let proxy_outbound = build_node_outbound(node, cfg.tun.enabled);
    outbounds.push(proxy_outbound);

    // 2. Direct Outbound (marked if in TUN mode to avoid routing loop)
    let mut direct_stream = json!({});
    if cfg.tun.enabled {
        direct_stream["sockopt"] = json!({ "mark": 255 });
    }
    outbounds.push(json!({
        "tag": "direct",
        "protocol": "freedom",
        "settings": {
            "domainStrategy": "UseIP"
        },
        "streamSettings": direct_stream
    }));

    // 3. Block Outbound
    outbounds.push(json!({
        "tag": "block",
        "protocol": "blackhole",
        "settings": {}
    }));

    // Build Routing Rules
    let mut rules = Vec::new();

    // Custom user rules from ~/.config/xrs/routes.json if present
    if let Ok(content) = fs::read_to_string(get_config_dir().join("routes.json"))
        && let Ok(custom_val) = serde_json::from_str::<Value>(&content)
    {
        rules.extend(custom_route_rules(&custom_val));
    }

    // Configured Routing Rules from cfg.routing.rules
    for rule in &cfg.routing.rules {
        if !rule.enabled {
            continue;
        }

        if !rule.block_domains.is_empty() {
            rules.push(json!({
                "type": "field",
                "outboundTag": "block",
                "domain": rule.block_domains
            }));
        }
        if !rule.block_ips.is_empty() {
            rules.push(json!({
                "type": "field",
                "outboundTag": "block",
                "ip": rule.block_ips
            }));
        }
        if !rule.direct_domains.is_empty() {
            rules.push(json!({
                "type": "field",
                "outboundTag": "direct",
                "domain": rule.direct_domains
            }));
        }
        if !rule.direct_ips.is_empty() {
            rules.push(json!({
                "type": "field",
                "outboundTag": "direct",
                "ip": rule.direct_ips
            }));
        }
        if !rule.proxy_domains.is_empty() {
            rules.push(json!({
                "type": "field",
                "outboundTag": "proxy",
                "domain": rule.proxy_domains
            }));
        }
        if !rule.proxy_ips.is_empty() {
            rules.push(json!({
                "type": "field",
                "outboundTag": "proxy",
                "ip": rule.proxy_ips
            }));
        }
    }

    // Catch-all: send to proxy
    rules.push(json!({
        "type": "field",
        "outboundTag": "proxy",
        "port": "0-65535"
    }));

    json!({
        "log": {
            "loglevel": "warning"
        },
        "inbounds": inbounds,
        "outbounds": outbounds,
        "routing": {
            "domainStrategy": cfg.routing.domain_strategy,
            "rules": rules
        }
    })
}

/// Accepts either a raw array of Xray rules or the `{direct, proxy, block}`
/// shorthand written by `ensure_directories`.
fn custom_route_rules(custom: &Value) -> Vec<Value> {
    if let Some(arr) = custom.as_array() {
        return arr.clone();
    }
    let mut rules = Vec::new();
    for tag in ["direct", "proxy", "block"] {
        let Some(section) = custom.get(tag) else {
            continue;
        };
        for (key, field) in [("domains", "domain"), ("ips", "ip")] {
            if let Some(list) = section.get(key).and_then(Value::as_array)
                && !list.is_empty()
            {
                rules.push(json!({ "type": "field", "outboundTag": tag, field: list }));
            }
        }
    }
    rules
}

/// Minimal config used by latency probes: one SOCKS inbound on `port` and a
/// single proxy outbound. When `bypass_tun` is set the outbound is fwmark'd so
/// it leaves via the physical uplink instead of looping into an active TUN.
pub fn generate_probe_config(node: &ProxyNode, port: u16, bypass_tun: bool) -> Value {
    let mut outbounds = vec![build_node_outbound(node, bypass_tun)];
    let mut direct_stream = json!({});
    if bypass_tun {
        direct_stream["sockopt"] = json!({ "mark": 255 });
    }
    outbounds.push(json!({
        "tag": "direct",
        "protocol": "freedom",
        "settings": { "domainStrategy": "UseIP" },
        "streamSettings": direct_stream
    }));
    json!({
        "log": { "loglevel": "error" },
        "inbounds": [{
            "tag": "probe-in",
            "listen": "127.0.0.1",
            "port": port,
            "protocol": "socks",
            "settings": { "udp": false }
        }],
        "outbounds": outbounds,
        "routing": {
            "domainStrategy": "AsIs",
            "rules": [{
                "type": "field",
                "outboundTag": "proxy",
                "port": "0-65535"
            }]
        }
    })
}

pub(crate) fn build_node_outbound(node: &ProxyNode, tun_enabled: bool) -> Value {
    let mut stream_settings = json!({
        "network": node.network,
        "security": node.security
    });

    if tun_enabled {
        stream_settings["sockopt"] = json!({ "mark": 255 });
    }

    if node.security == "tls" {
        let mut tls = json!({
            "allowInsecure": false
        });
        if let Some(ref sni) = node.sni {
            tls["serverName"] = json!(sni);
        }
        if let Some(ref fp) = node.fingerprint {
            tls["fingerprint"] = json!(fp);
        }
        if let Some(ref alpn) = node.alpn {
            tls["alpn"] = json!(alpn);
        }
        stream_settings["tlsSettings"] = tls;
    } else if node.security == "reality" {
        let mut reality = json!({
            "show": false
        });
        if let Some(ref sni) = node.sni {
            reality["serverName"] = json!(sni);
        }
        if let Some(ref fp) = node.fingerprint {
            reality["fingerprint"] = json!(fp);
        }
        if let Some(ref pbk) = node.pbk {
            reality["publicKey"] = json!(pbk);
        }
        if let Some(ref sid) = node.sid {
            reality["shortId"] = json!(sid);
        }
        if let Some(ref spx) = node.spider_x {
            reality["spiderX"] = json!(spx);
        }
        stream_settings["realitySettings"] = reality;
    }

    if node.network == "ws" {
        let mut ws = json!({});
        if let Some(ref path) = node.path {
            ws["path"] = json!(path);
        }
        if let Some(ref host) = node.host {
            ws["headers"] = json!({ "Host": host });
        }
        stream_settings["wsSettings"] = ws;
    } else if node.network == "grpc" {
        let mut grpc = json!({});
        if let Some(ref svc) = node.service_name {
            grpc["serviceName"] = json!(svc);
        }
        stream_settings["grpcSettings"] = grpc;
    } else if let Some(key) = match node.network.as_str() {
        "xhttp" => Some("xhttpSettings"),
        "splithttp" => Some("splithttpSettings"),
        "httpupgrade" => Some("httpupgradeSettings"),
        _ => None,
    } {
        let mut settings = json!({});
        if let Some(ref path) = node.path {
            settings["path"] = json!(path);
        }
        if let Some(ref host) = node.host {
            settings["host"] = json!(host);
        }
        stream_settings[key] = settings;
    } else if node.network == "h2" || node.network == "http" {
        let mut h2 = json!({});
        if let Some(ref path) = node.path {
            h2["path"] = json!(path);
        }
        if let Some(ref host) = node.host {
            h2["host"] = json!(host.split(',').map(str::trim).collect::<Vec<_>>());
        }
        stream_settings["httpSettings"] = h2;
    }

    match node.protocol {
        Protocol::Vless => {
            let mut user = json!({
                "id": node.secret,
                "encryption": "none"
            });
            if let Some(ref flow) = node.flow
                && !flow.is_empty() {
                    user["flow"] = json!(flow);
                }
            json!({
                "tag": "proxy",
                "protocol": "vless",
                "settings": {
                    "vnext": [{
                        "address": node.server,
                        "port": node.port,
                        "users": [user]
                    }]
                },
                "streamSettings": stream_settings
            })
        }
        Protocol::Vmess => {
            json!({
                "tag": "proxy",
                "protocol": "vmess",
                "settings": {
                    "vnext": [{
                        "address": node.server,
                        "port": node.port,
                        "users": [{
                            "id": node.secret,
                            "alterId": 0,
                            "security": "auto"
                        }]
                    }]
                },
                "streamSettings": stream_settings
            })
        }
        Protocol::Trojan => {
            json!({
                "tag": "proxy",
                "protocol": "trojan",
                "settings": {
                    "servers": [{
                        "address": node.server,
                        "port": node.port,
                        "password": node.secret
                    }]
                },
                "streamSettings": stream_settings
            })
        }
        Protocol::Shadowsocks => {
            json!({
                "tag": "proxy",
                "protocol": "shadowsocks",
                "settings": {
                    "servers": [{
                        "address": node.server,
                        "port": node.port,
                        "method": node.cipher.as_deref().unwrap_or("aes-256-gcm"),
                        "password": node.secret
                    }]
                },
                "streamSettings": stream_settings
            })
        }
        Protocol::Socks5 => {
            json!({
                "tag": "proxy",
                "protocol": "socks",
                "settings": {
                    "servers": [{
                        "address": node.server,
                        "port": node.port
                    }]
                },
                "streamSettings": stream_settings
            })
        }
        Protocol::Http => {
            json!({
                "tag": "proxy",
                "protocol": "http",
                "settings": {
                    "servers": [{
                        "address": node.server,
                        "port": node.port
                    }]
                },
                "streamSettings": stream_settings
            })
        }
    }
}

pub struct XrayRunner;

impl XrayRunner {
    pub fn pid_file() -> PathBuf {
        get_data_dir().join("xray.pid")
    }

    pub fn config_file() -> PathBuf {
        get_data_dir().join("xray_run.json")
    }

    pub fn get_running_pid() -> Option<u32> {
        let pf = Self::pid_file();
        let pid = fs::read_to_string(&pf).ok()?.trim().parse::<u32>().ok()?;
        if is_our_xray(pid, &Self::config_file()) {
            Some(pid)
        } else {
            let _ = fs::remove_file(&pf);
            None
        }
    }

    pub fn is_running() -> bool {
        Self::get_running_pid().is_some()
    }

    pub fn start(cfg: &AppConfig) -> Result<u32, String> {
        if let Some(pid) = Self::get_running_pid() {
            return Ok(pid);
        }

        // A dangling active id (e.g. the node vanished in a subscription
        // update) falls back to the first node instead of refusing to start.
        let node = cfg
            .active_node_id
            .as_ref()
            .and_then(|id| cfg.nodes.iter().find(|n| &n.id == id))
            .or_else(|| cfg.nodes.first());

        let node = node.ok_or_else(|| "No proxy node available. Add a subscription or single config first.".to_string())?;

        let xray_bin = find_xray_binary().ok_or_else(|| {
            "Xray binary not found. Run 'xrs install-xray' or place binary in ~/.local/share/xrs/xray".to_string()
        })?;

        // TUN mode setup is performed automatically here (and on every start,
        // idempotently) so no manual command is needed: file
        // capabilities are mandatory, the sudoers rule is best effort since
        // apply_tun_routing falls back to a single pkexec shell without it.
        if cfg.tun.enabled {
            if let Err(e) = check_or_setup_tun_caps(&xray_bin) {
                return Err(format!(
                    "TUN mode requires network capability: {e}. Run 'xrs install-xray' to (re)install the core with capabilities"
                ));
            }
            if !tun_sudoers_current() {
                log::warn!(
                    "TUN sudoers rule is missing or outdated (pre-0.7 rules miss /usr/sbin and /bin paths) — installing the refreshed rule, one-time auth"
                );
                if let Err(e) = install_tun_sudoers() {
                    log::warn!("Automatic TUN setup could not install the sudoers rule: {e}");
                }
            }
        }

        // Write config
        let xray_json = generate_xray_config(cfg, node);
        let cfg_path = Self::config_file();
        let content = serde_json::to_string_pretty(&xray_json)
            .map_err(|e| format!("Failed to serialize Xray config: {e}"))?;
        fs::write(&cfg_path, content)
            .map_err(|e| format!("Failed to write Xray config: {e}"))?;

        let log_file = fs::File::create(get_data_dir().join("xray.log"))
            .map_err(|e| format!("Failed to create log file: {e}"))?;
        let stdout_log = log_file
            .try_clone()
            .map_err(|e| format!("Failed to clone log file descriptor: {e}"))?;

        let asset_dir = get_data_dir();

        let mut child = Command::new(&xray_bin)
            .arg("run")
            .arg("-c")
            .arg(&cfg_path)
            .env("XRAY_LOCATION_ASSET", &asset_dir)
            .stdout(stdout_log)
            .stderr(log_file)
            .spawn()
            .map_err(|e| format!("Failed to spawn Xray process: {e}"))?;

        let pid = child.id();

        // Xray validates its config and loads geo data at startup, exiting
        // immediately on errors; catch that instead of reporting "connected".
        let deadline = Instant::now() + STARTUP_GRACE;
        while Instant::now() < deadline {
            if let Ok(Some(status)) = child.try_wait() {
                return Err(format!("Xray exited during startup ({status}): {}", log_tail()));
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        let _ = fs::write(Self::pid_file(), pid.to_string());

        // Reap the child when it exits. In a long-lived parent such as the TUI
        // an unreaped child lingers as a zombie that still looks "running".
        std::thread::spawn(move || {
            let _ = child.wait();
        });

        if cfg.tun.enabled {
            if let Err(e) = apply_tun_routing(&cfg.tun.name, cfg.tun.mtu, true) {
                let _ = Self::stop();
                return Err(format!("TUN routing failed: {e}. Connect once from a terminal so the one-time setup can finish."));
            }
        } else {
            set_system_proxy(true, cfg.inbounds.socks_port, cfg.inbounds.http_port);
        }

        Ok(pid)
    }

    pub fn stop() -> Result<(), String> {
        if let Some(pid) = Self::get_running_pid() {
            let pid_s = pid.to_string();
            let _ = Command::new("kill").args(["-TERM", &pid_s]).status();
            let deadline = Instant::now() + STOP_GRACE;
            while is_our_xray(pid, &Self::config_file()) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
            }
            if is_our_xray(pid, &Self::config_file()) {
                let _ = Command::new("kill").args(["-KILL", &pid_s]).status();
            }
            let _ = fs::remove_file(Self::pid_file());
        }

        // Remove TUN routing and disable system proxy
        let _ = apply_tun_routing("xrs-tun", 0, false);
        set_system_proxy(false, 10808, 10809);
        Ok(())
    }

    pub fn restart(cfg: &AppConfig) -> Result<u32, String> {
        let _ = Self::stop();
        std::thread::sleep(std::time::Duration::from_millis(150));
        Self::start(cfg)
    }
}

const STARTUP_GRACE: Duration = Duration::from_millis(400);
const STOP_GRACE: Duration = Duration::from_secs(2);

/// PID files outlive reboots and crashes, so the recorded PID may since have
/// been reused by an unrelated process or be a zombie. Only a live process
/// running our generated config counts; otherwise `stop` could kill a
/// stranger.
fn is_our_xray(pid: u32, config: &Path) -> bool {
    let proc_dir = PathBuf::from(format!("/proc/{pid}"));
    let Ok(stat) = fs::read_to_string(proc_dir.join("stat")) else {
        return false;
    };
    // The state field follows the parenthesised command name, which may itself contain spaces.
    let state = stat.rsplit_once(')').and_then(|(_, rest)| rest.split_whitespace().next());
    if !matches!(state, Some(s) if s != "Z" && s != "X") {
        return false;
    }
    let Ok(cmdline) = fs::read(proc_dir.join("cmdline")) else {
        return false;
    };
    let config = config.to_string_lossy();
    cmdline
        .split(|b| *b == 0)
        .any(|arg| String::from_utf8_lossy(arg) == config)
}

fn log_tail() -> String {
    let log = fs::read_to_string(get_data_dir().join("xray.log")).unwrap_or_default();
    let line = log.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("see xray.log");
    line.chars().take(240).collect()
}

pub fn check_or_setup_tun_caps(xray_path: &Path) -> Result<(), String> {
    // Check if binary has cap_net_admin
    let output = Command::new("getcap")
        .arg(xray_path)
        .output()
        .map_err(|e| e.to_string())?;

    let text = String::from_utf8_lossy(&output.stdout);
    if text.contains("cap_net_admin") {
        return Ok(());
    }

    // Diagnostic for the repeated-prompt report: names the exact core whose
    // capabilities are missing (it may differ from the default install path).
    log::warn!(
        "TUN capabilities missing on {}; requesting setcap via pkexec (one-time auth)",
        xray_path.display()
    );

    // Try setcap via pkexec
    let path_str = xray_path.to_string_lossy();
    let status = Command::new("pkexec")
        .args(["setcap", "cap_net_admin,cap_net_bind_service=+ep", path_str.as_ref()])
        .status()
        .map_err(|e| format!("Failed to elevate permissions for TUN mode: {e}"))?;

    if status.success() {
        Ok(())
    } else {
        Err("Permission denied for setting CAP_NET_ADMIN on xray".to_string())
    }
}

const TUN_TABLE: u32 = 1001;
const TUN_MARK: u32 = 255;
const TUN_ADDR: &str = "172.19.0.1/30";

fn is_valid_ifname(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 15
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

struct TunCmd {
    prog: String,
    args: Vec<String>,
    allow_failure: bool,
}

/// Where the routing tools may live across distros (Debian puts both in
/// /usr/sbin, usrmerged systems alias /bin and /sbin to /usr/*). resolve_tool
/// and the sudoers rule must stay in agreement: a path the rule does not
/// cover makes `sudo -n` fail and TUN routing fall back to a pkexec password
/// prompt on every connect.
const TOOL_DIRS: [&str; 4] = ["/usr/bin", "/usr/sbin", "/bin", "/sbin"];

/// Absolute path of a routing tool; falls back to /usr/bin/<name> (covered by
/// the sudoers rule) when nothing resolves, so the failure mode is unchanged.
fn resolve_tool(name: &str) -> String {
    TOOL_DIRS
        .iter()
        .map(|dir| Path::new(dir).join(name))
        .find(|p| p.is_file())
        .unwrap_or_else(|| Path::new("/usr/bin").join(name))
        .to_string_lossy()
        .into_owned()
}

fn tun_up_commands(ifname: &str, mtu: u32) -> Vec<TunCmd> {
    let ip = resolve_tool("ip");
    let sysctl = resolve_tool("sysctl");
    vec![
        TunCmd {
            prog: ip.clone(),
            args: vec![
                "link".to_string(),
                "set".to_string(),
                "dev".to_string(),
                ifname.to_string(),
                "up".to_string(),
                "mtu".to_string(),
                mtu.to_string(),
            ],
            allow_failure: false,
        },
        TunCmd {
            prog: ip.clone(),
            args: vec![
                "addr".to_string(),
                "replace".to_string(),
                TUN_ADDR.to_string(),
                "dev".to_string(),
                ifname.to_string(),
            ],
            allow_failure: false,
        },
        TunCmd {
            prog: ip.clone(),
            args: vec![
                "route".to_string(),
                "replace".to_string(),
                "default".to_string(),
                "dev".to_string(),
                ifname.to_string(),
                "table".to_string(),
                TUN_TABLE.to_string(),
            ],
            allow_failure: false,
        },
        // Delete stale rule first; failure means "no old rule", which is fine.
        TunCmd {
            prog: ip.clone(),
            args: vec![
                "rule".to_string(),
                "del".to_string(),
                "not".to_string(),
                "fwmark".to_string(),
                TUN_MARK.to_string(),
                "table".to_string(),
                TUN_TABLE.to_string(),
            ],
            allow_failure: true,
        },
        TunCmd {
            prog: ip.clone(),
            args: vec![
                "rule".to_string(),
                "add".to_string(),
                "not".to_string(),
                "fwmark".to_string(),
                TUN_MARK.to_string(),
                "table".to_string(),
                TUN_TABLE.to_string(),
            ],
            allow_failure: false,
        },
        TunCmd {
            prog: sysctl.clone(),
            args: vec!["-w".to_string(), "net.ipv4.conf.all.rp_filter=2".to_string()],
            allow_failure: false,
        },
        TunCmd {
            prog: sysctl.clone(),
            args: vec![
                "-w".to_string(),
                "net.ipv4.conf.default.rp_filter=2".to_string(),
            ],
            allow_failure: false,
        },
        // Per-interface sysctl may fail if the interface vanished; not fatal.
        TunCmd {
            prog: sysctl.clone(),
            args: vec![
                "-w".to_string(),
                format!("net.ipv4.conf.{ifname}.rp_filter=2"),
            ],
            allow_failure: true,
        },
    ]
}

fn tun_down_commands() -> Vec<TunCmd> {
    let ip = resolve_tool("ip");
    vec![
        TunCmd {
            prog: ip.clone(),
            args: vec![
                "rule".to_string(),
                "del".to_string(),
                "not".to_string(),
                "fwmark".to_string(),
                TUN_MARK.to_string(),
                "table".to_string(),
                TUN_TABLE.to_string(),
            ],
            allow_failure: true,
        },
        TunCmd {
            prog: ip.clone(),
            args: vec![
                "route".to_string(),
                "flush".to_string(),
                "table".to_string(),
                TUN_TABLE.to_string(),
            ],
            allow_failure: true,
        },
    ]
}

fn tun_batch_shell(ifname: &str, mtu: u32, up: bool) -> String {
    if up {
        format!(
            r#"for i in $(seq 1 20); do ip link show "{ifname}" >/dev/null 2>&1 && break; sleep 0.1; done
ip link set dev "{ifname}" up mtu {mtu}
ip addr replace {addr} dev "{ifname}"
ip route replace default dev "{ifname}" table {table}
ip rule del not fwmark {mark} table {table} 2>/dev/null || true
ip rule add not fwmark {mark} table {table}
sysctl -w net.ipv4.conf.all.rp_filter=2 >/dev/null
sysctl -w net.ipv4.conf.default.rp_filter=2 >/dev/null
sysctl -w net.ipv4.conf."{ifname}".rp_filter=2 >/dev/null 2>&1 || true
"#,
            ifname = ifname,
            mtu = mtu,
            addr = TUN_ADDR,
            table = TUN_TABLE,
            mark = TUN_MARK,
        )
    } else {
        format!(
            r#"ip rule del not fwmark {mark} table {table} 2>/dev/null || true
ip route flush table {table} 2>/dev/null || true
"#,
            mark = TUN_MARK,
            table = TUN_TABLE,
        )
    }
}

fn try_sudo(program: &str, args: &[String]) -> bool {
    let mut cmd_args: Vec<&str> = vec!["-n", program];
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    cmd_args.extend(arg_refs);
    match Command::new("sudo").args(&cmd_args).output() {
        Ok(out) => out.status.success(),
        Err(_) => false,
    }
}

pub fn apply_tun_routing(tun_name: &str, mtu: u32, up: bool) -> Result<(), String> {
    if !is_valid_ifname(tun_name) {
        return Err("Invalid TUN interface name".to_string());
    }

    // Wait for the kernel interface to appear (unprivileged, no auth needed).
    if up {
        for _ in 0..20 {
            if Command::new("ip")
                .args(["link", "show", tun_name])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    // Fast path: passwordless sudo per command (zero prompts after one-time setup).
    // Only /usr/bin/ip and /usr/bin/sysctl are used here so the sudoers rule
    // from the sudoers rule covers every command. Tolerant commands (stale
    // rule cleanup) ignore failure and continue.
    let commands = if up {
        tun_up_commands(tun_name, mtu)
    } else {
        tun_down_commands()
    };
    let mut sudo_ok = true;
    for cmd in &commands {
        if !try_sudo(&cmd.prog, &cmd.args) && !cmd.allow_failure {
            // Diagnostic for the repeated-prompt report: the failing command
            // is the one the sudoers rule does not (or no longer) cover.
            log::warn!(
                "Passwordless sudo failed for '{} {:?}' — falling back to pkexec (auth prompt)",
                cmd.prog,
                cmd.args
            );
            sudo_ok = false;
            break;
        }
    }
    if sudo_ok {
        return Ok(());
    }

    // Slow path: single elevated shell (exactly one auth prompt).
    let batch = tun_batch_shell(tun_name, mtu, up);
    match Command::new("pkexec").args(["sh", "-c", &batch]).status() {
        Ok(status) if status.success() => Ok(()),
        Ok(_) => Err("Elevated permissions rejected for TUN routing".to_string()),
        Err(e) => Err(format!("Failed to execute pkexec for TUN routing: {e}")),
    }
}

/// Pure comparison used by `tun_sudoers_current` and the staleness test.
fn sudoers_is_current(on_disk: &str, user: &str) -> bool {
    on_disk == sudoers_content(user)
}

/// Whether the passwordless sudoers rule on disk matches the rule this build
/// installs. Older releases wrote a narrower rule (missing /usr/sbin and /bin
/// paths) at mode 0440 — unreadable to us, hence treated as stale so the
/// next TUN start refreshes it once, after which the file is mode 0444 and
/// comparable.
fn tun_sudoers_current() -> bool {
    let user = match std::env::var("USER").or_else(|_| std::env::var("LOGNAME")) {
        Ok(user) => user,
        // No username to compare against; keep whatever exists rather than
        // failing the rule on every start.
        Err(_) => return fs::metadata("/etc/sudoers.d/xrs").is_ok(),
    };
    fs::read_to_string("/etc/sudoers.d/xrs").is_ok_and(|on_disk| sudoers_is_current(&on_disk, &user))
}

/// One rule line covering every path resolve_tool can pick for `ip` and
/// `sysctl` across distros. Must stay in sync with TOOL_DIRS: a command path
/// the rule misses makes `sudo -n` fail and drops TUN routing into a pkexec
/// password prompt on every connect, switch, and restart.
fn sudoers_content(user: &str) -> String {
    let mut entries: Vec<String> = Vec::new();
    for tool in ["ip", "sysctl"] {
        for dir in TOOL_DIRS {
            entries.push(format!("{dir}/{tool} *"));
        }
    }
    format!(
        "# xrs TUN routing - passwordless ip/sysctl for TUN mode (installed by xrs)\n{user} ALL=(root) NOPASSWD: {}\n",
        entries.join(", ")
    )
}

pub fn install_tun_sudoers() -> Result<(), String> {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .map_err(|_| "Cannot determine current username for sudoers rule".to_string())?;
    if user.is_empty() || user == "root" {
        return Err("Refusing to install sudoers rule for empty/root user".to_string());
    }
    if !user.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err("Invalid username for sudoers rule".to_string());
    }

    let content = sudoers_content(&user);

    // Write via a single elevated shell so the user authenticates exactly once.
    // Use a quoted heredoc-style printf to avoid shell injection from `content`
    // (username is already validated above, content is fully constructed here).
    // Mode 444 (world-readable, not writable) so tun_sudoers_current can
    // detect future rule changes without elevation.
    let script = format!(
        "cat > /etc/sudoers.d/xrs <<'XRS_EOF'\n{content}XRS_EOF\nchmod 444 /etc/sudoers.d/xrs\nvisudo -c -f /etc/sudoers.d/xrs\n"
    );
    match Command::new("pkexec").args(["sh", "-c", &script]).status() {
        Ok(status) if status.success() => Ok(()),
        Ok(_) => Err("Elevated permissions rejected while installing sudoers rule".to_string()),
        Err(e) => Err(format!("Failed to execute pkexec for sudoers install: {e}")),
    }
}

/// Whether the kernel TUN interface currently exists. Reads /sys, so it needs
/// no elevation and no PATH lookup; used by the daemon to notice a vanished
/// interface and bring the tunnel back up.
pub fn tun_interface_up(name: &str) -> bool {
    is_valid_ifname(name) && Path::new("/sys/class/net").join(name).exists()
}

pub fn set_system_proxy(enable: bool, socks_port: u16, http_port: u16) {
    if enable {
        let _ = Command::new("gsettings")
            .args(["set", "org.gnome.system.proxy", "mode", "manual"])
            .output();
        let _ = Command::new("gsettings")
            .args(["set", "org.gnome.system.proxy.socks", "host", "127.0.0.1"])
            .output();
        let _ = Command::new("gsettings")
            .args(["set", "org.gnome.system.proxy.socks", "port", &socks_port.to_string()])
            .output();
        let _ = Command::new("gsettings")
            .args(["set", "org.gnome.system.proxy.http", "host", "127.0.0.1"])
            .output();
        let _ = Command::new("gsettings")
            .args(["set", "org.gnome.system.proxy.http", "port", &http_port.to_string()])
            .output();
        let _ = Command::new("gsettings")
            .args(["set", "org.gnome.system.proxy.https", "host", "127.0.0.1"])
            .output();
        let _ = Command::new("gsettings")
            .args(["set", "org.gnome.system.proxy.https", "port", &http_port.to_string()])
            .output();
    } else {
        let _ = Command::new("gsettings")
            .args(["set", "org.gnome.system.proxy", "mode", "none"])
            .output();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_config_binds_localhost_socks_and_marks_when_tun_up() {
        let node = crate::parser::parse_link(
            "vless://uuid@example.com:443?type=tcp&security=none#probe",
            None,
        )
        .expect("valid link");
        let cfg = generate_probe_config(&node, 19080, true);
        assert_eq!(cfg["inbounds"][0]["listen"], "127.0.0.1");
        assert_eq!(cfg["inbounds"][0]["port"], 19080);
        assert_eq!(cfg["outbounds"][0]["streamSettings"]["sockopt"]["mark"], 255);
        assert_eq!(cfg["outbounds"][1]["tag"], "direct");
    }

    #[test]
    fn http_based_transports_keep_path_and_host() {
        let mut node = crate::parser::parse_link(
            "vless://uuid@example.com:443?type=xhttp&security=tls&path=%2Fup&host=cdn.example.com#x",
            None,
        )
        .expect("valid link");
        let out = build_node_outbound(&node, false);
        assert_eq!(out["streamSettings"]["xhttpSettings"]["path"], "/up");
        assert_eq!(out["streamSettings"]["xhttpSettings"]["host"], "cdn.example.com");

        node.network = "httpupgrade".to_string();
        let out = build_node_outbound(&node, false);
        assert_eq!(out["streamSettings"]["httpupgradeSettings"]["path"], "/up");

        node.network = "h2".to_string();
        let out = build_node_outbound(&node, false);
        assert_eq!(out["streamSettings"]["httpSettings"]["host"][0], "cdn.example.com");
    }

    #[test]
    fn custom_routes_honor_every_section() {
        let rules = custom_route_rules(&json!({
            "direct": { "domains": ["domain:local"], "ips": [] },
            "proxy": { "domains": ["domain:example.com"], "ips": ["1.2.3.4"] },
            "block": { "domains": [], "ips": [] }
        }));
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[1]["outboundTag"], "proxy");
        assert_eq!(rules[1]["domain"][0], "domain:example.com");
        assert_eq!(rules[2]["ip"][0], "1.2.3.4");

        let raw = json!([{ "type": "field", "outboundTag": "direct", "port": "53" }]);
        assert_eq!(custom_route_rules(&raw), raw.as_array().cloned().unwrap_or_default());
    }

    #[test]
    fn process_check_matches_only_live_processes_running_our_config() {
        let marker = "/tmp/xrs-test-run.json";
        let mut child = Command::new("sh")
            .args(["-c", "sleep 30", "sh", marker])
            .spawn()
            .expect("spawn sh");
        let pid = child.id();
        // cmdline only reflects the new argv once exec has completed.
        let deadline = Instant::now() + Duration::from_secs(2);
        while !is_our_xray(pid, Path::new(marker)) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(is_our_xray(pid, Path::new(marker)));
        assert!(!is_our_xray(pid, Path::new("/tmp/some-other-config.json")));

        let _ = child.kill();
        std::thread::sleep(Duration::from_millis(100));
        assert!(!is_our_xray(pid, Path::new(marker)), "zombie must not count as running");

        let _ = child.wait();
        assert!(!is_our_xray(pid, Path::new(marker)));
    }

    #[test]
    fn tun_setup_uses_configured_mtu() {
        let cmds = tun_up_commands("xrs-tun", 1400);
        assert!(cmds[0].args.iter().any(|a| a == "1400"));
        assert!(tun_batch_shell("xrs-tun", 1400, true).contains("mtu 1400"));
    }

    #[test]
    fn tun_interface_check_never_panics_and_rejects_bad_names() {
        assert!(!tun_interface_up(""));
        assert!(!tun_interface_up("bad name!"));
        assert!(!tun_interface_up("xrs-no-such-interface"));
    }

    #[test]
    fn resolves_tools_from_candidate_dirs_with_covered_fallback() {
        let sh = resolve_tool("sh");
        assert!(
            sh.starts_with('/') && TOOL_DIRS.iter().any(|d| sh.starts_with(&format!("{d}/"))),
            "resolved path {sh} is outside the sudoers-covered directories"
        );
        assert_eq!(resolve_tool("xrs-no-such-tool-xyz"), "/usr/bin/xrs-no-such-tool-xyz");
    }

    #[test]
    fn sudoers_rule_covers_every_candidate_tool_path() {
        let content = sudoers_content("mahdi");
        for tool in ["ip", "sysctl"] {
            for dir in TOOL_DIRS {
                assert!(content.contains(&format!("{dir}/{tool} *")), "rule misses {dir}/{tool}");
            }
        }
        // Passwordless setcap would be a full privilege-escalation hole.
        assert!(!content.contains("setcap"));
    }

    /// Pre-0.7 installs carry this narrower rule (no /usr/sbin/ip, no
    /// /bin/sysctl) at mode 0440. The freshness check must treat it — and
    /// anything that is not the current content — as stale so it gets
    /// refreshed once instead of failing `sudo -n` forever.
    #[test]
    fn legacy_narrow_sudoers_rule_is_detected_as_stale() {
        let legacy = "# xrs TUN routing - passwordless ip/sysctl for TUN mode (installed by `xrs setup-tun`)\nmahdi ALL=(root) NOPASSWD: /usr/bin/ip *, /sbin/ip *, /usr/bin/sysctl *, /usr/sbin/sysctl *\n";
        assert!(!sudoers_is_current(legacy, "mahdi"));
        assert!(sudoers_is_current(&sudoers_content("mahdi"), "mahdi"));
    }

    /// The prompt-free fast path only works when every command apply_tun_routing
    /// executes is matched by the sudoers rule; anything unmatched sends TUN
    /// routing to a pkexec password prompt on every connect and switch.
    #[test]
    fn every_routing_command_is_covered_by_the_sudoers_rule() {
        let content = sudoers_content("u");
        let up = tun_up_commands("xrs-tun", 1500);
        let down = tun_down_commands();
        for cmd in up.iter().chain(&down) {
            assert!(
                cmd.prog.starts_with('/') && content.contains(&format!("{} *", cmd.prog)),
                "sudoers rule does not cover {} ({:?})",
                cmd.prog,
                cmd.args
            );
        }
    }
}
