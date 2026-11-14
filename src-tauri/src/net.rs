//! TCP and TLS shared by IMAP and POP3.

use crate::accounts::Account;
use anyhow::{anyhow, Context, Result};
use std::time::Duration;
use tokio::net::TcpStream;

pub async fn tcp(host: &str, port: u16) -> Result<TcpStream> {
    tokio::time::timeout(Duration::from_secs(20), TcpStream::connect((host, port)))
        .await
        .map_err(|_| anyhow!("timed out reaching {host}:{port}"))?
        .with_context(|| format!("could not reach {host}:{port}"))
}

pub fn tls(acc: &Account) -> Result<tokio_native_tls::TlsConnector> {
    let mut b = native_tls::TlsConnector::builder();
    if acc.accept_invalid_certs {
        // Proton Mail Bridge and similar local bridges use their own self-made certificate.
        b.danger_accept_invalid_certs(true).danger_accept_invalid_hostnames(true);
    }
    Ok(tokio_native_tls::TlsConnector::from(b.build()?))
}
