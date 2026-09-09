//! UI state and the actions bound to keys.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use crate::diag;
use crate::keys::{Command, Keymap};
use crate::model::{GroupSource, Health, Rules, Server};
use crate::signature::{Evidence, Registry};
use crate::theme::Theme;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Group(usize),
    Server(usize),
}

/// What the shell around `App` must do after an input. Keeping this as data
/// rather than as side effects is what makes the whole interaction layer
/// testable without a terminal.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Action {
    None,
    Quit,
    Refresh,
    SetMouse(bool),
    ClearScreen,
    /// Re-read the config and theme from disk.
    Reload,
    /// Hand a URL to the user's browser.
    Open(String),
    /// Put text on the system clipboard.
    Copy(String),
    /// Signal a process. Only ever produced after an explicit confirmation.
    Signal {
        pid: u32,
        force: bool,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Good,
    Bad,
}

pub struct Group {
    pub key: String,
    pub source: GroupSource,
    pub branch: Option<String>,
    pub remote: Option<String>,
    pub count: usize,
    pub trouble: usize,
}

#[derive(Clone, Debug)]
pub struct Failure {
    pub detail: String,
    pub transient: bool,
    pub since: Instant,
    pub count: u32,
}

pub struct Confirm {
    pub prompt: String,
    pub detail: String,
    pub action: PendingAction,
}

#[derive(Clone, Copy)]
pub enum PendingAction {
    Kill { pid: u32, force: bool },
}

pub struct App {
    pub servers: Vec<Server>,
    pub groups: Vec<Group>,
    pub rows: Vec<Row>,
    /// Which group each server belongs to, by server index. Rebuilt with the
    /// rows, so a health update can adjust one counter instead of re-deriving
    /// every group from scratch.
    group_of: Vec<usize>,
    /// pid → the servers holding it. A scan of two thousand services delivers
    /// two thousand probe results, and finding each one by walking the list
    /// would be quadratic.
    by_pid: HashMap<u32, Vec<usize>>,
    pub selected: usize,
    pub offset: usize,
    pub collapsed: HashSet<String>,
    pub search: String,
    pub searching: bool,
    pub show_all: bool,
    pub help: bool,
    pub mouse: bool,
    pub scanning: bool,
    pub last_scan: Option<Instant>,
    /// Set when the scanner reports a failure; cleared by the next good scan.
    pub failure: Option<Failure>,
    /// False if the background thread has died — the UI keeps working and says so.
    pub scanner_alive: bool,
    pub diagnostics: bool,
    pub toast: Option<(String, ToastKind, Instant)>,
    pub confirm: Option<Confirm>,
    /// Wall-clock seconds, refreshed once per frame rather than read during a
    /// render. Rendering has to be a pure function of state, or a snapshot of
    /// the screen is not reproducible.
    pub now: u64,
    /// The active palette. An explicit part of the state rather than something
    /// read from the environment while drawing, so a rendered screen is a pure
    /// function of what is here.
    pub theme: Theme,
    /// The active bindings. Data rather than a `match`, so a config can change
    /// them and the help overlay can be generated from them.
    pub keymap: Keymap,
    /// The signature table, so a probe result can be re-identified against
    /// evidence the scan did not have: what the service said on connect, and
    /// what its HTTP response gave away.
    pub signatures: std::sync::Arc<Registry>,
    /// The user's own `[ports]` and `[names]` rules. Checked before the
    /// signature table on the second pass as well as the first: a statement
    /// the user made about their own machine must not be quietly overridden by
    /// something quarry guessed.
    pub rules: Rules,
    pub url_hitbox: Option<Rect>,
    pub list_area: Rect,
    pub should_quit: bool,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        Self {
            servers: Vec::new(),
            groups: Vec::new(),
            rows: Vec::new(),
            group_of: Vec::new(),
            by_pid: HashMap::new(),
            selected: 0,
            offset: 0,
            collapsed: HashSet::new(),
            search: String::new(),
            searching: false,
            show_all: false,
            help: false,
            mouse: true,
            scanning: true,
            last_scan: None,
            failure: None,
            scanner_alive: true,
            diagnostics: false,
            toast: None,
            confirm: None,
            now: unix_seconds(),
            theme: Theme::auto(true),
            keymap: Keymap::default(),
            signatures: std::sync::Arc::new(Registry::builtin()),
            rules: Rules::default(),
            url_hitbox: None,
            list_area: Rect::default(),
            should_quit: false,
        }
    }

    /// Replace the world, keeping the cursor on the same service where we can.
    pub fn ingest(&mut self, mut fresh: Vec<Server>) {
        let anchor = self.selected_server().map(|s| (s.pid, s.primary_port()));

        // Carry health forward so rows do not blink back to "checking" on every
        // refresh; the probe results for the new scan overwrite it shortly.
        for s in fresh.iter_mut() {
            if let Some(prev) = self
                .servers
                .iter()
                .find(|p| p.pid == s.pid && p.primary_port() == s.primary_port())
            {
                s.health = prev.health.clone();
            }
        }

        self.by_pid.clear();
        for (i, s) in fresh.iter().enumerate() {
            self.by_pid.entry(s.pid).or_default().push(i);
        }
        self.servers = fresh;
        self.scanning = false;
        self.failure = None;
        self.last_scan = Some(Instant::now());
        self.rebuild();

        if let Some((pid, port)) = anchor
            && let Some(idx) = self
                .servers
                .iter()
                .position(|s| s.pid == pid && s.primary_port() == port)
            && let Some(row) = self
                .rows
                .iter()
                .position(|r| matches!(r, Row::Server(i) if *i == idx))
        {
            self.selected = row;
        }
        self.clamp();
    }

    /// A probe result. Matched on pid *and* port: a pid can be reused between
    /// scans, and a stale answer landing on a new process would be a lie.
    ///
    /// This is also the second pass at identification. The scan could only see
    /// a port and a process name; the probe may have heard the service
    /// introduce itself, or read a title off its own page. Both outrank a port,
    /// so the answer can improve here — never silently, since the detail pane
    /// shows what the verdict rested on.
    pub fn apply_health(&mut self, pid: u32, port: u16, health: Health, banner: Option<Vec<u8>>) {
        let Some(indices) = self.by_pid.get(&pid) else {
            return;
        };
        let (server_header, title) = match &health {
            Health::Http { server, title, .. } => (server.clone(), title.clone()),
            _ => (None, None),
        };
        // Re-identify only when the probe learned something the scan did not.
        // A closed port and a silent socket carry no new evidence, and scoring
        // the whole table again to reach the same answer is work for nothing.
        let new_evidence = banner.is_some() || server_header.is_some() || title.is_some();

        for i in indices.clone() {
            let Some(server) = self.servers.get_mut(i) else {
                continue;
            };
            if !server.listeners.iter().any(|l| l.port == port) {
                continue;
            }

            // A container's identity comes from the daemon and is not a guess;
            // nothing the probe hears should overwrite it.
            let from_container = server.container.is_some();
            let user_ruled = self
                .rules
                .classify(&server.command, &server.cmdline, &[port])
                .is_some();
            if new_evidence && !user_ruled && !from_container {
                let ports: Vec<u16> = server.listeners.iter().map(|l| l.port).collect();
                let verdict = self.signatures.identify(&Evidence {
                    command: &server.command,
                    cmdline: &server.cmdline,
                    ports: &ports,
                    banner: banner.as_deref(),
                    http_server: server_header.as_deref(),
                    http_title: title.as_deref(),
                });
                if let Some(v) = verdict {
                    if let Some(sig) = self.signatures.get(v.index) {
                        server.kind = sig.kind;
                        if v.names_the_service() {
                            server.service = Some(sig.name.clone());
                        }
                        server.uri = sig.uri_for(port, None).or(server.uri.take());
                        server.note = sig.note.clone().or(server.note.take());
                    }
                    server.evidence = v.reasons.clone();
                }
                server.banner = banner.clone();
            }
            if server.service.is_none() {
                server.kind = crate::model::refine_kind(server.kind, &health);
            }

            // Adjust the one counter this changes rather than recomputing every
            // group; the alternative is quadratic in the number of services.
            let was_trouble = server.health.is_trouble();
            server.health = health.clone();
            let is_trouble = server.health.is_trouble();

            if was_trouble != is_trouble
                && let Some(group) = self
                    .group_of
                    .get(i)
                    .copied()
                    .and_then(|g| self.groups.get_mut(g))
            {
                if is_trouble {
                    group.trouble += 1;
                } else {
                    group.trouble = group.trouble.saturating_sub(1);
                }
            }
        }
    }

    /// The scan itself failed. Keep showing the last good data, marked stale.
    pub fn scan_failed(&mut self, detail: String, transient: bool) {
        self.scanning = false;
        let count = self.failure.as_ref().map(|f| f.count + 1).unwrap_or(1);
        self.failure = Some(Failure {
            detail,
            transient,
            since: Instant::now(),
            count,
        });
    }

    pub fn warn(&mut self, message: String) {
        diag::warn("app", message.clone());
        self.toast(message, ToastKind::Bad);
    }

    /// True when what is on screen is older than it should be.
    pub fn is_stale(&self) -> bool {
        self.failure.is_some() || !self.scanner_alive
    }

    /// Which row a click landed on, if any.
    pub fn row_at(&self, column: u16, row: u16) -> Option<usize> {
        let area = self.list_area;
        let inside = column > area.x
            && column < area.x + area.width.saturating_sub(1)
            && row > area.y
            && row < area.y + area.height.saturating_sub(1);
        if !inside {
            return None;
        }
        let idx = self.offset + usize::from(row - area.y - 1);
        (idx < self.rows.len()).then_some(idx)
    }

    /// Clicking a service selects it; clicking a group header folds it.
    pub fn click_row(&mut self, idx: usize) {
        match self.rows.get(idx) {
            Some(Row::Server(_)) => {
                self.selected = idx;
            }
            Some(Row::Group(_)) => {
                self.selected = idx;
                self.toggle_group();
            }
            None => {}
        }
    }

    fn visible(&self, s: &Server, needle: &str) -> bool {
        if !self.show_all && s.kind.is_background_noise() {
            return false;
        }
        // A developer machine has several hundred unix sockets and a couple of
        // dozen ports. Showing them all by default would bury the ports, so
        // they live behind the same switch as the rest of the background.
        if !self.show_all && s.is_socket_only() {
            return false;
        }
        if !needle.is_empty() && !s.matches(needle) {
            return false;
        }
        true
    }

    pub fn rebuild(&mut self) {
        // Group keys are derived from the repo, the directory and the kind, and
        // each derivation allocates. Computing them once and sorting on the
        // result turns O(n log n) allocations into O(n).
        let keys: Vec<String> = self.servers.iter().map(|s| s.group_key()).collect();
        let ranks: Vec<u8> = self
            .servers
            .iter()
            .map(|s| s.group_source().rank())
            .collect();

        // Lowercased once here rather than once per service per keystroke.
        let needle = self.search.to_lowercase();
        let mut indices: Vec<usize> = (0..self.servers.len())
            .filter(|i| self.visible(&self.servers[*i], &needle))
            .collect();
        indices.sort_by(|a, b| {
            ranks[*a]
                .cmp(&ranks[*b])
                .then_with(|| keys[*a].cmp(&keys[*b]))
                .then_with(|| {
                    self.servers[*a]
                        .health
                        .rank()
                        .cmp(&self.servers[*b].health.rank())
                })
                .then_with(|| {
                    self.servers[*a]
                        .primary_port()
                        .cmp(&self.servers[*b].primary_port())
                })
        });

        self.groups.clear();
        self.rows.clear();
        self.group_of.clear();
        self.group_of.resize(self.servers.len(), usize::MAX);

        let mut current: Option<&str> = None;
        for i in indices {
            let key = keys[i].as_str();
            if current != Some(key) {
                let repo = self.servers[i].repo.clone();
                self.groups.push(Group {
                    key: keys[i].clone(),
                    source: self.servers[i].group_source(),
                    branch: repo.as_ref().and_then(|r| r.branch.clone()),
                    remote: repo.as_ref().and_then(|r| r.remote.clone()),
                    count: 0,
                    trouble: 0,
                });
                self.rows.push(Row::Group(self.groups.len() - 1));
                current = Some(key);
            }
            let group_idx = self.groups.len() - 1;
            self.group_of[i] = group_idx;
            let g = self.groups.last_mut().expect("group pushed above");
            g.count += 1;
            if self.servers[i].health.is_trouble() {
                g.trouble += 1;
            }
            if !self.collapsed.contains(key) {
                self.rows.push(Row::Server(i));
            }
        }
        self.clamp();
    }

    /// Group headers are only landable when collapsed — otherwise the cursor
    /// would stop on a row with nothing behind it in the detail pane.
    fn is_selectable(&self, idx: usize) -> bool {
        match self.rows.get(idx) {
            Some(Row::Server(_)) => true,
            Some(Row::Group(g)) => self
                .groups
                .get(*g)
                .is_some_and(|g| self.collapsed.contains(&g.key)),
            None => false,
        }
    }

    pub fn clamp(&mut self) {
        if self.rows.is_empty() {
            self.selected = 0;
            return;
        }
        self.selected = self.selected.min(self.rows.len() - 1);
        if !self.is_selectable(self.selected)
            && let Some(next) = self.nearest_selectable(self.selected)
        {
            self.selected = next;
        }
    }

    fn nearest_selectable(&self, from: usize) -> Option<usize> {
        (from + 1..self.rows.len())
            .chain((0..from).rev())
            .find(|i| self.is_selectable(*i))
    }

    /// The group under the cursor, when the cursor is on a header.
    pub fn selected_group(&self) -> Option<&Group> {
        match self.rows.get(self.selected)? {
            Row::Group(g) => self.groups.get(*g),
            Row::Server(_) => None,
        }
    }

    /// Every service in a group, for the summary shown beside a folded one.
    pub fn servers_in(&self, key: &str) -> Vec<&Server> {
        self.servers
            .iter()
            .enumerate()
            .filter(|(i, _)| {
                self.group_of
                    .get(*i)
                    .and_then(|g| self.groups.get(*g))
                    .is_some_and(|g| g.key == key)
            })
            .map(|(_, s)| s)
            .collect()
    }

    pub fn selected_server(&self) -> Option<&Server> {
        match self.rows.get(self.selected)? {
            Row::Server(i) => self.servers.get(*i),
            Row::Group(_) => None,
        }
    }

    pub fn move_by(&mut self, delta: isize) {
        if self.rows.is_empty() || delta == 0 {
            return;
        }
        let len = self.rows.len() as isize;
        let step = delta.signum();
        let mut i = self.selected as isize;
        for _ in 0..delta.abs() {
            // Walk one landable row at a time, wrapping at the ends.
            for _ in 0..len {
                i = (i + step).rem_euclid(len);
                if self.is_selectable(i as usize) {
                    break;
                }
            }
        }
        self.selected = i as usize;
    }

    pub fn jump(&mut self, to_end: bool) {
        if self.rows.is_empty() {
            return;
        }
        self.selected = if to_end { self.rows.len() - 1 } else { 0 };
        self.clamp();
    }

    pub fn toggle_group(&mut self) {
        let key = match self.rows.get(self.selected) {
            Some(Row::Group(g)) => self.groups[*g].key.clone(),
            Some(Row::Server(i)) => self.servers[*i].group_key(),
            None => return,
        };
        let collapsing = !self.collapsed.contains(&key);
        if collapsing {
            self.collapsed.insert(key.clone());
        } else {
            self.collapsed.remove(&key);
        }
        self.rebuild();

        // Keep the cursor on the group we just acted on: its header when
        // collapsed, its first service when expanded.
        if let Some(header) = self
            .rows
            .iter()
            .position(|r| matches!(r, Row::Group(g) if self.groups[*g].key == key))
        {
            self.selected = if collapsing { header } else { header + 1 };
            self.clamp();
        }
    }

    pub fn toast(&mut self, msg: impl Into<String>, kind: ToastKind) {
        self.toast = Some((msg.into(), kind, Instant::now()));
    }

    /// Called once per frame by the shell.
    pub fn tick_clock(&mut self) {
        self.now = unix_seconds();
    }

    pub fn expire_toast(&mut self) {
        if let Some((_, _, at)) = &self.toast
            && at.elapsed() > Duration::from_millis(2600)
        {
            self.toast = None;
        }
    }

    /// Returns the URL to open. Performing it is the shell's job — `App` runs
    /// inside tests and fuzzers, and must never launch a browser or a process
    /// as a side effect of handling a key.
    /// Enter opens what a browser can open, and copies what it cannot.
    ///
    /// Launching a browser at `postgres://localhost:5432` is a promise that
    /// cannot be kept, and so is opening one at a unix socket. Copying the URI
    /// is the useful thing to do instead — it is what you would paste into a
    /// client.
    pub fn open_selected(&mut self) -> Action {
        let Some(s) = self.selected_server() else {
            return Action::None;
        };
        if s.kind.opens_in_a_browser() && !s.is_socket_only() {
            return Action::Open(s.url());
        }
        let what = s.service_name();
        let uri = s.url();
        self.toast(
            format!("{what} is not something a browser can open — copied instead"),
            ToastKind::Info,
        );
        Action::Copy(uri)
    }

    pub fn copy_selected(&mut self) -> Action {
        match self.selected_server() {
            Some(s) => Action::Copy(s.url()),
            None => Action::None,
        }
    }

    pub fn ask_kill(&mut self, force: bool) {
        let Some(s) = self.selected_server() else {
            return;
        };
        self.confirm = Some(Confirm {
            prompt: format!(
                "{} {} (pid {})?",
                if force { "Force kill" } else { "Stop" },
                s.title(),
                s.pid
            ),
            detail: format!("{} on :{}", s.command, s.primary_port()),
            action: PendingAction::Kill { pid: s.pid, force },
        });
    }

    pub fn resolve_confirm(&mut self, go: bool) -> Action {
        let Some(c) = self.confirm.take() else {
            return Action::None;
        };
        if !go {
            return Action::None;
        }
        match c.action {
            PendingAction::Kill { pid, force } => Action::Signal { pid, force },
        }
    }

    /// The whole keyboard interface. Modal layers get first refusal, in order,
    /// and only then does the keymap get a look.
    pub fn handle_key(&mut self, code: KeyCode, mods: KeyModifiers) -> Action {
        if self.confirm.is_some() {
            return match code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    self.resolve_confirm(true)
                }
                _ => self.resolve_confirm(false),
            };
        }
        if self.help || self.diagnostics {
            self.help = false;
            self.diagnostics = false;
            return Action::None;
        }
        if self.searching {
            match code {
                KeyCode::Esc => {
                    self.search.clear();
                    self.searching = false;
                    self.rebuild();
                }
                KeyCode::Enter => self.searching = false,
                KeyCode::Backspace => {
                    self.search.pop();
                    self.rebuild();
                }
                KeyCode::Char(c) => {
                    self.search.push(c);
                    self.rebuild();
                }
                _ => {}
            }
            return Action::None;
        }

        match self.keymap.lookup(code, mods) {
            Some(command) => self.run(command),
            None => Action::None,
        }
    }

    /// Perform a named command. Everything with an effect outside the process
    /// leaves here as an `Action` for the shell to carry out.
    pub fn run(&mut self, command: Command) -> Action {
        match command {
            Command::Quit => return Action::Quit,
            Command::Back => {
                // Backing out closes what is open; with nothing open it does
                // nothing, rather than quitting out from under you.
                if !self.search.is_empty() {
                    self.search.clear();
                    self.rebuild();
                    self.toast("filter cleared", ToastKind::Info);
                }
            }
            Command::Down => self.move_by(1),
            Command::Up => self.move_by(-1),
            Command::PageDown => self.move_by(10),
            Command::PageUp => self.move_by(-10),
            Command::First => self.jump(false),
            Command::Last => self.jump(true),
            Command::ToggleGroup => self.toggle_group(),
            Command::Open => return self.open_selected(),
            Command::Copy => return self.copy_selected(),
            Command::Filter => {
                self.searching = true;
                self.search.clear();
                self.rebuild();
            }
            Command::ToggleAll => {
                self.show_all = !self.show_all;
                self.rebuild();
                let msg = if self.show_all {
                    "showing system services"
                } else {
                    "hiding system services"
                };
                self.toast(msg, ToastKind::Info);
            }
            Command::Refresh => {
                self.scanning = true;
                return Action::Refresh;
            }
            Command::Reload => return Action::Reload,
            Command::Stop => self.ask_kill(false),
            Command::ForceKill => self.ask_kill(true),
            Command::Diagnostics => self.diagnostics = true,
            Command::Help => self.help = true,
            Command::ToggleMouse => {
                self.mouse = !self.mouse;
                let msg = if self.mouse {
                    "mouse capture on"
                } else {
                    "mouse capture off — text is selectable"
                };
                self.toast(msg, ToastKind::Info);
                return Action::SetMouse(self.mouse);
            }
        }
        Action::None
    }

    pub fn handle_mouse(&mut self, m: MouseEvent) -> Action {
        match m.kind {
            MouseEventKind::ScrollDown => self.move_by(1),
            MouseEventKind::ScrollUp => self.move_by(-1),
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(hit) = self.url_hitbox
                    && m.column >= hit.x
                    && m.column < hit.x + hit.width
                    && m.row >= hit.y
                    && m.row < hit.y + hit.height
                {
                    return self.open_selected();
                }
                if let Some(idx) = self.row_at(m.column, m.row) {
                    self.click_row(idx);
                }
            }
            _ => {}
        }
        Action::None
    }

    /// Everything that must be true of the state after any sequence of inputs.
    /// Asserted by the randomised tests, and cheap enough to call anywhere.
    pub fn check_invariants(&self) -> Result<(), String> {
        if self.rows.is_empty() {
            return if self.selected == 0 {
                Ok(())
            } else {
                Err(format!("selection {} with no rows", self.selected))
            };
        }
        if self.selected >= self.rows.len() {
            return Err(format!(
                "selection {} is past the last row {}",
                self.selected,
                self.rows.len() - 1
            ));
        }
        if !self.is_selectable(self.selected) {
            return Err(format!(
                "selection {} rests on an expanded group header",
                self.selected
            ));
        }
        for row in &self.rows {
            match row {
                Row::Server(i) if *i >= self.servers.len() => {
                    return Err(format!("row points at missing server {i}"));
                }
                Row::Group(g) if *g >= self.groups.len() => {
                    return Err(format!("row points at missing group {g}"));
                }
                _ => {}
            }
        }
        let counted: usize = self.groups.iter().map(|g| g.count).sum();
        let needle = self.search.to_lowercase();
        let visible = self
            .servers
            .iter()
            .filter(|s| self.visible(s, &needle))
            .count();
        if counted != visible {
            return Err(format!(
                "group counts total {counted} but {visible} services are visible"
            ));
        }
        Ok(())
    }
}

fn unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
