//! Building and sending mail with lettre. Every message goes out as HTML with a plain-text twin.

use crate::accounts::{Account, Security};
use anyhow::{anyhow, Context, Result};
use base64::Engine;
use lettre::message::{header::ContentType, Attachment, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct OutgoingFile {
    pub name: String,
    pub mime: String,
    /// Base64 contents, as read by the window.
    pub data: String,
}

#[derive(Debug, Deserialize)]
pub struct Draft {
    pub to: Vec<String>,
    #[serde(default)]
    pub cc: Vec<String>,
    #[serde(default)]
    pub bcc: Vec<String>,
    pub subject: String,
    pub html: String,
    pub text: String,
    #[serde(default)]
    pub in_reply_to: String,
    #[serde(default)]
    pub references: String,
    #[serde(default)]
    pub attachments: Vec<OutgoingFile>,
}

fn mailbox(s: &str) -> Result<Mailbox> {
    s.trim().parse().map_err(|_| anyhow!("not a valid address: {s}"))
}

/// The finished message, ready to send and to file in Sent.
pub fn build(acc: &Account, d: &Draft) -> Result<Message> {
    if d.to.iter().chain(&d.cc).chain(&d.bcc).all(|a| a.trim().is_empty()) {
        return Err(anyhow!("add at least one recipient"));
    }
    let from = Mailbox::new(Some(acc.name.clone()).filter(|n| !n.is_empty()), acc.email.parse()?);
    let mut b = Message::builder().from(from).subject(&d.subject);
    for a in d.to.iter().filter(|a| !a.trim().is_empty()) {
        b = b.to(mailbox(a)?);
    }
    for a in d.cc.iter().filter(|a| !a.trim().is_empty()) {
        b = b.cc(mailbox(a)?);
    }
    for a in d.bcc.iter().filter(|a| !a.trim().is_empty()) {
        b = b.bcc(mailbox(a)?);
    }
    if !d.in_reply_to.is_empty() {
        b = b.in_reply_to(d.in_reply_to.clone());
    }
    if !d.references.is_empty() {
        b = b.references(d.references.clone());
    }

    let body = MultiPart::alternative()
        .singlepart(SinglePart::plain(d.text.clone()))
        .singlepart(SinglePart::html(d.html.clone()));
    let msg = if d.attachments.is_empty() {
        b.multipart(body)?
    } else {
        let mut mixed = MultiPart::mixed().multipart(body);
        for f in &d.attachments {
            let bytes = base64::engine::general_purpose::STANDARD.decode(&f.data).context("bad attachment data")?;
            let ct = ContentType::parse(&f.mime).unwrap_or(ContentType::parse("application/octet-stream")?);
            mixed = mixed.singlepart(Attachment::new(f.name.clone()).body(bytes, ct));
        }
        b.multipart(mixed)?
    };
    Ok(msg)
}

pub async fn send(acc: &Account, password: &str, msg: Message) -> Result<()> {
    let creds = Credentials::new(acc.username.clone(), password.to_string());
    let params = TlsParameters::builder(acc.smtp_host.clone())
        .dangerous_accept_invalid_certs(acc.accept_invalid_certs)
        .dangerous_accept_invalid_hostnames(acc.accept_invalid_certs)
        .build()?;
    let tls = match acc.smtp_security {
        Security::Tls => Tls::Wrapper(params),
        Security::Starttls => Tls::Required(params),
    };
    let transport = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&acc.smtp_host)
        .tls(tls)
        .port(acc.smtp_port)
    .credentials(creds)
    .timeout(Some(std::time::Duration::from_secs(60)))
    .build();
    transport.send(msg).await.map_err(|e| anyhow!("sending failed: {e}"))?;
    Ok(())
}
