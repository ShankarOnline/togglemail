//! IMAP over TLS (port 993). Sessions are kept open per account and checked with NOOP before reuse.

use crate::accounts::{self, Account, Protocol, Security};
use crate::parse::{self, Summary};
use crate::{net, pop3};
use anyhow::{anyhow, Context, Result};
use async_imap::types::{Fetch, NameAttribute};
use futures::TryStreamExt;
use serde::Serialize;
use std::collections::HashMap;
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_native_tls::TlsStream;

pub type Session = async_imap::Session<TlsStream<TcpStream>>;

#[derive(Default)]
pub struct Pool {
    sessions: Mutex<HashMap<String, Session>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Folder {
    pub name: String,
    /// inbox, sent, drafts, trash, junk, archive, or "" for ordinary folders.
    pub role: String,
    pub delimiter: String,
}

pub async fn connect(acc: &Account, password: &str) -> Result<Session> {
    let tcp = net::tcp(&acc.imap_host, acc.imap_port).await?;
    let tls = net::tls(acc)?;
    let client = match acc.incoming_security {
        Security::Tls => {
            let s = tls.connect(&acc.imap_host, tcp).await.context("secure connection failed")?;
            let mut c = async_imap::Client::new(s);
            c.read_response().await.context("no greeting from the server")??;
            c
        }
        Security::Starttls => {
            let mut plain = async_imap::Client::new(tcp);
            plain.read_response().await.context("no greeting from the server")??;
            plain.run_command_and_check_ok("STARTTLS", None).await.context("the server refused STARTTLS")?;
            let s = tls.connect(&acc.imap_host, plain.into_inner()).await.context("secure connection failed")?;
            async_imap::Client::new(s)
        }
    };
    client
        .login(&acc.username, password)
        .await
        .map_err(|(e, _)| anyhow!("sign-in refused: {e}"))
}

/// Checks a sign-in for either protocol.
pub async fn check(acc: &Account, password: &str) -> Result<()> {
    if acc.protocol == Protocol::Pop3 {
        return pop3::connect(acc, password).await;
    }
    let mut s = connect(acc, password).await?;
    let _ = s.logout().await;
    Ok(())
}

pub(crate) fn pop(id: &str) -> Result<Option<Account>> {
    let acc = accounts::get(id)?;
    Ok((acc.protocol == Protocol::Pop3).then_some(acc))
}

const POP_ONLY_INBOX: &str = "POP3 accounts have only the inbox - switch the account to IMAP to use folders";

impl Pool {
    /// Takes the account's open session (if still alive) or opens a new one.
    pub(crate) async fn take(&self, id: &str) -> Result<Session> {
        let cached = self.sessions.lock().await.remove(id);
        if let Some(mut s) = cached {
            if s.noop().await.is_ok() {
                return Ok(s);
            }
        }
        let acc = accounts::get(id)?;
        connect(&acc, &accounts::password(id)?).await
    }

    pub(crate) async fn give_back(&self, id: &str, s: Session) {
        self.sessions.lock().await.insert(id.to_string(), s);
    }

    pub async fn forget(&self, id: &str) {
        if let Some(mut s) = self.sessions.lock().await.remove(id) {
            let _ = s.logout().await;
        }
    }

    pub async fn folders(&self, id: &str) -> Result<Vec<Folder>> {
        if pop(id)?.is_some() {
            return Ok(vec![Folder { name: "INBOX".into(), role: "inbox".into(), delimiter: "/".into() }]);
        }
        let mut s = self.take(id).await?;
        let mut out = list(&mut s).await?;
        // Like other mail apps, make sure Spam (Junk), Trash and Archive exist so those buttons work.
        let mut created = false;
        // Scheduled and Snoozed hold ToggleMail's send-later and snoozed mail, as in PhoneMail.
        for (role_name, folder) in [
            ("junk", "Junk"),
            ("trash", "Trash"),
            ("archive", "Archive"),
            ("scheduled", "Scheduled"),
            ("snoozed", "Snoozed"),
        ] {
            if !out.iter().any(|f| f.role == role_name) && s.create(folder).await.is_ok() {
                let _ = s.subscribe(folder).await;
                created = true;
            }
        }
        if created {
            out = list(&mut s).await?;
        }
        self.give_back(id, s).await;
        // PhoneMail's order; your own folders come after the standard ones.
        let order = |r: &str| {
            ["inbox", "snoozed", "drafts", "scheduled", "sent", "archive", "junk", "trash"]
                .iter()
                .position(|x| *x == r)
                .unwrap_or(9)
        };
        out.sort_by(|a, b| order(&a.role).cmp(&order(&b.role)).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        Ok(out)
    }

    /// Newest-first page of a folder. `before` is the count already shown.
    pub async fn messages(&self, id: &str, folder: &str, before: u32, limit: u32) -> Result<(Vec<Summary>, u32)> {
        if let Some(acc) = pop(id)? {
            let (mut items, total) = pop3::messages(&acc, before, limit).await?;
            items.reverse();
            return Ok((items, total));
        }
        let mut s = self.take(id).await?;
        let mailbox = s.select(folder).await?;
        let total = mailbox.exists;
        let mut out = vec![];
        if total > before {
            let hi = total - before;
            let lo = hi.saturating_sub(limit - 1).max(1);
            let fetches: Vec<Fetch> = s
                .fetch(format!("{lo}:{hi}"), "(UID FLAGS RFC822.SIZE BODY.PEEK[HEADER])")
                .await?
                .try_collect()
                .await?;
            out = fetches.iter().filter_map(summarise).collect();
        }
        self.give_back(id, s).await;
        out.sort_by(|a, b| b.uid.cmp(&a.uid));
        Ok((out, total))
    }

    pub async fn search(&self, id: &str, folder: &str, query: &str) -> Result<Vec<Summary>> {
        if let Some(acc) = pop(id)? {
            let mut items = pop3::search(&acc, query).await?;
            items.reverse();
            return Ok(items);
        }
        let mut s = self.take(id).await?;
        s.select(folder).await?;
        let q = query.replace('\\', "").replace('"', "");
        let mut uids: Vec<u32> = s.uid_search(format!("CHARSET UTF-8 TEXT \"{q}\"")).await?.into_iter().collect();
        uids.sort_unstable_by(|a, b| b.cmp(a));
        uids.truncate(100);
        let mut out = vec![];
        if !uids.is_empty() {
            let set = uids.iter().map(|u| u.to_string()).collect::<Vec<_>>().join(",");
            let fetches: Vec<Fetch> =
                s.uid_fetch(set, "(UID FLAGS RFC822.SIZE BODY.PEEK[HEADER])").await?.try_collect().await?;
            out = fetches.iter().filter_map(summarise).collect();
        }
        self.give_back(id, s).await;
        out.sort_by(|a, b| b.uid.cmp(&a.uid));
        Ok(out)
    }

    /// The whole message. Fetching BODY[] (not PEEK) marks it read on the server.
    pub async fn raw(&self, id: &str, folder: &str, uid: u32, mark_read: bool) -> Result<Vec<u8>> {
        if let Some(acc) = pop(id)? {
            return pop3::raw(&acc, uid, mark_read).await;
        }
        let mut s = self.take(id).await?;
        s.select(folder).await?;
        let item = if mark_read { "(UID BODY[])" } else { "(UID BODY.PEEK[])" };
        let fetches: Vec<Fetch> = s.uid_fetch(uid.to_string(), item).await?.try_collect().await?;
        self.give_back(id, s).await;
        fetches
            .iter()
            .find_map(|f| f.body().map(|b| b.to_vec()))
            .ok_or_else(|| anyhow!("message not found - it may have been moved or deleted"))
    }

    pub async fn set_flag(&self, id: &str, folder: &str, uid: u32, flag: &str, on: bool) -> Result<()> {
        if let Some(acc) = pop(id)? {
            return pop3::set_flag(&acc, uid, flag, on);
        }
        let mut s = self.take(id).await?;
        s.select(folder).await?;
        let op = if on { "+FLAGS.SILENT" } else { "-FLAGS.SILENT" };
        let _: Vec<Fetch> = s.uid_store(uid.to_string(), format!("{op} ({flag})")).await?.try_collect().await?;
        self.give_back(id, s).await;
        Ok(())
    }

    pub async fn move_to(&self, id: &str, folder: &str, uid: u32, target: &str) -> Result<()> {
        if pop(id)?.is_some() {
            return Err(anyhow!(POP_ONLY_INBOX));
        }
        let mut s = self.take(id).await?;
        s.select(folder).await?;
        if s.uid_mv(uid.to_string(), target).await.is_err() {
            // Servers without MOVE: copy, then delete the original.
            s.uid_copy(uid.to_string(), target).await?;
            let _: Vec<Fetch> = s.uid_store(uid.to_string(), "+FLAGS.SILENT (\\Deleted)").await?.try_collect().await?;
            let _: Vec<u32> = s.expunge().await?.try_collect().await?;
        }
        self.give_back(id, s).await;
        Ok(())
    }

    /// Permanently removes a message (used when it is already in Trash).
    pub async fn destroy(&self, id: &str, folder: &str, uid: u32) -> Result<()> {
        if let Some(acc) = pop(id)? {
            return pop3::delete(&acc, uid).await;
        }
        let mut s = self.take(id).await?;
        s.select(folder).await?;
        let _: Vec<Fetch> = s.uid_store(uid.to_string(), "+FLAGS.SILENT (\\Deleted)").await?.try_collect().await?;
        let _: Vec<u32> = s.expunge().await?.try_collect().await?;
        self.give_back(id, s).await;
        Ok(())
    }

    pub async fn append(&self, id: &str, folder: &str, raw: &[u8], flags: &str) -> Result<()> {
        let mut s = self.take(id).await?;
        s.append(folder, Some(flags), None, raw).await?;
        self.give_back(id, s).await;
        Ok(())
    }

    pub async fn unread_count(&self, id: &str, folder: &str) -> Result<u32> {
        if let Some(acc) = pop(id)? {
            return Ok(pop3::unread(&acc));
        }
        let mut s = self.take(id).await?;
        let st = s.status(folder, "(UNSEEN)").await?;
        self.give_back(id, s).await;
        Ok(st.unseen.unwrap_or(0))
    }
}

async fn list(s: &mut Session) -> Result<Vec<Folder>> {
    let names: Vec<_> = s.list(Some(""), Some("*")).await?.try_collect().await?;
    let found: Vec<(Folder, bool)> = names
        .iter()
        .filter(|n| !n.attributes().iter().any(|a| matches!(a, NameAttribute::NoSelect)))
        .map(|n| {
            let marked = special_use(n.attributes()).is_some();
            let folder = Folder {
                name: n.name().to_string(),
                role: role(n.name(), n.attributes()),
                delimiter: n.delimiter().unwrap_or("/").to_string(),
            };
            (folder, marked)
        })
        .collect();
    Ok(one_per_role(found))
}

/// Mailboxes sometimes hold two folders for one job (iCloud's "Sent Messages" beside an old "Sent").
/// Only one keeps the role, preferring the one the server marks as special-use; the others stay as
/// ordinary folders under their own names.
fn one_per_role(found: Vec<(Folder, bool)>) -> Vec<Folder> {
    let mut chosen: HashMap<String, usize> = HashMap::new();
    for (i, (f, marked)) in found.iter().enumerate() {
        if f.role.is_empty() {
            continue;
        }
        match chosen.get(&f.role) {
            Some(&j) if found[j].1 || !*marked => {}
            _ => {
                chosen.insert(f.role.clone(), i);
            }
        }
    }
    found
        .into_iter()
        .enumerate()
        .map(|(i, (mut f, _))| {
            if !f.role.is_empty() && chosen.get(&f.role) != Some(&i) {
                f.role.clear();
            }
            f
        })
        .collect()
}

impl Pool {
    pub async fn create_folder(&self, id: &str, name: &str) -> Result<()> {
        if pop(id)?.is_some() {
            return Err(anyhow!(POP_ONLY_INBOX));
        }
        let mut s = self.take(id).await?;
        s.create(name).await.context("the server would not create that folder")?;
        let _ = s.subscribe(name).await;
        self.give_back(id, s).await;
        Ok(())
    }

    pub async fn rename_folder(&self, id: &str, from: &str, to: &str) -> Result<()> {
        if pop(id)?.is_some() {
            return Err(anyhow!(POP_ONLY_INBOX));
        }
        let mut s = self.take(id).await?;
        s.rename(from, to).await.context("the server would not rename that folder")?;
        let _ = s.unsubscribe(from).await;
        let _ = s.subscribe(to).await;
        self.give_back(id, s).await;
        Ok(())
    }

    /// Deletes a folder and everything in it.
    pub async fn delete_folder(&self, id: &str, name: &str) -> Result<()> {
        if pop(id)?.is_some() {
            return Err(anyhow!(POP_ONLY_INBOX));
        }
        let mut s = self.take(id).await?;
        // Leave the folder first: some servers refuse to delete the selected mailbox.
        let _ = s.select("INBOX").await;
        let _ = s.unsubscribe(name).await;
        s.delete(name).await.context("the server would not delete that folder")?;
        self.give_back(id, s).await;
        Ok(())
    }
}

pub(crate) fn summarise(f: &Fetch) -> Option<Summary> {
    let flags: Vec<String> = f.flags().map(|fl| format!("{fl:?}")).map(flag_name).collect();
    Some(parse::summary(f.uid?, f.header().unwrap_or(b""), f.size.unwrap_or(0), &flags))
}

/// async-imap's Debug names (Seen, Flagged, Custom("x")) back to IMAP flag names.
fn flag_name(debug: String) -> String {
    match debug.as_str() {
        "Seen" | "Answered" | "Flagged" | "Deleted" | "Draft" | "Recent" => format!("\\{debug}"),
        // Keywords such as $SnoozedUntil1731700000 come through as Custom("...").
        other => other
            .strip_prefix("Custom(\"")
            .and_then(|s| s.strip_suffix("\")"))
            .unwrap_or(other)
            .to_string(),
    }
}

fn special_use(attrs: &[NameAttribute]) -> Option<&'static str> {
    attrs.iter().find_map(|a| match a {
        NameAttribute::Sent => Some("sent"),
        NameAttribute::Drafts => Some("drafts"),
        NameAttribute::Trash => Some("trash"),
        NameAttribute::Junk => Some("junk"),
        NameAttribute::Archive => Some("archive"),
        _ => None,
    })
}

fn role(name: &str, attrs: &[NameAttribute]) -> String {
    if let Some(r) = special_use(attrs) {
        return r.into();
    }
    let lower = name.to_lowercase();
    let last = lower.rsplit(['/', '.']).next().unwrap_or(&lower).to_string();
    match last.as_str() {
        "inbox" => "inbox",
        "sent" | "sent items" | "sent mail" | "sent messages" => "sent",
        "drafts" | "draft" => "drafts",
        "trash" | "deleted" | "deleted items" | "deleted messages" | "bin" => "trash",
        "junk" | "spam" | "junk email" | "junk e-mail" | "bulk mail" => "junk",
        "archive" | "archives" | "all mail" => "archive",
        "scheduled" => "scheduled",
        "snoozed" => "snoozed",
        _ => "",
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(name: &str, role: &str) -> Folder {
        Folder { name: name.into(), role: role.into(), delimiter: "/".into() }
    }

    #[test]
    fn one_folder_per_role_prefers_the_marked_one() {
        // iCloud: "Sent Messages" is marked \Sent; an old "Sent" is not.
        let out = one_per_role(vec![
            (f("INBOX", "inbox"), false),
            (f("Sent", "sent"), false),
            (f("Sent Messages", "sent"), true),
            (f("Trash", "trash"), false),
            (f("Deleted Messages", "trash"), true),
            (f("Junk", "junk"), true),
            (f("Spam", "junk"), false),
        ]);
        let role = |n: &str| out.iter().find(|x| x.name == n).unwrap().role.clone();
        assert_eq!(role("Sent Messages"), "sent");
        assert_eq!(role("Sent"), "");
        assert_eq!(role("Deleted Messages"), "trash");
        assert_eq!(role("Trash"), "");
        assert_eq!(role("Junk"), "junk");
        assert_eq!(role("Spam"), "");
        assert_eq!(out.len(), 7, "no folder is hidden");
    }

    #[test]
    fn without_marks_the_first_one_keeps_the_role() {
        let out = one_per_role(vec![(f("Sent", "sent"), false), (f("Sent Items", "sent"), false)]);
        assert_eq!(out[0].role, "sent");
        assert_eq!(out[1].role, "");
    }
}
