//! What a service is, decided by scoring evidence rather than by a chain of
//! guesses.
//!
//! Identification used to be a `match` over ports and substrings: first hit
//! wins, and everything a signature could carry — the display name, the health
//! path, the URI to copy, what the service says on connect — lived in a
//! different function keyed by the same guesswork.
//!
//! Here a signature is data, and matching is scored, because the evidence
//! genuinely disagrees. Port 3000 says "some web thing"; a `<title>` saying
//! `Grafana` says Grafana. The title should win, and it does:
//!
//! | evidence | weight | why |
//! |---|---|---|
//! | banner | 100 | the service introducing itself, unprompted |
//! | HTTP title | 70 | the service's own page, naming itself |
//! | HTTP `Server` | 50 | usually the service, sometimes a proxy in front |
//! | process name | 40 | what the user actually launched |
//! | port | 20 | a convention, and a crowded one |
//!
//! Scores add, so a signature matching a port *and* a process name beats one
//! matching either alone — which is how two things sharing port 3000 are told
//! apart.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;

use crate::model::{Kind, glob_match, process_match};

/// The signatures quarry ships with.
const BUILTIN: &str = include_str!("../signatures.toml");

pub const W_BANNER: u32 = 100;
pub const W_TITLE: u32 = 70;
pub const W_SERVER: u32 = 50;
pub const W_PROCESS: u32 = 40;
/// A pattern found among the *arguments* of a program that is not an
/// interpreter. `node .../serve` and `python -m http.server` are a runtime
/// running a thing, and the thing is the identity. `sitepack-web serve --port
/// 3400` is a program with a subcommand, and the subcommand is not what it is —
/// which is how quarry came to call it the `serve` package.
pub const W_ARGUMENT: u32 = 15;
pub const W_PORT: u32 = 20;

/// Below this, a match is not worth preferring over "we do not know".
pub const MIN_SCORE: u32 = W_PORT;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Origin {
    Builtin,
    User,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Signature {
    pub name: String,
    pub kind: Kind,
    pub ports: Vec<u16>,
    /// Substrings, or globs, matched against the command and its arguments.
    pub process: Vec<String>,
    /// Bytes the service sends unprompted on connect.
    pub banner: Option<Vec<u8>>,
    pub http_server: Option<String>,
    pub http_title: Option<String>,
    /// The path that means healthy. `/` when unset.
    pub health: Option<String>,
    /// What a user would paste into a client, with `{port}` and `{path}`.
    pub uri: Option<String>,
    /// A named handshake, for protocols that do not volunteer anything.
    pub probe: Option<String>,
    pub note: Option<String>,
    pub origin: Origin,
}

impl Signature {
    /// The URI to copy, with the placeholders filled in.
    ///
    /// A template that needs a port yields nothing when there is no port — a
    /// unix socket rendered as `http://localhost:0/` is worse than no URI at
    /// all, because it looks like something you could open.
    pub fn uri_for(&self, port: u16, path: Option<&str>) -> Option<String> {
        let template = self.uri.as_ref()?;
        if template.contains("{port}") && port == 0 {
            return None;
        }
        if template.contains("{path}") && path.is_none_or(str::is_empty) {
            return None;
        }
        Some(
            template
                .replace("{port}", &port.to_string())
                .replace("{path}", path.unwrap_or("")),
        )
    }
}

/// Everything known about a service at the moment identification runs. Fields
/// fill in as probing proceeds, so the same signature table serves a first pass
/// with only a port and a later one with a banner.
#[derive(Clone, Debug, Default)]
pub struct Evidence<'a> {
    pub command: &'a str,
    pub cmdline: &'a str,
    pub ports: &'a [u16],
    pub banner: Option<&'a [u8]>,
    pub http_server: Option<&'a str>,
    pub http_title: Option<&'a str>,
}

/// Why a signature matched, and how strongly. Shown by `quarry why`, because a
/// classifier nobody can interrogate is one people argue with rather than fix.
#[derive(Clone, Debug, PartialEq)]
pub struct Verdict {
    pub name: String,
    pub kind: Kind,
    pub score: u32,
    pub reasons: Vec<String>,
    pub origin: Origin,
    pub index: usize,
}

impl Verdict {
    /// Whether this is enough to put a name on the screen.
    ///
    /// A port on its own is not. Ports are conventions, they are reused, and
    /// several hundred signatures claim one — so a bare port match would let
    /// quarry announce "Dex" for whatever happened to bind 4470. It is still
    /// worth something: it carries the *kind*, which is a guess about a
    /// category rather than a claim about a specific piece of software.
    pub fn names_the_service(&self) -> bool {
        self.score > W_PORT
    }
}

#[derive(Debug, Default)]
pub struct Registry {
    signatures: Vec<Signature>,
}

impl Registry {
    /// The compiled-in table, plus the user's file merged over it.
    pub fn load() -> (Registry, Vec<String>) {
        let mut problems = Vec::new();
        let mut signatures = match parse(BUILTIN, Origin::Builtin) {
            Ok(s) => s,
            Err(e) => {
                // A broken built-in table is a bug in quarry, not in the user's
                // machine, and it must not stop the tool from running.
                problems.push(format!("built-in signatures: {e}"));
                Vec::new()
            }
        };

        let path = user_path();
        if path.is_file() {
            match std::fs::read_to_string(&path) {
                Ok(body) => match parse(&body, Origin::User) {
                    Ok(user) => signatures.extend(user),
                    Err(e) => problems.push(format!("{}: {e}", path.display())),
                },
                Err(e) => problems.push(format!("{}: {e}", path.display())),
            }
        }

        (Registry { signatures }, problems)
    }

    pub fn from_toml(body: &str, origin: Origin) -> Result<Registry, String> {
        Ok(Registry {
            signatures: parse(body, origin)?,
        })
    }

    pub fn builtin() -> Registry {
        Registry {
            signatures: parse(BUILTIN, Origin::Builtin).unwrap_or_default(),
        }
    }

    pub fn len(&self) -> usize {
        self.signatures.len()
    }

    pub fn is_empty(&self) -> bool {
        self.signatures.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Signature> {
        self.signatures.iter()
    }

    pub fn get(&self, index: usize) -> Option<&Signature> {
        self.signatures.get(index)
    }

    /// The best match, if anything matched well enough to be worth saying.
    pub fn identify(&self, evidence: &Evidence) -> Option<Verdict> {
        self.explain(evidence).into_iter().next()
    }

    /// Every signature that matched, best first. The whole ranking, so a wrong
    /// answer can be diagnosed rather than argued about.
    pub fn explain(&self, evidence: &Evidence) -> Vec<Verdict> {
        // Lowercased once for the whole table rather than once per signature.
        // This runs for every probe result on every scan, and a hundred
        // signatures used to mean a hundred copies of the same command line.
        let folded = Folded::new(evidence);

        let mut out: Vec<Verdict> = self
            .signatures
            .iter()
            .enumerate()
            .filter_map(|(index, sig)| score(sig, evidence, &folded, index))
            .filter(|v| v.score >= MIN_SCORE)
            .collect();

        out.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                // A user's own signature wins a tie: they know something about
                // their machine that a shipped table cannot.
                .then_with(|| user_first(b.origin).cmp(&user_first(a.origin)))
                .then_with(|| b.reasons.len().cmp(&a.reasons.len()))
                .then_with(|| a.name.cmp(&b.name))
        });
        out
    }
}

fn user_first(origin: Origin) -> u8 {
    match origin {
        Origin::User => 1,
        Origin::Builtin => 0,
    }
}

/// A command line reduced to the names in it, with the directories dropped.
///
/// Matching against a raw command line matches against every directory on the
/// way to the binary. On this machine that made `code` — from Xdebug's
/// signature — match `/Users/someone/code/buildstack/...`, and quarry announced
/// an IDE listener that was not there. A path is where a program lives, not
/// what it is.
fn basenames(cmdline: &str) -> String {
    cmdline
        .split_whitespace()
        .map(|token| {
            // Keep flags whole; a value after `=` is usually a path too.
            if token.starts_with('-') {
                return token;
            }
            token.rsplit('/').next().unwrap_or(token)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The case-folded evidence, computed once per lookup.
struct Folded {
    command: String,
    /// argv[0], reduced to its name.
    program: String,
    /// The rest of argv, reduced to names.
    arguments: String,
    /// Whether argv[0] is a runtime rather than an application, in which case
    /// what it is running is the identity.
    interpreter: bool,
    http_server: Option<String>,
    http_title: Option<String>,
}

impl Folded {
    fn new(evidence: &Evidence) -> Folded {
        let cmdline = basenames(&evidence.cmdline.to_lowercase());
        let (program, arguments) = cmdline.split_once(' ').unwrap_or((cmdline.as_str(), ""));
        Folded {
            interpreter: is_interpreter(program),
            command: evidence.command.to_lowercase(),
            program: program.to_string(),
            arguments: arguments.to_string(),
            http_server: evidence.http_server.map(str::to_lowercase),
            http_title: evidence.http_title.map(str::to_lowercase),
        }
    }
}

/// Runtimes that carry their identity in their arguments rather than in their
/// own name.
fn is_interpreter(program: &str) -> bool {
    let base = program.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    matches!(
        base,
        "node"
            | "nodejs"
            | "python"
            | "ruby"
            | "java"
            | "php"
            | "perl"
            | "deno"
            | "bun"
            | "uv"
            | "uvx"
            | "npx"
            | "pnpm"
            | "yarn"
            | "npm"
            | "bundle"
            | "poetry"
            | "pipx"
    )
}

fn score(sig: &Signature, evidence: &Evidence, folded: &Folded, index: usize) -> Option<Verdict> {
    let mut score = 0;
    let mut reasons = Vec::new();

    if let (Some(pattern), Some(banner)) = (&sig.banner, evidence.banner)
        && banner.starts_with(pattern.as_slice())
    {
        score += W_BANNER;
        reasons.push(format!("banner {}", printable(pattern)));
    }
    if let (Some(pattern), Some(title)) = (&sig.http_title, folded.http_title.as_deref())
        && glob_match(&pattern.to_lowercase(), title)
    {
        score += W_TITLE;
        reasons.push(format!("title {pattern:?}"));
    }
    if let (Some(pattern), Some(server)) = (&sig.http_server, folded.http_server.as_deref())
        && glob_match(&pattern.to_lowercase(), server)
    {
        score += W_SERVER;
        reasons.push(format!("server {pattern:?}"));
    }
    if !sig.process.is_empty() {
        if let Some(hit) = sig
            .process
            .iter()
            .find(|p| process_match(p, &folded.command) || process_match(p, &folded.program))
        {
            score += W_PROCESS;
            reasons.push(format!("process {hit:?}"));
        } else if let Some(hit) = sig
            .process
            .iter()
            .find(|p| process_match(p, &folded.arguments))
        {
            let weight = if folded.interpreter {
                W_PROCESS
            } else {
                W_ARGUMENT
            };
            score += weight;
            reasons.push(if folded.interpreter {
                format!("process {hit:?}")
            } else {
                format!("argument {hit:?}")
            });
        }
    }
    if !sig.ports.is_empty()
        && let Some(port) = evidence.ports.iter().find(|p| sig.ports.contains(p))
    {
        score += W_PORT;
        reasons.push(format!("port {port}"));
    }

    (score > 0).then(|| Verdict {
        name: sig.name.clone(),
        kind: sig.kind,
        score,
        reasons,
        origin: sig.origin,
        index,
    })
}

/// A banner is bytes, and most of them are text. Show the text where there is
/// text and hex where there is not.
fn printable(bytes: &[u8]) -> String {
    if bytes.iter().all(|b| b.is_ascii_graphic() || *b == b' ') {
        format!("{:?}", String::from_utf8_lossy(bytes))
    } else {
        format!(
            "hex:{}",
            bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
        )
    }
}

pub fn user_path() -> PathBuf {
    crate::theme::config_dir().join("signatures.toml")
}

// ─── the on-disk form ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct File {
    #[serde(default)]
    signature: Vec<Entry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    name: String,
    kind: String,
    #[serde(default)]
    ports: Vec<u16>,
    #[serde(default)]
    process: Vec<String>,
    banner: Option<String>,
    banner_text: Option<String>,
    http_server: Option<String>,
    http_title: Option<String>,
    health: Option<String>,
    uri: Option<String>,
    probe: Option<String>,
    note: Option<String>,
}

fn parse(body: &str, origin: Origin) -> Result<Vec<Signature>, String> {
    let file: File = toml::from_str(body).map_err(|e| terse(&e.to_string()))?;

    let mut out = Vec::with_capacity(file.signature.len());
    for entry in file.signature {
        let Some(kind) = Kind::from_label(&entry.kind) else {
            // One bad entry must not cost the rest of the table.
            crate::diag::warn(
                "signatures",
                format!("{}: {:?} is not a kind", entry.name, entry.kind),
            );
            continue;
        };
        let banner = match (&entry.banner, &entry.banner_text) {
            (Some(hex), _) => match parse_banner(hex) {
                Some(b) => Some(b),
                None => {
                    crate::diag::warn(
                        "signatures",
                        format!("{}: {:?} is not a byte pattern", entry.name, hex),
                    );
                    None
                }
            },
            (None, Some(text)) => Some(text.as_bytes().to_vec()),
            (None, None) => None,
        };
        out.push(Signature {
            name: entry.name,
            kind,
            ports: entry.ports,
            // Folded at load so the hot path never has to.
            process: entry.process.iter().map(|p| p.to_lowercase()).collect(),
            banner,
            http_server: entry.http_server,
            http_title: entry.http_title,
            health: entry.health,
            uri: entry.uri,
            probe: entry.probe,
            note: entry.note,
            origin,
        });
    }
    Ok(out)
}

/// A TOML error on one line, keeping the part that says what is wrong.
///
/// `toml` reports the location first and the reason last, with a caret diagram
/// between. Someone editing a signature file needs both ends and none of the
/// middle.
fn terse(error: &str) -> String {
    let mut lines = error.lines().filter(|l| !l.trim().is_empty());
    let location = lines.next().unwrap_or("could not be parsed").trim();
    let reason = error
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| {
            !l.is_empty()
                && !l.starts_with('|')
                && !l.starts_with('^')
                && !l.chars().next().is_some_and(|c| c.is_ascii_digit())
                && *l != "|"
        })
        .filter(|l| *l != location);
    match reason {
        Some(reason) => format!("{location}: {}", first_clause(reason)),
        None => location.to_string(),
    }
}

/// `unknown field \`prot\`, expected one of ...` — the list is longer than the
/// terminal and the field name is the whole message.
fn first_clause(reason: &str) -> &str {
    reason.split(", expected").next().unwrap_or(reason)
}

/// `hex:0a0b0c`, or a plain string taken literally. A binary greeting — MySQL's,
/// for one — cannot be written any other way.
fn parse_banner(raw: &str) -> Option<Vec<u8>> {
    let Some(hex) = raw.strip_prefix("hex:") else {
        return Some(raw.as_bytes().to_vec());
    };
    let hex: String = hex.chars().filter(|c| !c.is_whitespace()).collect();
    if hex.is_empty() || !hex.len().is_multiple_of(2) {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
}

/// A summary of the table, for `quarry signatures`.
pub fn summary(registry: &Registry) -> BTreeMap<Kind, usize> {
    let mut counts = BTreeMap::new();
    for sig in registry.iter() {
        *counts.entry(sig.kind).or_insert(0) += 1;
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[[signature]]
name = "PostgreSQL"
kind = "db"
ports = [5432]
process = ["postgres", "postmaster"]
uri = "postgres://localhost:{port}/"

[[signature]]
name = "Grafana"
kind = "metrics"
ports = [3000]
http_title = "Grafana*"
health = "/api/health"

[[signature]]
name = "Next.js"
kind = "web"
ports = [3000]
process = ["next-server", "next dev"]

[[signature]]
name = "OpenSSH"
kind = "system"
ports = [22]
banner_text = "SSH-"

[[signature]]
name = "MySQL"
kind = "db"
ports = [3306]
banner = "hex:0a"
"#;

    fn registry() -> Registry {
        Registry::from_toml(SAMPLE, Origin::Builtin).expect("sample parses")
    }

    fn evidence<'a>(ports: &'a [u16], command: &'a str, cmdline: &'a str) -> Evidence<'a> {
        Evidence {
            command,
            cmdline,
            ports,
            ..Default::default()
        }
    }

    /// A port carries a category and nothing more. Several hundred signatures
    /// claim a port; letting one of them put a name on the screen is how
    /// quarry ends up announcing "Dex" for whatever bound 4470.
    #[test]
    fn a_port_alone_suggests_a_kind_but_names_nothing() {
        let r = registry();
        let v = r.identify(&evidence(&[5432], "", "")).expect("matched");
        assert_eq!(v.kind, Kind::Database, "the category is a fair guess");
        assert_eq!(v.score, W_PORT);
        assert!(
            !v.names_the_service(),
            "a bare port must not be enough to name a service"
        );
    }

    #[test]
    fn any_second_piece_of_evidence_is_enough_to_name_it() {
        let r = registry();
        let v = r
            .identify(&evidence(&[5432], "postgres", "postgres -D /data"))
            .expect("matched");
        assert!(v.names_the_service());
        assert_eq!(v.name, "PostgreSQL");
    }

    #[test]
    fn evidence_adds_up() {
        let r = registry();
        let v = r
            .identify(&evidence(&[5432], "postgres", "/usr/bin/postgres -D /data"))
            .expect("matched");
        assert_eq!(v.score, W_PORT + W_PROCESS, "port and process both counted");
        assert_eq!(v.reasons.len(), 2);
    }

    /// The case the whole design is for: two services on one port.
    #[test]
    fn a_title_beats_a_shared_port() {
        let r = registry();
        let grafana = Evidence {
            ports: &[3000],
            http_title: Some("Grafana"),
            ..Default::default()
        };
        assert_eq!(r.identify(&grafana).expect("matched").name, "Grafana");

        let next = evidence(&[3000], "node", "next-server (v16.3.4)");
        assert_eq!(r.identify(&next).expect("matched").name, "Next.js");
    }

    #[test]
    fn a_banner_beats_everything_else() {
        let r = registry();
        // A port that says PostgreSQL and a banner that says SSH.
        let ev = Evidence {
            ports: &[5432],
            banner: Some(b"SSH-2.0-OpenSSH_9.6\r\n"),
            command: "postgres",
            cmdline: "postgres",
            ..Default::default()
        };
        let v = r.identify(&ev).expect("matched");
        assert_eq!(
            v.name, "OpenSSH",
            "the service speaking outranks the port it happens to be on"
        );
    }

    /// A path is where a program lives, not what it is.
    #[test]
    fn directories_on_the_way_to_a_binary_are_not_evidence() {
        let r = Registry::from_toml(
            "[[signature]]\nname = \"Xdebug\"\nkind = \"debug\"\nprocess = [\"code\"]\n",
            Origin::Builtin,
        )
        .expect("parses");
        let ev = evidence(
            &[8790],
            "buildstack",
            "/Users/someone/code/buildstack/target/release/buildstack up",
        );
        assert!(
            r.identify(&ev).is_none(),
            "a directory called `code` is not an Xdebug listener"
        );

        // And the real thing still matches.
        let real = evidence(&[8790], "code", "/usr/local/bin/code --listen");
        assert!(r.identify(&real).is_some());
    }

    /// `sitepack-web serve --port 3400` is not the `serve` package.
    #[test]
    fn a_subcommand_is_not_an_identity() {
        let r = Registry::from_toml(
            "[[signature]]\nname = \"serve\"\nkind = \"web\"\nprocess = [\"serve\"]\n",
            Origin::Builtin,
        )
        .expect("parses");

        let subcommand = evidence(
            &[3400],
            "sitepack-web",
            "target/debug/sitepack-web serve --port 3400",
        );
        let v = r.identify(&subcommand);
        assert!(
            v.is_none_or(|v| !v.names_the_service()),
            "a subcommand of another program must not name the service"
        );

        // A runtime running it, on the other hand, is exactly the identity.
        let via_node = evidence(
            &[3000],
            "node",
            "node /app/node_modules/.bin/serve -s build",
        );
        let v = r.identify(&via_node).expect("matched");
        assert!(v.names_the_service());
        assert_eq!(v.name, "serve");

        // And so is running it directly.
        let direct = evidence(&[3000], "serve", "/usr/local/bin/serve -s build");
        assert!(r.identify(&direct).expect("matched").names_the_service());
    }

    #[test]
    fn a_binary_banner_matches() {
        let r = registry();
        let ev = Evidence {
            ports: &[3306],
            banner: Some(&[0x0a, 0x38, 0x2e, 0x30]),
            ..Default::default()
        };
        assert_eq!(r.identify(&ev).expect("matched").name, "MySQL");
    }

    #[test]
    fn nothing_matching_is_not_an_identification() {
        let r = registry();
        assert!(
            r.identify(&evidence(&[54321], "mystery", "mystery"))
                .is_none()
        );
    }

    #[test]
    fn the_ranking_is_available_for_inspection() {
        let r = registry();
        let ev = evidence(&[3000], "node", "next-server");
        let all = r.explain(&ev);
        assert_eq!(all.len(), 2, "both port-3000 signatures matched: {all:?}");
        assert_eq!(all[0].name, "Next.js");
        assert!(all[0].score > all[1].score);
        assert!(
            all[0].reasons.iter().any(|r| r.contains("process")),
            "the winning reason has to be legible: {:?}",
            all[0].reasons
        );
    }

    #[test]
    fn a_user_signature_wins_a_tie() {
        let mut r = registry();
        let mut mine = Registry::from_toml(
            "[[signature]]\nname = \"Mine\"\nkind = \"api\"\nports = [5432]\n",
            Origin::User,
        )
        .expect("parses");
        r.signatures.append(&mut mine.signatures);

        let v = r.identify(&evidence(&[5432], "", "")).expect("matched");
        assert_eq!(v.name, "Mine", "the user knows their own machine");
    }

    #[test]
    fn uri_templates_are_filled_in() {
        let r = registry();
        let pg = r.iter().find(|s| s.name == "PostgreSQL").expect("present");
        assert_eq!(
            pg.uri_for(5433, None).as_deref(),
            Some("postgres://localhost:5433/")
        );
        assert_eq!(
            pg.uri_for(0, None),
            None,
            "a unix socket has no port, and `localhost:0` looks openable when it is not"
        );
    }

    #[test]
    fn banner_patterns_parse_in_both_forms() {
        assert_eq!(parse_banner("hex:0a1b"), Some(vec![0x0a, 0x1b]));
        assert_eq!(parse_banner("hex:0a 1b"), Some(vec![0x0a, 0x1b]));
        assert_eq!(parse_banner("SSH-"), Some(b"SSH-".to_vec()));
        assert_eq!(parse_banner("hex:0a1"), None, "odd digit count");
        assert_eq!(parse_banner("hex:zz"), None);
    }

    #[test]
    fn a_bad_entry_does_not_cost_the_rest_of_the_table() {
        let r = Registry::from_toml(
            "[[signature]]\nname = \"Bad\"\nkind = \"not-a-kind\"\n\n\
             [[signature]]\nname = \"Good\"\nkind = \"api\"\nports = [1234]\n",
            Origin::Builtin,
        )
        .expect("the file still parses");
        assert_eq!(r.len(), 1);
        assert_eq!(r.iter().next().expect("one").name, "Good");
    }

    #[test]
    fn an_unknown_field_is_rejected_so_a_typo_is_visible() {
        let err = Registry::from_toml(
            "[[signature]]\nname = \"X\"\nkind = \"api\"\nprot = [80]\n",
            Origin::User,
        )
        .unwrap_err();
        assert!(err.contains("prot"), "{err}");
    }

    // ── the shipped table ───────────────────────────────────────────────────

    #[test]
    fn the_builtin_table_is_valid() {
        let r = Registry::builtin();
        assert!(r.len() > 20, "only {} signatures shipped", r.len());
        for sig in r.iter() {
            assert!(!sig.name.is_empty());
            assert!(
                sig.ports.iter().all(|p| *p > 0),
                "{}: port 0 is not a port",
                sig.name
            );
            if let Some(health) = &sig.health {
                assert!(
                    health.starts_with('/'),
                    "{}: health path {health:?} is not a path",
                    sig.name
                );
            }
            if let Some(uri) = &sig.uri {
                assert!(
                    uri.contains("{port}") || uri.contains("{path}"),
                    "{}: uri {uri:?} has nothing to fill in",
                    sig.name
                );
            }
            assert!(
                sig.ports.is_empty()
                    || !sig.process.is_empty()
                    || sig.banner.is_some()
                    || sig.http_title.is_some()
                    || sig.http_server.is_some()
                    || !CONTESTED.contains(&sig.ports[0]),
                "{}: claims contested port {} with no other evidence",
                sig.name,
                sig.ports[0]
            );
        }
    }

    /// Ports so many things use that claiming one, alone, is not information.
    const CONTESTED: [u16; 9] = [80, 3000, 4000, 5000, 8000, 8080, 8081, 8888, 9000];

    #[test]
    fn no_two_shipped_signatures_share_a_name() {
        let r = Registry::builtin();
        let mut names: Vec<&str> = r.iter().map(|s| s.name.as_str()).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "duplicate names in the shipped table");
    }

    #[test]
    fn the_shipped_table_identifies_what_it_claims_to() {
        let r = Registry::builtin();
        for (name, ev) in [
            (
                "PostgreSQL",
                Evidence {
                    ports: &[5432],
                    command: "postgres",
                    cmdline: "postgres -D /data",
                    ..Default::default()
                },
            ),
            (
                "Redis",
                Evidence {
                    ports: &[6379],
                    command: "redis-server",
                    cmdline: "redis-server *:6379",
                    ..Default::default()
                },
            ),
        ] {
            let v = r
                .identify(&ev)
                .unwrap_or_else(|| panic!("{name} not identified"));
            assert!(
                v.name.contains(name),
                "expected {name}, got {} ({:?})",
                v.name,
                v.reasons
            );
        }
    }
}
