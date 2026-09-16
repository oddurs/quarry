//! All drawing. The screen is a title strip, a two-pane body, and a status
//! strip; overlays (help, confirm) are painted on top.

use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Clear, List, ListItem, ListState, Padding, Paragraph, Wrap,
};

use crate::app::{App, Row, ToastKind};
use crate::diag;
use crate::model::{GroupSource, Health, Server};
use crate::theme::Theme;

const SPINNER: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];

/// What the detail pane needs: two borders, an indent, and the longest line it
/// holds — a path, or `usage   0.5% cpu · 64 MB rss`.
const DETAIL_WIDTH: u16 = 46;

/// Below this the list is not worth reading, so the detail pane gives way.
const MIN_LIST: u16 = 44;

mod detail;
mod export;
mod list;
mod overlays;

use detail::draw_detail;
use list::draw_list;
use overlays::{draw_confirm, draw_diagnostics, draw_help};

pub use export::{
    render_background_to_string, render_frame, render_html, render_styles_to_string,
    render_to_string,
};

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

    // A share of the width was the wrong model. The detail pane holds short
    // key-value lines and needs about as much room whatever the terminal is;
    // half of a wide one was waste, and half of a narrow one starved the list
    // that was being read. So it takes a fixed width, the list takes the rest,
    // and on a terminal too narrow to afford both it goes away.
    let show_detail = app.detail && area.width >= DETAIL_WIDTH + MIN_LIST;
    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(if show_detail {
            [Constraint::Min(MIN_LIST), Constraint::Length(DETAIL_WIDTH)]
        } else {
            [Constraint::Percentage(100), Constraint::Length(0)]
        })
        .split(chunks[2]);

    app.list_area = split[0];
    draw_list(f, app, &t, split[0]);
    if show_detail {
        draw_detail(f, app, &t, split[1]);
    } else {
        app.url_hitbox = None;
    }
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

    // Everything after the first count is optional; a narrow terminal keeps the
    // identity and the number, and drops the rest rather than colliding with
    // the clock on the right.
    let roomy = area.width >= 78;
    let mut left = vec![
        Span::styled(" quarry", Style::default().fg(t.accent).bold()),
        Span::raw("  "),
    ];

    // Scoped, the repository is the headline: it is the answer to "which
    // project am I looking at", and without it the screen is indistinguishable
    // from a machine that happens to be quiet.
    if let Some(scope) = app.scoped() {
        left.push(Span::styled(
            scope.name.clone(),
            Style::default().fg(t.text).bold(),
        ));
        if roomy && let Some(remote) = &scope.remote {
            left.push(Span::styled(
                format!(" {remote}"),
                Style::default().fg(t.faint),
            ));
        }
        left.push(Span::raw("  "));
    }

    let mut tallies = vec![tally(services, "listening", t.text, t.muted)];
    if roomy {
        // Inside one repository the groups are its worktrees, so calling them
        // projects would be a different claim than the screen is making.
        let unit = match (app.scoped().is_some(), projects == 1) {
            (true, true) => "worktree",
            (true, false) => "worktrees",
            (false, true) => "project",
            (false, false) => "projects",
        };
        tallies.push(tally(projects, unit, t.text, t.muted));
        if trouble > 0 {
            tallies.push(tally(trouble, "unhealthy", t.server_error, t.server_error));
        }
        if !app.search.is_empty() {
            // With what it is hiding. "3 listening" beside a filter is
            // ambiguous between a quiet machine and a narrow filter, and those
            // two call for opposite reactions.
            let hidden = app.servers.len().saturating_sub(services);
            let text = match hidden {
                0 => format!("“{}”", app.search),
                n => format!("“{}” · {n} hidden", app.search),
            };
            tallies.push(vec![Span::styled(
                text,
                Style::default().fg(t.client_error),
            )]);
        }
    }
    for (i, tally) in tallies.into_iter().enumerate() {
        if i > 0 {
            left.push(Span::styled(" · ", Style::default().fg(t.faint)));
        }
        left.extend(tally);
    }

    f.render_widget(Line::from(left), area);
    let style = if app.is_stale() {
        Style::default().fg(t.server_error).bold()
    } else {
        Style::default().fg(t.muted)
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(scan_status(app, tick), style)))
            .alignment(Alignment::Right),
        area,
    );
}

/// `4 projects` — a number and what it counts, which is the shape of every
/// entry along the top.
fn tally(n: usize, label: &str, number: Color, word: Color) -> Vec<Span<'static>> {
    vec![
        Span::styled(n.to_string(), Style::default().fg(number).bold()),
        Span::styled(format!(" {label}"), Style::default().fg(word)),
    ]
}

/// What the scanner is doing, in the corner. Trailing space: it is drawn
/// right-aligned and would otherwise sit against the edge.
fn scan_status(app: &App, tick: usize) -> String {
    if let Some(f) = &app.failure {
        return format!(
            "⚠ scan failing ({}×){} ",
            f.count,
            if f.transient { ", retrying" } else { "" }
        );
    }
    if !app.scanner_alive {
        return "⚠ scanner stopped ".to_string();
    }
    if app.scanning {
        return format!("{} scanning ", SPINNER[tick % SPINNER.len()]);
    }
    match app.last_scan.map(|t| t.elapsed()) {
        None => "starting ".to_string(),
        Some(e) if e.as_secs() == 0 => "updated just now ".to_string(),
        Some(e) => format!("updated {} ago ", ago(e)),
    }
}

fn draw_rule(f: &mut Frame, t: &Theme, area: Rect) {
    let rule = "─".repeat(area.width as usize);
    f.render_widget(
        Line::from(Span::styled(rule, Style::default().fg(t.faint))),
        area,
    );
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
                // The syntax is worth teaching, and an empty box is the only
                // moment it is not in the way.
                if app.search.is_empty() {
                    "   :port  @kind  ~project  or any words"
                } else {
                    "   enter to keep · esc to clear"
                },
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

    // Which hints fit is decided by the keymap, which knows which of them
    // matter; the leading space is this renderer's, so it comes off the room.
    let mut spans = vec![Span::raw(" ")];
    let room = area.width.saturating_sub(1) as usize;
    for (i, (key, label)) in app.keymap.footer_hints(room).into_iter().enumerate() {
        if i > 0 {
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
