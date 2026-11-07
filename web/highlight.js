// Syntax highlighting for Strongtalk and TypeScript, shared by the website, the playground,
// the translator and the environment. Produces HTML with <span class="t-…"> tokens:
//   t-c comment · t-s string · t-ch character · t-sym symbol · t-n number · t-ty type annotation
//   t-kw keyword selector · t-pv pseudo-variable / language keyword · t-cls class name
//   t-ret return (^) · t-op operator · t-par block parameter · t-prim primitive · t-chunk chunk mark
//   t-fn function (TypeScript)

const esc = (s) => s.replace(/[&<>]/g, (c) => (c === '&' ? '&amp;' : c === '<' ? '&lt;' : '&gt;'));
const span = (cls, text) => `<span class="t-${cls}">${esc(text)}</span>`;

const PSEUDO = new Set(['self', 'super', 'nil', 'true', 'false', 'thisContext']);

/** Finds the end of a type annotation starting at `<` (index i), or -1 if it is not one. */
function typeEnd(src, i) {
  const next = src[i + 1];
  if (!next || !/[A-Z\[]/.test(next)) return -1;
  let depth = 0;
  for (let j = i + 1; j < src.length && j < i + 200; j++) {
    const c = src[j];
    if (c === '[') depth++;
    else if (c === ']') depth--;
    else if (c === '>' && depth <= 0) return j + 1;
    else if (c === '\n' || !/[A-Za-z0-9_\[\],|^ \t.]/.test(c)) return -1;
  }
  return -1;
}

export function highlightStrongtalk(src) {
  let out = '';
  let i = 0;
  const n = src.length;
  while (i < n) {
    const c = src[i];
    if (c === '"') {
      const j = src.indexOf('"', i + 1);
      const e = j < 0 ? n : j + 1;
      out += span('c', src.slice(i, e)); i = e; continue;
    }
    if (c === "'") {
      let j = i + 1;
      while (j < n) { if (src[j] === "'") { if (src[j + 1] === "'") { j += 2; continue; } break; } j++; }
      const e = Math.min(n, j + 1);
      out += span('s', src.slice(i, e)); i = e; continue;
    }
    if (c === '$' && i + 1 < n) { out += span('ch', src.slice(i, i + 2)); i += 2; continue; }
    if (c === '#') {
      const m = /^#(?:[A-Za-z_][A-Za-z0-9_:]*|[-+*\/\\<>=~@%&?!,|]+|'(?:[^']|'')*')/.exec(src.slice(i));
      if (m) { out += span('sym', m[0]); i += m[0].length; continue; }
      out += span('sym', '#'); i++; continue;
    }
    if (c === '{' && src[i + 1] === '{') { out += span('prim', '{{'); i += 2; continue; }
    if (c === '}' && src[i + 1] === '}') { out += span('prim', '}}'); i += 2; continue; }
    if (c === '^') {
      out += span('ret', '^'); i++;
      if (src[i] === '<') { const e = typeEnd(src, i); if (e > 0) { out += span('ty', src.slice(i, e)); i = e; } }
      continue;
    }
    if (c === '<') {
      const e = typeEnd(src, i);
      if (e > 0) { out += span('ty', src.slice(i, e)); i = e; continue; }
    }
    if (c === ':' && src[i + 1] === '=') { out += span('op', ':='); i += 2; continue; }
    if (c === ':' && /[A-Za-z_]/.test(src[i + 1] || '')) {
      const m = /^:[A-Za-z_][A-Za-z0-9_]*/.exec(src.slice(i));
      out += span('par', m[0]); i += m[0].length; continue;
    }
    if (/[0-9]/.test(c)) {
      const m = /^\d+(?:r[0-9A-Z]+|(?:\.\d+)?(?:e-?\d+)?)/.exec(src.slice(i));
      out += span('n', m[0]); i += m[0].length; continue;
    }
    if (/[A-Za-z_]/.test(c)) {
      const m = /^[A-Za-z_][A-Za-z0-9_]*/.exec(src.slice(i));
      const w = m[0];
      const after = src[i + w.length];
      if (after === ':' && src[i + w.length + 1] !== '=') { out += span('kw', w + ':'); i += w.length + 1; continue; }
      if (PSEUDO.has(w)) out += span('pv', w);
      else if (/^[A-Z]/.test(w)) out += span('cls', w);
      else out += esc(w);
      i += w.length; continue;
    }
    if (c === '!' && (i === 0 || src[i - 1] === '\n' || /\s/.test(src[i - 1] || ''))) { out += span('chunk', '!'); i++; continue; }
    if (/[-+*\/\\<>=~@%&?,|]/.test(c)) {
      const m = /^[-+*\/\\<>=~@%&?,|]+/.exec(src.slice(i));
      out += span('op', m[0]); i += m[0].length; continue;
    }
    out += esc(c); i++;
  }
  return out;
}

const TS_KEYWORDS = new Set(('const let var class extends return if else for of in while do try catch finally throw new ' +
  'static export import from function this super null undefined true false typeof instanceof break continue ' +
  'switch case default declare type interface implements private protected public readonly async await void yield as').split(' '));
const TS_TYPES = new Set('number string boolean any unknown never void bigint object symbol'.split(' '));

export function highlightTypeScript(src) {
  let out = '';
  let i = 0;
  const n = src.length;
  let prevSig = '';
  while (i < n) {
    const c = src[i];
    if (c === '/' && src[i + 1] === '/') {
      const j = src.indexOf('\n', i); const e = j < 0 ? n : j;
      out += span('c', src.slice(i, e)); i = e; continue;
    }
    if (c === '/' && src[i + 1] === '*') {
      const j = src.indexOf('*/', i + 2); const e = j < 0 ? n : j + 2;
      out += span('c', src.slice(i, e)); i = e; continue;
    }
    if (c === "'" || c === '"' || c === '`') {
      let j = i + 1;
      while (j < n && src[j] !== c) { if (src[j] === '\\') j++; if (src[j] === '\n' && c !== '`') break; j++; }
      const e = Math.min(n, j + 1);
      out += span('s', src.slice(i, e)); i = e; prevSig = 's'; continue;
    }
    if (c === '/' && /[(,=:\[!&|?{};]$/.test(prevSig || '(')) {
      const m = /^\/(?:\\.|\[(?:\\.|[^\]])*\]|[^\/\n\\])+\/[gimsuy]*/.exec(src.slice(i));
      if (m) { out += span('s', m[0]); i += m[0].length; prevSig = 's'; continue; }
    }
    if (/[0-9]/.test(c)) {
      const m = /^\d[\d_]*(?:\.\d+)?(?:e-?\d+)?n?/.exec(src.slice(i));
      out += span('n', m[0]); i += m[0].length; prevSig = 'n'; continue;
    }
    if (/[A-Za-z_$]/.test(c)) {
      const m = /^[A-Za-z_$][A-Za-z0-9_$]*/.exec(src.slice(i));
      const w = m[0];
      const rest = src.slice(i + w.length);
      if (TS_KEYWORDS.has(w)) out += span('pv', w);
      else if (TS_TYPES.has(w)) out += span('ty', w);
      else if (/^[A-Z]/.test(w)) out += span('cls', w);
      else if (/^\s*\(/.test(rest) || w.startsWith('$')) out += span('fn', w);
      else out += esc(w);
      i += w.length; prevSig = 'w'; continue;
    }
    if (!/\s/.test(c)) prevSig = c;
    if (/[-+*\/%<>=!&|?:]/.test(c)) {
      const m = /^[-+*\/%<>=!&|?:]+/.exec(src.slice(i));
      out += span('op', m[0]); i += m[0].length; continue;
    }
    out += esc(c); i++;
  }
  return out;
}

/** Highlights every <pre><code> (and <pre data-lang>) block on the page. */
export function highlightPage(root = document) {
  for (const pre of root.querySelectorAll('pre')) {
    if (pre.dataset.hl === 'done' || pre.querySelector('span')) continue;
    const lang = pre.dataset.lang || (pre.querySelector('code.ts, code.typescript') ? 'ts' : 'st');
    const target = pre.querySelector('code') || pre;
    const text = target.textContent;
    target.innerHTML = lang === 'ts' ? highlightTypeScript(text) : highlightStrongtalk(text);
    pre.dataset.hl = 'done';
  }
}

/**
 * Turns a <textarea> into a highlighted editor: a <pre> underneath shows the colours, the textarea
 * on top (with transparent text) does the editing. Returns the wrapper element.
 */
export function attachHighlighter(ta, lang = 'st') {
  const wrap = document.createElement('div');
  wrap.className = 'hl-wrap';
  const pre = document.createElement('pre');
  pre.className = 'hl-under';
  pre.setAttribute('aria-hidden', 'true');
  ta.classList.add('hl-over');
  const parent = ta.parentNode;
  if (parent) parent.insertBefore(wrap, ta);
  wrap.append(pre, ta);
  const hl = lang === 'ts' ? highlightTypeScript : highlightStrongtalk;
  const proto = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value');
  let pending = false;
  const paint = () => {
    pending = false;
    const v = proto.get.call(ta);
    // a trailing newline needs a character after it to take up a line in a <pre>
    pre.innerHTML = hl(v) + (v.endsWith('\n') || v === '' ? ' ' : '');
    sync();
  };
  const refresh = () => { if (!pending) { pending = true; requestAnimationFrame(paint); } };
  const sync = () => {
    const cs = getComputedStyle(ta);
    pre.style.top = cs.borderTopWidth;
    pre.style.left = cs.borderLeftWidth;
    pre.style.width = ta.clientWidth + 'px';
    pre.style.height = ta.clientHeight + 'px';
    pre.style.padding = cs.padding;
    pre.style.font = cs.font;
    pre.style.tabSize = cs.tabSize;
    pre.style.letterSpacing = cs.letterSpacing;
    pre.scrollTop = ta.scrollTop;
    pre.scrollLeft = ta.scrollLeft;
  };
  Object.defineProperty(ta, 'value', {
    configurable: true,
    get() { return proto.get.call(ta); },
    set(v) { proto.set.call(ta, v); refresh(); },
  });
  const setRange = ta.setRangeText.bind(ta);
  ta.setRangeText = (...a) => { setRange(...a); refresh(); };
  ta.addEventListener('input', refresh);
  ta.addEventListener('scroll', () => { pre.scrollTop = ta.scrollTop; pre.scrollLeft = ta.scrollLeft; });
  if (typeof ResizeObserver !== 'undefined') new ResizeObserver(sync).observe(ta);
  ta.highlight = refresh;
  refresh();
  return wrap;
}

/**
 * TypeScript from the translator, highlighted, with its runtime prelude folded into one
 * clickable line (the full text is still what gets copied or downloaded).
 */
export function highlightTranslation(code) {
  const start = code.indexOf('// ---- Strongtalk runtime');
  const endMark = '// ---- end of runtime ----';
  const end = code.indexOf(endMark);
  if (start < 0 || end < 0) return highlightTypeScript(code);
  const runtime = code.slice(start, end + endMark.length);
  const lines = runtime.split('\n').length;
  return highlightTypeScript(code.slice(0, start)) +
    `<span class="ts-fold" role="button" tabindex="0" title="Show or hide the runtime helpers">// ▸ Strongtalk runtime: ${lines} lines of helpers this program uses (click to show)</span>` +
    `<span class="ts-fold-body">${highlightTypeScript(runtime)}</span>` +
    highlightTypeScript(code.slice(end + endMark.length));
}

/** Makes the folds inside `el` open and close on click (call once per element). */
export function bindFolds(el) {
  const toggle = (f) => {
    const open = f.classList.toggle('open');
    f.textContent = f.textContent.replace(open ? '▸' : '▾', open ? '▾' : '▸').replace(open ? 'click to show' : 'click to hide', open ? 'click to hide' : 'click to show');
  };
  el.addEventListener('click', (e) => { const f = e.target.closest('.ts-fold'); if (f) toggle(f); });
  el.addEventListener('keydown', (e) => { const f = e.target.closest('.ts-fold'); if (f && (e.key === 'Enter' || e.key === ' ')) { e.preventDefault(); toggle(f); } });
}
