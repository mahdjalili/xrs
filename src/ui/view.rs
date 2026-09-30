use super::app::{App, Latency, LinkKind, Overlay, SortMode, Tab, ToastLevel, detect_link};
use crate::model::Subscription;
use crate::uri::Uri;
use crate::theme::Theme;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Margin, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, Cell, Clear, HighlightSpacing, Padding, Paragraph, Row, Scrollbar,
        ScrollbarOrientation, ScrollbarState, Table, Wrap,
    },
};
use std::time::{SystemTime, UNIX_EPOCH};
use unicode_width::UnicodeWidthStr;

const MIN_WIDTH: u16 = 60;
const MIN_HEIGHT: u16 = 14;
const DETAILS_MIN_WIDTH: u16 = 112;
const DETAILS_WIDTH: u16 = 40;
const ADDRESS_WIDTH: u16 = 28;
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Semantic colors resolved once per frame from the active theme.
struct Palette {
    fg: Color,
    bright: Color,
    muted: Color,
    faint: Color,
    bg: Color,
    surface: Color,
    selection: Color,
    accent: Color,
    on_accent: Color,
    ok: Color,
    warn: Color,
    err: Color,
    info: Color,
}

impl Palette {
    fn new(theme: &Theme) -> Self {
        Self {
            fg: theme.fg(),
            bright: theme.bright_fg(),
            muted: theme.muted(),
            faint: mix(theme.lighter_bg(), theme.muted(), 0.35),
            bg: theme.bg(),
            surface: theme.dark_bg(),
            selection: theme.lighter_bg(),
            accent: theme.accent(),
            on_accent: theme.on_accent(),
            ok: theme.green(),
            warn: theme.yellow(),
            err: theme.red(),
            info: theme.cyan(),
        }
    }

    fn latency(&self, ms: u64) -> Color {
        match ms {
            0..=150 => self.ok,
            151..=400 => self.warn,
            _ => self.err,
        }
    }

    fn panel(&self, title: &str) -> Block<'static> {
        Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(self.faint))
            .title(Line::from(format!(" {title} ")).fg(self.fg).bold())
            .padding(Padding::right(1))
            .style(Style::new().bg(self.bg))
    }
}

/// Linear blend of two RGB colors; non-RGB colors fall back to `a`.
fn mix(a: Color, b: Color, t: f32) -> Color {
    match (a, b) {
        (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
            let lerp = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
            Color::Rgb(lerp(r1, r2), lerp(g1, g2), lerp(b1, b2))
        }
        _ => a,
    }
}

pub fn render(app: &mut App, f: &mut Frame) {
    let p = Palette::new(&app.theme);
    let area = f.area();
    f.render_widget(Block::new().style(Style::new().bg(p.bg).fg(p.fg)), area);
    app.hit = Default::default();

    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        render_too_small(f, area, &p);
        return;
    }

    let outer = area.inner(Margin::new(1, 0));
    let [header, _, tabs, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(5),
        Constraint::Length(1),
    ])
    .areas(outer);

    render_header(f, app, header, &p);
    render_tabs(f, app, tabs, &p);

    let (main, side) = if body.width >= DETAILS_MIN_WIDTH {
        let [m, s] = Layout::horizontal([Constraint::Min(60), Constraint::Length(DETAILS_WIDTH)])
            .spacing(1)
            .areas(body);
        (m, Some(s))
    } else {
        (body, None)
    };

    match app.tab {
        Tab::Servers => {
            render_servers(f, app, main, &p);
            if let Some(side) = side {
                render_node_details(f, app, side, &p);
            }
        }
        Tab::Routing => {
            render_rules(f, app, main, &p);
            if let Some(side) = side {
                render_rule_details(f, app, side, &p);
            }
        }
        Tab::Subscriptions => {
            render_subs(f, app, main, &p);
            if let Some(side) = side {
                render_sub_details(f, app, side, &p);
            }
        }
    }

    render_footer(f, app, footer, &p);

    match &app.overlay {
        Overlay::None => {}
        Overlay::Help => render_help(f, area, &p),
        Overlay::Add(_) => render_add(f, app, area, &p),
        Overlay::Confirm(c) => render_confirm(f, area, &c.title, &c.message, &p),
    }
}

fn spinner(app: &App) -> &'static str {
    let frame = (app.started.elapsed().as_millis() / 80) as usize;
    SPINNER[frame % SPINNER.len()]
}

fn render_too_small(f: &mut Frame, area: Rect, p: &Palette) {
    let text = Text::from(vec![
        Line::from("Terminal too small").bold().fg(p.fg),
        Line::from(format!(
            "{}×{} — need at least {MIN_WIDTH}×{MIN_HEIGHT}",
            area.width, area.height
        ))
        .fg(p.muted),
    ]);
    let h = text.height() as u16;
    let rect = area.centered_vertically(Constraint::Length(h));
    f.render_widget(Paragraph::new(text).alignment(Alignment::Center), rect);
}

// ----- header & tabs -------------------------------------------------------

fn render_header(f: &mut Frame, app: &App, area: Rect, p: &Palette) {
    let pill = if let Some(op) = app.conn_busy {
        Span::styled(
            format!(" {} {} ", spinner(app), op.progress_label().to_uppercase()),
            Style::new().fg(p.on_accent).bg(p.warn).bold(),
        )
    } else if app.running {
        Span::styled(" ● CONNECTED ", Style::new().fg(p.on_accent).bg(p.ok).bold())
    } else {
        Span::styled(" ○ OFFLINE ", Style::new().fg(p.fg).bg(p.selection).bold())
    };

    let mut left = vec![
        Span::styled(" xrs ", Style::new().fg(p.on_accent).bg(p.accent).bold()),
        Span::raw("  "),
        pill,
        Span::raw("  "),
    ];
    match app.active_node() {
        Some(node) => {
            left.push(Span::styled(node.name.clone(), Style::new().fg(p.bright).bold()));
            if let Latency::Ms(ms) = app.latency(node) {
                left.push(Span::styled(format!("  {ms} ms"), Style::new().fg(p.latency(ms))));
            }
        }
        None => left.push(Span::styled("no server selected", Style::new().fg(p.muted).italic())),
    }

    let tun = if app.cfg.tun.enabled {
        Span::styled(" TUN ", Style::new().fg(p.on_accent).bg(p.accent).bold())
    } else {
        Span::styled(" TUN off ", Style::new().fg(p.muted))
    };
    let right = Line::from(vec![
        Span::styled("socks ", Style::new().fg(p.muted)),
        Span::styled(format!(":{}", app.cfg.inbounds.socks_port), Style::new().fg(p.fg)),
        Span::styled("  http ", Style::new().fg(p.muted)),
        Span::styled(format!(":{}", app.cfg.inbounds.http_port), Style::new().fg(p.fg)),
        Span::raw("  "),
        tun,
    ]);

    let left = Line::from(left);
    f.render_widget(Paragraph::new(left.clone()), area);
    if left.width() + right.width() + 2 <= area.width as usize {
        f.render_widget(Paragraph::new(right).alignment(Alignment::Right), area);
    }
}

fn render_tabs(f: &mut Frame, app: &mut App, area: Rect, p: &Palette) {
    let enabled_rules = app.cfg.routing.rules.iter().filter(|r| r.enabled).count();
    let mut spans = Vec::new();
    let mut x = area.x;
    for (i, tab) in Tab::ALL.into_iter().enumerate() {
        let count = match tab {
            Tab::Servers => app.cfg.nodes.len().to_string(),
            Tab::Routing => format!("{enabled_rules}/{}", app.cfg.routing.rules.len()),
            Tab::Subscriptions => app.cfg.subscriptions.len().to_string(),
        };
        let selected = app.tab == tab;
        let (label_style, count_style) = if selected {
            (
                Style::new().fg(p.on_accent).bg(p.accent).bold(),
                Style::new().fg(p.on_accent).bg(p.accent),
            )
        } else {
            (Style::new().fg(p.fg), Style::new().fg(p.muted))
        };
        let segment = vec![
            Span::styled(format!(" {} ", i + 1), count_style),
            Span::styled(tab.title(), label_style),
            Span::styled(format!(" {count} "), count_style),
        ];
        let width: u16 = segment.iter().map(|s| s.width() as u16).sum();
        app.hit.tabs.push((Rect::new(x, area.y, width, 1), tab));
        x += width + 1;
        spans.extend(segment);
        spans.push(Span::raw(" "));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);

    let mut activity = Vec::new();
    if app.pinging() {
        activity.push(Span::styled(
            format!("{} testing latency {}/{}", spinner(app), app.ping_done, app.ping_total),
            Style::new().fg(p.info),
        ));
    }
    if app.sub_busy {
        if !activity.is_empty() {
            activity.push(Span::raw("   "));
        }
        activity.push(Span::styled(
            format!("{} syncing subscriptions", spinner(app)),
            Style::new().fg(p.info),
        ));
    }
    if activity.is_empty() && app.tab == Tab::Servers && app.sort != SortMode::Config {
        activity.push(Span::styled("sorted by ", Style::new().fg(p.muted)));
        activity.push(Span::styled(app.sort.label(), Style::new().fg(p.fg)));
    }
    let tabs_width = (x - area.x) as usize;
    let activity = Line::from(activity);
    if tabs_width + activity.width() < area.width as usize {
        f.render_widget(Paragraph::new(activity).alignment(Alignment::Right), area);
    }
}

// ----- tables --------------------------------------------------------------

struct TableFrame<'a> {
    block: Block<'a>,
    header: Row<'a>,
    widths: Vec<Constraint>,
}

fn render_table(
    f: &mut Frame,
    app: &mut App,
    area: Rect,
    p: &Palette,
    frame: TableFrame<'_>,
    rows: Vec<Row<'_>>,
    empty: Option<Text<'_>>,
) {
    let len = rows.len();
    let inner = frame.block.inner(area);
    app.hit.table_body = Rect {
        y: inner.y + 1,
        height: inner.height.saturating_sub(1),
        ..inner
    };

    if let Some(empty) = empty.filter(|_| len == 0) {
        f.render_widget(frame.block, area);
        let h = empty.height() as u16;
        let rect = inner.centered_vertically(Constraint::Length(h));
        f.render_widget(Paragraph::new(empty).alignment(Alignment::Center), rect);
        return;
    }

    let table = Table::new(rows, frame.widths)
        .header(frame.header.style(Style::new().fg(p.muted).add_modifier(Modifier::BOLD)))
        .block(frame.block)
        .column_spacing(2)
        .row_highlight_style(Style::new().bg(p.selection).add_modifier(Modifier::BOLD))
        .highlight_symbol(Span::styled("▌", Style::new().fg(p.accent)))
        .highlight_spacing(HighlightSpacing::Always);

    let state = match app.tab {
        Tab::Servers => &mut app.servers,
        Tab::Routing => &mut app.rules,
        Tab::Subscriptions => &mut app.subs,
    };
    f.render_stateful_widget(table, area, state);

    let visible = app.hit.table_body.height as usize;
    if len > visible {
        let mut sb = ScrollbarState::new(len - visible + 1)
            .position(state.offset())
            .viewport_content_length(visible);
        let track = Rect::new(area.right().saturating_sub(1), area.y + 2, 1, area.height.saturating_sub(3));
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(Some("│"))
                .track_style(Style::new().fg(p.faint))
                .thumb_symbol("┃")
                .thumb_style(Style::new().fg(p.muted)),
            track,
            &mut sb,
        );
    }
}

fn position_label(selected: Option<usize>, shown: usize, total: usize, p: &Palette) -> Line<'static> {
    let pos = selected.map_or(0, |i| i + 1).min(shown);
    let text = if shown == total {
        format!(" {pos}/{total} ")
    } else {
        format!(" {pos}/{shown} of {total} ")
    };
    Line::from(text).fg(p.muted).right_aligned()
}

fn render_servers(f: &mut Frame, app: &mut App, area: Rect, p: &Palette) {
    let visible = app.visible_nodes();
    let wide = area.width >= 100;
    let medium = area.width >= 76;

    let mut block = p.panel("Servers").title_bottom(position_label(
        app.servers.selected(),
        visible.len(),
        app.cfg.nodes.len(),
        p,
    ));
    if app.filter_editing || !app.filter.is_empty() {
        let style = if app.filter_editing { Style::new().fg(p.bright) } else { Style::new().fg(p.accent) };
        let mut spans = vec![Span::styled(" / ", Style::new().fg(p.accent).bold()), Span::styled(app.filter.value().to_string(), style)];
        if app.filter_editing {
            spans.push(Span::styled("▏", Style::new().fg(p.accent)));
        }
        spans.push(Span::raw(" "));
        block = block.title_top(Line::from(spans).right_aligned());
    }
    if app.filter_editing {
        block = block.border_style(Style::new().fg(p.accent));
    }

    let mut header = vec!["", "Name", "Proto", "Net"];
    let mut widths = vec![
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(6),
        Constraint::Length(5),
    ];
    if medium {
        header.push("Security");
        widths.push(Constraint::Length(8));
    }
    if wide {
        header.push("Address");
        widths.push(Constraint::Length(ADDRESS_WIDTH));
    }
    header.push("Latency");
    widths.push(Constraint::Length(12));

    let active = app.cfg.active_node_id.as_deref();
    let rows: Vec<Row> = visible
        .iter()
        .map(|&i| {
            let n = &app.cfg.nodes[i];
            let is_active = active == Some(n.id.as_str());
            let marker = match (is_active, app.running) {
                (true, true) => Span::styled("●", Style::new().fg(p.ok)),
                (true, false) => Span::styled("○", Style::new().fg(p.accent)),
                _ => Span::raw(" "),
            };
            let name_style = if is_active { Style::new().fg(p.bright).bold() } else { Style::new().fg(p.fg) };
            let mut cells = vec![
                Cell::from(marker),
                Cell::from(Span::styled(n.name.clone(), name_style)),
                Cell::from(Span::styled(n.protocol.to_string(), Style::new().fg(p.accent))),
                Cell::from(Span::styled(n.network.clone(), Style::new().fg(p.muted))),
            ];
            if medium {
                cells.push(Cell::from(security_span(&n.security, p)));
            }
            if wide {
                cells.push(Cell::from(Span::styled(
                    truncate(&format!("{}:{}", n.server, n.port), ADDRESS_WIDTH as usize),
                    Style::new().fg(p.muted),
                )));
            }
            cells.push(Cell::from(latency_line(app, app.latency(n), p)));
            Row::new(cells)
        })
        .collect();

    let empty = if app.cfg.nodes.is_empty() {
        Some(empty_state(
            "No servers yet",
            &[("a", "add a share link or subscription URL"), ("?", "all shortcuts")],
            p,
        ))
    } else {
        Some(empty_state(
            &format!("No servers match “{}”", app.filter.value()),
            &[("esc", "clear the filter")],
            p,
        ))
    };

    render_table(
        f,
        app,
        area,
        p,
        TableFrame {
            block,
            header: Row::new(header),
            widths,
        },
        rows,
        empty,
    );
}

fn render_rules(f: &mut Frame, app: &mut App, area: Rect, p: &Palette) {
    let rules = &app.cfg.routing.rules;
    let block = p
        .panel("Routing rules")
        .title_bottom(position_label(app.rules.selected(), rules.len(), rules.len(), p));
    let rows: Vec<Row> = rules
        .iter()
        .map(|r| {
            let (glyph, style) = if r.enabled {
                ("◉ on ", Style::new().fg(p.ok))
            } else {
                ("○ off", Style::new().fg(p.muted))
            };
            let name_style = if r.enabled { Style::new().fg(p.fg) } else { Style::new().fg(p.muted) };
            Row::new(vec![
                Cell::from(Span::styled(glyph, style)),
                Cell::from(Span::styled(r.name.clone(), name_style)),
                Cell::from(count_span(r.direct_domains.len() + r.direct_ips.len(), p.info, p)),
                Cell::from(count_span(r.proxy_domains.len() + r.proxy_ips.len(), p.accent, p)),
                Cell::from(count_span(r.block_domains.len() + r.block_ips.len(), p.err, p)),
            ])
        })
        .collect();
    let empty = empty_state(
        "No routing rules",
        &[("i", "install the Iran bypass preset")],
        p,
    );
    render_table(
        f,
        app,
        area,
        p,
        TableFrame {
            block,
            header: Row::new(vec!["State", "Rule", "Direct", "Proxy", "Block"]),
            widths: vec![
                Constraint::Length(5),
                Constraint::Fill(1),
                Constraint::Length(6),
                Constraint::Length(6),
                Constraint::Length(6),
            ],
        },
        rows,
        Some(empty),
    );
}

fn render_subs(f: &mut Frame, app: &mut App, area: Rect, p: &Palette) {
    let subs = &app.cfg.subscriptions;
    let block = p
        .panel("Subscriptions")
        .title_bottom(position_label(app.subs.selected(), subs.len(), subs.len(), p));
    let wide = area.width >= 90;
    let rows: Vec<Row> = subs
        .iter()
        .map(|s| {
            let mut cells = vec![
                Cell::from(Span::styled(s.name.clone(), Style::new().fg(p.fg))),
                Cell::from(Span::styled(s.node_count.to_string(), Style::new().fg(p.accent))),
                Cell::from(Span::styled(relative_time(s.updated_at), Style::new().fg(p.muted))),
            ];
            if wide {
                cells.push(Cell::from(Span::styled(host_of(&s.url), Style::new().fg(p.muted))));
            }
            Row::new(cells)
        })
        .collect();
    let mut header = vec!["Name", "Servers", "Updated"];
    let mut widths = vec![Constraint::Fill(1), Constraint::Length(7), Constraint::Length(10)];
    if wide {
        header.push("Source");
        widths.push(Constraint::Fill(1));
    }
    let empty = empty_state(
        "No subscriptions",
        &[("a", "paste an https:// subscription URL")],
        p,
    );
    render_table(
        f,
        app,
        area,
        p,
        TableFrame {
            block,
            header: Row::new(header),
            widths,
        },
        rows,
        Some(empty),
    );
}

// ----- details panes -------------------------------------------------------

fn kv<'a>(key: &'a str, value: impl Into<String>, style: Style, p: &Palette) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("{key:<10}"), Style::new().fg(p.muted)),
        Span::styled(value.into(), style),
    ])
}

fn details_block(p: &Palette) -> Block<'static> {
    p.panel("Details").padding(Padding::horizontal(1))
}

fn render_node_details(f: &mut Frame, app: &App, area: Rect, p: &Palette) {
    let Some(n) = app.selected_node() else {
        f.render_widget(details_block(p), area);
        return;
    };
    let fg = Style::new().fg(p.fg);
    let mut lines = vec![
        Line::from(n.name.clone()).fg(p.bright).bold(),
        Line::from(format!("{} · {} · {}", n.protocol, n.network, n.security)).fg(p.muted),
        Line::default(),
    ];
    let mut latency = vec![Span::styled(format!("{:<10}", "Latency"), Style::new().fg(p.muted))];
    latency.extend(latency_line(app, app.latency(n), p).spans);
    lines.push(Line::from(latency));
    let status = if app.cfg.active_node_id.as_deref() == Some(n.id.as_str()) {
        if app.running {
            Span::styled("● in use", Style::new().fg(p.ok))
        } else {
            Span::styled("○ selected", Style::new().fg(p.accent))
        }
    } else {
        Span::styled("available", Style::new().fg(p.muted))
    };
    lines.push(Line::from(vec![Span::styled(format!("{:<10}", "Status"), Style::new().fg(p.muted)), status]));
    lines.push(Line::default());
    lines.push(kv("Server", n.server.clone(), fg, p));
    lines.push(kv("Port", n.port.to_string(), fg, p));
    let optional = [
        ("Path", n.path.as_deref()),
        ("Host", n.host.as_deref()),
        ("Service", n.service_name.as_deref()),
        ("SNI", n.sni.as_deref()),
        ("Finger", n.fingerprint.as_deref()),
        ("Flow", n.flow.as_deref()),
        ("Cipher", n.cipher.as_deref()),
    ];
    for (k, v) in optional {
        if let Some(v) = v.filter(|v| !v.is_empty()) {
            lines.push(kv(k, v.to_string(), fg, p));
        }
    }
    if let Some(alpn) = n.alpn.as_ref().filter(|a| !a.is_empty()) {
        lines.push(kv("ALPN", alpn.join(", "), fg, p));
    }
    lines.push(Line::default());
    let source = n
        .subscription_id
        .as_deref()
        .and_then(|id| app.cfg.subscriptions.iter().find(|s| s.id == id))
        .map_or_else(|| "manual".to_string(), |s| s.name.clone());
    lines.push(kv("Source", source, Style::new().fg(p.muted), p));

    f.render_widget(
        Paragraph::new(lines).block(details_block(p)).wrap(Wrap { trim: false }),
        area,
    );
}

fn render_rule_details(f: &mut Frame, app: &App, area: Rect, p: &Palette) {
    let Some(r) = app.rules.selected().and_then(|i| app.cfg.routing.rules.get(i)) else {
        f.render_widget(details_block(p), area);
        return;
    };
    let mut lines = vec![
        Line::from(r.name.clone()).fg(p.bright).bold(),
        if r.enabled {
            Line::from("◉ enabled").fg(p.ok)
        } else {
            Line::from("○ disabled").fg(p.muted)
        },
    ];
    if !r.description.is_empty() {
        lines.push(Line::default());
        lines.push(Line::from(r.description.clone()).fg(p.fg));
    }
    rule_section(&mut lines, "Direct", p.info, [&r.direct_domains, &r.direct_ips], p);
    rule_section(&mut lines, "Proxy", p.accent, [&r.proxy_domains, &r.proxy_ips], p);
    rule_section(&mut lines, "Block", p.err, [&r.block_domains, &r.block_ips], p);
    f.render_widget(
        Paragraph::new(lines).block(details_block(p)).wrap(Wrap { trim: false }),
        area,
    );
}

fn rule_section(lines: &mut Vec<Line<'_>>, title: &'static str, color: Color, lists: [&Vec<String>; 2], p: &Palette) {
    let entries: Vec<&String> = lists.into_iter().flatten().collect();
    if entries.is_empty() {
        return;
    }
    lines.push(Line::default());
    lines.push(Line::from(vec![
        Span::styled(title, Style::new().fg(color).bold()),
        Span::styled(format!("  {}", entries.len()), Style::new().fg(p.muted)),
    ]));
    for e in entries {
        lines.push(Line::from(vec![
            Span::styled("  • ", Style::new().fg(p.faint)),
            Span::styled(e.clone(), Style::new().fg(p.fg)),
        ]));
    }
}

fn render_sub_details(f: &mut Frame, app: &App, area: Rect, p: &Palette) {
    let Some(s): Option<&Subscription> = app.subs.selected().and_then(|i| app.cfg.subscriptions.get(i)) else {
        f.render_widget(details_block(p), area);
        return;
    };
    let fg = Style::new().fg(p.fg);
    let lines = vec![
        Line::from(s.name.clone()).fg(p.bright).bold(),
        Line::default(),
        kv("Servers", s.node_count.to_string(), fg, p),
        kv("Updated", relative_time(s.updated_at), fg, p),
        Line::default(),
        Line::from("URL").fg(p.muted),
        Line::from(redact_url(&s.url)).fg(p.fg),
    ];
    f.render_widget(
        Paragraph::new(lines).block(details_block(p)).wrap(Wrap { trim: false }),
        area,
    );
}

// ----- footer --------------------------------------------------------------

fn hints(app: &App) -> Vec<(&'static str, &'static str)> {
    if app.filter_editing {
        return vec![("type", "to filter"), ("↑↓", "move"), ("enter", "done"), ("esc", "clear")];
    }
    let conn = if app.running { "disconnect" } else { "connect" };
    let mut out = match app.tab {
        Tab::Servers => vec![("enter", "use server"), ("space", conn), ("/", "filter"), ("p", "ping"), ("s", "sort"), ("t", "tun"), ("a", "add")],
        Tab::Routing => vec![("space", "toggle"), ("i", "preset"), ("d", "delete"), ("c", conn), ("t", "tun")],
        Tab::Subscriptions => vec![("u", "update"), ("a", "add"), ("d", "delete"), ("c", conn)],
    };
    out.extend([("tab", "switch"), ("?", "help"), ("q", "quit")]);
    out
}

fn render_footer(f: &mut Frame, app: &App, area: Rect, p: &Palette) {
    let toast = app.toast.as_ref().map(|t| {
        let (icon, color) = match t.level {
            ToastLevel::Info => ("•", p.info),
            ToastLevel::Success => ("✓", p.ok),
            ToastLevel::Warning => ("!", p.warn),
            ToastLevel::Error => ("✕", p.err),
        };
        let max = (area.width as usize * 3 / 5).max(20);
        Line::from(vec![
            Span::styled(format!("{icon} "), Style::new().fg(color).bold()),
            Span::styled(truncate(&t.text, max), Style::new().fg(if t.level == ToastLevel::Error { p.err } else { p.fg })),
        ])
    });
    let reserved = toast.as_ref().map_or(0, |t| t.width() + 3);

    let mut spans = Vec::new();
    let mut used = 0;
    for (key, label) in hints(app) {
        let w = key.width() + label.width() + 3;
        if used + w + reserved > area.width as usize {
            break;
        }
        spans.push(Span::styled(key, Style::new().fg(p.accent).bold()));
        spans.push(Span::styled(format!(" {label}   "), Style::new().fg(p.muted)));
        used += w;
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
    if let Some(t) = toast {
        f.render_widget(Paragraph::new(t).alignment(Alignment::Right), area);
    }
}

// ----- overlays ------------------------------------------------------------

fn modal(f: &mut Frame, area: Rect, width: u16, height: u16, title: &str, border: Color, p: &Palette) -> Rect {
    let rect = area.centered(
        Constraint::Length(width.min(area.width.saturating_sub(4))),
        Constraint::Length(height.min(area.height.saturating_sub(2))),
    );
    f.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(border))
        .title(Line::from(format!(" {title} ")).fg(p.bright).bold())
        .padding(Padding::new(2, 2, 1, 0))
        .style(Style::new().bg(p.surface).fg(p.fg));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    inner
}

fn render_help(f: &mut Frame, area: Rect, p: &Palette) {
    let sections: [(&str, &[(&str, &str)]); 5] = [
        ("Navigate", &[("↑↓ j k", "move"), ("g G", "first / last"), ("PgUp PgDn", "page"), ("tab 1 2 3", "switch view"), ("mouse", "scroll, click, click again to connect")]),
        ("Connection", &[("enter", "connect to selected server"), ("space c", "connect / disconnect"), ("x", "disconnect"), ("t", "toggle TUN mode")]),
        ("Servers", &[("/", "filter (name, address, protocol…)"), ("s", "cycle sort order"), ("p", "test latency of all servers"), ("y", "copy share link"), ("a", "add link or subscription (or just paste)"), ("d", "delete")]),
        ("Routing & subscriptions", &[("space", "toggle rule"), ("i", "install Iran bypass preset"), ("u", "update all subscriptions")]),
        ("General", &[("?", "toggle this help"), ("q ctrl-c", "quit")]),
    ];
    let mut lines = Vec::new();
    for (i, (title, keys)) in sections.iter().enumerate() {
        if i > 0 {
            lines.push(Line::default());
        }
        lines.push(Line::from(*title).fg(p.accent).bold());
        for (k, v) in keys.iter() {
            lines.push(Line::from(vec![
                Span::styled(format!("  {k:<12}"), Style::new().fg(p.bright)),
                Span::styled(*v, Style::new().fg(p.fg)),
            ]));
        }
    }
    let height = lines.len() as u16 + 3;
    let inner = modal(f, area, 64, height, "Keyboard shortcuts", p.accent, p);
    f.render_widget(Paragraph::new(lines), inner);
}

fn render_add(f: &mut Frame, app: &App, area: Rect, p: &Palette) {
    let Overlay::Add(input) = &app.overlay else { return };
    let inner = modal(f, area, 84, 10, "Add server or subscription", p.accent, p);
    let [hint, _, field, status, _, keys] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(inner);

    f.render_widget(
        Paragraph::new("Paste a share link (vless, vmess, trojan, ss) or a subscription URL.")
            .fg(p.muted),
        hint,
    );

    f.render_widget(Block::new().style(Style::new().bg(p.selection)), field);
    let field_inner = field.inner(Margin::new(1, 0));
    let (visible, col) = input.viewport(field_inner.width.saturating_sub(1) as usize);
    let text = if input.is_empty() {
        Line::from("vless://…").fg(p.muted).italic()
    } else {
        Line::from(visible.to_string()).fg(p.bright)
    };
    f.render_widget(Paragraph::new(text), field_inner);
    f.set_cursor_position((field_inner.x + col as u16, field_inner.y));

    let detected = match detect_link(input.value()) {
        LinkKind::Empty => Line::default(),
        LinkKind::Share(kind) => Line::from(vec![Span::styled("✓ ", Style::new().fg(p.ok)), Span::styled(format!("{kind} share link"), Style::new().fg(p.fg))]),
        LinkKind::Subscription => Line::from(vec![Span::styled("✓ ", Style::new().fg(p.ok)), Span::styled("Subscription URL, servers will be fetched in the background", Style::new().fg(p.fg))]),
        LinkKind::Unknown => Line::from(vec![Span::styled("✕ ", Style::new().fg(p.err)), Span::styled("Unrecognized format", Style::new().fg(p.err))]),
    };
    f.render_widget(Paragraph::new(detected), status);
    f.render_widget(key_line(&[("enter", "add"), ("esc", "cancel"), ("ctrl-u", "clear")], p), keys);
}

fn render_confirm(f: &mut Frame, area: Rect, title: &str, message: &str, p: &Palette) {
    let inner = modal(f, area, 56, 7, title, p.err, p);
    let [msg, _, keys] = Layout::vertical([Constraint::Min(1), Constraint::Length(1), Constraint::Length(1)]).areas(inner);
    f.render_widget(Paragraph::new(message.to_string()).fg(p.fg).wrap(Wrap { trim: true }), msg);
    f.render_widget(key_line(&[("y", "delete"), ("n esc", "cancel")], p), keys);
}

// ----- helpers -------------------------------------------------------------

fn key_line<'a>(keys: &[(&'a str, &'a str)], p: &Palette) -> Paragraph<'a> {
    let mut spans = Vec::new();
    for (k, v) in keys {
        spans.push(Span::styled(*k, Style::new().fg(p.accent).bold()));
        spans.push(Span::styled(format!(" {v}   "), Style::new().fg(p.muted)));
    }
    Paragraph::new(Line::from(spans))
}

fn empty_state<'a>(title: &str, actions: &[(&'a str, &'a str)], p: &Palette) -> Text<'a> {
    let mut lines = vec![Line::from(title.to_string()).fg(p.fg).bold(), Line::default()];
    for (k, v) in actions {
        lines.push(Line::from(vec![
            Span::styled(*k, Style::new().fg(p.accent).bold()),
            Span::styled(format!("  {v}"), Style::new().fg(p.muted)),
        ]));
    }
    Text::from(lines)
}

fn security_span(security: &str, p: &Palette) -> Span<'static> {
    let color = match security {
        "reality" | "tls" => p.ok,
        "" | "none" => p.warn,
        _ => p.fg,
    };
    let label = if security.is_empty() { "none" } else { security };
    Span::styled(label.to_string(), Style::new().fg(color))
}

fn count_span(n: usize, color: Color, p: &Palette) -> Span<'static> {
    if n == 0 {
        Span::styled("—", Style::new().fg(p.faint))
    } else {
        Span::styled(n.to_string(), Style::new().fg(color))
    }
}

pub fn signal_bars(ms: u64) -> usize {
    match ms {
        0..=100 => 4,
        101..=200 => 3,
        201..=400 => 2,
        _ => 1,
    }
}

fn latency_line(app: &App, latency: Latency, p: &Palette) -> Line<'static> {
    match latency {
        Latency::Ms(ms) => {
            let bars = signal_bars(ms);
            let glyphs = ["▂", "▄", "▆", "█"];
            let color = p.latency(ms);
            let mut spans: Vec<Span> = glyphs
                .iter()
                .enumerate()
                .map(|(i, g)| Span::styled(*g, Style::new().fg(if i < bars { color } else { p.faint })))
                .collect();
            spans.push(Span::styled(format!(" {ms:>4} ms"), Style::new().fg(color)));
            Line::from(spans)
        }
        Latency::Testing => Line::from(format!("{} testing", spinner(app))).fg(p.info),
        Latency::Timeout => Line::from("✕ timeout").fg(p.err),
        Latency::Untested => Line::from("—").fg(p.faint),
    }
}

pub fn truncate(s: &str, max: usize) -> String {
    if s.width() <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = UnicodeWidthStr::width(c.encode_utf8(&mut [0; 4]) as &str);
        if used + w + 1 > max {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

pub fn relative_time(unix: u64) -> String {
    if unix == 0 {
        return "never".into();
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let secs = now.saturating_sub(unix);
    match secs {
        0..=59 => "just now".into(),
        60..=3599 => format!("{}m ago", secs / 60),
        3600..=86_399 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86_400),
    }
}

/// Subscription URLs usually carry an access token in the query string, so
/// only the host and path are shown on screen.
pub fn redact_url(url: &str) -> String {
    match Uri::parse(url) {
        Some(u) => {
            let base = format!("{}://{}{}", u.scheme, u.host_str(), u.http_path());
            if u.query.is_some() { format!("{base}?…") } else { base }
        }
        None => "(invalid URL)".to_string(),
    }
}

fn host_of(url: &str) -> String {
    Uri::parse(url).map(|u| u.host_str()).unwrap_or_else(|| url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AppConfig, Protocol, ProxyNode};
    use ratatui::{Terminal, backend::TestBackend};

    fn node(id: &str, name: &str, ping: Option<u64>) -> ProxyNode {
        ProxyNode {
            id: id.into(),
            name: name.into(),
            protocol: Protocol::Vless,
            server: format!("{id}.example.com"),
            port: 443,
            secret: String::new(),
            cipher: None,
            network: "ws".into(),
            path: Some("/ws".into()),
            host: None,
            service_name: None,
            security: "reality".into(),
            sni: Some("example.com".into()),
            alpn: None,
            fingerprint: Some("chrome".into()),
            pbk: None,
            sid: None,
            spider_x: None,
            flow: None,
            raw_link: String::new(),
            subscription_id: None,
            ping_ms: ping,
        }
    }

    fn draw(app: &mut App, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).expect("terminal");
        term.draw(|f| render(app, f)).expect("draw");
        let buf = term.backend().buffer().clone();
        buf.content().iter().map(|c| c.symbol()).collect::<String>()
    }

    fn app(nodes: Vec<ProxyNode>) -> App {
        let cfg = AppConfig {
            active_node_id: nodes.first().map(|n| n.id.clone()),
            nodes,
            ..AppConfig::default()
        };
        App::with_config(cfg, Theme::load())
    }

    #[test]
    fn renders_every_view_at_many_sizes() {
        let nodes = (0..40).map(|i| node(&format!("n{i}"), &format!("🇫🇮 FI{i}-[E1]"), Some(i * 20))).collect();
        let mut app = app(nodes);
        for (w, h) in [(60, 14), (80, 24), (100, 30), (140, 40), (220, 60), (30, 8)] {
            for tab in Tab::ALL {
                app.tab = tab;
                draw(&mut app, w, h);
                app.overlay = Overlay::Help;
                draw(&mut app, w, h);
                app.overlay = Overlay::Add(Default::default());
                draw(&mut app, w, h);
                app.overlay = Overlay::None;
            }
        }
    }

    #[test]
    fn wide_layout_shows_details_and_address() {
        let mut app = app(vec![node("a", "FI1", Some(42))]);
        let screen = draw(&mut app, 150, 30);
        assert!(screen.contains("Details"));
        assert!(screen.contains("a.example.com:443"));
        assert!(screen.contains("42 ms"));
    }

    #[test]
    fn empty_state_guides_user() {
        let mut app = app(Vec::new());
        let screen = draw(&mut app, 100, 24);
        assert!(screen.contains("No servers yet"));
    }

    #[test]
    fn tiny_terminal_shows_notice() {
        let mut app = app(Vec::new());
        assert!(draw(&mut app, 40, 10).contains("Terminal too small"));
    }

    #[test]
    fn truncates_by_display_width() {
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("abc", 4), "abc");
        assert_eq!(truncate("日本語日本", 5), "日本…");
    }

    #[test]
    fn hides_subscription_tokens() {
        assert_eq!(
            redact_url("https://sub.example.net/api/sub?token=secret"),
            "https://sub.example.net/api/sub?…"
        );
        assert_eq!(redact_url("https://a.example/x"), "https://a.example/x");
    }

    #[test]
    fn latency_tiers() {
        assert_eq!(signal_bars(40), 4);
        assert_eq!(signal_bars(150), 3);
        assert_eq!(signal_bars(300), 2);
        assert_eq!(signal_bars(900), 1);
    }
}
