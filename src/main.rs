#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used)]

mod latency;
mod logging;
mod model;
mod parser;
mod service;
mod storage;
mod theme;
mod ui;
mod uri;
mod xray;

use clap::{Parser, Subcommand};
use eyre::{eyre, Result};
use colored::*;
use model::{AppConfig, RouteRule};
use storage::*;
use log::{debug, error, info, warn};
use xray::{check_or_setup_tun_caps, find_xray_binary, install_tun_sudoers, tun_interface_up, XrayRunner};

#[derive(Parser)]
#[command(
    name = "xrs",
    author = "Mahdi",
    version,
    about = "xrs - an xray cli first ultra fast lightweight client"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Open the interactive terminal UI (default)
    Tui,
    /// Start the proxy daemon in the background (installs and starts the systemd user service)
    Start,
    /// Run the proxy daemon in the foreground (internal: the systemd service entry point)
    #[command(hide = true)]
    Run,
    /// Manage xrs systemd user service (install, start, stop, restart, status)
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
    /// Stop the proxy service daemon
    Stop,
    /// Restart the proxy service daemon
    Restart,
    /// Show proxy status (use --json for machine-readable output)
    Status {
        #[arg(long, help = "Output status as JSON for scripts and plugins")]
        json: bool,
    },
    /// Toggle proxy connection on/off
    Toggle,
    /// Manage TUN mode (full system-level VPN routing)
    Tun {
        #[arg(help = "on | off | status")]
        mode: Option<String>,
    },
    /// Grant CAP_NET_ADMIN permissions to Xray binary for TUN mode
    SetupTun,
    /// Manage routing rules (Iran bypass, adblock, custom domains & IPs)
    Route {
        #[command(subcommand)]
        action: RouteAction,
    },
    /// Manage subscriptions
    Sub {
        #[command(subcommand)]
        action: SubAction,
    },
    /// Manage, add, and select proxy nodes ('xrs node add <link>')
    Node {
        #[command(subcommand)]
        action: NodeAction,
    },
    /// Enable or disable GNOME/Hyprland system proxy (on | off)
    Proxy {
        #[arg(help = "on | off")]
        mode: String,
    },
    /// Download and install the Xray-core binary and its official geo routing data
    InstallXray,
}

#[derive(Subcommand)]
enum RouteAction {
    /// List all routing rules and their status
    List,
    /// Toggle a routing rule on or off
    Toggle {
        #[arg(help = "Rule ID or name keyword")]
        id_or_name: String,
    },
    /// Add a new custom routing rule
    Add {
        #[arg(help = "Rule name")]
        name: String,
        #[arg(long, help = "Direct domains (comma separated)")]
        direct_domains: Option<String>,
        #[arg(long, help = "Direct IPs (comma separated)")]
        direct_ips: Option<String>,
        #[arg(long, help = "Block domains (comma separated)")]
        block_domains: Option<String>,
        #[arg(long, help = "Block IPs (comma separated)")]
        block_ips: Option<String>,
    },
    /// Remove a routing rule (interactive if no ID specified)
    Remove {
        #[arg(help = "Optional Rule ID or 1-based index")]
        id: Option<String>,
        #[arg(short, long, help = "Skip confirmation prompt")]
        yes: bool,
    },
    /// Install the optional chocolate4u Iran preset (routing data + bypass rule)
    SetupIran,
}

#[derive(Subcommand)]
enum SubAction {
    /// Add a new subscription link
    Add {
        #[arg(help = "Subscription URL")]
        url: String,
        #[arg(short, long, help = "Optional name for subscription")]
        name: Option<String>,
    },
    /// List all subscriptions
    List,
    /// Update all subscriptions
    Update,
    /// Remove a subscription (interactive if no ID specified)
    Remove {
        #[arg(help = "Optional Subscription ID or 1-based index")]
        id: Option<String>,
        #[arg(short, long, help = "Skip confirmation prompt")]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum ServiceAction {
    /// Install and enable xrs as a systemd user service
    Install,
    /// Uninstall xrs systemd user service
    Uninstall,
    /// Start xrs systemd user service
    Start,
    /// Stop xrs systemd user service
    Stop,
    /// Restart xrs systemd user service
    Restart,
    /// Show status of xrs systemd user service
    Status,
}

#[derive(Subcommand)]
enum NodeAction {
    /// Add a single proxy node link (vless://, vmess://, trojan://, ss://)
    Add {
        #[arg(help = "Proxy URI link")]
        link: String,
    },
    /// List all available proxy nodes
    List,
    /// Select an active node (interactive if no ID specified)
    Select {
        #[arg(help = "Optional Node ID or 1-based index")]
        id_or_index: Option<String>,
    },
    /// Remove a proxy node (interactive if no ID specified)
    Remove {
        #[arg(help = "Optional Node ID or 1-based index")]
        id: Option<String>,
        #[arg(short, long, help = "Skip confirmation prompt")]
        yes: bool,
    },
    /// Test real (through-proxy) latency of all nodes
    Ping,
}

fn main() -> Result<()> {
    logging::init();

    let _ = ensure_directories();
    let cli = Cli::parse();
    let mut cfg = load_config();

    match cli.command {
        None | Some(Commands::Tui) => {
            // Opening xrs counts as a start: make sure the background service
            // is up so the proxy keeps running after the terminal closes.
            if !service::ensure_started() {
                debug!("No usable systemd user session; Xray will be managed directly");
            }
            ui::run_tui().map_err(|e| eyre!("{e}"))?;
        }
        Some(Commands::Start) => {
            if service::ensure_started() {
                info!("xrs service running in the background");
                match service::wait_running_pid() {
                    Some(pid) => {
                        println!(
                            "{} PID: {} (SOCKS: 127.0.0.1:{}, HTTP: 127.0.0.1:{}, TUN: {})",
                            "✔ Started xrs in the background.".green().bold(),
                            pid,
                            cfg.inbounds.socks_port,
                            cfg.inbounds.http_port,
                            if cfg.tun.enabled { "ON".green() } else { "OFF".dimmed() }
                        );
                        if let Some(ref id) = cfg.active_node_id
                            && let Some(n) = cfg.nodes.iter().find(|n| &n.id == id) {
                                println!("  Active Node: {} ({}:{})", n.name.cyan(), n.server, n.port);
                            }
                    }
                    None => println!("{}", "✔ xrs service is running in the background.".green().bold()),
                }
            } else {
                // No usable systemd user session (containers, some WSL
                // setups): keep the previous behavior of spawning directly.
                println!("{}", "ℹ systemd user service unavailable; starting Xray directly.".dimmed());
                if XrayRunner::is_running() {
                    warn!("xrs is already running.");
                    println!("{}", "xrs is already running.".yellow());
                } else {
                    match XrayRunner::start(&cfg) {
                        Ok(pid) => {
                            info!("Started xrs with PID {pid}");
                            println!(
                                "{} PID: {} (SOCKS: 127.0.0.1:{}, HTTP: 127.0.0.1:{}, TUN: {})",
                                "✔ Started xrs.".green().bold(),
                                pid,
                                cfg.inbounds.socks_port,
                                cfg.inbounds.http_port,
                                if cfg.tun.enabled { "ON".green() } else { "OFF".dimmed() }
                            );
                            if let Some(ref id) = cfg.active_node_id
                                && let Some(n) = cfg.nodes.iter().find(|n| &n.id == id) {
                                    println!("  Active Node: {} ({}:{})", n.name.cyan(), n.server, n.port);
                                }
                        }
                        Err(e) => {
                            error!("Failed to start xrs: {e}");
                            eprintln!("{} {e}", "✖ Failed to start:".red().bold());
                            std::process::exit(1);
                        }
                    }
                }
            }
        }
        Some(Commands::Run) => {
            match XrayRunner::start(&cfg) {
                Ok(pid) => {
                    info!("xrs daemon running (PID: {pid})");
                    println!("✔ xrs running in foreground (PID: {pid}). Press Ctrl+C to stop.");
                    let xray_died = supervise(&cfg);
                    println!("\nShutting down xrs...");
                    let _ = XrayRunner::stop();
                    if xray_died {
                        error!("Xray exited unexpectedly; see xray.log");
                        eprintln!("✖ Xray exited unexpectedly; see {}", get_data_dir().join("xray.log").display());
                        std::process::exit(1);
                    }
                    info!("xrs shutdown cleanly");
                }
                Err(e) => {
                    error!("Failed to start xrs: {e}");
                    eprintln!("✖ Failed to start: {e}");
                    std::process::exit(1);
                }
            }
        }
        Some(Commands::Service { action }) => match action {
            ServiceAction::Install => {
                if service::install_unit() {
                    let _ = std::process::Command::new("systemctl").args(["--user", "enable", "xrs.service"]).status();
                    println!("{}", "✔ Installed and enabled xrs systemd user service!".green().bold());
                    println!("  Run 'xrs service start' to start it in the background.");
                } else {
                    eprintln!("{}", "✖ Could not write the systemd user unit.".red());
                }
            }
            ServiceAction::Uninstall => {
                let _ = std::process::Command::new("systemctl").args(["--user", "disable", "--now", "xrs.service"]).status();
                let home = std::env::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
                let service_file = home.join(".config/systemd/user/xrs.service");
                let _ = std::fs::remove_file(service_file);
                let _ = std::process::Command::new("systemctl").args(["--user", "daemon-reload"]).status();
                println!("{}", "✔ Uninstalled xrs systemd user service.".yellow());
            }
            ServiceAction::Start => {
                let _ = std::process::Command::new("systemctl").args(["--user", "start", "xrs.service"]).status();
                println!("{}", "✔ Started xrs systemd user service in background.".green());
            }
            ServiceAction::Stop => {
                let _ = std::process::Command::new("systemctl").args(["--user", "stop", "xrs.service"]).status();
                println!("{}", "✔ Stopped xrs systemd user service.".green());
            }
            ServiceAction::Restart => {
                let _ = std::process::Command::new("systemctl").args(["--user", "restart", "xrs.service"]).status();
                println!("{}", "✔ Restarted xrs systemd user service.".green());
            }
            ServiceAction::Status => {
                let _ = std::process::Command::new("systemctl").args(["--user", "status", "xrs.service"]).status();
            }
        },
        Some(Commands::Stop) => {
            // With the service as runtime mode, stopping must take down the
            // unit — otherwise Restart=on-failure would revive the proxy.
            if service::stop_unit() {
                info!("Stopped xrs service");
                println!("{}", "✔ Stopped xrs.".green());
            } else if XrayRunner::is_running() {
                let _ = XrayRunner::stop();
                info!("Stopped xrs");
                println!("{}", "✔ Stopped xrs.".green());
            } else {
                println!("{}", "xrs is not running.".dimmed());
            }
        }
        Some(Commands::Restart) => {
            if service::restart_unit() {
                info!("Restarted xrs service");
                match service::wait_running_pid() {
                    Some(pid) => println!("{} New PID: {}", "✔ Restarted xrs.".green(), pid),
                    None => println!("{}", "✔ Restarted xrs service.".green()),
                }
            } else {
                match XrayRunner::restart(&cfg) {
                    Ok(pid) => {
                        info!("Restarted xrs with PID {pid}");
                        println!("{} New PID: {}", "✔ Restarted xrs.".green(), pid);
                    }
                    Err(e) => {
                        error!("Restart failed: {e}");
                        eprintln!("{} {e}", "✖ Restart failed:".red());
                    }
                }
            }
        }
        Some(Commands::Status { json }) => {
            show_status(&cfg, json);
        }
        Some(Commands::Toggle) => {
            // The unit owns the daemon when present (same rationale as stop):
            // disconnecting must stop the unit or it would revive the proxy.
            if service::is_active() {
                let _ = service::stop_unit();
                println!("{}", "○ Disconnected xrs.".yellow());
            } else if XrayRunner::is_running() {
                let _ = XrayRunner::stop();
                println!("{}", "○ Disconnected xrs.".yellow());
            } else if service::ensure_started() {
                match service::wait_running_pid() {
                    Some(pid) => println!("{} PID: {pid}", "● Connected xrs.".green().bold()),
                    None => println!("{}", "● Connected xrs.".green().bold()),
                }
            } else {
                match XrayRunner::start(&cfg) {
                    Ok(pid) => println!("{} PID: {pid}", "● Connected xrs.".green().bold()),
                    Err(e) => eprintln!("{} {e}", "✖ Failed:".red()),
                }
            }
        }
        Some(Commands::Tun { mode }) => match mode.as_deref() {
            Some("on") | Some("enable") => {
                cfg.tun.enabled = true;
                let _ = save_config(&cfg);
                println!("{}", "✔ TUN mode ENABLED.".green().bold());
                println!("  Interface: {}", cfg.tun.name.cyan());
                if XrayRunner::is_running() {
                    let _ = XrayRunner::restart(&cfg);
                    println!("  Restarted xrs to apply TUN mode.");
                }
            }
            Some("off") | Some("disable") => {
                cfg.tun.enabled = false;
                let _ = save_config(&cfg);
                println!("{}", "✔ TUN mode DISABLED.".yellow());
                if XrayRunner::is_running() {
                    let _ = XrayRunner::restart(&cfg);
                    println!("  Restarted xrs to switch back to system proxy.");
                }
            }
            _ => {
                let state = if cfg.tun.enabled { "ENABLED".green().bold() } else { "DISABLED".dimmed() };
                println!("TUN Mode:   {state}");
                println!("Interface:  {}", cfg.tun.name);
                println!("Auto Route: {}", cfg.tun.auto_route);
                println!("\nUsage: xrs tun on | off");
            }
        },
        Some(Commands::SetupTun) => {
            if let Some(xray_bin) = find_xray_binary() {
                debug!("Setting up TUN capabilities for {}", xray_bin.display());
                match check_or_setup_tun_caps(&xray_bin) {
                    Ok(_) => println!("{}", "✔ TUN file capabilities configured.".green().bold()),
                    Err(e) => eprintln!("{} {e}", "✖ setcap failed:".red()),
                }
            } else {
                eprintln!("{}", "✖ Xray binary not found. Run 'xrs install-xray' first.".red());
            }
            match install_tun_sudoers() {
                Ok(_) => println!(
                    "{}",
                    "✔ Passwordless sudo installed for TUN routing (one-time auth, no more prompts).".green().bold()
                ),
                Err(e) => eprintln!("{} {e}", "✖ sudoers install failed:".red()),
            }
        }
        Some(Commands::Route { action }) => match action {
            RouteAction::List => {
                println!("{}", "Routing Rules:".bold());
                if cfg.routing.rules.is_empty() {
                    println!("  No routing rules defined. Use 'xrs route add' or 'xrs route setup-iran'");
                } else {
                    for (i, r) in cfg.routing.rules.iter().enumerate() {
                        let state = if r.enabled { "[ENABLED]".green().bold() } else { "[DISABLED]".dimmed() };
                        println!("  {:2}. {} {}", i + 1, state, r.name.cyan().bold());
                        if !r.description.is_empty() {
                            println!("      {}", r.description.dimmed());
                        }
                        if !r.direct_domains.is_empty() || !r.direct_ips.is_empty() {
                            println!("      Direct: {} domains, {} IPs", r.direct_domains.len(), r.direct_ips.len());
                        }
                        if !r.block_domains.is_empty() || !r.block_ips.is_empty() {
                            println!("      Block: {} domains, {} IPs", r.block_domains.len(), r.block_ips.len());
                        }
                    }
                }
            }
            RouteAction::Toggle { id_or_name } => {
                match toggle_route_rule(&mut cfg, &id_or_name) {
                    Ok(new_state) => {
                        let state_str = if new_state { "ENABLED".green().bold() } else { "DISABLED".yellow() };
                        println!("✔ Rule '{}' is now {}.", id_or_name.cyan(), state_str);
                        if XrayRunner::is_running() {
                            let _ = XrayRunner::restart(&cfg);
                            println!("  Restarted xrs to apply routing changes.");
                        }
                    }
                    Err(e) => eprintln!("{} {e}", "✖".red()),
                }
            }
            RouteAction::Add {
                name,
                direct_domains,
                direct_ips,
                block_domains,
                block_ips,
            } => {
                let id = format!(
                    "rule_{:x}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis()
                );
                let parse_comma = |opt: Option<String>| {
                    opt.map(|s| s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect())
                        .unwrap_or_default()
                };

                let rule = RouteRule {
                    id: id.clone(),
                    name: name.clone(),
                    enabled: true,
                    description: "User-defined custom rule".to_string(),
                    direct_domains: parse_comma(direct_domains),
                    direct_ips: parse_comma(direct_ips),
                    block_domains: parse_comma(block_domains),
                    block_ips: parse_comma(block_ips),
                    proxy_domains: Vec::new(),
                    proxy_ips: Vec::new(),
                };

                if add_route_rule(&mut cfg, rule).is_ok() {
                    println!("{} '{}' (ID: {})", "✔ Added routing rule:".green().bold(), name.cyan(), id);
                    if XrayRunner::is_running() {
                        let _ = XrayRunner::restart(&cfg);
                    }
                }
            }
            RouteAction::Remove { id, yes } => {
                let target_id = if let Some(id_str) = id {
                    id_str
                } else {
                    if cfg.routing.rules.is_empty() {
                        println!("No routing rules available to remove.");
                        return Ok(());
                    }
                    println!("{}", "Routing Rules:".bold());
                    for (i, r) in cfg.routing.rules.iter().enumerate() {
                        println!("  {:2}. {} ({})", i + 1, r.name.cyan(), r.id.dimmed());
                    }
                    if let Some(sel) = prompt_select("Select rule number or ID to remove (or Enter to cancel)") {
                        sel
                    } else {
                        println!("Cancelled.");
                        return Ok(());
                    }
                };

                let rule_index = if let Ok(idx) = target_id.parse::<usize>() {
                    if idx > 0 && idx <= cfg.routing.rules.len() {
                        Some(idx - 1)
                    } else {
                        None
                    }
                } else {
                    cfg.routing.rules.iter().position(|r| r.id == target_id)
                };

                if let Some(idx) = rule_index {
                    let rule_name = cfg.routing.rules[idx].name.clone();
                    let rule_id = cfg.routing.rules[idx].id.clone();
                    if yes || prompt_confirm(&format!("Are you sure you want to remove rule '{}'?", rule_name.cyan())) {
                        cfg.routing.rules.remove(idx);
                        let _ = save_config(&cfg);
                        println!("✔ Removed routing rule '{}' ({}).", rule_name, rule_id.dimmed());
                        if XrayRunner::is_running() {
                            let _ = XrayRunner::restart(&cfg);
                        }
                    } else {
                        println!("Cancelled.");
                    }
                } else {
                    eprintln!("{} Rule '{}' not found.", "✖".red(), target_id);
                }
            }
            RouteAction::SetupIran => {
                // Opt-in preset: fetch its chocolate4u routing data first —
                // the official geodata from install-xray lacks the preset's
                // geo codes, which would fatally break Xray at startup.
                if let Err(e) = install_iran_geodata() {
                    eprintln!("{} {e}", "✖ Iran routing data download failed:".red());
                } else if setup_iran_rule_preset(&mut cfg).is_ok() {
                    println!("{}", "✔ Iran bypass routing rule preset installed and enabled!".green().bold());
                    if XrayRunner::is_running() {
                        let _ = XrayRunner::restart(&cfg);
                    }
                }
            }
        },
        Some(Commands::Sub { action }) => match action {
            SubAction::Add { url, name } => {
                println!("Fetching subscription from: {}", url.dimmed());
                match add_subscription(&mut cfg, &url, name.as_deref()) {
                    Ok(sub) => {
                        println!(
                            "{} '{}' with {} nodes.",
                            "✔ Added subscription".green().bold(),
                            sub.name.cyan(),
                            sub.node_count
                        );
                    }
                    Err(e) => {
                        eprintln!("{} {e}", "✖ Error:".red());
                    }
                }
            }
            SubAction::List => {
                println!("{}", "Subscriptions:".bold());
                if cfg.subscriptions.is_empty() {
                    println!("  No subscriptions added yet. Use 'xrs sub add <URL>'");
                } else {
                    for (i, sub) in cfg.subscriptions.iter().enumerate() {
                        println!(
                            "  {}. {} (ID: {}, Nodes: {})",
                            i + 1,
                            sub.name.cyan().bold(),
                            sub.id.dimmed(),
                            sub.node_count
                        );
                        println!("     URL: {}", sub.url.dimmed());
                    }
                }
            }
            SubAction::Update => {
                println!("Updating all subscriptions...");
                let results = update_all_subscriptions(&mut cfg);
                for (name, res) in results {
                    match res {
                        Ok(cnt) => println!("  ✔ {}: {} nodes", name.cyan(), cnt),
                        Err(e) => println!("  ✖ {}: {}", name.red(), e),
                    }
                }
            }
            SubAction::Remove { id, yes } => {
                let target_id = if let Some(id_str) = id {
                    id_str
                } else {
                    if cfg.subscriptions.is_empty() {
                        println!("No subscriptions available to remove.");
                        return Ok(());
                    }
                    println!("{}", "Subscriptions:".bold());
                    for (i, sub) in cfg.subscriptions.iter().enumerate() {
                        println!("  {:2}. {} (Nodes: {}, ID: {})", i + 1, sub.name.cyan(), sub.node_count, sub.id.dimmed());
                    }
                    if let Some(sel) = prompt_select("Select subscription number or ID to remove (or Enter to cancel)") {
                        sel
                    } else {
                        println!("Cancelled.");
                        return Ok(());
                    }
                };

                let sub_index = if let Ok(idx) = target_id.parse::<usize>() {
                    if idx > 0 && idx <= cfg.subscriptions.len() {
                        Some(idx - 1)
                    } else {
                        None
                    }
                } else {
                    cfg.subscriptions.iter().position(|s| s.id == target_id)
                };

                if let Some(idx) = sub_index {
                    let sub_name = cfg.subscriptions[idx].name.clone();
                    let sub_id = cfg.subscriptions[idx].id.clone();
                    let node_count = cfg.subscriptions[idx].node_count;

                    if yes || prompt_confirm(&format!("Are you sure you want to remove subscription '{}' and its {} nodes?", sub_name.cyan(), node_count)) {
                        cfg.subscriptions.remove(idx);
                        cfg.nodes.retain(|n| n.subscription_id.as_deref() != Some(&sub_id));
                        let _ = save_config(&cfg);
                        println!("✔ Removed subscription '{}' and associated nodes.", sub_name);
                    } else {
                        println!("Cancelled.");
                    }
                } else {
                    eprintln!("{} Subscription '{}' not found.", "✖".red(), target_id);
                }
            }
        },
        Some(Commands::Node { action }) => match action {
            NodeAction::Add { link } => match add_single_node(&mut cfg, &link) {
                Ok(node) => {
                    println!(
                        "{} '{}' ({}:{} via {})",
                        "✔ Added proxy node:".green().bold(),
                        node.name.cyan().bold(),
                        node.server,
                        node.port,
                        node.protocol
                    );
                    println!("  ID: {}", node.id.dimmed());
                }
                Err(e) => {
                    eprintln!("{} {e}", "✖ Failed to add node:".red());
                }
            },
            NodeAction::Remove { id, yes } => {
                let target_id = if let Some(id_str) = id {
                    id_str
                } else {
                    if cfg.nodes.is_empty() {
                        println!("No proxy nodes available to remove.");
                        return Ok(());
                    }
                    println!("{}", "Available Proxy Nodes:".bold());
                    for (i, node) in cfg.nodes.iter().enumerate() {
                        println!("  {:2}. [{}] {} ({}:{})", i + 1, node.protocol, node.name.cyan(), node.server, node.port);
                    }
                    if let Some(sel) = prompt_select("Select node number or ID to remove (or Enter to cancel)") {
                        sel
                    } else {
                        println!("Cancelled.");
                        return Ok(());
                    }
                };

                let node_index = if let Ok(idx) = target_id.parse::<usize>() {
                    if idx > 0 && idx <= cfg.nodes.len() {
                        Some(idx - 1)
                    } else {
                        None
                    }
                } else {
                    cfg.nodes.iter().position(|n| n.id == target_id)
                };

                if let Some(idx) = node_index {
                    let node_name = cfg.nodes[idx].name.clone();
                    let node_id = cfg.nodes[idx].id.clone();

                    if yes || prompt_confirm(&format!("Are you sure you want to remove node '{}'?", node_name.cyan())) {
                        cfg.nodes.remove(idx);
                        if cfg.active_node_id.as_deref() == Some(&node_id) {
                            cfg.active_node_id = cfg.nodes.first().map(|n| n.id.clone());
                            if XrayRunner::is_running() {
                                let _ = XrayRunner::restart(&cfg);
                            }
                        }
                        let _ = save_config(&cfg);
                        println!("✔ Removed proxy node '{}'.", node_name);
                    } else {
                        println!("Cancelled.");
                    }
                } else {
                    eprintln!("{} Proxy node '{}' not found.", "✖".red(), target_id);
                }
            }
            NodeAction::List => {
                println!("{}", "Available Proxy Nodes:".bold());
                if cfg.nodes.is_empty() {
                    println!("  No nodes available. Add a subscription ('xrs sub add <url>') or single node ('xrs node add <uri>')");
                } else {
                    for (i, node) in cfg.nodes.iter().enumerate() {
                        let is_active = cfg.active_node_id.as_deref() == Some(&node.id);
                        let marker = if is_active { "★".green().bold() } else { " ".normal() };
                        let ping = match node.ping_ms {
                            Some(ms) => format!("{ms}ms").green(),
                            None => "---".dimmed(),
                        };
                        println!(
                            "  {} {:2}. [{:<5}] {:<32} {:<24} ping: {}",
                            marker,
                            i + 1,
                            node.protocol.to_string().cyan(),
                            node.name,
                            format!("{}:{}", node.server, node.port).dimmed(),
                            ping
                        );
                    }
                }
            }
            NodeAction::Select { id_or_index } => {
                let target_id = if let Some(id_str) = id_or_index {
                    id_str
                } else {
                    if cfg.nodes.is_empty() {
                        println!("No proxy nodes available. Add one with 'xrs node add <uri>'.");
                        return Ok(());
                    }
                    println!("{}", "Available Proxy Nodes:".bold());
                    for (i, node) in cfg.nodes.iter().enumerate() {
                        let is_active = cfg.active_node_id.as_deref() == Some(&node.id);
                        let marker = if is_active { "★".green().bold() } else { " ".normal() };
                        let ping = match node.ping_ms {
                            Some(ms) => format!("{ms}ms").green(),
                            None => "---".dimmed(),
                        };
                        println!(
                            "  {} {:2}. [{:<5}] {:<32} {:<24} ping: {}",
                            marker,
                            i + 1,
                            node.protocol.to_string().cyan(),
                            node.name,
                            format!("{}:{}", node.server, node.port).dimmed(),
                            ping
                        );
                    }
                    if let Some(sel) = prompt_select("Select node number or ID (or Enter to cancel)") {
                        sel
                    } else {
                        println!("Cancelled.");
                        return Ok(());
                    }
                };

                let target_node = if let Ok(idx) = target_id.parse::<usize>() {
                    if idx > 0 && idx <= cfg.nodes.len() {
                        Some(cfg.nodes[idx - 1].clone())
                    } else {
                        None
                    }
                } else {
                    cfg.nodes.iter().find(|n| n.id == target_id).cloned()
                };

                if let Some(node) = target_node {
                    cfg.active_node_id = Some(node.id.clone());
                    let _ = save_config(&cfg);
                    println!("✔ Switched active node to: {}", node.name.cyan().bold());
                    if XrayRunner::is_running() {
                        let _ = XrayRunner::restart(&cfg);
                        println!("  Restarted Xray core with new node.");
                    }
                } else {
                    eprintln!("{} Invalid node ID or index.", "✖".red());
                }
            }
            NodeAction::Ping => {
                println!("Testing real latency (through-proxy) for all nodes...");
                if find_xray_binary().is_none() {
                    eprintln!(
                        "{}",
                        "✖ Xray binary not found. Run 'xrs install-xray' first.".red()
                    );
                    return Ok(());
                }
                let tun_up = std::path::Path::new("/sys/class/net")
                    .join(&cfg.tun.name)
                    .exists();
                if tun_up {
                    println!(
                        "  {}",
                        "TUN is active; probe outbounds bypass the tunnel via fwmark.".dimmed()
                    );
                }
                for node in &mut cfg.nodes {
                    node.ping_ms = latency::real_latency(node, &cfg.tun.name);
                    match node.ping_ms {
                        Some(ms) => println!("  ✔ {:<30} -> {}ms", node.name.cyan(), ms),
                        None => println!("  ✖ {:<30} -> unreachable", node.name.dimmed()),
                    }
                }
                let _ = save_config(&cfg);
            }
        },
        Some(Commands::Proxy { mode }) => match mode.to_lowercase().as_str() {
            "on" | "enable" => {
                xray::set_system_proxy(true, cfg.inbounds.socks_port, cfg.inbounds.http_port);
                println!("✔ System proxy enabled (127.0.0.1:{})", cfg.inbounds.socks_port);
            }
            "off" | "disable" => {
                xray::set_system_proxy(false, cfg.inbounds.socks_port, cfg.inbounds.http_port);
                println!("✔ System proxy disabled.");
            }
            _ => {
                println!("Usage: xrs proxy on | off");
            }
        },
        Some(Commands::InstallXray) => {
            install_xray_and_assets()?;
        }
    }

    Ok(())
}

/// Waits until the daemon should shut down. Returns `true` if Xray died on
/// its own, or if a TUN-enabled session could not be healed, so the caller
/// can exit non-zero and let systemd's `Restart=on-failure` bring it back.
///
/// SIGTERM must be handled as well as Ctrl-C: it is what `systemctl stop`
/// sends, and dying without cleanup leaves TUN policy routing pointing at a
/// vanished interface, which black-holes all traffic.
fn supervise(cfg: &AppConfig) -> bool {
    use signal_hook::consts::{SIGINT, SIGTERM};
    use std::sync::mpsc::{self, RecvTimeoutError};

    const HEALTH_CHECK: std::time::Duration = std::time::Duration::from_secs(2);
    let (tx, rx) = mpsc::channel::<()>();
    // Held so the channel stays open (and recv_timeout keeps blocking) even
    // if signal registration fails.
    let _keepalive = tx.clone();
    match signal_hook::iterator::Signals::new([SIGINT, SIGTERM]) {
        Ok(mut signals) => {
            std::thread::spawn(move || {
                if signals.forever().next().is_some() {
                    let _ = tx.send(());
                }
            });
        }
        Err(e) => warn!("Could not install signal handlers: {e}"),
    }
    loop {
        match rx.recv_timeout(HEALTH_CHECK) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => return false,
            Err(RecvTimeoutError::Timeout) => {
                if !XrayRunner::is_running() {
                    return true;
                }
                // Keep TUN up continuously: a vanished interface (network
                // manager restart, manual deletion) is recreated by
                // restarting the core, which also reapplies policy routing.
                if cfg.tun.enabled
                    && !tun_interface_up(&cfg.tun.name)
                {
                    warn!("TUN interface '{}' disappeared; restarting Xray to recreate it", cfg.tun.name);
                    if XrayRunner::restart(cfg).is_err() {
                        return true;
                    }
                }
            }
        }
    }
}


fn prompt_confirm(prompt: &str) -> bool {
    use std::io::Write;
    print!("{prompt} [y/N]: ");
    let _ = std::io::stdout().flush();
    let mut input = String::new();
    if std::io::stdin().read_line(&mut input).is_ok() {
        let trimmed = input.trim().to_lowercase();
        trimmed == "y" || trimmed == "yes"
    } else {
        false
    }
}

fn prompt_select(prompt: &str) -> Option<String> {
    use std::io::Write;
    print!("{prompt}: ");
    let _ = std::io::stdout().flush();
    let mut input = String::new();
    if std::io::stdin().read_line(&mut input).is_ok() {
        let trimmed = input.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    } else {
        None
    }
}

fn show_status(cfg: &AppConfig, json_output: bool) {
    let running = XrayRunner::is_running();
    let pid = XrayRunner::get_running_pid();

    let active_node = if let Some(ref id) = cfg.active_node_id {
        cfg.nodes.iter().find(|n| &n.id == id).map(|n| n.name.as_str()).unwrap_or("None")
    } else {
        "None"
    };

    let active_rules: Vec<String> = cfg
        .routing
        .rules
        .iter()
        .filter(|r| r.enabled)
        .map(|r| r.name.clone())
        .collect();

    if json_output {
        let nodes: Vec<_> = cfg
            .nodes
            .iter()
            .map(|n| {
                serde_json::json!({
                    "id": n.id,
                    "name": n.name,
                    "protocol": n.protocol.to_string(),
                    "server": n.server,
                    "port": n.port,
                    "network": n.network,
                    "ping_ms": n.ping_ms,
                    "is_active": cfg.active_node_id.as_deref() == Some(&n.id)
                })
            })
            .collect();

        let rules_json: Vec<_> = cfg
            .routing
            .rules
            .iter()
            .map(|r| {
                serde_json::json!({
                    "id": r.id,
                    "name": r.name,
                    "enabled": r.enabled,
                    "direct_domains": r.direct_domains.len(),
                    "direct_ips": r.direct_ips.len(),
                    "block_domains": r.block_domains.len(),
                    "block_ips": r.block_ips.len()
                })
            })
            .collect();

        let obj = serde_json::json!({
            "running": running,
            "pid": pid,
            "active_node_id": cfg.active_node_id,
            "active_node_name": active_node,
            "socks_port": cfg.inbounds.socks_port,
            "http_port": cfg.inbounds.http_port,
            "tun_enabled": cfg.tun.enabled,
            "tun_interface": cfg.tun.name,
            "active_rules": active_rules,
            "nodes": nodes,
            "rules": rules_json
        });

        if let Ok(json_str) = serde_json::to_string(&obj) {
            println!("{json_str}");
        }
        return;
    }

    if running {
        println!("Status:       {}", format!("● Connected (PID: {})", pid.unwrap_or(0)).green().bold());
    } else {
        println!("Status:       {}", "○ Disconnected".red().bold());
    }

    println!("Active Node:  {}", active_node.cyan().bold());
    println!("SOCKS5 Port:  127.0.0.1:{}", cfg.inbounds.socks_port);
    println!("HTTP Port:    127.0.0.1:{}", cfg.inbounds.http_port);
    println!(
        "TUN Mode:     {}",
        if cfg.tun.enabled { format!("ON ({})", cfg.tun.name).green().bold() } else { "OFF".dimmed() }
    );
    println!("Active Rules: {}", if active_rules.is_empty() { "None".dimmed() } else { active_rules.join(", ").green() });
    println!("Total Nodes:  {}", cfg.nodes.len());
    println!("Subscriptions:{}", cfg.subscriptions.len());
}

/// Fallback when the GitHub API is unreachable. Keep this equal to the newest
/// known Xray-core tag (including prereleases — that is how Xray ships).
const XRAY_FALLBACK_TAG: &str = "v26.9.30";

fn xray_linux_asset() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "Xray-linux-arm64-v8a.zip",
        _ => "Xray-linux-64.zip",
    }
}

fn latest_xray_release_tag() -> Option<String> {
    let resp = ureq::get("https://api.github.com/repos/XTLS/Xray-core/releases")
        .header("User-Agent", concat!("xrs/", env!("CARGO_PKG_VERSION")))
        .call()
        .ok()?;
    let body = resp.into_body().read_to_string().ok()?;
    let releases: Vec<serde_json::Value> = serde_json::from_str(&body).ok()?;
    releases
        .into_iter()
        .find(|r| r.get("draft").and_then(|d| d.as_bool()) != Some(true))
        .and_then(|r| r.get("tag_name")?.as_str().map(str::to_string))
}

fn install_xray_and_assets() -> Result<()> {
    let data_dir = get_data_dir();
    std::fs::create_dir_all(&data_dir)?;

    let tag = latest_xray_release_tag().unwrap_or_else(|| {
        warn!("Could not resolve latest Xray-core tag; using {XRAY_FALLBACK_TAG}");
        XRAY_FALLBACK_TAG.to_string()
    });
    let asset = xray_linux_asset();
    let xray_url = format!("https://github.com/XTLS/Xray-core/releases/download/{tag}/{asset}");

    println!("{}", format!("1. Downloading Xray-core {tag} ({asset})...").cyan());
    let zip_dest = data_dir.join("xray.zip");

    let status = std::process::Command::new("curl")
        .args(["-L", "-f", "-o", &zip_dest.to_string_lossy(), &xray_url])
        .status()?;
    if !status.success() {
        return Err(eyre!("Failed to download Xray-core release ({tag})."));
    }

    println!("{}", "2. Extracting Xray binary and geo routing data...".cyan());
    // The release zip ships the official geoip/geosite data. Keep the
    // chocolate4u data untouched when the opt-in Iran preset is in use:
    // replacing it would fatally break the preset's geo lookups at startup.
    let iran_preset_on = load_config()
        .routing
        .rules
        .iter()
        .any(|r| r.id == "iran_bypass" && r.enabled);
    let mut unzip_args = vec![
        "-o".to_string(),
        zip_dest.to_string_lossy().to_string(),
        "xray".to_string(),
    ];
    if iran_preset_on {
        println!("{}", "   Iran routing preset is enabled — keeping its chocolate4u routing data.".cyan());
    } else {
        unzip_args.push("geoip.dat".to_string());
        unzip_args.push("geosite.dat".to_string());
    }
    unzip_args.push("-d".to_string());
    unzip_args.push(data_dir.to_string_lossy().to_string());
    let status = std::process::Command::new("unzip")
        .args(&unzip_args)
        .status()?;
    if !status.success() {
        return Err(eyre!("Failed to unzip Xray binary."));
    }
    let _ = std::fs::remove_file(zip_dest);

    // Make xray executable
    let xray_path = data_dir.join("xray");
    let status = std::process::Command::new("chmod")
        .args(["+x", &xray_path.to_string_lossy()])
        .status()?;
    if !status.success() {
        return Err(eyre!("Failed to set executable permissions on Xray binary."));
    }

    // Report the installed binary version when possible.
    let ver = std::process::Command::new(&xray_path)
        .arg("version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.lines().next().map(str::to_string))
        .unwrap_or_else(|| tag.clone());

    println!("{}", "✔ Xray core installed successfully!".green().bold());
    println!("  Version:  {}", ver.cyan());
    println!("  Location: {}", data_dir.display());
    if iran_preset_on {
        println!("  Iran routing preset stays installed (data: chocolate4u).");
    } else {
        println!("  Optional: Iran routing rules via '{}'", "xrs route setup-iran".cyan());
    }

    Ok(())
}

/// Downloads the chocolate4u Iran routing data over the dat files in the data
/// dir. The Iran preset is opt-in, so this runs only from `xrs route
/// setup-iran` (and the TUI preset action) — never from install-xray, whose
/// official geodata cannot satisfy the preset's geo lookups.
fn install_iran_geodata() -> Result<(), String> {
    let data_dir = get_data_dir();
    std::fs::create_dir_all(&data_dir).map_err(|e| format!("Failed to create data dir: {e}"))?;
    for (file, url) in [
        (
            "geoip.dat",
            "https://raw.githubusercontent.com/Chocolate4U/Iran-v2ray-rules/release/geoip.dat",
        ),
        (
            "geosite.dat",
            "https://raw.githubusercontent.com/Chocolate4U/Iran-v2ray-rules/release/geosite.dat",
        ),
    ] {
        println!("{}", format!("Downloading chocolate4u {file}...").cyan());
        let dest = data_dir.join(file);
        let status = std::process::Command::new("curl")
            .args(["-L", "-f", "-o", &dest.to_string_lossy(), url])
            .status()
            .map_err(|e| format!("Failed to run curl: {e}"))?;
        if !status.success() {
            return Err(format!("Failed to download {file}."));
        }
    }
    Ok(())
}
