# Strongtalk — Rust and WebAssembly edition

Strongtalk is Smalltalk with an optional type system: types are written in angle brackets
(`x <Int>`, `^<Str>`, `<[Int, ^Bool]>`, `Array[E]`) and checked without changing how code runs.
This folder re-implements the Strongtalk environment in Rust:

* `vm/` — the virtual machine in Rust: parser for the typed syntax, a compiler to bytecode,
  a bytecode interpreter (Smalltalk activations live on the heap, so recursion can go very deep),
  SmallInteger/LargeInteger/Float/Fraction arithmetic, blocks with non-local return, ANSI
  exceptions, primitives named after the original VM's (`{{self primitiveAdd: x ifFail: [...]}}`),
  file-in/file-out in Strongtalk's `Delta define:as:` chunk format, and a type checker.
* `vm/lib/*.dlt` — the class library, written in typed Strongtalk (about 70 classes).
* `web/` — the website: a home page with a live playground (`web/play.js`) and articles (`web/index.html`, `web/articles/`) about
  Strongtalk's history, type system and VM, and in `web/try/` the browser environment: Launcher with Transcript, workspaces, class hierarchy,
  outliner class browsers, inspectors, error windows, picture windows. The VM runs in a Web Worker.

## Online

**https://volland.github.io/strongtalk/** — the home page has a live workspace, and
[`/try/`](https://volland.github.io/strongtalk/try/) opens the full environment. Everything runs in the
visitor's browser: nothing is installed and no server is needed.

The site deploys automatically from `main` via GitHub Actions (`.github/workflows/pages.yml`).
The WebAssembly VM in `web/try/pkg` is committed prebuilt, so after changing the Rust code run
`npm run build` and commit the updated `web/try/pkg` before pushing.

## On the Mac

* Double-click **Strongtalk.command** for the terminal version (an interactive workspace).
  Or: `vm/target/release/strongtalk file.dlt`, `strongtalk -e '100 factorial'`,
  `strongtalk --typecheck`, `strongtalk --bench`.
* Double-click **Strongtalk Web.command** to open the WebAssembly website in your browser.

## Building

    npm run build          # WebAssembly VM + website into dist/
    npm run build:native   # the terminal program
    npm test               # language tests

Needs Rust with the `wasm32-unknown-unknown` target and `wasm-bindgen` 0.2.128 for the website.
