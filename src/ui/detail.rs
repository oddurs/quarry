//! The pane beside the list: everything known about one service, or about a
//! folded group, assembled section by section.

use super::*;

pub(super) fn draw_detail(f: &mut Frame, app: &mut App, t: &Theme, area: Rect) {
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
        let lines = match app.selected_group() {
            Some(group) => group_detail(app, group, t),
            None => vec![Line::from(Span::styled(
                "Select a service",
                Style::default().fg(t.faint),
            ))],
        };
        let empty = app.selected_group().is_none();
        let para = Paragraph::new(lines).block(block);
        f.render_widget(
            if empty {
                para.alignment(Alignment::Center)
            } else {
                para
            },
            area,
        );
        return;
    };

    // Assembled section by section, in the order they are read. Where the URL
    // lands has to be known for the mouse, and it is simply how many lines came
    // before it.
    let mut lines = detail_heading(s, t);
    let url_row = lines.len() + 1;
    lines.extend(detail_address(s, t));
    lines.extend(detail_health(s, t));
    lines.extend(detail_listening(s, t));
    lines.extend(detail_folder(s, t));
    lines.extend(detail_container(s, t));
    lines.extend(detail_repository(s, t));
    lines.extend(detail_process(s, t, app.now));
    lines.extend(detail_command(s, t));

    let inner = block.inner(area);
    if s.opens_in_a_browser() && (url_row as u16) < inner.height {
        // +2 for the leading indent on the url row; +3 on the width for the
        // arrow that follows it.
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

/// The name, and what it is in one line.
fn detail_heading(s: &Server, t: &Theme) -> Vec<Line<'static>> {
    let (dot, colour) = (s.health.glyph(), t.health(&s.health));
    vec![
        Line::from(vec![
            Span::styled(format!("{dot} "), Style::default().fg(colour)),
            Span::styled(s.title(), Style::default().fg(t.text).bold()),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(s.service_name(), Style::default().fg(t.muted)),
            Span::styled(" · ", Style::default().fg(t.faint)),
            Span::styled(s.kind.label(), Style::default().fg(t.kind(s.kind))),
            Span::styled(" · ", Style::default().fg(t.faint)),
            Span::styled(format!("pid {}", s.pid), Style::default().fg(t.muted)),
        ]),
        Line::from(""),
    ]
}

/// The row the mouse can hit.
fn detail_address(s: &Server, t: &Theme) -> Vec<Line<'static>> {
    let mut lines = vec![section("Address", t)];
    if s.opens_in_a_browser() {
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
        ]));
        // On its own line. Beside a long URI it wrapped through the middle of
        // the parenthesis, which looked like a rendering fault rather than a
        // note about what enter will do.
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "enter copies this, it cannot be opened",
                Style::default().fg(t.faint),
            ),
        ]));
    }
    lines.push(Line::from(""));
    lines
}

fn detail_health(s: &Server, t: &Theme) -> Vec<Line<'static>> {
    let (dot, colour) = (s.health.glyph(), t.health(&s.health));
    let mut lines = vec![
        section("Health", t),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{dot} "), Style::default().fg(colour)),
            Span::styled(s.health.summary(), Style::default().fg(colour)),
        ]),
    ];
    if let Health::Http { server, title, .. } = &s.health {
        lines.extend(title.as_deref().map(|page| kv("page", page, t)));
        lines.extend(server.as_deref().map(|name| kv("server", name, t)));
    }
    lines.push(Line::from(""));
    lines
}

fn detail_listening(s: &Server, t: &Theme) -> Vec<Line<'static>> {
    let mut lines = vec![section("Listening", t)];
    lines.extend(s.listeners.iter().map(|l| {
        Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{:<6}", l.port), Style::default().fg(t.text)),
            Span::styled(
                format!("{}  {}", l.addr, l.scope()),
                Style::default().fg(if l.wildcard { t.client_error } else { t.muted }),
            ),
        ])
    }));
    lines.push(Line::from(""));
    lines
}

/// Where it is running, when that is all we know — a directory that is not a
/// repository is still a project to whoever started it.
fn detail_folder(s: &Server, t: &Theme) -> Vec<Line<'static>> {
    let Some(folder) = s.folder_name().filter(|_| s.repo.is_none()) else {
        return Vec::new();
    };
    let mut lines = vec![
        section("Folder", t),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(folder, Style::default().fg(t.open).bold()),
        ]),
    ];
    lines.extend(
        s.cwd
            .as_ref()
            .map(|cwd| kv("path", &tilde(&cwd.display().to_string()), t)),
    );
    lines.push(Line::from(Span::styled(
        "  not a git repository",
        Style::default().fg(t.faint),
    )));
    lines.push(Line::from(""));
    lines
}

fn detail_container(s: &Server, t: &Theme) -> Vec<Line<'static>> {
    let Some(container) = &s.container else {
        return Vec::new();
    };
    let state = match &container.health {
        Some(h) => format!("{} ({h})", container.state),
        None => container.state.clone(),
    };
    let mut lines = vec![
        section("Container", t),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                container.display_name().to_string(),
                Style::default()
                    .fg(t.kind(crate::model::Kind::Container))
                    .bold(),
            ),
        ]),
        kv("image", &container.image, t),
        kv("state", &state, t),
    ];
    lines.extend(
        container
            .project
            .as_deref()
            .map(|project| kv("compose", project, t)),
    );
    lines.push(Line::from(""));
    lines
}

fn detail_repository(s: &Server, t: &Theme) -> Vec<Line<'static>> {
    let Some(repo) = &s.repo else {
        return Vec::new();
    };
    let mut lines = vec![
        section("Repository", t),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(repo.name.clone(), Style::default().fg(t.repo).bold()),
        ]),
        kv("path", &tilde(&repo.root.display().to_string()), t),
    ];
    lines.extend(repo.branch.as_deref().map(|b| kv("branch", b, t)));
    lines.extend(repo.remote.as_deref().map(|r| kv("remote", r, t)));
    lines.push(Line::from(""));
    lines
}

fn detail_process(s: &Server, t: &Theme, now: u64) -> Vec<Line<'static>> {
    let mut lines = vec![section("Process", t), kv("user", &s.user, t)];
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
    lines.extend(s.ppid.map(|ppid| kv("parent", &ppid.to_string(), t)));
    lines.extend(
        s.exe
            .as_ref()
            .map(|exe| kv("binary", &tilde(&exe.display().to_string()), t)),
    );
    lines.extend(
        s.cwd
            .as_ref()
            .map(|cwd| kv("cwd", &tilde(&cwd.display().to_string()), t)),
    );
    lines.push(Line::from(""));
    lines
}

fn detail_command(s: &Server, t: &Theme) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if !s.evidence.is_empty() {
        lines.push(kv("named by", &s.evidence.join(", "), t));
    }
    lines.push(section("Command", t));
    lines.push(Line::from(Span::styled(
        format!("  {}", s.cmdline),
        Style::default().fg(t.faint),
    )));
    lines
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

    // The path earns the section on its own. Narrowed to one repository the
    // group heading is already the branch, so the branch and remote are not
    // repeated here — and where you would `cd` to is the thing left to say.
    let root = members.first().and_then(|s| s.repo.as_ref());
    if group.branch.is_some() || group.remote.is_some() || root.is_some() {
        lines.push(section("Repository", t));
        if let Some(branch) = &group.branch {
            lines.push(kv("branch", branch, t));
        }
        if let Some(remote) = &group.remote {
            lines.push(kv("remote", remote, t));
        }
        if let Some(root) = root {
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
