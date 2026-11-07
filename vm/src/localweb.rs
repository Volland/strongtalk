//! `strongtalk web`: serves the Strongtalk website and environment, embedded in this program,
//! on http://127.0.0.1 and opens it in the browser. Everything runs locally and offline.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};

include!(concat!(env!("OUT_DIR"), "/web_assets.rs"));

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "wasm" => "application/wasm",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "sh" | "ps1" | "dlt" | "txt" | "md" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn find(path: &str) -> Option<(&'static str, &'static [u8])> {
    let p = path.split(['?', '#']).next().unwrap_or("").trim_start_matches('/');
    let p = percent_decode(p);
    let candidates = if p.is_empty() || p.ends_with('/') { vec![format!("{}index.html", p)] } else { vec![p.clone(), format!("{}/index.html", p)] };
    for c in candidates {
        if let Some((name, data)) = WEB_ASSETS.iter().find(|(n, _)| *n == c) {
            return Some((name, data));
        }
    }
    None
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn handle(mut stream: TcpStream) {
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    });
    let mut request = String::new();
    if reader.read_line(&mut request).is_err() {
        return;
    }
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) if line == "\r\n" || line == "\n" => break,
            _ => {}
        }
    }
    let mut parts = request.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("/");
    let (status, ctype, body): (&str, &str, &[u8]) = if method != "GET" && method != "HEAD" {
        ("405 Method Not Allowed", "text/plain", b"method not allowed")
    } else if let Some((name, data)) = find(path) {
        // a directory without its trailing slash: redirect so relative links work
        let p = path.split(['?', '#']).next().unwrap_or("");
        if name.ends_with("index.html") && !p.ends_with('/') && !p.ends_with(".html") && p != "/" {
            let _ = write!(stream, "HTTP/1.1 301 Moved Permanently\r\nLocation: {}/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", p);
            return;
        }
        ("200 OK", content_type(name), data)
    } else {
        ("404 Not Found", "text/plain; charset=utf-8", b"not found")
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nCross-Origin-Opener-Policy: same-origin\r\nConnection: close\r\n\r\n",
        status,
        ctype,
        body.len()
    );
    if method != "HEAD" {
        let _ = stream.write_all(body);
    }
}

fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let r = std::process::Command::new("xdg-open").arg(url).spawn();
    if r.is_err() {
        println!("Open {} in your browser.", url);
    }
}

/// `strongtalk web [--port N] [--no-open] [page]` (page: try, translate, or a path).
pub fn serve(args: &[String]) -> i32 {
    if WEB_ASSETS.is_empty() {
        eprintln!("this build of strongtalk has no website embedded");
        return 1;
    }
    let mut port: u16 = 8642;
    let mut open = true;
    let mut page = String::from("try/");
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--port" | "-p" => {
                i += 1;
                port = args.get(i).and_then(|p| p.parse().ok()).unwrap_or(port);
            }
            "--no-open" => open = false,
            "translate" | "translator" | "ts" => page = "translate/".into(),
            "home" | "site" => page = String::new(),
            "try" | "environment" => page = "try/".into(),
            other => page = other.trim_start_matches('/').to_string(),
        }
        i += 1;
    }
    let mut listener = None;
    for p in port..port.saturating_add(20) {
        if let Ok(l) = TcpListener::bind(("127.0.0.1", p)) {
            port = p;
            listener = Some(l);
            break;
        }
    }
    let Some(listener) = listener else {
        eprintln!("could not open a local port near {}", port);
        return 1;
    };
    let url = format!("http://127.0.0.1:{}/{}", port, page);
    println!("Strongtalk is running at {}", url);
    println!("  environment: http://127.0.0.1:{}/try/", port);
    println!("  translator:  http://127.0.0.1:{}/translate/", port);
    println!("Press Ctrl+C to stop.");
    if open {
        open_browser(&url);
    }
    for stream in listener.incoming().flatten() {
        std::thread::spawn(move || handle(stream));
    }
    0
}
