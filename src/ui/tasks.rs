use crate::latency;
use crate::model::{AppConfig, ProxyNode};
use crate::service;
use crate::storage::{add_subscription, setup_iran_rule_preset, update_all_subscriptions};
use crate::xray::XrayRunner;
use std::collections::VecDeque;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;

/// Real latency spawns a short-lived Xray per probe, so keep concurrency low.
const PING_WORKERS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnOp {
    Connect,
    Disconnect,
    Reconnect,
}

impl ConnOp {
    pub fn progress_label(self) -> &'static str {
        match self {
            ConnOp::Connect => "Connecting",
            ConnOp::Disconnect => "Disconnecting",
            ConnOp::Reconnect => "Reconnecting",
        }
    }
}

#[derive(Debug)]
pub enum TaskEvent {
    Ping { id: String, ms: Option<u64> },
    Connection { op: ConnOp, result: Result<(), String> },
    SubscriptionsUpdated(Vec<(String, Result<usize, String>)>),
    SubscriptionAdded(Result<(String, usize), String>),
    IranPreset(Result<(), String>),
}

pub struct PingTarget {
    pub id: String,
    pub node: ProxyNode,
}

pub fn spawn_connection(tx: Sender<TaskEvent>, op: ConnOp, cfg: AppConfig) {
    thread::spawn(move || {
        // The background service owns the daemon when systemd is available;
        // Xray is managed directly only as a fallback.
        let result = match op {
            ConnOp::Connect => {
                if service::ensure_started() {
                    Ok(())
                } else {
                    XrayRunner::start(&cfg).map(|_| ())
                }
            }
            ConnOp::Disconnect => {
                if service::stop_unit() {
                    Ok(())
                } else {
                    XrayRunner::stop()
                }
            }
            ConnOp::Reconnect => {
                if service::restart_unit() {
                    Ok(())
                } else {
                    XrayRunner::restart(&cfg).map(|_| ())
                }
            }
        };
        let _ = tx.send(TaskEvent::Connection { op, result });
    });
}

/// Probes every target concurrently with a small worker pool, streaming one
/// event per node so the table fills in live instead of after the slowest host.
/// Each probe measures real through-proxy latency (HTTP via a throwaway Xray).
pub fn spawn_ping(tx: Sender<TaskEvent>, targets: Vec<PingTarget>, tun_name: String) {
    let workers = PING_WORKERS.min(targets.len());
    let queue = Arc::new(Mutex::new(targets.into_iter().collect::<VecDeque<_>>()));
    let tun_name = Arc::new(tun_name);
    for _ in 0..workers {
        let queue = Arc::clone(&queue);
        let tun_name = Arc::clone(&tun_name);
        let tx = tx.clone();
        thread::spawn(move || {
            loop {
                let job = match queue.lock() {
                    Ok(mut q) => q.pop_front(),
                    Err(_) => None,
                };
                let Some(target) = job else { break };
                let ms = latency::measure(&target.node, &tun_name);
                if tx.send(TaskEvent::Ping { id: target.id, ms }).is_err() {
                    break;
                }
            }
        });
    }
}

pub fn spawn_update_subscriptions(tx: Sender<TaskEvent>, mut cfg: AppConfig) {
    thread::spawn(move || {
        let results = update_all_subscriptions(&mut cfg);
        let _ = tx.send(TaskEvent::SubscriptionsUpdated(results));
    });
}

pub fn spawn_add_subscription(tx: Sender<TaskEvent>, mut cfg: AppConfig, url: String, name: String) {
    thread::spawn(move || {
        let result = add_subscription(&mut cfg, &url, Some(&name)).map(|s| (s.name, s.node_count));
        let _ = tx.send(TaskEvent::SubscriptionAdded(result));
    });
}

/// The Iran preset downloads its chocolate4u routing data (tens of MB), so it
/// runs off the UI thread like every other network operation. The geodata is
/// part of the preset (opt-in), never of `xrs install-xray`.
pub fn spawn_install_iran_preset(tx: Sender<TaskEvent>, mut cfg: AppConfig) {
    thread::spawn(move || {
        let result = crate::install_iran_geodata().and_then(|_| setup_iran_rule_preset(&mut cfg));
        let _ = tx.send(TaskEvent::IranPreset(result));
    });
}