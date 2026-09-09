//! `quarry --doctor`: check every external thing quarry depends on, and say
//! which of them is broken before the user has to guess from an empty screen.

use std::time::{Duration, Instant};

use crate::config::Config;
use crate::engine::Engine;
use crate::exec;
use crate::source::SocketSource;
use crate::theme::Theme;

pub struct Check {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
    /// False when a failure only costs a feature, not the whole tool.
    pub fatal: bool,
}

pub fn run(config: &Config, config_path: Option<&std::path::Path>, theme: &Theme) -> Vec<Check> {
    // Annotated because the first push is inside a platform-specific block:
    // without this the type is never inferred on some platforms.
    let mut checks: Vec<Check> = Vec::new();

    // Where listening sockets actually come from on this machine, and what
    // that costs. Reported on every platform, naming whichever source is live.
    let started = Instant::now();
    let mut source = crate::engine::Engine::socket_source();
    let name = source.describe();
    match source.listening() {
        Ok(sockets) => checks.push(Check {
            name: "sockets",
            ok: true,
            detail: format!(
                "{} listening in {}ms via {name}",
                sockets.len(),
                started.elapsed().as_millis()
            ),
            fatal: false,
        }),
        Err(e) => checks.push(Check {
            name: "sockets",
            ok: false,
            detail: format!("{name}: {e}"),
            fatal: true,
        }),
    }

    // The fallback, where there is a native path to fall back from.
    #[cfg(target_os = "macos")]
    {
        let started = Instant::now();
        let mut lsof = crate::lsof::Lsof;
        let native_ok = checks.iter().any(|c| c.name == "sockets" && c.ok);
        match lsof.listening() {
            Ok(sockets) => checks.push(Check {
                name: "lsof",
                ok: true,
                detail: format!(
                    "{} listening in {}ms (fallback, not in use)",
                    sockets.len(),
                    started.elapsed().as_millis()
                ),
                fatal: false,
            }),
            Err(e) => checks.push(Check {
                name: "lsof",
                ok: native_ok,
                detail: if native_ok {
                    format!("unavailable ({e}), but not needed")
                } else {
                    e.to_string()
                },
                fatal: !native_ok,
            }),
        }
    }

    // Everything else degrades rather than fails.
    let (rules, rule_problems) = crate::model::Rules::from_config(&config.ports, &config.names);
    let mut engine = Engine::live().with_rules(rules.clone());
    match engine.scan() {
        Ok(report) => {
            let attributed = report.servers.iter().filter(|s| s.repo.is_some()).count();
            let with_cwd = report.servers.iter().filter(|s| s.cwd.is_some()).count();
            checks.push(Check {
                name: "processes",
                ok: with_cwd > 0 || report.servers.is_empty(),
                detail: format!(
                    "{}/{} have a readable working directory",
                    with_cwd,
                    report.servers.len()
                ),
                fatal: false,
            });
            checks.push(Check {
                name: "projects",
                ok: true,
                detail: format!(
                    "{}/{} attributed to a project",
                    attributed,
                    report.servers.len()
                ),
                fatal: false,
            });
        }
        Err(e) => checks.push(Check {
            name: "scan",
            ok: false,
            detail: e.to_string(),
            fatal: true,
        }),
    }

    checks.push(Check {
        name: "config",
        ok: rule_problems.is_empty(),
        detail: match (config_path, rule_problems.len()) {
            (_, n) if n > 0 => format!(
                "{n} rule(s) could not be understood: {}",
                rule_problems.join("; ")
            ),
            (Some(p), _) => {
                let set = config.overridden();
                if set.is_empty() {
                    format!("{} (nothing overridden)", p.display())
                } else {
                    format!("{} — {}", p.display(), set.join(", "))
                }
            }
            (None, _) => "no config file; using defaults".to_string(),
        },
        fatal: false,
    });

    checks.push(Check {
        name: "theme",
        ok: true,
        detail: format!(
            "{} ({}){}",
            theme.name,
            theme.source.label(),
            if rules.is_empty() {
                String::new()
            } else {
                format!(", {} custom rule(s)", rules.len())
            }
        ),
        fatal: false,
    });

    let containers = crate::docker::Containers::query();
    checks.push(Check {
        name: "containers",
        ok: true,
        detail: if containers.is_empty() {
            "no container runtime answering; published ports stay unattributed".to_string()
        } else {
            format!(
                "{} published port(s) attributed to a container",
                containers.len()
            )
        },
        fatal: false,
    });

    checks.push(tool_check("clipboard", clipboard_tool()));
    checks.push(tool_check("browser", browser_tool()));

    checks.push(Check {
        name: "terminal",
        ok: true,
        detail: match crossterm::terminal::size() {
            Ok((w, h)) => {
                let cramped = w < 60 || h < 12;
                format!(
                    "{w}×{h}{}",
                    if cramped {
                        " (cramped; 80×24 or more is better)"
                    } else {
                        ""
                    }
                )
            }
            Err(e) => format!("size unknown: {e}"),
        },
        fatal: false,
    });

    checks.push(Check {
        name: "log",
        ok: true,
        detail: match std::env::var("QUARRY_LOG") {
            Ok(p) if !p.is_empty() => format!("writing to {p}"),
            _ => "off (set QUARRY_LOG=/path/to/file)".to_string(),
        },
        fatal: false,
    });

    checks
}

fn tool_check(name: &'static str, found: Option<&'static str>) -> Check {
    match found {
        Some(tool) => Check {
            name,
            ok: true,
            detail: format!("using {tool}"),
            fatal: false,
        },
        None => Check {
            name,
            ok: false,
            detail: "no supported helper found".to_string(),
            fatal: false,
        },
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
