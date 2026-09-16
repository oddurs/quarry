//! Help, diagnostics and the confirmation, painted over whatever is beneath.

use super::*;

pub(super) fn draw_help(f: &mut Frame, app: &App, t: &Theme, area: Rect) {
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

pub(super) fn draw_diagnostics(f: &mut Frame, app: &App, t: &Theme, area: Rect) {
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

pub(super) fn draw_confirm(f: &mut Frame, app: &App, t: &Theme, area: Rect) {
    let Some(c) = &app.confirm else { return };
    let popup = centered(area, 56.min(area.width.saturating_sub(4)), 7);
    // Two borders and the indent. Without this a long command line runs
    // straight through the right-hand border, and the box the user is being
    // asked to answer looks broken.
    let room = popup.width.saturating_sub(4) as usize;
    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                truncate(&c.prompt, room),
                Style::default().fg(t.text).bold(),
            ),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(truncate(&c.detail, room), Style::default().fg(t.faint)),
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
