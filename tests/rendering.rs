//! Golden screens. These are the regression net for the part of the tool that
//! is hardest to check by reasoning: what it actually looks like.

mod support;

use quarry::app::App;
use quarry::testkit;
use quarry::theme::{Source, Theme};
use quarry::ui;
use support::assert_snapshot;

/// Two hours after the fixture's processes started. Pinned, because a snapshot
/// that renders "uptime 1028d 10h" today and "1028d 11h" in an hour is a test
/// that fails for no reason.
const NOW: u64 = 1_700_000_000 + 2 * 60 * 60;

/// The theme the layout snapshots are taken under. Pinned, and pinned to a
/// *file* theme rather than to `auto`: `auto` deliberately has no opinion about
/// most roles, so it would hide a change to any of them.
const SNAPSHOT_THEME: &str = "night";

fn loaded() -> App {
    let mut app = App::new();
    app.ingest(testkit::scenario());
    app.now = NOW;
    app.theme = Theme::resolve(SNAPSHOT_THEME).expect("the snapshot theme resolves");
    app
}

#[test]
fn standard_screen() {
    let mut app = loaded();
    assert_snapshot("standard", &ui::render_to_string(&mut app, 118, 30, 0));
}

/// One colour snapshot per built-in theme, so a change to any palette lands in
/// review as a diff rather than as a surprise on somebody's screen.
#[test]
fn colours_are_stable_in_every_builtin_theme() {
    for name in ["auto", "mono", "gotham", "night", "paper"] {
        let mut app = loaded();
        app.theme = Theme::resolve(name).expect("a built-in theme resolves");
        assert_snapshot(
            &format!("colours_{name}"),
            &ui::render_styles_to_string(&mut app, 118, 30, 0),
        );
    }
}

/// `auto` exists to inherit the terminal. An absolute colour anywhere in a
/// rendered frame means it has stopped doing that.
#[test]
fn the_auto_theme_paints_nothing_absolute() {
    let mut app = loaded();
    app.theme = Theme::auto(true);
    let styles = ui::render_styles_to_string(&mut app, 118, 30, 0);
    let legend: Vec<&str> = styles
        .lines()
        .take_while(|l| !l.is_empty())
        .filter(|l| l.contains('='))
        .collect();
    assert!(!legend.is_empty(), "nothing was drawn");
    for line in legend {
        assert!(
            !line.contains("Rgb"),
            "auto painted an absolute colour: {line}"
        );
    }
}

/// The same for `mono`, which must emit no colour whatsoever.
#[test]
fn the_mono_theme_paints_no_colour() {
    let mut app = loaded();
    app.theme = Theme::mono();
    let styles = ui::render_styles_to_string(&mut app, 118, 30, 0);
    for line in styles.lines().take_while(|l| !l.is_empty()) {
        if let Some((_, colour)) = line.split_once('=') {
            assert_eq!(colour.trim(), "Reset", "mono painted {colour}");
        }
    }
}

/// Every theme quarry can resolve has to survive being drawn with.
#[test]
fn every_available_theme_renders() {
    for (name, source) in quarry::theme::available() {
        let Ok(theme) = Theme::resolve(&name) else {
            continue; // An unparseable Ghostty file is reported elsewhere.
        };
        let mut app = loaded();
        app.theme = theme;
        let screen = ui::render_to_string(&mut app, 100, 24, 0);
        assert!(
            !screen.is_empty(),
            "theme {name} ({}) rendered nothing",
            source.label()
        );
        for line in screen.lines() {
            assert!(line.chars().count() <= 100, "theme {name}: {line:?}");
        }
    }
}

/// A theme from the terminal's own files must be usable, not merely parseable.
#[test]
fn a_ghostty_theme_renders() {
    let body = include_str!("fixtures/ghostty-gotham");
    let theme = Theme::from_ghostty(body, "gotham").expect("fixture parses");
    assert_eq!(theme.source, Source::Ghostty);
    let mut app = loaded();
    app.theme = theme;
    assert_snapshot(
        "colours_ghostty_gotham",
        &ui::render_styles_to_string(&mut app, 118, 30, 0),
    );
}

#[test]
fn narrow_screen() {
    let mut app = loaded();
    assert_snapshot("narrow", &ui::render_to_string(&mut app, 64, 20, 0));
}

#[test]
fn collapsed_groups() {
    let mut app = loaded();
    app.collapsed.insert("acme-web".into());
    app.rebuild();
    assert_snapshot("collapsed", &ui::render_to_string(&mut app, 118, 24, 0));
}

/// A folded group is a selection too. The detail pane used to say "select a
/// service" beside one, which left it dead exactly when the user had just
/// collapsed something in order to look at it as a whole.
#[test]
fn a_folded_group_shows_what_is_inside_it() {
    let mut app = loaded();
    app.collapsed.insert("acme-web".into());
    app.rebuild();
    app.selected = app
        .rows
        .iter()
        .position(|r| matches!(r, quarry::app::Row::Group(_)))
        .map(|i| i + 2)
        .unwrap_or(0);
    app.clamp();
    let screen = ui::render_to_string(&mut app, 100, 20, 0);
    assert!(
        !screen.contains("Select a service"),
        "the pane is dead beside a folded group:\n{screen}"
    );
    assert_snapshot("folded_group", &screen);
}

#[test]
fn filtered() {
    let mut app = loaded();
    app.search = "redis".into();
    app.rebuild();
    assert_snapshot("filtered", &ui::render_to_string(&mut app, 118, 20, 0));
}

#[test]
fn empty_machine() {
    let mut app = App::new();
    app.ingest(Vec::new());
    app.now = NOW;
    app.scanning = false;
    assert_snapshot("empty", &ui::render_to_string(&mut app, 100, 18, 0));
}

#[test]
fn scan_failure_keeps_the_last_good_data_and_says_so() {
    let mut app = loaded();
    app.scan_failed("lsof: timed out after 4000ms".into(), true);
    assert_snapshot("degraded", &ui::render_to_string(&mut app, 118, 22, 0));
}

#[test]
fn help_overlay() {
    let mut app = loaded();
    app.help = true;
    assert_snapshot("help", &ui::render_to_string(&mut app, 100, 26, 0));
}

#[test]
fn confirm_overlay() {
    let mut app = loaded();
    app.ask_kill(false);
    assert_snapshot("confirm", &ui::render_to_string(&mut app, 100, 20, 0));
}

#[test]
fn diagnostics_overlay() {
    let mut app = loaded();
    app.scan_failed("lsof: not found".into(), false);
    app.diagnostics = true;
    // Only the framing is asserted; the event list is machine dependent.
    let screen = ui::render_to_string(&mut app, 100, 20, 0);
    assert!(screen.contains("Diagnostics"), "{screen}");
    assert!(screen.contains("lsof: not found"), "{screen}");
}

/// A render must depend only on the state handed to it. If it reads the clock
/// itself, the snapshots above rot on their own.
#[test]
fn rendering_is_deterministic() {
    let mut a = loaded();
    let mut b = loaded();
    let first = ui::render_to_string(&mut a, 118, 30, 0);
    std::thread::sleep(std::time::Duration::from_millis(50));
    let second = ui::render_to_string(&mut b, 118, 30, 0);
    assert_eq!(first, second, "the same state rendered differently twice");
}

/// The selected row, which is the thing the eye tracks while moving.
mod selection {
    use super::*;

    fn body(styles: &str) -> Vec<String> {
        styles
            .lines()
            .skip_while(|l| !l.is_empty())
            .skip(1)
            .map(str::to_string)
            .collect()
    }

    /// Reverse video swaps each cell's foreground into its background. Applied
    /// to a row whose spans are individually coloured — a green status, a
    /// magenta project, a cyan badge — it produced a bar striped in five
    /// colours. The row has to be flattened before it is inverted.
    #[test]
    fn an_inverted_row_is_one_colour() {
        let mut app = loaded();
        app.theme = Theme::auto(true);
        let styles = ui::render_styles_to_string(&mut app, 118, 20, 0);
        let rows = body(&styles);
        let selected = rows
            .iter()
            .enumerate()
            .find(|(i, _)| *i == 4)
            .map(|(_, r)| r.clone())
            .expect("the list has a fifth row");

        // Only the list pane; the detail pane beside it is not selected.
        let list = &selected[1..50];
        let inks: std::collections::HashSet<char> = list.chars().filter(|c| *c != '.').collect();
        assert!(
            inks.len() <= 2,
            "the inverted row carries {} different foregrounds, which reverse \
             video turns into that many background stripes: {inks:?}",
            inks.len()
        );
    }

    /// A theme with a real selection colour keeps its colours: there is nothing
    /// to invert, so nothing to flatten.
    #[test]
    fn a_tinted_row_keeps_its_colours() {
        let mut app = loaded();
        app.theme = Theme::resolve("gotham").expect("gotham");
        let styles = ui::render_styles_to_string(&mut app, 118, 20, 0);
        let selected = body(&styles).get(4).cloned().expect("a fifth row");
        let inks: std::collections::HashSet<char> =
            selected[1..50].chars().filter(|c| *c != '.').collect();
        assert!(
            inks.len() > 2,
            "a tinted selection should keep the row's own colours: {inks:?}"
        );
    }

    /// A bar that stops short of the pane edge, or has a gap in it, reads as a
    /// rendering fault rather than as a cursor.
    #[test]
    fn the_bar_spans_the_whole_pane_without_a_gap() {
        let mut app = loaded();
        app.theme = Theme::resolve("gotham").expect("gotham");
        let map = ui::render_background_to_string(&mut app, 118, 20, 0);
        let rows = body(&map);
        let selected = rows.get(4).expect("a fifth row");
        // Whatever the *unselected* rows are painted with is the page, so the
        // highlight is the other thing.
        let page = rows
            .get(6)
            .and_then(|r| r.chars().next())
            .expect("an unselected row");

        let run: Vec<usize> = selected
            .char_indices()
            .filter(|(_, c)| *c != page)
            .map(|(i, _)| i)
            .collect();
        assert!(!run.is_empty(), "no highlight was drawn at all");
        let (first, last) = (run[0], run[run.len() - 1]);
        assert_eq!(
            last - first + 1,
            run.len(),
            "the highlight has a hole in it: columns {first}..{last}"
        );
        assert!(
            first <= 1,
            "the bar starts at column {first}, not the pane edge"
        );
        assert!(
            last - first > 20,
            "the bar is only {} columns wide",
            last - first + 1
        );
    }

    /// A `highlight_symbol` shifts only the selected row, so the list twitches
    /// as the cursor moves. Every row reserves the column instead.
    #[test]
    fn nothing_shifts_as_the_cursor_moves() {
        let mut app = loaded();
        // The list pane only. The detail pane is *meant* to change when the
        // selection does, and measuring it would be measuring the wrong thing.
        // Character columns, not byte offsets: the gutter glyph is multi-byte,
        // so measuring bytes would report a shift every time it moved — which
        // is the very thing this is supposed to detect the absence of.
        let columns = |app: &mut App| -> Vec<usize> {
            ui::render_to_string(app, 118, 20, 0)
                .lines()
                .skip(3)
                .take(6)
                .filter_map(|line| line.chars().take(60).position(|c| c.is_ascii_digit()))
                .collect()
        };
        let before = columns(&mut app);
        app.move_by(1);
        let after = columns(&mut app);
        assert_eq!(
            before, after,
            "moving the cursor moved the text underneath it"
        );
    }

    /// Bolding a whole row on selection nudges every glyph in it.
    #[test]
    fn selection_does_not_embolden_the_row() {
        for theme in ["auto", "gotham", "night", "paper", "mono"] {
            let t = Theme::resolve(theme).expect("theme");
            assert!(
                !t.selected()
                    .add_modifier
                    .contains(ratatui::style::Modifier::BOLD),
                "{theme} bolds the selected row, which shifts its type"
            );
        }
    }

    #[test]
    fn the_marker_survives_a_screenshot_with_no_colour() {
        let mut app = loaded();
        app.theme = Theme::mono();
        let screen = ui::render_to_string(&mut app, 118, 20, 0);
        assert!(
            screen.contains('▌'),
            "with no colour at all, nothing marks the selected row:\n{screen}"
        );
    }
}

/// The keys along the bottom used to be sliced through the middle of a word on
/// a narrow terminal, which reads as a rendering fault rather than as a list
/// that did not fit.
#[test]
fn footer_hints_drop_whole_rather_than_clipping() {
    let mut app = loaded();
    for width in [30u16, 44, 58, 72, 96, 140] {
        let screen = ui::render_to_string(&mut app, width, 14, 0);
        let footer = screen.lines().last().unwrap_or_default().to_string();
        assert!(
            footer.chars().count() <= width as usize,
            "footer overflows at {width}: {footer:?}"
        );
        // Every hint that is shown is shown whole: no trailing fragment.
        for word in [
            "move", "open", "copy", "filter", "all", "stop", "refresh", "help",
        ] {
            let truncated = &word[..word.len() - 1];
            if footer.ends_with(truncated) && !footer.ends_with(word) {
                panic!("at {width} the footer ends mid-word: {footer:?}");
            }
        }
    }
}
