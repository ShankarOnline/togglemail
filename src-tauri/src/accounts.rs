//! Accounts: settings live in a JSON file in the user's config folder; passwords live in the
//! operating system's keychain (Windows Credential Manager, macOS Keychain, Secret Service on Linux).

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const KEYRING_SERVICE: &str = "ToggleMail";
/// The app's first name; settings and passwords saved under it are moved over on first use.
const OLD_KEYRING_SERVICE: &str = "Shankars Mail";
const OLD_DIR: &str = "shankars-mail";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    #[default]
    Imap,
    Pop3,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Security {
    /// TLS from the first byte (IMAP 993, POP3 995, SMTP 465).
    #[default]
    Tls,
    /// Plain connection upgraded with STARTTLS (IMAP 143, POP3 110, SMTP 587).
    Starttls,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub name: String,
    pub email: String,
    pub username: String,
    #[serde(default)]
    pub protocol: Protocol,
    /// Incoming server (IMAP or POP3, per `protocol`).
    pub imap_host: String,
    pub imap_port: u16,
    #[serde(default)]
    pub incoming_security: Security,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_security: Security,
    /// Only for local bridges with their own certificate (Proton Mail Bridge on 127.0.0.1).
    #[serde(default)]
    pub accept_invalid_certs: bool,
    #[serde(default)]
    pub signature: String,
}

/// A mail service the app knows. `{domain}` in a host is replaced by the address's domain.
#[derive(Clone, Debug, Serialize)]
pub struct Provider {
    pub id: &'static str,
    pub label: &'static str,
    pub domains: &'static [&'static str],
    pub imap: Option<(&'static str, u16, Security)>,
    pub pop3: Option<(&'static str, u16, Security)>,
    pub smtp: Option<(&'static str, u16, Security)>,
    pub accept_invalid_certs: bool,
    pub note: &'static str,
}

use Security::{Starttls as S, Tls as T};

const fn p(
    id: &'static str,
    label: &'static str,
    domains: &'static [&'static str],
    imap: Option<(&'static str, u16, Security)>,
    pop3: Option<(&'static str, u16, Security)>,
    smtp: Option<(&'static str, u16, Security)>,
    note: &'static str,
) -> Provider {
    Provider { id, label, domains, imap, pop3, smtp, accept_invalid_certs: false, note }
}

pub fn providers() -> Vec<Provider> {
    vec![
        p("ours", "Shankars / Jai Aditya / KNKOA / Toggle Minds / PhoneMail",
          &["shankars.org", "jaiaditya.org", "knkoa.org", "toggleminds.com", "phonemail.net"],
          Some(("mail.jaiaditya.org", 993, T)), None, Some(("mail.jaiaditya.org", 587, S)), ""),
        p("gmail", "Gmail", &["gmail.com", "googlemail.com"],
          Some(("imap.gmail.com", 993, T)), Some(("pop.gmail.com", 995, T)), Some(("smtp.gmail.com", 587, S)),
          "Gmail needs an app password: Google Account > Security > 2-Step Verification > App passwords. For POP3, turn it on in Gmail settings > Forwarding and POP/IMAP."),
        p("outlook", "Outlook.com / Hotmail / Microsoft 365", &["outlook.com", "hotmail.com", "live.com", "msn.com", "outlook.in"],
          Some(("outlook.office365.com", 993, T)), Some(("outlook.office365.com", 995, T)), Some(("smtp.office365.com", 587, S)),
          "Microsoft is retiring password sign-in for IMAP/POP/SMTP; if it is refused, the account needs an app password or OAuth (not yet supported)."),
        p("yahoo", "Yahoo Mail", &["yahoo.com", "yahoo.in", "yahoo.co.in", "ymail.com", "rocketmail.com"],
          Some(("imap.mail.yahoo.com", 993, T)), Some(("pop.mail.yahoo.com", 995, T)), Some(("smtp.mail.yahoo.com", 465, T)),
          "Yahoo needs an app password: Account info > Account security > Generate app password."),
        p("icloud", "iCloud Mail", &["icloud.com", "me.com", "mac.com"],
          Some(("imap.mail.me.com", 993, T)), None, Some(("smtp.mail.me.com", 587, S)),
          "iCloud needs an app-specific password from account.apple.com. The user name is the part before @."),
        Provider {
            accept_invalid_certs: true,
            ..p("proton", "Proton Mail (via Proton Mail Bridge)", &["proton.me", "protonmail.com", "protonmail.ch", "pm.me"],
                Some(("127.0.0.1", 1143, S)), None, Some(("127.0.0.1", 1025, S)),
                "Proton has no direct IMAP. Install and sign in to Proton Mail Bridge (paid plans), then use the user name and password the Bridge shows - not your Proton password.")
        },
        p("zoho", "Zoho Mail", &["zoho.com", "zohomail.com"],
          Some(("imap.zoho.com", 993, T)), Some(("pop.zoho.com", 995, T)), Some(("smtp.zoho.com", 465, T)),
          "Turn on IMAP/POP access in Zoho Mail settings. Accounts in India use imap.zoho.in / smtp.zoho.in."),
        p("zoho-in", "Zoho Mail (India data centre)", &["zohomail.in"],
          Some(("imap.zoho.in", 993, T)), Some(("pop.zoho.in", 995, T)), Some(("smtp.zoho.in", 465, T)), ""),
        p("tuta", "Tuta (Tutanota)", &["tuta.com", "tutanota.com", "tutanota.de", "tutamail.com", "tuta.io", "keemail.me"],
          None, None, None,
          "Tuta does not offer IMAP, POP3 or SMTP, so no other mail app can connect to it. Use the Tuta app."),
        p("startmail", "StartMail", &["startmail.com"],
          Some(("imap.startmail.com", 993, T)), None, Some(("smtp.startmail.com", 465, T)),
          "Create an IMAP/SMTP password in StartMail settings > Password management."),
        p("mailfence", "Mailfence", &["mailfence.com"],
          Some(("imap.mailfence.com", 993, T)), Some(("pop3.mailfence.com", 995, T)), Some(("smtp.mailfence.com", 465, T)),
          "IMAP/POP/SMTP need a paid Mailfence plan."),
        p("mailboxorg", "mailbox.org", &["mailbox.org"],
          Some(("imap.mailbox.org", 993, T)), Some(("pop3.mailbox.org", 995, T)), Some(("smtp.mailbox.org", 465, T)), ""),
        p("runbox", "Runbox", &["runbox.com", "runbox.no"],
          Some(("mail.runbox.com", 993, T)), Some(("mail.runbox.com", 995, T)), Some(("mail.runbox.com", 465, T)), ""),
        p("posteo", "Posteo", &["posteo.de", "posteo.net", "posteo.org", "posteo.eu", "posteo.in"],
          Some(("posteo.de", 993, T)), Some(("posteo.de", 995, T)), Some(("posteo.de", 465, T)), ""),
        p("kolab", "Kolab Now", &["kolabnow.com", "kolabnow.ch", "mykolab.com"],
          Some(("imap.kolabnow.com", 993, T)), None, Some(("smtp.kolabnow.com", 587, S)), ""),
        p("soverin", "Soverin", &["soverin.net"],
          Some(("imap.soverin.net", 993, T)), None, Some(("smtp.soverin.net", 587, S)), ""),
        p("hostinger", "Hostinger email (your domain)", &[],
          Some(("imap.hostinger.com", 993, T)), Some(("pop.hostinger.com", 995, T)), Some(("smtp.hostinger.com", 465, T)), ""),
        p("rackspace", "Rackspace email (your domain)", &[],
          Some(("secure.emailsrvr.com", 993, T)), Some(("secure.emailsrvr.com", 995, T)), Some(("secure.emailsrvr.com", 465, T)), ""),
        p("neo", "Neo mail (your domain)", &["neo.space"],
          Some(("imap0001.neo.space", 993, T)), Some(("pop0001.neo.space", 995, T)), Some(("smtp0001.neo.space", 465, T)),
          "If sign-in fails, check the server names in Neo's settings > Other email apps."),
        p("chemicloud", "ChemiCloud email (your domain)", &[],
          Some(("mail.{domain}", 993, T)), Some(("mail.{domain}", 995, T)), Some(("mail.{domain}", 465, T)),
          "ChemiCloud uses cPanel: if mail.<domain> gives a certificate error, use the server name shown in cPanel > Email Accounts > Connect Devices."),
        p("seekahost", "SeekaHost email (your domain)", &[],
          Some(("mail.{domain}", 993, T)), Some(("mail.{domain}", 995, T)), Some(("mail.{domain}", 465, T)),
          "SeekaHost uses cPanel: if mail.<domain> gives a certificate error, use the server name shown in cPanel > Email Accounts > Connect Devices."),
        p("bigrock", "BigRock email (your domain)", &[],
          Some(("mail.{domain}", 993, T)), Some(("mail.{domain}", 995, T)), Some(("mail.{domain}", 465, T)),
          "Check the exact server names in the BigRock control panel > Email > Settings; Titan-based plans use imap.titan.email / smtp.titan.email."),
    ]
}

#[derive(Clone, Debug, Serialize)]
pub struct ServerSettings {
    pub provider: String,
    pub protocol: Protocol,
    pub imap_host: String,
    pub imap_port: u16,
    pub incoming_security: Security,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_security: Security,
    pub accept_invalid_certs: bool,
    pub pop3_available: bool,
    pub supported: bool,
    pub note: String,
}

fn domain_of(email: &str) -> String {
    email.rsplit('@').next().unwrap_or("").trim().to_lowercase()
}

/// Server settings for an address, from a named provider or (with "") by matching the domain.
pub fn settings_for(email: &str, provider_id: &str, protocol: Protocol) -> ServerSettings {
    let domain = domain_of(email);
    let all = providers();
    let found = if provider_id.is_empty() {
        all.iter().find(|p| p.domains.contains(&domain.as_str()))
    } else {
        all.iter().find(|p| p.id == provider_id)
    };
    let fill = |h: &str| h.replace("{domain}", &domain);
    match found {
        Some(p) => {
            let protocol = if protocol == Protocol::Pop3 && p.pop3.is_some() { Protocol::Pop3 } else { Protocol::Imap };
            let incoming = if protocol == Protocol::Pop3 { p.pop3 } else { p.imap };
            let (ih, ip, is) = incoming.unwrap_or(("", 993, T));
            let (sh, sp, ss) = p.smtp.unwrap_or(("", 587, S));
            ServerSettings {
                provider: p.id.into(),
                protocol,
                imap_host: fill(ih),
                imap_port: ip,
                incoming_security: is,
                smtp_host: fill(sh),
                smtp_port: sp,
                smtp_security: ss,
                accept_invalid_certs: p.accept_invalid_certs,
                pop3_available: p.pop3.is_some(),
                supported: p.imap.is_some(),
                note: p.note.into(),
            }
        }
        None => {
            let pop = protocol == Protocol::Pop3;
            ServerSettings {
                provider: String::new(),
                protocol,
                imap_host: format!("mail.{domain}"),
                imap_port: if pop { 995 } else { 993 },
                incoming_security: T,
                smtp_host: format!("mail.{domain}"),
                smtp_port: 587,
                smtp_security: S,
                accept_invalid_certs: false,
                pop3_available: true,
                supported: true,
                note: "Guessed - check the server names with your mail provider.".into(),
            }
        }
    }
}

fn data_dir() -> Result<PathBuf> {
    let base = dirs::config_dir().ok_or_else(|| anyhow!("no config folder"))?;
    let dir = base.join("togglemail");
    let old = base.join(OLD_DIR);
    if !dir.exists() && old.exists() {
        let _ = std::fs::rename(&old, &dir);
    }
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// A per-account file in the app's folder (POP3 read state and ids).
pub fn account_file(id: &str, suffix: &str) -> Result<PathBuf> {
    let dir = data_dir()?.join("state");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join(format!("{id}.{suffix}")))
}

fn accounts_file() -> Result<PathBuf> {
    Ok(data_dir()?.join("accounts.json"))
}

pub fn load() -> Result<Vec<Account>> {
    let path = accounts_file()?;
    if !path.exists() {
        return Ok(vec![]);
    }
    let text = std::fs::read_to_string(&path)?;
    // Windows editors may add a byte-order mark when the file is edited by hand.
    serde_json::from_str(text.trim_start_matches('\u{feff}')).context("accounts.json is not valid")
}

fn save_all(accounts: &[Account]) -> Result<()> {
    let path = accounts_file()?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(accounts)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

pub fn get(id: &str) -> Result<Account> {
    load()?.into_iter().find(|a| a.id == id).ok_or_else(|| anyhow!("account not found"))
}

/// Linux and BSD desktops without a keychain service (some COSMIC and minimal setups) fall back to a file
/// readable only by the user, in the app's config folder.
#[cfg(all(unix, not(target_os = "macos")))]
fn fallback_file(id: &str) -> Result<PathBuf> {
    let dir = data_dir()?.join("secrets");
    std::fs::create_dir_all(&dir)?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    Ok(dir.join(id))
}

fn store_password(id: &str, password: &str) -> Result<()> {
    let saved = keyring::Entry::new(KEYRING_SERVICE, id).and_then(|e| e.set_password(password));
    #[cfg(all(unix, not(target_os = "macos")))]
    if saved.is_err() {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(fallback_file(id)?)?;
        f.write_all(password.as_bytes())?;
        return Ok(());
    }
    saved.context("could not save the password in the system keychain")
}

/// Adds or replaces an account. An empty password keeps the stored one.
pub fn upsert(account: Account, password: &str) -> Result<()> {
    if !password.is_empty() {
        store_password(&account.id, password)?;
    }
    let mut all = load()?;
    match all.iter_mut().find(|a| a.id == account.id) {
        Some(existing) => *existing = account,
        None => all.push(account),
    }
    save_all(&all)
}

pub fn remove(id: &str) -> Result<()> {
    let mut all = load()?;
    all.retain(|a| a.id != id);
    save_all(&all)?;
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, id) {
        let _ = entry.delete_credential();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    if let Ok(path) = fallback_file(id) {
        let _ = std::fs::remove_file(path);
    }
    for suffix in ["pop3.json"] {
        if let Ok(path) = account_file(id, suffix) {
            let _ = std::fs::remove_file(path);
        }
    }
    Ok(())
}

pub fn password(id: &str) -> Result<String> {
    let mut found = keyring::Entry::new(KEYRING_SERVICE, id).and_then(|e| e.get_password());
    if found.is_err() {
        // Saved before the rename: move it to the new name.
        if let Ok(old) = keyring::Entry::new(OLD_KEYRING_SERVICE, id) {
            if let Ok(pw) = old.get_password() {
                if store_password(id, &pw).is_ok() {
                    let _ = old.delete_credential();
                }
                found = Ok(pw);
            }
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    if found.is_err() {
        if let Ok(pw) = std::fs::read_to_string(fallback_file(id)?) {
            return Ok(pw);
        }
    }
    found.context("no saved password for this account - edit the account and enter it again")
}

/// A stable id for a new account, derived from the address.
pub fn new_id(email: &str) -> String {
    let base: String = email
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let taken: Vec<String> = load().unwrap_or_default().into_iter().map(|a| a.id).collect();
    if !taken.contains(&base) {
        return base;
    }
    (2..).map(|n| format!("{base}-{n}")).find(|id| !taken.contains(id)).unwrap()
}
