//! How the list is arranged, and what the filter box is asking for.

use super::{Server, contains_ci, port_contains};

/// How the list is divided.
///
/// By project is what quarry is for — the question it answers that `lsof` does
/// not. But once you are asking a different question the division gets in the
/// way: "every database on this machine" wants them together, and a filtered
/// list wants no headings at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum GroupBy {
    #[default]
    Project,
    Kind,
    /// One flat list. Not a group of everything — no headings at all.
    Nothing,
}

impl GroupBy {
    pub const ALL: &'static [GroupBy] = &[GroupBy::Project, GroupBy::Kind, GroupBy::Nothing];

    pub fn next(self) -> GroupBy {
        match self {
            GroupBy::Project => GroupBy::Kind,
            GroupBy::Kind => GroupBy::Nothing,
            GroupBy::Nothing => GroupBy::Project,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            GroupBy::Project => "project",
            GroupBy::Kind => "kind",
            GroupBy::Nothing => "nothing",
        }
    }

    pub fn from_name(name: &str) -> Option<GroupBy> {
        let name = name.trim().to_lowercase();
        GroupBy::ALL.iter().copied().find(|g| g.name() == name)
    }
}

/// The order within a group.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SortBy {
    /// Health, then port. What you want when you are looking for a problem.
    #[default]
    Health,
    Port,
    Name,
    /// Most recently started first — what did I just start, and what has been
    /// up since Tuesday.
    Newest,
}

impl SortBy {
    pub const ALL: &'static [SortBy] =
        &[SortBy::Health, SortBy::Port, SortBy::Name, SortBy::Newest];

    pub fn next(self) -> SortBy {
        match self {
            SortBy::Health => SortBy::Port,
            SortBy::Port => SortBy::Name,
            SortBy::Name => SortBy::Newest,
            SortBy::Newest => SortBy::Health,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            SortBy::Health => "health",
            SortBy::Port => "port",
            SortBy::Name => "name",
            SortBy::Newest => "newest",
        }
    }

    pub fn from_name(name: &str) -> Option<SortBy> {
        let name = name.trim().to_lowercase();
        SortBy::ALL.iter().copied().find(|s| s.name() == name)
    }
}

/// What the filter box is asking for.
///
/// Plain words match anything about a service, which is the right default —
/// most of the time you half-remember one thing about it. But "3000" also
/// matches a pid, a latency and a command line, and `db` matches every
/// database *and* every path with `db` in it. A prefix narrows the question to
/// one field, and several terms narrow together.
///
/// `:3000 @web` — web servers on a port containing 3000.
/// `~acme` — anything belonging to the acme project.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    terms: Vec<Term>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Term {
    /// `!` in front: match everything this does not.
    negated: bool,
    what: Match,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Match {
    /// `:3000` — the port.
    Port(String),
    /// `@web` — the kind.
    Kind(String),
    /// `~acme` — the project.
    Project(String),
    /// Anything at all.
    Any(String),
}

impl Query {
    pub fn parse(text: &str) -> Query {
        let terms = text
            .split_whitespace()
            .filter_map(|word| {
                let (negated, word) = match word.strip_prefix('!') {
                    Some(rest) => (true, rest),
                    None => (false, word),
                };
                let (make, rest) = match word.split_at_checked(1) {
                    Some((":", rest)) => (Match::Port as fn(String) -> Match, rest),
                    Some(("@", rest)) => (Match::Kind as fn(String) -> Match, rest),
                    Some(("~", rest)) => (Match::Project as fn(String) -> Match, rest),
                    _ => (Match::Any as fn(String) -> Match, word),
                };
                // A lone `:` is someone part-way through typing, not a term
                // that matches everything — and a lone `!` is not a term that
                // matches nothing.
                (!rest.is_empty()).then(|| Term {
                    negated,
                    what: make(rest.to_lowercase()),
                })
            })
            .collect();
        Query { terms }
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }
}

impl Server {
    /// Every term has to match. Terms narrow; they do not accumulate
    /// alternatives — `:3000 @web` means both, which is the only reading that
    /// makes typing a second term useful.
    pub fn satisfies(&self, query: &Query) -> bool {
        query.terms.iter().all(|term| {
            let hit = match &term.what {
                Match::Port(p) => self.listeners.iter().any(|l| port_contains(l.port, p)),
                Match::Kind(k) => contains_ci(self.kind.label(), k),
                Match::Project(r) => contains_ci(&self.group_key(), r),
                Match::Any(word) => self.matches(word),
            };
            hit != term.negated
        })
    }
}
