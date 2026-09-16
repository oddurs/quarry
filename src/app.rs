//! UI state and the actions bound to keys.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use crate::diag;
use crate::keys::{Command, Keymap};
use crate::lifecycle::{Op, Target};
use crate::model::{GroupBy, GroupSource, Health, Query, Rules, Scope, Server, SortBy};
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
    /// Stop, restart or kill a service. Only ever produced after an explicit
    /// confirmation, and always carried out on a worker: a stop waits on a
    /// grace period, and the UI must not wait with it.
    Lifecycle {
        /// More than one when a whole group was selected. A worktree is a unit
        /// people think in — "restart this branch" — and doing it one row at a
        /// time is four confirmations for one intention.
        targets: Vec<Target>,
        op: Op,
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
    /// Whether `source` says anything about this heading. Grouped by kind, the
    /// key is the whole name — and the source belongs to whichever member
    /// sorted first, which would have rendered two different kinds as "no
    /// project" because neither had one.
    pub describes_a_project: bool,
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

#[derive(Clone)]
pub enum PendingAction {
    Lifecycle { targets: Vec<Target>, op: Op },
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
    /// Whether the detail pane is on screen. The list is what you read; the
    /// detail is what you look up, so it can get out of the way.
    pub detail: bool,
    /// How the list is divided, and the order within each division.
    pub group_by: GroupBy,
    pub sort_by: SortBy,
    pub show_all: bool,
    /// The repository quarry was started in, if it was started in one. Found
    /// once: the working directory cannot change while it runs.
    pub scope: Option<Scope>,
    /// Whether that scope is being applied. Separate from `scope` so the key
    /// that turns it off can turn it back on without another look at the disk.
    pub here: bool,
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
            detail: true,
            group_by: GroupBy::default(),
            sort_by: SortBy::default(),
            show_all: false,
            scope: None,
            here: false,
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
        //
        // The same walk answers what changed. quarry rescans every few seconds
        // and has always known exactly what appeared and vanished; saying so is
        // the difference between a tool you run and one you leave open.
        let watching = self.last_scan.is_some();
        let now = Instant::now();
        for s in fresh.iter_mut() {
            match self
                .servers
                .iter()
                .find(|p| p.pid == s.pid && p.primary_port() == s.primary_port())
            {
                Some(prev) => {
                    s.health = prev.health.clone();
                    s.appeared = prev.appeared;
                }
                // The first scan is not news. Marking every service on the
                // machine as new would be true and useless.
                None => s.appeared = watching.then_some(now),
            }
        }

        let gone: Vec<String> = self
            .servers
            .iter()
            .filter(|p| {
                !fresh
                    .iter()
                    .any(|s| s.pid == p.pid && s.primary_port() == p.primary_port())
            })
            .map(|p| format!("{} {}", p.title(), p.primary_label()))
            .collect();
        if watching && !gone.is_empty() {
            // A departure leaves no row to mark, so it has to be said once
            // rather than shown. Two names and a count, because a toast
            // listing nine services is a toast nobody finishes reading.
            let text = match gone.len() {
                1 => format!("{} stopped", gone[0]),
                2 => format!("{} and {} stopped", gone[0], gone[1]),
                n => format!("{}, {} and {} more stopped", gone[0], gone[1], n - 2),
            };
            self.toast(text, ToastKind::Info);
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

    fn visible(&self, s: &Server, query: &Query) -> bool {
        // Narrowing to a project is one more filter, not a different mode.
        // `-a` has to keep meaning what it means everywhere else, or a repo
        // with a couple of unix sockets in it looks like it has servers.
        if let Some(scope) = self.scoped()
            && !s.in_scope(scope)
        {
            return false;
        }
        if !self.show_all && s.kind.is_background_noise() {
            return false;
        }
        // A developer machine has several hundred unix sockets and a couple of
        // dozen ports. Showing them all by default would bury the ports, so
        // they live behind the same switch as the rest of the background.
        if !self.show_all && s.is_socket_only() {
            return false;
        }
        if !query.is_empty() && !s.satisfies(query) {
            return false;
        }
        true
    }

    /// Rebuild the rows from the services, the filter and the arrangement.
    ///
    /// Three steps, and the order matters: decide what each service is grouped
    /// under, put the visible ones in order, then walk that order emitting a
    /// heading whenever the group changes.
    pub fn rebuild(&mut self) {
        let keys = self.group_keys();
        let order = self.ordering(&keys);

        self.groups.clear();
        self.rows.clear();
        self.group_of.clear();
        self.group_of.resize(self.servers.len(), usize::MAX);

        let flat = self.group_by == GroupBy::Nothing;
        let mut current: Option<&str> = None;
        for i in order {
            let key = keys[i].as_str();
            // Flat means no headings at all, not one heading over everything.
            if flat {
                self.rows.push(Row::Server(i));
                continue;
            }
            if current != Some(key) {
                self.groups.push(self.heading(i, &keys));
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

    /// What each service is grouped under, by index.
    ///
    /// Computed once and sorted on, rather than derived during the sort: every
    /// key allocates, and deriving them inside the comparator turns O(n log n)
    /// allocations into O(n).
    fn group_keys(&self) -> Vec<String> {
        // Inside one repository every service shares a project, so grouping by
        // project would produce a single heap. The branch is what tells two of
        // them apart, and it is what a person calls a worktree.
        let scoped = self.scoped().is_some();
        self.servers
            .iter()
            .map(|s| match self.group_by {
                GroupBy::Kind => s.kind.label().to_string(),
                // One key for everything: the rows still sort as one run, and
                // the heading is simply never emitted.
                GroupBy::Nothing => String::new(),
                GroupBy::Project if scoped => s.worktree_key(),
                GroupBy::Project => s.group_key(),
            })
            .collect()
    }

    /// The visible services, in the order they appear on screen.
    fn ordering(&self, keys: &[String]) -> Vec<usize> {
        // Parsed once here rather than once per service per keystroke.
        let query = Query::parse(&self.search);
        let mut order: Vec<usize> = (0..self.servers.len())
            .filter(|i| self.visible(&self.servers[*i], &query))
            .collect();

        // A group holding something broken sorts first. That row was reliably
        // the hardest to reach: unattributed services sort last by rank, and a
        // stray broken container is exactly the kind of thing with no project.
        let troubled: HashSet<&str> = order
            .iter()
            .filter(|i| self.servers[**i].health.is_trouble())
            .map(|i| keys[*i].as_str())
            .collect();
        let calm = |i: usize| !troubled.contains(keys[i].as_str());

        // Where the name came from only orders groups by project. Grouped by
        // kind, or not at all, it would shuffle rows for a reason that is not
        // on screen.
        let rank = |i: usize| match self.group_by {
            GroupBy::Project => self.servers[i].group_source().rank(),
            _ => 0,
        };

        order.sort_by(|a, b| {
            calm(*a)
                .cmp(&calm(*b))
                .then_with(|| rank(*a).cmp(&rank(*b)))
                .then_with(|| keys[*a].cmp(&keys[*b]))
                .then_with(|| self.compare(*a, *b))
        });
        order
    }

    /// Two services in the same group, under the chosen order.
    fn compare(&self, a: usize, b: usize) -> std::cmp::Ordering {
        let (x, y) = (&self.servers[a], &self.servers[b]);
        match self.sort_by {
            SortBy::Health => x.health.rank().cmp(&y.health.rank()),
            SortBy::Port => std::cmp::Ordering::Equal,
            SortBy::Name => x.service_name().cmp(&y.service_name()),
            // Reversed: the one you just started is the one you are looking
            // for, and it is the last to have been started.
            SortBy::Newest => y.started_at.cmp(&x.started_at),
        }
        // Port always breaks the tie, so the order is total and the list does
        // not reshuffle between two equal rows on every scan.
        .then_with(|| x.primary_port().cmp(&y.primary_port()))
    }

    /// The heading a group gets, from the first service under it.
    fn heading(&self, i: usize, keys: &[String]) -> Group {
        // A heading only speaks for a project when the grouping is by project.
        // Grouped by kind it belongs to whichever service happened to sort
        // first, which is nobody.
        let by_project = self.group_by == GroupBy::Project;
        let scoped = self.scoped().is_some();
        let repo = self.servers[i].repo.as_ref();
        Group {
            key: keys[i].clone(),
            source: self.servers[i].group_source(),
            describes_a_project: by_project,
            // Scoped, the key is already the branch and the remote is the same
            // for every group. Printing either again would be noise on every
            // row.
            branch: repo
                .and_then(|r| r.branch.clone())
                .filter(|_| by_project && !scoped),
            remote: repo
                .and_then(|r| r.remote.clone())
                .filter(|_| by_project && !scoped),
            count: 0,
            trouble: 0,
        }
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
        if s.opens_in_a_browser() {
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

    /// Move to the next service that is not answering, wrapping around.
    ///
    /// A collapsed group is expanded to get there. The alternative is a key
    /// that reports trouble it will not show you.
    pub fn jump_to_trouble(&mut self, forward: bool) {
        if !self.servers.iter().any(|s| s.health.is_trouble()) {
            self.toast("everything is answering".to_string(), ToastKind::Good);
            return;
        }
        let hidden: Vec<String> = self
            .groups
            .iter()
            .filter(|g| g.trouble > 0 && self.collapsed.contains(&g.key))
            .map(|g| g.key.clone())
            .collect();
        if !hidden.is_empty() {
            for key in hidden {
                self.collapsed.remove(&key);
            }
            self.rebuild();
        }

        let n = self.rows.len();
        let broken = |row: &Row| match row {
            Row::Server(i) => self.servers[*i].health.is_trouble(),
            Row::Group(_) => false,
        };
        // From the row after this one, so repeated presses walk the list
        // rather than sticking on whatever is already selected.
        let found = (1..=n).find_map(|step| {
            let i = if forward {
                (self.selected + step) % n
            } else {
                (self.selected + n - step % n) % n
            };
            broken(&self.rows[i]).then_some(i)
        });
        if let Some(i) = found {
            // The renderer follows the selection; nothing to scroll here.
            self.selected = i;
        }
    }

    /// Cycle how the list is divided. Reports the new arrangement, because
    /// the change is easy to miss on a machine with one project.
    pub fn cycle_group_by(&mut self) {
        self.group_by = self.group_by.next();
        let text = match self.group_by {
            GroupBy::Nothing => "one flat list".to_string(),
            other => format!("grouped by {}", other.name()),
        };
        // Folding is remembered by group key, and the keys are different in
        // every arrangement. Carrying them across means a group folding itself
        // because something with the same name was folded two modes ago.
        self.collapsed.clear();
        self.rebuild();
        self.toast(text, ToastKind::Info);
    }

    pub fn cycle_sort_by(&mut self) {
        self.sort_by = self.sort_by.next();
        let text = match self.sort_by {
            SortBy::Newest => "newest first".to_string(),
            other => format!("sorted by {}", other.name()),
        };
        self.rebuild();
        self.toast(text, ToastKind::Info);
    }

    /// What the list pane calls itself: the arrangement, when it is not the
    /// default one. A mode you cannot see you are in is a bug report waiting
    /// to happen.
    pub fn arrangement(&self) -> Option<String> {
        let group = (self.group_by != GroupBy::default()).then(|| match self.group_by {
            GroupBy::Nothing => "ungrouped".to_string(),
            other => format!("by {}", other.name()),
        });
        let sort = (self.sort_by != SortBy::default()).then(|| match self.sort_by {
            SortBy::Newest => "newest first".to_string(),
            other => format!("{} order", other.name()),
        });
        match (group, sort) {
            (None, None) => None,
            (Some(g), None) => Some(g),
            (None, Some(s)) => Some(s),
            (Some(g), Some(s)) => Some(format!("{g}, {s}")),
        }
    }

    /// The scope, if one was found *and* is being applied.
    pub fn scoped(&self) -> Option<&Scope> {
        self.here.then_some(self.scope.as_ref()).flatten()
    }

    /// Turn the repository scope on or off. Asking to narrow to a project
    /// while standing outside one has to fail visibly, or the screen simply
    /// does not change and nothing explains why.
    pub fn toggle_here(&mut self) {
        match &self.scope {
            None => self.toast(
                "not in a git repository — nothing to narrow to".to_string(),
                ToastKind::Bad,
            ),
            Some(scope) => {
                let label = scope.label();
                self.here = !self.here;
                let text = if self.here {
                    format!("showing {label} only")
                } else {
                    "showing every project".to_string()
                };
                self.toast(text, ToastKind::Info);
                self.rebuild();
            }
        }
    }

    /// Raise the confirmation for stopping, restarting or killing what is
    /// selected.
    ///
    /// The prompt names what will actually happen rather than what was pressed.
    /// "Restart" means two different things depending on whether a daemon or
    /// the kernel owns the service, and a confirmation that hides the
    /// difference is not a confirmation.
    pub fn ask(&mut self, op: Op) {
        let verb = match op {
            Op::Stop => "Stop",
            Op::Restart => "Restart",
            Op::Kill => "Force kill",
        };
        // A group heading is a selection too, and a worktree is a unit people
        // think in: "restart this branch" rather than four rows in turn.
        if let Some(group) = self.selected_group() {
            let key = group.key.clone();
            let members = self.servers_in(&key);
            let n = members.len();
            if n == 0 {
                return;
            }
            let targets: Vec<Target> = members.iter().map(|s| s.lifecycle()).collect();
            let containers = targets
                .iter()
                .filter(|t| matches!(t, Target::Container(_)))
                .count();
            // What is about to happen differs by who owns each service, and
            // with several at once the honest summary is the split.
            let detail = match (containers, n - containers) {
                (0, _) => format!("{}, one at a time", plural(n, "process", "processes")),
                (_, 0) => format!(
                    "{}, through their runtime",
                    plural(n, "container", "containers")
                ),
                (c, p) => format!(
                    "{} and {}, one at a time",
                    plural(c, "container", "containers"),
                    plural(p, "process", "processes")
                ),
            };
            self.confirm = Some(Confirm {
                prompt: format!("{verb} all of {key}?"),
                detail,
                action: PendingAction::Lifecycle { targets, op },
            });
            return;
        }

        let Some(s) = self.selected_server() else {
            return;
        };
        let target = s.lifecycle();
        let prompt = format!("{verb} {}?", s.title());
        // ":8000" rather than the bare "8000" the list column shows — in a
        // sentence the colon is what makes it read as a port.
        let at = match s.primary() {
            Some(l) if l.is_unix() => l.label(),
            Some(l) => format!(":{}", l.port),
            None => "no listener".to_string(),
        };
        let detail = match (&target, op) {
            (Target::Container(c), Op::Stop) => {
                format!("the runtime stops {} · {at}", c.display_name())
            }
            (Target::Container(c), Op::Restart) => {
                format!("the runtime restarts {} · {at}", c.display_name())
            }
            (Target::Container(c), Op::Kill) => {
                format!("the runtime kills {} · no grace period", c.display_name())
            }
            (Target::Process { pid, .. }, Op::Stop) => {
                format!("SIGTERM {pid} · {} on {at}", s.command)
            }
            (Target::Process { pid, .. }, Op::Restart) => {
                format!("SIGTERM {pid}, then start it again · {at}")
            }
            (Target::Process { pid, .. }, Op::Kill) => {
                format!("SIGKILL {pid}, no clean shutdown · {at}")
            }
        };
        self.confirm = Some(Confirm {
            prompt,
            detail,
            action: PendingAction::Lifecycle {
                targets: vec![target],
                op,
            },
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
            PendingAction::Lifecycle { targets, op } => Action::Lifecycle { targets, op },
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
            Command::ToggleHere => self.toggle_here(),
            Command::ToggleDetail => self.detail = !self.detail,
            Command::GroupBy => self.cycle_group_by(),
            Command::SortBy => self.cycle_sort_by(),
            Command::NextTrouble => self.jump_to_trouble(true),
            Command::PrevTrouble => self.jump_to_trouble(false),
            Command::Stop => self.ask(Op::Stop),
            Command::Restart => self.ask(Op::Restart),
            Command::ForceKill => self.ask(Op::Kill),
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
        // Flat has no groups to count, so the rows carry the number instead.
        let counted: usize = if self.group_by == GroupBy::Nothing {
            self.rows.len()
        } else {
            self.groups.iter().map(|g| g.count).sum()
        };
        let query = Query::parse(&self.search);
        let visible = self
            .servers
            .iter()
            .filter(|s| self.visible(s, &query))
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

/// `1 container`, `3 containers`.
fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}
