#!/usr/bin/env python3
"""Writes THIRD-PARTY-NOTICES.txt: the license texts of the Rust crates the program is built from
(native and WebAssembly). Run after changing dependencies:  python3 scripts/third-party-notices.py"""
import json, os, subprocess, sys

root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
manifest = os.path.join(root, 'vm', 'Cargo.toml')

def packages(extra):
    out = subprocess.run(['cargo', 'metadata', '--format-version', '1', '--manifest-path', manifest] + extra,
                         capture_output=True, text=True, check=True).stdout
    d = json.loads(out)
    used = {n['id'] for n in d['resolve']['nodes']}
    return {p['name']: p for p in d['packages'] if p['id'] in used and p['name'] != 'strongtalk'}

pkgs = packages([])
pkgs.update(packages(['--filter-platform', 'wasm32-unknown-unknown']))

PREFER = ['LICENSE-MIT', 'LICENSE-MIT.md', 'LICENSE.md', 'LICENSE', 'COPYING', 'LICENSE.txt', 'UNLICENSE']
lines = ['THIRD-PARTY NOTICES', '===================', '',
         'Strongtalk (Rust edition) is built with the following open-source Rust crates. Where a crate',
         'offers a choice of licenses, it is used under the MIT license; that license text is reproduced',
         'below with the crate\'s copyright notice.', '']
for name in sorted(pkgs):
    p = pkgs[name]
    d = os.path.dirname(p['manifest_path'])
    text = None
    for f in PREFER:
        path = os.path.join(d, f)
        if os.path.isfile(path):
            text = open(path, encoding='utf-8', errors='replace').read().strip()
            break
    lines += ['-' * 79, f"{name} {p['version']}  ({p['license']})", p.get('repository') or '', '']
    lines.append(text or f"Licensed under {p['license']}.")
    extra = os.path.join(d, 'LICENSE-UNICODE')
    if os.path.isfile(extra):
        lines += ['', open(extra, encoding='utf-8').read().strip()]
    lines.append('')
open(os.path.join(root, 'THIRD-PARTY-NOTICES.txt'), 'w', encoding='utf-8').write('\n'.join(lines) + '\n')
print(f'wrote THIRD-PARTY-NOTICES.txt ({len(pkgs)} crates)', file=sys.stderr)
