//! Embeds the website (../web) into the native `strongtalk` program, so `strongtalk web` can serve
//! the whole environment offline. The WebAssembly library build ignores the generated list.
use std::fs;
use std::path::{Path, PathBuf};

fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.path());
    for e in entries {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        if p.is_dir() {
            walk(&p, base, out);
        } else {
            let rel = p.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
            out.push((rel, fs::canonicalize(&p).unwrap_or(p)));
        }
    }
}

fn main() {
    let web = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("web");
    println!("cargo:rerun-if-changed={}", web.display());
    let mut files = vec![];
    walk(&web, &web, &mut files);
    for (_, p) in &files {
        println!("cargo:rerun-if-changed={}", p.display());
    }
    // the license files, served next to the site as in the online version
    let top = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    for (src, served) in [("LICENSE", "LICENSE.txt"), ("LICENSE-0BSD", "LICENSE-0BSD.txt"), ("THIRD-PARTY-NOTICES.txt", "THIRD-PARTY-NOTICES.txt")] {
        let p = top.join(src);
        if p.is_file() {
            println!("cargo:rerun-if-changed={}", p.display());
            files.push((served.to_string(), fs::canonicalize(&p).unwrap_or(p)));
        }
    }
    let mut code = String::from("pub static WEB_ASSETS: &[(&str, &[u8])] = &[\n");
    for (rel, p) in &files {
        code.push_str(&format!("    ({:?}, include_bytes!({:?})),\n", rel, p.display().to_string()));
    }
    code.push_str("];\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("web_assets.rs");
    fs::write(out, code).unwrap();
}
