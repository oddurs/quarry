//! Whether a service answered, and how well.

use std::time::Duration;

#[derive(Clone, Debug, Default)]
pub enum Health {
    #[default]
    Unknown,
    /// Answered an HTTP request.
    Http {
        status: u16,
        scheme: &'static str,
        latency: Duration,
        server: Option<String>,
        title: Option<String>,
        is_html: bool,
    },
    /// Accepted a TCP connection but is not HTTP (or refused to answer one).
    Open { latency: Duration },
    /// Bound, but nothing was tested. A UDP socket cannot be connected to, and
    /// saying "open" would claim a check that did not happen.
    Bound,
    /// Just appeared, and not yet serving what it is expected to serve.
    ///
    /// A dev server binds its port the moment it starts and then spends thirty
    /// seconds compiling. Reporting that as `open` says less than it could, and
    /// reporting it as `not responding` says something false: nothing is wrong,
    /// it is not ready. Only a service seen for the first time within the last
    /// minute qualifies — after that, not answering is not a phase.
    Starting,
    /// The port is gone since the last scan.
    Closed,
}

impl Health {
    /// The glyph alone, so a colour-blind reader and a `NO_COLOR` terminal get
    /// the same information as everyone else.
    pub fn glyph(&self) -> &'static str {
        match self {
            Health::Unknown => "○",
            Health::Bound => "◍",
            Health::Starting => "◌",
            Health::Open { .. } => "●",
            Health::Closed => "✕",
            Health::Http { status, .. } => match status {
                // Answered, and asked who you are. Nothing is wrong: the
                // service is running and doing exactly what it should.
                401 | 403 => "◆",
                400..=599 => "▲",
                _ => "●",
            },
        }
    }

    pub fn summary(&self) -> String {
        match self {
            Health::Unknown => "checking".into(),
            Health::Bound => "bound".into(),
            Health::Starting => "starting".into(),
            Health::Open { latency } => format!("open · {}", fmt_ms(*latency)),
            Health::Closed => "not responding".into(),
            Health::Http {
                status, latency, ..
            } if self.is_protected() => {
                format!("{status} protected · {}", fmt_ms(*latency))
            }
            Health::Http {
                status, latency, ..
            } => {
                format!("{} {} · {}", status, status_text(*status), fmt_ms(*latency))
            }
        }
    }

    /// Rank for sorting: healthy things first, broken things last.
    pub fn rank(&self) -> u8 {
        match self {
            Health::Http { status, .. } if *status < 400 => 0,
            // A service asking who you are is working, so it sorts with the
            // working ones rather than with the failures.
            _ if self.is_protected() => 1,
            Health::Http { .. } => 2,
            Health::Open { .. } => 3,
            Health::Bound => 4,
            // Above `Unknown`: a service we know is coming up is a better
            // answer than one we have not asked about.
            Health::Starting => 5,
            Health::Unknown => 6,
            Health::Closed => 7,
        }
    }

    /// Answered, and asked for credentials. Anything behind auth — Grafana,
    /// Keycloak, a private API, an admin panel — is healthy, and colouring it
    /// like a 404 teaches you to ignore the colour.
    pub fn is_protected(&self) -> bool {
        matches!(
            self,
            Health::Http {
                status: 401 | 403,
                ..
            }
        )
    }

    /// Not yet serving, and not yet a problem.
    pub fn is_starting(&self) -> bool {
        matches!(self, Health::Starting)
    }

    pub fn is_trouble(&self) -> bool {
        matches!(self, Health::Closed)
            || matches!(self, Health::Http { status, .. } if *status >= 500)
    }
}

pub fn fmt_ms(d: Duration) -> String {
    let ms = d.as_secs_f64() * 1000.0;
    if ms < 10.0 {
        format!("{ms:.1}ms")
    } else {
        format!("{ms:.0}ms")
    }
}

pub fn status_text(code: u16) -> &'static str {
    match code {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        301 => "Moved",
        302 => "Found",
        304 => "Not Modified",
        307 | 308 => "Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Not Allowed",
        426 => "Upgrade Required",
        500 => "Server Error",
        502 => "Bad Gateway",
        503 => "Unavailable",
        _ => "",
    }
}

#[cfg(test)]
mod health_tests {
    use super::*;

    /// Anything behind auth — Grafana, Keycloak, an admin panel — is running
    /// exactly as intended. Drawing it like a 404 teaches you to ignore the
    /// colour, which is the one thing the colour must never do.
    #[test]
    fn a_protected_service_is_not_a_broken_one() {
        let protected = Health::Http {
            status: 401,
            scheme: "http",
            latency: Duration::from_millis(4),
            server: None,
            title: None,
            is_html: false,
        };
        assert!(protected.is_protected());
        assert!(!protected.is_trouble(), "401 is not a failure");
        assert!(
            protected.rank()
                < Health::Open {
                    latency: Duration::ZERO
                }
                .rank(),
            "it sorts with the working services"
        );
        assert!(
            protected.summary().contains("protected"),
            "{}",
            protected.summary()
        );

        // And the glyph differs, so the distinction survives `mono` and a
        // colour-blind reader.
        let missing = Health::Http {
            status: 404,
            scheme: "http",
            latency: Duration::from_millis(4),
            server: None,
            title: None,
            is_html: false,
        };
        assert_ne!(protected.glyph(), missing.glyph());
        assert!(!missing.is_protected());
    }

    #[test]
    fn every_health_state_has_its_own_glyph_where_it_matters() {
        let states = [
            Health::Unknown,
            Health::Bound,
            Health::Closed,
            Health::Open {
                latency: Duration::ZERO,
            },
        ];
        let mut glyphs: Vec<&str> = states.iter().map(|h| h.glyph()).collect();
        glyphs.sort_unstable();
        let before = glyphs.len();
        glyphs.dedup();
        assert_eq!(before, glyphs.len(), "two states share a glyph: {glyphs:?}");
    }

    #[test]
    fn ranking_puts_working_before_broken() {
        let ok = Health::Http {
            status: 200,
            scheme: "http",
            latency: Duration::ZERO,
            server: None,
            title: None,
            is_html: true,
        };
        assert!(ok.rank() < Health::Closed.rank());
        assert!(
            Health::Open {
                latency: Duration::ZERO
            }
            .rank()
                < Health::Closed.rank()
        );
        assert!(Health::Bound.rank() < Health::Closed.rank());
    }
}
