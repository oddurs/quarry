//! Getting a look at the certificate a service is serving.
//!
//! quarry turns verification off so a self-signed local certificate can be
//! probed at all, and an HTTP client has no reason to hand the certificate
//! back afterwards. So the handshake is run once, directly, for the sole
//! purpose of seeing what was presented.
//!
//! **Nothing here enforces anything.** The verifier accepts every certificate
//! it is shown; its only job is to keep a copy on the way past. That is the
//! same trust posture quarry already had, made explicit rather than hidden in
//! a builder flag.

use std::io::Write;
use std::net::{SocketAddr, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, ClientConnection, DigitallySignedStruct, SignatureScheme};

use crate::certificate::{Certificate, parse};

/// Run a TLS handshake and report what the far end presented.
///
/// `None` when the service does not speak TLS, which is the ordinary answer
/// for most ports and not an error.
pub fn peek(addr: SocketAddr, host: &str, timeout: Duration) -> Option<Certificate> {
    let seen: Arc<Mutex<Option<Vec<u8>>>> = Arc::new(Mutex::new(None));
    let config = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Keeper {
            seen: Arc::clone(&seen),
        }))
        .with_no_client_auth();

    // The name is only used for SNI here; a mismatch is something to report,
    // not something to fail on.
    let server_name = ServerName::try_from(host.to_string())
        .or_else(|_| ServerName::try_from("localhost".to_string()))
        .ok()?;
    let mut connection = ClientConnection::new(Arc::new(config), server_name).ok()?;

    let mut socket = TcpStream::connect_timeout(&addr, timeout).ok()?;
    socket.set_read_timeout(Some(timeout)).ok()?;
    socket.set_write_timeout(Some(timeout)).ok()?;

    // Run the handshake and no further. A certificate arrives well before
    // anything is sent, and quarry has nothing to say to the service here.
    let mut stream = rustls::Stream::new(&mut connection, &mut socket);
    let _ = stream.flush();

    let der = seen.lock().ok()?.take()?;
    let certificate = parse(&der);
    (!certificate.is_empty()).then_some(certificate)
}

/// A verifier that verifies nothing and remembers everything.
///
/// Every method returns success. That is deliberate and is the whole point:
/// quarry looks at services with self-signed certificates for a living, and
/// refusing them would mean reporting nothing about exactly the certificates
/// worth reporting on.
#[derive(Debug)]
struct Keeper {
    seen: Arc<Mutex<Option<Vec<u8>>>>,
}

impl ServerCertVerifier for Keeper {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if let Ok(mut seen) = self.seen.lock() {
            *seen = Some(end_entity.as_ref().to_vec());
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        // Everything ring implements, since refusing a scheme would refuse the
        // certificate we are trying to look at.
        vec![
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ECDSA_NISTP521_SHA512,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::ED25519,
        ]
    }
}
