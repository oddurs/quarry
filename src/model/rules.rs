//! Classification rules a user has added, which outrank every guess quarry
//! makes about their own machine.

use super::Kind;

/// Classification rules a user has added, merged over the built-in tables.
///
/// The built-in guesses are good about common software and wrong about anybody's
/// private service on port 9174. These let that be fixed in a config file
/// instead of a pull request, and user rules win: a rule that never overrides
/// anything is not a rule.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rules {
    ports: std::collections::BTreeMap<u16, Kind>,
    names: Vec<(String, Kind)>,
}

impl Rules {
    /// Build from the config, returning what could not be understood so the
    /// caller can report it. A bad rule is skipped, never fatal.
    pub fn from_config(
        ports: &std::collections::BTreeMap<String, String>,
        names: &std::collections::BTreeMap<String, String>,
    ) -> (Rules, Vec<String>) {
        let mut rules = Rules::default();
        let mut problems = Vec::new();

        for (port, kind) in ports {
            match (port.trim().parse::<u16>(), Kind::from_label(kind)) {
                (Ok(p), Some(k)) if p != 0 => {
                    rules.ports.insert(p, k);
                }
                (Err(_), _) => problems.push(format!("[ports] {port:?} is not a port number")),
                (_, None) => problems.push(format!("[ports] {port} = {kind:?} is not a kind")),
                _ => problems.push(format!("[ports] {port:?} is not a port number")),
            }
        }
        for (pattern, kind) in names {
            match Kind::from_label(kind) {
                Some(k) => rules.names.push((pattern.to_lowercase(), k)),
                None => problems.push(format!("[names] {pattern:?} = {kind:?} is not a kind")),
            }
        }
        // Longest pattern first, so a specific rule beats a broad one.
        rules
            .names
            .sort_by_key(|(p, _)| std::cmp::Reverse(p.chars().filter(|c| *c != '*').count()));

        (rules, problems)
    }

    pub fn is_empty(&self) -> bool {
        self.ports.is_empty() && self.names.is_empty()
    }

    pub fn len(&self) -> usize {
        self.ports.len() + self.names.len()
    }

    /// A user rule, if one matches. Ports are checked before names, matching
    /// the built-in order.
    pub fn classify(&self, command: &str, cmdline: &str, ports: &[u16]) -> Option<Kind> {
        for port in ports {
            if let Some(kind) = self.ports.get(port) {
                return Some(*kind);
            }
        }
        let command = command.to_lowercase();
        let cmdline = cmdline.to_lowercase();
        for (pattern, kind) in &self.names {
            if glob_match(pattern, &command) || glob_match(pattern, &cmdline) {
                return Some(*kind);
            }
        }
        None
    }
}

/// Match a process pattern against a command line.
///
/// A literal pattern must land on a word boundary. Plain substring matching is
/// far too loose for process names, and the failures are not theoretical: on
/// this machine `serve` matched `redis-server`, and `dex` matched `index.ts`,
/// so quarry announced a static file server and an OIDC provider that were not
/// there. Short names are common and argv is full of paths.
///
/// A pattern containing `*` is a glob and means what it says.
pub fn process_match(pattern: &str, text: &str) -> bool {
    if pattern.contains('*') {
        return glob_match(pattern, text);
    }
    if pattern.is_empty() || pattern.len() > text.len() {
        return false;
    }
    let bytes = text.as_bytes();
    let needle = pattern.as_bytes();
    let word = |b: u8| b.is_ascii_alphanumeric();

    let mut from = 0;
    while let Some(at) = text[from..].find(pattern).map(|i| i + from) {
        let before_ok = at == 0 || !word(bytes[at - 1]);
        let after = at + needle.len();
        let after_ok = after >= bytes.len() || !word(bytes[after]);
        if before_ok && after_ok {
            return true;
        }
        from = at + 1;
        if from >= text.len() {
            break;
        }
    }
    false
}

/// `*` matches any run of characters, anywhere. Deliberately not a full glob:
/// argv is messy enough without a regex dialect in the config file.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    if !pattern.contains('*') {
        return text.contains(pattern);
    }
    let parts: Vec<&str> = pattern.split('*').collect();
    let mut rest = text;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        match rest.find(part) {
            Some(at) => {
                // A pattern that does not start with `*` must match at the start.
                if i == 0 && !pattern.starts_with('*') && at != 0 {
                    return false;
                }
                rest = &rest[at + part.len()..];
            }
            None => return false,
        }
    }
    // A pattern that does not end with `*` must reach the end.
    if !pattern.ends_with('*')
        && let Some(last) = parts.last().filter(|p| !p.is_empty())
    {
        return text.ends_with(last);
    }
    true
}

#[cfg(test)]
mod rule_tests {
    use super::*;
    use crate::model::{classify, classify_with};
    use std::collections::BTreeMap;

    fn rules(ports: &[(&str, &str)], names: &[(&str, &str)]) -> (Rules, Vec<String>) {
        let p: BTreeMap<String, String> = ports
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        let n: BTreeMap<String, String> = names
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        Rules::from_config(&p, &n)
    }

    #[test]
    fn a_user_port_rule_wins_over_the_built_in_table() {
        // 3000 is a web port as far as the built-in table is concerned.
        assert_eq!(classify("node", "", &[3000]), Kind::Web);
        let (r, problems) = rules(&[("3000", "queue")], &[]);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(classify_with(&r, "node", "", &[3000]), Kind::Queue);
    }

    #[test]
    fn a_port_quarry_has_never_heard_of_can_be_named() {
        assert_eq!(classify("mine", "", &[9174]), Kind::Other);
        let (r, _) = rules(&[("9174", "queue")], &[]);
        assert_eq!(classify_with(&r, "mine", "", &[9174]), Kind::Queue);
    }

    #[test]
    fn name_rules_glob() {
        let (r, problems) = rules(&[], &[("*-worker", "queue"), ("my-daemon", "api")]);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(
            classify_with(&r, "billing-worker", "", &[8123]),
            Kind::Queue
        );
        assert_eq!(classify_with(&r, "my-daemon", "", &[8123]), Kind::Api);
        assert_eq!(classify_with(&r, "unrelated", "", &[8123]), Kind::Other);
    }

    #[test]
    fn a_bad_rule_is_reported_and_skipped_rather_than_fatal() {
        let (r, problems) = rules(&[("not-a-port", "web"), ("80", "not-a-kind")], &[]);
        assert!(r.is_empty(), "neither rule should have survived");
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("not a port number")));
        assert!(problems.iter().any(|p| p.contains("not a kind")));
    }

    /// The failures that produced this rule, taken from a real machine.
    #[test]
    fn a_short_process_name_does_not_match_the_middle_of_a_word() {
        assert!(
            !process_match("serve", "redis-server *:6379"),
            "`serve` inside `redis-server` announced a static file server"
        );
        assert!(
            !process_match("dex", "node index.ts"),
            "`dex` inside `index.ts` announced an OIDC provider"
        );
        assert!(!process_match("api", "rapidapi-thing"));

        // And the matches that must keep working.
        assert!(process_match("serve", "node /app/node_modules/.bin/serve"));
        assert!(process_match("serve", "serve -s build"));
        assert!(process_match(
            "postgres",
            "/usr/local/bin/postgres -D /data"
        ));
        assert!(process_match("next-server", "next-server (v16.3.4)"));
        assert!(process_match("redis-server", "redis-server *:6379"));
        assert!(process_match("node", "node index.ts"));
        // A glob still means what it says.
        assert!(process_match("*-worker", "billing-worker"));
    }

    #[test]
    fn globs_match_where_you_would_expect() {
        assert!(glob_match("worker", "billing-worker"));
        assert!(glob_match("*-worker", "billing-worker"));
        assert!(glob_match("billing-*", "billing-worker"));
        assert!(glob_match("*ill*ork*", "billing-worker"));
        assert!(!glob_match("*-worker", "worker-billing"));
        assert!(!glob_match("nope", "billing-worker"));
        assert!(glob_match("*", "anything"));
    }

    #[test]
    fn every_kind_label_round_trips() {
        for kind in Kind::ALL {
            assert_eq!(Kind::from_label(kind.label()), Some(kind), "{kind:?}");
        }
        assert_eq!(Kind::from_label("nonsense"), None);
    }
}
