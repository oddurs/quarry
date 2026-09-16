//! What a TLS certificate says about itself.
//!
//! quarry turns certificate verification off so that a local service with a
//! self-signed certificate can be probed at all. That is the right call, and
//! it means quarry currently knows something about every TLS service and
//! reports none of it.
//!
//! Local TLS goes wrong in a few recognisable ways — a `mkcert` certificate
//! that expired, one issued for a hostname that is not the one you are using —
//! and those are otherwise diagnosed by reading a browser error.
//!
//! **This reports; it does not enforce.** Nothing here rejects a connection.
//!
//! The DER walk is deliberately narrow. It reads the four fields worth showing
//! and steps over everything else without interpreting it, so a certificate
//! using anything unusual costs the field it is in rather than the whole
//! answer.

/// The fields worth showing, and nothing else.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Certificate {
    /// The common name, which is what people read even though the SANs are
    /// what actually match.
    pub subject: Option<String>,
    pub issuer: Option<String>,
    /// `subjectAltName` dNSNames — the names this certificate is actually for.
    pub names: Vec<String>,
    /// Seconds since the epoch.
    pub not_before: Option<i64>,
    pub not_after: Option<i64>,
}

impl Certificate {
    /// Expired, or close enough that it is about to be somebody's morning.
    pub fn expires_within(&self, seconds: i64, now: i64) -> bool {
        self.not_after.is_some_and(|end| end - now < seconds)
    }

    pub fn expired(&self, now: i64) -> bool {
        self.not_after.is_some_and(|end| end <= now)
    }

    /// Whether this certificate is for the host being used.
    ///
    /// The common `mkcert` failure: a certificate issued for one name, used
    /// under another. Matched against the SANs, which is what a client checks;
    /// the common name has not been authoritative for twenty years.
    pub fn covers(&self, host: &str) -> bool {
        let host = host.to_lowercase();
        self.names.iter().any(|name| {
            let name = name.to_lowercase();
            match name.strip_prefix("*.") {
                // A wildcard matches one label, not several.
                Some(rest) => host.split_once('.').is_some_and(|(_, tail)| tail == rest),
                None => name == host,
            }
        })
    }

    /// Whether it says anything at all.
    pub fn is_empty(&self) -> bool {
        *self == Certificate::default()
    }
}

/// One DER value: its tag, its contents, and whatever follows it.
struct Value<'a> {
    tag: u8,
    contents: &'a [u8],
    rest: &'a [u8],
}

/// Read one tag-length-value. Lengths over 127 are written as a count of
/// length bytes followed by that many big-endian bytes.
fn read(der: &[u8]) -> Option<Value<'_>> {
    let tag = *der.first()?;
    let first = *der.get(1)? as usize;
    let (len, header) = if first < 0x80 {
        (first, 2)
    } else {
        let count = first & 0x7f;
        // Four bytes is 4GB; anything claiming more is not a certificate.
        if count == 0 || count > 4 {
            return None;
        }
        let bytes = der.get(2..2 + count)?;
        (
            bytes.iter().fold(0usize, |acc, b| (acc << 8) | *b as usize),
            2 + count,
        )
    };
    let contents = der.get(header..header + len)?;
    Some(Value {
        tag,
        contents,
        rest: &der[header + len..],
    })
}

/// Every value in a sequence's contents, stopping at the first that will not
/// parse rather than guessing past it.
fn children(contents: &[u8]) -> impl Iterator<Item = Value<'_>> {
    let mut left = contents;
    std::iter::from_fn(move || {
        let value = read(left)?;
        left = value.rest;
        Some(value)
    })
}

const SEQUENCE: u8 = 0x30;
const SET: u8 = 0x31;
const OID: u8 = 0x06;
const UTC_TIME: u8 = 0x17;
const GENERALIZED_TIME: u8 = 0x18;
/// `[3]` — the explicitly tagged extensions block.
const EXTENSIONS: u8 = 0xa3;
/// `id-at-commonName`, 2.5.4.3.
const COMMON_NAME: &[u8] = &[0x55, 0x04, 0x03];
/// `id-ce-subjectAltName`, 2.5.29.17.
const SUBJECT_ALT_NAME: &[u8] = &[0x55, 0x1d, 0x11];

/// Parse a DER-encoded X.509 certificate for the fields worth showing.
pub fn parse(der: &[u8]) -> Certificate {
    let mut out = Certificate::default();

    // Certificate ::= SEQUENCE { tbsCertificate, signatureAlgorithm, signature }
    let Some(certificate) = read(der).filter(|v| v.tag == SEQUENCE) else {
        return out;
    };
    let Some(tbs) = read(certificate.contents).filter(|v| v.tag == SEQUENCE) else {
        return out;
    };

    // TBSCertificate ::= SEQUENCE { [0] version, serial, signature, issuer,
    //                               validity, subject, … }
    // The version is optional, so the fields are found by shape rather than by
    // counting: the first two SEQUENCEs after the serial are the algorithm and
    // the issuer, then validity, then the subject.
    let mut sequences = children(tbs.contents).filter(|v| v.tag == SEQUENCE);
    let _algorithm = sequences.next();
    let issuer = sequences.next();
    let validity = sequences.next();
    let subject = sequences.next();

    out.issuer = issuer.and_then(|v| common_name(v.contents));
    out.subject = subject.and_then(|v| common_name(v.contents));

    if let Some(validity) = validity {
        let mut times = children(validity.contents);
        out.not_before = times.next().and_then(|v| time(&v));
        out.not_after = times.next().and_then(|v| time(&v));
    }

    if let Some(extensions) = children(tbs.contents).find(|v| v.tag == EXTENSIONS) {
        out.names = alt_names(extensions.contents);
    }
    out
}

/// A Name is a sequence of RDNs, each a SET of attribute/value pairs. Only the
/// common name is read; the rest is organisation and country, which nobody is
/// looking for here.
fn common_name(name: &[u8]) -> Option<String> {
    for rdn in children(name).filter(|v| v.tag == SET) {
        for pair in children(rdn.contents).filter(|v| v.tag == SEQUENCE) {
            let mut parts = children(pair.contents);
            let oid = parts.next()?;
            if oid.tag == OID && oid.contents == COMMON_NAME {
                let value = parts.next()?;
                return Some(text(value.contents));
            }
        }
    }
    None
}

/// `subjectAltName`'s dNSNames, which is what a client actually matches.
fn alt_names(extensions: &[u8]) -> Vec<String> {
    let Some(list) = read(extensions).filter(|v| v.tag == SEQUENCE) else {
        return Vec::new();
    };
    for extension in children(list.contents).filter(|v| v.tag == SEQUENCE) {
        let mut parts = children(extension.contents);
        let Some(oid) = parts.next() else { continue };
        if oid.tag != OID || oid.contents != SUBJECT_ALT_NAME {
            continue;
        }
        // The value is an OCTET STRING wrapping the real sequence. A critical
        // extension puts a BOOLEAN before it.
        let Some(octets) = parts.find(|v| v.tag == 0x04) else {
            continue;
        };
        let Some(names) = read(octets.contents).filter(|v| v.tag == SEQUENCE) else {
            continue;
        };
        return children(names.contents)
            // GeneralName ::= CHOICE, and dNSName is [2] — an IP address is
            // [7] and is not a name anybody types.
            .filter(|v| v.tag == 0x82)
            .map(|v| text(v.contents))
            .collect();
    }
    Vec::new()
}

/// DER strings here are ASCII in practice; anything else is shown as far as it
/// is readable rather than refused.
fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_string()
}

/// `UTCTime` is `YYMMDDHHMMSSZ` with a fifty-year pivot; `GeneralizedTime` is
/// `YYYYMMDDHHMMSSZ`. Both are always UTC in a certificate.
fn time(value: &Value<'_>) -> Option<i64> {
    let s = text(value.contents);
    let digits: Vec<i64> = s
        .trim_end_matches('Z')
        .chars()
        .map(|c| c.to_digit(10).map(i64::from))
        .collect::<Option<Vec<i64>>>()?;
    let pair = |i: usize| Some(digits.get(i)? * 10 + digits.get(i + 1)?);

    let (year, rest) = match value.tag {
        UTC_TIME if digits.len() >= 12 => {
            let yy = pair(0)?;
            // RFC 5280: 50 and above is 19xx, below is 20xx.
            (if yy >= 50 { 1900 + yy } else { 2000 + yy }, 2)
        }
        GENERALIZED_TIME if digits.len() >= 14 => (pair(0)? * 100 + pair(2)?, 4),
        _ => return None,
    };
    let (month, day) = (pair(rest)?, pair(rest + 2)?);
    let (hour, minute, second) = (pair(rest + 4)?, pair(rest + 6)?, pair(rest + 8)?);
    Some(epoch_seconds(year, month, day, hour, minute, second))
}

/// Days since 1970 by the civil-calendar algorithm, so no date library is
/// needed for four fields.
fn epoch_seconds(year: i64, month: i64, day: i64, hour: i64, minute: i64, second: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    days * 86_400 + hour * 3_600 + minute * 60 + second
}
