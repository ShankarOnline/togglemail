mod accounts;
mod imap;
mod net;
mod parse;
mod pop3;
mod smtp;

use accounts::{Account, Protocol, Provider, Security, ServerSettings};
use imap::{Folder, Pool};
use parse::{Full, Summary};
use serde::{Deserialize, Serialize};
use tauri::State;

type Res<T> = Result<T, String>;

fn err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

#[tauri::command]
fn list_accounts() -> Res<Vec<Account>> {
    accounts::load().map_err(err)
}

#[tauri::command]
fn providers() -> Vec<Provider> {
    accounts::providers()
}

/// Server settings for an address; `provider` "" means match by the address's domain.
#[tauri::command]
fn guess_settings(email: String, provider: String, protocol: Protocol) -> ServerSettings {
    accounts::settings_for(&email, &provider, protocol)
}

#[derive(Deserialize)]
struct AccountInput {
    id: Option<String>,
    name: String,
    email: String,
    username: String,
    protocol: Protocol,
    imap_host: String,
    imap_port: u16,
    incoming_security: Security,
    smtp_host: String,
    smtp_port: u16,
    smtp_security: Security,
    #[serde(default)]
    accept_invalid_certs: bool,
    #[serde(default)]
    signature: String,
}

/// Checks the sign-in against the IMAP server, then saves. An empty password keeps the saved one.
#[tauri::command]
async fn save_account(pool: State<'_, Pool>, input: AccountInput, password: String) -> Res<Account> {
    let email = input.email.trim().to_string();
    let id = input.id.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| accounts::new_id(&email));
    let acc = Account {
        id: id.clone(),
        name: input.name.trim().to_string(),
        username: if input.username.trim().is_empty() { email.clone() } else { input.username.trim().to_string() },
        email,
        protocol: input.protocol,
        imap_host: input.imap_host.trim().to_string(),
        imap_port: input.imap_port,
        incoming_security: input.incoming_security,
        smtp_host: input.smtp_host.trim().to_string(),
        smtp_port: input.smtp_port,
        smtp_security: input.smtp_security,
        accept_invalid_certs: input.accept_invalid_certs,
        signature: input.signature,
    };
    if acc.imap_host.is_empty() || acc.smtp_host.is_empty() {
        return Err("enter the incoming and outgoing (SMTP) server names".into());
    }
    let pw = if password.is_empty() { accounts::password(&id).map_err(err)? } else { password.clone() };
    imap::check(&acc, &pw).await.map_err(err)?;
    pool.forget(&id).await;
    accounts::upsert(acc.clone(), &password).map_err(err)?;
    Ok(acc)
}

#[tauri::command]
async fn remove_account(pool: State<'_, Pool>, id: String) -> Res<()> {
    pool.forget(&id).await;
    accounts::remove(&id).map_err(err)
}

#[tauri::command]
async fn folders(pool: State<'_, Pool>, account: String) -> Res<Vec<Folder>> {
    pool.folders(&account).await.map_err(err)
}

#[derive(Serialize)]
struct Page {
    items: Vec<Summary>,
    total: u32,
}

#[tauri::command]
async fn messages(pool: State<'_, Pool>, account: String, folder: String, before: u32, limit: u32) -> Res<Page> {
    let (items, total) = pool.messages(&account, &folder, before, limit.clamp(1, 200)).await.map_err(err)?;
    Ok(Page { items, total })
}

#[tauri::command]
async fn search(pool: State<'_, Pool>, account: String, folder: String, query: String) -> Res<Vec<Summary>> {
    pool.search(&account, &folder, &query).await.map_err(err)
}

#[tauri::command]
async fn open_message(pool: State<'_, Pool>, account: String, folder: String, uid: u32) -> Res<Full> {
    let raw = pool.raw(&account, &folder, uid, true).await.map_err(err)?;
    parse::full(uid, &raw).ok_or_else(|| "could not read this message".to_string())
}

#[tauri::command]
async fn set_flag(pool: State<'_, Pool>, account: String, folder: String, uid: u32, flag: String, on: bool) -> Res<()> {
    let flag = match flag.as_str() {
        "seen" => "\\Seen",
        "flagged" => "\\Flagged",
        "answered" => "\\Answered",
        _ => return Err("unknown flag".into()),
    };
    pool.set_flag(&account, &folder, uid, flag, on).await.map_err(err)
}

#[tauri::command]
async fn move_message(pool: State<'_, Pool>, account: String, folder: String, uid: u32, target: String) -> Res<()> {
    pool.move_to(&account, &folder, uid, &target).await.map_err(err)
}

/// Moves to Trash; deletes for good only when the message is already in Trash (or there is none).
#[tauri::command]
async fn delete_message(pool: State<'_, Pool>, account: String, folder: String, uid: u32) -> Res<()> {
    let all = pool.folders(&account).await.map_err(err)?;
    match all.iter().find(|f| f.role == "trash") {
        Some(t) if t.name != folder => pool.move_to(&account, &folder, uid, &t.name).await.map_err(err),
        _ => pool.destroy(&account, &folder, uid).await.map_err(err),
    }
}

/// Saves an attachment into Downloads (never overwriting) and returns the path.
#[tauri::command]
async fn save_attachment(pool: State<'_, Pool>, account: String, folder: String, uid: u32, index: usize) -> Res<String> {
    let raw = pool.raw(&account, &folder, uid, false).await.map_err(err)?;
    let (name, bytes) = parse::attachment(&raw, index).ok_or("attachment not found")?;
    let dir = dirs::download_dir().or_else(dirs::home_dir).ok_or("no Downloads folder")?;
    let safe: String = name.chars().map(|c| if "/\\:*?\"<>|".contains(c) || c.is_control() { '_' } else { c }).collect();
    let safe = if safe.trim().is_empty() { "attachment".to_string() } else { safe };
    let (stem, ext) = match safe.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (safe.clone(), String::new()),
    };
    let mut path = dir.join(&safe);
    let mut n = 1;
    while path.exists() {
        path = dir.join(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
async fn send(pool: State<'_, Pool>, account: String, draft: smtp::Draft, reply_to_uid: Option<u32>, reply_folder: Option<String>) -> Res<()> {
    let acc = accounts::get(&account).map_err(err)?;
    let pw = accounts::password(&account).map_err(err)?;
    let msg = smtp::build(&acc, &draft).map_err(err)?;
    let raw = msg.formatted();
    smtp::send(&acc, &pw, msg).await.map_err(err)?;

    // File a copy in Sent (Gmail and Outlook.com do this themselves).
    let saves_itself = acc.smtp_host.contains("gmail.com") || acc.smtp_host.contains("outlook.com");
    if !saves_itself {
        if let Ok(all) = pool.folders(&account).await {
            if let Some(sent) = all.iter().find(|f| f.role == "sent") {
                let _ = pool.append(&account, &sent.name, &raw, "(\\Seen)").await;
            }
        }
    }
    if let (Some(uid), Some(folder)) = (reply_to_uid, reply_folder) {
        let _ = pool.set_flag(&account, &folder, uid, "\\Answered", true).await;
    }
    Ok(())
}

#[tauri::command]
async fn unread(pool: State<'_, Pool>, account: String, folder: String) -> Res<u32> {
    pool.unread_count(&account, &folder).await.map_err(err)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Linux and BSD (Pop!_OS COSMIC, Ubuntu on Wayland, FreeBSD): WebKitGTK's DMA-BUF renderer shows a blank
    // window on some GPU drivers; the plain renderer works everywhere.
    #[cfg(all(unix, not(target_os = "macos")))]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(Pool::default())
        .invoke_handler(tauri::generate_handler![
            list_accounts,
            providers,
            guess_settings,
            save_account,
            remove_account,
            folders,
            messages,
            search,
            open_message,
            set_flag,
            move_message,
            delete_message,
            save_attachment,
            send,
            unread
        ])
        .run(tauri::generate_context!())
        .expect("error while running ToggleMail");
}
