# ToggleMail

A desktop email app for **Windows, macOS and Linux**, written in Rust with [Tauri 2](https://tauri.app).
It works with any IMAP, POP3 and SMTP mail service - a free family project by Toggle Minds. Settings are filled in automatically for:

| Addresses | Servers |
|---|---|
| shankars.org, jaiaditya.org, knkoa.org, toggleminds.com, phonemail.net | `mail.jaiaditya.org` IMAP 993 (TLS), SMTP 587 (STARTTLS) |
| Gmail | `imap.gmail.com` / `smtp.gmail.com` (needs an app password) |
| Outlook.com / Hotmail | `outlook.office365.com` / `smtp.office365.com` |
| Yahoo, iCloud, Zoho, StartMail, Mailfence, mailbox.org, Runbox, Posteo, Kolab Now, Soverin | their standard servers (app passwords where required) |
| Proton Mail | through Proton Mail Bridge on the same computer |
| Hostinger, Rackspace, Neo, ChemiCloud, SeekaHost, BigRock | hosting for your own domain |
| Tuta | not possible: Tuta offers no IMAP, POP3 or SMTP |
| anything else | IMAP or POP3, SSL/TLS or STARTTLS, any host and port under *Server settings* |

## What it does

- Several accounts side by side, each with its folders and unread counts
- Message list with search (IMAP server-side), paging, flags and replied markers
- Reader that shows HTML mail safely: scripts and forms are stripped, the frame never runs code,
  internet pictures stay blocked until you click *Show pictures*, links open in your browser
- Reply, reply all, forward, with the original quoted
- HTML compose (bold, italic, lists, links), attachments up to 25 MB, per-account HTML signature
- Every message is sent as HTML with a plain-text copy, and filed in Sent
- Archive, Junk / Not junk, Delete (to Trash; deleting from Trash removes for good), move to any folder
- Attachments save to Downloads
- Checks for new mail every two minutes
- Keyboard: ↑/↓ next/previous, `r` reply, `n` new mail, `Del` delete, `F5` refresh

## Where things are kept

- Account settings: `accounts.json` in the config folder
  (`%APPDATA%\togglemail`, `~/Library/Application Support/togglemail`, `~/.config/togglemail`)
- Passwords: the system keychain (Windows Credential Manager, macOS Keychain, Linux Secret Service /
  GNOME Keyring / KWallet). On Linux desktops without a keychain service the password is kept in
  `~/.config/togglemail/secrets/` with owner-only permissions.
- Mail itself stays on the server; nothing is cached on disk.

## Installers

| System | File |
|---|---|
| Windows 10/11 | `.msi` or `-setup.exe` |
| macOS (Apple silicon and Intel) | `.dmg` |
| Ubuntu, Pop!_OS (incl. COSMIC), Debian, Mint | `.deb` — `sudo apt install ./ToggleMail_*.deb` |
| Fedora, openSUSE, RHEL | `.rpm` — `sudo dnf install ./ToggleMail-*.rpm` |
| Any other Linux | `.AppImage` — `chmod +x` and run |

GitHub Actions (`.github/workflows/build.yml`) builds all of them on every push; a `v*` tag
also makes a draft release with the files attached. Linux builds run on Ubuntu 22.04 so they work on
22.04 and newer (Pop!_OS 22.04/24.04 with COSMIC, Ubuntu 24.04/26.04, Debian 12+).

On Linux the app turns off WebKitGTK's DMA-BUF renderer, which shows a blank window on some
graphics drivers under Wayland (COSMIC, GNOME). Set `WEBKIT_DISABLE_DMABUF_RENDERER=0` to re-enable it.

The macOS and Windows installers are not code-signed yet, so the first launch asks for confirmation
(macOS: right-click > Open; Windows: *More info > Run anyway*).

## Building locally

Needs Rust (rustup) and the Tauri CLI: `cargo install tauri-cli --version "^2"`.

- Windows: Visual Studio Build Tools with *Desktop development with C++*; WebView2 (already in Windows 11)
- macOS: Xcode command-line tools
- Linux: `sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libssl-dev libdbus-1-dev patchelf`

```
cargo tauri dev      # run with live reload of ui/
cargo tauri build    # installers in src-tauri/target/release/bundle/
```

## Layout

```
ui/                  the window: index.html, style.css, app.js (no build step)
src-tauri/src/
  lib.rs             commands the window calls
  accounts.rs        account settings, provider presets, keychain
  imap.rs            IMAP over TLS: folders, lists, search, flags, move, delete, append
  smtp.rs            building and sending (lettre)
  parse.rs           MIME parsing (mail-parser) and HTML sanitising (ammonia)
```

## Open source

ToggleMail is free and open source: GNU GPL version 3 or later, with an additional permission for
app stores ([COPYING.md](COPYING.md), full text in [LICENSE](LICENSE)). Contributions are welcome —
see [CONTRIBUTING.md](CONTRIBUTING.md).

ToggleMail is a free family project by [Toggle Minds](https://toggleminds.com/).
