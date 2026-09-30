use super::input::TextInput;
use super::tasks::{self, ConnOp, PingTarget, TaskEvent};
use crate::latency::ProbeRoute;
use crate::model::{AppConfig, ProxyNode};
use crate::storage::{add_single_node, load_config, save_config, setup_iran_rule_preset};
use crate::theme::Theme;
use crate::uri::Uri;
use crate::xray::XrayRunner;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position, Rect};
use ratatui::widgets::TableState;
use std::collections::HashMap;
use std::io::Write;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

const THEME_REFRESH: Duration = Duration::from_secs(2);
const STATUS_REFRESH: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Servers,
    Routing,
    Subscriptions,
}

impl Tab {
    pub const ALL: [Tab; 3] = [Tab::Servers, Tab::Routing, Tab::Subscriptions];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Servers => "Servers",
            Tab::Routing => "Routing",
            Tab::Subscriptions => "Subscriptions",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    fn prev(self) -> Self {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortMode {
    #[default]
    Config,
    Latency,
    Name,
}

impl SortMode {
    pub fn label(self) -> &'static str {
        match self {
            SortMode::Config => "config order",
            SortMode::Latency => "latency",
            SortMode::Name => "name",
        }
    }

    fn next(self) -> Self {
        match self {
            SortMode::Config => SortMode::Latency,
            SortMode::Latency => SortMode::Name,
            SortMode::Name => SortMode::Config,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Latency {
    Untested,
    Testing,
    Ms(u64),
    Timeout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PingState {
    Testing,
    Timeout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastLevel {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub level: ToastLevel,
    pub text: String,
    created: Instant,
}

impl Toast {
    fn ttl(&self) -> Duration {
        match self.level {
            ToastLevel::Error => Duration::from_secs(8),
            ToastLevel::Warning => Duration::from_secs(6),
            _ => Duration::from_secs(4),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingDelete {
    Node(String),
    Rule(String),
    Subscription(String),
}

#[derive(Debug, Clone)]
pub struct Confirm {
    pub title: String,
    pub message: String,
    pub action: PendingDelete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    Empty,
    Share(&'static str),
    Subscription,
    Unknown,
}

pub fn detect_link(input: &str) -> LinkKind {
    let s = input.trim().to_ascii_lowercase();
    if s.is_empty() {
        return LinkKind::Empty;
    }
    for (prefix, label) in [
        ("vless://", "VLESS"),
        ("vmess://", "VMess"),
        ("trojan://", "Trojan"),
        ("ss://", "Shadowsocks"),
    ] {
        if s.starts_with(prefix) {
            return LinkKind::Share(label);
        }
    }
    if s.starts_with("https://") || s.starts_with("http://") {
        return LinkKind::Subscription;
    }
    LinkKind::Unknown
}

pub enum Overlay {
    None,
    Help,
    Add(TextInput),
    Confirm(Confirm),
}

/// Screen regions recorded during rendering so mouse events can be mapped
/// back to tabs and table rows.
#[derive(Debug, Default, Clone)]
pub struct HitMap {
    pub tabs: Vec<(Rect, Tab)>,
    pub table_body: Rect,
}

pub struct App {
    pub cfg: AppConfig,
    pub theme: Theme,
    pub tab: Tab,
    pub overlay: Overlay,
    pub servers: TableState,
    pub rules: TableState,
    pub subs: TableState,
    pub filter: TextInput,
    pub filter_editing: bool,
    pub sort: SortMode,
    pub running: bool,
    pub conn_busy: Option<ConnOp>,
    pub sub_busy: bool,
    pub ping_total: usize,
    pub ping_done: usize,
    pub toast: Option<Toast>,
    pub hit: HitMap,
    pub started: Instant,
    pub should_quit: bool,
    ping: HashMap<String, PingState>,
    restart_pending: bool,
    last_theme_check: Instant,
    last_status_check: Instant,
    tx: Sender<TaskEvent>,
    rx: Receiver<TaskEvent>,
}

impl App {
    pub fn new() -> Self {
        let mut app = Self::with_config(load_config(), Theme::load());
        app.running = XrayRunner::is_running();
        app
    }

    pub fn with_config(cfg: AppConfig, theme: Theme) -> Self {
        let (tx, rx) = mpsc::channel();
        let now = Instant::now();
        let mut app = Self {
            cfg,
            theme,
            tab: Tab::Servers,
            overlay: Overlay::None,
            servers: TableState::default(),
            rules: TableState::default(),
            subs: TableState::default(),
            filter: TextInput::default(),
            filter_editing: false,
            sort: SortMode::default(),
            running: false,
            conn_busy: None,
            sub_busy: false,
            ping_total: 0,
            ping_done: 0,
            toast: None,
            hit: HitMap::default(),
            started: now,
            should_quit: false,
            ping: HashMap::new(),
            restart_pending: false,
            last_theme_check: now,
            last_status_check: now,
            tx,
            rx,
        };
        let active = app.cfg.active_node_id.clone();
        app.reselect_node(active.as_deref());
        app.clamp_selection(Tab::Routing);
        app.clamp_selection(Tab::Subscriptions);
        app
    }

    pub fn pinging(&self) -> bool {
        self.ping_done < self.ping_total
    }

    /// True while a spinner is on screen and needs frame-rate redraws.
    pub fn animating(&self) -> bool {
        self.conn_busy.is_some() || self.sub_busy || self.pinging()
    }

    pub fn active_node(&self) -> Option<&ProxyNode> {
        let id = self.cfg.active_node_id.as_deref()?;
        self.cfg.nodes.iter().find(|n| n.id == id)
    }

    pub fn latency(&self, node: &ProxyNode) -> Latency {
        match self.ping.get(&node.id) {
            Some(PingState::Testing) => Latency::Testing,
            Some(PingState::Timeout) => Latency::Timeout,
            None => node.ping_ms.map_or(Latency::Untested, Latency::Ms),
        }
    }

    /// Indices into `cfg.nodes` after applying the filter and sort order.
    pub fn visible_nodes(&self) -> Vec<usize> {
        let terms: Vec<String> = self
            .filter
            .value()
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        let mut out: Vec<usize> = self
            .cfg
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| terms.iter().all(|t| node_matches(n, t)))
            .map(|(i, _)| i)
            .collect();
        match self.sort {
            SortMode::Config => {}
            SortMode::Name => out.sort_by_cached_key(|&i| self.cfg.nodes[i].name.to_lowercase()),
            SortMode::Latency => out.sort_by_key(|&i| match self.latency(&self.cfg.nodes[i]) {
                Latency::Ms(ms) => (0, ms),
                Latency::Testing => (1, 0),
                Latency::Untested => (2, 0),
                Latency::Timeout => (3, 0),
            }),
        }
        out
    }

    pub fn selected_node(&self) -> Option<&ProxyNode> {
        let visible = self.visible_nodes();
        let i = *visible.get(self.servers.selected()?)?;
        self.cfg.nodes.get(i)
    }

    pub fn row_count(&self, tab: Tab) -> usize {
        match tab {
            Tab::Servers => self.visible_nodes().len(),
            Tab::Routing => self.cfg.routing.rules.len(),
            Tab::Subscriptions => self.cfg.subscriptions.len(),
        }
    }

    fn table_state(&mut self, tab: Tab) -> &mut TableState {
        match tab {
            Tab::Servers => &mut self.servers,
            Tab::Routing => &mut self.rules,
            Tab::Subscriptions => &mut self.subs,
        }
    }

    fn clamp_selection(&mut self, tab: Tab) {
        let len = self.row_count(tab);
        let state = self.table_state(tab);
        match (len, state.selected()) {
            (0, _) => state.select(None),
            (_, None) => state.select(Some(0)),
            (n, Some(i)) if i >= n => state.select(Some(n - 1)),
            _ => {}
        }
    }

    fn reselect_node(&mut self, id: Option<&str>) {
        let visible = self.visible_nodes();
        let pos = id.and_then(|id| visible.iter().position(|&i| self.cfg.nodes[i].id == id));
        match pos {
            Some(p) => self.servers.select(Some(p)),
            None => self.clamp_selection(Tab::Servers),
        }
    }

    fn move_selection(&mut self, delta: isize) {
        let tab = self.tab;
        let len = self.row_count(tab);
        if len == 0 {
            return;
        }
        let state = self.table_state(tab);
        let cur = state.selected().unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, len as isize - 1) as usize;
        state.select(Some(next));
    }

    fn select_edge(&mut self, last: bool) {
        let tab = self.tab;
        let len = self.row_count(tab);
        if len > 0 {
            self.table_state(tab).select(Some(if last { len - 1 } else { 0 }));
        }
    }

    fn page(&self) -> isize {
        self.hit.table_body.height.max(1) as isize
    }

    pub fn notify(&mut self, level: ToastLevel, text: impl Into<String>) {
        self.toast = Some(Toast {
            level,
            text: text.into(),
            created: Instant::now(),
        });
    }

    fn persist(&mut self) {
        if let Err(e) = save_config(&self.cfg) {
            self.notify(ToastLevel::Error, format!("Could not save config: {e}"));
        }
    }

    /// Config mutations are rejected while a subscription sync owns the file,
    /// since the worker thread writes its own copy back when it finishes.
    fn config_locked(&mut self) -> bool {
        if self.sub_busy {
            self.notify(ToastLevel::Warning, "Subscriptions are syncing, try again in a moment");
        }
        self.sub_busy
    }

    // ----- lifecycle -------------------------------------------------------

    pub fn tick(&mut self) {
        while let Ok(event) = self.rx.try_recv() {
            self.on_task(event);
        }
        if self.last_status_check.elapsed() >= STATUS_REFRESH && self.conn_busy.is_none() {
            self.running = XrayRunner::is_running();
            self.last_status_check = Instant::now();
        }
        if self.last_theme_check.elapsed() >= THEME_REFRESH {
            self.theme.refresh();
            self.last_theme_check = Instant::now();
        }
        if self.toast.as_ref().is_some_and(|t| t.created.elapsed() >= t.ttl()) {
            self.toast = None;
        }
    }

    fn on_task(&mut self, event: TaskEvent) {
        match event {
            TaskEvent::Ping { id, ms } => {
                let selected = self.selected_node().map(|n| n.id.clone());
                match ms {
                    Some(_) => {
                        self.ping.remove(&id);
                    }
                    None => {
                        self.ping.insert(id.clone(), PingState::Timeout);
                    }
                }
                if let Some(node) = self.cfg.nodes.iter_mut().find(|n| n.id == id) {
                    node.ping_ms = ms;
                }
                self.ping_done += 1;
                if self.sort == SortMode::Latency {
                    self.reselect_node(selected.as_deref());
                }
                if !self.pinging() {
                    self.finish_ping();
                }
            }
            TaskEvent::Connection { op, result } => {
                self.conn_busy = None;
                self.running = XrayRunner::is_running();
                self.last_status_check = Instant::now();
                match result {
                    Ok(()) => {
                        let name = self.active_node().map(|n| n.name.clone()).unwrap_or_default();
                        let text = match op {
                            ConnOp::Connect => format!("Connected to {name}"),
                            ConnOp::Reconnect => format!("Reconnected to {name}"),
                            ConnOp::Disconnect => "Disconnected".to_string(),
                        };
                        self.notify(ToastLevel::Success, text);
                    }
                    Err(e) => self.notify(ToastLevel::Error, e),
                }
                if std::mem::take(&mut self.restart_pending) && self.running {
                    self.start_conn(ConnOp::Reconnect);
                }
            }
            TaskEvent::SubscriptionsUpdated(results) => {
                self.sub_busy = false;
                self.reload_config();
                let failed: Vec<&str> = results
                    .iter()
                    .filter(|(_, r)| r.is_err())
                    .map(|(n, _)| n.as_str())
                    .collect();
                let total: usize = results.iter().filter_map(|(_, r)| r.as_ref().ok()).sum();
                if results.is_empty() {
                    self.notify(ToastLevel::Info, "No subscriptions yet, press a to add one");
                } else if failed.is_empty() {
                    self.notify(
                        ToastLevel::Success,
                        format!("Synced {} subscription(s), {total} servers", results.len()),
                    );
                } else {
                    self.notify(ToastLevel::Error, format!("Sync failed for {}", failed.join(", ")));
                }
                self.request_reconnect();
            }
            TaskEvent::SubscriptionAdded(result) => {
                self.sub_busy = false;
                self.reload_config();
                match result {
                    Ok((name, count)) => {
                        self.notify(ToastLevel::Success, format!("Added subscription {name} with {count} servers"))
                    }
                    Err(e) => self.notify(ToastLevel::Error, e),
                }
            }
        }
    }

    fn finish_ping(&mut self) {
        self.persist();
        let reachable: Vec<u64> = self.cfg.nodes.iter().filter_map(|n| n.ping_ms).collect();
        let text = match reachable.iter().min() {
            Some(best) => format!(
                "Latency test done: {}/{} reachable, best {best} ms",
                reachable.len(),
                self.cfg.nodes.len()
            ),
            None => "Latency test done: no server reachable".to_string(),
        };
        let level = if reachable.is_empty() { ToastLevel::Warning } else { ToastLevel::Success };
        self.notify(level, text);
    }

    fn reload_config(&mut self) {
        let selected = self.selected_node().map(|n| n.id.clone());
        self.cfg = load_config();
        self.ping.retain(|id, _| self.cfg.nodes.iter().any(|n| &n.id == id));
        self.reselect_node(selected.as_deref());
        self.clamp_selection(Tab::Routing);
        self.clamp_selection(Tab::Subscriptions);
    }

    // ----- actions ---------------------------------------------------------

    fn start_conn(&mut self, op: ConnOp) {
        if self.conn_busy.is_some() {
            return;
        }
        self.conn_busy = Some(op);
        tasks::spawn_connection(self.tx.clone(), op, self.cfg.clone());
    }

    /// Applies config changes to a live session, coalescing bursts of edits
    /// into a single restart once the in-flight one completes.
    fn request_reconnect(&mut self) {
        if self.conn_busy.is_some() {
            self.restart_pending = true;
        } else if self.running {
            self.start_conn(ConnOp::Reconnect);
        }
    }

    fn toggle_connection(&mut self) {
        if self.conn_busy.is_some() {
            return;
        }
        if self.running {
            self.start_conn(ConnOp::Disconnect);
        } else if self.cfg.nodes.is_empty() {
            self.notify(ToastLevel::Warning, "No servers yet, press a to add one");
        } else {
            self.start_conn(ConnOp::Connect);
        }
    }

    fn disconnect(&mut self) {
        if self.running && self.conn_busy.is_none() {
            self.start_conn(ConnOp::Disconnect);
        }
    }

    fn connect_selected(&mut self) {
        let Some(node) = self.selected_node() else { return };
        let (id, name) = (node.id.clone(), node.name.clone());
        if self.running && self.cfg.active_node_id.as_deref() == Some(id.as_str()) {
            self.notify(ToastLevel::Info, format!("Already connected to {name}"));
            return;
        }
        if self.conn_busy.is_some() || self.config_locked() {
            return;
        }
        self.cfg.active_node_id = Some(id);
        self.persist();
        self.start_conn(if self.running { ConnOp::Reconnect } else { ConnOp::Connect });
    }

    fn toggle_tun(&mut self) {
        if self.config_locked() {
            return;
        }
        self.cfg.tun.enabled = !self.cfg.tun.enabled;
        self.persist();
        let state = if self.cfg.tun.enabled { "on" } else { "off" };
        self.notify(ToastLevel::Info, format!("TUN mode {state}"));
        self.request_reconnect();
    }

    fn start_ping(&mut self) {
        if self.pinging() {
            return;
        }
        if self.sub_busy {
            self.notify(ToastLevel::Warning, "Subscriptions are syncing, try again in a moment");
            return;
        }
        if self.cfg.nodes.is_empty() {
            self.notify(ToastLevel::Warning, "No servers to test");
            return;
        }
        let targets: Vec<PingTarget> = self
            .cfg
            .nodes
            .iter()
            .map(|n| PingTarget {
                id: n.id.clone(),
                host: n.server.clone(),
                port: n.port,
            })
            .collect();
        for t in &targets {
            self.ping.insert(t.id.clone(), PingState::Testing);
        }
        self.ping_total = targets.len();
        self.ping_done = 0;
        tasks::spawn_ping(self.tx.clone(), targets, ProbeRoute::detect(&self.cfg.tun.name));
    }

    fn update_subscriptions(&mut self) {
        if self.sub_busy || self.pinging() {
            return;
        }
        if self.cfg.subscriptions.is_empty() {
            self.notify(ToastLevel::Info, "No subscriptions yet, press a to add one");
            return;
        }
        self.sub_busy = true;
        tasks::spawn_update_subscriptions(self.tx.clone(), self.cfg.clone());
    }

    fn cycle_sort(&mut self) {
        let selected = self.selected_node().map(|n| n.id.clone());
        self.sort = self.sort.next();
        self.reselect_node(selected.as_deref());
        self.notify(ToastLevel::Info, format!("Sorted by {}", self.sort.label()));
    }

    fn copy_selected_link(&mut self) {
        let Some(node) = self.selected_node() else { return };
        let (name, link) = (node.name.clone(), node.raw_link.clone());
        // OSC 52 asks the terminal emulator to set the clipboard, which also
        // works over SSH without a local clipboard daemon.
        let seq = format!("\x1b]52;c;{}\x07", STANDARD.encode(link));
        let mut out = std::io::stdout();
        match out.write_all(seq.as_bytes()).and_then(|_| out.flush()) {
            Ok(()) => self.notify(ToastLevel::Success, format!("Copied share link for {name}")),
            Err(e) => self.notify(ToastLevel::Error, format!("Copy failed: {e}")),
        }
    }

    fn submit_add(&mut self, raw: String) {
        let link = raw.trim().to_string();
        match detect_link(&link) {
            LinkKind::Empty => {}
            LinkKind::Unknown => self.notify(
                ToastLevel::Error,
                "Unrecognized link, expected vless://, vmess://, trojan://, ss:// or an https:// URL",
            ),
            LinkKind::Share(_) => {
                if self.config_locked() {
                    return;
                }
                match add_single_node(&mut self.cfg, &link) {
                    Ok(node) => {
                        self.tab = Tab::Servers;
                        self.filter.clear();
                        self.reselect_node(Some(&node.id));
                        self.notify(ToastLevel::Success, format!("Added {}", node.name));
                    }
                    Err(e) => self.notify(ToastLevel::Error, e),
                }
            }
            LinkKind::Subscription => {
                if self.sub_busy {
                    self.notify(ToastLevel::Warning, "A subscription sync is already running");
                    return;
                }
                let name = Uri::parse(&link).map(|u| u.host_str()).unwrap_or_else(|| "Subscription".to_string());
                self.sub_busy = true;
                self.tab = Tab::Subscriptions;
                tasks::spawn_add_subscription(self.tx.clone(), self.cfg.clone(), link, name);
            }
        }
    }

    fn ask_delete(&mut self) {
        let confirm = match self.tab {
            Tab::Servers => self.selected_node().map(|n| Confirm {
                title: "Delete server".into(),
                message: format!("Remove “{}” from your server list?", n.name),
                action: PendingDelete::Node(n.id.clone()),
            }),
            Tab::Routing => self.rules.selected().and_then(|i| self.cfg.routing.rules.get(i)).map(|r| Confirm {
                title: "Delete rule".into(),
                message: format!("Remove routing rule “{}”?", r.name),
                action: PendingDelete::Rule(r.id.clone()),
            }),
            Tab::Subscriptions => self.subs.selected().and_then(|i| self.cfg.subscriptions.get(i)).map(|s| Confirm {
                title: "Delete subscription".into(),
                message: format!("Remove “{}” and its {} servers?", s.name, s.node_count),
                action: PendingDelete::Subscription(s.id.clone()),
            }),
        };
        if let Some(c) = confirm {
            self.overlay = Overlay::Confirm(c);
        }
    }

    fn perform_delete(&mut self, action: PendingDelete) {
        if self.config_locked() {
            return;
        }
        let active_before = self.cfg.active_node_id.clone();
        let text = match action {
            PendingDelete::Node(id) => {
                let name = self.cfg.nodes.iter().find(|n| n.id == id).map(|n| n.name.clone());
                self.cfg.nodes.retain(|n| n.id != id);
                format!("Removed {}", name.unwrap_or_default())
            }
            PendingDelete::Rule(id) => {
                let name = self.cfg.routing.rules.iter().find(|r| r.id == id).map(|r| r.name.clone());
                self.cfg.routing.rules.retain(|r| r.id != id);
                self.request_reconnect();
                format!("Removed rule {}", name.unwrap_or_default())
            }
            PendingDelete::Subscription(id) => {
                let name = self.cfg.subscriptions.iter().find(|s| s.id == id).map(|s| s.name.clone());
                self.cfg.subscriptions.retain(|s| s.id != id);
                self.cfg.nodes.retain(|n| n.subscription_id.as_deref() != Some(id.as_str()));
                format!("Removed subscription {}", name.unwrap_or_default())
            }
        };
        if self.active_node().is_none() {
            self.cfg.active_node_id = self.cfg.nodes.first().map(|n| n.id.clone());
        }
        self.persist();
        if self.cfg.active_node_id != active_before {
            self.request_reconnect();
        }
        for tab in Tab::ALL {
            self.clamp_selection(tab);
        }
        self.notify(ToastLevel::Info, text);
    }

    fn toggle_rule(&mut self) {
        let Some(i) = self.rules.selected() else { return };
        if self.config_locked() {
            return;
        }
        let Some(rule) = self.cfg.routing.rules.get_mut(i) else { return };
        rule.enabled = !rule.enabled;
        let text = format!("{} {}", rule.name, if rule.enabled { "enabled" } else { "disabled" });
        self.persist();
        self.notify(ToastLevel::Info, text);
        self.request_reconnect();
    }

    fn install_preset(&mut self) {
        if self.config_locked() {
            return;
        }
        match setup_iran_rule_preset(&mut self.cfg) {
            Ok(()) => {
                self.clamp_selection(Tab::Routing);
                self.notify(ToastLevel::Success, "Iran bypass preset installed");
                self.request_reconnect();
            }
            Err(e) => self.notify(ToastLevel::Error, e),
        }
    }

    // ----- input -----------------------------------------------------------

    pub fn on_event(&mut self, event: Event) {
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => self.on_key(key),
            Event::Mouse(mouse) => self.on_mouse(mouse),
            Event::Paste(text) => self.on_paste(&text),
            _ => {}
        }
    }

    fn on_paste(&mut self, text: &str) {
        match &mut self.overlay {
            Overlay::Add(input) => input.insert_str(text.trim()),
            Overlay::None if self.filter_editing => {
                self.filter.insert_str(text.trim());
                self.reselect_node(None);
            }
            Overlay::None => {
                let mut input = TextInput::default();
                input.insert_str(text.trim());
                self.overlay = Overlay::Add(input);
            }
            _ => {}
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }

        match std::mem::replace(&mut self.overlay, Overlay::None) {
            Overlay::None => {}
            Overlay::Help => {
                if !matches!(key.code, KeyCode::Esc | KeyCode::Char('?' | 'q') | KeyCode::Enter) {
                    self.overlay = Overlay::Help;
                }
                return;
            }
            Overlay::Confirm(c) => {
                match key.code {
                    KeyCode::Char('y' | 'Y') | KeyCode::Enter => self.perform_delete(c.action),
                    KeyCode::Char('n' | 'N' | 'q') | KeyCode::Esc => {}
                    _ => self.overlay = Overlay::Confirm(c),
                }
                return;
            }
            Overlay::Add(mut input) => {
                match key.code {
                    KeyCode::Esc => return,
                    KeyCode::Enter => {
                        self.submit_add(input.value().to_string());
                        return;
                    }
                    _ => edit_input(&mut input, key),
                }
                self.overlay = Overlay::Add(input);
                return;
            }
        }

        if self.filter_editing {
            match key.code {
                KeyCode::Esc => {
                    self.filter_editing = false;
                    self.filter.clear();
                    let id = self.selected_node().map(|n| n.id.clone());
                    self.reselect_node(id.as_deref());
                }
                KeyCode::Enter => self.filter_editing = false,
                KeyCode::Up => self.move_selection(-1),
                KeyCode::Down => self.move_selection(1),
                _ => {
                    edit_input(&mut self.filter, key);
                    self.servers.select(Some(0));
                    self.clamp_selection(Tab::Servers);
                }
            }
            return;
        }

        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') => self.overlay = Overlay::Help,
            KeyCode::Tab => self.tab = self.tab.next(),
            KeyCode::BackTab => self.tab = self.tab.prev(),
            KeyCode::Char('1') => self.tab = Tab::Servers,
            KeyCode::Char('2') => self.tab = Tab::Routing,
            KeyCode::Char('3') => self.tab = Tab::Subscriptions,
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::PageDown => self.move_selection(self.page()),
            KeyCode::PageUp => self.move_selection(-self.page()),
            KeyCode::Char('d') if ctrl => self.move_selection(self.page() / 2),
            KeyCode::Char('u') if ctrl => self.move_selection(-self.page() / 2),
            KeyCode::Home | KeyCode::Char('g') => self.select_edge(false),
            KeyCode::End | KeyCode::Char('G') => self.select_edge(true),
            KeyCode::Char('c') => self.toggle_connection(),
            KeyCode::Char('x') => self.disconnect(),
            KeyCode::Char('t') => self.toggle_tun(),
            KeyCode::Char('a') => self.overlay = Overlay::Add(TextInput::default()),
            KeyCode::Char('u') => self.update_subscriptions(),
            KeyCode::Char('p') => self.start_ping(),
            KeyCode::Char('d') | KeyCode::Delete => self.ask_delete(),
            _ => self.on_tab_key(key),
        }
    }

    fn on_tab_key(&mut self, key: KeyEvent) {
        match (self.tab, key.code) {
            (Tab::Servers, KeyCode::Enter) => self.connect_selected(),
            (Tab::Servers, KeyCode::Char(' ')) => self.toggle_connection(),
            (Tab::Servers, KeyCode::Char('/')) => self.filter_editing = true,
            (Tab::Servers, KeyCode::Char('s')) => self.cycle_sort(),
            (Tab::Servers, KeyCode::Char('y')) => self.copy_selected_link(),
            (Tab::Servers, KeyCode::Esc) if !self.filter.is_empty() => {
                let id = self.selected_node().map(|n| n.id.clone());
                self.filter.clear();
                self.reselect_node(id.as_deref());
            }
            (Tab::Routing, KeyCode::Enter | KeyCode::Char(' ')) => self.toggle_rule(),
            (Tab::Routing, KeyCode::Char('i')) => self.install_preset(),
            (Tab::Subscriptions, KeyCode::Enter) => self.update_subscriptions(),
            _ => {}
        }
    }

    fn on_mouse(&mut self, mouse: MouseEvent) {
        if !matches!(self.overlay, Overlay::None) {
            return;
        }
        let pos = Position::new(mouse.column, mouse.row);
        match mouse.kind {
            MouseEventKind::ScrollDown => self.move_selection(1),
            MouseEventKind::ScrollUp => self.move_selection(-1),
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some((_, tab)) = self.hit.tabs.iter().find(|(r, _)| r.contains(pos)) {
                    self.tab = *tab;
                    return;
                }
                let body = self.hit.table_body;
                if body.contains(pos) {
                    let tab = self.tab;
                    let len = self.row_count(tab);
                    let state = self.table_state(tab);
                    let row = state.offset() + (pos.y - body.y) as usize;
                    if row < len {
                        if tab == Tab::Servers && state.selected() == Some(row) {
                            self.connect_selected();
                        } else {
                            state.select(Some(row));
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

fn edit_input(input: &mut TextInput, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('w') if ctrl => input.delete_word(),
        KeyCode::Char('u') if ctrl => input.clear(),
        KeyCode::Char('a') if ctrl => input.home(),
        KeyCode::Char('e') if ctrl => input.end(),
        KeyCode::Char(c) if !ctrl => input.insert(c),
        KeyCode::Backspace => input.backspace(),
        KeyCode::Delete => input.delete(),
        KeyCode::Left => input.left(),
        KeyCode::Right => input.right(),
        KeyCode::Home => input.home(),
        KeyCode::End => input.end(),
        _ => {}
    }
}

/// `term` is already lowercase.
fn node_matches(node: &ProxyNode, term: &str) -> bool {
    [
        node.name.as_str(),
        node.server.as_str(),
        node.network.as_str(),
        node.security.as_str(),
        node.protocol.label(),
    ]
    .iter()
    .any(|f| contains_lowercase(f, term))
}

/// `haystack.to_lowercase().contains(needle)` without allocating for the
/// common all-ASCII case; this runs for every node on every frame while a
/// filter is active.
fn contains_lowercase(haystack: &str, needle: &str) -> bool {
    if !haystack.is_ascii() {
        return haystack.to_lowercase().contains(needle);
    }
    let (h, n) = (haystack.as_bytes(), needle.as_bytes());
    n.len() <= h.len() && h.windows(n.len()).any(|w| w.eq_ignore_ascii_case(n))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Protocol;

    #[test]
    fn lowercase_contains_matches_the_allocating_version() {
        for hay in ["Germany-01", "VLESS", "آلمان Berlin", "İstanbul", "ÄÖÜ", "", "abc"] {
            for needle in ["germany", "01", "vl", "آلمان", "berlin", "i̇st", "äö", "x", "abcd", "c"] {
                assert_eq!(contains_lowercase(hay, needle), hay.to_lowercase().contains(needle), "{hay} / {needle}");
            }
        }
    }

    pub fn node(id: &str, name: &str, ping: Option<u64>) -> ProxyNode {
        ProxyNode {
            id: id.into(),
            name: name.into(),
            protocol: Protocol::Vless,
            server: format!("{id}.example.com"),
            port: 443,
            secret: String::new(),
            cipher: None,
            network: "ws".into(),
            path: None,
            host: None,
            service_name: None,
            security: "tls".into(),
            sni: None,
            alpn: None,
            fingerprint: None,
            pbk: None,
            sid: None,
            spider_x: None,
            flow: None,
            raw_link: format!("vless://{id}"),
            subscription_id: None,
            ping_ms: ping,
        }
    }

    fn app() -> App {
        let cfg = AppConfig {
            nodes: vec![
                node("a", "DE1 Frankfurt", Some(180)),
                node("b", "FI1 Helsinki", Some(40)),
                node("c", "NL1 Amsterdam", None),
            ],
            active_node_id: Some("b".into()),
            ..AppConfig::default()
        };
        App::with_config(cfg, Theme::load())
    }

    fn press(app: &mut App, code: KeyCode) {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    #[test]
    fn starts_on_active_node() {
        let app = app();
        assert_eq!(app.selected_node().map(|n| n.id.as_str()), Some("b"));
    }

    #[test]
    fn filter_narrows_rows_and_esc_restores() {
        let mut app = app();
        press(&mut app, KeyCode::Char('/'));
        for c in "hel".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        assert_eq!(app.visible_nodes(), vec![1]);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.visible_nodes().len(), 3);
        assert!(!app.filter_editing);
    }

    #[test]
    fn esc_clears_applied_filter_and_keeps_selection() {
        let mut app = app();
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('n'));
        press(&mut app, KeyCode::Char('l'));
        press(&mut app, KeyCode::Enter);
        assert!(!app.filter_editing);
        assert_eq!(app.visible_nodes(), vec![2]);
        press(&mut app, KeyCode::Esc);
        assert!(app.filter.is_empty());
        assert_eq!(app.selected_node().map(|n| n.id.as_str()), Some("c"));
    }

    #[test]
    fn filter_terms_are_anded() {
        let mut app = app();
        app.filter.insert_str("vless ams");
        assert_eq!(app.visible_nodes(), vec![2]);
    }

    #[test]
    fn latency_sort_keeps_selection_and_puts_untested_last() {
        let mut app = app();
        press(&mut app, KeyCode::Char('s'));
        assert_eq!(app.sort, SortMode::Latency);
        assert_eq!(app.visible_nodes(), vec![1, 0, 2]);
        assert_eq!(app.selected_node().map(|n| n.id.as_str()), Some("b"));
    }

    #[test]
    fn navigation_clamps_at_edges() {
        let mut app = app();
        press(&mut app, KeyCode::Char('G'));
        press(&mut app, KeyCode::Down);
        assert_eq!(app.servers.selected(), Some(2));
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Up);
        assert_eq!(app.servers.selected(), Some(0));
    }

    #[test]
    fn delete_requires_confirmation() {
        let mut app = app();
        app.sub_busy = true;
        press(&mut app, KeyCode::Char('d'));
        assert!(matches!(app.overlay, Overlay::Confirm(_)));
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.overlay, Overlay::None));
        assert_eq!(app.cfg.nodes.len(), 3);
    }

    #[test]
    fn esc_does_not_quit() {
        let mut app = app();
        press(&mut app, KeyCode::Esc);
        assert!(!app.should_quit);
        press(&mut app, KeyCode::Char('q'));
        assert!(app.should_quit);
    }

    #[test]
    fn detects_link_kinds() {
        assert_eq!(detect_link("  "), LinkKind::Empty);
        assert_eq!(detect_link("VLESS://abc"), LinkKind::Share("VLESS"));
        assert_eq!(detect_link("ss://abc"), LinkKind::Share("Shadowsocks"));
        assert_eq!(detect_link("https://sub.example/x"), LinkKind::Subscription);
        assert_eq!(detect_link("hello"), LinkKind::Unknown);
    }
}
