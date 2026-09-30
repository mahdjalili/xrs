use crate::latency::ProbeRoute;
use crate::model::AppConfig;
use crate::storage::{add_subscription, update_all_subscriptions};
use crate::xray::XrayRunner;
use std::collections::VecDeque;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;

const PING_WORKERS: usize = 16;

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
}

pub struct PingTarget {
    pub id: String,
    pub host: String,
    pub port: u16,
}

pub fn spawn_connection(tx: Sender<TaskEvent>, op: ConnOp, cfg: AppConfig) {
    thread::spawn(move || {
        let result = match op {
            ConnOp::Connect => XrayRunner::start(&cfg).map(|_| ()),
            ConnOp::Disconnect => XrayRunner::stop(),
            ConnOp::Reconnect => XrayRunner::restart(&cfg).map(|_| ()),
        };
        let _ = tx.send(TaskEvent::Connection { op, result });
    });
}

/// Probes every target concurrently with a small worker pool, streaming one
/// event per node so the table fills in live instead of after the slowest host.
pub fn spawn_ping(tx: Sender<TaskEvent>, targets: Vec<PingTarget>, route: ProbeRoute) {
    let workers = PING_WORKERS.min(targets.len());
    let queue = Arc::new(Mutex::new(targets.into_iter().collect::<VecDeque<_>>()));
    let route = Arc::new(route);
    for _ in 0..workers {
        let queue = Arc::clone(&queue);
        let route = Arc::clone(&route);
        let tx = tx.clone();
        thread::spawn(move || {
            loop {
                let job = match queue.lock() {
                    Ok(mut q) => q.pop_front(),
                    Err(_) => None,
                };
                let Some(target) = job else { break };
                let ms = route.tcp_latency(&target.host, target.port);
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