'use strict';

const invoke = (cmd, args) => window.__TAURI__.core.invoke(cmd, args);
const openUrl = (url) => invoke('plugin:opener|open_url', { url });
const $ = (sel) => document.querySelector(sel);
const PAGE = 50;

const state = {
  accounts: [],
  folders: {},      // account id -> [{name, role, delimiter}]
  unread: {},       // "id|folder" -> count
  account: null,    // current account id
  folder: null,     // current folder name
  items: [],        // summaries shown in the list
  total: 0,
  searching: false,
  open: null,       // the message open in the reader
  showImages: false,
};

// ---------- helpers ----------
function toast(text, ms = 3500) {
  const t = $('#toast');
  t.textContent = text;
  t.hidden = false;
  clearTimeout(toast.timer);
  toast.timer = setTimeout(() => (t.hidden = true), ms);
}
const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const who = (p) => (p && (p.name || p.email)) || '(unknown)';
const addr = (p) => (p.name ? `${p.name} <${p.email}>` : p.email);
function when(ts) {
  if (!ts) return '';
  const d = new Date(ts * 1000);
  const now = new Date();
  if (d.toDateString() === now.toDateString()) return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  if (d.getFullYear() === now.getFullYear()) return d.toLocaleDateString([], { day: 'numeric', month: 'short' });
  return d.toLocaleDateString([], { day: 'numeric', month: 'short', year: 'numeric' });
}
const fullDate = (ts) => (ts ? new Date(ts * 1000).toLocaleString() : '');
const size = (n) => (n > 1048576 ? (n / 1048576).toFixed(1) + ' MB' : Math.max(1, Math.round(n / 1024)) + ' KB');
const folderOf = (id, role) => (state.folders[id] || []).find((f) => f.role === role);
const label = (f) => {
  const names = { inbox: 'Inbox', sent: 'Sent', drafts: 'Drafts', trash: 'Trash', junk: 'Junk', archive: 'Archive' };
  if (names[f.role] && !f.name.includes(f.delimiter)) return names[f.role];
  return f.name.split(f.delimiter).pop();
};

// ---------- sidebar ----------
async function loadAccounts() {
  state.accounts = await invoke('list_accounts');
  $('#compose-btn').disabled = state.accounts.length === 0;
  $('#reader-empty').querySelector('p').hidden = state.accounts.length > 0;
  renderSidebar();
  await Promise.all(state.accounts.map((a) => loadFolders(a.id)));
  if (!state.account && state.accounts.length) {
    const a = state.accounts[0];
    const inbox = folderOf(a.id, 'inbox') || (state.folders[a.id] || [])[0];
    if (inbox) selectFolder(a.id, inbox.name);
  }
}

async function loadFolders(id) {
  try {
    state.folders[id] = await invoke('folders', { account: id });
    state.folders[id].error = null;
  } catch (e) {
    state.folders[id] = Object.assign([], { error: String(e) });
  }
  renderSidebar();
  refreshUnread(id);
}

async function refreshUnread(id) {
  for (const f of state.folders[id] || []) {
    if (!['inbox', 'junk'].includes(f.role)) continue;
    try {
      state.unread[`${id}|${f.name}`] = await invoke('unread', { account: id, folder: f.name });
    } catch { /* offline: keep the old number */ }
  }
  renderSidebar();
}

function renderSidebar() {
  const nav = $('#accounts');
  nav.innerHTML = '';
  for (const a of state.accounts) {
    const box = document.createElement('div');
    box.className = 'acct';
    box.innerHTML = `<div class="acct-head"><span title="${esc(a.email)}">${esc(a.email)}</span><button title="Account settings">⚙</button></div>`;
    box.querySelector('button').onclick = () => openAccountForm(a);
    const folders = state.folders[a.id];
    if (folders?.error) {
      const e = document.createElement('div');
      e.className = 'acct-error';
      e.textContent = folders.error;
      box.append(e);
    }
    for (const f of folders || []) {
      const row = document.createElement('div');
      row.className = 'folder' + (f.role === '' && f.name.includes(f.delimiter) ? ' sub' : '');
      if (a.id === state.account && f.name === state.folder) row.classList.add('active');
      const n = state.unread[`${a.id}|${f.name}`];
      row.innerHTML = `<span>${esc(label(f))}</span>${n ? `<span class="count">${n}</span>` : ''}`;
      row.onclick = () => selectFolder(a.id, f.name);
      box.append(row);
    }
    nav.append(box);
  }
}

// ---------- message list ----------
async function selectFolder(id, folder) {
  state.account = id;
  state.folder = folder;
  state.searching = false;
  $('#search').value = '';
  closeReader();
  renderSidebar();
  const acct = state.accounts.find((a) => a.id === id);
  $('#list-title').textContent = `${label((state.folders[id] || []).find((f) => f.name === folder) || { name: folder, role: '', delimiter: '/' })} · ${acct?.email ?? ''}`;
  await loadMessages(true);
  renderMoveMenu();
}

async function loadMessages(reset) {
  const list = $('#messages');
  if (reset) {
    state.items = [];
    list.innerHTML = '<li class="placeholder">Loading…</li>';
  }
  try {
    const page = await invoke('messages', { account: state.account, folder: state.folder, before: state.items.length, limit: PAGE });
    state.total = page.total;
    state.items = reset ? page.items : state.items.concat(page.items);
    renderList();
  } catch (e) {
    list.innerHTML = `<li class="placeholder">${esc(e)}</li>`;
  }
}

function renderList() {
  const list = $('#messages');
  list.innerHTML = '';
  if (!state.items.length) list.innerHTML = `<li class="placeholder">${state.searching ? 'Nothing found' : 'No messages'}</li>`;
  const sentLike = ['sent', 'drafts'].includes((state.folders[state.account] || []).find((f) => f.name === state.folder)?.role);
  for (const m of state.items) {
    const li = document.createElement('li');
    li.dataset.uid = m.uid;
    if (!m.seen) li.classList.add('unread');
    if (state.open?.uid === m.uid) li.classList.add('active');
    const name = sentLike ? 'To: ' + (m.to.map(who).join(', ') || '—') : who(m.from);
    li.innerHTML = `<div class="top"><span class="who">${esc(name)}</span>${m.flagged ? '<span class="flag">★</span>' : ''}<span class="when">${when(m.date)}</span></div>
      <div class="subj">${m.answered ? '↩ ' : ''}${esc(m.subject)}</div>`;
    li.onclick = () => openMessage(m.uid);
    list.append(li);
  }
  $('#more-btn').hidden = state.searching || state.items.length >= state.total;
}

// ---------- reader ----------
function closeReader() {
  state.open = null;
  $('#reader-msg').hidden = true;
  $('#reader-empty').hidden = false;
}

async function openMessage(uid) {
  state.showImages = false;
  document.querySelectorAll('#messages li').forEach((li) => li.classList.toggle('active', +li.dataset.uid === uid));
  try {
    const m = await invoke('open_message', { account: state.account, folder: state.folder, uid });
    state.open = m;
    const item = state.items.find((x) => x.uid === uid);
    if (item && !item.seen) {
      item.seen = true;
      renderList();
      refreshUnread(state.account);
    }
    renderReader();
  } catch (e) {
    toast(String(e));
  }
}

function renderReader() {
  const m = state.open;
  $('#reader-empty').hidden = true;
  $('#reader-msg').hidden = false;
  $('#r-subject').textContent = m.subject;
  $('#r-from').textContent = m.from.name || m.from.email;
  $('#r-from-email').textContent = m.from.name ? `<${m.from.email}>` : '';
  const to = m.to.map(addr).join(', ');
  const cc = m.cc.map(addr).join(', ');
  $('#r-to').textContent = `To: ${to}${cc ? ' · Cc: ' + cc : ''}`;
  $('#r-date').textContent = fullDate(m.date);
  const item = state.items.find((x) => x.uid === m.uid);
  $('#reader-msg [data-act=flag]').textContent = item?.flagged ? '★ Unflag' : '☆ Flag';
  $('#remote-banner').hidden = !m.has_remote_images || state.showImages;

  const ul = $('#r-attachments');
  ul.innerHTML = '';
  for (const a of m.attachments) {
    const li = document.createElement('li');
    const b = document.createElement('button');
    b.textContent = `📎 ${a.name} (${size(a.size)})`;
    b.onclick = async () => {
      try {
        const path = await invoke('save_attachment', { account: state.account, folder: state.folder, uid: m.uid, index: a.index });
        toast(`Saved to ${path}`);
        invoke('plugin:opener|reveal_item_in_dir', { path }).catch(() => {});
      } catch (e) { toast(String(e)); }
    };
    li.append(b);
    ul.append(li);
  }

  const imgs = state.showImages ? 'data: https: http:' : 'data:';
  const frame = $('#r-body');
  frame.srcdoc = `<!doctype html><html><head><meta charset="utf-8">
    <meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src ${imgs}; style-src 'unsafe-inline'; font-src data:">
    <base target="_blank"><style>body{font:14px/1.5 system-ui,-apple-system,"Segoe UI",Ubuntu,sans-serif;margin:16px;color:#1c2430;word-wrap:break-word}
    img{max-width:100%;height:auto}blockquote{border-left:3px solid #ccd;margin-left:4px;padding-left:10px;color:#556}
    </style></head><body>${m.html}</body></html>`;
  frame.onload = () => {
    // Links open in the system browser; the frame itself never runs scripts.
    frame.contentDocument.addEventListener('click', (ev) => {
      const a = ev.target.closest('a[href]');
      if (!a) return;
      ev.preventDefault();
      const href = a.getAttribute('href');
      if (href.startsWith('mailto:')) composeTo(decodeURIComponent(href.slice(7).split('?')[0]));
      else if (/^https?:/i.test(href)) openUrl(href);
    });
  };
}

function renderMoveMenu() {
  const sel = $('#move-to');
  sel.innerHTML = '<option value="">Move to…</option>';
  for (const f of state.folders[state.account] || []) {
    if (f.name === state.folder) continue;
    const o = document.createElement('option');
    o.value = f.name;
    o.textContent = label(f);
    sel.append(o);
  }
}

async function afterRemoval(uid) {
  const i = state.items.findIndex((x) => x.uid === uid);
  state.items = state.items.filter((x) => x.uid !== uid);
  state.total = Math.max(0, state.total - 1);
  renderList();
  const next = state.items[Math.min(i, state.items.length - 1)];
  if (next) openMessage(next.uid); else closeReader();
  refreshUnread(state.account);
}

async function moveOpen(target) {
  const uid = state.open.uid;
  try {
    await invoke('move_message', { account: state.account, folder: state.folder, uid, target });
    toast(`Moved to ${target}`);
    afterRemoval(uid);
  } catch (e) { toast(String(e)); }
}

async function act(action) {
  const m = state.open;
  if (!m) return;
  const item = state.items.find((x) => x.uid === m.uid);
  try {
    switch (action) {
      case 'reply': return composeReply(false);
      case 'replyall': return composeReply(true);
      case 'forward': return composeForward();
      case 'delete':
        await invoke('delete_message', { account: state.account, folder: state.folder, uid: m.uid });
        toast('Deleted');
        return afterRemoval(m.uid);
      case 'archive': {
        const f = folderOf(state.account, 'archive');
        if (!f) return toast('This account has no Archive folder');
        return moveOpen(f.name);
      }
      case 'junk': {
        const inJunk = folderOf(state.account, 'junk')?.name === state.folder;
        const f = folderOf(state.account, inJunk ? 'inbox' : 'junk');
        if (!f) return toast('This account has no Junk folder');
        return moveOpen(f.name);
      }
      case 'unread':
        await invoke('set_flag', { account: state.account, folder: state.folder, uid: m.uid, flag: 'seen', on: false });
        if (item) item.seen = false;
        renderList();
        refreshUnread(state.account);
        return closeReader();
      case 'flag': {
        const on = !item?.flagged;
        await invoke('set_flag', { account: state.account, folder: state.folder, uid: m.uid, flag: 'flagged', on });
        if (item) item.flagged = on;
        renderList();
        return renderReader();
      }
    }
  } catch (e) { toast(String(e)); }
}

// ---------- compose ----------
let draftMeta = {};
let files = [];
let savedRange = null;

function fillFromSelect() {
  const sel = $('#compose-form [name=account]');
  sel.innerHTML = '';
  for (const a of state.accounts) {
    const o = document.createElement('option');
    o.value = a.id;
    o.textContent = a.name ? `${a.name} <${a.email}>` : a.email;
    sel.append(o);
  }
  sel.value = state.account || state.accounts[0]?.id;
}

function signatureHtml(id) {
  const a = state.accounts.find((x) => x.id === id);
  return a?.signature ? `<p><br></p><div class="signature">-- <br>${a.signature}</div>` : '';
}

function openCompose({ to = '', cc = '', subject = '', body = '', meta = {} } = {}) {
  if (!state.accounts.length) return toast('Add an account first');
  const f = $('#compose-form');
  fillFromSelect();
  f.to.value = to;
  f.cc.value = cc;
  f.bcc.value = '';
  f.subject.value = subject;
  $('#editor').innerHTML = `<p><br></p>${signatureHtml(f.account.value)}${body}`;
  draftMeta = meta;
  files = [];
  renderFiles();
  $('#compose-error').textContent = '';
  $('#compose-dlg').showModal();
  (to ? $('#editor') : f.to).focus();
}

const composeTo = (address) => openCompose({ to: address });

function quoteBlock(m) {
  return `<p>On ${esc(fullDate(m.date))}, ${esc(addr(m.from))} wrote:</p><blockquote>${m.html}</blockquote>`;
}

function composeReply(all) {
  const m = state.open;
  const me = state.accounts.find((a) => a.id === state.account)?.email.toLowerCase();
  const primary = (m.reply_to.length ? m.reply_to : [m.from]).map(addr);
  let cc = [];
  if (all) {
    cc = [...m.to, ...m.cc]
      .filter((p) => p.email && p.email.toLowerCase() !== me && !primary.some((x) => x.includes(p.email)))
      .map(addr);
  }
  const subject = /^re:/i.test(m.subject) ? m.subject : `Re: ${m.subject}`;
  openCompose({
    to: primary.join(', '),
    cc: cc.join(', '),
    subject,
    body: quoteBlock(m),
    meta: { in_reply_to: m.message_id, references: `${m.references} ${m.message_id}`.trim(), reply_uid: m.uid, reply_folder: state.folder },
  });
}

function composeForward() {
  const m = state.open;
  const subject = /^fwd?:/i.test(m.subject) ? m.subject : `Fwd: ${m.subject}`;
  const head = `<p>---------- Forwarded message ----------<br>From: ${esc(addr(m.from))}<br>Date: ${esc(fullDate(m.date))}<br>Subject: ${esc(m.subject)}<br>To: ${esc(m.to.map(addr).join(', '))}</p>`;
  openCompose({ subject, body: head + m.html });
}

function renderFiles() {
  const ul = $('#compose-files');
  ul.innerHTML = '';
  files.forEach((f, i) => {
    const li = document.createElement('li');
    li.innerHTML = `📎 ${esc(f.name)} (${size(f.size)})<button type="button" title="Remove">✕</button>`;
    li.querySelector('button').onclick = () => { files.splice(i, 1); renderFiles(); };
    ul.append(li);
  });
}

function readFile(file) {
  return new Promise((resolve, reject) => {
    const r = new FileReader();
    r.onload = () => resolve({ name: file.name, mime: file.type || 'application/octet-stream', size: file.size, data: String(r.result).split(',')[1] || '' });
    r.onerror = () => reject(r.error);
    r.readAsDataURL(file);
  });
}

const splitList = (s) => s.split(/[,;]/).map((x) => x.trim()).filter(Boolean);

function htmlToText(html) {
  const div = document.createElement('div');
  div.innerHTML = html.replace(/<br\s*\/?>/gi, '\n').replace(/<\/(p|div|li|h\d|blockquote)>/gi, '\n');
  return div.textContent.replace(/\n{3,}/g, '\n\n').trim();
}

async function send(ev) {
  ev.preventDefault();
  const f = $('#compose-form');
  const btn = $('#send-btn');
  const html = `<div style="font-family:system-ui,-apple-system,'Segoe UI',sans-serif;font-size:14px">${$('#editor').innerHTML}</div>`;
  const draft = {
    to: splitList(f.to.value),
    cc: splitList(f.cc.value),
    bcc: splitList(f.bcc.value),
    subject: f.subject.value,
    html,
    text: htmlToText($('#editor').innerHTML),
    in_reply_to: draftMeta.in_reply_to || '',
    references: draftMeta.references || '',
    attachments: files.map(({ name, mime, data }) => ({ name, mime, data })),
  };
  btn.disabled = true;
  btn.textContent = 'Sending…';
  $('#compose-error').textContent = '';
  try {
    const sameAccount = f.account.value === state.account;
    await invoke('send', {
      account: f.account.value,
      draft,
      replyToUid: sameAccount ? draftMeta.reply_uid ?? null : null,
      replyFolder: sameAccount ? draftMeta.reply_folder ?? null : null,
    });
    $('#compose-dlg').close();
    toast('Sent');
    if (draftMeta.reply_uid) {
      const it = state.items.find((x) => x.uid === draftMeta.reply_uid);
      if (it) { it.answered = true; renderList(); }
    }
  } catch (e) {
    $('#compose-error').textContent = String(e);
  } finally {
    btn.disabled = false;
    btn.textContent = 'Send';
  }
}

// ---------- account form ----------
let editing = null;

function openAccountForm(account = null) {
  editing = account;
  const f = $('#account-form');
  f.reset();
  $('#account-title').textContent = account ? 'Account settings' : 'Add account';
  $('#account-save').textContent = account ? 'Save' : 'Sign in';
  $('#account-remove').hidden = !account;
  $('#account-error').textContent = '';
  $('#provider-note').textContent = '';
  $('#advanced').open = !!account;
  if (account) {
    for (const k of ['name', 'email', 'username', 'protocol', 'imap_host', 'imap_port', 'incoming_security', 'smtp_host', 'smtp_port', 'smtp_security', 'signature']) f[k].value = account[k] ?? f[k].value;
    f.accept_invalid_certs.checked = !!account.accept_invalid_certs;
    f.password.placeholder = 'Leave blank to keep the saved password';
    f.password.required = false;
  } else {
    f.password.placeholder = '';
    f.password.required = true;
  }
  $('#incoming-label').textContent = f.protocol.value === 'pop3' ? 'POP3 server' : 'IMAP server';
  $('#account-save').disabled = false;
  $('#account-dlg').showModal();
}

async function loadProviders() {
  const sel = $('#account-form').provider;
  for (const p of await invoke('providers')) {
    const o = document.createElement('option');
    o.value = p.id;
    o.textContent = p.label;
    sel.append(o);
  }
  const custom = document.createElement('option');
  custom.value = 'custom';
  custom.textContent = 'Other / custom servers';
  sel.append(custom);
}

/// Fills the server fields from the chosen provider (or the address), unless the user picked custom.
async function suggestServers() {
  const f = $('#account-form');
  $('#incoming-label').textContent = f.protocol.value === 'pop3' ? 'POP3 server' : 'IMAP server';
  if (f.provider.value === 'custom') {
    $('#provider-note').textContent = 'Enter the server names and ports your mail provider gives you.';
    $('#advanced').open = true;
    $('#account-save').disabled = false;
    return;
  }
  if (!f.email.value.includes('@')) return;
  const s = await invoke('guess_settings', { email: f.email.value, provider: f.provider.value, protocol: f.protocol.value });
  if (s.protocol !== f.protocol.value) f.protocol.value = s.protocol;
  $('#incoming-label').textContent = s.protocol === 'pop3' ? 'POP3 server' : 'IMAP server';
  f.imap_host.value = s.imap_host;
  f.imap_port.value = s.imap_port;
  f.incoming_security.value = s.incoming_security;
  f.smtp_host.value = s.smtp_host;
  f.smtp_port.value = s.smtp_port;
  f.smtp_security.value = s.smtp_security;
  f.accept_invalid_certs.checked = s.accept_invalid_certs;
  if (!f.username.value) f.username.placeholder = f.email.value;
  let note = s.note;
  if (!s.pop3_available && s.supported) note = (note ? note + ' ' : '') + 'This provider offers IMAP only.';
  $('#provider-note').textContent = note;
  $('#account-save').disabled = !s.supported;
}

/// Standard ports when the security choice changes on custom servers.
function defaultPorts(ev) {
  const f = $('#account-form');
  const pop = f.protocol.value === 'pop3';
  if (ev.target === f.incoming_security) f.imap_port.value = f.incoming_security.value === 'tls' ? (pop ? 995 : 993) : (pop ? 110 : 143);
  if (ev.target === f.smtp_security) f.smtp_port.value = f.smtp_security.value === 'tls' ? 465 : 587;
}

async function saveAccount(ev) {
  ev.preventDefault();
  const f = $('#account-form');
  const btn = $('#account-save');
  btn.disabled = true;
  btn.textContent = 'Checking…';
  $('#account-error').textContent = '';
  try {
    const input = {
      id: editing?.id ?? null,
      name: f.name.value,
      email: f.email.value,
      username: f.username.value,
      protocol: f.protocol.value,
      imap_host: f.imap_host.value,
      imap_port: +f.imap_port.value,
      incoming_security: f.incoming_security.value,
      smtp_host: f.smtp_host.value,
      smtp_port: +f.smtp_port.value,
      smtp_security: f.smtp_security.value,
      accept_invalid_certs: f.accept_invalid_certs.checked,
      signature: f.signature.value,
    };
    const saved = await invoke('save_account', { input, password: f.password.value });
    $('#account-dlg').close();
    toast(`${saved.email} is ready`);
    if (!editing) state.account = null;
    await loadAccounts();
    if (!editing) {
      const inbox = folderOf(saved.id, 'inbox');
      if (inbox) selectFolder(saved.id, inbox.name);
    }
  } catch (e) {
    $('#account-error').textContent = String(e);
    $('#advanced').open = true;
  } finally {
    btn.disabled = false;
    btn.textContent = editing ? 'Save' : 'Sign in';
  }
}

async function removeAccount() {
  if (!editing) return;
  const btn = $('#account-remove');
  if (btn.dataset.confirm !== '1') {
    btn.dataset.confirm = '1';
    btn.textContent = 'Click again to remove';
    setTimeout(() => { btn.dataset.confirm = ''; btn.textContent = 'Remove account'; }, 4000);
    return;
  }
  await invoke('remove_account', { id: editing.id });
  delete state.folders[editing.id];
  if (state.account === editing.id) {
    state.account = null;
    state.items = [];
    renderList();
    closeReader();
  }
  $('#account-dlg').close();
  toast('Account removed from this computer (mail stays on the server)');
  loadAccounts();
}

// ---------- wiring ----------
function wire() {
  $('#compose-btn').onclick = () => openCompose();
  $('#add-account-btn').onclick = () => openAccountForm();
  $('#refresh-btn').onclick = () => state.account && selectFolder(state.account, state.folder);
  $('#more-btn').onclick = () => loadMessages(false);
  $('#show-images').onclick = () => { state.showImages = true; renderReader(); };
  $('#move-to').onchange = (e) => { if (e.target.value) moveOpen(e.target.value); e.target.value = ''; };
  document.querySelectorAll('#reader-msg [data-act]').forEach((b) => (b.onclick = () => act(b.dataset.act)));

  let timer;
  $('#search').oninput = (e) => {
    clearTimeout(timer);
    const q = e.target.value.trim();
    timer = setTimeout(async () => {
      if (!state.account) return;
      if (!q) return selectFolder(state.account, state.folder);
      state.searching = true;
      try {
        state.items = await invoke('search', { account: state.account, folder: state.folder, query: q });
        renderList();
      } catch (err) { toast(String(err)); }
    }, 400);
  };

  $('#account-form').onsubmit = saveAccount;
  $('#account-form').email.onchange = () => !editing && suggestServers();
  $('#account-form').provider.onchange = suggestServers;
  $('#account-form').protocol.onchange = suggestServers;
  $('#account-form').incoming_security.onchange = defaultPorts;
  $('#account-form').smtp_security.onchange = defaultPorts;
  $('#account-cancel').onclick = () => $('#account-dlg').close();
  $('#account-remove').onclick = removeAccount;

  $('#compose-form').onsubmit = send;
  $('#compose-cancel').onclick = () => $('#compose-dlg').close();
  document.querySelectorAll('.editor-bar [data-cmd]').forEach((b) => {
    b.onmousedown = (e) => e.preventDefault();
    b.onclick = () => {
      if (b.dataset.cmd === 'createLink') {
        // Remember the selection, then ask for the address in the inline box.
        const sel = window.getSelection();
        savedRange = sel.rangeCount ? sel.getRangeAt(0).cloneRange() : null;
        $('#link-box').hidden = false;
        $('#link-url').value = 'https://';
        $('#link-url').focus();
        return;
      }
      document.execCommand(b.dataset.cmd, false, null);
      $('#editor').focus();
    };
  });
  $('#link-add').onclick = () => {
    const url = $('#link-url').value.trim();
    $('#link-box').hidden = true;
    if (!savedRange || !/^(https?:\/\/|mailto:)\S+$/i.test(url)) return;
    const sel = window.getSelection();
    sel.removeAllRanges();
    sel.addRange(savedRange);
    if (savedRange.collapsed) document.execCommand('insertHTML', false, `<a href="${esc(url)}">${esc(url)}</a>`);
    else document.execCommand('createLink', false, url);
    $('#editor').focus();
  };
  $('#link-url').onkeydown = (e) => {
    if (e.key === 'Enter') { e.preventDefault(); $('#link-add').click(); }
    if (e.key === 'Escape') { e.preventDefault(); $('#link-box').hidden = true; }
  };
  $('#attach-input').onchange = async (e) => {
    for (const file of e.target.files) {
      if (file.size > 25 * 1048576) { toast(`${file.name} is over 25 MB`); continue; }
      files.push(await readFile(file));
    }
    e.target.value = '';
    renderFiles();
  };

  document.addEventListener('keydown', (e) => {
    if (document.querySelector('dialog[open]')) return;
    if (e.target.matches('input, textarea, [contenteditable]')) return;
    if (e.key === 'F5') { e.preventDefault(); $('#refresh-btn').click(); }
    else if (e.key === 'Delete' && state.open) act('delete');
    else if (e.key === 'r' && state.open) act('reply');
    else if (e.key === 'n' || (e.ctrlKey && e.key === 'n')) { e.preventDefault(); openCompose(); }
    else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      if (!state.items.length) return;
      e.preventDefault();
      const i = state.items.findIndex((x) => x.uid === state.open?.uid);
      const j = Math.max(0, Math.min(state.items.length - 1, i + (e.key === 'ArrowDown' ? 1 : -1)));
      openMessage(state.items[j].uid);
    }
  });

  // New mail check every two minutes.
  setInterval(async () => {
    for (const a of state.accounts) refreshUnread(a.id);
    if (state.account && !state.searching && !state.open) {
      try {
        const page = await invoke('messages', { account: state.account, folder: state.folder, before: 0, limit: PAGE });
        state.total = page.total;
        const known = new Set(state.items.map((x) => x.uid));
        const fresh = page.items.filter((x) => !known.has(x.uid));
        if (fresh.length) { state.items = fresh.concat(state.items); renderList(); }
      } catch { /* offline */ }
    }
  }, 120000);
}

wire();
loadProviders().catch(() => {});
loadAccounts().catch((e) => toast(String(e)));
