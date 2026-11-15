//! Apple Watch. watchOS lets apps make web requests but not mail connections, so the iPhone app does the
//! mail work: it sends the watch a snapshot of every inbox and carries out what the watch asks for (open,
//! mark read, delete, reply). The Swift side, apple/ios/WatchBridge.swift, speaks WatchConnectivity and
//! calls in here through the C functions at the bottom. The rest builds on every platform so it is checked.
#![cfg_attr(not(target_os = "ios"), allow(dead_code))]

use crate::{accounts, imap::Pool, parse, smtp};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

/// Messages per inbox on the watch.
const PER_ACCOUNT: u32 = 15;
/// Longest message text sent to the watch.
const BODY_CHARS: usize = 4000;

#[derive(Serialize)]
struct Snapshot {
    accounts: Vec<Inbox>,
    /// Unix seconds.
    updated: i64,
}

#[derive(Serialize)]
struct Inbox {
    id: String,
    name: String,
    email: String,
    unread: u32,
    messages: Vec<Item>,
    /// Why this inbox could not be read, else empty.
    error: String,
}

#[derive(Serialize)]
struct Item {
    uid: u32,
    from: String,
    email: String,
    subject: String,
    date: i64,
    seen: bool,
    flagged: bool,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Request {
    Refresh,
    Open { account: String, uid: u32 },
    Seen { account: String, uid: u32, on: bool },
    Delete { account: String, uid: u32 },
    Reply { account: String, uid: u32, text: String },
}

const INBOX: &str = "INBOX";

async fn snapshot(app: &AppHandle) -> Snapshot {
    let pool = app.state::<Pool>();
    let mut out = vec![];
    for acc in accounts::load().unwrap_or_default() {
        let mut inbox = Inbox {
            id: acc.id.clone(),
            name: acc.name.clone(),
            email: acc.email.clone(),
            unread: 0,
            messages: vec![],
            error: String::new(),
        };
        match pool.messages(&acc.id, INBOX, 0, PER_ACCOUNT).await {
            Ok((items, _)) => {
                inbox.messages = items
                    .into_iter()
                    .map(|m| Item {
                        uid: m.uid,
                        from: if m.from.name.is_empty() { m.from.email.clone() } else { m.from.name },
                        email: m.from.email,
                        subject: m.subject,
                        date: m.date,
                        seen: m.seen,
                        flagged: m.flagged,
                    })
                    .collect();
                inbox.unread = pool.unread_count(&acc.id, INBOX).await.unwrap_or(0);
            }
            Err(e) => inbox.error = format!("{e:#}"),
        }
        out.push(inbox);
    }
    Snapshot { accounts: out, updated: chrono::Utc::now().timestamp() }
}

/// Readable text of a message for the small screen.
fn body_text(full: &parse::Full) -> String {
    let text = if full.text.trim().is_empty() { strip_tags(&full.html) } else { full.text.clone() };
    let text = text.trim();
    match text.char_indices().nth(BODY_CHARS) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    }
}

fn strip_tags(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    let out = out.replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&amp;", "&");
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

async fn reply(app: &AppHandle, account: String, uid: u32, text: String) -> Result<(), String> {
    let full = crate::open_message(app.state(), account.clone(), INBOX.into(), uid, Some(true)).await?;
    let to = full.reply_to.first().unwrap_or(&full.from).email.clone();
    let subject = if full.subject.to_lowercase().starts_with("re:") { full.subject.clone() } else { format!("Re: {}", full.subject) };
    let references = format!("{} {}", full.references, full.message_id).trim().to_string();
    let draft = smtp::Draft {
        to: vec![to],
        cc: vec![],
        bcc: vec![],
        subject,
        html: parse::text_to_html(&text),
        text,
        in_reply_to: full.message_id,
        references,
        attachments: vec![],
    };
    crate::send(app.state(), account, draft, Some(uid), Some(INBOX.into())).await
}

async fn handle(app: &AppHandle, req: Request) -> Result<serde_json::Value, String> {
    use serde_json::json;
    match req {
        Request::Refresh => Ok(json!({ "snapshot": snapshot(app).await })),
        Request::Open { account, uid } => {
            let full = crate::open_message(app.state(), account, INBOX.into(), uid, Some(true)).await?;
            Ok(json!({ "body": body_text(&full) }))
        }
        Request::Seen { account, uid, on } => {
            crate::set_flag(app.state(), account, INBOX.into(), uid, "seen".into(), on).await?;
            Ok(json!({}))
        }
        Request::Delete { account, uid } => {
            crate::delete_message(app.state(), account, INBOX.into(), uid).await?;
            Ok(json!({}))
        }
        Request::Reply { account, uid, text } => {
            reply(app, account, uid, text).await?;
            Ok(json!({}))
        }
    }
}

/// Answers one request from the watch: `{"ok":true,...}` or `{"ok":false,"error":"..."}`.
fn answer(app: &AppHandle, json: &str) -> String {
    let result = match serde_json::from_str::<Request>(json) {
        Ok(req) => {
            let changes = !matches!(req, Request::Refresh | Request::Open { .. });
            let r = tauri::async_runtime::block_on(handle(app, req));
            if changes && r.is_ok() {
                push_soon(app.clone());
            }
            r
        }
        Err(e) => Err(format!("unknown request: {e}")),
    };
    let mut v = match result {
        Ok(serde_json::Value::Object(m)) => serde_json::Value::Object(m),
        Ok(_) => serde_json::json!({}),
        Err(e) => serde_json::json!({ "error": e }),
    };
    v["ok"] = serde_json::Value::Bool(v.get("error").is_none());
    v.to_string()
}

/// Sends a fresh snapshot to the watch, when one is paired and has the app.
fn push_soon(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        if !ffi::installed() {
            return;
        }
        if let Ok(json) = serde_json::to_string(&snapshot(&app).await) {
            ffi::push(&json);
        }
    });
}

/// Called once at start-up: wakes WatchConnectivity and keeps the watch up to date while the app runs.
pub fn start(app: &AppHandle) {
    if ffi::start(app.clone()) {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                push_soon(app.clone());
                tokio::time::sleep(std::time::Duration::from_secs(300)).await;
            }
        });
    }
}

#[cfg(target_os = "ios")]
mod ffi {
    use std::ffi::{c_char, CStr, CString};
    use std::sync::OnceLock;
    use tauri::AppHandle;

    static APP: OnceLock<AppHandle> = OnceLock::new();

    // In apple/ios/WatchBridge.swift.
    extern "C" {
        fn togglemail_watch_start();
        fn togglemail_watch_installed() -> bool;
        fn togglemail_watch_push(json: *const c_char);
    }

    pub fn start(app: AppHandle) -> bool {
        let _ = APP.set(app);
        unsafe { togglemail_watch_start() };
        true
    }

    pub fn installed() -> bool {
        unsafe { togglemail_watch_installed() }
    }

    pub fn push(json: &str) {
        if let Ok(s) = CString::new(json) {
            unsafe { togglemail_watch_push(s.as_ptr()) };
        }
    }

    /// For WatchBridge.swift: answers a request from the watch. Free the result with `togglemail_watch_free`.
    #[no_mangle]
    pub extern "C" fn togglemail_watch_request(json: *const c_char) -> *mut c_char {
        let Some(app) = APP.get() else { return std::ptr::null_mut() };
        if json.is_null() {
            return std::ptr::null_mut();
        }
        let json = unsafe { CStr::from_ptr(json) }.to_string_lossy();
        CString::new(super::answer(app, &json)).map(CString::into_raw).unwrap_or(std::ptr::null_mut())
    }

    #[no_mangle]
    pub extern "C" fn togglemail_watch_free(s: *mut c_char) {
        if !s.is_null() {
            drop(unsafe { CString::from_raw(s) });
        }
    }
}

/// Other platforms have no watch.
#[cfg(not(target_os = "ios"))]
mod ffi {
    use tauri::AppHandle;
    pub fn start(_: AppHandle) -> bool {
        false
    }
    pub fn installed() -> bool {
        false
    }
    pub fn push(_: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_parse() {
        assert!(matches!(serde_json::from_str::<Request>(r#"{"op":"refresh"}"#), Ok(Request::Refresh)));
        assert!(matches!(
            serde_json::from_str::<Request>(r#"{"op":"reply","account":"a","uid":7,"text":"Thanks!"}"#),
            Ok(Request::Reply { uid: 7, .. })
        ));
        assert!(serde_json::from_str::<Request>(r#"{"op":"format_disk"}"#).is_err());
    }

    #[test]
    fn html_becomes_text() {
        assert_eq!(strip_tags("<p>Hello&nbsp;<b>there</b></p><br>See you"), "Hello there See you");
    }
}
