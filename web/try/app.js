// The Strongtalk desktop: Launcher with Transcript, workspaces, class hierarchy, outliner
// class browsers, inspectors, error windows and picture windows, all talking to the Rust VM
// running in a Web Worker.

import { attachHighlighter, highlightTranslation, bindFolds } from '../highlight.js';

const CHANGES_KEY = 'strongtalk.changes.v1';
const desktop = document.getElementById('desktop');
const tasksEl = document.getElementById('tasks');
const statusEl = document.getElementById('vmstatus');
const phone = () => window.matchMedia('(max-width: 720px)').matches;

// ------------------------------------------------------------------ the VM connection

class VMClient {
  constructor() {
    this.pending = new Map();
    this.nextId = 1;
    this.busy = 0;
    this.onTranscript = () => {};
    this.start();
  }
  start() {
    this.worker = new Worker(new URL('./worker.js', import.meta.url), { type: 'module' });
    this.worker.onmessage = (ev) => this.receive(ev.data);
    this.worker.onerror = (ev) => {
      setStatus('dead', 'VM failed to start');
      transcript.error('The virtual machine could not start: ' + (ev.message || 'unknown error') + '\n');
    };
  }
  receive(m) {
    if (m.type === 'transcript') { this.onTranscript(m.text); return; }
    const p = this.pending.get(m.id);
    if (!p) return;
    this.pending.delete(m.id);
    this.busy = Math.max(0, this.busy - 1);
    if (!this.busy) setStatus('ready', 'ready');
    if (m.changes) saveChanges(m.changes);
    if (m.fatal) {
      transcript.error(`\nThe virtual machine stopped (${m.error}). This happens when a computation recurses deeper than the browser allows. Restarting it; your classes and methods are kept.\n`);
      p.resolve({ ok: false, error: 'The computation recursed too deeply for the browser (' + m.error + ')', errorClass: 'Error', stack: [], restarted: true });
      this.interrupt();
      return;
    }
    if (m.error) { p.reject(new Error(m.error)); return; }
    const r = m.result;
    if (r && typeof r === 'object' && !Array.isArray(r)) r._canvases = m.canvases;
    p.resolve(r);
  }
  call(op, ...args) {
    const id = this.nextId++;
    this.busy++;
    setStatus('busy', 'running…');
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject, op });
      this.worker.postMessage({ id, op, args });
    });
  }
  async interrupt() {
    this.worker.terminate();
    for (const p of this.pending.values()) p.reject(new Error('interrupted'));
    this.pending.clear();
    this.busy = 0;
    this.start();
    await boot(true);
  }
}

function setStatus(kind, text) {
  statusEl.className = 'vmstatus ' + kind;
  statusEl.querySelector('.label').textContent = 'VM: ' + text;
  document.querySelectorAll('.licon.stop').forEach((b) => (b.style.visibility = kind === 'busy' ? 'visible' : 'hidden'));
  const stop = document.getElementById('stopbtn');
  if (stop) stop.hidden = kind !== 'busy';
}

function loadChanges() {
  try { return localStorage.getItem(CHANGES_KEY) || ''; } catch { return ''; }
}
function saveChanges(text) {
  try { localStorage.setItem(CHANGES_KEY, loadChanges() + text); } catch { /* storage full or blocked */ }
}

const vm = new VMClient();

// ------------------------------------------------------------------ window manager

let zTop = 10;
const windows = new Set();
let cascade = 0;

function el(tag, attrs = {}, ...kids) {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === 'class') e.className = v;
    else if (k === 'text') e.textContent = v;
    else if (k.startsWith('on')) e.addEventListener(k.slice(2), v);
    else if (v !== undefined && v !== null && v !== false) e.setAttribute(k, v === true ? '' : v);
  }
  for (const k of kids.flat()) if (k != null) e.append(k instanceof Node ? k : document.createTextNode(String(k)));
  return e;
}

function placeFor(w, h) {
  const dw = desktop.clientWidth, dh = desktop.clientHeight;
  const x = Math.max(4, Math.min(dw - w - 4, 360 + (cascade % 8) * 28));
  const y = Math.max(4, Math.min(dh - h - 4, 60 + (cascade % 8) * 26));
  cascade++;
  return [x, y];
}

function createWindow({ title, width = 560, height = 400, x, y, kind = 'win', tall = false, onClose }) {
  const dw = desktop.clientWidth, dh = desktop.clientHeight;
  width = Math.min(width, dw - 8);
  height = Math.min(height, dh - 8);
  if (x === undefined || y === undefined) [x, y] = placeFor(width, height);
  const titleText = el('span', { class: 'ttext', text: title });
  const win = el('section', { class: 'win' + (tall ? ' tall' : ''), 'data-kind': kind, role: 'dialog', 'aria-label': title });
  const bar = el('div', { class: 'titlebar' },
    el('span', { class: 'ticon', text: 'S' }),
    titleText,
    el('button', { class: 'mx', title: 'Maximize', text: '□', onclick: () => win.classList.toggle('maximized') }),
    el('button', { title: 'Close', text: '✕', onclick: () => api.close() }));
  const body = el('div', { class: 'body' });
  const grip = el('div', { class: 'grip' });
  win.append(bar, body, grip);
  Object.assign(win.style, { left: x + 'px', top: y + 'px', width: width + 'px', height: height + 'px' });
  const task = el('button', { text: title, onclick: () => api.focus() });
  const api = {
    el: win, body, kind,
    setTitle(t) { titleText.textContent = t; task.textContent = t; win.setAttribute('aria-label', t); },
    focus() {
      for (const w of windows) { w.el.classList.remove('active'); w.task.classList.remove('on'); }
      win.classList.add('active'); task.classList.add('on');
      win.style.zIndex = ++zTop;
      if (phone()) win.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
    },
    close() {
      if (onClose && onClose() === false) return;
      win.remove(); task.remove(); windows.delete(api);
    },
    task,
  };
  win.addEventListener('mousedown', () => api.focus(), true);
  dragger(bar, (dx, dy, s) => { win.style.left = s.left + dx + 'px'; win.style.top = Math.max(0, s.top + dy) + 'px'; },
    () => ({ left: win.offsetLeft, top: win.offsetTop }));
  dragger(grip, (dx, dy, s) => { win.style.width = Math.max(220, s.w + dx) + 'px'; win.style.height = Math.max(120, s.h + dy) + 'px'; },
    () => ({ w: win.offsetWidth, h: win.offsetHeight }));
  bar.addEventListener('dblclick', () => win.classList.toggle('maximized'));
  desktop.append(win);
  tasksEl.append(task);
  windows.add(api);
  api.focus();
  return api;
}

function dragger(handle, move, start) {
  handle.addEventListener('pointerdown', (ev) => {
    if (ev.target.tagName === 'BUTTON' || phone()) return;
    ev.preventDefault();
    const sx = ev.clientX, sy = ev.clientY, s = start();
    const mm = (e) => move(e.clientX - sx, e.clientY - sy, s);
    const up = () => { window.removeEventListener('pointermove', mm); window.removeEventListener('pointerup', up); };
    window.addEventListener('pointermove', mm);
    window.addEventListener('pointerup', up);
  });
}

// ------------------------------------------------------------------ menus

let openMenu = null;
function closeMenus() { if (openMenu) { openMenu.remove(); openMenu = null; } document.querySelectorAll('.mitem.open').forEach((m) => m.classList.remove('open')); }
document.addEventListener('mousedown', (ev) => { if (openMenu && !openMenu.contains(ev.target) && !ev.target.classList.contains('mitem')) closeMenus(); });

function showMenu(items, x, y, owner) {
  closeMenus();
  const m = el('div', { class: 'menu', role: 'menu' });
  for (const it of items) {
    if (it === '-') { m.append(el('div', { class: 'msep' })); continue; }
    m.append(el('div', { class: 'mrow', role: 'menuitem', onclick: () => { closeMenus(); it[1](); } },
      el('span', { text: it[0] }), el('span', { class: 'key', text: it[2] || '' })));
  }
  document.body.append(m);
  const r = m.getBoundingClientRect();
  m.style.left = Math.max(0, Math.min(x, window.innerWidth - r.width - 2)) + 'px';
  m.style.top = Math.max(0, Math.min(y, window.innerHeight - r.height - 2)) + 'px';
  openMenu = m;
  if (owner) owner.classList.add('open');
}

function menubar(spec) {
  const bar = el('div', { class: 'menubar', role: 'menubar' });
  for (const [name, items] of spec) {
    const mi = el('div', { class: 'mitem', role: 'menuitem', text: name });
    mi.addEventListener('mousedown', (ev) => {
      ev.preventDefault();
      if (mi.classList.contains('open')) { closeMenus(); return; }
      const r = mi.getBoundingClientRect();
      showMenu(typeof items === 'function' ? items() : items, r.left, r.bottom, mi);
    });
    mi.addEventListener('mouseenter', () => {
      if (openMenu && !mi.classList.contains('open')) {
        const r = mi.getBoundingClientRect();
        showMenu(typeof items === 'function' ? items() : items, r.left, r.bottom, mi);
      }
    });
    bar.append(mi);
  }
  return bar;
}

// ------------------------------------------------------------------ transcript (in the Launcher)

const transcript = {
  pre: null,
  pending: '',
  write(text, cls) {
    if (!this.pre) { this.pending += text; return; }
    if (cls) this.pre.append(el('span', { class: cls, text }));
    else {
      const last = this.pre.lastChild;
      if (last && last.nodeType === 3) last.appendData(text); else this.pre.append(document.createTextNode(text));
    }
    if (this.pre.childNodes.length > 4000) this.pre.firstChild.remove();
    this.pre.scrollTop = this.pre.scrollHeight;
  },
  sys(text) { this.write(text, 'sys'); },
  error(text) { this.write(text, 'err'); },
  clear() { if (this.pre) this.pre.textContent = ''; },
};
vm.onTranscript = (t) => transcript.write(t);

// ------------------------------------------------------------------ results of an evaluation

function handleReply(r, opts = {}) {
  if (!r) return r;
  if (r.transcript) transcript.write(r.transcript);
  for (const e of r.events || []) {
    if (e.type === 'inspect') openInspector(e.id);
    else if (e.type === 'clear') transcript.clear();
    else if (e.type === 'browse') openBrowser(e.name);
    else if (e.type === 'canvas') {
      const c = (r._canvases || []).find((x) => x.id === e.id);
      if (c) showCanvas(e, c.pixels);
    }
  }
  if (r.ok === false && r.error && !opts.quiet && r.errorClass !== 'SyntaxError') openError(r, opts.source);
  return r;
}

async function evaluate(src, ws, print) {
  try {
    return handleReply(await vm.call('eval', src, ws, print), { source: src });
  } catch (e) {
    if (e.message === 'interrupted') return { ok: false, error: 'Interrupted', interrupted: true };
    transcript.error('VM error: ' + e.message + '\n');
    return { ok: false, error: e.message };
  }
}

// ------------------------------------------------------------------ Launcher

function openLauncher() {
  const w = createWindow({ title: 'Strongtalk Launcher', width: 560, height: 330, x: 12, y: 12, kind: 'launcher' });
  w.close = () => {}; // the Launcher stays open, as in the original
  w.el.querySelector('.titlebar button[title=Close]').disabled = true;
  const bar = menubar([
    ['File', [
      ['File in…', () => document.getElementById('filepick').click()],
      ['Download my changes', downloadChanges],
      '-',
      ['Reset to a fresh image…', resetImage],
    ]],
    ['Browse', [
      ['Class hierarchy', () => openHierarchy()],
      ['Find class…', findClass, 'Ctrl+B'],
      '-',
      ['Senders of…', () => askText('Senders of', 'Selector, for example printOn:', '').then((s) => s && openSenders(s.trim()))],
      ['Implementors of…', () => askText('Implementors of', 'Selector, for example printOn:', '').then((s) => s && openImplementors(s.trim()))],
    ]],
    ['Tools', [
      ['New workspace', () => openWorkspace()],
      ['Type-check all classes', typecheckAll],
      ['Translate a class to TypeScript…', () => askText('Translate to TypeScript', 'Class name, for example BankAccount', 'BankAccount').then((c) => c && translateClass(c.trim()))],
      ['Open the TypeScript translator page', () => window.open('../translate/', '_blank')],
      ['Run benchmarks', () => evaluate('Benchmark runAll', '', false)],
      '-',
      ['Clear Transcript', () => transcript.clear()],
      ['Interrupt the running program', () => interrupt()],
    ]],
    ['Demos', [
      ['Mandelbrot set', () => evaluate('Mandelbrot show', '', false)],
      ['Mandelbrot in pure Smalltalk', () => evaluate('Mandelbrot showInSmalltalk', '', false)],
      ['Pen: spiral', () => evaluate('Shapes spiral', '', false)],
      ['Pen: flower', () => evaluate('Shapes flower', '', false)],
      ['Pen: tree', () => evaluate('Shapes tree', '', false)],
      '-',
      ['Browse BankAccount (a typed class)', () => openBrowser('BankAccount')],
      ['Type checking tour', () => openWorkspace(TYPE_TOUR, 'Workspace: type checking')],
    ]],
    ['Help', [
      ['Language guide', openGuide],
      ['Articles: history, types, the VM', () => { location.href = '../'; }],
      ['About Strongtalk', openAbout],
    ]],
  ]);
  const icons = el('div', { class: 'launch-icons' },
    el('button', { class: 'licon', title: 'Open a new workspace', onclick: () => openWorkspace() }, el('span', { class: 'gl', text: '✎' }), 'Workspace'),
    el('button', { class: 'licon', title: 'Class hierarchy', onclick: () => openHierarchy() }, el('span', { class: 'gl', text: '≣' }), 'Hierarchy'),
    el('button', { class: 'licon', title: 'Find a class by name', onclick: findClass }, el('span', { class: 'gl', text: '⌕' }), 'Find class'),
    el('button', { class: 'licon', title: 'Type-check every class', onclick: typecheckAll }, el('span', { class: 'gl', text: '✓' }), 'Typecheck'),
    el('button', { class: 'licon', title: 'File in a .dlt source file', onclick: () => document.getElementById('filepick').click() }, el('span', { class: 'gl', text: '⇩' }), 'File in'),
    el('button', { class: 'licon stop', title: 'Stop the running program', onclick: () => interrupt(), style: 'visibility:hidden' }, el('span', { class: 'gl', text: '■' }), 'Interrupt'));
  const pre = el('pre', { class: 'transcript sunken code', 'aria-label': 'Transcript', 'aria-live': 'polite' });
  w.body.append(bar, icons, el('div', { style: 'font-size:11px;padding:0 2px 2px;color:#333', text: 'Transcript' }), pre);
  transcript.pre = pre;
  if (transcript.pending) { transcript.write(transcript.pending); transcript.pending = ''; }
  return w;
}

async function interrupt() {
  if (!vm.busy) { transcript.sys('Nothing is running.\n'); return; }
  transcript.sys('\nInterrupted. Restarting the virtual machine (your classes and methods are kept; workspace variables are reset)…\n');
  await vm.interrupt();
}

function resetImage() {
  if (!confirm('Forget every class and method you defined, and start from the original class library?')) return;
  try { localStorage.removeItem(CHANGES_KEY); } catch {}
  location.reload();
}

function downloadChanges() {
  const text = loadChanges();
  if (!text) { transcript.sys('You have not changed anything yet.\n'); return; }
  download('my-changes.dlt', text);
}

function download(name, text) {
  const a = el('a', { href: URL.createObjectURL(new Blob([text], { type: 'text/plain' })), download: name });
  document.body.append(a); a.click(); a.remove();
}

document.getElementById('filepick').addEventListener('change', async (ev) => {
  const f = ev.target.files[0];
  ev.target.value = '';
  if (!f) return;
  const text = await f.text();
  transcript.sys(`Filing in ${f.name}…\n`);
  const r = handleReply(await vm.call('fileIn', text));
  for (const e of r.errors || []) transcript.error(e + '\n');
  transcript.sys(`${f.name}: ${r.methods} methods, ${r.doits} definitions${r.errors.length ? ', ' + r.errors.length + ' problem(s)' : ''}.\n`);
  refreshBrowsers();
});

// ------------------------------------------------------------------ prompts

function askText(title, label, value) {
  return new Promise((resolve) => {
    const w = createWindow({ title, width: 340, height: 140, kind: 'prompt', onClose: () => resolve(null) });
    const input = el('input', { value: value || '', 'aria-label': label });
    const ok = () => { const v = input.value; w.close = () => {}; w.el.remove(); w.task.remove(); windows.delete(w); resolve(v); };
    input.addEventListener('keydown', (e) => { if (e.key === 'Enter') ok(); if (e.key === 'Escape') w.close(); });
    w.body.append(el('div', { class: 'prompt-body' }, el('label', { text: label }), input,
      el('div', { class: 'toolbar' }, el('button', { class: 'btn', text: 'OK', onclick: ok }), el('button', { class: 'btn', text: 'Cancel', onclick: () => w.close() }))));
    setTimeout(() => input.focus(), 0);
  });
}

async function findClass() {
  const name = await askText('Find class', 'Class name (part of it is enough)', '');
  if (!name) return;
  const classes = await vm.call('classes');
  const q = name.trim().toLowerCase();
  const exact = classes.find((c) => c.name.toLowerCase() === q);
  const hits = exact ? [exact] : classes.filter((c) => c.name.toLowerCase().includes(q));
  if (hits.length === 1) openBrowser(hits[0].name);
  else if (!hits.length) transcript.sys(`No class matches "${name}".\n`);
  else openHierarchy(name.trim());
}

// ------------------------------------------------------------------ workspaces

let wsCount = 0;
const WELCOME = `"Welcome to Strongtalk, the typed Smalltalk.
Put the cursor in an example and press Print it (Ctrl+P),
Do it (Ctrl+D) or Inspect it (Ctrl+I). Select text to run just that."

3 + 4 * 5

100 factorial printString size

(1 to: 10) collect: [:i <Int> | i * i]

#(5 3 9 1 7) asSortedCollection asArray

(1/3) + (1/6)

Transcript show: 'Hello from Strongtalk'; cr

| a <BankAccount> |
a := BankAccount owner: 'Ada'.
a deposit: 100; withdraw: 30.
a

Mandelbrot show

Shapes tree

Benchmark runAll

"Type checking: press Typecheck with the cursor here."
| n <Int> |
n := 'not a number'.
n + 1
`;

const TYPE_TOUR = `"Strongtalk's types are optional and never change how code runs.
The type checker reads the annotations and reports mistakes before you run anything.
Put the cursor in an example and press Typecheck."

"1. A string assigned to an Int variable"
| n <Int> |
n := 'seven'.
n

"2. A message the declared type does not understand"
| s <Str> |
s := 'hello'.
s squared

"3. A wrong argument type: deposit: expects an Int"
(BankAccount owner: 'Ada') deposit: 'lots'

"4. Untyped code is dynamic and is never reported"
| x |
x := 'hello'.
x squared

"5. Declare a typed class, then browse and type-check it"
Delta define: #Temperature as: (
	(Class subclassOf: 'Object' instanceVariables: 'celsius <Float>'))

Temperature compile: 'celsius: c <Number> ^<Self>
	celsius := c asFloat'

Temperature compile: 'fahrenheit ^<Float>
	^celsius * 9 / 5 + 32'

(Temperature new celsius: 21) fahrenheit
`;

function paragraphAt(text, pos) {
  let a = text.lastIndexOf('\n\n', pos - 1);
  a = a < 0 ? 0 : a + 2;
  let b = text.indexOf('\n\n', pos);
  if (b < 0) b = text.length;
  return [a, b];
}

function codeArea(value) {
  const ta = el('textarea', { class: 'code sunken', spellcheck: 'false', autocapitalize: 'off', autocomplete: 'off' });
  ta.value = value || '';
  ta.addEventListener('keydown', (e) => {
    if (e.key === 'Tab' && !e.ctrlKey && !e.metaKey) {
      e.preventDefault();
      const s = ta.selectionStart;
      ta.setRangeText('\t', s, ta.selectionEnd, 'end');
    }
  });
  // syntax highlighting: `ta.mount` is the element to put in the window
  ta.mount = attachHighlighter(ta, 'st');
  return ta;
}

// ------------------------------------------------------------------ TypeScript translation

const looksLikeFileIn = (t) => /Delta define:|methodsFor:/.test(t);

function openTypeScript(title, r, sourceName) {
  if (!r.ok) {
    transcript.error(`Could not translate ${sourceName}: line ${r.line}, column ${r.column}: ${r.error}\n`);
    return;
  }
  const w = createWindow({ title: 'TypeScript: ' + title, width: 680, height: 560, kind: 'typescript', tall: true });
  const pre = el('pre', { class: 'code sunken tsout' });
  pre.innerHTML = highlightTranslation(r.code);
  bindFolds(pre);
  const fileName = (sourceName || 'translation').replace(/[^A-Za-z0-9_-]+/g, '_') + '.ts';
  const status = el('div', { class: 'status' + (r.warnings.length ? ' warn' : ''),
    text: r.warnings.length ? `${r.warnings.length} note(s): ` + r.warnings.join(' · ') : 'Translated with no notes. Runs with node ' + fileName + ' (Node 22+) or any TypeScript compiler.',
    title: r.warnings.join('\n') });
  const tb = el('div', { class: 'toolbar' },
    el('button', { class: 'btn', text: 'Copy', onclick: async () => { try { await navigator.clipboard.writeText(r.code); status.textContent = 'Copied to the clipboard.'; } catch { status.textContent = 'Copying is not allowed here; select the text instead.'; } } }),
    el('button', { class: 'btn', text: 'Download .ts', onclick: () => download(fileName, r.code) }),
    el('span', { class: 'tsep' }),
    el('button', { class: 'btn', text: 'Open in the translator', title: 'The side-by-side translator page, with this code', onclick: () => {
      const b = btoa(String.fromCharCode(...new TextEncoder().encode(r.source || ''))).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
      window.open('../translate/#code=' + b, '_blank');
    } }));
  w.body.append(tb, pre, status);
  return w;
}

async function translateText(text, title) {
  const r = await vm.call('toTypeScript', text);
  r.source = text;
  return openTypeScript(title, r, title);
}

async function translateClass(name) {
  const r = await vm.call('classToTypeScript', name);
  if (r.ok) r.source = await vm.call('fileOut', name);
  return openTypeScript(name, r, name);
}

function selectionOrParagraph(ta) {
  let s = ta.selectionStart, e = ta.selectionEnd;
  if (s === e) [s, e] = paragraphAt(ta.value, s);
  const text = ta.value.slice(s, e);
  // skip leading comment-only lines' whitespace for nicer selection
  return { s, e, text };
}

function openWorkspace(text, title) {
  wsCount++;
  const id = 'ws' + wsCount + '-' + Date.now();
  const w = createWindow({ title: title || `Workspace ${wsCount}`, width: 640, height: 600,
    x: wsCount === 1 ? 584 : undefined, y: wsCount === 1 ? 12 : undefined, kind: 'workspace', tall: true });
  const ta = codeArea(text === undefined ? (wsCount === 1 ? WELCOME : '') : text);
  const status = el('div', { class: 'status', text: 'Ctrl+D do it · Ctrl+P print it · Ctrl+I inspect it · Ctrl+T typecheck · Ctrl+E TypeScript' });
  const setMsg = (t, err) => { status.textContent = t; status.classList.toggle('err', !!err); };

  async function run(mode) {
    const { s, e, text } = selectionOrParagraph(ta);
    if (!text.trim()) return;
    ta.setSelectionRange(s, e);
    setMsg('running…');
    if (mode === 'typecheck') {
      const r = await vm.call('typecheckDoit', text);
      if (!r.ok) { setMsg('Syntax error: ' + r.error, true); return; }
      if (!r.problems.length) { setMsg('Type check passed: no type errors.'); return; }
      setMsg(`${r.problems.length} type error(s): ` + r.problems.map((p) => `line ${p.line}: ${p.message}`).join('; '), true);
      return;
    }
    const r = await evaluate(text, id, mode !== 'do');
    if (r.interrupted) { setMsg('Interrupted.', true); return; }
    if (!r.ok) {
      if (r.errorClass === 'SyntaxError') {
        const at = s + (r.pos || 0);
        const msg = ' ← ' + r.error.replace(/^Syntax error \(line \d+\): /, '') + ' ';
        ta.setRangeText(msg, at, at, 'select');
        ta.focus();
        setMsg(r.error, true);
      } else setMsg(`${r.errorClass}: ${r.error}`, true);
      return;
    }
    setMsg(`${r.class} · ${r.ms < 1 ? '<1' : Math.round(r.ms)} ms`);
    if (mode === 'print') {
      ta.setRangeText(' ' + r.value, e, e, 'select');
      ta.focus();
    } else if (mode === 'inspect') openInspector(r.id);
  }

  ta.addEventListener('keydown', (ev) => {
    if (!(ev.ctrlKey || ev.metaKey)) return;
    const k = ev.key.toLowerCase();
    const map = { d: 'do', p: 'print', i: 'inspect', t: 'typecheck', enter: 'print' };
    if (map[k]) { ev.preventDefault(); run(map[k]); }
    if (k === 'b') { ev.preventDefault(); const t = ta.value.slice(ta.selectionStart, ta.selectionEnd).trim(); t ? openBrowser(t) : findClass(); }
  });
  const tb = el('div', { class: 'toolbar' },
    el('button', { class: 'btn', text: 'Do it', title: 'Run (Ctrl+D)', onclick: () => run('do') }),
    el('button', { class: 'btn', text: 'Print it', title: 'Run and show the result (Ctrl+P)', onclick: () => run('print') }),
    el('button', { class: 'btn', text: 'Inspect it', title: 'Run and inspect the result (Ctrl+I)', onclick: () => run('inspect') }),
    el('button', { class: 'btn', text: 'Typecheck', title: 'Check the types without running (Ctrl+T)', onclick: () => run('typecheck') }),
    el('span', { class: 'tsep' }),
    el('button', { class: 'btn', text: 'File it in', title: 'Read the text as chunk-format source (Delta define: ... !)', onclick: async () => {
      const r = handleReply(await vm.call('fileIn', ta.value));
      setMsg(`Filed in: ${r.methods} methods, ${r.doits} definitions` + (r.errors.length ? ` — ${r.errors[0]}` : ''), r.errors.length > 0);
      refreshBrowsers();
    } }),
    el('button', { class: 'btn', text: '→ TypeScript', title: 'Translate the selection (or the whole text if it is a file-in, else the paragraph) to TypeScript (Ctrl+E)', onclick: () => toTS() }),
    el('button', { class: 'btn', text: 'Clear', onclick: () => { ta.value = ''; ta.focus(); } }));
  function toTS() {
    const sel = ta.value.slice(ta.selectionStart, ta.selectionEnd);
    const text = sel.trim() ? sel : looksLikeFileIn(ta.value) ? ta.value : selectionOrParagraph(ta).text;
    if (!text.trim()) return;
    translateText(text, w.title || 'workspace');
  }
  ta.addEventListener('keydown', (ev) => { if ((ev.ctrlKey || ev.metaKey) && ev.key.toLowerCase() === 'e') { ev.preventDefault(); toTS(); } });
  w.body.append(tb, ta.mount, status);
  setTimeout(() => { ta.focus(); ta.setSelectionRange(0, 0); ta.scrollTop = 0; }, 0);
  return w;
}

// ------------------------------------------------------------------ class hierarchy

const browsers = new Set();
async function refreshBrowsers() { for (const b of browsers) b.refresh(); }

async function openHierarchy(filter) {
  const existing = [...windows].find((w) => w.kind === 'hierarchy');
  if (existing) { existing.focus(); if (filter !== undefined) existing.setFilter(filter); return; }
  const w = createWindow({ title: 'Class Hierarchy', width: 330, height: 520, x: 12, y: 356, kind: 'hierarchy', tall: true });
  const input = el('input', { placeholder: 'Filter classes…', 'aria-label': 'Filter classes', value: filter || '' });
  const tree = el('div', { class: 'tree sunken', role: 'tree' });
  const collapsed = new Set();
  let classes = [];
  async function load() { classes = await vm.call('classes'); render(); }
  function render() {
    const q = input.value.trim().toLowerCase();
    const kids = new Map();
    for (const c of classes) {
      const p = c.superclass || '';
      if (!kids.has(p)) kids.set(p, []);
      kids.get(p).push(c);
    }
    const matches = (c) => c.name.toLowerCase().includes(q);
    const subtreeHit = new Map();
    const hit = (c) => {
      if (subtreeHit.has(c.name)) return subtreeHit.get(c.name);
      const v = matches(c) || (kids.get(c.name) || []).some(hit);
      subtreeHit.set(c.name, v);
      return v;
    };
    tree.textContent = '';
    const walk = (c, depth) => {
      if (q && !hit(c)) return;
      const ch = kids.get(c.name) || [];
      const open = q ? true : !collapsed.has(c.name);
      const row = el('div', { class: 'node' + (q && matches(c) ? ' hit' : ''), role: 'treeitem', style: `padding-left:${depth * 14 + 2}px` },
        el('span', { class: 'tw', text: ch.length ? (open ? '▾' : '▸') : '', onclick: () => { collapsed.has(c.name) ? collapsed.delete(c.name) : collapsed.add(c.name); render(); } }),
        el('span', { class: 'nm', text: c.name + (c.generic ? '[…]' : ''), onclick: () => openBrowser(c.name) }),
        el('span', { class: 'gp', text: c.group }));
      tree.append(row);
      if (open) for (const k of ch) walk(k, depth + 1);
    };
    for (const root of kids.get('') || []) walk(root, 0);
  }
  input.addEventListener('input', render);
  w.body.append(el('div', { class: 'filterrow' }, input, el('button', { class: 'btn small', text: 'Refresh', onclick: load })), tree);
  w.setFilter = (f) => { input.value = f; render(); };
  w.refresh = load;
  browsers.add(w);
  const close = w.close;
  w.close = () => { browsers.delete(w); close(); };
  await load();
}

// ------------------------------------------------------------------ outliner class browser

function sigNodes(sig) {
  // "at: index <Int> put: value <E> ^<E>" -> keywords bold, types coloured
  const out = [];
  const re = /(\^?<[^<>]*(?:<[^<>]*>[^<>]*)*>)|([A-Za-z_][\w]*:)|(\s+)|([^\s<]+)/g;
  let m;
  while ((m = re.exec(sig))) {
    if (m[1]) out.push(el('span', { class: 'ty', text: m[1] }));
    else if (m[2]) out.push(el('span', { class: 'kw', text: m[2] }));
    else if (m[3]) out.push(m[3]);
    else out.push(el('span', { class: out.length === 0 ? 'kw' : 'ar', text: m[4] }));
  }
  return out;
}

async function openBrowser(name, focus) {
  if (!name) return;
  name = name.replace(/ class$/, '');
  const existing = [...windows].find((w) => w.kind === 'browser' && w.className === name);
  if (existing) { existing.focus(); if (focus) existing.reveal(focus); return existing; }
  const info0 = await vm.call('classInfo', name);
  if (!info0.ok) { transcript.sys(`There is no class named ${name}.\n`); return; }
  const w = createWindow({ title: 'Class: ' + name, width: 660, height: 560, kind: 'browser', tall: true });
  w.className = name;
  let side = focus && focus.meta ? 'class' : 'instance';
  const open = new Set(['Definition', 'Comment']);
  const openMethods = new Set();
  if (focus) { open.add('cat:' + (focus.meta ? 'class' : 'instance') + ':' + (focus.category || '')); openMethods.add((focus.meta ? 'c:' : 'i:') + focus.selector); }
  const sideBtns = {
    instance: el('button', { class: 'btn', text: 'Instance side', onclick: () => { side = 'instance'; render(); } }),
    class: el('button', { class: 'btn', text: 'Class side', onclick: () => { side = 'class'; render(); } }),
  };
  const problemsBox = el('div');
  const tb = el('div', { class: 'toolbar' }, sideBtns.instance, sideBtns.class, el('span', { class: 'tsep' }),
    el('button', { class: 'btn', text: 'New method', onclick: () => newMethod() }),
    el('button', { class: 'btn', text: 'Typecheck', onclick: () => typecheckClass() }),
    el('button', { class: 'btn', text: 'Hierarchy', onclick: () => openHierarchy(name) }),
    el('button', { class: 'btn', text: 'File out', onclick: async () => download(name + '.dlt', await vm.call('fileOut', name)) }),
    el('button', { class: 'btn', text: '→ TypeScript', title: 'Translate this class to TypeScript', onclick: () => translateClass(name) }));
  const outline = el('div', { class: 'outline sunken' });
  w.body.append(tb, outline);
  let info = info0;
  let pendingNew = null;

  function section(key, label, count, build) {
    const isOpen = open.has(key);
    const kids = el('div', { class: 'okids' });
    const sec = el('div', { class: 'osec' },
      el('div', { class: 'oline', onclick: () => { isOpen ? open.delete(key) : open.add(key); render(); } },
        el('span', { class: 'tri', text: isOpen ? '▾' : '▸' }),
        el('span', { class: 'olabel', text: label }),
        count !== undefined ? el('span', { class: 'ocount', text: `(${count})` }) : null),
      kids);
    if (isOpen) build(kids);
    return sec;
  }

  function editor(source, onAccept, extra = []) {
    const ta = codeArea(source);
    const lines = Math.min(28, Math.max(4, source.split('\n').length + 1));
    ta.style.height = lines * 19 + 10 + 'px';
    const msg = el('div', { class: 'msg' });
    const accept = async () => {
      msg.className = 'msg'; msg.textContent = 'accepting…';
      const r = await onAccept(ta.value, msg);
      if (r && r.problems && r.problems.length) {
        msg.className = 'msg err';
        msg.textContent = 'Accepted, but the type checker found:\n' + r.problems.map((p) => `  line ${p.line}: ${p.message}`).join('\n');
      }
    };
    ta.addEventListener('keydown', (e) => { if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 's') { e.preventDefault(); accept(); } });
    return el('div', { class: 'editor' }, ta.mount, el('div', { class: 'toolbar' }, el('button', { class: 'btn small', text: 'Accept', title: 'Compile (Ctrl+S)', onclick: accept }), ...extra), msg);
  }

  function methodRow(m, meta) {
    const key = (meta ? 'c:' : 'i:') + m.selector;
    const isOpen = openMethods.has(key);
    const row = el('div', { class: 'mrowx' });
    const sig = el('div', { class: 'msig', onclick: () => { isOpen ? openMethods.delete(key) : openMethods.add(key); render(); } },
      el('span', { class: 'tri', text: isOpen ? '▾' : '▸' }), ...sigNodes(m.signature),
      m.native ? el('span', { class: 'nat', text: 'in the VM' }) : null);
    row.append(sig);
    if (isOpen) {
      const holder = el('div', { class: 'editor', text: 'loading…' });
      row.append(holder);
      vm.call('method', name, meta, m.selector).then((src) => {
        const ed = editor(src.source || '', async (text, msg) => {
          const r = await vm.call('compile', name, meta, m.category, text);
          if (!r.ok) { msg.className = 'msg err'; msg.textContent = 'Not accepted: ' + r.error; return r; }
          msg.className = 'msg ok'; msg.textContent = 'Accepted: #' + r.selector;
          if (r.selector !== m.selector) { openMethods.add((meta ? 'c:' : 'i:') + r.selector); }
          await reload(); return r;
        }, [
          el('button', { class: 'btn small', text: 'Senders', onclick: () => openSenders(m.selector) }),
          el('button', { class: 'btn small', text: 'Implementors', onclick: () => openImplementors(m.selector) }),
          el('button', { class: 'btn small', text: 'Remove', onclick: async () => {
            if (!confirm(`Remove ${name}${meta ? ' class' : ''}>>${m.selector}?`)) return;
            await vm.call('removeMethod', name, meta, m.selector); openMethods.delete(key); await reload();
          } }),
        ]);
        holder.replaceWith(ed);
      });
    }
    return row;
  }

  function render() {
    sideBtns.instance.classList.toggle('on', side === 'instance');
    sideBtns.class.classList.toggle('on', side === 'class');
    const meta = side === 'class';
    const methods = meta ? info.classMethods : info.instanceMethods;
    const scroll = outline.scrollTop;
    outline.textContent = '';
    const chain = el('span', { class: 'chain' });
    chain.append('← ');
    info.superclasses.forEach((s, i) => { if (i) chain.append(' ← '); chain.append(el('a', { text: s, onclick: () => openBrowser(s) })); });
    outline.append(el('div', { class: 'ohead' }, el('span', { class: 'cname', text: info.name + (meta ? ' class' : '') }),
      info.superclasses.length ? chain : el('span', { class: 'chain', text: '(root class)' })));
    if (info.generics.length) outline.append(el('div', { class: 'cmt', text: 'Generic over ' + info.generics.map((g) => g.name + (g.type ? ' <' + g.type + '>' : '')).join(', ') }));
    if (info.subclasses.length) {
      const sub = el('div', { class: 'chain', style: 'margin:2px 0 6px' }, 'Subclasses: ');
      info.subclasses.forEach((s, i) => { if (i) sub.append(', '); sub.append(el('a', { text: s, onclick: () => openBrowser(s) })); });
      outline.append(sub);
    }
    outline.append(problemsBox);
    outline.append(section('Definition', 'Definition', undefined, (k) => {
      k.append(editor(info.definition, async (text, msg) => {
        const r = await evaluate(text, '', false);
        if (r.ok) { msg.className = 'msg ok'; msg.textContent = 'Class definition accepted.'; await reload(); refreshHierarchy(); }
        else { msg.className = 'msg err'; msg.textContent = 'Not accepted: ' + r.error; }
        return r;
      }));
      const ivars = meta ? info.classIvars : info.ivars;
      if (ivars.length) {
        k.append(el('div', { class: 'cmt', style: 'margin:0 0 6px 16px' }, (meta ? 'Class-side instance variables: ' : 'Instance variables: ') +
          ivars.map((v) => v.name + (v.type ? ' <' + v.type + '>' : '')).join(', ')));
      }
    }));
    outline.append(section('Comment', 'Comment', undefined, (k) => {
      k.append(editor(info.comment || '', async (text, msg) => {
        await vm.call('setComment', name, text);
        msg.className = 'msg ok'; msg.textContent = 'Comment saved.'; info.comment = text; return null;
      }));
    }));
    const cats = new Map();
    for (const m of methods) { if (!cats.has(m.category)) cats.set(m.category, []); cats.get(m.category).push(m); }
    const catNames = [...cats.keys()].sort();
    if (pendingNew && pendingNew.meta === meta && !cats.has(pendingNew.category)) catNames.push(pendingNew.category);
    for (const cat of catNames) {
      const ms = cats.get(cat) || [];
      const key = 'cat:' + side + ':' + cat;
      outline.append(section(key, cat, ms.length, (k) => {
        for (const m of ms) k.append(methodRow(m, meta));
        if (pendingNew && pendingNew.meta === meta && pendingNew.category === cat) k.append(newEditor(meta, cat));
      }));
    }
    if (!methods.length && !pendingNew) outline.append(el('div', { class: 'cmt', style: 'margin:8px 2px', text: `No ${side}-side methods yet. Use New method to add one.` }));
    outline.scrollTop = scroll;
  }

  function newEditor(meta, cat) {
    const tpl = 'messageName: argument <Object> ^<Object>\n\t"Say what this method does."\n\t^argument';
    return editor(tpl, async (text, msg) => {
      const r = await vm.call('compile', name, meta, cat, text);
      if (!r.ok) { msg.className = 'msg err'; msg.textContent = 'Not accepted: ' + r.error; return r; }
      pendingNew = null;
      openMethods.add((meta ? 'c:' : 'i:') + r.selector);
      await reload();
      return r;
    }, [el('button', { class: 'btn small', text: 'Cancel', onclick: () => { pendingNew = null; render(); } })]);
  }

  async function newMethod() {
    const cat = await askText('New method', 'Category for the new method', 'accessing');
    if (!cat) return;
    pendingNew = { meta: side === 'class', category: cat.trim() || 'as yet unclassified' };
    open.add('cat:' + side + ':' + pendingNew.category);
    render();
  }

  async function typecheckClass() {
    const r = await vm.call('typecheckClass', name);
    problemsBox.textContent = '';
    const box = el('div', { class: 'problems' });
    if (!r.problems.length) box.append(el('div', { class: 'ok-note', text: `✓ ${name} type-checks: no type errors.` }));
    else {
      box.append(el('div', { style: 'font-weight:bold', text: `${r.problems.length} type error(s) in ${name}:` }));
      for (const p of r.problems) {
        box.append(el('div', {}, el('a', { class: 'link', text: `${p.meta ? 'class ' : ''}${p.selector}`, onclick: () => reveal({ meta: p.meta, selector: p.selector }) }), ` line ${p.line}: ${p.message}`));
      }
    }
    box.append(el('a', { class: 'link', style: 'font-size:11px', text: 'hide', onclick: () => (problemsBox.textContent = '') }));
    problemsBox.append(box);
  }

  async function reload() {
    const r = await vm.call('classInfo', name);
    if (r.ok) info = r;
    render();
  }

  function reveal(f) {
    side = f.meta ? 'class' : 'instance';
    const m = (f.meta ? info.classMethods : info.instanceMethods).find((x) => x.selector === f.selector);
    if (m) { open.add('cat:' + side + ':' + m.category); openMethods.add((f.meta ? 'c:' : 'i:') + f.selector); }
    render();
    setTimeout(() => {
      const t = [...outline.querySelectorAll('.msig')].find((x) => x.textContent.replace(/\s+/g, '').startsWith(('▾' + (m ? m.signature : f.selector)).replace(/\s+/g, '').slice(0, 6)));
      if (t) t.scrollIntoView({ block: 'center' });
    }, 80);
  }
  if (focus) {
    const m = (focus.meta ? info.classMethods : info.instanceMethods).find((x) => x.selector === focus.selector);
    if (m) open.add('cat:' + (focus.meta ? 'class' : 'instance') + ':' + m.category);
  }
  w.reveal = reveal;
  w.refresh = reload;
  browsers.add(w);
  const close = w.close;
  w.close = () => { browsers.delete(w); close(); };
  render();
  return w;
}

function refreshHierarchy() { const h = [...windows].find((w) => w.kind === 'hierarchy'); if (h) h.refresh(); }

// ------------------------------------------------------------------ method lists

function openMethodList(title, rows, empty) {
  const w = createWindow({ title, width: 380, height: 300, kind: 'list' });
  const list = el('div', { class: 'list sunken', style: 'flex:1' });
  if (!rows.length) list.append(el('div', { class: 'li', text: empty }));
  for (const r of rows) {
    list.append(el('div', { class: 'li', title: 'Open in a class browser', ondblclick: () => openBrowser(r.class, r), onclick: (e) => {
      list.querySelectorAll('.sel').forEach((x) => x.classList.remove('sel')); e.currentTarget.classList.add('sel'); openBrowser(r.class, r);
    } }, r.label || `${r.class}>>${r.selector}`));
  }
  w.body.append(list, el('div', { class: 'status', text: `${rows.length} method(s)` }));
}

async function openSenders(sel) { openMethodList(`Senders of #${sel}`, await vm.call('senders', sel), 'No methods send this message.'); }
async function openImplementors(sel) { openMethodList(`Implementors of #${sel}`, await vm.call('implementors', sel), 'No class implements this message.'); }

async function typecheckAll() {
  transcript.sys('Type-checking every class…\n');
  const t0 = performance.now();
  const r = await vm.call('typecheckAll');
  const ms = Math.round(performance.now() - t0);
  transcript.sys(`Type check: ${r.classes} classes, ${r.problems.length} type error(s) (${ms} ms).\n`);
  if (r.problems.length) {
    openMethodList('Type errors', r.problems.map((p) => ({ class: p.class, meta: p.meta, selector: p.selector,
      label: `${p.class}${p.meta ? ' class' : ''}>>${p.selector} — ${p.message}` })), '');
  }
}

// ------------------------------------------------------------------ inspector

async function openInspector(id) {
  const r = await vm.call('inspect', id);
  if (!r.ok) return;
  const w = createWindow({ title: 'Inspector: ' + short(r.title, 48), width: 480, height: 320, kind: 'inspector' });
  const list = el('div', { class: 'list sunken', style: 'width:42%;min-width:120px' });
  const val = el('div', { class: 'valpane sunken' });
  const show = (name, text) => { val.textContent = text; };
  const selfRow = el('div', { class: 'li sel' }, el('span', { class: 'k', text: 'self' }));
  selfRow.onclick = () => { mark(selfRow); show('self', r.title); };
  list.append(selfRow);
  const mark = (row) => { list.querySelectorAll('.sel').forEach((x) => x.classList.remove('sel')); row.classList.add('sel'); };
  for (const f of r.fields) {
    const row = el('div', { class: 'li', title: 'Double-click to inspect this value' }, el('span', { class: 'k', text: f.name }), ': ', short(f.value, 40));
    row.onclick = () => { mark(row); show(f.name, f.value); };
    row.ondblclick = () => openInspector(f.id);
    list.append(row);
  }
  show('self', r.title);
  w.body.append(el('div', { class: 'split' }, list, val),
    el('div', { class: 'status', text: `${r.class} · ${r.fields.length} field(s) · double-click a field to inspect it` }));
}

function short(s, n) { s = String(s); return s.length > n ? s.slice(0, n - 1) + '…' : s; }

// ------------------------------------------------------------------ errors (a simple debugger view)

function openError(r, source) {
  const w = createWindow({ title: (r.errorClass || 'Error') + ': ' + short(r.error, 60), width: 520, height: 320, kind: 'error' });
  const list = el('div', { class: 'list sunken', style: 'flex:1' });
  for (const s of r.stack || []) {
    const m = /^(?:\[\] in )?(\w+)( class)?>>(.+)$/.exec(s);
    const row = el('div', { class: 'li', text: s, title: m && m[1] !== 'UndefinedObject' ? 'Open this method' : '' });
    if (m && !(m[1] === 'UndefinedObject' && m[3] === 'DoIt')) row.onclick = () => openBrowser(m[1], { meta: !!m[2], selector: m[3] });
    list.append(row);
  }
  w.body.append(el('div', { class: 'errhead' }, el('span', { class: 'bang', text: '!' }), el('span', { text: r.error })),
    el('div', { style: 'padding:0 2px 2px;font-size:11px', text: 'Stack (most recent call first):' }), list,
    el('div', { class: 'toolbar' }, el('button', { class: 'btn', text: 'Close', onclick: () => w.close() }),
      source ? el('button', { class: 'btn', text: 'Copy the code to a workspace', onclick: () => { openWorkspace(source); w.close(); } }) : null));
}

// ------------------------------------------------------------------ canvases

const canvasWindows = new Map();
function showCanvas(e, pixels) {
  let w = canvasWindows.get(e.id);
  if (!w || !document.body.contains(w.el)) {
    w = createWindow({ title: e.title, width: Math.max(240, e.width + 16), height: e.height + 64, kind: 'canvas' });
    const c = el('canvas', { width: e.width, height: e.height, 'aria-label': e.title });
    w.canvas = c;
    w.body.append(el('div', { class: 'canvas-wrap sunken' }, c),
      el('div', { class: 'status', text: `${e.width} × ${e.height}` }));
    canvasWindows.set(e.id, w);
  }
  const ctx = w.canvas.getContext('2d');
  ctx.putImageData(new ImageData(new Uint8ClampedArray(pixels), e.width, e.height), 0, 0);
  w.focus();
}

// ------------------------------------------------------------------ help windows

function openGuide() {
  const w = createWindow({ title: 'Strongtalk language guide', width: 600, height: 560, kind: 'doc', tall: true });
  w.body.append(el('div', { class: 'doc sunken' }));
  w.body.firstChild.innerHTML = GUIDE;
  w.body.firstChild.querySelectorAll('pre').forEach((p) => {
    p.title = 'Double-click to copy this example to a new workspace';
    p.ondblclick = () => openWorkspace(p.textContent);
  });
}

function openAbout() {
  const w = createWindow({ title: 'About Strongtalk', width: 420, height: 300, kind: 'doc' });
  w.body.append(el('div', { class: 'doc sunken' }));
  w.body.firstChild.innerHTML = `<h2>Strongtalk</h2>
  <p>Smalltalk with an optional, type-safe type system: types are written in angle brackets
  (<code>x &lt;Int&gt;</code>, <code>^&lt;Str&gt;</code>) and checked without affecting how programs run.</p>
  <p>This edition re-implements the Strongtalk virtual machine in <b>Rust</b>. The same engine runs
  on the Mac as a terminal program and here, compiled to <b>WebAssembly</b>, in a background thread of the page.</p>
  <p>The class library, browsers and type checker are new, written from the language's published design;
  the original image and its sources are not included.</p>
  <p style="font-size:11.5px">Strongtalk © 1994–2006 Sun Microsystems Inc.; this edition © 2026 its contributors.
  Free software under the <a href="../LICENSE.txt" target="_blank">BSD 3-Clause license</a>, the license Sun released Strongtalk under.
  <a href="../THIRD-PARTY-NOTICES.txt" target="_blank">Third-party notices</a>.</p>
  <p id="aboutstats" style="color:#555"></p>`;
  vm.call('eval', "Smalltalk version, ' - ', Smalltalk classNames size printString, ' classes'", '', true)
    .then((r) => { const p = document.getElementById('aboutstats'); if (p && r.ok) p.textContent = r.value.replace(/^'|'$/g, ''); });
}

const GUIDE = `<h2>Strongtalk in ten minutes</h2>
<p>Everything is an object and everything happens by sending messages. Put the cursor in an example
in a workspace and press <b>Print it</b>. Double-click an example here to copy it into a workspace.</p>
<h3>Messages</h3>
<pre>3 + 4 * 2              "binary messages go left to right: 14"
'hello' size           "unary message"
'hello' copyFrom: 2 to: 4    "keyword message"
Transcript show: 'hi'; cr    "a cascade: two messages to Transcript"</pre>
<h3>Variables and blocks</h3>
<pre>| total &lt;Int&gt; square &lt;[Int, ^Int]&gt; |
square := [:x &lt;Int&gt; | x * x].
total := 0.
1 to: 5 do: [:i &lt;Int&gt; | total := total + (square value: i)].
total</pre>
<h3>Control structures are messages too</h3>
<pre>| n |
n := 17.
n isPrime
    ifTrue: ['prime']
    ifFalse: ['not prime']</pre>
<h3>Collections</h3>
<pre>#(3 1 4 1 5 9 2 6) asSortedCollection asArray
(1 to: 20) select: [:i | i even]
#('apple' 'fig' 'banana') inject: 0 into: [:sum :s | sum + s size]
| d |
d := Dictionary new.
d at: #one put: 1; at: #two put: 2.
d keys</pre>
<h3>Types</h3>
<p>Annotations are optional. <code>&lt;Int&gt;</code> is an Integer, <code>&lt;Str&gt;</code> a String,
<code>&lt;Self&gt;</code> the receiver's class, <code>&lt;Int | Str&gt;</code> either one,
<code>&lt;[Int, ^Bool]&gt;</code> a block from Int to Boolean, and <code>Array[Int]</code> a generic.
Press <b>Typecheck</b> to check a workspace selection, or use a class browser's Typecheck button.</p>
<pre>| s &lt;Str&gt; |
s := 42.          "the type checker reports: assigning SmallInteger to 's'"
s</pre>
<h3>Defining a class</h3>
<pre>Delta define: #Counter as: (
    (Class subclassOf: 'Object' instanceVariables: 'count &lt;Int&gt;'))</pre>
<pre>Counter compile: 'increment ^&lt;Int&gt;
    count := (count ifNil: [0]) + 1.
    ^count'</pre>
<pre>| c |
c := Counter new.
c increment; increment; increment</pre>
<p>Or open the class in a browser (Browse &rsaquo; Find class), press <b>New method</b>, type the method and press <b>Accept</b>.
Everything you define is kept in this browser and comes back when you reload the page.</p>
<h3>Errors</h3>
<pre>[10 / 0] on: ZeroDivide do: [:e | e return: 0]
[nil foo] on: MessageNotUnderstood do: [:e | e messageText]
[^42] ensure: [Transcript show: 'cleaned up'; cr]</pre>
<h3>Pictures</h3>
<pre>| pen |
pen := Pen new: 260.
5 timesRepeat: [pen go: 100; turn: 144].
pen show</pre>`;

// ------------------------------------------------------------------ start-up

document.getElementById('start').addEventListener('click', (ev) => {
  const r = ev.currentTarget.getBoundingClientRect();
  showMenu([
    ['New workspace', () => openWorkspace()],
    ['Class hierarchy', () => openHierarchy()],
    ['Find class…', findClass],
    ['Type-check all classes', typecheckAll],
    '-',
    ['Language guide', openGuide],
    ['Articles and history', () => { location.href = '../'; }],
    ['About Strongtalk', openAbout],
  ], r.left, r.top - 212);
});

document.getElementById('stopbtn').addEventListener('click', () => interrupt());

document.addEventListener('keydown', (e) => {
  if ((e.ctrlKey || e.metaKey) && e.key === '.' ) { e.preventDefault(); interrupt(); }
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'b' && e.target.tagName !== 'TEXTAREA') { e.preventDefault(); findClass(); }
});

async function boot(restart) {
  const t0 = performance.now();
  setStatus('busy', 'starting…');
  let errors = [];
  try { const b = await vm.call('boot'); errors = b.errors; vm.maxNesting = b.maxNesting; } catch (e) { setStatus('dead', 'failed'); transcript.error('Start-up failed: ' + e.message + '\n'); return; }
  const changes = loadChanges();
  let restored = null;
  if (changes) {
    try { restored = await vm.call('replay', changes); } catch { restored = null; }
  }
  const classes = await vm.call('classes');
  const ms = Math.round(performance.now() - t0);
  if (!restart) {
    transcript.sys(`Strongtalk 2.0 — Rust virtual machine running as WebAssembly\n`);
    transcript.sys(`${classes.length} classes loaded in ${ms} ms. Open Help › Language guide to start.\n`);
  } else transcript.sys(`Virtual machine restarted (${ms} ms).\n`);
  for (const e of errors) transcript.error('library: ' + e + '\n');
  if (restored) {
    transcript.sys(`Restored your changes: ${restored.methods} methods, ${restored.doits} definitions.\n`);
    for (const e of restored.errors || []) transcript.error('  ' + e + '\n');
  }
  setStatus('ready', 'ready');
}

(async function main() {
  document.getElementById('boot').remove();
  openLauncher();
  if (phone()) {
    openWorkspace();
    await boot(false);
    return;
  }
  openWorkspace();
  await boot(false);
  await openHierarchy();
  [...windows].find((w) => w.kind === 'workspace')?.focus();
})();
