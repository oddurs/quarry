//! The binary: argument handling, and the event loop that owns the terminal.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyEventKind};

use quarry::app::{Action, App, ToastKind};
use quarry::config::Config;
use quarry::engine::Engine;
use quarry::keys::Keymap;
use quarry::model::Rules;
use quarry::probe::{NetProber, Pool, Target};
use quarry::runtime::{self, Msg, Settings};
use quarry::term::{self, Guard, Tui};
use quarry::theme::{self, Theme};
use quarry::{diag, doctor, model, ui};

const TICK: Duration = Duration::from_millis(100);

/// Everything the command line and the config file resolve to between them.
struct Startup {
    config: Config,
    config_path: Option<PathBuf>,
    theme: Theme,
    keymap: Keymap,
    rules: Rules,
    signatures: quarry::signature::Registry,
    show_all: bool,
}

fn main() -> Result<()> {
    diag::init_from_env();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |names: &[&str]| args.iter().any(|a| names.contains(&a.as_str()));

    if has(&["-h", "--help"]) {
        print_usage();
        return Ok(());
    }
    if has(&["-V", "--version"]) {
        println!("quarry {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if has(&["--fix-terminal"]) {
        term::print_reset();
        println!("terminal reset");
        return Ok(());
    }

    // Subcommands come before flag validation so `quarry config --write` works.
    match args.first().map(String::as_str) {
        Some("config") => return cmd_config(&args),
        Some("themes") => return cmd_themes(&args),
        Some("signatures") => return cmd_signatures(&args),
        Some("why") => return cmd_why(&args),
        _ => {}
    }

    if has(&["--doctor"]) {
        let startup = resolve(&args);
        std::process::exit(doctor::report(&doctor::run(
            &startup.config,
            startup.config_path.as_deref(),
            &startup.theme,
        )));
    }

    let known_flags = [
        "-a",
        "--all",
        "-p",
        "--plain",
        "--doctor",
        "--fix-terminal",
        "--no-color",
        "--demo",
        "-h",
        "--help",
        "-V",
        "--version",
    ];
    let valued = ["--theme", "--config", "--screenshot", "--color", "--format"];
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a.starts_with('-') {
            let base = a.split('=').next().unwrap_or(a);
            if valued.contains(&base) {
                if !a.contains('=') {
                    i += 1;
                }
            } else if !known_flags.contains(&a.as_str()) {
                eprintln!("quarry: unknown option {a}\n");
                print_usage();
                std::process::exit(2);
            }
        }
        i += 1;
    }

    let startup = resolve(&args);

    if let Some(spec) = flag_value(&args, "--screenshot") {
        return screenshot(&spec, startup);
    }
    if has(&["-p", "--plain"]) {
        return plain(startup);
    }
    run_tui(startup)
}

fn print_usage() {
    println!(
        "quarry — see every server running on this machine\n\n\
         USAGE:\n  quarry [options]\n  quarry config [--write] [--force]\n\
         \x20 quarry themes [filter]\n  quarry signatures [filter]\n  quarry why <port>\n\n\
         OPTIONS:\n\
         \x20 -a, --all             include system services\n\
         \x20 -p, --plain           print one line per service and exit\n\
         \x20     --theme <NAME>    use a theme for this run (auto, mono, gotham, …)\n\
         \x20     --config <PATH>   read this config file instead of the usual one\n\
         \x20     --color <WHEN>    always, never, or auto\n\
         \x20     --no-color        same as --color never\n\
         \x20     --doctor          check everything quarry depends on\n\
         \x20     --screenshot WxH  render one frame as text and exit\n\
         \x20     --format html     with --screenshot, emit markup instead of text\n\
         \x20     --demo            with --screenshot, render a synthetic machine\n\
         \x20     --fix-terminal    undo a terminal left in mouse-reporting mode\n\
         \x20 -h, --help            show this help\n\
         \x20 -V, --version         show the version\n\n\
         ENVIRONMENT:\n\
         \x20 QUARRY_CONFIG  config file to read\n\
         \x20 QUARRY_LOG     append diagnostics to this file\n\
         \x20 NO_COLOR       render without colour\n"
    );
}

/// Resolve the config, then let the command line override it. Flags win over
/// the file, and the file wins over the defaults.
fn resolve(args: &[String]) -> Startup {
    let explicit = flag_value(args, "--config").map(PathBuf::from);
    let (config, config_path) = Config::load(explicit.as_deref());

    let colour = colour_choice(args);
    let spec = flag_value(args, "--theme").unwrap_or_else(|| config.theme.clone());
    let theme = match colour {
        Colour::Never => Theme::mono(),
        _ => {
            let (theme, err) = Theme::resolve_or_default(&spec);
            if let Some(e) = err {
                diag::warn("theme", e.to_string());
            }
            theme
        }
    };

    let (keymap, key_problems) = Keymap::from_config(&config.keys);
    for p in key_problems {
        diag::warn("keys", p);
    }
    let (rules, rule_problems) = Rules::from_config(&config.ports, &config.names);
    for p in rule_problems {
        diag::warn("rules", p);
    }

    let (signatures, signature_problems) = quarry::signature::Registry::load();
    for p in signature_problems {
        diag::warn("signatures", p);
    }

    let show_all = config.show_all || args.iter().any(|a| a == "-a" || a == "--all");

    Startup {
        config,
        config_path,
        theme,
        keymap,
        rules,
        signatures,
        show_all,
    }
}

enum Colour {
    Auto,
    Never,
}

/// `--color`, `--no-color`, `NO_COLOR`, and a terminal that cannot show any.
fn colour_choice(args: &[String]) -> Colour {
    if let Some(when) = flag_value(args, "--color") {
        return match when.as_str() {
            "never" | "no" | "off" => Colour::Never,
            _ => Colour::Auto,
        };
    }
    if args.iter().any(|a| a == "--no-color") {
        return Colour::Never;
    }
    if std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()) {
        return Colour::Never;
    }
    match std::env::var("TERM").as_deref() {
        Ok("dumb") | Ok("") => Colour::Never,
        _ => Colour::Auto,
    }
}

fn engine_for(startup: &Startup) -> Engine {
    let (signatures, _) = quarry::signature::Registry::load();
    Engine::live()
        .with_rules(startup.rules.clone())
        .with_signatures(signatures)
}

fn cmd_config(args: &[String]) -> Result<()> {
    let path = flag_value(args, "--config")
        .map(PathBuf::from)
        .unwrap_or_else(Config::path);

    if args.iter().any(|a| a == "--write") {
        let force = args.iter().any(|a| a == "--force");
        match Config::write_default(&path, force) {
            Ok(()) => println!("wrote {}", path.display()),
            Err(e) => {
                eprintln!("quarry: {e}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }

    let startup = resolve(args);
    match &startup.config_path {
        Some(p) => println!("# from {}", p.display()),
        None => {
            println!("# no config file; showing defaults\n# write one with: quarry config --write")
        }
    }
    let overridden = startup.config.overridden();
    if !overridden.is_empty() {
        println!("# set by the file: {}", overridden.join(", "));
    }
    println!(
        "# theme resolves to {} ({})",
        startup.theme.name,
        startup.theme.source.label()
    );
    println!();
    print!("{}", startup.config.to_toml());
    Ok(())
}

/// `quarry themes [filter]`. There are several hundred Ghostty themes on a
/// typical machine, so a filter is not a luxury.
fn cmd_themes(args: &[String]) -> Result<()> {
    let filter = args
        .iter()
        .skip(1)
        .find(|a| !a.starts_with('-'))
        .map(|s| s.to_lowercase());
    let startup = resolve(args);
    let active = startup.theme.name.to_lowercase();

    let mut shown = 0;
    for (name, source) in theme::available() {
        if let Some(f) = &filter
            && !name.to_lowercase().contains(f)
        {
            continue;
        }
        let mark = if name.to_lowercase() == active {
            "*"
        } else {
            " "
        };
        let status = match Theme::resolve(&name) {
            Ok(_) => String::new(),
            Err(e) => format!("  ({e})"),
        };
        println!("{mark} {:<30} {}{}", name, source.label(), status);
        shown += 1;
    }
    if shown == 0 {
        println!("no theme matches {}", filter.unwrap_or_default());
    }
    Ok(())
}

/// `quarry signatures [filter]` — what quarry knows, and where it came from.
fn cmd_signatures(args: &[String]) -> Result<()> {
    let (registry, problems) = quarry::signature::Registry::load();
    for p in &problems {
        eprintln!("quarry: {p}");
    }
    let filter = args
        .iter()
        .skip(1)
        .find(|a| !a.starts_with('-'))
        .map(|s| s.to_lowercase());

    let mut shown = 0;
    for sig in registry.iter() {
        if let Some(f) = &filter
            && !sig.name.to_lowercase().contains(f)
            && !sig.kind.label().contains(f.as_str())
        {
            continue;
        }
        let evidence = [
            (!sig.ports.is_empty()).then(|| {
                format!(
                    "ports {}",
                    sig.ports
                        .iter()
                        .map(u16::to_string)
                        .collect::<Vec<_>>()
                        .join(",")
                )
            }),
            (!sig.process.is_empty()).then(|| format!("process {}", sig.process.join(","))),
            sig.banner.as_ref().map(|_| "banner".to_string()),
            sig.http_title.as_ref().map(|t| format!("title {t}")),
            sig.http_server.as_ref().map(|t| format!("server {t}")),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        let origin = match sig.origin {
            quarry::signature::Origin::User => "user",
            quarry::signature::Origin::Builtin => "built-in",
        };
        println!(
            "{:<30} {:<10} {:<9} {}",
            sig.name,
            sig.kind.label(),
            origin,
            evidence
        );
        shown += 1;
    }
    if shown == 0 {
        println!("nothing matches {}", filter.unwrap_or_default());
    } else if filter.is_none() {
        println!(
            "\n{shown} signatures. Add your own in {}",
            quarry::signature::user_path().display()
        );
    }
    Ok(())
}

/// `quarry why <port>` — the evidence behind one verdict.
///
/// A classifier nobody can interrogate is one people argue with rather than
/// fix. This prints what matched, what it scored, and what it lost to.
fn cmd_why(args: &[String]) -> Result<()> {
    let Some(target) = args.iter().skip(1).find(|a| !a.starts_with('-')) else {
        eprintln!("quarry: which port? try `quarry why 3000`");
        std::process::exit(2);
    };
    let startup = resolve(args);
    let mut engine = engine_for(&startup);
    let report = match engine.scan() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("quarry: {e}");
            std::process::exit(1);
        }
    };

    let wanted = target.parse::<u16>().ok();
    let found: Vec<_> = report
        .servers
        .iter()
        .filter(|s| match wanted {
            Some(port) => s.listeners.iter().any(|l| l.port == port),
            None => s.primary_label().contains(target.as_str()),
        })
        .collect();

    if found.is_empty() {
        println!("nothing is listening on {target}");
        return Ok(());
    }

    let (registry, _) = quarry::signature::Registry::load();
    for s in found {
        let ports: Vec<u16> = s
            .listeners
            .iter()
            .map(|l| l.port)
            .filter(|p| *p != 0)
            .collect();
        println!("{} — pid {}", s.primary_label(), s.pid);
        println!("  process   {}", s.cmdline);
        println!("  verdict   {} ({})", s.service_name(), s.kind.label());
        println!();

        let ranked = registry.explain(&quarry::signature::Evidence {
            command: &s.command,
            cmdline: &s.cmdline,
            ports: &ports,
            banner: s.banner.as_deref(),
            ..Default::default()
        });
        if ranked.is_empty() {
            println!("  nothing in the signature table matched.");
            println!("  the kind came from the port conventions in src/model.rs.");
        }
        for (i, v) in ranked.iter().take(8).enumerate() {
            let mark = if i == 0 { "→" } else { " " };
            let named = if v.names_the_service() {
                ""
            } else {
                "  (port only — not enough to name it)"
            };
            println!(
                "  {mark} {:<28} {:>4}  {}{named}",
                v.name,
                v.score,
                v.reasons.join(", ")
            );
        }
        println!();
    }
    Ok(())
}

/// A scriptable one-shot: scan, give the probes a moment, print a table.
fn plain(startup: Startup) -> Result<()> {
    let mut engine = engine_for(&startup);
    let report = match engine.scan() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("quarry: {e}");
            std::process::exit(1);
        }
    };
    for w in &report.warnings {
        eprintln!("quarry: {w}");
    }

    let mut servers = report.servers;
    let outcomes = probe_all(&startup.config, &servers);
    for o in outcomes {
        for s in servers.iter_mut().filter(|s| s.pid == o.pid) {
            s.kind = model::refine_kind(s.kind, &o.health);
            s.health = o.health.clone();
        }
    }

    servers.retain(|s| startup.show_all || (!s.kind.is_background_noise() && !s.is_socket_only()));
    servers.sort_by_key(|s| (s.group_key(), s.primary_port()));

    for s in &servers {
        println!(
            "{:<9}  {:<9}  {:<22}  {:<22}  {:<26}  {}",
            ellipsis(&s.primary_label(), 9),
            s.kind.label(),
            ellipsis(&s.title(), 22),
            ellipsis(&s.service_name(), 22),
            ellipsis(&s.health.summary(), 26),
            s.url()
        );
    }
    Ok(())
}

/// Probe every service once and wait, briefly, for the answers.
fn probe_all(config: &Config, servers: &[quarry::model::Server]) -> Vec<quarry::probe::Outcome> {
    let pool = Pool::with_workers(
        Arc::new(NetProber::from_config(config)),
        config.probe_workers,
    );
    let mut expected = 0;
    for s in servers {
        if let Some(l) = s.listeners.first() {
            let mut target = Target::from_listener(s.pid, l, s.kind);
            target.path = s
                .health_path
                .clone()
                .unwrap_or_else(|| config.health_path(l.port).to_string());
            target.handshake = s.handshake.clone();
            if pool.submit(target) {
                expected += 1;
            }
        }
    }
    pool.collect(expected, Duration::from_millis(2500))
}

fn ellipsis(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n.saturating_sub(1)).chain(['…']).collect()
    }
}

/// `--screenshot 120x40`. Renders one frame with no terminal at all, which is
/// what makes a bug report reproducible.
fn screenshot(spec: &str, startup: Startup) -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let html = flag_value(&args, "--format").as_deref() == Some("html");
    let demo = args.iter().any(|a| a == "--demo");

    let (w, h) = spec
        .split_once(['x', 'X'])
        .and_then(|(a, b)| Some((a.trim().parse().ok()?, b.trim().parse().ok()?)))
        .unwrap_or((120u16, 40u16));

    // Built before the app takes ownership of the pieces it needs.
    let engine = (!demo).then(|| engine_for(&startup));
    let config = startup.config.clone();

    let mut app = App::new();
    app.show_all = startup.show_all;
    app.theme = startup.theme;
    app.keymap = startup.keymap;
    app.rules = startup.rules.clone();
    app.signatures = std::sync::Arc::new(startup.signatures);

    if demo {
        // A synthetic machine. Rendering a real scan into a published page
        // would publish the name of whatever the author is working on.
        app.ingest(quarry::testkit::demo());
        app.now = 1_700_000_000 + 4 * 60 * 60;
    } else {
        let mut engine = engine.expect("built above when not in demo mode");
        let report = engine.scan().unwrap_or_else(|e| {
            eprintln!("quarry: {e}");
            std::process::exit(1);
        });
        let outcomes = probe_all(&config, &report.servers);
        app.ingest(report.servers);
        for o in outcomes {
            app.apply_health(o.pid, o.port, o.health, o.banner);
        }
    }
    app.scanning = false;

    if html {
        println!("{}", ui::render_html(&mut app, w, h, 0));
    } else {
        println!("{}", ui::render_to_string(&mut app, w, h, 0));
    }
    Ok(())
}

fn flag_value(args: &[String], name: &str) -> Option<String> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == name {
            return it.next().cloned();
        }
        if let Some(v) = a.strip_prefix(&format!("{name}=")) {
            return Some(v.to_string());
        }
    }
    None
}

fn run_tui(mut startup: Startup) -> Result<()> {
    let engine = engine_for(&startup);
    let prober = Arc::new(NetProber::from_config(&startup.config));
    let settings = Settings::from_config(&startup.config);

    let (mut guard, mut terminal) = Guard::new()?;

    // Asked once, in raw mode, before anything else reads stdin. `auto` is the
    // only theme whose choices depend on the answer.
    if startup.theme.source == quarry::theme::Source::Auto && startup.theme.name == "auto" {
        let started = std::time::Instant::now();
        let dark = term::background_is_dark(Duration::from_millis(120));
        diag::info(
            "theme",
            format!(
                "terminal background looks {} (asked in {}ms)",
                if dark { "dark" } else { "light" },
                started.elapsed().as_millis()
            ),
        );
        startup.theme = Theme::auto(dark);
    }

    let mut app = App::new();
    app.show_all = startup.show_all;
    app.theme = startup.theme;
    app.keymap = startup.keymap;
    app.rules = startup.rules.clone();
    app.signatures = std::sync::Arc::new(startup.signatures);

    let (handle, msgs) = runtime::spawn(engine, prober, settings);

    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = event_loop(&mut terminal, &mut app, &msgs, &handle, &mut guard, &args);
    drop(handle);
    guard.restore();
    result
}

fn event_loop(
    terminal: &mut Tui,
    app: &mut App,
    msgs: &std::sync::mpsc::Receiver<Msg>,
    handle: &runtime::Handle,
    guard: &mut Guard,
    args: &[String],
) -> Result<()> {
    let mut tick = 0usize;
    let mut dirty = true;
    let mut last_draw = std::time::Instant::now();
    let mut shown_second = 0u64;

    loop {
        for msg in msgs.try_iter() {
            match msg {
                Msg::Scanning => app.scanning = true,
                Msg::Servers(list) => app.ingest(list),
                Msg::Health {
                    pid,
                    port,
                    health,
                    banner,
                } => app.apply_health(pid, port, health, banner),
                Msg::ScanFailed { detail, transient } => app.scan_failed(detail, transient),
                Msg::Warning(w) => app.warn(w),
            }
            dirty = true;
        }
        let had_toast = app.toast.is_some();
        app.tick_clock();
        app.expire_toast();
        let alive = handle.is_alive();
        if alive != app.scanner_alive {
            app.scanner_alive = alive;
            dirty = true;
        }
        // The parts of the screen that move on their own: the scan spinner, the
        // "updated N ago" clock, and a toast that has just gone.
        if app.scanning || app.now != shown_second || had_toast != app.toast.is_some() {
            shown_second = app.now;
            dirty = true;
        }

        // Both checks belong before the draw and the read. A closed terminal
        // makes crossterm's read spin forever, so the loop must never enter it
        // once the far end has gone.
        if term::terminating() || term::input_closed() {
            return Ok(());
        }

        // Idle, quarry redraws once a second rather than ten times. The
        // once-a-second floor bounds how stale the screen can get if something
        // changes without setting the flag.
        if dirty || last_draw.elapsed() >= Duration::from_secs(1) {
            terminal.draw(|f| ui::draw(f, app, tick))?;
            last_draw = std::time::Instant::now();
            dirty = false;
        }

        if event::poll(TICK)? {
            let action = match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    app.handle_key(key.code, key.modifiers)
                }
                Event::Mouse(m) => app.handle_mouse(m),
                _ => Action::None,
            };
            dirty = true;
            if dispatch(action, app, handle, guard, terminal, args)? {
                return Ok(());
            }
        }
        tick = tick.wrapping_add(1);
    }
}

/// The only place in the program that touches the outside world on the user's
/// behalf. `App` decides *what* should happen; this decides *how*, and reports
/// the outcome back as a toast.
fn dispatch(
    action: Action,
    app: &mut App,
    handle: &runtime::Handle,
    guard: &mut Guard,
    terminal: &mut Tui,
    args: &[String],
) -> Result<bool> {
    match action {
        Action::None => {}
        Action::Quit => return Ok(true),
        Action::Refresh => handle.refresh(),
        Action::Reload => {
            let startup = resolve(args);
            let name = startup.theme.name.clone();
            app.theme = startup.theme;
            app.keymap = startup.keymap;
            app.show_all = startup.show_all;
            app.rebuild();
            // Deliberately no `terminal.clear()`. It issues a cursor-position
            // query and waits for a reply, which stalls for seconds on a
            // terminal that does not answer — and it is not needed: ratatui's
            // diff compares styles, so a changed palette repaints itself.
            //
            // Scan and probe settings are read when the runtime starts, so say
            // so rather than implying a reload did more than it did.
            app.toast(format!("reloaded — theme {name}"), ToastKind::Good);
        }
        Action::SetMouse(on) => guard.set_mouse(on)?,
        Action::ClearScreen => terminal.clear()?,
        Action::Open(url) => match open::that_detached(&url) {
            Ok(()) => app.toast(format!("opened {url}"), ToastKind::Good),
            Err(e) => app.toast(format!("could not open: {e}"), ToastKind::Bad),
        },
        Action::Copy(text) => match copy_to_clipboard(&text) {
            Ok(()) => app.toast(format!("copied {text}"), ToastKind::Good),
            Err(e) => app.toast(format!("clipboard unavailable: {e}"), ToastKind::Bad),
        },
        Action::Signal { pid, force } => {
            let sig = if force { "-KILL" } else { "-TERM" };
            let ok = Command::new("kill")
                .args([sig, &pid.to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if ok {
                app.toast(format!("sent {sig} to {pid}"), ToastKind::Good);
                handle.refresh();
            } else {
                app.toast(format!("could not signal {pid}"), ToastKind::Bad);
            }
        }
    }
    Ok(false)
}

fn copy_to_clipboard(text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let candidates: [&[&str]; 3] = [
        &["pbcopy"],
        &["wl-copy"],
        &["xclip", "-selection", "clipboard"],
    ];
    let mut last = std::io::Error::other("no clipboard tool found");
    for cmd in candidates {
        let mut child = match Command::new(cmd[0])
            .args(&cmd[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                last = e;
                continue;
            }
        };
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(text.as_bytes())?;
        }
        child.wait()?;
        return Ok(());
    }
    Err(last)
}
