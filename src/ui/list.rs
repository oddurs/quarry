//! The list of services: the pane that is actually read.

use super::*;

pub(super) fn draw_list(f: &mut Frame, app: &mut App, t: &Theme, area: Rect) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(t.faint))
        .title(Line::from({
            let mut title = vec![Span::styled(
                " Services ",
                Style::default().fg(t.text).bold(),
            )];
            // A mode you cannot see you are in is a bug report waiting to
            // happen, and the pane's own title is where it belongs: beside the
            // thing it rearranged, not in a status bar across the screen.
            if let Some(how) = app.arrangement() {
                title.push(Span::styled(
                    format!("· {how} "),
                    Style::default().fg(t.faint),
                ));
            }
            title
        }));

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

    // Sized from every visible row rather than from the window, so the column
    // does not change width as you scroll past a long socket name.
    let label_width = app
        .rows
        .iter()
        .filter_map(|r| match r {
            Row::Server(i) => Some(app.servers[*i].primary_column().chars().count()),
            Row::Group(_) => None,
        })
        .max()
        .unwrap_or(PORT_WIDTH)
        // A third of the row, but never less than a port: on a pane four
        // columns wide the cap would otherwise fall below the floor, and
        // `clamp` is entitled to panic when it does.
        .clamp(
            PORT_WIDTH,
            LABEL_MAX.min(inner_width.min(ROW_MAX) / 3).max(PORT_WIDTH),
        );

    let cursor = app.selected.saturating_sub(app.offset);
    let items: Vec<ListItem> = window
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let selected = i == cursor;
            // Capped for both, or a folded group's count would sit against the
            // pane edge while the rows beneath it stopped short.
            let row_width = inner_width.min(ROW_MAX);
            let line = match row {
                Row::Group(g) => group_line(app, t, *g, row_width, selected),
                Row::Server(s) => {
                    server_line(&app.servers[*s], t, row_width, label_width, selected)
                }
            };
            // Never both: two highlights on one line is one too many, and the
            // cursor is the one that has to win.
            let fresh = !selected
                && matches!(row, Row::Server(i) if is_fresh(&app.servers[*i]))
                && t.tints_arrivals();
            // Reverse video inverts whatever colour it finds, so a row of many
            // colours becomes a bar striped in as many. Flattened first, it
            // becomes what every terminal list looks like: one solid block.
            let item = ListItem::new(if selected && t.selection_inverts() {
                flatten(line)
            } else {
                line
            });
            match t.fresh().filter(|_| fresh) {
                Some(style) => item.style(style),
                None => item,
            }
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

/// The widest a row's content is allowed to get.
///
/// A row carries a port, a name, a kind and a status — around seventy columns
/// of information. Stretched across a wide pane the name ends up on the far
/// left and the status on the far right with nothing in between, and the eye
/// has to cross the gap on every line. Past this, the pane gets a right margin
/// instead.
const ROW_MAX: usize = 72;

/// Five digits, which is every port there is.
const PORT_WIDTH: usize = 5;

/// The widest the leftmost column may get when a unix socket is on screen.
/// Enough of `local-27472-81dd8dbc70.ctrl.sock` to tell it from its neighbour,
/// and not so much that it is the only thing on the line.
const LABEL_MAX: usize = 18;

fn group_line(app: &App, t: &Theme, idx: usize, width: usize, selected: bool) -> Line<'static> {
    let g = &app.groups[idx];
    let collapsed = app.collapsed.contains(&g.key);
    let name = match g.source {
        GroupSource::Unattributed if g.describes_a_project => "no project".to_string(),
        _ if !g.describes_a_project => g.key.clone(),
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

/// How long a newly-appeared service stays highlighted.
///
/// Measured from the other end: you start a server, watch it boot, and switch
/// to quarry. That is ten or fifteen seconds on a slow one, so anything much
/// shorter is a highlight you arrive too late to see.
const FRESH: Duration = Duration::from_secs(30);

/// Has this appeared recently enough to still be worth pointing at?
fn is_fresh(s: &Server) -> bool {
    s.appeared.is_some_and(|at| at.elapsed() < FRESH)
}

fn server_line(
    s: &Server,
    t: &Theme,
    width: usize,
    label_width: usize,
    selected: bool,
) -> Line<'static> {
    let (dot, dot_color) = (s.health.glyph(), t.health(&s.health));
    // The column between the selection bar and the health dot was already a
    // blank space, so marking an arrival costs no width and shifts nothing.
    let fresh = if is_fresh(s) {
        Span::styled("+", Style::default().fg(t.accent).bold())
    } else {
        Span::raw(" ")
    };
    // Right-aligned in a width the table chose, not one this value chose. A
    // port is five digits; a unix socket's name is as long as someone felt
    // like making it, and letting it size the column pushed the name, the kind
    // and the status off their lines for every other row on screen.
    let port = format!(
        "{:>label_width$}",
        truncate(&s.primary_column(), label_width)
    );
    let extra_ports = if s.listeners.len() > 1 {
        format!(" +{}", s.listeners.len() - 1)
    } else {
        String::new()
    };
    let badge = s.kind.label();
    let (status_full, status_color) = status_cell(&s.health, t);
    let status_short = status_abbrev(&s.health);

    // "  " + dot + " " + port, then one trailing space at the end. The count
    // of further listeners is deliberately not here: it appears on one row in
    // twenty, and in the lead it moved that row's name column out of line with
    // every other row on screen. It is spent out of the name's budget instead.
    let lead = 2 + 1 + 1 + port.chars().count() + 1;
    let avail = width.saturating_sub(lead + 1);

    // The name is the row. Everything else is an attribute of it, so
    // everything else goes first: the kind — which the colour and often the
    // name already imply — then the latency. `Sync…storage` is not a row worth
    // keeping a badge for.
    const MIN_NAME: usize = 14;
    // One space of it is the gap before whatever follows, so a name that fills
    // its column does not run into the badge.
    const GAP: usize = 1;
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
    let name = truncate(
        &s.service_name(),
        name_width
            .saturating_sub(GAP)
            .saturating_sub(extra_ports.chars().count()),
    );
    let pad = name_width
        .saturating_sub(name.chars().count())
        .saturating_sub(extra_ports.chars().count());

    let mut spans = vec![
        gutter(selected, t),
        fresh,
        Span::styled(dot, Style::default().fg(dot_color)),
        Span::raw(" "),
        Span::styled(port, Style::default().fg(t.text).bold()),
        Span::raw(" "),
        Span::styled(name, Style::default().fg(t.text)),
        Span::styled(extra_ports, Style::default().fg(t.faint)),
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
        Health::Starting => "up…".into(),
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
        // The latency was measured; dropping it read as "not checked" rather
        // than "checked, and not HTTP". Padded to the width of a status line
        // so the column does not move.
        Health::Open { latency } => (format!("open{:>6}", crate::model::fmt_ms(*latency)), t.open),
        Health::Bound => ("bound     ".to_string(), t.open),
        Health::Starting => ("starting  ".to_string(), t.starting),
        Health::Closed => ("no answer ".to_string(), t.server_error),
        Health::Unknown => ("···       ".to_string(), t.faint),
    }
}
