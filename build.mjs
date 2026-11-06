// Builds the website into dist/: compiles the Rust VM to WebAssembly, generates its
// JavaScript binding with wasm-bindgen, and copies the page files next to it.
// If Rust is not available, a previously built copy in web/pkg is used.
import { execFileSync } from 'node:child_process';
import { cpSync, existsSync, mkdirSync, rmSync, readdirSync, statSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = dirname(fileURLToPath(import.meta.url));
const vmDir = join(root, 'vm');
const webDir = join(root, 'web');
const pkgDir = join(webDir, 'try', 'pkg');
const dist = join(root, 'dist');

function run(cmd, args, cwd) {
  console.log('> ' + [cmd, ...args].join(' '));
  execFileSync(cmd, args, { cwd, stdio: 'inherit' });
}

let built = false;
try {
  run('cargo', ['build', '--release', '--target', 'wasm32-unknown-unknown', '--lib'], vmDir);
  const wasm = join(vmDir, 'target', 'wasm32-unknown-unknown', 'release', 'strongtalk.wasm');
  rmSync(pkgDir, { recursive: true, force: true });
  run('wasm-bindgen', ['--target', 'web', '--no-typescript', '--out-dir', pkgDir, wasm], root);
  built = true;
} catch (e) {
  if (!existsSync(join(pkgDir, 'strongtalk_bg.wasm'))) {
    console.error('Could not build the WebAssembly VM (is Rust with the wasm32-unknown-unknown target and wasm-bindgen 0.2.128 installed?)');
    process.exit(1);
  }
  console.warn('Rust build unavailable; using the previously built VM in web/pkg.');
}

rmSync(dist, { recursive: true, force: true });
mkdirSync(dist, { recursive: true });
cpSync(webDir, dist, { recursive: true });
// the Strongtalk sources of the class library, for reading or downloading
cpSync(join(vmDir, 'lib'), join(dist, 'try', 'lib'), { recursive: true });

const size = (d) => readdirSync(d).reduce((n, f) => {
  const p = join(d, f);
  return n + (statSync(p).isDirectory() ? size(p) : statSync(p).size);
}, 0);
console.log(`dist/ ready (${(size(dist) / 1024).toFixed(0)} KB)${built ? '' : ' [prebuilt VM]'}`);
