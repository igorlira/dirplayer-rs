// File dialogs for the FileIO Xtra's displayOpen and displaySave.
//
// A projector shows the operating system's file dialog and the movie waits
// for the answer. vm-rust awaits `showFileDialog(request)` the same way, and
// this module puts a small dialog over the page:
//
//   open: the files the virtual filesystem holds that pass the movie's filter
//         mask, newest first, and a button that opens a file from the
//         player's own computer (File System Access API where the browser has
//         it, a plain <input type=file> where not). Such a file is handed back
//         with its bytes, and vm-rust stores it in the virtual filesystem.
//   save: a file name, prefilled with the movie's suggestion. The file always
//         goes to the virtual filesystem (the browser); the player can also
//         pick a file on the computer to write it to. When the movie has
//         written and closed the file, vm-rust calls
//         `onFileDialogSaveWritten(name, bytes)`, and the page writes that
//         file (if one was picked) and offers the bytes as a download.
//
// Host options, read each time a dialog opens, so a host script may set them
// before or after the engine loads:
//
//   window.dirplayerFileDialog = {
//     language: 'da' | () => 'da',   // default: <html lang>, then the browser
//     strings: { da: { open: '...' } },  // override any text below
//     usePickers: false,             // never use the File System Access API
//   };
//
// While a dialog is up, <html> carries data-dirplayer-file-dialog, so a host
// can tell a dialog (or the OS file picker it opened) from the player leaving.

const STRINGS = {
  en: {
    openTitle: 'Open',
    saveTitle: 'Save',
    savedHere: 'Saved in this browser',
    noFiles: 'There are no saved files in this browser yet.',
    fromComputerHint: 'A file saved elsewhere, for example by the original program, can be opened from this computer.',
    openFromComputer: 'Open a file from this computer...',
    open: 'Open',
    cancel: 'Cancel',
    fileName: 'File name',
    save: 'Save',
    saveToComputer: 'Save to a file on this computer...',
    saveToKnownFile: 'Save, also to {file} on this computer',
    saveAndDownload: 'Save and download a copy',
    savedToast: 'Saved in this browser as {file}.',
    savedToFileToast: 'Saved as {file}, in this browser and on this computer.',
    writeFailedToast: 'Saved in this browser as {file}, but writing the file on this computer failed.',
    download: 'Download a copy',
    close: 'Close',
    readFailed: 'The file could not be read.',
  },
  da: {
    openTitle: 'Åbn',
    saveTitle: 'Gem',
    savedHere: 'Gemt i denne browser',
    noFiles: 'Der er endnu ingen gemte filer i denne browser.',
    fromComputerHint: 'En fil, der er gemt et andet sted, for eksempel af det oprindelige program, kan åbnes fra denne computer.',
    openFromComputer: 'Åbn en fil fra denne computer...',
    open: 'Åbn',
    cancel: 'Annuller',
    fileName: 'Filnavn',
    save: 'Gem',
    saveToComputer: 'Gem i en fil på denne computer...',
    saveToKnownFile: 'Gem, også i {file} på denne computer',
    saveAndDownload: 'Gem og hent en kopi',
    savedToast: 'Gemt i denne browser som {file}.',
    savedToFileToast: 'Gemt som {file} i denne browser og på denne computer.',
    writeFailedToast: 'Gemt i denne browser som {file}, men filen på denne computer kunne ikke skrives.',
    download: 'Hent en kopi',
    close: 'Luk',
    readFailed: 'Filen kunne ikke læses.',
  },
  sv: {
    openTitle: 'Öppna',
    saveTitle: 'Spara',
    savedHere: 'Sparat i den här webbläsaren',
    noFiles: 'Det finns inga sparade filer i den här webbläsaren än.',
    fromComputerHint: 'En fil som sparats någon annanstans, till exempel av originalprogrammet, kan öppnas från den här datorn.',
    openFromComputer: 'Öppna en fil från den här datorn...',
    open: 'Öppna',
    cancel: 'Avbryt',
    fileName: 'Filnamn',
    save: 'Spara',
    saveToComputer: 'Spara i en fil på den här datorn...',
    saveToKnownFile: 'Spara, även i {file} på den här datorn',
    saveAndDownload: 'Spara och ladda ned en kopia',
    savedToast: 'Sparat i den här webbläsaren som {file}.',
    savedToFileToast: 'Sparat som {file} i den här webbläsaren och på den här datorn.',
    writeFailedToast: 'Sparat i den här webbläsaren som {file}, men filen på den här datorn kunde inte skrivas.',
    download: 'Ladda ned en kopia',
    close: 'Stäng',
    readFailed: 'Filen kunde inte läsas.',
  },
  nb: {
    openTitle: 'Åpne',
    saveTitle: 'Lagre',
    savedHere: 'Lagret i denne nettleseren',
    noFiles: 'Det er ingen lagrede filer i denne nettleseren ennå.',
    fromComputerHint: 'En fil som er lagret et annet sted, for eksempel av det opprinnelige programmet, kan åpnes fra denne datamaskinen.',
    openFromComputer: 'Åpne en fil fra denne datamaskinen...',
    open: 'Åpne',
    cancel: 'Avbryt',
    fileName: 'Filnavn',
    save: 'Lagre',
    saveToComputer: 'Lagre i en fil på denne datamaskinen...',
    saveToKnownFile: 'Lagre, også i {file} på denne datamaskinen',
    saveAndDownload: 'Lagre og last ned en kopi',
    savedToast: 'Lagret i denne nettleseren som {file}.',
    savedToFileToast: 'Lagret som {file} i denne nettleseren og på denne datamaskinen.',
    writeFailedToast: 'Lagret i denne nettleseren som {file}, men filen på denne datamaskinen kunne ikke skrives.',
    download: 'Last ned en kopi',
    close: 'Lukk',
    readFailed: 'Filen kunne ikke leses.',
  },
};

let languageSetting = null;

/** Set the dialog language (a BCP 47 tag, or a function returning one). */
export function setFileDialogLanguage(lang) {
  languageSetting = lang;
}

function options() {
  const o = typeof window !== 'undefined' ? window.dirplayerFileDialog : null;
  return o && typeof o === 'object' ? o : {};
}

function languageTag() {
  let v = languageSetting != null ? languageSetting : options().language;
  if (typeof v === 'function') {
    try { v = v(); } catch (e) { v = null; }
  }
  if (!v) v = document.documentElement.lang || navigator.language || 'en';
  return String(v);
}

function tableFor(tag) {
  const primary = tag.toLowerCase().replace('_', '-').split('-')[0];
  if (primary === 'da' || primary === 'sv') return primary;
  if (primary === 'nb' || primary === 'no' || primary === 'nn') return 'nb';
  return 'en';
}

function strings() {
  const tag = languageTag();
  const key = tableFor(tag);
  const custom = (options().strings || {})[key] || {};
  const table = Object.assign({}, STRINGS.en, STRINGS[key], custom);
  return {
    tag,
    t(id, vars) {
      let s = table[id] || id;
      if (vars) for (const k in vars) s = s.split('{' + k + '}').join(vars[k]);
      return s;
    },
  };
}

// ── Files on the player's computer ────────────────────────────────────

function pickersAvailable(kind) {
  if (options().usePickers === false) return false;
  const fn = kind === 'open' ? 'showOpenFilePicker' : 'showSaveFilePicker';
  return typeof window[fn] === 'function';
}

function pickerTypes(extensions) {
  if (!extensions || !extensions.length) return undefined;
  const exts = extensions.map((e) => '.' + e.toLowerCase());
  return [{ description: extensions.map((e) => e.toUpperCase()).join(', '), accept: { 'application/octet-stream': exts } }];
}

// File handles picked this session, by lowercased file name, so the next save
// of the same file can write it again. Also kept in IndexedDB where the
// browser allows it; permission is asked for again before any write.
const handles = new Map();
const HANDLE_DB = 'dirplayer-file-handles';

function openHandleDb() {
  return new Promise((resolve) => {
    try {
      const req = indexedDB.open(HANDLE_DB, 1);
      req.onupgradeneeded = () => req.result.createObjectStore('handles');
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => resolve(null);
    } catch (e) {
      resolve(null);
    }
  });
}

async function rememberHandle(name, handle) {
  handles.set(name.toLowerCase(), handle);
  const db = await openHandleDb();
  if (!db) return;
  try {
    db.transaction('handles', 'readwrite').objectStore('handles').put(handle, name.toLowerCase());
  } catch (e) { /* not cloneable here: the in-memory copy is enough */ }
}

async function knownHandle(name) {
  const key = name.toLowerCase();
  if (handles.has(key)) return handles.get(key);
  const db = await openHandleDb();
  if (!db) return null;
  return new Promise((resolve) => {
    try {
      const req = db.transaction('handles').objectStore('handles').get(key);
      req.onsuccess = () => {
        if (req.result) handles.set(key, req.result);
        resolve(req.result || null);
      };
      req.onerror = () => resolve(null);
    } catch (e) {
      resolve(null);
    }
  });
}

async function mayWrite(handle) {
  try {
    const q = { mode: 'readwrite' };
    if (handle.queryPermission && (await handle.queryPermission(q)) === 'granted') return true;
    return handle.requestPermission ? (await handle.requestPermission(q)) === 'granted' : true;
  } catch (e) {
    return false;
  }
}

function downloadBytes(name, bytes) {
  const url = URL.createObjectURL(new Blob([bytes], { type: 'application/octet-stream' }));
  const a = document.createElement('a');
  a.href = url;
  a.download = name;
  a.style.display = 'none';
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 60000);
}

// ── Look ──────────────────────────────────────────────────────────────

const CSS = `
:host { all: initial; }
* { box-sizing: border-box; }
.backdrop { position: fixed; inset: 0; z-index: 2147483000; display: flex;
  align-items: center; justify-content: center; padding: 16px;
  background: rgba(0, 0, 0, 0.55);
  font: 400 17px/1.4 system-ui, -apple-system, "Segoe UI", Roboto, sans-serif; color: #f3f3f3; }
.panel { width: min(100%, 520px); max-height: calc(100% - 16px); display: flex; flex-direction: column;
  gap: 14px; padding: 20px; border-radius: 12px; background: #202227; overflow-y: auto;
  border: 1px solid rgba(255, 255, 255, 0.16); box-shadow: 0 12px 40px rgba(0, 0, 0, 0.6); }
h2 { margin: 0; font-size: 22px; font-weight: 600; }
.label { font-size: 14px; opacity: 0.75; margin: 0; }
.list { list-style: none; margin: 0; padding: 0; overflow-y: auto; min-height: 52px; flex-shrink: 0; max-height: 40vh;
  border: 1px solid rgba(255, 255, 255, 0.14); border-radius: 8px; background: #15171b; }
.list li { display: flex; justify-content: space-between; align-items: center; gap: 12px;
  min-height: 52px; padding: 8px 14px; cursor: pointer; user-select: none;
  border-bottom: 1px solid rgba(255, 255, 255, 0.07); }
.list li:last-child { border-bottom: 0; }
.list li:hover { background: #2a2d34; }
.list li[aria-selected="true"] { background: #2f5fb3; }
.list .name { font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.list .when { font-size: 14px; opacity: 0.8; white-space: nowrap; }
.empty { margin: 0; padding: 16px; border-radius: 8px; background: #15171b; text-align: center; }
.hint { margin: 0; font-size: 14px; opacity: 0.75; }
.error { margin: 0; color: #ff9d9d; }
input[type="text"] { width: 100%; min-height: 48px; padding: 8px 12px; font: inherit; color: #fff;
  background: #15171b; border: 1px solid rgba(255, 255, 255, 0.3); border-radius: 8px; }
input[type="text"]:focus { outline: 2px solid #6ea0ff; outline-offset: 1px; }
button { min-height: 48px; padding: 10px 18px; font: inherit; color: #fff; cursor: pointer;
  background: #353a44; border: 1px solid rgba(255, 255, 255, 0.2); border-radius: 8px; }
button:hover { background: #414755; }
button:focus-visible, .list:focus-visible { outline: 2px solid #6ea0ff; outline-offset: 2px; }
button.primary { background: #2f6fe0; border-color: #2f6fe0; font-weight: 600; }
button.primary:hover { background: #3b7cf0; }
button:disabled { opacity: 0.45; cursor: default; }
.wide { width: 100%; }
.row { display: flex; gap: 10px; justify-content: flex-end; flex-wrap: wrap; }
.row button { flex: 1 1 140px; }
/* A phone on its side: little height, so less air and a smaller title. */
@media (max-height: 460px) {
  .backdrop { padding: 8px; }
  .panel { gap: 8px; padding: 12px 16px; }
  h2 { font-size: 18px; }
  .empty { padding: 10px; }
  .list { max-height: 35vh; }
}
.file-input { position: absolute; width: 1px; height: 1px; opacity: 0; pointer-events: none; }
.toast { position: fixed; left: 50%; bottom: 16px; transform: translateX(-50%); z-index: 2147483000;
  width: max-content; max-width: calc(100% - 32px); display: flex; align-items: center; gap: 12px; flex-wrap: wrap;
  padding: 10px 12px 10px 16px; border-radius: 10px; background: #202227; color: #f3f3f3;
  border: 1px solid rgba(255, 255, 255, 0.16); box-shadow: 0 8px 28px rgba(0, 0, 0, 0.55);
  font: 400 16px/1.4 system-ui, -apple-system, "Segoe UI", Roboto, sans-serif; }
.toast button { min-height: 44px; padding: 8px 14px; }
`;

function el(tag, attrs, children) {
  const e = document.createElement(tag);
  for (const k in attrs || {}) {
    if (k === 'text') e.textContent = attrs[k];
    else if (k.startsWith('on')) e.addEventListener(k.slice(2), attrs[k]);
    else e.setAttribute(k, attrs[k]);
  }
  for (const c of children || []) if (c) e.appendChild(c);
  return e;
}

function mountShadow(className) {
  const host = document.createElement('div');
  host.className = className;
  const shadow = host.attachShadow({ mode: 'open' });
  shadow.appendChild(el('style', { text: CSS }));
  document.body.appendChild(host);
  return { host, shadow };
}

// ── The dialog ────────────────────────────────────────────────────────

let pendingSave = null; // { name, handle, download } for the save in progress

function formatWhen(ms, tag) {
  if (!ms || ms < 1e11) return '';
  try {
    return new Intl.DateTimeFormat(tag, { dateStyle: 'medium', timeStyle: 'short' }).format(new Date(ms));
  } catch (e) {
    return new Date(ms).toLocaleString();
  }
}

function cleanName(name) {
  return String(name || '').replace(/[\\/:*?"<>|\u0000-\u001f]/g, '').trim();
}

function withExtension(name, request) {
  const exts = request.extensions || [];
  if (!name || /\.[^.]+$/.test(name) || exts.length !== 1) return name;
  // Spell the extension the way the movie's suggestion does, if it has it.
  const m = /\.([^.]+)$/.exec(request.defaultName || '');
  const ext = m && m[1].toLowerCase() === exts[0].toLowerCase() ? m[1] : exts[0];
  return name + '.' + ext;
}

/**
 * Called by vm-rust. Resolves with null (cancelled) or `{ name, bytes? }`.
 * Never rejects.
 */
export function showFileDialog(request) {
  return new Promise((resolve) => {
    try {
      buildDialog(request || {}, resolve);
    } catch (e) {
      console.warn('[dirplayer] file dialog failed', e);
      resolve(null);
    }
  });
}

function buildDialog(request, resolve) {
  const { tag, t } = strings();
  const isOpen = request.kind === 'open';
  const { host, shadow } = mountShadow('dirplayer-file-dialog');
  const previousFocus = document.activeElement;
  document.documentElement.setAttribute('data-dirplayer-file-dialog', request.kind || 'open');
  if (document.pointerLockElement && document.exitPointerLock) document.exitPointerLock();

  let done = false;
  function finish(answer) {
    if (done) return;
    done = true;
    host.remove();
    document.documentElement.removeAttribute('data-dirplayer-file-dialog');
    try { if (previousFocus && previousFocus.focus) previousFocus.focus({ preventScroll: true }); } catch (e) {}
    resolve(answer);
  }

  const titleId = 'dirplayer-file-dialog-title';
  const panel = el('div', { class: 'panel', role: 'dialog', 'aria-modal': 'true', 'aria-labelledby': titleId });
  const backdrop = el('div', { class: 'backdrop' }, [panel]);
  shadow.appendChild(backdrop);
  const title = request.title && String(request.title).trim() ? String(request.title) : t(isOpen ? 'openTitle' : 'saveTitle');
  panel.appendChild(el('h2', { id: titleId, text: title }));
  const errorLine = el('p', { class: 'error', role: 'alert', hidden: '' });
  const showError = (msg) => { errorLine.textContent = msg; errorLine.hidden = false; };

  let onEnter = () => {};
  if (isOpen) onEnter = buildOpen(request, panel, t, tag, finish, showError);
  else onEnter = buildSave(request, panel, t, finish, showError);
  panel.appendChild(errorLine);

  // Keys stay in the dialog: the page and the movie behind it do not see
  // them while it is up.
  const stop = (e) => e.stopPropagation();
  backdrop.addEventListener('keyup', stop);
  backdrop.addEventListener('keypress', stop);
  backdrop.addEventListener('keydown', (e) => {
    e.stopPropagation();
    if (e.key === 'Escape') {
      e.preventDefault();
      finish(null);
    } else if (e.key === 'Enter' && !(e.target && e.target.tagName === 'BUTTON')) {
      e.preventDefault();
      onEnter();
    } else if (e.key === 'Tab') {
      const items = Array.from(panel.querySelectorAll('button:not([disabled]), input[type="text"], [tabindex="0"]'));
      if (!items.length) return;
      const i = items.indexOf(shadow.activeElement);
      const next = e.shiftKey ? (i <= 0 ? items.length - 1 : i - 1) : (i === items.length - 1 ? 0 : i + 1);
      e.preventDefault();
      items[next].focus();
    }
  });
  for (const type of ['pointerdown', 'pointerup', 'mousedown', 'mouseup', 'click', 'touchstart', 'touchend', 'wheel']) {
    backdrop.addEventListener(type, stop, { passive: true });
  }
}

function buildOpen(request, panel, t, tag, finish, showError) {
  const files = Array.isArray(request.files) ? request.files : [];
  let selected = files.length ? 0 : -1;
  const openButton = el('button', { class: 'primary', type: 'button', text: t('open') });
  const choose = () => { if (selected >= 0) finish({ name: files[selected].name }); };
  openButton.addEventListener('click', choose);

  let list = null;
  if (files.length) {
    panel.appendChild(el('p', { class: 'label', text: t('savedHere') }));
    list = el('ul', { class: 'list', role: 'listbox', tabindex: '0', 'aria-label': t('savedHere') });
    const rows = files.map((f, i) => {
      const li = el('li', { role: 'option', 'aria-selected': String(i === selected) }, [
        el('span', { class: 'name', text: f.name }),
        el('span', { class: 'when', text: formatWhen(f.modified, tag) }),
      ]);
      li.addEventListener('click', () => select(i));
      li.addEventListener('dblclick', choose);
      list.appendChild(li);
      return li;
    });
    const select = (i) => {
      selected = Math.max(0, Math.min(files.length - 1, i));
      rows.forEach((r, j) => r.setAttribute('aria-selected', String(j === selected)));
      rows[selected].scrollIntoView({ block: 'nearest' });
    };
    list.addEventListener('keydown', (e) => {
      if (e.key === 'ArrowDown') { e.preventDefault(); select(selected + 1); }
      else if (e.key === 'ArrowUp') { e.preventDefault(); select(selected - 1); }
      else if (e.key === 'Home') { e.preventDefault(); select(0); }
      else if (e.key === 'End') { e.preventDefault(); select(files.length - 1); }
    });
    panel.appendChild(list);
  } else {
    openButton.disabled = true;
    panel.appendChild(el('p', { class: 'empty', text: t('noFiles') }));
  }

  panel.appendChild(el('p', { class: 'hint', text: t('fromComputerHint') }));
  const accept = (request.extensions || []).map((e) => '.' + e.toLowerCase()).join(',');
  const input = el('input', { type: 'file', class: 'file-input', tabindex: '-1', 'aria-hidden': 'true' });
  if (accept) input.setAttribute('accept', accept);
  const takeFile = async (file, handle) => {
    try {
      const bytes = new Uint8Array(await file.arrayBuffer());
      if (handle) rememberHandle(file.name, handle);
      finish({ name: file.name, bytes });
    } catch (e) {
      showError(t('readFailed'));
    }
  };
  input.addEventListener('change', () => {
    const file = input.files && input.files[0];
    if (file) takeFile(file, null);
  });
  const fromComputer = el('button', { class: 'wide', type: 'button', text: t('openFromComputer') });
  fromComputer.addEventListener('click', async () => {
    if (!pickersAvailable('open')) {
      input.value = '';
      input.click();
      return;
    }
    try {
      const [handle] = await window.showOpenFilePicker({ types: pickerTypes(request.extensions), excludeAcceptAllOption: false, multiple: false });
      await takeFile(await handle.getFile(), handle);
    } catch (e) {
      if (e && e.name !== 'AbortError') showError(t('readFailed'));
    }
  });
  panel.appendChild(input);
  panel.appendChild(fromComputer);

  const cancel = el('button', { type: 'button', text: t('cancel') });
  cancel.addEventListener('click', () => finish(null));
  panel.appendChild(el('div', { class: 'row' }, [cancel, openButton]));
  (list || fromComputer).focus();
  return choose;
}

function buildSave(request, panel, t, finish, showError) {
  const inputId = 'dirplayer-file-dialog-name';
  panel.appendChild(el('label', { class: 'label', for: inputId, text: t('fileName') }));
  const nameInput = el('input', { type: 'text', id: inputId, autocomplete: 'off', spellcheck: 'false' });
  nameInput.value = request.defaultName || '';
  panel.appendChild(nameInput);

  const saveButton = el('button', { class: 'primary', type: 'button', text: t('save') });
  const chosenName = () => withExtension(cleanName(nameInput.value), request);
  const saveHere = () => {
    const name = chosenName();
    if (!name) return;
    pendingSave = { name, handle: null, download: false };
    finish({ name });
  };
  saveButton.addEventListener('click', saveHere);
  const refresh = () => { saveButton.disabled = !chosenName(); };
  nameInput.addEventListener('input', refresh);
  refresh();

  // Where the browser can write files, the player can pick one, and the next
  // save of the same name offers that file again. Elsewhere: a download.
  const second = el('button', { class: 'wide', type: 'button' });
  let known = null;
  const label = async () => {
    if (!pickersAvailable('save')) { second.textContent = t('saveAndDownload'); return; }
    second.textContent = t('saveToComputer');
    const name = chosenName();
    known = name ? await knownHandle(name) : null;
    if (known && chosenName() === name) second.textContent = t('saveToKnownFile', { file: known.name });
  };
  label();
  nameInput.addEventListener('input', () => { label(); });
  second.addEventListener('click', async () => {
    const name = chosenName();
    if (!name) return;
    if (!pickersAvailable('save')) {
      pendingSave = { name, handle: null, download: true };
      finish({ name });
      return;
    }
    try {
      let handle = known && (await mayWrite(known)) ? known : null;
      if (!handle) {
        handle = await window.showSaveFilePicker({ suggestedName: name, types: pickerTypes(request.extensions) });
      }
      rememberHandle(name, handle);
      pendingSave = { name, handle, download: false };
      finish({ name });
    } catch (e) {
      if (e && e.name !== 'AbortError') showError(String(e.message || e));
    }
  });
  panel.appendChild(second);

  const cancel = el('button', { type: 'button', text: t('cancel') });
  cancel.addEventListener('click', () => { pendingSave = null; finish(null); });
  panel.appendChild(el('div', { class: 'row' }, [cancel, saveButton]));
  nameInput.focus();
  nameInput.select();
  return saveHere;
}

// ── After the movie has written the file ──────────────────────────────

let toastHost = null;

function showToast(message, bytes, name) {
  const { t } = strings();
  if (toastHost) toastHost.remove();
  const { host, shadow } = mountShadow('dirplayer-file-toast');
  toastHost = host;
  const close = () => { if (toastHost === host) toastHost = null; host.remove(); };
  const box = el('div', { class: 'toast', role: 'status' }, [el('span', { text: message })]);
  if (bytes) {
    const dl = el('button', { type: 'button', class: 'primary', text: t('download') });
    dl.addEventListener('click', () => downloadBytes(name, bytes));
    box.appendChild(dl);
  }
  const x = el('button', { type: 'button', text: t('close'), 'aria-label': t('close') });
  x.addEventListener('click', close);
  box.appendChild(x);
  for (const type of ['pointerdown', 'pointerup', 'mousedown', 'mouseup', 'click', 'touchstart', 'touchend', 'keydown', 'keyup']) {
    box.addEventListener(type, (e) => e.stopPropagation());
  }
  shadow.appendChild(box);
  setTimeout(close, 20000);
}

/**
 * Called by vm-rust when the file a displaySave answer named has been
 * written and closed. Returns at once; the writing happens afterwards.
 */
export function onFileDialogSaveWritten(name, data) {
  // `data` is a view into wasm memory, valid only during this call.
  const bytes = new Uint8Array(data);
  const target = pendingSave;
  pendingSave = null;
  const { t } = strings();
  (async () => {
    if (target && target.handle) {
      try {
        const w = await target.handle.createWritable();
        await w.write(bytes);
        await w.close();
        showToast(t('savedToFileToast', { file: target.handle.name || name }), bytes, name);
      } catch (e) {
        console.warn('[dirplayer] writing the picked file failed', e);
        showToast(t('writeFailedToast', { file: name }), bytes, name);
      }
      return;
    }
    if (target && target.download) downloadBytes(name, bytes);
    showToast(t('savedToast', { file: name }), bytes, name);
  })();
}
