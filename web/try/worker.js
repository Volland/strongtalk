// Runs the Strongtalk virtual machine (Rust compiled to WebAssembly) off the page's thread,
// so a long or endless computation never freezes the window and can be interrupted.
import init, { Strongtalk, translateToTypeScript } from './pkg/strongtalk.js';

let vm = null;
const NESTING_PER_FRAME = 0.05;

// Smalltalk calls nest on the heap, but exception handlers and ensure: blocks nest on the
// browser's machine stack, which is small in a worker. Measure it with plain JavaScript
// recursion and cap that nesting with a safety margin.
function jsStackDepth() {
  let d = 0;
  const f = () => { d++; f(); };
  try { f(); } catch { /* reached the limit */ }
  return d;
}
const maxNesting = Math.max(40, Math.min(5000, Math.floor(jsStackDepth() * NESTING_PER_FRAME)));

const ready = (async () => {
  await init();
  vm = new Strongtalk();
  vm.setMaxNesting(maxNesting);
  vm.setTranscript((text) => postMessage({ type: 'transcript', text }));
})();

function call(op, a) {
  switch (op) {
    case 'boot': return JSON.stringify({ errors: JSON.parse(vm.bootErrors()), maxNesting });
    case 'replay': {
      vm.setRecordChanges(false);
      const r = vm.fileIn(a[0]);
      vm.setRecordChanges(true);
      return r;
    }
    case 'eval': return vm.eval(a[0], a[1], a[2]);
    case 'fileIn': return vm.fileIn(a[0]);
    case 'classes': return vm.classes();
    case 'classInfo': return vm.classInfo(a[0]);
    case 'method': return vm.method(a[0], a[1], a[2]);
    case 'compile': return vm.compile(a[0], a[1], a[2], a[3]);
    case 'removeMethod': return vm.removeMethod(a[0], a[1], a[2]);
    case 'setComment': return vm.setComment(a[0], a[1]);
    case 'typecheckClass': return vm.typecheckClass(a[0]);
    case 'typecheckAll': return vm.typecheckAll();
    case 'typecheckDoit': return vm.typecheckDoit(a[0]);
    case 'senders': return vm.senders(a[0]);
    case 'implementors': return vm.implementors(a[0]);
    case 'fileOut': return JSON.stringify(vm.fileOut(a[0]));
    case 'inspect': return vm.inspect(a[0]);
    case 'toTypeScript': return translateToTypeScript(a[0]);
    case 'classToTypeScript': return translateToTypeScript(vm.fileOut(a[0]));
    default: throw new Error('unknown operation ' + op);
  }
}

onmessage = async (ev) => {
  const { id, op, args } = ev.data;
  try {
    await ready;
    const result = JSON.parse(call(op, args || []));
    const changes = vm.takeChanges();
    const canvases = [];
    for (const e of (result && result.events) || []) {
      if (e.type === 'canvas') canvases.push({ id: e.id, pixels: vm.canvasPixels(e.id) });
    }
    postMessage({ type: 'reply', id, result, changes, canvases });
  } catch (err) {
    // A JavaScript-level failure inside WebAssembly (the browser's stack ran out, or the VM
    // stopped on an internal error) leaves the VM unusable: the page restarts it.
    postMessage({ type: 'reply', id, error: String(err && err.message || err), fatal: true });
  }
};
