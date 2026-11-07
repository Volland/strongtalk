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
* `web/translate/` — the Strongtalk → TypeScript translator page; `web/download/` — downloads; `web/highlight.js` —
  syntax highlighting for Strongtalk and TypeScript, used by every page and the environment's editors.
* `packaging/` and `.github/workflows/release.yml` — the installers, built and published for a `v*` tag.
* `web/` — the website: a home page with a live playground (`web/play.js`) and articles (`web/index.html`, `web/articles/`) about
  Strongtalk's history, type system and VM, and in `web/try/` the browser environment: Launcher with Transcript, workspaces, class hierarchy,
  outliner class browsers, inspectors, error windows, picture windows. The VM runs in a Web Worker.

## Download

Installers for every system are on the [download page](https://volland.github.io/strongtalk/download/) and
[GitHub Releases](https://github.com/Volland/strongtalk/releases): a universal `.pkg` for macOS, a setup program and a
portable zip for Windows, and static `.deb` packages and archives for Linux (x64 and arm64). Or in a terminal:

    curl -fsSL https://volland.github.io/strongtalk/install.sh | sh        # macOS, Linux
    irm https://volland.github.io/strongtalk/install.ps1 | iex             # Windows PowerShell

The one program contains everything:

    strongtalk                          # the terminal workspace
    strongtalk web                      # the full environment, served locally and opened in your browser (offline)
    strongtalk web translate            # the TypeScript translator page
    strongtalk --to-ts shapes.dlt -o shapes.ts      # translate Strongtalk to TypeScript
    strongtalk --to-ts -e '(1 to: 10) collect: [:i <Int> | i * i]'

## Strongtalk → TypeScript

`vm/src/typescript.rs` translates a file-in (classes) or workspace code to one self-contained TypeScript file.
Type annotations become TypeScript types, control messages with literal blocks become `if`/`for`/`while`/`try`,
collection messages become array methods, and a `^` inside a block returns from its method as in Smalltalk. The declared
types choose direct translations; untyped receivers go through a small runtime prelude, emitted only as far as it is used.
It is available in the console (`--to-ts`), in the environment (the **→ TypeScript** buttons in workspaces and class
browsers), and on the website at [/translate/](https://volland.github.io/strongtalk/translate/), where **Run both**
runs the original in the VM and the translation as TypeScript, side by side.

`scripts/check-typescript.sh` runs every case in `vm/tests/typescript/` in the VM and as translated TypeScript (Node
22+), compares the output and type-checks each translation with `tsc`. Known differences: numbers are JavaScript
numbers, characters and symbols become strings, and OrderedCollections become arrays.

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

## License

Free software under the **BSD 3-Clause license** ([LICENSE](LICENSE)), the license Sun Microsystems released
Strongtalk under in 2006; Sun's copyright notice is kept because this edition was made by studying the original
system. The parts that are entirely new work (the website, the TypeScript translator, the highlighter, packaging and
scripts) may also be used under the **Zero-Clause BSD license** ([LICENSE-0BSD](LICENSE-0BSD)), with no conditions at
all. The Rust libraries the program is built with are MIT / Apache-2.0 licensed: see
[THIRD-PARTY-NOTICES.txt](THIRD-PARTY-NOTICES.txt) (regenerate with `python3 scripts/third-party-notices.py`).
`strongtalk --license` prints all of it.

## Building

    npm run build          # WebAssembly VM + website into dist/
    npm run build:native   # the native program (embeds web/, so run npm run build first after web changes)
    npm test               # language tests
    scripts/check-typescript.sh   # TypeScript translations against the VM

Needs Rust with the `wasm32-unknown-unknown` target and `wasm-bindgen` 0.2.128 for the website.
