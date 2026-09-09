//! All drawing. The screen is a title strip, a two-pane body, and a status
//! strip; overlays (help, confirm) are painted on top.

use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Clear, List, ListItem, ListState, Padding, Paragraph, Wrap,
};

use crate::app::{App, Row, ToastKind};
use crate::diag;
use crate::model::{GroupSource, Health, Server};
use crate::theme::Theme;

const SPINNER: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];

pub fn draw(f: &mut Frame, app: &mut App, tick: usize) {
    // Cloned once per frame rather than borrowed, so the drawing code can keep
    // taking `&mut App` for hit-test bookkeeping.
    let t = app.theme.clone();
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(6),
            Constraint::Length(1),
        ])
        .split(area);

    draw_titlebar(f, app, &t, chunks[0], tick);
    draw_rule(f, &t, chunks[1]);

    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
        .split(chunks[2]);

    app.list_area = split[0];
    draw_list(f, app, &t, split[0]);
    draw_detail(f, app, &t, split[1]);
    draw_status(f, app, &t, chunks[3]);

    if app.help {
        draw_help(f, app, &t, area);
    }
    if app.diagnostics {
        draw_diagnostics(f, app, &t, area);
    }
    if app.confirm.is_some() {
        draw_confirm(f, app, &t, area);
    }
}

fn draw_titlebar(f: &mut Frame, app: &App, t: &Theme, area: Rect, tick: usize) {
    let services = app
        .rows
        .iter()
        .filter(|r| matches!(r, Row::Server(_)))
        .count();
    let projects = app
        .groups
        .iter()
        .filter(|g| matches!(g.source, GroupSource::Repo | GroupSource::Folder))
        .count();
    let trouble: usize = app.groups.iter().map(|g| g.trouble).sum();

    // Everything after the count is optional; a narrow terminal keeps the
    // identity and the number, and drops the rest rather than colliding.
    let roomy = area.width >= 78;
    let mut left = vec![
        Span::styled(" quarry", Style::default().fg(t.accent).bold()),
        Span::styled("  ", Style::default()),
        Span::styled(format!("{services}"), Style::default().fg(t.text).bold()),
        Span::styled(" listening", Style::default().fg(t.muted)),
    ];
    if roomy {
        left.push(Span::styled(" · ", Style::default().fg(t.faint)));
        left.push(Span::styled(
            format!("{projects}"),
            Style::default().fg(t.text).bold(),
        ));
        left.push(Span::styled(
            if projects == 1 {
                " project"
            } else {
                " projects"
            },
            Style::default().fg(t.muted),
        ));
    }
    if trouble > 0 && roomy {
        left.push(Span::styled(" · ", Style::default().fg(t.faint)));
        left.push(Span::styled(
            format!("{trouble}"),
            Style::default().fg(t.server_error).bold(),
        ));
        left.push(Span::styled(
            " unhealthy",
            Style::default().fg(t.server_error),
        ));
    }
    if !app.search.is_empty() && roomy {
        left.push(Span::styled(" · ", Style::default().fg(t.faint)));
        left.push(Span::styled(
            format!("filter “{}”", app.search),
            Style::default().fg(t.client_error),
        ));
    }

    let right = if let Some(f) = &app.failure {
        format!(
            "⚠ scan failing ({}×){} ",
            f.count,
            if f.transient { ", retrying" } else { "" }
        )
    } else if !app.scanner_alive {
        "⚠ scanner stopped ".to_string()
    } else if app.scanning {
        format!("{} scanning ", SPINNER[tick % SPINNER.len()])
    } else {
        match app.last_scan {
            Some(t) => {
                let e = t.elapsed();
                if e.as_secs() == 0 {
                    "updated just now ".to_string()
                } else {
                    format!("updated {} ago ", ago(e))
                }
            }
            None => String::from("starting "),
        }
    };

    f.render_widget(Line::from(left), area);
    let right_style = if app.is_stale() {
        Style::default().fg(t.server_error).bold()
    } else {
        Style::default().fg(t.muted)
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(right, right_style))).alignment(Alignment::Right),
        area,
    );
}

fn draw_rule(f: &mut Frame, t: &Theme, area: Rect) {
    let rule = "─".repeat(area.width as usize);
    f.render_widget(
        Line::from(Span::styled(rule, Style::default().fg(t.faint))),
        area,
    );
}

fn draw_list(f: &mut Frame, app: &mut App, t: &Theme, area: Rect) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(t.faint))
        .title(Line::from(vec![Span::styled(
            " Services ",
            Style::default().fg(t.text).bold(),
        )]));

    if app.rows.is_empty() {
        let msg = if app.servers.is_empty() {
            vec![
                Line::from(""),
                Line::from(Span::styled(
                    "Nothing is listening.",
                    Style::default().fg(t.muted),
                )),
                Line::from(Span::styled(
                    "Start a dev server and quarry will pick it up.",
                    Style::default().fg(t.faint),
                )),
            ]
        } else {
            vec![
                Line::from(""),
                Line::from(Span::styled("No matches.", Style::default().fg(t.muted))),
                Line::from(Span::styled(
                    "Press a to include system services, or / to change the filter.",
                    Style::default().fg(t.faint),
                )),
            ]
        };
        f.render_widget(
            Paragraph::new(msg)
                .alignment(Alignment::Center)
                .block(block),
            area,
        );
        return;
    }

    let inner_width = area.width.saturating_sub(2) as usize;
    let inner_height = area.height.saturating_sub(2) as usize;

    // Only the rows that will be on screen are built. Handing ratatui two
    // thousand `ListItem`s so it can draw forty of them made the cost of a
    // frame scale with the size of the machine rather than the size of the
    // window.
    app.offset = scroll_to(app.offset, app.selected, app.rows.len(), inner_height);
    let end = (app.offset + inner_height).min(app.rows.len());
    let window = &app.rows[app.offset.min(end)..end];

    let cursor = app.selected.saturating_sub(app.offset);
    let items: Vec<ListItem> = window
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let selected = i == cursor;
            let line = match row {
                Row::Group(g) => group_line(app, t, *g, inner_width, selected),
                Row::Server(s) => server_line(&app.servers[*s], t, inner_width, selected),
            };
            // Reverse video inverts whatever colour it finds, so a row of many
            // colours becomes a bar striped in as many. Flattened first, it
            // becomes what every terminal list looks like: one solid block.
            ListItem::new(if selected && t.selection_inverts() {
                flatten(line)
            } else {
                line
            })
        })
        .collect();

    let list = List::new(items).block(block).highlight_style(t.selected());

    // The selection is relative to the window, since the window is all the
    // widget can see.
    let mut state =
        ListState::default().with_selected(Some(app.selected.saturating_sub(app.offset)));
    f.render_stateful_widget(list, area, &mut state);
}

/// Keep `selected` inside a window of `height` rows, moving as little as
/// possible — the list should not jump when the cursor is already visible.
fn scroll_to(offset: usize, selected: usize, len: usize, height: usize) -> usize {
    if height == 0 || len == 0 {
        return 0;
    }
    let max_offset = len.saturating_sub(height);
    let mut offset = offset.min(max_offset);
    if selected < offset {
        offset = selected;
    } else if selected >= offset + height {
        offset = selected + 1 - height;
    }
    offset.min(max_offset)
}

/// The one-column gutter every row carries, filled for the selected one.
///
/// A `highlight_symbol` would shift the selected row's contents right by its
/// own width and leave every other row where it was, so the list appears to
/// twitch as the cursor moves. Reserving the column on every row costs one
/// character and keeps the type still.
/// Strip every span's own colour, leaving the text and its weight.
///
/// The information the colours carried is not lost: the health glyph, the
/// group marker and the badge text all still say what they said. What goes is
/// the striping, which said nothing.
fn flatten(line: Line<'static>) -> Line<'static> {
    Line::from(
        line.spans
            .into_iter()
            .map(|span| {
                let modifiers = span.style.add_modifier;
                Span::styled(span.content, Style::default().add_modifier(modifiers))
            })
            .collect::<Vec<_>>(),
    )
}

fn gutter(selected: bool, t: &Theme) -> Span<'static> {
    if selected {
        Span::styled("▌", Style::default().fg(t.accent))
    } else {
        Span::raw(" ")
    }
}

fn group_line(app: &App, t: &Theme, idx: usize, width: usize, selected: bool) -> Line<'static> {
    let g = &app.groups[idx];
    let collapsed = app.collapsed.contains(&g.key);
    let name = match g.source {
        GroupSource::Unattributed => "no project".to_string(),
        GroupSource::System => "system services".to_string(),
        _ => g.key.clone(),
    };

    // The count on the right is the point of the row, so it gets its width
    // first; the branch and remote fill whatever is left, in that order.
    let right = if g.trouble > 0 {
        format!("✕{}  {} ", g.trouble, g.count)
    } else {
        format!("{} ", g.count)
    };
    let marker = if collapsed { "▸ " } else { "▾ " };
    let mut budget = width
        .saturating_sub(marker.chars().count())
        .saturating_sub(right.chars().count());

    let name = truncate(&name, budget);
    budget = budget.saturating_sub(name.chars().count());

    let mut spans = vec![
        gutter(selected, t),
        Span::styled(marker, Style::default().fg(t.faint)),
        Span::styled(
            name,
            match g.source {
                // A repository is a firmer claim than a directory that merely
                // has a name, and the colour says which one you are looking at.
                GroupSource::Repo => Style::default().fg(t.repo).bold(),
                GroupSource::Folder => Style::default().fg(t.folder).bold(),
                _ => Style::default().fg(t.generic).italic(),
            },
        ),
    ];

    // Only worth showing if there is room for more than an ellipsis.
    if let Some(branch) = &g.branch
        && budget > 6
    {
        let text = format!("  {}", truncate(branch, budget.saturating_sub(3)));
        budget = budget.saturating_sub(text.chars().count());
        spans.push(Span::styled(text, Style::default().fg(t.faint)));
    }
    if let Some(remote) = &g.remote {
        let needed = remote.chars().count() + 2;
        if budget >= needed + 2 {
            budget -= needed;
            spans.push(Span::styled(
                format!("  {remote}"),
                Style::default().fg(t.faint).italic(),
            ));
        }
    }

    spans.push(Span::raw(" ".repeat(budget)));
    spans.push(Span::styled(
        right,
        if g.trouble > 0 {
            Style::default().fg(t.server_error)
        } else {
            Style::default().fg(t.faint)
        },
    ));
    Line::from(spans)
}

fn server_line(s: &Server, t: &Theme, width: usize, selected: bool) -> Line<'static> {
    let (dot, dot_color) = (s.health.glyph(), t.health(&s.health));
    let port = format!("{:>5}", s.primary_label());
    let extra_ports = if s.listeners.len() > 1 {
        format!(" +{}", s.listeners.len() - 1)
    } else {
        String::new()
    };
    let badge = s.kind.label();
    let (status_full, status_color) = status_cell(&s.health, t);
    let status_short = status_abbrev(&s.health);

    // "  " + dot + " " + port + extras, then one trailing space at the end.
    let lead = 2 + 1 + 1 + port.chars().count() + extra_ports.chars().count() + 1;
    let avail = width.saturating_sub(lead + 1);

    // Degrade in a defined order — badge first, then the latency, then the
    // name — so a narrow terminal loses detail instead of losing its shape.
    const MIN_NAME: usize = 4;
    let badge_cost = badge.chars().count() + 2;
    let (show_badge, status) = if avail >= badge_cost + status_full.chars().count() + MIN_NAME {
        (true, status_full.clone())
    } else if avail >= status_full.chars().count() + MIN_NAME {
        (false, status_full.clone())
    } else {
        (false, status_short.clone())
    };

    let reserved = status.chars().count() + if show_badge { badge_cost } else { 0 };
    let name_width = avail.saturating_sub(reserved);
    let name = truncate(&s.service_name(), name_width);
    let pad = name_width.saturating_sub(name.chars().count());

    let mut spans = vec![
        gutter(selected, t),
        Span::raw(" "),
        Span::styled(dot, Style::default().fg(dot_color)),
        Span::raw(" "),
        Span::styled(port, Style::default().fg(t.text).bold()),
        Span::styled(extra_ports, Style::default().fg(t.faint)),
        Span::raw(" "),
        Span::styled(name, Style::default().fg(t.text)),
        Span::raw(" ".repeat(pad)),
    ];
    if show_badge {
        spans.push(Span::styled(
            format!("{badge}  "),
            Style::default().fg(t.kind(s.kind)),
        ));
    }
    spans.push(Span::styled(status, Style::default().fg(status_color)));
    spans.push(Span::raw(" "));
    Line::from(spans)
}

/// The shortest honest form of a status, for terminals with no room.
fn status_abbrev(h: &Health) -> String {
    match h {
        Health::Http { status, .. } => status.to_string(),
        Health::Open { .. } => "open".into(),
        Health::Bound => "bound".into(),
        Health::Closed => "down".into(),
        Health::Unknown => "···".into(),
    }
}

fn status_cell(h: &Health, t: &Theme) -> (String, ratatui::style::Color) {
    match h {
        Health::Http {
            status, latency, ..
        } => {
            let c = match status {
                200..=299 => t.ok,
                300..=399 => t.secondary,
                400..=499 => t.client_error,
                _ => t.server_error,
            };
            (format!("{status} {:>6}", crate::model::fmt_ms(*latency)), c)
        }
        Health::Open { .. } => ("open      ".to_string(), t.open),
        Health::Bound => ("bound     ".to_string(), t.open),
        Health::Closed => ("no answer ".to_string(), t.server_error),
        Health::Unknown => ("···       ".to_string(), t.faint),
    }
}

fn draw_detail(f: &mut Frame, app: &mut App, t: &Theme, area: Rect) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(t.faint))
        .padding(Padding::horizontal(1))
        .title(Span::styled(" Detail ", Style::default().fg(t.text).bold()));

    app.url_hitbox = None;

    let Some(s) = app.selected_server() else {
        // A folded group is a selection too. Showing "select a service" beside
        // one leaves the pane dead exactly when the user has just collapsed
        // something to look at it as a whole.
        if let Some(group) = app.selected_group() {
            f.render_widget(
                Paragraph::new(group_detail(app, group, t)).block(block),
                area,
            );
            return;
        }
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "Select a service",
                Style::default().fg(t.faint),
            )))
            .alignment(Alignment::Center)
            .block(block),
            area,
        );
        return;
    };

    let now = app.now;

    let (hero_dot, hero_color) = (s.health.glyph(), t.health(&s.health));
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(format!("{hero_dot} "), Style::default().fg(hero_color)),
        Span::styled(s.title(), Style::default().fg(t.text).bold()),
    ]));
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(s.service_name(), Style::default().fg(t.muted)),
        Span::styled(" · ", Style::default().fg(t.faint)),
        Span::styled(s.kind.label(), Style::default().fg(t.kind(s.kind))),
        Span::styled(" · ", Style::default().fg(t.faint)),
        Span::styled(format!("pid {}", s.pid), Style::default().fg(t.muted)),
    ]));
    lines.push(Line::from(""));

    // Address — the row the mouse can hit.
    lines.push(section("Address", t));
    let url_row = lines.len();
    let openable = s.kind.opens_in_a_browser() && !s.is_socket_only();
    if openable {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                s.url(),
                Style::default()
                    .fg(t.secondary)
                    .add_modifier(Modifier::UNDERLINED),
            ),
            Span::styled("  ↗", Style::default().fg(t.faint)),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(s.url(), Style::default().fg(t.secondary)),
            Span::styled("  (copied, not opened)", Style::default().fg(t.faint)),
        ]));
    }
    lines.push(Line::from(""));

    lines.push(section("Health", t));
    let (dot, color) = (s.health.glyph(), t.health(&s.health));
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(format!("{dot} "), Style::default().fg(color)),
        Span::styled(s.health.summary(), Style::default().fg(color)),
    ]));
    if let Health::Http { server, title, .. } = &s.health {
        if let Some(page) = title {
            lines.push(kv("page", page, t));
        }
        if let Some(sv) = server {
            lines.push(kv("server", sv, t));
        }
    }
    lines.push(Line::from(""));

    lines.push(section("Listening", t));
    for l in &s.listeners {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{:<6}", l.port), Style::default().fg(t.text)),
            Span::styled(
                format!("{}  {}", l.addr, l.scope()),
                Style::default().fg(if l.wildcard { t.client_error } else { t.muted }),
            ),
        ]));
    }
    lines.push(Line::from(""));

    if let Some(folder) = s.folder_name().filter(|_| s.repo.is_none()) {
        {
            lines.push(section("Folder", t));
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(folder, Style::default().fg(t.open).bold()),
            ]));
            if let Some(cwd) = &s.cwd {
                lines.push(kv("path", &tilde(&cwd.display().to_string()), t));
            }
            lines.push(Line::from(Span::styled(
                "  not a git repository",
                Style::default().fg(t.faint),
            )));
            lines.push(Line::from(""));
        }
    }

    if let Some(container) = &s.container {
        lines.push(section("Container", t));
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                container.display_name().to_string(),
                Style::default()
                    .fg(t.kind(crate::model::Kind::Container))
                    .bold(),
            ),
        ]));
        lines.push(kv("image", &container.image, t));
        lines.push(kv(
            "state",
            &match &container.health {
                Some(h) => format!("{} ({h})", container.state),
                None => container.state.clone(),
            },
            t,
        ));
        if let Some(project) = &container.project {
            lines.push(kv("compose", project, t));
        }
        lines.push(Line::from(""));
    }

    if let Some(repo) = &s.repo {
        lines.push(section("Repository", t));
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(repo.name.clone(), Style::default().fg(t.repo).bold()),
        ]));
        lines.push(kv("path", &tilde(&repo.root.display().to_string()), t));
        if let Some(b) = &repo.branch {
            lines.push(kv("branch", b, t));
        }
        if let Some(r) = &repo.remote {
            lines.push(kv("remote", r, t));
        }
        lines.push(Line::from(""));
    }

    lines.push(section("Process", t));
    lines.push(kv("user", &s.user, t));
    if s.started_at > 0
        && let Some(up) = s.uptime(now)
    {
        lines.push(kv("uptime", &ago(up), t));
    }
    lines.push(kv(
        "usage",
        &format!("{:.1}% cpu · {} rss", s.cpu, bytes(s.mem)),
        t,
    ));
    if let Some(ppid) = s.ppid {
        lines.push(kv("parent", &ppid.to_string(), t));
    }
    if let Some(exe) = &s.exe {
        lines.push(kv("binary", &tilde(&exe.display().to_string()), t));
    }
    if let Some(cwd) = &s.cwd {
        lines.push(kv("cwd", &tilde(&cwd.display().to_string()), t));
    }
    lines.push(Line::from(""));
    if !s.evidence.is_empty() {
        lines.push(kv("named by", &s.evidence.join(", "), t));
    }
    lines.push(section("Command", t));
    lines.push(Line::from(Span::styled(
        format!("  {}", s.cmdline),
        Style::default().fg(t.faint),
    )));

    let inner = block.inner(area);
    // +1 for the top border, and the leading two-space indent on the url row.
    if openable && (url_row as u16) < inner.height {
        let width = (s.url().chars().count() + 3) as u16;
        app.url_hitbox = Some(Rect {
            x: inner.x + 2,
            y: inner.y + url_row as u16,
            width: width.min(inner.width.saturating_sub(2)),
            height: 1,
        });
    }

    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

/// What a folded group has to say for itself.
fn group_detail(app: &App, group: &crate::app::Group, t: &Theme) -> Vec<Line<'static>> {
    let members = app.servers_in(&group.key);
    let name = match group.source {
        GroupSource::Unattributed => "no project".to_string(),
        GroupSource::System => "system services".to_string(),
        _ => group.key.clone(),
    };

    let mut lines = vec![
        Line::from(vec![
            Span::styled("▸ ", Style::default().fg(t.faint)),
            Span::styled(
                name,
                Style::default()
                    .fg(match group.source {
                        GroupSource::Repo => t.repo,
                        GroupSource::Folder => t.folder,
                        _ => t.generic,
                    })
                    .bold(),
            ),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format!(
                    "{} service{}",
                    group.count,
                    if group.count == 1 { "" } else { "s" }
                ),
                Style::default().fg(t.muted),
            ),
            Span::styled(
                if group.trouble > 0 {
                    format!(" · {} not answering", group.trouble)
                } else {
                    String::new()
                },
                Style::default().fg(t.server_error),
            ),
        ]),
        Line::from(""),
    ];

    if group.branch.is_some() || group.remote.is_some() {
        lines.push(section("Repository", t));
        if let Some(branch) = &group.branch {
            lines.push(kv("branch", branch, t));
        }
        if let Some(remote) = &group.remote {
            lines.push(kv("remote", remote, t));
        }
        if let Some(root) = members.first().and_then(|s| s.repo.as_ref()) {
            lines.push(kv("path", &tilde(&root.root.display().to_string()), t));
        }
        lines.push(Line::from(""));
    }

    lines.push(section("Services", t));
    for s in members {
        let (glyph, colour) = (s.health.glyph(), t.health(&s.health));
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{glyph} "), Style::default().fg(colour)),
            Span::styled(
                format!("{:<7}", s.primary_label()),
                Style::default().fg(t.text),
            ),
            Span::styled(
                truncate(&s.service_name(), 22),
                Style::default().fg(t.muted),
            ),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled("space to expand", Style::default().fg(t.faint)),
    ]));
    lines
}

fn section(name: &str, t: &Theme) -> Line<'static> {
    Line::from(vec![Span::styled(
        name.to_uppercase(),
        Style::default().fg(t.faint).add_modifier(Modifier::BOLD),
    )])
}

fn kv(key: &str, value: &str, t: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled(format!("{key:<8}"), Style::default().fg(t.faint)),
        Span::styled(value.to_string(), Style::default().fg(t.muted)),
    ])
}

fn draw_status(f2: &mut Frame, app: &App, t: &Theme, area: Rect) {
    if app.searching {
        let line = Line::from(vec![
            Span::styled(
                " filter ",
                Style::default().bg(t.accent).fg(t.background).bold(),
            ),
            Span::raw(" "),
            Span::styled(app.search.clone(), Style::default().fg(t.text)),
            Span::styled("▏", Style::default().fg(t.accent)),
            Span::styled(
                "   enter to keep · esc to clear",
                Style::default().fg(t.faint),
            ),
        ]);
        f2.render_widget(line, area);
        return;
    }

    if let Some((msg, kind, _)) = &app.toast {
        let color = match kind {
            ToastKind::Good => t.ok,
            ToastKind::Bad => t.server_error,
            ToastKind::Info => t.accent,
        };
        f2.render_widget(
            Line::from(vec![
                Span::styled(" ● ", Style::default().fg(color)),
                Span::styled(msg.clone(), Style::default().fg(color)),
            ]),
            area,
        );
        return;
    }

    if let Some(f) = &app.failure {
        let line = Line::from(vec![
            Span::styled(" ⚠ ", Style::default().fg(t.server_error).bold()),
            Span::styled(
                format!("{}  ", f.detail),
                Style::default().fg(t.server_error),
            ),
            Span::styled("d", Style::default().fg(t.accent).bold()),
            Span::styled(" diagnostics", Style::default().fg(t.faint)),
        ]);
        f2.render_widget(line, area);
        return;
    }

    // Drop whole hints that will not fit rather than letting the last one be
    // sliced through the middle of a word.
    let mut spans = vec![Span::raw(" ")];
    let mut used = 1usize;
    let room = area.width as usize;
    for (i, (key, label)) in app.keymap.footer_hints().into_iter().enumerate() {
        let gap = if i > 0 { 2 } else { 0 };
        if used + gap + key.chars().count() + 1 + label.chars().count() > room {
            break;
        }
        used += gap + key.chars().count() + 1 + label.chars().count();
        if gap > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(key, Style::default().fg(t.accent).bold()));
        spans.push(Span::styled(
            format!(" {label}"),
            Style::default().fg(t.faint),
        ));
    }
    f2.render_widget(Line::from(spans), area);
}

fn draw_help(f: &mut Frame, app: &App, t: &Theme, area: Rect) {
    // Generated from the active bindings. A help screen that lists the defaults
    // while the user runs something else is worse than no help screen.
    let rows = app.keymap.help_rows();
    let width = 68u16.min(area.width.saturating_sub(4));
    let height = (rows.len() as u16 + 4).min(area.height.saturating_sub(2));
    let popup = centered(area, width, height);

    let key_col = rows
        .iter()
        .map(|(k, _)| k.chars().count())
        .max()
        .unwrap_or(8)
        .clamp(8, 18);
    let room = (width as usize).saturating_sub(key_col + 5);

    let mut lines = vec![Line::from("")];
    for (keys, description) in rows {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format!("{keys:<key_col$}"),
                Style::default().fg(t.accent).bold(),
            ),
            Span::raw(" "),
            Span::styled(truncate(description, room), Style::default().fg(t.muted)),
        ]));
    }

    f.render_widget(Clear, popup);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(t.accent))
                .title(Span::styled(" Keys ", Style::default().fg(t.accent).bold())),
        ),
        popup,
    );
}

fn draw_diagnostics(f: &mut Frame, app: &App, t: &Theme, area: Rect) {
    let width = 88u16.min(area.width.saturating_sub(4));
    let height = 24u16.min(area.height.saturating_sub(2));
    let popup = centered(area, width, height);
    let (warns, errors) = diag::counts();

    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format!("{} warnings · {} errors", warns, errors),
                Style::default().fg(if errors > 0 { t.server_error } else { t.muted }),
            ),
        ]),
    ];
    if let Some(fail) = &app.failure {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format!("last scan failed: {}", fail.detail),
                Style::default().fg(t.server_error),
            ),
        ]));
    }
    if !app.scanner_alive {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "the scanner thread is not running — restart quarry",
                Style::default().fg(t.server_error),
            ),
        ]));
    }
    lines.push(Line::from(""));

    let room = height.saturating_sub(lines.len() as u16 + 3) as usize;
    let events = diag::recent(room);
    if events.is_empty() {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled("Nothing logged.", Style::default().fg(t.faint)),
        ]));
    }
    for e in events {
        let color = match e.level {
            diag::Level::Error => t.server_error,
            diag::Level::Warn => t.client_error,
            diag::Level::Info => t.faint,
        };
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{:<5} ", e.level), Style::default().fg(color)),
            Span::styled(format!("{:<9} ", e.scope), Style::default().fg(t.faint)),
            Span::styled(
                truncate(&e.message, width.saturating_sub(22) as usize),
                Style::default().fg(t.muted),
            ),
        ]));
    }

    f.render_widget(Clear, popup);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(t.client_error))
                .title(Span::styled(
                    " Diagnostics ",
                    Style::default().fg(t.client_error).bold(),
                )),
        ),
        popup,
    );
}

fn draw_confirm(f: &mut Frame, app: &App, t: &Theme, area: Rect) {
    let Some(c) = &app.confirm else { return };
    let popup = centered(area, 56.min(area.width.saturating_sub(4)), 7);
    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(c.prompt.clone(), Style::default().fg(t.text).bold()),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(c.detail.clone(), Style::default().fg(t.faint)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("y", Style::default().fg(t.server_error).bold()),
            Span::styled(" confirm    ", Style::default().fg(t.muted)),
            Span::styled("n / esc", Style::default().fg(t.accent).bold()),
            Span::styled(" cancel", Style::default().fg(t.muted)),
        ]),
    ];
    f.render_widget(Clear, popup);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(t.server_error)),
        ),
        popup,
    );
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

fn truncate(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        return s.to_string();
    }
    let keep = width.saturating_sub(1);
    let mut out: String = s.chars().take(keep).collect();
    out.push('…');
    out
}

pub fn ago(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0 => "just now".into(),
        1..=59 => format!("{s}s"),
        60..=3599 => format!("{}m {}s", s / 60, s % 60),
        3600..=86399 => format!("{}h {}m", s / 3600, (s % 3600) / 60),
        _ => format!("{}d {}h", s / 86400, (s % 86400) / 3600),
    }
}

fn bytes(b: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = b as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{b} B")
    } else {
        format!("{v:.0} {}", UNITS[u])
    }
}

fn tilde(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => {
            format!("~{}", &path[home.len()..])
        }
        _ => path.to_string(),
    }
}

/// Render the whole screen into plain text.
///
/// This is the seam the snapshot tests and `--screenshot` both use: it needs no
/// terminal, so a rendering regression is caught in CI rather than by eye.
pub fn render_to_string(app: &mut App, width: u16, height: u16, tick: usize) -> String {
    let buf = render_frame(app, width, height, tick);
    (0..buf.area.height)
        .map(|y| {
            let line: String = (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect();
            line.trim_end().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Render the screen as a map of foreground colours — one character per cell,
/// with a legend. Plain text says where things are; this says how they read.
pub fn render_styles_to_string(app: &mut App, width: u16, height: u16, tick: usize) -> String {
    render_map(app, width, height, tick, false)
}

/// The same, for backgrounds and modifiers. A selected row is a change of
/// ground, not of ink, so the foreground map cannot show it at all.
pub fn render_background_to_string(app: &mut App, width: u16, height: u16, tick: usize) -> String {
    render_map(app, width, height, tick, true)
}

/// Render one frame as HTML: a `<pre>` of `<span>`s carrying the real colours.
///
/// Not a screenshot and not a mock — the same drawing code that paints the
/// terminal, with the same theme file, emitted as markup. A landing page built
/// from this cannot drift from the tool, because it *is* the tool.
pub fn render_html(app: &mut App, width: u16, height: u16, tick: usize) -> String {
    let buf = render_frame(app, width, height, tick);
    let theme = app.theme.clone();
    let mut out = String::from("<pre class=\"tui\" aria-label=\"quarry running in a terminal\">");

    for y in 0..buf.area.height {
        let mut run: Option<(String, String)> = None; // (colour, text)
        for x in 0..buf.area.width {
            let cell = &buf[(x, y)];
            let colour = css_colour(cell.style().fg, &theme);
            let symbol = escape(cell.symbol());
            match &mut run {
                Some((current, text)) if *current == colour => text.push_str(&symbol),
                Some((current, text)) => {
                    push_span(&mut out, current, text);
                    run = Some((colour, symbol));
                }
                None => run = Some((colour, symbol)),
            }
        }
        if let Some((colour, text)) = run.take() {
            push_span(&mut out, &colour, &text);
        }
        if y + 1 < buf.area.height {
            out.push('\n');
        }
    }
    out.push_str("</pre>");
    out
}

fn push_span(out: &mut String, colour: &str, text: &str) {
    if text.trim().is_empty() {
        // Blank runs need no colour, and leaving them bare keeps the markup
        // roughly half the size.
        out.push_str(text);
    } else {
        out.push_str(&format!("<span style=\"color:{colour}\">{text}</span>"));
    }
}

/// A theme colour as CSS. `Reset` and the ANSI slots become custom properties,
/// so a page can supply its own values for the themes that follow the terminal.
fn css_colour(colour: Option<ratatui::style::Color>, theme: &Theme) -> String {
    use ratatui::style::Color;
    match colour.unwrap_or(Color::Reset) {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Reset => "var(--fg)".to_string(),
        Color::Indexed(n) => format!("var(--ansi-{n})"),
        named => {
            // A named ANSI colour, mapped to the slot it stands for.
            let slot = match named {
                Color::Black => 0,
                Color::Red => 1,
                Color::Green => 2,
                Color::Yellow => 3,
                Color::Blue => 4,
                Color::Magenta => 5,
                Color::Cyan => 6,
                Color::Gray => 7,
                Color::DarkGray => 8,
                Color::LightRed => 9,
                Color::LightGreen => 10,
                Color::LightYellow => 11,
                Color::LightBlue => 12,
                Color::LightMagenta => 13,
                Color::LightCyan => 14,
                Color::White => 15,
                _ => return "var(--fg)".to_string(),
            };
            let _ = theme;
            format!("var(--ansi-{slot})")
        }
    }
}

fn escape(symbol: &str) -> String {
    match symbol {
        "&" => "&amp;".to_string(),
        "<" => "&lt;".to_string(),
        ">" => "&gt;".to_string(),
        other => other.to_string(),
    }
}

fn render_map(app: &mut App, width: u16, height: u16, tick: usize, background: bool) -> String {
    let buf = render_frame(app, width, height, tick);
    let mut legend: Vec<(String, char)> = Vec::new();
    let alphabet: Vec<char> = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"
        .chars()
        .collect();

    let mut grid = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            let cell = &buf[(x, y)];
            let style = cell.style();
            let key = if background {
                // Blank cells matter here: a highlight bar is mostly padding,
                // and a bar that stops short of the edge is exactly the defect
                // this map exists to show.
                format!(
                    "{:?}{}",
                    style.bg.unwrap_or(ratatui::style::Color::Reset),
                    if style.add_modifier.contains(Modifier::REVERSED) {
                        " +reversed"
                    } else {
                        ""
                    }
                )
            } else {
                if cell.symbol().trim().is_empty() {
                    grid.push('.');
                    continue;
                }
                format!("{:?}", style.fg.unwrap_or(ratatui::style::Color::Reset))
            };
            let ch = match legend.iter().find(|(c, _)| *c == key) {
                Some((_, ch)) => *ch,
                None => {
                    let ch = *alphabet.get(legend.len()).unwrap_or(&'?');
                    legend.push((key, ch));
                    ch
                }
            };
            grid.push(ch);
        }
        grid.push('\n');
    }

    let mut out = String::from("legend:\n");
    for (color, ch) in &legend {
        out.push_str(&format!("  {ch} = {color}\n"));
    }
    out.push('\n');
    out.push_str(&grid);
    out
}

/// Draw one frame into a buffer, with no terminal involved. The seam the
/// snapshot tests, `--screenshot` and the performance guards all use.
pub fn render_frame(
    app: &mut App,
    width: u16,
    height: u16,
    tick: usize,
) -> ratatui::buffer::Buffer {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
        .expect("the test backend cannot fail to construct");
    terminal
        .draw(|f| draw(f, app, tick))
        .expect("the test backend cannot fail to draw");
    terminal.backend().buffer().clone()
}
