//! Colour themes.
//!
//! A theme is a set of *roles* — what a colour is for, not what colour it is —
//! so a palette can be swapped without touching a line of drawing code. Themes
//! come from four places, all through the same parser:
//!
//!   1. `auto`, which maps every role onto the terminal's own ANSI palette and
//!      is the default: quarry should look like the terminal it runs in, not
//!      like somebody else's screenshot;
//!   2. the built-in files below, compiled in and parsed like any other, so a
//!      built-in cannot drift from the format users write;
//!   3. `.toml` files in `~/.config/quarry/themes/`;
//!   4. **Ghostty theme files**, read directly. If you have already picked a
//!      theme for your terminal, quarry can wear it rather than making you
//!      transcribe it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;

use crate::model::Kind;

/// Theme files shipped with quarry.
const BUILTIN: &[(&str, &str)] = &[
    ("gotham", include_str!("../themes/gotham.toml")),
    ("night", include_str!("../themes/night.toml")),
    ("paper", include_str!("../themes/paper.toml")),
];

pub const DEFAULT: &str = "auto";

/// The names that resolve without touching the filesystem.
pub const SPECIAL: &[&str] = &["auto", "mono"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    Auto,
    Builtin,
    User,
    Ghostty,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Auto => "terminal",
            Source::Builtin => "built-in",
            Source::User => "user",
            Source::Ghostty => "ghostty",
        }
    }
}

/// Every colour the interface can use, named by purpose.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub name: String,
    pub source: Source,
    pub dark: bool,

    // Structure.
    pub background: Color,
    pub surface: Color,
    pub overlay: Color,
    pub border: Color,
    pub border_focus: Color,
    pub selection: Color,
    /// Draw the selected row in reverse video instead of on `selection`. The
    /// only way to mark a row legibly when the ground colour is unknown.
    pub selection_reverse: bool,

    // Text.
    pub text: Color,
    pub muted: Color,
    pub faint: Color,
    pub heading: Color,

    // Emphasis.
    pub accent: Color,
    pub secondary: Color,

    // Health.
    pub ok: Color,
    pub redirect: Color,
    pub client_error: Color,
    /// Answered, and asked who you are. Healthy, and not to be confused with a
    /// 404 — a service behind auth is working exactly as intended.
    pub protected: Color,
    pub server_error: Color,
    pub open: Color,
    pub closed: Color,
    pub unknown: Color,

    // Where a project's name came from.
    pub repo: Color,
    pub folder: Color,
    pub generic: Color,

    /// One colour per [`Kind`], indexed by [`Kind::index`].
    pub kinds: [Color; 25],
}

impl Theme {
    pub fn kind(&self, kind: Kind) -> Color {
        self.kinds[kind.index()]
    }

    /// The style for the selected row.
    ///
    /// Deliberately not bold: bolding a whole row on selection nudges every
    /// glyph in it, so the text appears to shift as the cursor moves. The bar
    /// and the gutter mark the row; the type stays where it is.
    pub fn selected(&self) -> Style {
        if self.selection_reverse {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default().bg(self.selection)
        }
    }

    /// Whether the selected row is drawn by inverting it.
    ///
    /// Reverse video swaps each cell's foreground into its background. On a row
    /// whose spans are individually coloured — a green status, a magenta repo,
    /// a cyan badge — that produces a bar striped in five different colours,
    /// which is what it looked like. Reverse may only be applied to a row of
    /// one colour, so the caller has to flatten it first.
    pub fn selection_inverts(&self) -> bool {
        self.selection_reverse
    }

    /// Health colour, from the same decision the glyph is made from.
    pub fn health(&self, health: &crate::model::Health) -> Color {
        use crate::model::Health;
        if health.is_protected() {
            return self.protected;
        }
        match health {
            Health::Unknown => self.unknown,
            Health::Bound => self.open,
            Health::Open { .. } => self.open,
            Health::Closed => self.closed,
            Health::Http { status, .. } => match status {
                200..=299 => self.ok,
                300..=399 => self.redirect,
                400..=499 => self.client_error,
                _ => self.server_error,
            },
        }
    }

    /// The terminal's own palette.
    ///
    /// Every role is an ANSI slot or `Reset`, never an RGB value — that is the
    /// whole point. The terminal substitutes its configured colours, so quarry
    /// matches whatever theme is already on screen and follows it when it
    /// changes. The cost is that roles wanting a shade *between* two slots do
    /// not get one: `surface` and `background` are the same, and selection is
    /// reverse video. Correct in every terminal beats ideal in one.
    pub fn auto(dark: bool) -> Theme {
        Theme {
            name: "auto".into(),
            source: Source::Auto,
            dark,
            background: Color::Reset,
            surface: Color::Reset,
            overlay: Color::Reset,
            border: Color::DarkGray,
            border_focus: Color::Blue,
            selection: Color::Reset,
            selection_reverse: true,
            text: Color::Reset,
            muted: Color::Gray,
            faint: Color::DarkGray,
            heading: Color::Reset,
            accent: Color::Blue,
            secondary: Color::Cyan,
            ok: Color::Green,
            redirect: Color::Cyan,
            client_error: Color::Yellow,
            protected: Color::LightBlue,
            server_error: Color::Red,
            open: Color::Cyan,
            closed: Color::Red,
            unknown: Color::DarkGray,
            repo: Color::Magenta,
            folder: Color::Cyan,
            generic: Color::DarkGray,
            kinds: [Color::Reset; 25],
        }
        .with_derived_kinds()
    }

    /// Give every kind a colour derived from the theme's own roles.
    ///
    /// A theme file may then override any of them, but never *has* to: adding a
    /// kind to quarry must not be able to leave a theme with a hole in it, and
    /// a user's three-line theme should still colour twenty-five badges
    /// sensibly.
    fn with_derived_kinds(mut self) -> Theme {
        use Kind::*;
        for kind in Kind::ALL {
            self.kinds[kind.index()] = match kind {
                Web | Emulator => self.secondary,
                Api | Container | Metrics => self.accent,
                Database | Vector | Ai | Auth => self.repo,
                Cache | Queue | Workflow => self.client_error,
                Search | Storage | Mail | Notebook | Realtime | Proxy => self.open,
                Registry | Game => self.folder,
                // A debugger left attached and a tunnel to the public internet
                // are both worth noticing, so both borrow the alarm colour.
                Debug | Tunnel => self.server_error,
                DevTool | System => self.faint,
                Other => self.muted,
            };
        }
        self
    }

    /// No colour at all. Emphasis is carried by bold, dim and reverse, which is
    /// what `NO_COLOR`, `TERM=dumb` and a colour-blind reader all need.
    pub fn mono() -> Theme {
        let r = Color::Reset;
        Theme {
            name: "mono".into(),
            source: Source::Auto,
            dark: true,
            background: r,
            surface: r,
            overlay: r,
            border: r,
            border_focus: r,
            selection: r,
            selection_reverse: true,
            text: r,
            muted: r,
            faint: r,
            heading: r,
            accent: r,
            secondary: r,
            ok: r,
            redirect: r,
            client_error: r,
            protected: r,
            server_error: r,
            open: r,
            closed: r,
            unknown: r,
            repo: r,
            folder: r,
            generic: r,
            kinds: [r; 25],
        }
    }

    /// Fill in from a parsed file, leaving unmentioned roles as they are. A
    /// theme file that only changes the accent is valid and useful.
    fn apply(mut self, file: ThemeFile, name: String, source: Source) -> Theme {
        #![allow(clippy::needless_late_init)]
        macro_rules! set {
            ($($field:ident),* $(,)?) => {
                $(if let Some(v) = file.$field.as_deref().and_then(parse_color) {
                    self.$field = v;
                })*
            };
        }
        set!(
            background,
            surface,
            overlay,
            border,
            border_focus,
            selection,
            text,
            muted,
            faint,
            heading,
            accent,
            secondary,
            ok,
            redirect,
            client_error,
            protected,
            server_error,
            open,
            closed,
            unknown,
            repo,
            folder,
            generic,
        );
        if let Some(dark) = file.dark {
            self.dark = dark;
        }
        if let Some(rev) = file.selection_reverse {
            self.selection_reverse = rev;
        } else if file.selection.is_some() {
            // A file that names a selection colour means it to be used.
            self.selection_reverse = false;
        }
        // Roles have changed, so the derived kind colours have too. Re-derive
        // before applying the file's explicit overrides, or a theme that sets
        // `accent` would leave every kind on the previous theme's hues.
        let mono = self.kinds.iter().all(|c| *c == Color::Reset);
        if !mono {
            self = self.with_derived_kinds();
        }
        for kind in Kind::ALL {
            if let Some(v) = file.kinds.get(kind.label()).and_then(|s| parse_color(s)) {
                self.kinds[kind.index()] = v;
            }
        }
        self.name = file.name.unwrap_or(name);
        self.source = source;
        self
    }

    /// Parse a quarry theme file.
    pub fn from_toml(body: &str, name: &str, source: Source) -> Result<Theme, ThemeError> {
        let file: ThemeFile = toml::from_str(body).map_err(|e| ThemeError::Parse {
            name: name.to_string(),
            detail: e.to_string(),
        })?;
        // A theme starts from a readable base so an incomplete file cannot
        // produce an unreadable screen.
        let base = if file.dark == Some(false) {
            Theme::auto(false)
        } else {
            Theme::auto(true)
        };
        Ok(base.apply(file, name.to_string(), source))
    }

    /// Read a Ghostty theme file: `background`, `foreground`, `palette = N=#hex`.
    pub fn from_ghostty(body: &str, name: &str) -> Result<Theme, ThemeError> {
        let mut palette: BTreeMap<u8, String> = BTreeMap::new();
        let mut keys: BTreeMap<&str, String> = BTreeMap::new();
        for line in body.lines() {
            let line = line.trim();
            // A leading '#' is a comment; a '#' inside a value is a colour.
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            if key == "palette" {
                if let Some((slot, colour)) = value.split_once('=')
                    && let Ok(slot) = slot.trim().parse::<u8>()
                {
                    palette.insert(slot, colour.trim().to_string());
                }
            } else {
                let mapped = match key {
                    "background" => "background",
                    "foreground" => "foreground",
                    "selection-background" => "selection",
                    _ => continue,
                };
                keys.insert(mapped, value.to_string());
            }
        }
        if !keys.contains_key("background") && palette.is_empty() {
            return Err(ThemeError::NotGhostty {
                name: name.to_string(),
            });
        }

        let slot = |n: u8| palette.get(&n).cloned();
        // Bright first: the official Gotham port fills its bright slots with
        // background shades, and a port that does that is exactly the one worth
        // honouring rather than second-guessing.
        let pick = |bright: u8, normal: u8| slot(bright).or_else(|| slot(normal));

        let background = keys.get("background").cloned().or_else(|| slot(0));
        let foreground = keys.get("foreground").cloned().or_else(|| slot(7));
        let dark = background
            .as_deref()
            .and_then(parse_color)
            .map(is_dark)
            .unwrap_or(true);

        let file = ThemeFile {
            name: Some(name.to_string()),
            dark: Some(dark),
            background: background.clone(),
            surface: slot(0).or_else(|| background.clone()),
            overlay: slot(8).or_else(|| slot(0)),
            border: slot(8),
            border_focus: pick(12, 4),
            selection: keys.get("selection").cloned().or_else(|| slot(8)),
            selection_reverse: Some(false),
            text: foreground.clone(),
            muted: pick(15, 7),
            faint: pick(8, 0),
            heading: foreground,
            accent: pick(11, 3),
            secondary: pick(14, 6),
            ok: pick(10, 2),
            redirect: pick(14, 6),
            client_error: pick(11, 3),
            protected: pick(12, 4),
            server_error: pick(9, 1),
            open: pick(14, 6),
            closed: pick(9, 1),
            unknown: slot(8),
            repo: pick(13, 5),
            folder: pick(14, 6),
            generic: slot(8),
            kinds: BTreeMap::from_iter(
                [
                    ("web", pick(14, 6)),
                    ("api", pick(12, 4)),
                    ("db", pick(13, 5)),
                    ("cache", pick(11, 3)),
                    ("search", pick(14, 6)),
                    ("queue", pick(11, 3)),
                    ("proxy", pick(10, 2)),
                    ("mail", pick(14, 6)),
                    ("container", pick(12, 4)),
                    ("ai", pick(13, 5)),
                    ("tool", slot(8)),
                    ("system", slot(8)),
                    ("other", pick(15, 7)),
                ]
                .into_iter()
                .filter_map(|(k, v)| v.map(|v| (k.to_string(), v))),
            ),
        };
        Ok(Theme::auto(dark).apply(file, name.to_string(), Source::Ghostty))
    }

    /// Resolve a theme spec: `auto`, `mono`, a built-in name, `ghostty:<name>`,
    /// a user theme name, or a path.
    pub fn resolve(spec: &str) -> Result<Theme, ThemeError> {
        let spec = spec.trim();
        if spec.is_empty() || spec == "auto" {
            return Ok(Theme::auto(true));
        }
        if spec == "mono" || spec == "none" {
            return Ok(Theme::mono());
        }

        if let Some(name) = spec.strip_prefix("ghostty:") {
            return load_ghostty(name);
        }

        if let Some((_, body)) = BUILTIN.iter().find(|(n, _)| *n == spec) {
            return Theme::from_toml(body, spec, Source::Builtin);
        }

        // A path, if it really is one. Not merely "has a dot in it": Ghostty
        // ships themes called `Hopscotch.256`, and treating those as filenames
        // made them unresolvable.
        let as_path = Path::new(spec);
        if spec.contains('/') || as_path.is_file() {
            let body = std::fs::read_to_string(as_path).map_err(|e| ThemeError::Io {
                name: spec.to_string(),
                detail: e.to_string(),
            })?;
            return parse_either(&body, spec, Source::User);
        }

        // A user theme.
        let user = user_theme_dir().join(format!("{spec}.toml"));
        if user.is_file() {
            let body = std::fs::read_to_string(&user).map_err(|e| ThemeError::Io {
                name: spec.to_string(),
                detail: e.to_string(),
            })?;
            return parse_either(&body, spec, Source::User);
        }

        // Whatever the terminal already has under that name.
        load_ghostty(spec)
    }

    /// Resolve, and fall back to `auto` rather than refusing to start. Returns
    /// the error so the caller can report it where the user will see it.
    pub fn resolve_or_default(spec: &str) -> (Theme, Option<ThemeError>) {
        match Theme::resolve(spec) {
            Ok(t) => (t, None),
            Err(e) => (Theme::auto(true), Some(e)),
        }
    }
}

fn parse_either(body: &str, name: &str, source: Source) -> Result<Theme, ThemeError> {
    // A Ghostty theme has no section headers and uses `palette =` lines.
    if body.contains("palette") && !body.contains('[') {
        return Theme::from_ghostty(body, name);
    }
    Theme::from_toml(body, name, source)
}

fn load_ghostty(name: &str) -> Result<Theme, ThemeError> {
    for dir in ghostty_dirs() {
        let path = dir.join(name);
        if path.is_file() {
            let body = std::fs::read_to_string(&path).map_err(|e| ThemeError::Io {
                name: name.to_string(),
                detail: e.to_string(),
            })?;
            return Theme::from_ghostty(&body, name);
        }
    }
    Err(ThemeError::NotFound {
        name: name.to_string(),
    })
}

/// Every theme that can be resolved right now, with where it came from.
pub fn available() -> Vec<(String, Source)> {
    let mut out: Vec<(String, Source)> = vec![
        ("auto".to_string(), Source::Auto),
        ("mono".to_string(), Source::Auto),
    ];
    for (name, _) in BUILTIN {
        out.push((name.to_string(), Source::Builtin));
    }
    collect_dir(&user_theme_dir(), Source::User, &mut out);
    for dir in ghostty_dirs() {
        collect_dir(&dir, Source::Ghostty, &mut out);
    }
    out.dedup_by(|a, b| a.0 == b.0);
    out
}

fn collect_dir(dir: &Path, source: Source, out: &mut Vec<(String, Source)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|e| e.path().is_file())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if source == Source::User {
                name.strip_suffix(".toml").map(str::to_string)
            } else {
                Some(name)
            }
        })
        .filter(|n| !n.starts_with('.'))
        .collect();
    names.sort();
    for name in names {
        if !out.iter().any(|(n, _)| *n == name) {
            out.push((name, source));
        }
    }
}

pub fn config_dir() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(xdg).join("quarry");
    }
    match std::env::var_os("HOME") {
        Some(home) => PathBuf::from(home).join(".config/quarry"),
        None => PathBuf::from(".config/quarry"),
    }
}

pub fn user_theme_dir() -> PathBuf {
    config_dir().join("themes")
}

fn ghostty_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        dirs.push(PathBuf::from(xdg).join("ghostty/themes"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(&home).join(".config/ghostty/themes"));
    }
    dirs.push(PathBuf::from(
        "/Applications/Ghostty.app/Contents/Resources/ghostty/themes",
    ));
    dirs.push(PathBuf::from("/usr/share/ghostty/themes"));
    dirs
}

#[derive(Debug, Clone, PartialEq)]
pub enum ThemeError {
    NotFound { name: String },
    NotGhostty { name: String },
    Parse { name: String, detail: String },
    Io { name: String, detail: String },
}

impl std::fmt::Display for ThemeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ThemeError::NotFound { name } => write!(f, "no theme called {name:?}"),
            ThemeError::NotGhostty { name } => {
                write!(f, "{name:?} is not a theme file quarry understands")
            }
            ThemeError::Parse { name, detail } => {
                write!(f, "theme {name:?}: {}", detail.lines().next().unwrap_or(""))
            }
            ThemeError::Io { name, detail } => write!(f, "theme {name:?}: {detail}"),
        }
    }
}

impl std::error::Error for ThemeError {}

/// The on-disk form. Every colour optional, so a file states only what it means
/// to change.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeFile {
    name: Option<String>,
    dark: Option<bool>,
    selection_reverse: Option<bool>,
    background: Option<String>,
    surface: Option<String>,
    overlay: Option<String>,
    border: Option<String>,
    border_focus: Option<String>,
    selection: Option<String>,
    text: Option<String>,
    muted: Option<String>,
    faint: Option<String>,
    heading: Option<String>,
    accent: Option<String>,
    secondary: Option<String>,
    ok: Option<String>,
    redirect: Option<String>,
    client_error: Option<String>,
    protected: Option<String>,
    server_error: Option<String>,
    open: Option<String>,
    closed: Option<String>,
    unknown: Option<String>,
    repo: Option<String>,
    folder: Option<String>,
    generic: Option<String>,
    #[serde(default)]
    kinds: BTreeMap<String, String>,
}

/// Accepts `#rrggbb`, `#rgb`, `rgb:RR/GG/BB` as xterm writes it, an ANSI index,
/// a colour name, and `reset` for "whatever the terminal already uses".
pub fn parse_color(raw: &str) -> Option<Color> {
    let s = raw.trim().trim_matches('"');
    if s.is_empty() {
        return None;
    }
    let lower = s.to_ascii_lowercase();

    match lower.as_str() {
        "reset" | "default" | "terminal" | "none" => return Some(Color::Reset),
        "black" => return Some(Color::Black),
        "red" => return Some(Color::Red),
        "green" => return Some(Color::Green),
        "yellow" => return Some(Color::Yellow),
        "blue" => return Some(Color::Blue),
        "magenta" | "purple" => return Some(Color::Magenta),
        "cyan" => return Some(Color::Cyan),
        "white" => return Some(Color::Gray),
        "bright-black" | "gray" | "grey" => return Some(Color::DarkGray),
        "bright-red" => return Some(Color::LightRed),
        "bright-green" => return Some(Color::LightGreen),
        "bright-yellow" => return Some(Color::LightYellow),
        "bright-blue" => return Some(Color::LightBlue),
        "bright-magenta" => return Some(Color::LightMagenta),
        "bright-cyan" => return Some(Color::LightCyan),
        "bright-white" => return Some(Color::White),
        _ => {}
    }

    if let Some(rest) = lower.strip_prefix("ansi:") {
        return rest.trim().parse::<u8>().ok().map(Color::Indexed);
    }
    // A bare small number is an ANSI slot, which is how a theme asks to follow
    // the terminal for one particular role.
    if let Ok(n) = lower.parse::<u8>() {
        return Some(Color::Indexed(n));
    }

    if let Some(rest) = lower.strip_prefix("rgb:") {
        let parts: Vec<&str> = rest.split('/').collect();
        if parts.len() == 3 {
            let c = |p: &str| u16::from_str_radix(p, 16).ok().map(|v| scale(v, p.len()));
            if let (Some(r), Some(g), Some(b)) = (c(parts[0]), c(parts[1]), c(parts[2])) {
                return Some(Color::Rgb(r, g, b));
            }
        }
        return None;
    }

    let hex = lower.strip_prefix('#').unwrap_or(&lower);
    match hex.len() {
        3 => {
            let d = |i: usize| u8::from_str_radix(&hex[i..i + 1], 16).ok().map(|v| v * 17);
            match (d(0), d(1), d(2)) {
                (Some(r), Some(g), Some(b)) => Some(Color::Rgb(r, g, b)),
                _ => None,
            }
        }
        6 => {
            let d = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
            match (d(0), d(2), d(4)) {
                (Some(r), Some(g), Some(b)) => Some(Color::Rgb(r, g, b)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// xterm writes 1, 2 or 4 hex digits per channel; normalise to 8 bits.
fn scale(value: u16, digits: usize) -> u8 {
    match digits {
        1 => (value as u8) * 17,
        2 => value as u8,
        _ => (value >> 8) as u8,
    }
}

/// Perceived lightness, for deciding whether a background is dark.
pub fn is_dark(color: Color) -> bool {
    match color {
        Color::Rgb(r, g, b) => {
            let l = 0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32;
            l < 128.0
        }
        Color::Black | Color::DarkGray => true,
        Color::White | Color::Gray => false,
        Color::Indexed(n) => n < 8 || (16..=231).contains(&n) && n < 100,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_never_uses_a_colour_the_terminal_did_not_choose() {
        // The entire point of `auto`: one RGB value here and the theme stops
        // following the terminal.
        let t = Theme::auto(true);
        let mut all = vec![
            t.background,
            t.surface,
            t.overlay,
            t.border,
            t.border_focus,
            t.selection,
            t.text,
            t.muted,
            t.faint,
            t.heading,
            t.accent,
            t.secondary,
            t.ok,
            t.redirect,
            t.client_error,
            t.server_error,
            t.open,
            t.closed,
            t.unknown,
            t.repo,
            t.folder,
            t.generic,
        ];
        all.extend_from_slice(&t.kinds);
        for c in all {
            assert!(
                !matches!(c, Color::Rgb(..)),
                "auto must not name an absolute colour, found {c:?}"
            );
        }
    }

    #[test]
    fn mono_emits_no_colour_at_all() {
        let t = Theme::mono();
        let mut all = vec![t.text, t.accent, t.ok, t.closed, t.repo, t.border];
        all.extend_from_slice(&t.kinds);
        for c in all {
            assert_eq!(c, Color::Reset, "mono must be colourless");
        }
        assert!(t.selection_reverse, "mono has to mark selection somehow");
    }

    #[test]
    fn every_builtin_parses_and_defines_every_role() {
        for (name, body) in BUILTIN {
            let t = Theme::from_toml(body, name, Source::Builtin)
                .unwrap_or_else(|e| panic!("built-in {name} does not parse: {e}"));
            assert!(t.name.to_lowercase().replace(' ', "-").contains(name));
            // Every role is filled: unset ones inherit from auto rather than
            // being left invalid, which is what makes a partial file legal.
            for (role, c) in [
                ("text", t.text),
                ("accent", t.accent),
                ("ok", t.ok),
                ("closed", t.closed),
                ("repo", t.repo),
            ] {
                assert_ne!(c, Color::Indexed(255), "{name}: {role} looks unset");
            }
        }
    }

    #[test]
    fn a_partial_theme_file_inherits_the_rest() {
        let t = Theme::from_toml("accent = \"#ff0000\"\n", "partial", Source::User)
            .expect("a one-line theme is valid");
        assert_eq!(t.accent, Color::Rgb(255, 0, 0));
        assert_eq!(t.text, Theme::auto(true).text, "unset roles inherit");
    }

    #[test]
    fn a_broken_theme_file_reports_rather_than_panicking() {
        let err = Theme::from_toml("accent = ", "broken", Source::User).unwrap_err();
        assert!(err.to_string().contains("broken"), "{err}");

        // An unparseable colour leaves the role alone rather than failing.
        let t = Theme::from_toml("accent = \"not-a-colour\"\n", "odd", Source::User)
            .expect("a bad value is not a bad file");
        assert_eq!(t.accent, Theme::auto(true).accent);
    }

    #[test]
    fn unknown_keys_are_rejected_so_a_typo_is_visible() {
        let err = Theme::from_toml("acccent = \"#fff\"\n", "typo", Source::User).unwrap_err();
        assert!(err.to_string().contains("typo"), "{err}");
    }

    #[test]
    fn colours_parse_in_every_form_a_theme_might_use() {
        assert_eq!(parse_color("#ff8800"), Some(Color::Rgb(255, 136, 0)));
        assert_eq!(parse_color("#f80"), Some(Color::Rgb(255, 136, 0)));
        assert_eq!(parse_color("ff8800"), Some(Color::Rgb(255, 136, 0)));
        assert_eq!(parse_color("rgb:ff/88/00"), Some(Color::Rgb(255, 136, 0)));
        assert_eq!(
            parse_color("rgb:ffff/8888/0000"),
            Some(Color::Rgb(255, 136, 0))
        );
        assert_eq!(parse_color("4"), Some(Color::Indexed(4)));
        assert_eq!(parse_color("ansi:12"), Some(Color::Indexed(12)));
        assert_eq!(parse_color("reset"), Some(Color::Reset));
        assert_eq!(parse_color("bright-cyan"), Some(Color::LightCyan));
        for bad in ["", "   ", "#12", "#1234567", "zzz", "rgb:1/2"] {
            assert_eq!(parse_color(bad), None, "accepted {bad:?}");
        }
    }

    #[test]
    fn a_ghostty_theme_becomes_a_usable_theme() {
        let body = include_str!("../tests/fixtures/ghostty-gotham");
        let t = Theme::from_ghostty(body, "gotham").expect("gotham parses");
        assert_eq!(t.source, Source::Ghostty);
        assert!(t.dark, "gotham is a dark theme");
        assert_eq!(t.background, Color::Rgb(0x0a, 0x0f, 0x14));
        assert!(
            !t.selection_reverse,
            "a full palette can colour the selection"
        );
        assert_ne!(t.ok, t.server_error, "healthy and broken must differ");
    }

    #[test]
    fn a_ghostty_theme_with_only_a_palette_still_works() {
        let body = "palette = 0=#000000\npalette = 2=#00ff00\npalette = 10=#88ff88\n";
        let t = Theme::from_ghostty(body, "sparse").expect("a palette alone is enough");
        assert_eq!(t.ok, Color::Rgb(0x88, 0xff, 0x88), "bright slot wins");
    }

    #[test]
    fn something_that_is_not_a_theme_is_rejected() {
        let err = Theme::from_ghostty("hello\nworld\n", "nope").unwrap_err();
        assert!(matches!(err, ThemeError::NotGhostty { .. }), "{err:?}");
    }

    #[test]
    fn resolve_finds_the_names_that_need_no_filesystem() {
        assert_eq!(Theme::resolve("auto").unwrap().source, Source::Auto);
        assert_eq!(Theme::resolve("").unwrap().name, "auto");
        assert_eq!(Theme::resolve("mono").unwrap().name, "mono");
        assert_eq!(Theme::resolve("night").unwrap().source, Source::Builtin);
        assert!(Theme::resolve("definitely-not-a-theme").is_err());
    }

    #[test]
    fn an_unresolvable_theme_falls_back_instead_of_failing() {
        let (theme, err) = Theme::resolve_or_default("definitely-not-a-theme");
        assert_eq!(theme.name, "auto");
        assert!(err.is_some(), "the failure must still be reportable");
    }

    /// Adding a kind must not be able to leave a theme with a hole in it, so
    /// every theme is checked for a usable colour for every kind — including a
    /// three-line user theme that mentions none of them.
    #[test]
    fn every_kind_has_a_colour() {
        let partial = Theme::from_toml("accent = \"#ff0000\"\n", "partial", Source::User)
            .expect("a one-line theme is valid");
        let themes = [
            Theme::auto(true),
            Theme::mono(),
            Theme::resolve("gotham").expect("gotham"),
            Theme::resolve("night").expect("night"),
            Theme::resolve("paper").expect("paper"),
            partial,
        ];
        for theme in themes {
            for kind in Kind::ALL {
                let colour = theme.kind(kind);
                if theme.name != "mono" {
                    // `Reset` here would mean the badge inherits the foreground
                    // and the column stops carrying any information.
                    assert_ne!(
                        colour,
                        Color::Reset,
                        "theme {} has no colour for {}",
                        theme.name,
                        kind.label()
                    );
                }
            }
        }
        assert_eq!(Kind::ALL.len(), 25);
        for (i, k) in Kind::ALL.iter().enumerate() {
            assert_eq!(k.index(), i);
        }
    }
}
