//! TCP and TLS shared by IMAP and POP3. TLS is rustls (pure Rust), so the same code builds on
//! Windows, macOS, Linux, BSD, Android and iOS; servers are checked against the Mozilla roots.

use crate::accounts::Account;
use anyhow::{anyhow, Context, Result};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;

pub type TlsStream<S> = tokio_rustls::client::TlsStream<S>;

pub async fn tcp(host: &str, port: u16) -> Result<TcpStream> {
    tokio::time::timeout(Duration::from_secs(20), TcpStream::connect((host, port)))
        .await
        .map_err(|_| anyhow!("timed out reaching {host}:{port}"))?
        .with_context(|| format!("could not reach {host}:{port}"))
}

pub struct Tls(tokio_rustls::TlsConnector);

impl Tls {
    pub async fn connect<S: AsyncRead + AsyncWrite + Unpin>(&self, host: &str, stream: S) -> std::io::Result<TlsStream<S>> {
        let name = ServerName::try_from(host.to_string())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        self.0.connect(name, stream).await
    }
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

pub fn tls(acc: &Account) -> Result<Tls> {
    let builder = rustls::ClientConfig::builder_with_provider(provider()).with_safe_default_protocol_versions()?;
    let config = if acc.accept_invalid_certs {
        // Proton Mail Bridge and similar local bridges use their own self-made certificate.
        builder.dangerous().with_custom_certificate_verifier(Arc::new(AcceptAny(provider()))).with_no_client_auth()
    } else {
        let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
        builder.with_root_certificates(roots).with_no_client_auth()
    };
    Ok(Tls(tokio_rustls::TlsConnector::from(Arc::new(config))))
}

/// Accepts any certificate, but still checks the handshake signatures. Only for local bridges.
#[derive(Debug)]
struct AcceptAny(Arc<CryptoProvider>);

impl ServerCertVerifier for AcceptAny {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod live_tests {
    //! Real network checks: `cargo test --lib -- --ignored`. They only read greetings; no sign-in.
    use super::*;
    use crate::accounts::{Protocol, Security};
    use tokio::io::AsyncReadExt;

    fn acc(host: &str) -> Account {
        Account {
            id: "t".into(), name: String::new(), email: format!("x@{host}"), username: String::new(),
            protocol: Protocol::Imap, imap_host: host.into(), imap_port: 993, incoming_security: Security::Tls,
            smtp_host: host.into(), smtp_port: 587, smtp_security: Security::Starttls,
            accept_invalid_certs: false, signature: String::new(),
        }
    }

    async fn imap_greeting(host: &str) -> String {
        let a = acc(host);
        let s = tls(&a).unwrap().connect(host, tcp(host, 993).await.unwrap()).await.unwrap();
        let mut s = s;
        let mut buf = [0u8; 200];
        let n = s.read(&mut buf).await.unwrap();
        String::from_utf8_lossy(&buf[..n]).to_string()
    }

    #[tokio::test]
    #[ignore]
    async fn imap_tls_to_real_servers() {
        for host in ["imap.gmail.com", "mail.jaiaditya.org"] {
            let g = imap_greeting(host).await;
            assert!(g.starts_with("* OK"), "{host}: {g}");
        }
    }

    #[tokio::test]
    #[ignore]
    async fn smtp_starttls_to_real_servers() {
        use lettre::transport::smtp::client::{Tls as LTls, TlsParameters};
        use lettre::{AsyncSmtpTransport, Tokio1Executor};
        for host in ["smtp.gmail.com", "mail.jaiaditya.org"] {
            let t: AsyncSmtpTransport<Tokio1Executor> = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host)
                .port(587)
                .tls(LTls::Required(TlsParameters::new(host.into()).unwrap()))
                .build();
            assert!(t.test_connection().await.unwrap(), "{host}");
        }
    }
}
