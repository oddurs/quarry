//! 0042 — what a TLS certificate says, read against real certificates.
//!
//! The fixtures are DER produced by OpenSSL and committed, so these run
//! anywhere and the parser is measured against something it did not write.

use quarry::certificate::{Certificate, parse};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("tests/fixtures/certs/{name}.der"))
        .unwrap_or_else(|e| panic!("read {name}.der: {e}"))
}

fn cert(name: &str) -> Certificate {
    parse(&fixture(name))
}

/// Generated with `-subj "/CN=localhost/O=quarry test"`. The organisation is
/// deliberately there: reading the first attribute rather than finding the
/// common name would pick whichever OpenSSL happened to put first.
#[test]
fn the_subject_and_issuer_are_the_common_names() {
    let c = cert("valid");
    assert_eq!(c.subject.as_deref(), Some("localhost"));
    assert_eq!(c.issuer.as_deref(), Some("localhost"));

    let other = cert("wrongname");
    assert_eq!(other.subject.as_deref(), Some("example.com"));
}

/// The SANs are what a client actually matches; the common name has not been
/// authoritative for twenty years.
#[test]
fn the_names_come_from_subject_alt_name() {
    let c = cert("valid");
    assert_eq!(c.names, vec!["localhost", "quarry.test"]);

    // The fixture also carries `IP:127.0.0.1`, which is a different choice in
    // the same structure and not a name anybody types.
    assert!(
        !c.names.iter().any(|n| n.contains("127.0.0.1")),
        "{:?}",
        c.names
    );
}

/// `notBefore=Jan 1 2019`, `notAfter=Jan 1 2020`, both from OpenSSL.
#[test]
fn validity_dates_are_read_as_seconds_since_the_epoch() {
    let c = cert("expired");
    assert_eq!(c.not_before, Some(1_546_300_800), "2019-01-01T00:00:00Z");
    assert_eq!(c.not_after, Some(1_577_836_800), "2020-01-01T00:00:00Z");
}

#[test]
fn an_expired_certificate_is_known_to_be_expired() {
    let now = 1_767_225_600; // 2026-01-01
    assert!(cert("expired").expired(now));
    assert!(!cert("valid").expired(now));

    // And one about to expire is worth the same warning a little earlier.
    let week = 7 * 86_400;
    assert!(cert("expired").expires_within(week, now));
    assert!(!cert("valid").expires_within(week, now));
}

/// The common `mkcert` failure: a certificate issued for one name, used under
/// another.
#[test]
fn a_name_mismatch_is_visible() {
    assert!(cert("valid").covers("localhost"));
    assert!(cert("valid").covers("quarry.test"));
    assert!(!cert("valid").covers("example.com"));

    assert!(cert("wrongname").covers("example.com"));
    assert!(
        !cert("wrongname").covers("localhost"),
        "a certificate for example.com was accepted for localhost"
    );
}

/// A wildcard covers one label, not a whole subtree.
#[test]
fn a_wildcard_matches_one_label() {
    let c = Certificate {
        names: vec!["*.example.com".into()],
        ..Default::default()
    };
    assert!(c.covers("api.example.com"));
    assert!(c.covers("API.Example.com"), "matching is case-insensitive");
    assert!(
        !c.covers("example.com"),
        "the bare domain is not a subdomain"
    );
    assert!(
        !c.covers("a.b.example.com"),
        "a wildcard is one label, not several"
    );
}

/// Bytes that are not a certificate must produce nothing, not a panic. This is
/// parsing whatever a socket sent us.
#[test]
fn nothing_that_is_not_a_certificate_parses() {
    for bytes in [
        &b""[..],
        b"not der at all",
        &[0x30],
        &[0x30, 0x82],
        &[0x30, 0x82, 0xff, 0xff],
        &[0x30, 0x85, 1, 1, 1, 1, 1],
        &[0x30, 0x00],
    ] {
        assert!(parse(bytes).is_empty(), "{bytes:02x?} parsed as something");
    }
}

/// Truncation is the realistic failure: a read that stopped early. Every
/// prefix of a real certificate has to stop rather than run off the end.
#[test]
fn every_truncation_of_a_real_certificate_is_survivable() {
    let full = fixture("valid");
    for cut in 0..full.len() {
        let partial = parse(&full[..cut]);
        // Whatever it read, it must not have invented a name.
        for name in &partial.names {
            assert!(
                full.windows(name.len()).any(|w| w == name.as_bytes()),
                "invented {name:?} from a truncated certificate"
            );
        }
    }
    // And the whole thing still parses after all that.
    assert_eq!(parse(&full).subject.as_deref(), Some("localhost"));
}

/// A byte flipped anywhere must not panic either.
#[test]
fn a_corrupted_certificate_is_survivable() {
    let full = fixture("valid");
    for i in (0..full.len()).step_by(7) {
        for flip in [0x01u8, 0x80, 0xff] {
            let mut bytes = full.clone();
            bytes[i] ^= flip;
            let _ = parse(&bytes);
        }
    }
}

/// The whole path: a certificate on a service, shown and marked.
mod on_screen {
    use quarry::app::{App, Row};
    use quarry::certificate::Certificate;
    use quarry::model::Kind;
    use quarry::testkit::{self, server};

    const NOW: u64 = 1_767_225_600; // 2026-01-01

    fn with(certificate: Certificate) -> App {
        let mut app = App::new();
        let mut s = server(8443, "node")
            .service("dev server")
            .kind(Kind::Web)
            .health(testkit::served(200, 4, None, None))
            .build();
        s.certificate = Some(certificate);
        app.ingest(vec![s]);
        app.now = NOW;
        app
    }

    fn good() -> Certificate {
        Certificate {
            subject: Some("localhost".into()),
            issuer: Some("mkcert dev".into()),
            names: vec!["localhost".into()],
            not_before: Some(NOW as i64 - 86_400),
            not_after: Some(NOW as i64 + 400 * 86_400),
        }
    }

    fn row(app: &mut App) -> String {
        let screen = quarry::ui::render_to_string(app, 100, 26, 0);
        screen
            .lines()
            .find(|l| l.contains("8443"))
            .expect("the row")
            .to_string()
    }

    #[test]
    fn subject_issuer_and_expiry_are_in_the_detail_pane() {
        let mut app = with(good());
        let screen = quarry::ui::render_to_string(&mut app, 100, 26, 0);
        assert!(screen.contains("CERTIFICATE"), "{screen}");
        assert!(screen.contains("mkcert dev"), "{screen}");
        assert!(screen.contains("localhost"), "{screen}");
        assert!(screen.contains("expires"), "{screen}");
    }

    #[test]
    fn a_healthy_certificate_is_not_marked() {
        let mut app = with(good());
        assert!(!row(&mut app).contains('!'), "{:?}", row(&mut app));
    }

    #[test]
    fn an_expired_certificate_is_marked_in_the_list() {
        let mut app = with(Certificate {
            not_after: Some(NOW as i64 - 86_400),
            ..good()
        });
        assert!(row(&mut app).contains('!'), "{:?}", row(&mut app));
        let screen = quarry::ui::render_to_string(&mut app, 100, 26, 0);
        assert!(screen.contains("certificate expired"), "{screen}");
    }

    #[test]
    fn one_expiring_this_week_is_marked_too() {
        let mut app = with(Certificate {
            not_after: Some(NOW as i64 + 2 * 86_400),
            ..good()
        });
        assert!(row(&mut app).contains('!'));
        let screen = quarry::ui::render_to_string(&mut app, 100, 26, 0);
        assert!(screen.contains("expires within a week"), "{screen}");
    }

    /// The common `mkcert` failure.
    #[test]
    fn a_certificate_for_another_name_is_reported() {
        let mut app = with(Certificate {
            names: vec!["example.com".into()],
            ..good()
        });
        assert!(row(&mut app).contains('!'));
        let screen = quarry::ui::render_to_string(&mut app, 100, 26, 0);
        assert!(screen.contains("not for localhost"), "{screen}");
    }

    /// A service with no certificate says nothing about certificates.
    #[test]
    fn a_plain_http_service_gets_no_section() {
        let mut app = App::new();
        app.ingest(vec![
            server(3000, "node")
                .service("dev server")
                .kind(Kind::Web)
                .build(),
        ]);
        app.now = NOW;
        let screen = quarry::ui::render_to_string(&mut app, 100, 26, 0);
        assert!(!screen.contains("CERTIFICATE"), "{screen}");
        assert!(!screen.contains('!'), "{screen}");
        let _ = Row::Group(0);
    }
}
