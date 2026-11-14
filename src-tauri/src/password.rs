//! Changing an account's password. Our own domains change it here through the mail server's
//! password endpoint (the current password is checked there); other providers only allow it on
//! their website, so the app opens that page and then saves the new password.

use crate::accounts::{self, Account};
use anyhow::{anyhow, Result};
use serde::Serialize;

const OUR_ENDPOINT: &str = "https://mail.jaiaditya.org/togglemail/password";

#[derive(Serialize)]
pub struct Plan {
    /// True when the password can be changed inside the app.
    pub in_app: bool,
    /// The provider's password page, when it has to be changed there.
    pub url: String,
}

fn provider_id(acc: &Account) -> String {
    accounts::settings_for(&acc.email, "", acc.protocol).provider
}

pub fn plan(acc: &Account) -> Plan {
    let id = provider_id(acc);
    let url = match id.as_str() {
        "gmail" => "https://myaccount.google.com/signinoptions/password",
        "outlook" => "https://account.live.com/password/change",
        "yahoo" => "https://login.yahoo.com/account/security",
        "icloud" => "https://account.apple.com/account/manage",
        "zoho" => "https://accounts.zoho.com/home#security/security_pwd",
        "zoho-in" => "https://accounts.zoho.in/home#security/security_pwd",
        "proton" => "https://account.proton.me/u/0/mail/account-password",
        "mailboxorg" => "https://login.mailbox.org/",
        "posteo" => "https://posteo.de/en/login",
        "runbox" => "https://runbox.com/mail/account_security",
        "mailfence" => "https://mailfence.com/",
        _ => "",
    };
    Plan { in_app: id == "ours", url: url.to_string() }
}

/// Changes the password on our mail server, then saves it in the keychain.
pub async fn change_ours(acc: &Account, current: &str, new: &str) -> Result<()> {
    if new.chars().count() < 10 {
        return Err(anyhow!("the new password must be at least 10 characters"));
    }
    let reply = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?
        .post(OUR_ENDPOINT)
        .form(&[("username", acc.username.to_lowercase()), ("curpass", current.to_string()), ("newpass", new.to_string())])
        .send()
        .await
        .map_err(|e| anyhow!("could not reach the mail server: {e}"))?;
    let status = reply.status();
    let text = reply.text().await.unwrap_or_default();
    let text = text.trim();
    if status.as_u16() == 429 || status.as_u16() == 503 {
        return Err(anyhow!("too many attempts - wait a minute and try again"));
    }
    if text != "OK" {
        let reason = text.strip_prefix("ERR ").unwrap_or(text);
        return Err(anyhow!("{}", if reason.is_empty() { "the server refused the change" } else { reason }));
    }
    accounts::upsert(acc.clone(), new)
}
