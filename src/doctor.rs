//! `quarry --doctor`: check every external thing quarry depends on, and say
//! which of them is broken before the user has to guess from an empty screen.

use std::time::{Duration, Instant};

use crate::config::Config;
use crate::engine::Engine;
use crate::exec;
use crate::theme::Theme;

pub struct Check {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
    /// False when a failure only costs a feature, not the whole tool.
    pub fatal: bool,
}

impl Check {
    /// Three outcomes, spelled once. As struct literals, sixteen checks came
    /// to sixteen blocks that differed only in two booleans — and the booleans
    /// are what a reader had to decode to know which outcome it was.
    fn ok(name: &'static str, detail: impl Into<String>) -> Check {
        Check {
            name,
            ok: true,
            detail: detail.into(),
            fatal: false,
        }
    }

    /// Broken, but it only costs a feature.
    fn warn(name: &'static str, detail: impl Into<String>) -> Check {
        Check {
            name,
            ok: false,
            detail: detail.into(),
            fatal: false,
        }
    }

    /// Broken, and quarry cannot do its job without it.
    fn fatal(name: &'static str, detail: impl Into<String>) -> Check {
        Check {
            name,
            ok: false,
            detail: detail.into(),
            fatal: true,
        }
    }
}

/// Every check, in the order they are printed.
///
/// A list of named checks rather than one long procedure: what quarry verifies
/// about a machine should be readable as a list, and each entry should be
/// findable by the name it prints.
pub fn run(config: &Config, config_path: Option<&std::path::Path>, theme: &Theme) -> Vec<Check> {
    // Annotated because the first push is inside a platform-specific block:
    // without this the type is never inferred on some platforms.
    let mut checks: Vec<Check> = vec![socket_source()];

    #[cfg(target_os = "macos")]
    checks.push(lsof_fallback(checks[0].ok));

    let (rules, rule_problems) = crate::model::Rules::from_config(&config.ports, &config.names);
    checks.extend(a_real_scan(&rules));
    checks.push(configuration(config, config_path, &rule_problems));
    checks.push(theme_in_use(theme, &rules));
    checks.push(container_runtime());
    checks.push(tool_check("clipboard", clipboard_tool()));
    checks.push(tool_check("browser", browser_tool()));
    checks.push(terminal_size());
    checks.push(logging());
    checks
}

/// Where listening sockets actually come from on this machine, and what that
/// costs. Reported on every platform, naming whichever source is live.
fn socket_source() -> Check {
    let started = Instant::now();
    let mut source = crate::engine::Engine::socket_source();
    let name = source.describe();
    match source.listening() {
        Ok(sockets) => Check::ok(
            "sockets",
            format!(
                "{} listening in {}ms via {name}",
                sockets.len(),
                started.elapsed().as_millis()
            ),
        ),
        Err(e) => Check::fatal("sockets", format!("{name}: {e}")),
    }
}

/// The fallback, where there is a native path to fall back from.
#[cfg(target_os = "macos")]
fn lsof_fallback(native_ok: bool) -> Check {
    // Needed to call `listening` on the concrete `Lsof`; a trait object does
    // not require it, which is why this is not at the top of the file.
    use crate::source::SocketSource;

    let started = Instant::now();
    let mut lsof = crate::lsof::Lsof;
    match lsof.listening() {
        Ok(sockets) => Check::ok(
            "lsof",
            format!(
                "{} listening in {}ms (fallback, not in use)",
                sockets.len(),
                started.elapsed().as_millis()
            ),
        ),
        Err(e) if native_ok => Check::warn("lsof", format!("unavailable ({e}), but not needed")),
        Err(e) => Check::fatal("lsof", e.to_string()),
    }
}

/// A scan of this machine, for what it says about the two things a scan can
/// only partly know: working directories, and which project a service is in.
fn a_real_scan(rules: &crate::model::Rules) -> Vec<Check> {
    let mut engine = Engine::live().with_rules(rules.clone());
    let report = match engine.scan() {
        Ok(report) => report,
        Err(e) => return vec![Check::fatal("scan", e.to_string())],
    };
    let total = report.servers.len();
    let with_cwd = report.servers.iter().filter(|s| s.cwd.is_some()).count();
    let attributed = report.servers.iter().filter(|s| s.repo.is_some()).count();
    vec![
        Check::warn(
            "processes",
            format!("{with_cwd}/{total} have a readable working directory"),
        ),
        Check::ok(
            "projects",
            format!("{attributed}/{total} attributed to a project"),
        ),
    ]
}

fn configuration(config: &Config, path: Option<&std::path::Path>, problems: &[String]) -> Check {
    if !problems.is_empty() {
        return Check::warn(
            "config",
            format!(
                "{} rule(s) could not be understood: {}",
                problems.len(),
                problems.join("; ")
            ),
        );
    }
    let Some(path) = path else {
        return Check::warn("config", "no config file; using defaults");
    };
    let overridden = config.overridden();
    Check::warn(
        "config",
        if overridden.is_empty() {
            format!("{} (nothing overridden)", path.display())
        } else {
            format!("{} — {}", path.display(), overridden.join(", "))
        },
    )
}

fn theme_in_use(theme: &Theme, rules: &crate::model::Rules) -> Check {
    let custom = if rules.is_empty() {
        String::new()
    } else {
        format!(", {} custom rule(s)", rules.len())
    };
    Check::ok(
        "theme",
        format!("{} ({}){custom}", theme.name, theme.source.label()),
    )
}

fn container_runtime() -> Check {
    let containers = crate::docker::Containers::query();
    Check::ok(
        "containers",
        if containers.is_empty() {
            "no container runtime answering; published ports stay unattributed".to_string()
        } else {
            format!(
                "{} published port(s) attributed to a container",
                containers.len()
            )
        },
    )
}

fn terminal_size() -> Check {
    let detail = match crossterm::terminal::size() {
        Ok((w, h)) if w < 60 || h < 12 => format!("{w}×{h} (cramped; 80×24 or more is better)"),
        Ok((w, h)) => format!("{w}×{h}"),
        Err(e) => format!("size unknown: {e}"),
    };
    Check::ok("terminal", detail)
}

fn logging() -> Check {
    Check::ok(
        "log",
        match std::env::var("QUARRY_LOG") {
            Ok(p) if !p.is_empty() => format!("writing to {p}"),
            _ => "off (set QUARRY_LOG=/path/to/file)".to_string(),
        },
    )
}

fn tool_check(name: &'static str, found: Option<&'static str>) -> Check {
    match found {
        Some(tool) => Check::ok(name, format!("using {tool}")),
        None => Check::warn(name, "no supported helper found"),
    }
}

fn clipboard_tool() -> Option<&'static str> {
    ["pbcopy", "wl-copy", "xclip"].into_iter().find(on_path)
}

fn browser_tool() -> Option<&'static str> {
    ["open", "xdg-open"].into_iter().find(on_path)
}

fn on_path(tool: &&'static str) -> bool {
    exec::run(
        "sh",
        &["-c", &format!("command -v {tool}")],
        Duration::from_secs(2),
    )
    .map(|o| !o.trim().is_empty())
    .unwrap_or(false)
}

/// Exit code: 0 if nothing fatal is broken.
pub fn report(checks: &[Check]) -> i32 {
    let mut fatal = 0;
    for c in checks {
        let mark = if c.ok {
            "ok  "
        } else if c.fatal {
            "FAIL"
        } else {
            "warn"
        };
        println!("{mark}  {:<11} {}", c.name, c.detail);
        if !c.ok && c.fatal {
            fatal += 1;
        }
    }
    println!();
    if fatal == 0 {
        println!("quarry can see this machine.");
        0
    } else {
        println!("{fatal} fatal problem(s) — quarry cannot see this machine.");
        1
    }
}
