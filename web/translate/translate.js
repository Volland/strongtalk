// The translator page: Strongtalk → TypeScript in the browser (the Rust translator compiled to
// WebAssembly), plus "Run both": the source in the Strongtalk VM and the translation as JavaScript.
import init, { translateToTypeScript } from '../try/pkg/strongtalk.js';
import { attachHighlighter, highlightTranslation, highlightPage, bindFolds } from '../highlight.js';

const TS_COMPILER = 'https://cdnjs.cloudflare.com/ajax/libs/typescript/5.6.3/typescript.min.js';
const RUN_TIMEOUT_MS = 10000;

const EXAMPLES = [
  ['Squares (a do-it)', `"Select an example above, or type your own Strongtalk."
(1 to: 10) collect: [:i <Int> | i * i]`],
  ['Loops and the Transcript', `| sum <Int> |
sum := 0.
1 to: 100 do: [:i <Int> |
    i odd ifTrue: [sum := sum + i]].
Transcript show: 'sum of odd numbers: '; print: sum; cr.
#(3 1 4 1 5 9 2 6) asSortedCollection asArray`],
  ['Collections', `| words <OrderedCollection[Str]> counts <Dictionary[Str, Int]> |
words := 'the quick brown fox jumps over the lazy dog the end' substrings asOrderedCollection.
counts := Dictionary new.
words do: [:w <Str> | counts at: w put: (counts at: w ifAbsent: [0]) + 1].
Transcript show: 'distinct words: '; print: counts size; cr.
(words select: [:w | w size > 3]) inject: 0 into: [:a :w | a + w size]`],
  ['Exceptions', `| results |
results := OrderedCollection new.
#(4 2 0 5) do: [:d <Int> |
    results add: ([100 / d] on: ZeroDivide do: [:e | 'caught: ', e messageText])].
results asArray`],
  ['A typed class', `Delta define: #BankAccount as: (
(Class subclassOf: 'Object' instanceVariables: 'owner <Str> balance <Int> history <OrderedCollection[Int]>')) !
(Delta mirrorFor: #BankAccount) comment: 'A small typed class.' !

! (Delta mirrorFor: #BankAccount) classSide methodsFor: 'instance creation' !

owner: name <Str> ^<Instance>
	^self new setOwner: name
! !

! (Delta mirrorFor: #BankAccount) methodsFor: 'accessing' !

setOwner: name <Str> ^<Self>
	owner := name.
	balance := 0.
	history := OrderedCollection new
!

balance ^<Int>
	^balance
! !

! (Delta mirrorFor: #BankAccount) methodsFor: 'transactions' !

deposit: amount <Int> ^<Self>
	"Add money; deposits must be positive."
	amount <= 0 ifTrue: [^self error: 'deposits must be positive'].
	balance := balance + amount.
	history add: amount
!

withdraw: amount <Int> ^<Self>
	amount > balance ifTrue: [^self error: 'insufficient funds'].
	balance := balance - amount.
	history add: amount negated
! !

! (Delta mirrorFor: #BankAccount) methodsFor: 'printing' !

printOn: aStream <WriteStream>
	aStream nextPutAll: owner; nextPutAll: '''s account: '; print: balance
! !

| acct <BankAccount> |
acct := BankAccount owner: 'Ada'.
acct deposit: 100; withdraw: 30.
Transcript print: acct; cr.
Transcript show: 'balance: '; print: acct balance; cr !
`],
  ['Classes and inheritance', `Delta define: #Shape as: (
(Class subclassOf: 'Object' instanceVariables: 'name <Str>')) !

! (Delta mirrorFor: #Shape) methodsFor: 'accessing' !

name ^<Str>
	^name
!

name: aName <Str> ^<Self>
	name := aName
!

area ^<Float>
	"Subclasses answer their area."
	^self subclassResponsibility
!

describe ^<Str>
	^name , ' with area ' , self area printString
! !

Delta define: #Circle as: (
(Class subclassOf: 'Shape' instanceVariables: 'radius <Int>')) !

! (Delta mirrorFor: #Circle) classSide methodsFor: 'instance creation' !

radius: r <Int> ^<Instance>
	^self new setRadius: r
! !

! (Delta mirrorFor: #Circle) methodsFor: 'accessing' !

setRadius: r <Int> ^<Self>
	radius := r.
	self name: 'circle'
!

area ^<Float>
	^radius * radius * 3
! !

Delta define: #Square as: (
(Class subclassOf: 'Shape' instanceVariables: 'side <Int>')) !

! (Delta mirrorFor: #Square) classSide methodsFor: 'instance creation' !

side: s <Int> ^<Instance>
	^self new setSide: s
! !

! (Delta mirrorFor: #Square) methodsFor: 'accessing' !

setSide: s <Int> ^<Self>
	side := s.
	self name: 'square'
!

area ^<Float>
	^side * side
! !

| shapes <OrderedCollection[Shape]> |
shapes := OrderedCollection new.
shapes add: (Circle radius: 2); add: (Square side: 3); add: (Circle radius: 1).
shapes do: [:s <Shape> | Transcript show: s describe; cr].
Transcript show: 'largest: ';
    show: (shapes inject: shapes first into: [:a :b | a area > b area ifTrue: [a] ifFalse: [b]]) describe; cr !
`],
];

const $ = (id) => document.getElementById(id);
const src = $('src');
const out = $('out');
const notes = $('notes');
const mode = $('mode');
const tsinfo = $('tsinfo');
let ready = false;
let last = null;

// ------------------------------------------------------------------ translation

function translate() {
  if (!ready) return;
  const text = src.value;
  mode.textContent = /Delta define:|methodsFor:/.test(text) ? 'file-in with classes' : 'workspace code';
  const r = JSON.parse(translateToTypeScript(text));
  last = r;
  if (!r.ok) {
    tsinfo.textContent = 'syntax error';
    tsinfo.className = 'badge err';
    notes.hidden = false;
    notes.innerHTML = `<b>Syntax error</b> at line ${r.line}, column ${r.column}: ${esc(r.error)}`;
    notes.className = 'tr-notes err';
    return;
  }
  out.innerHTML = highlightTranslation(r.code);
  const lines = r.code.split('\n').length;
  tsinfo.className = 'badge';
  tsinfo.textContent = `${lines} lines${r.classes.length ? ' · ' + r.classes.length + ' class' + (r.classes.length > 1 ? 'es' : '') : ''}`;
  notes.className = 'tr-notes';
  if (r.warnings.length) {
    notes.hidden = false;
    notes.innerHTML = `<b>${r.warnings.length} note${r.warnings.length > 1 ? 's' : ''}</b><ul>${r.warnings.map((w) => `<li>${esc(w)}</li>`).join('')}</ul>`;
  } else {
    notes.hidden = true;
  }
}

const esc = (s) => String(s).replace(/[&<>]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;' })[c]);

let timer = 0;
src.addEventListener('input', () => { clearTimeout(timer); timer = setTimeout(translate, 180); });
src.addEventListener('keydown', (e) => {
  if (e.key === 'Tab' && !e.shiftKey && !e.metaKey && !e.ctrlKey) {
    e.preventDefault();
    src.setRangeText('\t', src.selectionStart, src.selectionEnd, 'end');
    src.dispatchEvent(new Event('input'));
  }
  if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') { e.preventDefault(); runBoth(); }
});

// ------------------------------------------------------------------ examples, sharing

const sel = $('examples');
EXAMPLES.forEach(([label], i) => sel.append(new Option(label, String(i))));
sel.addEventListener('change', () => { load(EXAMPLES[+sel.value][1]); $('runbox').hidden = true; });

function load(text) { src.value = text; translate(); }

function fromHash() {
  const m = /^#code=(.+)$/.exec(location.hash);
  if (!m) return null;
  try { return new TextDecoder().decode(Uint8Array.from(atob(m[1].replace(/-/g, '+').replace(/_/g, '/')), (c) => c.charCodeAt(0))); } catch { return null; }
}

$('share').addEventListener('click', async () => {
  const b = btoa(String.fromCharCode(...new TextEncoder().encode(src.value))).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
  const url = location.origin + location.pathname + '#code=' + b;
  history.replaceState(null, '', url);
  flash($('share'), (await copyText(url)) ? 'Link copied' : 'Link in the address bar');
});

$('copy').addEventListener('click', async () => {
  if (last && last.ok) flash($('copy'), (await copyText(last.code)) ? 'Copied' : 'Select and copy');
});

$('download').addEventListener('click', () => {
  if (!last || !last.ok) return;
  const name = (last.classes[0] || 'translation') + '.ts';
  const a = document.createElement('a');
  a.href = URL.createObjectURL(new Blob([last.code], { type: 'text/plain' }));
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(a.href), 1000);
});

async function copyText(t) { try { await navigator.clipboard.writeText(t); return true; } catch { return false; } }
function flash(btn, text) { const old = btn.textContent; btn.textContent = text; setTimeout(() => { btn.textContent = old; }, 1400); }

// ------------------------------------------------------------------ run both

let tsLoading = null;
function loadCompiler() {
  if (window.ts) return Promise.resolve(window.ts);
  return tsLoading ||= new Promise((resolve, reject) => {
    const s = document.createElement('script');
    s.src = TS_COMPILER;
    s.onload = () => resolve(window.ts);
    s.onerror = () => { tsLoading = null; reject(new Error('could not load the TypeScript compiler')); };
    document.head.append(s);
  });
}

/** Runs JavaScript in a throwaway worker, capturing console output. */
function runJs(js) {
  const body = `
    const lines = [];
    const fmt = (a) => a.map((x) => typeof x === 'string' ? x : (() => { try { return JSON.stringify(x); } catch { return String(x); } })()).join(' ');
    console.log = (...a) => lines.push(fmt(a));
    console.error = console.warn = console.log;
    console.dir = (x) => lines.push(fmt([x]));
    onmessage = () => {
      const t0 = performance.now();
      try { const exports = {}; (new Function('exports', ${JSON.stringify(js)}))(exports); postMessage({ ok: true, out: lines.join('\\n'), ms: performance.now() - t0 }); }
      catch (e) { postMessage({ ok: false, out: lines.join('\\n'), error: (e && e.constructor ? e.constructor.name + ': ' : '') + (e && e.message || e), ms: performance.now() - t0 }); }
    };`;
  const url = URL.createObjectURL(new Blob([body], { type: 'text/javascript' }));
  const w = new Worker(url);
  return new Promise((resolve) => {
    const t = setTimeout(() => { w.terminate(); resolve({ ok: false, out: '', error: `stopped after ${RUN_TIMEOUT_MS / 1000} s` }); }, RUN_TIMEOUT_MS);
    w.onmessage = (e) => { clearTimeout(t); w.terminate(); URL.revokeObjectURL(url); resolve(e.data); };
    w.onerror = (e) => { clearTimeout(t); w.terminate(); resolve({ ok: false, out: '', error: e.message }); };
    w.postMessage(0);
  });
}

/** Runs the Strongtalk source in the VM (the environment's worker). */
function runVm(text) {
  const w = new Worker(new URL('../try/worker.js', import.meta.url), { type: 'module' });
  let transcript = '';
  let id = 0;
  const call = (op, ...args) => new Promise((resolve, reject) => {
    const my = ++id;
    const prev = w.onmessage;
    w.onmessage = (e) => {
      const m = e.data;
      if (m.type === 'transcript') { transcript += m.text; return; }
      if (m.type === 'reply' && m.id === my) { w.onmessage = prev; m.error ? reject(new Error(m.error)) : resolve(m.result); }
    };
    w.postMessage({ id: my, op, args });
  });
  return new Promise((resolve) => {
    const t = setTimeout(() => { w.terminate(); resolve({ ok: false, out: transcript, error: `stopped after ${RUN_TIMEOUT_MS / 1000} s` }); }, RUN_TIMEOUT_MS + 3000);
    (async () => {
      try {
        await call('boot');
        const t0 = performance.now();
        const fileIn = /Delta define:|methodsFor:/.test(text);
        const r = fileIn ? await call('fileIn', text) : await call('eval', text, 'translator', true);
        const ms = performance.now() - t0;
        let o = transcript || r.transcript || '';
        if (!fileIn) {
          if (r.ok) o += (o && !o.endsWith('\n') ? '\n' : '') + r.value;
          else { clearTimeout(t); w.terminate(); resolve({ ok: false, out: o, error: `${r.errorClass || 'Error'}: ${r.error}`, ms }); return; }
        } else if (r.errors && r.errors.length) {
          clearTimeout(t); w.terminate(); resolve({ ok: false, out: o, error: r.errors[0], ms }); return;
        }
        clearTimeout(t); w.terminate();
        resolve({ ok: true, out: o, ms });
      } catch (e) { clearTimeout(t); w.terminate(); resolve({ ok: false, out: transcript, error: e.message }); }
    })();
  });
}

async function runBoth() {
  translate();
  if (!last || !last.ok) return;
  const box = $('runbox');
  box.hidden = false;
  $('vmout').textContent = 'running…';
  $('tsout').textContent = 'loading the TypeScript compiler…';
  $('verdict').className = 'verdict';
  $('verdict').textContent = '';
  const btn = $('run');
  btn.disabled = true;
  try {
    const vmP = runVm(src.value);
    let tsR;
    try {
      const ts = await loadCompiler();
      $('tsout').textContent = 'running…';
      const js = ts.transpile(last.code, { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS });
      tsR = await runJs(js);
    } catch (e) { tsR = { ok: false, out: '', error: e.message }; }
    const vmR = await vmP;
    show($('vmout'), $('vmtime'), vmR);
    show($('tsout'), $('tstime'), tsR);
    const norm = (s) => (s || '').replace(/\s+$/g, '').replace(/\r/g, '');
    const v = $('verdict');
    if (vmR.ok && tsR.ok && norm(vmR.out) === norm(tsR.out)) {
      v.className = 'verdict ok'; v.textContent = '✓ Same output from the Strongtalk VM and the TypeScript translation.';
    } else if (vmR.ok && tsR.ok) {
      v.className = 'verdict diff'; v.textContent = 'The outputs differ: see "Where it differs" below (numbers, characters and symbols are the usual reasons).';
    } else {
      v.className = 'verdict diff'; v.textContent = 'One of the runs stopped with an error.';
    }
    box.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
  } finally {
    btn.disabled = false;
  }
}

function show(pre, badge, r) {
  pre.innerHTML = esc(r.out || '') + (r.ok ? '' : `${r.out ? '\n' : ''}<span class="e">${esc(r.error)}</span>`);
  badge.textContent = r.ms != null ? `${r.ms < 1 ? '<1' : Math.round(r.ms)} ms` : '';
}

$('run').addEventListener('click', runBoth);

// ------------------------------------------------------------------ start

attachHighlighter(src, 'st');
bindFolds(out);
highlightPage(document.querySelector('.map'));
src.value = fromHash() || EXAMPLES[0][1];
init().then(() => { ready = true; translate(); }).catch((e) => {
  tsinfo.textContent = 'the translator could not load';
  out.textContent = String(e);
});
