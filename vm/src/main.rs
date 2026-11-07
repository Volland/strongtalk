//! The `strongtalk` command: an interactive workspace in the terminal, plus file-in and
//! type checking for scripts.
//!
//!   strongtalk                     interactive: type expressions, end with an empty line
//!   strongtalk file.dlt ...        file in each file (chunk format) and run its do-its
//!   strongtalk -e '3 + 4'          print the value of one expression
//!   strongtalk --typecheck [Class] type-check a class (or every class)
//!   strongtalk --bench             run Benchmark runAll
//!   strongtalk --to-ts file.dlt    translate Strongtalk to TypeScript (-e 'code' for a do-it,
//!                                  -o out.ts to write a file)
//!   strongtalk web                 open the Strongtalk environment in the browser (local server)

mod localweb;

use serde_json::Value as J;
use std::io::{self, BufRead, Write};
use strongtalk::Vm;

fn print_result(vm: &mut Vm, src: &str) -> bool {
    let out: J = serde_json::from_str(&vm.eval(src, "terminal", true)).unwrap_or(J::Null);
    if let Some(t) = out["transcript"].as_str() {
        print!("{}", t);
    }
    for ev in out["events"].as_array().cloned().unwrap_or_default() {
        if ev["type"] == "canvas" {
            println!("[canvas \"{}\" {}x{} drawn - open the web version to see pictures]", ev["title"].as_str().unwrap_or(""), ev["width"], ev["height"]);
        } else if ev["type"] == "inspect" {
            let id = ev["id"].as_u64().unwrap_or(0) as usize;
            let ins: J = serde_json::from_str(&vm.inspect(id)).unwrap_or(J::Null);
            println!("Inspector on {} ({})", ins["title"].as_str().unwrap_or(""), ins["class"].as_str().unwrap_or(""));
            for f in ins["fields"].as_array().cloned().unwrap_or_default() {
                println!("  {:>12}: {}", f["name"].as_str().unwrap_or(""), f["value"].as_str().unwrap_or(""));
            }
        }
    }
    if out["ok"] == true {
        println!("{}", out["value"].as_str().unwrap_or(""));
        true
    } else {
        println!("Error: {}", out["error"].as_str().unwrap_or("?"));
        for (i, s) in out["stack"].as_array().cloned().unwrap_or_default().iter().enumerate() {
            if i >= 12 {
                break;
            }
            println!("    {}", s.as_str().unwrap_or(""));
        }
        false
    }
}

fn print_problems(out: &str) -> usize {
    let j: J = serde_json::from_str(out).unwrap_or(J::Null);
    let ps = j["problems"].as_array().cloned().unwrap_or_default();
    for p in &ps {
        println!(
            "{}{}>>{} (line {}): {}",
            p["class"].as_str().unwrap_or(""),
            if p["meta"] == true { " class" } else { "" },
            p["selector"].as_str().unwrap_or(""),
            p["line"],
            p["message"].as_str().unwrap_or("")
        );
    }
    ps.len()
}

fn repl(vm: &mut Vm) {
    println!("Strongtalk 2.0 - Rust edition. Type an expression; an empty line runs it. Ctrl-D quits.");
    println!("Try:  3 + 4 * 5     100 factorial     #(3 1 2) asSortedCollection     Benchmark runAll");
    let stdin = io::stdin();
    let mut buf = String::new();
    loop {
        print!("{}", if buf.is_empty() { "st> " } else { "... " });
        io::stdout().flush().ok();
        let mut line = String::new();
        if stdin.lock().read_line(&mut line).unwrap_or(0) == 0 {
            if !buf.trim().is_empty() {
                print_result(vm, &buf);
            }
            println!();
            break;
        }
        let complete_line = line.trim_end().ends_with('.') || line.trim().is_empty();
        buf.push_str(&line);
        // single-line expressions run right away; longer ones wait for an empty line
        if line.trim().is_empty() || (!complete_line && buf.lines().count() == 1 && balanced(&buf)) {
            if !buf.trim().is_empty() {
                print_result(vm, &buf);
            }
            buf.clear();
        }
    }
}

fn balanced(s: &str) -> bool {
    let mut depth = 0i32;
    let mut in_str = false;
    let mut in_comment = false;
    let mut prev = ' ';
    for c in s.chars() {
        if in_str {
            if c == '\'' {
                in_str = false;
            }
        } else if in_comment {
            if c == '"' {
                in_comment = false;
            }
        } else if prev == '$' {
        } else {
            match c {
                '\'' => in_str = true,
                '"' => in_comment = true,
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                _ => {}
            }
        }
        prev = if prev == '$' && !in_str && !in_comment { ' ' } else { c };
    }
    depth <= 0 && !in_str && !in_comment
}

/// `strongtalk --to-ts [file.dlt | -e code | -] [-o out.ts]`: no VM needed, only the parser.
fn to_typescript(args: &[String]) -> i32 {
    let mut src: Option<String> = None;
    let mut out: Option<String> = None;
    let mut name = String::from("input");
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-e" => {
                i += 1;
                src = args.get(i).cloned();
                name = "expression".into();
            }
            "-o" => {
                i += 1;
                out = args.get(i).cloned();
            }
            "-" => {
                let mut s = String::new();
                if io::Read::read_to_string(&mut io::stdin(), &mut s).is_err() {
                    eprintln!("cannot read standard input");
                    return 1;
                }
                src = Some(s);
                name = "stdin".into();
            }
            path => match std::fs::read_to_string(path) {
                Ok(s) => {
                    src = Some(s);
                    name = path.to_string();
                }
                Err(e) => {
                    eprintln!("cannot read {}: {}", path, e);
                    return 1;
                }
            },
        }
        i += 1;
    }
    let src = match src {
        Some(s) => s,
        None => {
            eprintln!("usage: strongtalk --to-ts file.dlt [-o out.ts]\n       strongtalk --to-ts -e '(1 to: 10) collect: [:i | i * i]'");
            return 1;
        }
    };
    let t = strongtalk::typescript::translate(&src);
    if !t.ok {
        eprintln!("{}:{}:{}: syntax error: {}", name, t.line, t.column, t.error.unwrap_or_default());
        return 1;
    }
    for w in &t.warnings {
        eprintln!("note: {}", w);
    }
    match out {
        Some(path) => {
            if let Err(e) = std::fs::write(&path, &t.code) {
                eprintln!("cannot write {}: {}", path, e);
                return 1;
            }
            eprintln!("wrote {}", path);
        }
        None => print!("{}", t.code),
    }
    0
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(|a| a.as_str()) {
        Some("--to-ts") | Some("ts") => std::process::exit(to_typescript(&args[1..])),
        Some("web") | Some("--web") => std::process::exit(localweb::serve(&args[1..])),
        Some("--version") | Some("-V") => {
            println!("strongtalk {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        _ => {}
    }
    let child = std::thread::Builder::new()
        .stack_size(512 * 1024 * 1024)
        .spawn(move || run(args))
        .expect("start interpreter thread");
    let code = child.join().unwrap_or(1);
    std::process::exit(code);
}

fn run(args: Vec<String>) -> i32 {
    let mut vm = Vm::new();
    vm.it.max_depth = 1_000_000;
    vm.it.depth_limit = 1_000_000;
    vm.it.max_run_depth = 20_000;
    if !vm.boot_errors.is_empty() {
        eprintln!("warning: the class library loaded with {} problem(s):", vm.boot_errors.len());
        for e in vm.boot_errors.iter().take(20) {
            eprintln!("  {}", e);
        }
    }
    if args.is_empty() {
        repl(&mut vm);
        return 0;
    }
    let mut i = 0;
    let mut code = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                println!("Strongtalk {} - the typed Smalltalk, in Rust\n", env!("CARGO_PKG_VERSION"));
                println!("usage:");
                println!("  strongtalk                        interactive workspace (an empty line runs the code)");
                println!("  strongtalk file.dlt ...           file in source files and run their do-its");
                println!("  strongtalk -e '3 + 4'             print the value of an expression");
                println!("  strongtalk --typecheck [Class]    type-check one class or all classes");
                println!("  strongtalk --bench                run the benchmarks");
                println!("  strongtalk --to-ts file.dlt [-o out.ts]   translate Strongtalk to TypeScript");
                println!("  strongtalk --to-ts -e 'code'      translate a snippet to TypeScript");
                println!("  strongtalk web [translate]        open the environment (or translator) in the browser");
                println!("  strongtalk --version");
            }
            "-e" => {
                i += 1;
                if let Some(src) = args.get(i) {
                    if !print_result(&mut vm, src) {
                        code = 1;
                    }
                }
            }
            "--typecheck" => {
                let target = args.get(i + 1).filter(|a| !a.starts_with('-')).cloned();
                let out = match &target {
                    Some(c) => {
                        i += 1;
                        vm.typecheck_class(c)
                    }
                    None => vm.typecheck_all(),
                };
                let n = print_problems(&out);
                println!("{} type problem(s)", n);
                if n > 0 {
                    code = 2;
                }
            }
            "--bench" => {
                print_result(&mut vm, "Benchmark runAll");
            }
            "-i" => repl(&mut vm),
            path => match std::fs::read_to_string(path) {
                Ok(text) => {
                    let out: J = serde_json::from_str(&vm.file_in(&text)).unwrap_or(J::Null);
                    print!("{}", out["transcript"].as_str().unwrap_or(""));
                    for e in out["errors"].as_array().cloned().unwrap_or_default() {
                        eprintln!("{}: {}", path, e.as_str().unwrap_or(""));
                        code = 1;
                    }
                }
                Err(e) => {
                    eprintln!("cannot read {}: {}", path, e);
                    code = 1;
                }
            },
        }
        i += 1;
    }
    code
}
