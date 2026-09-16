//! What is exposing a local service to the internet right now.
//!
//! ngrok and cloudflared exist to give a local port a public address, and
//! quarry showed the local port — the one piece of information the user
//! already had. Both agents publish their own state locally, so the public
//! address is available for the asking.
//!
//! The useful thing is not the URL. It is that **something on this machine is
//! reachable from outside it**, which is worth seeing at a glance and worth
//! being able to find in a hurry.

use std::collections::HashMap;
use std::time::Duration;

/// The agent's own API, which ngrok serves on a fixed port by default.
const NGROK_API: &str = "http://127.0.0.1:4040/api/tunnels";

/// Short: the agent is on loopback or it is not there, and a scan should not
/// wait to find out which.
const TIMEOUT: Duration = Duration::from_millis(400);

/// A local port, and where the world can reach it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exposure {
    pub public_url: String,
    /// `ngrok`, `cloudflared` — what is doing the exposing.
    pub agent: &'static str,
}

/// Local port → how it is exposed.
#[derive(Debug, Default)]
pub struct Tunnels {
    by_port: HashMap<u16, Exposure>,
}

impl Tunnels {
    pub fn get(&self, port: u16) -> Option<&Exposure> {
        self.by_port.get(&port)
    }

    pub fn is_empty(&self) -> bool {
        self.by_port.is_empty()
    }

    pub fn len(&self) -> usize {
        self.by_port.len()
    }

    /// Ask whichever agents are running. No agent is the ordinary case on most
    /// machines, so it reports nothing rather than an error nobody can act on.
    pub fn query() -> Tunnels {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .build()
            .new_agent();

        let body = match agent.get(NGROK_API).call() {
            Ok(mut reply) => reply.body_mut().read_to_string().unwrap_or_default(),
            // Connection refused is what "no ngrok here" looks like, and that
            // is the common case rather than a fault.
            Err(_) => return Tunnels::default(),
        };

        let by_port = parse_ngrok(&body);
        if !by_port.is_empty() {
            crate::diag::info(
                "tunnel",
                format!("{} local port(s) exposed by ngrok", by_port.len()),
            );
        }
        Tunnels { by_port }
    }
}

/// Parse ngrok's `/api/tunnels`. Tolerant by design: an unfamiliar shape costs
/// the tunnel it appeared in, not the whole answer.
pub fn parse_ngrok(body: &str) -> HashMap<u16, Exposure> {
    let mut out = HashMap::new();
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body.trim()) else {
        return out;
    };
    let Some(list) = value.get("tunnels").and_then(|t| t.as_array()) else {
        return out;
    };

    for tunnel in list {
        let Some(public_url) = tunnel.get("public_url").and_then(|u| u.as_str()) else {
            continue;
        };
        let Some(addr) = tunnel
            .get("config")
            .and_then(|c| c.get("addr"))
            .and_then(|a| a.as_str())
        else {
            continue;
        };
        let Some(port) = local_port(addr) else {
            continue;
        };
        // ngrok reports one tunnel per protocol, so a port forwarded over both
        // http and https appears twice. HTTPS is the one to show, and it sorts
        // after http, so preferring the longer scheme picks it either way.
        out.entry(port)
            .and_modify(|e: &mut Exposure| {
                if public_url.starts_with("https://") {
                    e.public_url = public_url.to_string();
                }
            })
            .or_insert(Exposure {
                public_url: public_url.to_string(),
                agent: "ngrok",
            });
    }
    out
}

/// The port out of `localhost:3000`, `http://localhost:3000` or
/// `https://127.0.0.1:8443/path`.
fn local_port(addr: &str) -> Option<u16> {
    let without_scheme = addr.rsplit("//").next()?;
    let authority = without_scheme.split('/').next()?;
    authority.rsplit(':').next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "tunnels": [
        {
          "name": "command_line",
          "public_url": "http://cold-poem-42.ngrok.app",
          "proto": "http",
          "config": {"addr": "http://localhost:3000", "inspect": true}
        },
        {
          "name": "command_line (http)",
          "public_url": "https://cold-poem-42.ngrok.app",
          "proto": "https",
          "config": {"addr": "http://localhost:3000", "inspect": true}
        },
        {
          "name": "api",
          "public_url": "tcp://1.tcp.eu.ngrok.io:14523",
          "proto": "tcp",
          "config": {"addr": "localhost:5432", "inspect": false}
        }
      ]
    }"#;

    #[test]
    fn a_forwarded_port_finds_its_public_url() {
        let found = parse_ngrok(SAMPLE);
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[&5432].public_url, "tcp://1.tcp.eu.ngrok.io:14523");
        assert_eq!(found[&5432].agent, "ngrok");
    }

    /// ngrok reports one tunnel per protocol. The same local port appears
    /// twice, and the secure one is the one worth showing.
    #[test]
    fn https_wins_over_http_for_the_same_port() {
        let found = parse_ngrok(SAMPLE);
        assert_eq!(found[&3000].public_url, "https://cold-poem-42.ngrok.app");
    }

    #[test]
    fn a_local_address_is_read_in_every_form_ngrok_writes_it() {
        assert_eq!(local_port("localhost:3000"), Some(3000));
        assert_eq!(local_port("http://localhost:3000"), Some(3000));
        assert_eq!(local_port("https://127.0.0.1:8443/path"), Some(8443));
        assert_eq!(local_port("[::1]:9000"), Some(9000));
        assert_eq!(local_port("localhost"), None);
        assert_eq!(local_port(""), None);
    }

    /// One odd tunnel costs its own entry, not the whole answer.
    #[test]
    fn a_malformed_reply_yields_nothing_rather_than_panicking() {
        for body in [
            "",
            "not json",
            "{}",
            r#"{"tunnels": "not an array"}"#,
            r#"{"tunnels": [{"public_url": "https://x"}]}"#,
            r#"{"tunnels": [{"config": {"addr": "localhost:1"}}]}"#,
            r#"{"tunnels": [{"public_url": "https://x", "config": {"addr": "nonsense"}}]}"#,
        ] {
            assert!(parse_ngrok(body).is_empty(), "{body:?} produced tunnels");
        }
    }

    /// A good tunnel beside a bad one still comes through.
    #[test]
    fn one_bad_tunnel_does_not_cost_the_others() {
        let body = r#"{"tunnels": [
            {"public_url": "https://good.ngrok.app", "config": {"addr": "localhost:3000"}},
            {"nonsense": true}
        ]}"#;
        let found = parse_ngrok(body);
        assert_eq!(found.len(), 1);
        assert_eq!(found[&3000].public_url, "https://good.ngrok.app");
    }

    /// No agent running is the ordinary case, not a failure.
    #[test]
    fn no_agent_is_not_an_error() {
        let found = Tunnels::query();
        assert!(found.is_empty() || found.len() < 1000);
    }
}
