//! PhoneMail accounts. PhoneMail has no IMAP, so ToggleMail uses PhoneMail's own HTTPS interface,
//! the way the PhoneMail apps do: sign in with the phone number and password, then a token.
//!
//! Folders map onto PhoneMail's views. A row is a whole chat in Inbox, Starred, Snoozed, Archive and
//! your own folders (uid = conversation id); a single email in Spam and Trash (uid = message id);
//! a draft in Drafts and Scheduled (uid = draft id).

use crate::accounts::{self, Account, PHONEMAIL_HOST};
use crate::imap::Folder;
use crate::parse::{self, Attachment, Full, Person, Summary};
use crate::smtp::Draft as Outgoing;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;

/// PhoneMail's standard views as ToggleMail folders: (folder name, role).
const STANDARD: [(&str, &str); 7] = [
    ("Inbox", "inbox"),
    ("Snoozed", "snoozed"),
    ("Drafts", "drafts"),
    ("Scheduled", "scheduled"),
    ("Archive", "archive"),
    ("Spam", "junk"),
    ("Trash", "trash"),
];

static TOKENS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

fn url(path: &str) -> String {
    format!("https://{PHONEMAIL_HOST}{path}")
}

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder().timeout(std::time::Duration::from_secs(60)).build()?)
}

/// The 10-digit number from "9884078828@phonemail.net" (or the number itself).
fn phone_of(acc: &Account) -> String {
    let source = if acc.username.contains('@') || acc.username.is_empty() { &acc.email } else { &acc.username };
    let digits: String = source.split('@').next().unwrap_or("").chars().filter(|c| c.is_ascii_digit()).collect();
    digits.chars().rev().take(10).collect::<Vec<_>>().into_iter().rev().collect()
}

fn error_text(v: &Value, fallback: &str) -> String {
    v["error"]["message"].as_str().unwrap_or(fallback).to_string()
}

async fn login(acc: &Account, password: &str) -> Result<String> {
    let phone = phone_of(acc);
    if phone.len() != 10 {
        return Err(anyhow!("enter your PhoneMail address, for example 9876543210@phonemail.net"));
    }
    let r = client()?
        .post(url("/api/auth/login"))
        .header("X-Requested-With", "ToggleMail")
        .json(&json!({ "phone": phone, "password": password, "client": "mobile" }))
        .send()
        .await
        .map_err(|e| anyhow!("could not reach PhoneMail: {e}"))?;
    let ok = r.status().is_success();
    let v: Value = r.json().await.unwrap_or(Value::Null);
    if !ok {
        return Err(anyhow!("sign-in refused: {}", error_text(&v, "wrong number or password")));
    }
    v["token"].as_str().map(str::to_string).ok_or_else(|| anyhow!("PhoneMail did not return a sign-in"))
}

pub async fn check(acc: &Account, password: &str) -> Result<()> {
    let token = login(acc, password).await?;
    TOKENS.lock().unwrap().get_or_insert_with(HashMap::new).insert(acc.id.clone(), token);
    Ok(())
}

async fn token(acc: &Account) -> Result<String> {
    if let Some(t) = TOKENS.lock().unwrap().as_ref().and_then(|m| m.get(&acc.id).cloned()) {
        return Ok(t);
    }
    let t = login(acc, &accounts::password(&acc.id)?).await?;
    TOKENS.lock().unwrap().get_or_insert_with(HashMap::new).insert(acc.id.clone(), t.clone());
    Ok(t)
}

fn forget(acc: &Account) {
    if let Some(m) = TOKENS.lock().unwrap().as_mut() {
        m.remove(&acc.id);
    }
}

/// One call to PhoneMail; signs in again once if the token has expired.
async fn call(acc: &Account, method: &str, path: &str, body: Option<Value>) -> Result<Value> {
    for attempt in 0..2 {
        let t = token(acc).await?;
        let mut req = client()?
            .request(method.parse()?, url(path))
            .bearer_auth(&t)
            .header("X-Requested-With", "ToggleMail");
        if let Some(b) = &body {
            req = req.json(b);
        }
        let r = req.send().await.map_err(|e| anyhow!("could not reach PhoneMail: {e}"))?;
        if r.status().as_u16() == 401 && attempt == 0 {
            forget(acc);
            continue;
        }
        let ok = r.status().is_success();
        let text = r.text().await.unwrap_or_default();
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if !ok {
            return Err(anyhow!("{}", error_text(&v, "PhoneMail refused that")));
        }
        return Ok(v);
    }
    Err(anyhow!("PhoneMail sign-in expired - edit the account and sign in again"))
}

fn ts(v: &Value) -> i64 {
    v.as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.timestamp())
        .unwrap_or(0)
}

fn iso(unix: i64) -> String {
    chrono::DateTime::from_timestamp(unix, 0).map(|d| d.to_rfc3339()).unwrap_or_default()
}

fn person(v: &Value) -> Person {
    Person {
        name: v["display_name"].as_str().unwrap_or("").to_string(),
        email: v["address"].as_str().unwrap_or("").to_string(),
    }
}

fn id(v: &Value) -> u32 {
    v.as_u64().unwrap_or(0) as u32
}

fn blank() -> Summary {
    Summary {
        uid: 0,
        from: Person { name: String::new(), email: String::new() },
        to: vec![],
        subject: String::new(),
        date: 0,
        seen: true,
        flagged: false,
        answered: false,
        size: 0,
        send_at: 0,
        snoozed_until: 0,
    }
}

fn chat_row(c: &Value) -> Summary {
    let from = if c["kind"] == "group" {
        Person { name: c["name"].as_str().unwrap_or("Group").to_string(), email: String::new() }
    } else {
        person(&c["peer"])
    };
    Summary {
        uid: id(&c["conversation_id"]),
        from,
        subject: c["snippet"].as_str().unwrap_or("").to_string(),
        date: ts(&c["last_message_at"]),
        seen: c["unread_count"].as_u64().unwrap_or(0) == 0,
        flagged: c["favourite_count"].as_u64().unwrap_or(0) > 0,
        snoozed_until: ts(&c["snoozed_until"]),
        ..blank()
    }
}

fn message_row(m: &Value) -> Summary {
    Summary {
        uid: id(&m["id"]),
        from: person(&m["sender"]),
        subject: m["subject"].as_str().filter(|s| !s.is_empty()).unwrap_or("(no subject)").to_string(),
        date: ts(&m["sent_at"]),
        seen: m["is_read"].as_bool().unwrap_or(true),
        flagged: m["is_favourite"].as_bool().unwrap_or(false),
        answered: m["is_replied"].as_bool().unwrap_or(false),
        ..blank()
    }
}

fn draft_rows(v: &Value) -> Vec<&Value> {
    v.as_array().or_else(|| v["items"].as_array()).map(|a| a.iter().collect()).unwrap_or_default()
}

fn draft_row(d: &Value) -> Summary {
    let to = d["recipients"]["to"]
        .as_array()
        .map(|a| a.iter().map(|r| Person { name: String::new(), email: r["address"].as_str().unwrap_or("").to_string() }).collect())
        .unwrap_or_default();
    Summary {
        uid: id(&d["id"]),
        to,
        subject: d["subject"].as_str().filter(|s| !s.is_empty()).unwrap_or("(no subject)").to_string(),
        date: ts(&d["updated_at"]),
        send_at: ts(&d["send_at"]),
        ..blank()
    }
}

fn is_chat_folder(folder: &str) -> bool {
    !matches!(folder, "Spam" | "Trash" | "Drafts" | "Scheduled")
}

pub async fn folders(acc: &Account) -> Result<Vec<Folder>> {
    let mut out: Vec<Folder> = STANDARD
        .iter()
        .map(|(n, r)| Folder { name: n.to_string(), role: r.to_string(), delimiter: "/".into() })
        .collect();
    let own = call(acc, "GET", "/api/mail/folders", None).await?;
    for f in own["items"].as_array().into_iter().flatten() {
        if let Some(n) = f["name"].as_str() {
            out.push(Folder { name: n.to_string(), role: String::new(), delimiter: "/".into() });
        }
    }
    Ok(out)
}

async fn own_folder_id(acc: &Account, name: &str) -> Result<u64> {
    let own = call(acc, "GET", "/api/mail/folders", None).await?;
    own["items"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|f| f["name"].as_str() == Some(name))
        .and_then(|f| f["id"].as_u64())
        .ok_or_else(|| anyhow!("folder {name} not found"))
}

async fn chats(acc: &Account, filter: &str) -> Result<Vec<Summary>> {
    let v = call(acc, "GET", &format!("/api/mail/conversations?filter={filter}&limit=50"), None).await?;
    Ok(v["items"].as_array().into_iter().flatten().map(chat_row).collect())
}

pub async fn messages(acc: &Account, folder: &str, before: u32) -> Result<(Vec<Summary>, u32)> {
    if before > 0 {
        return Ok((vec![], 0));
    }
    let items = match folder {
        "Inbox" => chats(acc, "all").await?,
        "Snoozed" => chats(acc, "snoozed").await?,
        "Archive" => chats(acc, "archived").await?,
        "Spam" | "Trash" => {
            let v = call(acc, "GET", &format!("/api/mail/folders/{}?limit=100", folder.to_lowercase()), None).await?;
            v["items"].as_array().into_iter().flatten().map(message_row).collect()
        }
        "Drafts" | "Scheduled" => {
            let v = call(acc, "GET", "/api/mail/drafts", None).await?;
            draft_rows(&v)
                .into_iter()
                .map(draft_row)
                .filter(|d| (folder == "Scheduled") == (d.send_at > 0))
                .collect()
        }
        own => chats(acc, &format!("folder:{}", own_folder_id(acc, own).await?)).await?,
    };
    let total = items.len() as u32;
    Ok((items, total))
}

pub async fn starred(acc: &Account) -> Result<Vec<Summary>> {
    chats(acc, "favorites").await
}

pub async fn search(acc: &Account, query: &str) -> Result<Vec<Summary>> {
    let q: String = query.chars().map(|c| if c.is_alphanumeric() { c.to_string() } else { format!("%{:02X}", c as u32 & 0xff) }).collect();
    let v = call(acc, "GET", &format!("/api/mail/search?q={q}"), None).await?;
    Ok(v["messages"].as_array().into_iter().flatten().map(message_row).collect())
}

fn body_html(m: &Value) -> String {
    match m["body_html"].as_str().filter(|h| !h.trim().is_empty()) {
        Some(h) => parse::sanitize(h),
        None => parse::text_to_html(m["body_text"].as_str().unwrap_or("")),
    }
}

fn attachments_of(m: &Value) -> Vec<Attachment> {
    m["attachments"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|a| Attachment {
            index: a["id"].as_u64().unwrap_or(0) as usize,
            name: a["filename"].as_str().unwrap_or("attachment").to_string(),
            mime: a["content_type"].as_str().unwrap_or("application/octet-stream").to_string(),
            size: a["size_bytes"].as_u64().unwrap_or(0) as usize,
        })
        .collect()
}

fn full_from(uid: u32, list: &[Value], conversation: u64, subject: String) -> Full {
    let last = list.last().cloned().unwrap_or(Value::Null);
    let mut html = String::new();
    for (i, m) in list.iter().enumerate() {
        if i > 0 {
            html.push_str("<hr style=\"border:none;border-top:1px solid #e3e7ec;margin:18px 0\">");
        }
        let who = person(&m["sender"]);
        let when = chrono::DateTime::from_timestamp(ts(&m["sent_at"]), 0)
            .map(|d| d.with_timezone(&chrono::Local).format("%d %b %Y, %H:%M").to_string())
            .unwrap_or_default();
        let files: Vec<String> = attachments_of(m).iter().map(|a| a.name.clone()).collect();
        html.push_str(&format!(
            "<div style=\"color:#6b7686;font-size:13px;margin-bottom:6px\"><b style=\"color:#1c2430\">{}</b> &middot; {}{}</div>{}",
            parse::escape(if who.name.is_empty() { &who.email } else { &who.name }),
            when,
            if files.is_empty() { String::new() } else { format!(" &middot; &#128206; {}", parse::escape(&files.join(", "))) },
            body_html(m)
        ));
    }
    Full {
        uid,
        from: person(&last["sender"]),
        to: vec![],
        cc: vec![],
        reply_to: vec![],
        subject,
        date: ts(&last["sent_at"]),
        message_id: format!("<pm-msg-{}>", last["id"].as_u64().unwrap_or(0)),
        references: format!("<pm-conv-{conversation}>"),
        html,
        text: last["body_text"].as_str().unwrap_or("").to_string(),
        has_remote_images: false,
        attachments: list.iter().flat_map(attachments_of).collect(),
    }
}

pub async fn open(acc: &Account, folder: &str, uid: u32, mark_read: bool) -> Result<Full> {
    match folder {
        "Drafts" | "Scheduled" => {
            let v = call(acc, "GET", "/api/mail/drafts", None).await?;
            let d = draft_rows(&v).into_iter().find(|d| id(&d["id"]) == uid).cloned().ok_or_else(|| anyhow!("draft not found"))?;
            let mut f = full_from(uid, &[], 0, d["subject"].as_str().unwrap_or("").to_string());
            f.html = match d["body_html"].as_str().filter(|h| !h.trim().is_empty()) {
                Some(h) => parse::sanitize(h),
                None => parse::text_to_html(d["body_text"].as_str().unwrap_or("")),
            };
            f.to = draft_row(&d).to;
            Ok(f)
        }
        "Spam" | "Trash" => {
            let m = call(acc, "GET", &format!("/api/mail/messages/{uid}"), None).await?;
            let conv = m["conversation_id"].as_u64().unwrap_or(0);
            if mark_read {
                let _ = call(acc, "PATCH", &format!("/api/mail/mailbox/{uid}"), Some(json!({ "conversation_id": conv, "is_read": true }))).await;
            }
            let subject = m["subject"].as_str().unwrap_or("").to_string();
            Ok(full_from(uid, &[m], conv, subject))
        }
        _ => {
            let page = call(acc, "GET", &format!("/api/mail/conversations/{uid}/messages?limit=50"), None).await?;
            let mut list: Vec<Value> = page["items"].as_array().cloned().unwrap_or_default();
            list.sort_by_key(|m| ts(&m["sent_at"]));
            // A list may hold only the start of long emails: fetch those in full.
            for m in list.iter_mut() {
                if m["truncated"].as_bool() == Some(true) {
                    if let Ok(whole) = call(acc, "GET", &format!("/api/mail/messages/{}?conversation_id={uid}", m["id"]), None).await {
                        *m = whole;
                    }
                }
            }
            if mark_read {
                let _ = call(acc, "POST", "/api/mail/conversations/actions", Some(json!({ "conversation_ids": [uid], "action": "read" }))).await;
            }
            let subject = list.iter().rev().find_map(|m| m["subject"].as_str().filter(|s| !s.is_empty())).unwrap_or("(no subject)").to_string();
            Ok(full_from(uid, &list, uid as u64, subject))
        }
    }
}

/// Bytes and name of an attachment (`index` is PhoneMail's attachment id).
pub async fn attachment(acc: &Account, index: usize, name_hint: Option<String>) -> Result<(String, Vec<u8>)> {
    let t = token(acc).await?;
    let r = client()?.get(url(&format!("/api/mail/attachments/{index}"))).bearer_auth(&t).send().await?;
    if !r.status().is_success() {
        return Err(anyhow!("could not download that attachment"));
    }
    let name = r
        .headers()
        .get("content-disposition")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split("filename=").nth(1))
        .map(|s| s.trim_matches(['"', ';', ' ']).to_string())
        .or(name_hint)
        .unwrap_or_else(|| "attachment".into());
    Ok((name, r.bytes().await?.to_vec()))
}

async fn chat_action(acc: &Account, uid: u32, action: &str, extra: Value) -> Result<()> {
    let mut body = json!({ "conversation_ids": [uid], "action": action });
    if let (Some(b), Some(e)) = (body.as_object_mut(), extra.as_object()) {
        b.extend(e.clone());
    }
    call(acc, "POST", "/api/mail/conversations/actions", Some(body)).await.map(|_| ())
}

/// The newest email of a chat (stars and spam apply to an email).
async fn last_message(acc: &Account, conversation: u32) -> Result<u64> {
    let c = call(acc, "GET", &format!("/api/mail/conversations/{conversation}"), None).await?;
    c["last_message_id"].as_u64().ok_or_else(|| anyhow!("this chat has no email yet"))
}

async fn conversation_of(acc: &Account, message: u32) -> Result<u64> {
    let m = call(acc, "GET", &format!("/api/mail/messages/{message}"), None).await?;
    m["conversation_id"].as_u64().ok_or_else(|| anyhow!("email not found"))
}

async fn mailbox(acc: &Account, message: u64, conversation: u64, change: Value) -> Result<()> {
    let mut body = json!({ "conversation_id": conversation });
    if let (Some(b), Some(c)) = (body.as_object_mut(), change.as_object()) {
        b.extend(c.clone());
    }
    call(acc, "PATCH", &format!("/api/mail/mailbox/{message}"), Some(body)).await.map(|_| ())
}

pub async fn set_flag(acc: &Account, folder: &str, uid: u32, flag: &str, on: bool) -> Result<()> {
    let chat = is_chat_folder(folder);
    match (flag, chat) {
        ("\\Seen", true) => chat_action(acc, uid, if on { "read" } else { "unread" }, json!({})).await,
        ("\\Seen", false) => mailbox(acc, uid as u64, conversation_of(acc, uid).await?, json!({ "is_read": on })).await,
        ("\\Flagged", true) => mailbox(acc, last_message(acc, uid).await?, uid as u64, json!({ "is_favourite": on })).await,
        ("\\Flagged", false) => mailbox(acc, uid as u64, conversation_of(acc, uid).await?, json!({ "is_favourite": on })).await,
        _ => Ok(()),
    }
}

pub async fn move_to(acc: &Account, from: &str, uid: u32, target: &str) -> Result<()> {
    if matches!(from, "Drafts" | "Scheduled") {
        return match (from, target) {
            ("Scheduled", "Drafts") => call(acc, "POST", &format!("/api/mail/drafts/{uid}/unschedule"), Some(json!({}))).await.map(|_| ()),
            _ => Err(anyhow!("drafts stay in Drafts")),
        };
    }
    if !is_chat_folder(from) {
        // One email in Spam or Trash.
        let conv = conversation_of(acc, uid).await?;
        let folder = match target {
            "Spam" => "spam",
            "Trash" => "trash",
            _ => "inbox",
        };
        return mailbox(acc, uid as u64, conv, json!({ "folder": folder })).await;
    }
    match target {
        "Archive" => chat_action(acc, uid, "archive", json!({})).await,
        "Trash" => chat_action(acc, uid, "trash", json!({})).await,
        "Spam" => mailbox(acc, last_message(acc, uid).await?, uid as u64, json!({ "folder": "spam" })).await,
        "Inbox" => match from {
            "Archive" => chat_action(acc, uid, "unarchive", json!({})).await,
            "Snoozed" => chat_action(acc, uid, "unsnooze", json!({})).await,
            _ => chat_action(acc, uid, "unfile", json!({})).await,
        },
        "Snoozed" | "Drafts" | "Scheduled" => Err(anyhow!("use Snooze for that")),
        own => chat_action(acc, uid, "move", json!({ "folder_id": own_folder_id(acc, own).await? })).await,
    }
}

pub async fn destroy(acc: &Account, folder: &str, uid: u32) -> Result<()> {
    match folder {
        "Drafts" | "Scheduled" => call(acc, "DELETE", &format!("/api/mail/drafts/{uid}"), None).await.map(|_| ()),
        "Trash" => Err(anyhow!("emails in Trash are removed by PhoneMail after 30 days")),
        "Spam" => move_to(acc, "Spam", uid, "Trash").await,
        _ => chat_action(acc, uid, "trash", json!({})).await,
    }
}

fn recipients(list: &[String]) -> Vec<Value> {
    list.iter().filter(|a| !a.trim().is_empty()).map(|a| json!({ "address": bare_address(a) })).collect()
}

/// "Ravi <ravi@x.com>" -> "ravi@x.com".
fn bare_address(a: &str) -> String {
    match (a.find('<'), a.rfind('>')) {
        (Some(s), Some(e)) if e > s => a[s + 1..e].trim().to_string(),
        _ => a.trim().to_string(),
    }
}

fn number_in(text: &str, prefix: &str) -> Option<u64> {
    text.split(prefix).nth(1)?.split('>').next()?.parse().ok()
}

pub async fn send(acc: &Account, d: &Outgoing) -> Result<()> {
    if !d.attachments.is_empty() {
        return Err(anyhow!("sending files to PhoneMail from ToggleMail is coming soon - use the PhoneMail app for now"));
    }
    if let (Some(msg), Some(conv)) = (number_in(&d.in_reply_to, "<pm-msg-"), number_in(&d.references, "<pm-conv-")) {
        let to_others = d.cc.iter().chain(&d.bcc).any(|a| !a.trim().is_empty());
        if !to_others {
            return call(acc, "POST", &format!("/api/mail/messages/{msg}/reply"), Some(json!({ "conversation_id": conv, "body_text": d.text })))
                .await
                .map(|_| ());
        }
    }
    let body = json!({
        "to": recipients(&d.to), "cc": recipients(&d.cc), "bcc": recipients(&d.bcc),
        "subject": d.subject, "body_text": d.text,
    });
    call(acc, "POST", "/api/mail/messages", Some(body)).await.map(|_| ())
}

pub async fn schedule(acc: &Account, d: &Outgoing, send_at: i64) -> Result<()> {
    if !d.attachments.is_empty() {
        return Err(anyhow!("scheduling emails with files to PhoneMail is coming soon"));
    }
    let draft = call(acc, "POST", "/api/mail/drafts", Some(json!({
        "recipients": { "to": recipients(&d.to), "cc": recipients(&d.cc), "bcc": recipients(&d.bcc) },
        "subject": d.subject, "body_text": d.text, "body_html": d.html,
    }))).await?;
    let id = draft["id"].as_u64().ok_or_else(|| anyhow!("PhoneMail did not save the draft"))?;
    call(acc, "POST", &format!("/api/mail/drafts/{id}/send"), Some(json!({ "send_at": iso(send_at) }))).await.map(|_| ())
}

pub async fn send_now(acc: &Account, uid: u32) -> Result<()> {
    call(acc, "POST", &format!("/api/mail/drafts/{uid}/send"), Some(json!({}))).await.map(|_| ())
}

pub async fn snooze(acc: &Account, uid: u32, until: i64) -> Result<()> {
    chat_action(acc, uid, "snooze", json!({ "until": iso(until) })).await
}

pub async fn unsnooze(acc: &Account, uid: u32) -> Result<()> {
    chat_action(acc, uid, "unsnooze", json!({})).await
}

pub async fn unread(acc: &Account, folder: &str) -> Result<u32> {
    if folder != "Inbox" {
        return Ok(0);
    }
    let v = call(acc, "GET", "/api/mail/conversations?filter=unread&limit=50", None).await?;
    Ok(v["items"].as_array().map(|a| a.len()).unwrap_or(0) as u32)
}

pub async fn create_folder(acc: &Account, name: &str) -> Result<()> {
    call(acc, "POST", "/api/mail/folders", Some(json!({ "name": name }))).await.map(|_| ())
}

pub async fn rename_folder(acc: &Account, from: &str, to: &str) -> Result<()> {
    let id = own_folder_id(acc, from).await?;
    call(acc, "PATCH", &format!("/api/mail/folders/{id}"), Some(json!({ "name": to }))).await.map(|_| ())
}

pub async fn delete_folder(acc: &Account, name: &str) -> Result<()> {
    let id = own_folder_id(acc, name).await?;
    call(acc, "DELETE", &format!("/api/mail/folders/{id}"), None).await.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_from_address_and_reply_markers() {
        let mut a = Account {
            id: "x".into(), name: String::new(), email: "9884078828@PhoneMail.net".into(), username: String::new(),
            protocol: accounts::Protocol::Phonemail, imap_host: String::new(), imap_port: 443,
            incoming_security: accounts::Security::Tls, smtp_host: String::new(), smtp_port: 443,
            smtp_security: accounts::Security::Tls, accept_invalid_certs: false, signature: String::new(),
        };
        assert_eq!(phone_of(&a), "9884078828");
        a.username = "+91 98840 78828".into();
        assert_eq!(phone_of(&a), "9884078828");
        assert_eq!(number_in("<pm-msg-412>", "<pm-msg-"), Some(412));
        assert_eq!(bare_address("Ravi <ravi@example.com>"), "ravi@example.com");
    }
}
