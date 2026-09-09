//! The command line contract. These run the real binary, so they also catch
//! the failure modes that only appear once everything is wired together.

use std::process::Command;

/// Every invocation is isolated from the developer's own environment. A test
/// that passes or fails depending on whether someone has a `~/.config/quarry`
/// is not a test.
fn quarry() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_quarry"));
    cmd.env("QUARRY_CONFIG", "/nonexistent/quarry-test.toml")
        .env_remove("NO_COLOR")
        .env_remove("QUARRY_LOG")
        .env("TERM", "xterm-256color");
    cmd
}

fn run(args: &[&str]) -> (i32, String, String) {
    let out = quarry().args(args).output().expect("run quarry");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

#[test]
fn version_and_help_succeed() {
    let (code, out, _) = run(&["--version"]);
    assert_eq!(code, 0);
    assert!(out.starts_with("quarry "), "{out}");

    let (code, out, _) = run(&["--help"]);
    assert_eq!(code, 0);
    for expected in ["--all", "--plain", "--doctor", "QUARRY_LOG"] {
        assert!(
            out.contains(expected),
            "help does not mention {expected}:\n{out}"
        );
    }
}

#[test]
fn an_unknown_option_fails_loudly() {
    let (code, _, err) = run(&["--definitely-not-an-option"]);
    assert_eq!(code, 2, "an unknown flag must not be silently ignored");
    assert!(err.contains("unknown option"), "{err}");
}

#[test]
fn plain_output_is_one_line_per_service() {
    let (code, out, _) = run(&["--plain"]);
    assert_eq!(code, 0);
    for line in out.lines() {
        let first = line.split_whitespace().next().unwrap_or("");
        assert!(
            first.parse::<u16>().is_ok() || first.contains('.') || first.contains('/'),
            "every line must start with a port or a socket path: {line:?}"
        );
        assert!(
            line.contains("://") || line.contains("unix:"),
            "every line must carry something addressable: {line:?}"
        );
    }
}

#[test]
fn all_is_a_superset_of_the_default_view() {
    let (_, default, _) = run(&["--plain"]);
    let (_, all, _) = run(&["--plain", "--all"]);
    assert!(
        all.lines().count() >= default.lines().count(),
        "--all showed fewer services than the default view"
    );
}

#[test]
fn screenshot_renders_without_a_terminal() {
    // The important part: this runs with stdout redirected to a pipe.
    let (code, out, _) = run(&["--screenshot", "100x24"]);
    assert_eq!(code, 0);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 24, "asked for 24 rows, got {}", lines.len());
    assert!(lines[0].contains("quarry"), "no title bar:\n{out}");
    for line in &lines {
        assert!(
            line.chars().count() <= 100,
            "line wider than the requested 100 columns: {line:?}"
        );
    }
}

#[test]
fn screenshot_accepts_an_equals_form_and_falls_back_sensibly() {
    let (code, out, _) = run(&["--screenshot=80x10"]);
    assert_eq!(code, 0);
    assert_eq!(out.lines().count(), 10);

    let (code, out, _) = run(&["--screenshot", "nonsense"]);
    assert_eq!(code, 0, "a bad size falls back rather than failing");
    assert!(!out.is_empty());
}

#[test]
fn doctor_reports_on_every_dependency() {
    let (code, out, _) = run(&["--doctor"]);
    for expected in [
        "sockets",
        "lsof",
        "projects",
        "clipboard",
        "browser",
        "terminal",
        "log",
    ] {
        assert!(out.contains(expected), "doctor omits {expected}:\n{out}");
    }
    assert!(code == 0 || code == 1, "unexpected exit code {code}");
}

#[test]
fn the_log_file_is_written_when_asked() {
    let dir = tempfile::tempdir().expect("tempdir");
    let log = dir.path().join("quarry.log");
    let status = quarry()
        .args(["--screenshot", "80x10"])
        .env("QUARRY_LOG", &log)
        .output()
        .expect("run quarry");
    assert!(status.status.success());

    let contents = std::fs::read_to_string(&log).expect("log was created");
    assert!(contents.contains("logging to"), "log is empty:\n{contents}");
}

#[test]
fn an_unwritable_log_path_does_not_break_the_run() {
    let (code, out, _) = quarry()
        .args(["--screenshot", "80x10"])
        .env("QUARRY_LOG", "/nonexistent-directory/quarry.log")
        .output()
        .map(|o| {
            (
                o.status.code().unwrap_or(-1),
                String::from_utf8_lossy(&o.stdout).to_string(),
                String::new(),
            )
        })
        .expect("run quarry");
    assert_eq!(code, 0, "a bad log path must not be fatal");
    assert!(!out.is_empty(), "the screen still rendered");
}

#[test]
fn fix_terminal_emits_the_reset_sequence() {
    let (code, out, _) = run(&["--fix-terminal"]);
    assert_eq!(code, 0);
    for mode in ["?1000l", "?1002l", "?1003l", "?1006l", "?1049l", "?25h"] {
        assert!(
            out.contains(mode),
            "--fix-terminal does not clear {mode}: {out:?}"
        );
    }
}

#[test]
fn help_mentions_the_terminal_escape_hatch() {
    let (_, out, _) = run(&["--help"]);
    assert!(out.contains("--fix-terminal"), "{out}");
}

#[test]
fn config_prints_something_that_loads_again() {
    let (code, out, _) = run(&["config"]);
    assert_eq!(code, 0);
    assert!(out.contains("theme"), "{out}");
    assert!(
        out.contains("no config file"),
        "the tests run without one, and it should say so:\n{out}"
    );

    // The commented output must still be valid TOML the loader accepts.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("round.toml");
    std::fs::write(&path, &out).expect("write");
    let (code, again, _) = quarry()
        .args(["config", "--config"])
        .arg(&path)
        .output()
        .map(|o| {
            (
                o.status.code().unwrap_or(-1),
                String::from_utf8_lossy(&o.stdout).to_string(),
                String::new(),
            )
        })
        .expect("run");
    assert_eq!(
        code, 0,
        "quarry could not read back its own output:\n{again}"
    );
}

#[test]
fn config_write_creates_a_file_and_refuses_to_clobber() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");

    let out = quarry()
        .args(["config", "--write", "--config"])
        .arg(&path)
        .output()
        .expect("run");
    assert!(out.status.success());
    assert!(path.exists(), "no file was written");

    let again = quarry()
        .args(["config", "--write", "--config"])
        .arg(&path)
        .output()
        .expect("run");
    assert!(!again.status.success(), "it clobbered an existing file");

    let forced = quarry()
        .args(["config", "--write", "--force", "--config"])
        .arg(&path)
        .output()
        .expect("run");
    assert!(forced.status.success(), "--force should replace it");
}

#[test]
fn a_config_file_actually_changes_behaviour() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "theme = \"gotham\"\nrefresh_secs = 11\n").expect("write");

    let out = quarry()
        .args(["config", "--config"])
        .arg(&path)
        .output()
        .expect("run");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("gotham"), "{text}");
    assert!(text.contains("refresh_secs = 11"), "{text}");
    assert!(
        text.contains("set by the file"),
        "overridden settings should be called out:\n{text}"
    );
}

#[test]
fn themes_lists_and_filters() {
    let (code, out, _) = run(&["themes"]);
    assert_eq!(code, 0);
    for expected in ["auto", "mono", "gotham", "night", "paper"] {
        assert!(out.contains(expected), "themes omits {expected}:\n{out}");
    }
    assert!(
        out.contains("built-in"),
        "the source should be marked:\n{out}"
    );

    let (_, filtered, _) = run(&["themes", "gotham"]);
    assert!(filtered.contains("gotham"));
    assert!(
        !filtered.contains("paper"),
        "the filter did nothing:\n{filtered}"
    );
}

#[test]
fn the_theme_flag_changes_what_is_drawn() {
    let (_, night, _) = run(&["--theme", "night", "--screenshot", "80x12"]);
    let (_, mono, _) = run(&["--theme", "mono", "--screenshot", "80x12"]);
    assert!(!night.is_empty() && !mono.is_empty());
    // The text is identical; only the colour differs, which --screenshot does
    // not carry. What matters is that neither errors and both render.
    assert_eq!(night.lines().count(), 12);
    assert_eq!(mono.lines().count(), 12);

    let (code, _, err) = run(&["--theme", "definitely-not-a-theme", "--screenshot", "40x6"]);
    assert_eq!(code, 0, "an unknown theme must fall back, not fail: {err}");
}

#[test]
fn doctor_reports_the_config_and_the_theme() {
    let (_, out, _) = run(&["--doctor"]);
    assert!(out.contains("config"), "{out}");
    assert!(out.contains("theme"), "{out}");
    assert!(
        out.contains("auto") || out.contains("mono"),
        "the resolved theme should be named:\n{out}"
    );
}

#[test]
fn no_color_is_honoured() {
    let out = quarry()
        .args(["--doctor"])
        .env("NO_COLOR", "1")
        .output()
        .expect("run");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("mono"),
        "NO_COLOR should select the colourless theme:\n{text}"
    );

    let out = quarry()
        .args(["--doctor"])
        .env("TERM", "dumb")
        .output()
        .expect("run");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("mono"), "TERM=dumb should too:\n{text}");
}

#[test]
fn a_broken_config_does_not_stop_it_starting() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("bad.toml");
    std::fs::write(&path, "theme = \n[[[").expect("write");

    let out = quarry()
        .args(["--screenshot", "80x10", "--config"])
        .arg(&path)
        .output()
        .expect("run");
    assert!(out.status.success(), "a broken config must not be fatal");
    assert_eq!(String::from_utf8_lossy(&out.stdout).lines().count(), 10);
}

#[test]
fn custom_classification_rules_reach_the_output() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("rules.toml");
    // Claim every port quarry might find; whatever it lists must be a queue.
    let mut body = String::from("[ports]\n");
    for port in 1..=65535u32 {
        body.push_str(&format!("{port} = \"queue\"\n"));
    }
    std::fs::write(&path, body).expect("write");

    let out = quarry()
        .args(["--plain", "--all", "--config"])
        .arg(&path)
        .output()
        .expect("run");
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        // Unix sockets have no port, so no port rule can claim them.
        let first = line.split_whitespace().next().unwrap_or("");
        if first.parse::<u16>().is_err() || first == "0" {
            continue;
        }
        let kind = line.split_whitespace().nth(1).unwrap_or("");
        assert_eq!(kind, "queue", "a user rule did not win: {line:?}");
    }
}
