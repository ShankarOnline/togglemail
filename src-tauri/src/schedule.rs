//! Scheduled sending, snoozing and the Starred view, kept on the IMAP server so every copy of
//! ToggleMail sees the same state (as PhoneMail's Scheduled and Snoozed folders do).
//!
//! - Scheduled: the finished message waits in the "Scheduled" folder with two extra header lines
//!   (send time and the envelope recipients, so Bcc survives). When it is due, ToggleMail strips
//!   those lines, sends it, files it in Sent and removes it from Scheduled.
//! - Snoozed: the message moves to "Snoozed" with a $SnoozedUntil<unix seconds> keyword; when that
//!   time passes it goes back to the inbox.

use crate::accounts;
use crate::imap::{self, Pool};
use crate::parse::{Summary, SEND_AT_HEADER, SNOOZE_KEYWORD};
use crate::smtp;
use anyhow::{anyhow, Result};
use async_imap::types::Fetch;
use futures::TryStreamExt;
use lettre::address::{Address, Envelope};

const RCPT_HEADER: &str = "X-ToggleMail-Rcpt";

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

async fn folder(pool: &Pool, id: &str, role: &str) -> Result<String> {
    pool.folders(id)
        .await?
        .into_iter()
        .find(|f| f.role == role)
        .map(|f| f.name)
        .ok_or_else(|| anyhow!("this account has no {role} folder"))
}

fn no_pop(id: &str) -> Result<()> {
    if imap::pop(id)?.is_some() {
        return Err(anyhow!("scheduling and snoozing need an IMAP account (POP3 has only the inbox)"));
    }
    Ok(())
}

/// Puts a finished message in Scheduled, to be sent at `send_at` (unix seconds).
pub async fn schedule(pool: &Pool, id: &str, draft: &smtp::Draft, send_at: i64) -> Result<()> {
    no_pop(id)?;
    if send_at <= now() {
        return Err(anyhow!("pick a time in the future"));
    }
    let acc = accounts::get(id)?;
    let msg = smtp::build(&acc, draft)?;
    let rcpts: Vec<String> = msg.envelope().to().iter().map(|a| a.to_string()).collect();
    let mut raw = format!("{SEND_AT_HEADER}: {send_at}\r\n{RCPT_HEADER}: {}\r\n", rcpts.join(", ")).into_bytes();
    raw.extend_from_slice(&msg.formatted());
    let target = folder(pool, id, "scheduled").await?;
    pool.append(id, &target, &raw, "(\\Seen)").await
}

/// Splits our two header lines off a scheduled message: (send time, recipients, message to send).
fn unwrap_scheduled(raw: &[u8]) -> (i64, Vec<String>, Vec<u8>) {
    let mut send_at = 0;
    let mut rcpts = vec![];
    let mut rest = raw;
    loop {
        let end = rest.iter().position(|&b| b == b'\n').map(|i| i + 1).unwrap_or(rest.len());
        let line = String::from_utf8_lossy(&rest[..end]);
        if let Some(v) = line.strip_prefix(&format!("{SEND_AT_HEADER}:")) {
            send_at = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = line.strip_prefix(&format!("{RCPT_HEADER}:")) {
            rcpts = v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        } else {
            break;
        }
        rest = &rest[end..];
    }
    (send_at, rcpts, rest.to_vec())
}

/// Sends scheduled messages that are due (or the one `only` uid, now). Returns how many were sent.
pub async fn send_due(pool: &Pool, id: &str, only: Option<u32>) -> Result<u32> {
    if imap::pop(id)?.is_some() {
        return Ok(0);
    }
    let all = pool.folders(id).await?;
    let (Some(sched), sent) = (
        all.iter().find(|f| f.role == "scheduled").map(|f| f.name.clone()),
        all.iter().find(|f| f.role == "sent").map(|f| f.name.clone()),
    ) else {
        return Ok(0);
    };
    let acc = accounts::get(id)?;
    let mut s = pool.take(id).await?;
    let mailbox = s.select(&sched).await?;
    if mailbox.exists == 0 {
        pool.give_back(id, s).await;
        return Ok(0);
    }
    let set = only.map(|u| u.to_string()).unwrap_or_else(|| "1:*".into());
    let fetches: Vec<Fetch> = s.uid_fetch(&set, "(UID BODY.PEEK[])").await?.try_collect().await?;
    let mut done = 0;
    let mut first_error = None;
    for f in &fetches {
        let (Some(uid), Some(body)) = (f.uid, f.body()) else { continue };
        let (send_at, rcpts, message) = unwrap_scheduled(body);
        if only.is_none() && (send_at == 0 || send_at > now()) {
            continue;
        }
        let to: Vec<Address> = rcpts.iter().filter_map(|r| r.parse().ok()).collect();
        let result = async {
            let envelope = Envelope::new(Some(acc.email.parse()?), to)?;
            smtp::send_raw(&acc, &accounts::password(id)?, &envelope, &message).await
        }
        .await;
        match result {
            Ok(()) => {
                if let Some(sent) = &sent {
                    if !acc.smtp_host.contains("gmail.com") && !acc.smtp_host.contains("outlook.com") {
                        let _ = s.append(sent, Some("(\\Seen)"), None, &message).await;
                    }
                }
                let _: Vec<Fetch> = s.uid_store(uid.to_string(), "+FLAGS.SILENT (\\Deleted)").await?.try_collect().await?;
                done += 1;
            }
            Err(e) => {
                first_error.get_or_insert(e);
            }
        }
    }
    if done > 0 {
        let _: Vec<u32> = s.expunge().await?.try_collect().await?;
    }
    pool.give_back(id, s).await;
    match first_error {
        Some(e) if only.is_some() => Err(e),
        _ => Ok(done),
    }
}

/// Snoozes a message until `until` (unix seconds): keyword, then move to Snoozed.
pub async fn snooze(pool: &Pool, id: &str, from: &str, uid: u32, until: i64) -> Result<()> {
    no_pop(id)?;
    if until <= now() {
        return Err(anyhow!("pick a time in the future"));
    }
    let target = folder(pool, id, "snoozed").await?;
    let mut s = pool.take(id).await?;
    s.select(from).await?;
    let _: Vec<Fetch> = s
        .uid_store(uid.to_string(), format!("+FLAGS.SILENT ({SNOOZE_KEYWORD}{until})"))
        .await?
        .try_collect()
        .await?;
    pool.give_back(id, s).await;
    pool.move_to(id, from, uid, &target).await
}

/// Returns snoozed messages whose time has come (or the one `only` uid) to the inbox.
pub async fn wake_due(pool: &Pool, id: &str, only: Option<u32>) -> Result<u32> {
    if imap::pop(id)?.is_some() {
        return Ok(0);
    }
    let all = pool.folders(id).await?;
    let (Some(snoozed), Some(inbox)) = (
        all.iter().find(|f| f.role == "snoozed").map(|f| f.name.clone()),
        all.iter().find(|f| f.role == "inbox").map(|f| f.name.clone()),
    ) else {
        return Ok(0);
    };
    let mut s = pool.take(id).await?;
    let mailbox = s.select(&snoozed).await?;
    if mailbox.exists == 0 {
        pool.give_back(id, s).await;
        return Ok(0);
    }
    let set = only.map(|u| u.to_string()).unwrap_or_else(|| "1:*".into());
    let fetches: Vec<Fetch> = s.uid_fetch(&set, "(UID FLAGS BODY.PEEK[HEADER])").await?.try_collect().await?;
    let due: Vec<Summary> = fetches
        .iter()
        .filter_map(imap::summarise)
        .filter(|m| only.is_some() || (m.snoozed_until > 0 && m.snoozed_until <= now()))
        .collect();
    for m in &due {
        if m.snoozed_until > 0 {
            let kw = format!("-FLAGS.SILENT ({SNOOZE_KEYWORD}{})", m.snoozed_until);
            let _: Vec<Fetch> = s.uid_store(m.uid.to_string(), kw).await?.try_collect().await?;
        }
        // Back to the inbox, unread, so it is noticed again.
        let _: Vec<Fetch> = s.uid_store(m.uid.to_string(), "-FLAGS.SILENT (\\Seen)").await?.try_collect().await?;
        if s.uid_mv(m.uid.to_string(), &inbox).await.is_err() {
            s.uid_copy(m.uid.to_string(), &inbox).await?;
            let _: Vec<Fetch> = s.uid_store(m.uid.to_string(), "+FLAGS.SILENT (\\Deleted)").await?.try_collect().await?;
            let _: Vec<u32> = s.expunge().await?.try_collect().await?;
        }
    }
    pool.give_back(id, s).await;
    Ok(due.len() as u32)
}

/// Starred: flagged messages in the inbox, newest first.
pub async fn starred(pool: &Pool, id: &str) -> Result<(String, Vec<Summary>)> {
    let inbox = folder(pool, id, "inbox").await?;
    if imap::pop(id)?.is_some() {
        let (items, _) = pool.messages(id, &inbox, 0, 500).await?;
        return Ok((inbox, items.into_iter().filter(|m| m.flagged).collect()));
    }
    let mut s = pool.take(id).await?;
    s.select(&inbox).await?;
    let mut uids: Vec<u32> = s.uid_search("FLAGGED").await?.into_iter().collect();
    uids.sort_unstable_by(|a, b| b.cmp(a));
    uids.truncate(200);
    let mut out = vec![];
    if !uids.is_empty() {
        let set = uids.iter().map(|u| u.to_string()).collect::<Vec<_>>().join(",");
        let fetches: Vec<Fetch> = s.uid_fetch(set, "(UID FLAGS RFC822.SIZE BODY.PEEK[HEADER])").await?.try_collect().await?;
        out = fetches.iter().filter_map(imap::summarise).collect();
    }
    pool.give_back(id, s).await;
    out.sort_by(|a, b| b.uid.cmp(&a.uid));
    Ok((inbox, out))
}
