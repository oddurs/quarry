//! User configuration.
//!
//! Read from `$QUARRY_CONFIG`, else `~/.config/quarry/config.toml`, else
//! defaults. Deliberately not per-directory: quarry looks at the whole machine,
//! so a config that changed depending on where you launched it would be a trap.
//!
//! Nothing here is fatal. A missing file is the normal case, a malformed one is
//! reported and stepped over, and an unknown key is a warning rather than a
//! refusal — a config written for a newer quarry has to keep working on an
//! older one.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::diag;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// `auto`, `mono`, a built-in name, `ghostty:<name>`, or a path.
    pub theme: String,
    /// Include system services on startup, as `--all` does.
    pub show_all: bool,
    /// Seconds between automatic rescans.
    pub refresh_secs: u64,
    /// How long to wait for a TCP connection before calling a port closed.
    pub connect_ms: u64,
    /// How long to wait for an HTTP response.
    pub request_ms: u64,
    /// Concurrent probes. More helps a machine with many services; each one is
    /// a thread.
    pub probe_workers: usize,
    /// How much of a response body to read when looking for a `<title>`.
    pub max_body_kb: u64,
    /// How long to wait for a service to introduce itself on connect.
    pub banner_ms: u64,

    /// Port → kind, merged over the built-in table.
    #[serde(default)]
    pub ports: BTreeMap<String, String>,
    /// Process name → kind. `*` matches any run of characters.
    #[serde(default)]
    pub names: BTreeMap<String, String>,
    /// Port → the path to probe. `"*"` sets the default for everything else.
    #[serde(default)]
    pub health: BTreeMap<String, String>,
    /// Key → action name.
    #[serde(default)]
    pub keys: BTreeMap<String, String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: crate::theme::DEFAULT.to_string(),
            show_all: false,
            refresh_secs: 6,
            connect_ms: 400,
            request_ms: 1800,
            probe_workers: 12,
            max_body_kb: 96,
            banner_ms: 250,
            ports: BTreeMap::new(),
            names: BTreeMap::new(),
            health: BTreeMap::new(),
            keys: BTreeMap::new(),
        }
    }
}

/// Top-level keys quarry knows. Anything else is warned about rather than
/// rejected, so a config written for a newer version still loads here.
const KNOWN: &[&str] = &[
    "theme",
    "show_all",
    "refresh_secs",
    "connect_ms",
    "request_ms",
    "probe_workers",
    "max_body_kb",
    "banner_ms",
    "ports",
    "names",
    "health",
    "keys",
];

impl Config {
    /// Where the config would be read from, if it exists.
    pub fn path() -> PathBuf {
        if let Some(explicit) = std::env::var_os("QUARRY_CONFIG").filter(|v| !v.is_empty()) {
            return PathBuf::from(explicit);
        }
        crate::theme::config_dir().join("config.toml")
    }

    /// Load, reporting problems rather than failing. Returns the config and the
    /// path it came from, if any.
    pub fn load(explicit: Option<&Path>) -> (Config, Option<PathBuf>) {
        let path = match explicit {
            Some(p) => p.to_path_buf(),
            None => Self::path(),
        };
        if !path.exists() {
            if explicit.is_some() {
                diag::warn("config", format!("{} does not exist", path.display()));
            }
            return (Config::default(), None);
        }
        let body = match std::fs::read_to_string(&path) {
            Ok(b) => b,
            Err(e) => {
                diag::error("config", format!("{}: {e}", path.display()));
                return (Config::default(), None);
            }
        };
        let config = Self::parse(&body, &path.display().to_string());
        (config, Some(path))
    }

    /// Parse a config body. Never fails: a broken file yields defaults and a
    /// diagnostic, because refusing to start over a typo in an optional file is
    /// worse than starting without it.
    pub fn parse(body: &str, source: &str) -> Config {
        let table: toml::Table = match toml::from_str(body) {
            Ok(t) => t,
            Err(e) => {
                diag::error(
                    "config",
                    format!("{source}: {}", e.to_string().lines().next().unwrap_or("")),
                );
                return Config::default();
            }
        };
        for key in table.keys() {
            if !KNOWN.contains(&key.as_str()) {
                diag::warn("config", format!("{source}: ignoring unknown key {key:?}"));
            }
        }
        let mut config: Config = match table.try_into() {
            Ok(c) => c,
            Err(e) => {
                diag::error(
                    "config",
                    format!("{source}: {}", e.to_string().lines().next().unwrap_or("")),
                );
                return Config::default();
            }
        };
        config.clamp(source);
        config
    }

    /// Hold every setting inside a range where quarry still works. A refresh of
    /// zero is a busy loop, and four thousand probe threads is a fork bomb with
    /// extra steps.
    fn clamp(&mut self, source: &str) {
        let note = |what: &str, from: String, to: String| {
            diag::warn(
                "config",
                format!("{source}: {what} {from} out of range, using {to}"),
            );
        };
        if !(1..=3600).contains(&self.refresh_secs) {
            let was = self.refresh_secs;
            self.refresh_secs = self.refresh_secs.clamp(1, 3600);
            note(
                "refresh_secs",
                was.to_string(),
                self.refresh_secs.to_string(),
            );
        }
        if !(20..=60_000).contains(&self.connect_ms) {
            let was = self.connect_ms;
            self.connect_ms = self.connect_ms.clamp(20, 60_000);
            note("connect_ms", was.to_string(), self.connect_ms.to_string());
        }
        if !(50..=120_000).contains(&self.request_ms) {
            let was = self.request_ms;
            self.request_ms = self.request_ms.clamp(50, 120_000);
            note("request_ms", was.to_string(), self.request_ms.to_string());
        }
        if !(1..=128).contains(&self.probe_workers) {
            let was = self.probe_workers;
            self.probe_workers = self.probe_workers.clamp(1, 128);
            note(
                "probe_workers",
                was.to_string(),
                self.probe_workers.to_string(),
            );
        }
        if !(0..=5_000).contains(&self.banner_ms) {
            let was = self.banner_ms;
            self.banner_ms = self.banner_ms.clamp(0, 5_000);
            note("banner_ms", was.to_string(), self.banner_ms.to_string());
        }
        if !(1..=8192).contains(&self.max_body_kb) {
            let was = self.max_body_kb;
            self.max_body_kb = self.max_body_kb.clamp(1, 8192);
            note("max_body_kb", was.to_string(), self.max_body_kb.to_string());
        }
    }

    pub fn refresh(&self) -> Duration {
        Duration::from_secs(self.refresh_secs)
    }

    pub fn connect_timeout(&self) -> Duration {
        Duration::from_millis(self.connect_ms)
    }

    pub fn request_timeout(&self) -> Duration {
        Duration::from_millis(self.request_ms)
    }

    pub fn max_body(&self) -> u64 {
        self.max_body_kb * 1024
    }

    /// The path to probe for a given port.
    pub fn health_path(&self, port: u16) -> &str {
        self.health
            .get(&port.to_string())
            .or_else(|| self.health.get("*"))
            .map(String::as_str)
            .unwrap_or("/")
    }

    /// Render as TOML, for `quarry config`.
    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).unwrap_or_else(|e| format!("# could not render: {e}\n"))
    }

    /// A fully commented default file, for `quarry config --write`. Kept here
    /// rather than in the docs so the two cannot drift.
    pub fn commented_default() -> String {
        let d = Config::default();
        format!(
            r##"# quarry configuration
#
# Every key is optional. Delete anything you do not want to change — quarry
# reads what is here and uses its own defaults for the rest.
#
#   quarry config       show what is actually in effect
#   quarry themes       list every theme quarry can find

# A theme name, or "auto" to use your terminal's own colours.
# Try: auto, mono, gotham, night, paper, ghostty:<name>, or a path to a file.
theme = "{theme}"

# Include system services — Dropbox, mDNS, and the rest — on startup.
show_all = {show_all}

# Seconds between automatic rescans.
refresh_secs = {refresh_secs}

# How long to wait for a TCP connection before calling a port closed.
connect_ms = {connect_ms}

# How long to wait for an HTTP response.
request_ms = {request_ms}

# Concurrent probes. Each is a thread; more helps a busy machine.
probe_workers = {probe_workers}

# How much of a response body to read while looking for a <title>.
max_body_kb = {max_body_kb}

# How long to wait for a service to introduce itself on connect. Many protocols
# greet you — SSH, SMTP, MySQL, NATS — and hearing them costs nothing when they
# do. Only silent services pay this, and only ones nothing else could identify.
banner_ms = {banner_ms}

# Ports quarry does not already know about. These merge over the built-in
# table, so you add what you run without losing what quarry knows.
# Kinds: web api db cache search queue proxy mail container ai tool system other
[ports]
# 9174 = "queue"
# 7788 = "api"

# Process names, matched against the command and its arguments.
# "*" matches any run of characters.
[names]
# "my-daemon" = "api"
# "*-worker"  = "queue"

# The path to probe, per port. A dev server that 404s on / shows red while
# being perfectly healthy, which trains you to ignore the colour.
[health]
# "*"  = "/"
# 3000 = "/healthz"

# Keys, bound to action names. See `quarry --help` for the defaults.
# Actions: quit refresh open copy filter toggle-all toggle-group stop
#          force-kill diagnostics help toggle-mouse reload back
#          down up page-down page-up first last
[keys]
# "ctrl-r" = "reload"
# "x"      = "stop"
"##,
            theme = d.theme,
            show_all = d.show_all,
            refresh_secs = d.refresh_secs,
            connect_ms = d.connect_ms,
            request_ms = d.request_ms,
            probe_workers = d.probe_workers,
            max_body_kb = d.max_body_kb,
            banner_ms = d.banner_ms,
        )
    }

    /// Write the commented default. Refuses to clobber.
    pub fn write_default(path: &Path, force: bool) -> std::io::Result<()> {
        if path.exists() && !force {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!(
                    "{} already exists; pass --force to replace it",
                    path.display()
                ),
            ));
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, Self::commented_default())
    }

    /// Which settings differ from the defaults. Used by `quarry config` so a
    /// value that came from the file is visible as such.
    pub fn overridden(&self) -> Vec<&'static str> {
        let d = Config::default();
        let mut out = Vec::new();
        if self.theme != d.theme {
            out.push("theme");
        }
        if self.show_all != d.show_all {
            out.push("show_all");
        }
        if self.refresh_secs != d.refresh_secs {
            out.push("refresh_secs");
        }
        if self.connect_ms != d.connect_ms {
            out.push("connect_ms");
        }
        if self.request_ms != d.request_ms {
            out.push("request_ms");
        }
        if self.probe_workers != d.probe_workers {
            out.push("probe_workers");
        }
        if self.max_body_kb != d.max_body_kb {
            out.push("max_body_kb");
        }
        if self.banner_ms != d.banner_ms {
            out.push("banner_ms");
        }
        if !self.ports.is_empty() {
            out.push("ports");
        }
        if !self.names.is_empty() {
            out.push("names");
        }
        if !self.health.is_empty() {
            out.push("health");
        }
        if !self.keys.is_empty() {
            out.push("keys");
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_file_is_the_defaults() {
        assert_eq!(Config::parse("", "test"), Config::default());
    }

    #[test]
    fn a_partial_file_changes_only_what_it_names() {
        let c = Config::parse("theme = \"gotham\"\n", "test");
        assert_eq!(c.theme, "gotham");
        assert_eq!(c.refresh_secs, Config::default().refresh_secs);
        assert_eq!(c.overridden(), vec!["theme"]);
    }

    #[test]
    fn a_malformed_file_yields_defaults_rather_than_failing() {
        let _guard = diag::test_lock();
        diag::reset();
        let c = Config::parse("theme = \n", "broken.toml");
        assert_eq!(c, Config::default());
        let seen = diag::recent(20);
        assert!(
            seen.iter().any(|e| e.message.contains("broken.toml")),
            "the failure has to be reportable: {seen:?}"
        );
    }

    #[test]
    fn an_unknown_key_warns_and_is_ignored() {
        let _guard = diag::test_lock();
        diag::reset();
        let c = Config::parse("theme = \"night\"\nfuture_setting = 3\n", "test");
        assert_eq!(c.theme, "night", "the keys we know still apply");
        assert!(
            diag::recent(20)
                .iter()
                .any(|e| e.message.contains("future_setting")),
            "an unknown key must be reported"
        );
    }

    #[test]
    fn a_wrong_type_does_not_take_the_rest_down_with_it() {
        let c = Config::parse("refresh_secs = \"soon\"\n", "test");
        assert_eq!(c, Config::default());
    }

    #[test]
    fn absurd_values_are_clamped_rather_than_obeyed() {
        let c = Config::parse("refresh_secs = 0\nprobe_workers = 100000\n", "test");
        assert_eq!(c.refresh_secs, 1, "a zero refresh is a busy loop");
        assert_eq!(
            c.probe_workers, 128,
            "and four thousand threads is a fork bomb"
        );
    }

    #[test]
    fn health_paths_fall_back_to_the_wildcard_then_to_root() {
        let c = Config::parse("[health]\n\"*\" = \"/up\"\n3000 = \"/healthz\"\n", "test");
        assert_eq!(c.health_path(3000), "/healthz");
        assert_eq!(c.health_path(9999), "/up");
        assert_eq!(Config::default().health_path(3000), "/");
    }

    #[test]
    fn the_written_default_parses_back_to_the_default() {
        let text = Config::commented_default();
        let parsed = Config::parse(&text, "commented_default");
        assert_eq!(
            parsed,
            Config::default(),
            "the file quarry writes must mean what quarry defaults to"
        );
    }

    #[test]
    fn the_rendered_config_round_trips() {
        let mut c = Config {
            theme: "gotham".into(),
            ..Default::default()
        };
        c.ports.insert("9174".into(), "queue".into());
        c.keys.insert("ctrl-r".into(), "reload".into());
        let back = Config::parse(&c.to_toml(), "roundtrip");
        assert_eq!(back, c, "`quarry config` output must load again");
    }

    #[test]
    fn write_default_refuses_to_clobber() {
        let dir = std::env::temp_dir().join(format!("quarry-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("config.toml");
        let _ = std::fs::remove_file(&path);

        Config::write_default(&path, false).expect("first write");
        let err = Config::write_default(&path, false).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        Config::write_default(&path, true).expect("--force replaces it");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_file_is_not_an_error() {
        let (c, path) = Config::load(Some(Path::new("/nonexistent/quarry.toml")));
        assert_eq!(c, Config::default());
        assert_eq!(path, None);
    }
}
