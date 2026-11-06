// The home page's live workspace: starts the same WebAssembly VM the environment uses
// (try/worker.js) on first use, and prints the result of the code in the box.
const EXAMPLES = [
  ['Squares', '(1 to: 10) collect: [:i <Int> | i * i]'],
  ['100 factorial', '100 factorial'],
  ['Fractions', '(3/4) + (1/6)'],
  ['Big integers', '2 raisedTo: 100'],
  ['Typed block', '[:x <Int> | x * 2] value: 21'],
  ['Primes', '(1 to: 50) select: [:n | n isPrime]'],
  ['Exceptions', "[1/0] on: ZeroDivide do: [:e | 'caught: ', e messageText]"],
  ['Transcript', "Transcript show: 'Hello from Strongtalk'; cr.\n3 + 4 * 2   \"binary messages go left to right\""],
  ['Sorting', '#(3 1 4 1 5 9 2 6) asSortedCollection asArray'],
];
const TIMEOUT_MS = 10000;

const play = document.getElementById('play');
const src = document.getElementById('play-src');
const out = document.getElementById('play-out');
const runBtn = document.getElementById('play-run');
const status = document.getElementById('play-status');
const chips = document.getElementById('play-chips');

let worker = null;
let booting = null;
let nextId = 1;
const pending = new Map();
let transcriptBuf = '';

function setState(state, label) { play.dataset.state = state; status.textContent = label; }

function startWorker() {
  if (booting) return booting;
  setState('busy', 'starting VM…');
  worker = new Worker(new URL('./try/worker.js', import.meta.url), { type: 'module' });
  worker.onmessage = (ev) => {
    const m = ev.data;
    if (m.type === 'transcript') { transcriptBuf += m.text; return; }
    if (m.type === 'reply' && pending.has(m.id)) {
      const { resolve, reject, timer } = pending.get(m.id);
      clearTimeout(timer);
      pending.delete(m.id);
      m.error ? reject(new Error(m.error)) : resolve(m.result);
    }
  };
  worker.onerror = () => { setState('error', 'VM failed to load'); };
  const t0 = performance.now();
  booting = call('boot').then(() => {
    setState('ready', `ready in ${Math.round(performance.now() - t0)} ms`);
  }).catch((e) => { booting = null; setState('error', 'VM failed'); throw e; });
  return booting;
}

function call(op, ...args) {
  return new Promise((resolve, reject) => {
    const id = nextId++;
    const timer = setTimeout(() => {
      pending.delete(id);
      // An endless loop: throw this VM away and start a fresh one next time.
      worker.terminate();
      worker = null; booting = null;
      reject(new Error('timeout'));
    }, TIMEOUT_MS);
    pending.set(id, { resolve, reject, timer });
    worker.postMessage({ id, op, args });
  });
}

const esc = (s) => String(s).replace(/[&<>]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;' })[c]);

async function run() {
  const code = src.value.trim();
  if (!code) return;
  runBtn.disabled = true;
  out.innerHTML = '<span class="m">running…</span>';
  try {
    await startWorker();
    setState('busy', 'running…');
    transcriptBuf = '';
    const t0 = performance.now();
    const r = await call('eval', code, '', true);
    const ms = Math.round(performance.now() - t0);
    let html = '';
    // Transcript text comes either streamed from the worker or in the reply, never both.
    const tx = transcriptBuf || (r && r.transcript) || '';
    if (tx) html += `<span class="t">${esc(tx)}</span>${tx.endsWith('\n') ? '' : '\n'}`;
    if (r && r.ok !== false) html += `<span class="p">→ </span><span class="v">${esc(r.value ?? 'nil')}</span>`;
    else html += `<span class="e">${esc((r && (r.errorClass ? r.errorClass + ': ' : '') + (r.error || 'error')) || 'error')}</span>`;
    out.innerHTML = html;
    setState('ready', `done in ${ms} ms`);
  } catch (e) {
    if (e.message === 'timeout') {
      out.innerHTML = `<span class="e">Stopped after ${TIMEOUT_MS / 1000} s.</span>\n<span class="m">Long-running code is better in the full environment, which has a Stop button.</span>`;
      setState('idle', 'VM restarted');
    } else {
      out.innerHTML = `<span class="e">${esc(e.message)}</span>`;
      setState('error', 'VM error');
      worker?.terminate(); worker = null; booting = null;
    }
  } finally {
    runBtn.disabled = false;
  }
}

for (const [label, code] of EXAMPLES) {
  const b = document.createElement('button');
  b.type = 'button'; b.className = 'chip'; b.textContent = label;
  b.setAttribute('aria-pressed', String(code === src.value));
  b.addEventListener('click', () => {
    src.value = code;
    chips.querySelectorAll('.chip').forEach((c) => c.setAttribute('aria-pressed', String(c === b)));
    run();
  });
  chips.appendChild(b);
}

runBtn.addEventListener('click', run);
src.addEventListener('keydown', (e) => {
  if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') { e.preventDefault(); run(); }
  if (e.key === 'Tab' && !e.shiftKey) { e.preventDefault(); document.execCommand('insertText', false, '    '); }
});
src.addEventListener('input', () => chips.querySelectorAll('.chip').forEach((c) => c.setAttribute('aria-pressed', 'false')));

// Warm the VM up once the page is idle and the playground is on screen.
const io = new IntersectionObserver((entries) => {
  if (entries.some((e) => e.isIntersecting)) {
    io.disconnect();
    const warm = () => startWorker().catch(() => {});
    if (window.requestIdleCallback) requestIdleCallback(warm, { timeout: 2000 });
    else setTimeout(warm, 600);
  }
});
io.observe(play);
