//! POP3 (RFC 1939) for accounts that use it. POP3 has only the inbox and no read/flag state on the
//! server, so read and flagged marks are kept per account in the app's state folder. Mail is left
//! on the server unless the user deletes it. Each call opens a session and ends with QUIT, which is
//! when the server applies deletions.

use crate::accounts::{self, Account, Security};
use crate::net;
use crate::parse::{self, Summary};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

struct Pop {
    r: BufReader<Box<dyn Io>>,
}

impl Pop {
    async fn line(&mut self) -> Result<String> {
        let mut buf = Vec::new();
        let n = self.r.read_until(b'\n', &mut buf).await?;
        if n == 0 {
            bail!("the server closed the connection");
        }
        Ok(String::from_utf8_lossy(&buf).trim_end().to_string())
    }

    async fn status(&mut self) -> Result<String> {
        let l = self.line().await?;
        match l.strip_prefix("+OK") {
            Some(rest) => Ok(rest.trim().to_string()),
            None => Err(anyhow!("{}", l.strip_prefix("-ERR").unwrap_or(&l).trim().to_string())),
        }
    }

    async fn cmd(&mut self, c: &str) -> Result<String> {
        self.r.get_mut().write_all(format!("{c}\r\n").as_bytes()).await?;
        self.r.get_mut().flush().await?;
        self.status().await
    }

    /// The body of a multi-line reply, with the closing "." removed and dot-stuffing undone.
    async fn multi(&mut self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        loop {
            let mut buf = Vec::new();
            if self.r.read_until(b'\n', &mut buf).await? == 0 {
                bail!("the server closed the connection");
            }
            if buf == b".\r\n" || buf == b".\n" {
                return Ok(out);
            }
            let start = usize::from(buf.starts_with(b".."));
            out.extend_from_slice(&buf[start..]);
        }
    }

    async fn quit(mut self) {
        let _ = self.cmd("QUIT").await;
    }

    /// message number -> unique id, from UIDL.
    async fn uidl(&mut self) -> Result<Vec<(u32, String)>> {
        self.cmd("UIDL").await.context("this server does not support UIDL")?;
        let text = String::from_utf8_lossy(&self.multi().await?).to_string();
        Ok(text
            .lines()
            .filter_map(|l| {
                let mut it = l.split_whitespace();
                Some((it.next()?.parse().ok()?, it.next()?.to_string()))
            })
            .collect())
    }

    async fn sizes(&mut self) -> Result<HashMap<u32, u32>> {
        self.cmd("LIST").await?;
        let text = String::from_utf8_lossy(&self.multi().await?).to_string();
        Ok(text
            .lines()
            .filter_map(|l| {
                let mut it = l.split_whitespace();
                Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
            })
            .collect())
    }
}

pub async fn connect(acc: &Account, password: &str) -> Result<()> {
    open(acc, password).await?.quit().await;
    Ok(())
}

async fn open(acc: &Account, password: &str) -> Result<Pop> {
    let tcp = net::tcp(&acc.imap_host, acc.imap_port).await?;
    let tls = net::tls(acc)?;
    let mut pop = match acc.incoming_security {
        Security::Tls => {
            let s = tls.connect(&acc.imap_host, tcp).await.context("secure connection failed")?;
            let mut p = Pop { r: BufReader::new(Box::new(s)) };
            p.status().await.context("no greeting from the server")?;
            p
        }
        Security::Starttls => {
            let mut plain = BufReader::new(tcp);
            plain_status(&mut plain).await.context("no greeting from the server")?;
            plain.get_mut().write_all(b"STLS\r\n").await?;
            plain_status(&mut plain).await.context("the server refused STARTTLS")?;
            if !plain.buffer().is_empty() {
                bail!("unexpected data from the server after STARTTLS");
            }
            let s = tls.connect(&acc.imap_host, plain.into_inner()).await.context("secure connection failed")?;
            Pop { r: BufReader::new(Box::new(s)) }
        }
    };
    pop.cmd(&format!("USER {}", acc.username)).await.map_err(|e| anyhow!("sign-in refused: {e}"))?;
    pop.cmd(&format!("PASS {password}")).await.map_err(|e| anyhow!("sign-in refused: {e}"))?;
    Ok(pop)
}

async fn plain_status(r: &mut BufReader<tokio::net::TcpStream>) -> Result<()> {
    let mut line = String::new();
    r.read_line(&mut line).await?;
    if line.starts_with("+OK") {
        Ok(())
    } else {
        Err(anyhow!("{}", line.trim()))
    }
}

#[derive(Default, Serialize, Deserialize)]
struct State {
    next: u32,
    ids: HashMap<String, u32>,
    seen: HashSet<u32>,
    flagged: HashSet<u32>,
}

fn load_state(id: &str) -> State {
    accounts::account_file(id, "pop3.json")
        .ok()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_state(id: &str, st: &State) -> Result<()> {
    std::fs::write(accounts::account_file(id, "pop3.json")?, serde_json::to_vec(st)?)?;
    Ok(())
}

/// Gives every message on the server a stable number and forgets ones that are gone.
fn sync_ids(st: &mut State, list: &[(u32, String)]) -> Vec<(u32, u32)> {
    let present: HashSet<&str> = list.iter().map(|(_, u)| u.as_str()).collect();
    st.ids.retain(|u, _| present.contains(u.as_str()));
    let live: HashSet<u32> = st.ids.values().copied().collect();
    st.seen.retain(|i| live.contains(i));
    st.flagged.retain(|i| live.contains(i));
    list.iter()
        .map(|(num, u)| {
            let id = *st.ids.entry(u.clone()).or_insert_with(|| {
                st.next += 1;
                st.next
            });
            (*num, id)
        })
        .collect()
}

async fn summaries(pop: &mut Pop, st: &State, wanted: &[(u32, u32)], sizes: &HashMap<u32, u32>) -> Result<Vec<Summary>> {
    let mut out = vec![];
    for (num, id) in wanted {
        pop.cmd(&format!("TOP {num} 0")).await?;
        let header = pop.multi().await?;
        let mut flags = vec![];
        if st.seen.contains(id) {
            flags.push("\\Seen".to_string());
        }
        if st.flagged.contains(id) {
            flags.push("\\Flagged".to_string());
        }
        out.push(parse::summary(*id, &header, sizes.get(num).copied().unwrap_or(0), &flags));
    }
    Ok(out)
}

pub async fn messages(acc: &Account, before: u32, limit: u32) -> Result<(Vec<Summary>, u32)> {
    let mut pop = open(acc, &accounts::password(&acc.id)?).await?;
    let list = pop.uidl().await?;
    let sizes = pop.sizes().await?;
    let mut st = load_state(&acc.id);
    let ids = sync_ids(&mut st, &list);
    let total = ids.len() as u32;
    let end = total.saturating_sub(before) as usize;
    let start = end.saturating_sub(limit as usize);
    let out = summaries(&mut pop, &st, &ids[start..end], &sizes).await?;
    pop.quit().await;
    save_state(&acc.id, &st)?;
    Ok((out, total))
}

/// POP3 has no server search: match sender and subject over the newest 500 messages.
pub async fn search(acc: &Account, query: &str) -> Result<Vec<Summary>> {
    let (items, _) = messages(acc, 0, 500).await?;
    let q = query.to_lowercase();
    Ok(items
        .into_iter()
        .filter(|m| {
            m.subject.to_lowercase().contains(&q)
                || m.from.name.to_lowercase().contains(&q)
                || m.from.email.to_lowercase().contains(&q)
        })
        .collect())
}

async fn find(pop: &mut Pop, acc: &Account, id: u32) -> Result<u32> {
    let list = pop.uidl().await?;
    let st = load_state(&acc.id);
    let uidl = st.ids.iter().find(|(_, v)| **v == id).map(|(k, _)| k.clone());
    uidl.and_then(|u| list.iter().find(|(_, x)| *x == u).map(|(n, _)| *n))
        .ok_or_else(|| anyhow!("message not found - it may have been deleted"))
}

pub async fn raw(acc: &Account, id: u32, mark_read: bool) -> Result<Vec<u8>> {
    let mut pop = open(acc, &accounts::password(&acc.id)?).await?;
    let num = find(&mut pop, acc, id).await?;
    pop.cmd(&format!("RETR {num}")).await?;
    let body = pop.multi().await?;
    pop.quit().await;
    if mark_read {
        set_flag(acc, id, "\\Seen", true)?;
    }
    Ok(body)
}

pub fn set_flag(acc: &Account, id: u32, flag: &str, on: bool) -> Result<()> {
    let mut st = load_state(&acc.id);
    let set = match flag {
        "\\Seen" => &mut st.seen,
        "\\Flagged" => &mut st.flagged,
        _ => return Ok(()),
    };
    if on {
        set.insert(id);
    } else {
        set.remove(&id);
    }
    save_state(&acc.id, &st)
}

pub async fn delete(acc: &Account, id: u32) -> Result<()> {
    let mut pop = open(acc, &accounts::password(&acc.id)?).await?;
    let num = find(&mut pop, acc, id).await?;
    pop.cmd(&format!("DELE {num}")).await?;
    pop.cmd("QUIT").await?;
    Ok(())
}

pub fn unread(acc: &Account) -> u32 {
    let st = load_state(&acc.id);
    st.ids.values().filter(|i| !st.seen.contains(i)).count() as u32
}
