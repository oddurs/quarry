//! Rendering a frame to something other than a terminal.
//!
//! Text for the snapshot tests, and markup for the landing page. Not a
//! screenshot and not a mock — the same drawing code, the same theme file,
//! emitted differently, so neither can drift from the tool.

use super::*;

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
    // The page's own ground, so the markup does not depend on whatever it is
    // dropped into. A light theme rendered onto a dark page is not that theme.
    let ground = css_colour(Some(theme.background), &theme);
    let mut out = format!(
        "<pre class=\"tui\" style=\"background:{ground}\" \
         aria-label=\"quarry running in a terminal\">"
    );

    for y in 0..buf.area.height {
        let mut run: Option<(Ink, String)> = None;
        for x in 0..buf.area.width {
            let cell = &buf[(x, y)];
            let style = cell.style();
            // Backgrounds are not decoration here: the selected row and a
            // service that has just appeared are *only* a change of ground, so
            // markup that carries the foreground alone shows neither of them.
            let ink = Ink {
                fg: css_colour(style.fg, &theme),
                bg: (style.bg.unwrap_or(ratatui::style::Color::Reset)
                    != ratatui::style::Color::Reset)
                    .then(|| css_colour(style.bg, &theme)),
            };
            let symbol = escape(cell.symbol());
            match &mut run {
                Some((current, text)) if *current == ink => text.push_str(&symbol),
                Some((current, text)) => {
                    push_span(&mut out, current, text);
                    run = Some((ink, symbol));
                }
                None => run = Some((ink, symbol)),
            }
        }
        if let Some((ink, text)) = run.take() {
            push_span(&mut out, &ink, &text);
        }
        if y + 1 < buf.area.height {
            out.push('\n');
        }
    }
    out.push_str("</pre>");
    out
}

/// What a run of cells is painted with.
#[derive(PartialEq, Eq)]
struct Ink {
    fg: String,
    /// `None` for the page's own ground, which needs no markup.
    bg: Option<String>,
}

fn push_span(out: &mut String, ink: &Ink, text: &str) {
    match (&ink.bg, text.trim().is_empty()) {
        // Blank runs on the page's own ground need no colour, and leaving them
        // bare keeps the markup roughly half the size.
        (None, true) => out.push_str(text),
        (None, false) => out.push_str(&format!("<span style=\"color:{}\">{text}</span>", ink.fg)),
        // A run on its own ground is kept whether or not it has glyphs in it: a
        // highlight bar is mostly padding, and dropping the blanks would leave
        // it in pieces.
        (Some(bg), _) => out.push_str(&format!(
            "<span style=\"color:{};background:{bg}\">{text}</span>",
            ink.fg
        )),
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
