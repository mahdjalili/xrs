use crate::model::AppConfig;
use crate::storage::{
    add_single_node, load_config, save_config, setup_iran_rule_preset, update_all_subscriptions,
};
use crate::theme::Theme;
use crate::xray::XrayRunner;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame, Terminal,
};
use std::io;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

pub fn run_tui() -> Result<(), Box<dyn std::error::Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = tui_loop(&mut terminal);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        eprintln!("TUI Error: {err:?}");
    }

    Ok(())
}

enum InputMode {
    Normal,
    AddConfig,
    ManageRoutes,
}

fn tui_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<(), Box<dyn std::error::Error>> {
    let mut cfg = load_config();
    let mut theme = Theme::load();
    let mut list_state = ListState::default();
    if !cfg.nodes.is_empty() {
        list_state.select(Some(0));
    }
    let mut route_list_state = ListState::default();
    if !cfg.routing.rules.is_empty() {
        route_list_state.select(Some(0));
    }

    let mut input_mode = InputMode::Normal;
    let mut input_buffer = String::new();
    let mut status_message = String::from("Ready");
    let mut last_theme_check = Instant::now();

    loop {
        // Periodically refresh theme if theme changed
        if last_theme_check.elapsed() > Duration::from_secs(2) {
            theme = Theme::load();
            last_theme_check = Instant::now();
        }

        let is_running = XrayRunner::is_running();

        terminal.draw(|f| {
            render_ui(
                f,
                &cfg,
                &theme,
                &mut list_state,
                &mut route_list_state,
                is_running,
                &status_message,
                &input_mode,
                &input_buffer,
            );
        })?;

        if event::poll(Duration::from_millis(200))?
            && let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                match input_mode {
                    InputMode::Normal => match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Char('x') => {
                            let _ = XrayRunner::stop();
                            status_message = "xrs stopped".to_string();
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            if !cfg.nodes.is_empty() {
                                let i = match list_state.selected() {
                                    Some(i) => (i + 1) % cfg.nodes.len(),
                                    None => 0,
                                };
                                list_state.select(Some(i));
                            }
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            if !cfg.nodes.is_empty() {
                                let i = match list_state.selected() {
                                    Some(i) => {
                                        if i == 0 {
                                            cfg.nodes.len() - 1
                                        } else {
                                            i - 1
                                        }
                                    }
                                    None => 0,
                                };
                                list_state.select(Some(i));
                            }
                        }
                        KeyCode::Enter => {
                            if let Some(i) = list_state.selected()
                                && let Some(node) = cfg.nodes.get(i) {
                                    cfg.active_node_id = Some(node.id.clone());
                                    let _ = save_config(&cfg);
                                    if is_running {
                                        match XrayRunner::restart(&cfg) {
                                            Ok(_) => status_message = format!("Switched to: {}", node.name),
                                            Err(e) => status_message = format!("Restart error: {e}"),
                                        }
                                    } else {
                                        match XrayRunner::start(&cfg) {
                                            Ok(_) => status_message = format!("Connected to: {}", node.name),
                                            Err(e) => status_message = format!("Start error: {e}"),
                                        }
                                    }
                                }
                        }
                        KeyCode::Char(' ') => {
                            if is_running {
                                let _ = XrayRunner::stop();
                                status_message = "Disconnected".to_string();
                            } else {
                                match XrayRunner::start(&cfg) {
                                    Ok(_) => status_message = "Connected".to_string(),
                                    Err(e) => status_message = format!("Failed to start: {e}"),
                                }
                            }
                        }
                        KeyCode::Char('t') => {
                            cfg.tun.enabled = !cfg.tun.enabled;
                            let _ = save_config(&cfg);
                            if is_running {
                                let _ = XrayRunner::restart(&cfg);
                            }
                            status_message = format!(
                                "TUN Mode: {}",
                                if cfg.tun.enabled { "ON" } else { "OFF" }
                            );
                        }
                        KeyCode::Char('r') => {
                            input_mode = InputMode::ManageRoutes;
                            if route_list_state.selected().is_none() && !cfg.routing.rules.is_empty() {
                                route_list_state.select(Some(0));
                            }
                        }
                        KeyCode::Char('p') => {
                            let _ = terminal.draw(|f| {
                                render_ui(
                                    f,
                                    &cfg,
                                    &theme,
                                    &mut list_state,
                                    &mut route_list_state,
                                    is_running,
                                    "Testing node latencies...",
                                    &input_mode,
                                    &input_buffer,
                                );
                            });
                            for node in &mut cfg.nodes {
                                let start = Instant::now();
                                node.ping_ms = None;
                                if let Ok(mut addrs) = (node.server.as_str(), node.port).to_socket_addrs()
                                    && let Some(addr) = addrs.next()
                                        && TcpStream::connect_timeout(&addr, Duration::from_millis(1500)).is_ok() {
                                            node.ping_ms = Some(start.elapsed().as_millis() as u64);
                                        }
                            }
                            let _ = save_config(&cfg);
                            status_message = "Latency test finished".to_string();
                        }
                        KeyCode::Char('u') => {
                            let _ = terminal.draw(|f| {
                                render_ui(
                                    f,
                                    &cfg,
                                    &theme,
                                    &mut list_state,
                                    &mut route_list_state,
                                    is_running,
                                    "Updating subscriptions...",
                                    &input_mode,
                                    &input_buffer,
                                );
                            });
                            let results = update_all_subscriptions(&mut cfg);
                            cfg = load_config();
                            status_message = format!("Updated {} subscriptions", results.len());
                        }
                        KeyCode::Char('a') => {
                            input_mode = InputMode::AddConfig;
                            input_buffer.clear();
                        }
                        _ => {}
                    },
                    InputMode::AddConfig => match key.code {
                        KeyCode::Enter => {
                            if !input_buffer.trim().is_empty() {
                                match add_single_node(&mut cfg, input_buffer.trim()) {
                                    Ok(n) => {
                                        cfg = load_config();
                                        status_message = format!("Added config: {}", n.name);
                                    }
                                    Err(e) => {
                                        status_message = format!("Add error: {e}");
                                    }
                                }
                            }
                            input_mode = InputMode::Normal;
                        }
                        KeyCode::Esc => {
                            input_mode = InputMode::Normal;
                        }
                        KeyCode::Backspace => {
                            input_buffer.pop();
                        }
                        KeyCode::Char(c) => {
                            input_buffer.push(c);
                        }
                        _ => {}
                    },
                    InputMode::ManageRoutes => match key.code {
                        KeyCode::Esc => {
                            input_mode = InputMode::Normal;
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            if !cfg.routing.rules.is_empty() {
                                let i = match route_list_state.selected() {
                                    Some(i) => (i + 1) % cfg.routing.rules.len(),
                                    None => 0,
                                };
                                route_list_state.select(Some(i));
                            }
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            if !cfg.routing.rules.is_empty() {
                                let i = match route_list_state.selected() {
                                    Some(i) => {
                                        if i == 0 {
                                            cfg.routing.rules.len() - 1
                                        } else {
                                            i - 1
                                        }
                                    }
                                    None => 0,
                                };
                                route_list_state.select(Some(i));
                            }
                        }
                        KeyCode::Enter | KeyCode::Char(' ') => {
                            if let Some(i) = route_list_state.selected() {
                                let info = if let Some(rule) = cfg.routing.rules.get_mut(i) {
                                    rule.enabled = !rule.enabled;
                                    Some((rule.enabled, rule.name.clone()))
                                } else {
                                    None
                                };

                                if let Some((new_state, name)) = info {
                                    let _ = save_config(&cfg);
                                    if is_running {
                                        let _ = XrayRunner::restart(&cfg);
                                    }
                                    status_message = format!(
                                        "Rule '{}': {}",
                                        name,
                                        if new_state { "ENABLED" } else { "DISABLED" }
                                    );
                                }
                            }
                        }
                        KeyCode::Char('i') => {
                            let _ = setup_iran_rule_preset(&mut cfg);
                            if is_running {
                                let _ = XrayRunner::restart(&cfg);
                            }
                            status_message = "Iran rule preset installed & enabled".to_string();
                        }
                        KeyCode::Char('d') => {
                            if let Some(i) = route_list_state.selected()
                                && i < cfg.routing.rules.len() {
                                    let removed = cfg.routing.rules.remove(i);
                                    let _ = save_config(&cfg);
                                    if is_running {
                                        let _ = XrayRunner::restart(&cfg);
                                    }
                                    status_message = format!("Removed rule '{}'", removed.name);
                                    if !cfg.routing.rules.is_empty() {
                                        route_list_state.select(Some(0));
                                    }
                                }
                        }
                        _ => {}
                    },
                }
            }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn render_ui(
    f: &mut Frame,
    cfg: &AppConfig,
    theme: &Theme,
    list_state: &mut ListState,
    route_list_state: &mut ListState,
    is_running: bool,
    status_msg: &str,
    input_mode: &InputMode,
    input_buffer: &str,
) {
    let size = f.area();

    // Outer layout: Header, Node List, Footer
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Min(8),
            Constraint::Length(4),
        ])
        .split(size);

    let accent = theme.accent();
    let bg = theme.bg();
    let dark_bg = theme.dark_bg();
    let fg = theme.fg();
    let muted = theme.muted();
    let green = theme.green();
    let red = theme.red();

    // 1. Header
    let status_span = if is_running {
        Span::styled(" ● CONNECTED ", Style::default().fg(Color::Black).bg(green).add_modifier(Modifier::BOLD))
    } else {
        Span::styled(" ○ DISCONNECTED ", Style::default().fg(fg).bg(red).add_modifier(Modifier::BOLD))
    };

    let tun_span = if cfg.tun.enabled {
        Span::styled(" [TUN: ON] ", Style::default().fg(Color::Black).bg(accent).add_modifier(Modifier::BOLD))
    } else {
        Span::styled(" [TUN: OFF] ", Style::default().fg(muted))
    };

    let active_node_name = if let Some(ref id) = cfg.active_node_id {
        cfg.nodes.iter().find(|n| &n.id == id).map(|n| n.name.as_str()).unwrap_or("None")
    } else {
        "None"
    };

    let active_rules_count = cfg.routing.rules.iter().filter(|r| r.enabled).count();

    let header_lines = vec![
        Line::from(vec![
            Span::styled(" xrs ", Style::default().fg(accent).add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            status_span,
            Span::raw(" "),
            tun_span,
            Span::raw("  Active Node: "),
            Span::styled(active_node_name, Style::default().fg(accent).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled(format!(" SOCKS: 127.0.0.1:{} ", cfg.inbounds.socks_port), Style::default().fg(fg)),
            Span::raw(" | "),
            Span::styled(format!("HTTP: 127.0.0.1:{} ", cfg.inbounds.http_port), Style::default().fg(fg)),
            Span::raw(" | "),
            Span::styled("Active Rules: ", Style::default().fg(muted)),
            Span::styled(format!("{}/{} [R]", active_rules_count, cfg.routing.rules.len()), Style::default().fg(accent)),
            Span::raw(" | "),
            Span::styled(format!("Nodes: {}", cfg.nodes.len()), Style::default().fg(accent)),
        ]),
    ];

    let header_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(accent))
        .border_type(BorderType::Rounded)
        .style(Style::default().bg(dark_bg));

    let header_p = Paragraph::new(header_lines)
        .block(header_block)
        .alignment(Alignment::Left);

    f.render_widget(header_p, chunks[0]);

    // 2. Node List
    let items: Vec<ListItem> = cfg
        .nodes
        .iter()
        .enumerate()
        .map(|(idx, node)| {
            let is_active = cfg.active_node_id.as_deref() == Some(&node.id);
            let marker = if is_active { "★ " } else { "  " };

            let ping_str = match node.ping_ms {
                Some(ms) => format!("{ms}ms"),
                None => "---".to_string(),
            };

            let line = Line::from(vec![
                Span::styled(marker, Style::default().fg(if is_active { accent } else { muted })),
                Span::styled(format!("{:2}. ", idx + 1), Style::default().fg(muted)),
                Span::styled(
                    format!("[{:<5}] ", node.protocol.to_string()),
                    Style::default().fg(accent).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("{:<30} ", node.name), Style::default().fg(fg)),
                Span::styled(
                    format!("{}:{} ", node.server, node.port),
                    Style::default().fg(muted),
                ),
                Span::styled(
                    format!("({}) ", node.network),
                    Style::default().fg(theme.blue()),
                ),
                Span::styled(format!("{ping_str:>6}"), Style::default().fg(if node.ping_ms.is_some() { green } else { muted })),
            ]);

            ListItem::new(line)
        })
        .collect();

    let list_block = Block::default()
        .title(Span::styled(" Proxy Nodes ", Style::default().fg(accent).add_modifier(Modifier::BOLD)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if is_running { accent } else { muted }))
        .border_type(BorderType::Rounded)
        .style(Style::default().bg(bg));

    let list = List::new(items)
        .block(list_block)
        .highlight_style(
            Style::default()
                .bg(theme.lighter_bg())
                .fg(theme.bright_fg())
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    f.render_stateful_widget(list, chunks[1], list_state);

    // 3. Footer / Help & Status
    let footer_text = vec![
        Line::from(vec![
            Span::styled("[Enter] ", Style::default().fg(accent).add_modifier(Modifier::BOLD)),
            Span::raw("Select  "),
            Span::styled("[Space] ", Style::default().fg(accent).add_modifier(Modifier::BOLD)),
            Span::raw("Toggle  "),
            Span::styled("[T] ", Style::default().fg(accent).add_modifier(Modifier::BOLD)),
            Span::raw("TUN  "),
            Span::styled("[R] ", Style::default().fg(accent).add_modifier(Modifier::BOLD)),
            Span::raw("Routes  "),
            Span::styled("[P] ", Style::default().fg(accent).add_modifier(Modifier::BOLD)),
            Span::raw("Ping  "),
            Span::styled("[U] ", Style::default().fg(accent).add_modifier(Modifier::BOLD)),
            Span::raw("Update  "),
            Span::styled("[A] ", Style::default().fg(accent).add_modifier(Modifier::BOLD)),
            Span::raw("Add  "),
            Span::styled("[X] ", Style::default().fg(accent).add_modifier(Modifier::BOLD)),
            Span::raw("Stop  "),
            Span::styled("[Q] ", Style::default().fg(accent).add_modifier(Modifier::BOLD)),
            Span::raw("Quit"),
        ]),
        Line::from(vec![
            Span::styled("Status: ", Style::default().fg(muted)),
            Span::styled(status_msg, Style::default().fg(accent)),
        ]),
    ];

    let footer_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(muted))
        .border_type(BorderType::Rounded)
        .style(Style::default().bg(dark_bg));

    let footer_p = Paragraph::new(footer_text).block(footer_block);
    f.render_widget(footer_p, chunks[2]);

    // Popup modal if in AddConfig mode
    if let InputMode::AddConfig = input_mode {
        let area = centered_rect(70, 20, size);
        f.render_widget(Clear, area);

        let popup_block = Block::default()
            .title(Span::styled(" Add Single Config (vless://, vmess://, trojan://, ss://) ", Style::default().fg(accent).add_modifier(Modifier::BOLD)))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(accent))
            .border_type(BorderType::Double)
            .style(Style::default().bg(dark_bg));

        let input_p = Paragraph::new(vec![
            Line::from("Paste proxy URI below and press Enter (Esc to cancel):"),
            Line::from(""),
            Line::from(Span::styled(input_buffer, Style::default().fg(accent))),
        ])
        .block(popup_block);

        f.render_widget(input_p, area);
    }

    // Popup modal if in ManageRoutes mode
    if let InputMode::ManageRoutes = input_mode {
        let area = centered_rect(80, 70, size);
        f.render_widget(Clear, area);

        let popup_block = Block::default()
            .title(Span::styled(" Routing Rules Manager ", Style::default().fg(accent).add_modifier(Modifier::BOLD)))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(accent))
            .border_type(BorderType::Double)
            .style(Style::default().bg(dark_bg));

        let inner_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(6),
                Constraint::Length(2),
            ])
            .split(popup_block.inner(area));

        let route_items: Vec<ListItem> = cfg
            .routing
            .rules
            .iter()
            .map(|r| {
                let check = if r.enabled { "[✓] " } else { "[ ] " };
                let check_style = if r.enabled {
                    Style::default().fg(accent).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(muted)
                };

                let counts = format!(
                    "(Direct: {}d/{}ip | Block: {}d/{}ip)",
                    r.direct_domains.len(),
                    r.direct_ips.len(),
                    r.block_domains.len(),
                    r.block_ips.len()
                );

                let line = Line::from(vec![
                    Span::styled(check, check_style),
                    Span::styled(format!("{:<30} ", r.name), Style::default().fg(fg).add_modifier(Modifier::BOLD)),
                    Span::styled(counts, Style::default().fg(muted)),
                ]);

                ListItem::new(line)
            })
            .collect();

        let route_list = List::new(route_items)
            .highlight_style(
                Style::default()
                    .bg(theme.lighter_bg())
                    .fg(theme.bright_fg())
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▶ ");

        let help_text = Line::from(vec![
            Span::styled("[Space/Enter] ", Style::default().fg(accent).add_modifier(Modifier::BOLD)),
            Span::raw("Toggle  "),
            Span::styled("[I] ", Style::default().fg(accent).add_modifier(Modifier::BOLD)),
            Span::raw("Install Iran Preset  "),
            Span::styled("[D] ", Style::default().fg(accent).add_modifier(Modifier::BOLD)),
            Span::raw("Delete  "),
            Span::styled("[Esc] ", Style::default().fg(accent).add_modifier(Modifier::BOLD)),
            Span::raw("Close"),
        ]);
        let help_p = Paragraph::new(help_text);

        f.render_widget(popup_block, area);
        f.render_stateful_widget(route_list, inner_layout[0], route_list_state);
        f.render_widget(help_p, inner_layout[1]);
    }
}

impl Theme {
    pub fn bright_fg(&self) -> Color {
        crate::theme::hex_to_ratatui(&self.colors.bright_foreground)
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
