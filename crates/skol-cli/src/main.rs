//! `skol` — the Skol command-line tool.

#![forbid(unsafe_code)]

use std::io::{BufRead, Read, Write};
use std::process::ExitCode;

use skol_sema::{Checker, FsLoader};
use skol_vm::{compile, compile_into, Vm};

const USAGE: &str = "\
usage:
  skol <file.skl>              run a program (same as `skol run`)
  skol run <file.skl>          type-check, compile and run
  skol check <file.skl>        type-check only; errors as path:line:col: error: msg
  skol repl                    interactive prompt (also: `skol` with no arguments)
  skol ast <file.skl>          print the parsed syntax tree
  skol tokens <file.skl>       print the token stream
  skol ir <file.skl>           print the typed IR
  skol disasm <file.skl>       print the compiled bytecode

options:
  -                           read the program from stdin instead of a file
  --name <path>               file name to report (and import relative to) for stdin
  --seed <n>                  seed the random number generator";

struct Opts { cmd: String, path: Option<String>, name: Option<String>, seed: Option<u64> }

fn parse_args() -> Result<Opts, String> {
    let mut positional = Vec::new();
    let mut name = None;
    let mut seed = None;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--name" => name = Some(it.next().ok_or("--name needs a value")?),
            "--seed" => seed = Some(it.next().ok_or("--seed needs a value")?.parse().map_err(|_| "--seed needs a number")?),
            "-h" | "--help" => { println!("{USAGE}"); std::process::exit(0); }
            "-V" | "--version" => { println!("skol {}", env!("CARGO_PKG_VERSION")); std::process::exit(0); }
            _ => positional.push(a),
        }
    }
    let commands = ["run", "check", "repl", "ast", "tokens", "ir", "disasm"];
    let (cmd, path) = match positional.as_slice() {
        [] => ("repl".to_string(), None),
        [c] if commands.contains(&c.as_str()) => (c.clone(), None),
        [file] => ("run".to_string(), Some(file.clone())),
        [c, file] if commands.contains(&c.as_str()) => (c.clone(), Some(file.clone())),
        _ => return Err(USAGE.to_string()),
    };
    if cmd != "repl" && path.is_none() { return Err(format!("skol {cmd} needs a file\n\n{USAGE}")); }
    Ok(Opts { cmd, path, name, seed })
}

fn read_source(path: &str) -> Result<String, String> {
    if path == "-" {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s).map_err(|e| format!("skol: couldn't read stdin: {e}"))?;
        Ok(s)
    } else {
        std::fs::read_to_string(path).map_err(|e| format!("skol: couldn't read {path}: {e}"))
    }
}

fn main() -> ExitCode {
    let opts = match parse_args() {
        Ok(o) => o,
        Err(e) => { eprintln!("{e}"); return ExitCode::from(2); }
    };
    // A bug in Skol itself should never look like a crash in the user's
    // program: report it clearly instead of Rust's default panic output.
    std::panic::set_hook(Box::new(|info| {
        let msg = info.payload().downcast_ref::<&str>().map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown".into());
        let at = info.location().map(|l| format!(" at {}:{}", l.file(), l.line())).unwrap_or_default();
        eprintln!("skol: internal error: {msg}{at}\nThis is a bug in Skol, not in your program. Please report it with the code that caused it.");
    }));
    // Run on a thread with a large stack: signal handlers and zero-value
    // construction re-enter the VM recursively.
    let handle = std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(move || real_main(opts))
        .expect("spawn interpreter thread");
    let code = handle.join().unwrap_or(101);
    let _ = std::io::stdout().flush();
    ExitCode::from(code)
}

fn real_main(opts: Opts) -> u8 {
    if opts.cmd == "repl" { return repl(opts.seed); }
    let path = opts.path.clone().unwrap();
    let source = match read_source(&path) {
        Ok(s) => s,
        Err(e) => { eprintln!("{e}"); return 2; }
    };
    let shown = opts.name.clone().unwrap_or_else(|| if path == "-" { "<stdin>".into() } else { path.clone() });

    match opts.cmd.as_str() {
        "tokens" => match skol_syntax::tokenize(&source) {
            Ok(tokens) => { for t in tokens { println!("{}:{}\t{:?}", t.line, t.col, t.kind); } 0 }
            Err(e) => { println!("{shown}:{}:{}: error: {}", e.line, e.col, e.message); 1 }
        },
        "ast" => match skol_syntax::parse(&source) {
            Ok(p) => { println!("{p:#?}"); 0 }
            Err(e) => { println!("{shown}:{}:{}: error: {}", e.line, e.col, e.message); 1 }
        },
        _ => {
            let mut checker = Checker::new();
            if let Err(d) = checker.check_entry(&shown, &source, &mut FsLoader) {
                println!("{d}");
                return 1;
            }
            let prog = checker.into_program();
            match opts.cmd.as_str() {
                "check" => { println!("{shown}: ok"); 0 }
                "ir" => { println!("{prog:#?}"); 0 }
                "disasm" => match compile(&prog) {
                    Ok(bc) => { print!("{}", bc.disassemble()); 0 }
                    Err(e) => { println!("{shown}: error: {e}"); 1 }
                },
                _ => {
                    let bc = match compile(&prog) {
                        Ok(bc) => bc,
                        Err(e) => { println!("{shown}: error: {e}"); return 1; }
                    };
                    let mut vm = Vm::new(bc);
                    if let Some(s) = opts.seed { vm.seed(s); }
                    match vm.run_program() {
                        Ok(code) => (code & 0xff) as u8,
                        Err(e) => {
                            let _ = std::io::stdout().flush();
                            eprintln!("{e}");
                            70
                        }
                    }
                }
            }
        }
    }
}

/// Net count of open brackets, ignoring strings and comments, so the REPL
/// knows when a multi-line input is complete.
fn open_brackets(src: &str) -> i32 {
    let mut depth = 0;
    let mut in_str = false;
    let mut chars = src.chars();
    while let Some(c) = chars.next() {
        if in_str {
            match c { '\\' => { chars.next(); } '"' => in_str = false, _ => {} }
            continue;
        }
        match c {
            '"' => in_str = true,
            '#' => { for c2 in chars.by_ref() { if c2 == '\n' { break; } } }
            '{' | '(' | '[' => depth += 1,
            '}' | ')' | ']' => depth -= 1,
            _ => {}
        }
    }
    depth
}

fn repl(seed: Option<u64>) -> u8 {
    println!("Skol {} REPL — :quit to exit, :help for help", env!("CARGO_PKG_VERSION"));
    let mut checker = Checker::new();
    let mut vm = Vm::new(compile(checker.program()).unwrap_or_default());
    if let Some(s) = seed { vm.seed(s); }
    let stdin = std::io::stdin();
    let mut buf = String::new();
    loop {
        print!("{}", if buf.is_empty() { "skol> " } else { "...> " });
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => { println!(); return 0; }
            Ok(_) => {}
            Err(_) => return 1,
        }
        if buf.is_empty() {
            match line.trim() {
                "" => continue,
                ":q" | ":quit" | ":exit" => return 0,
                ":help" | ":h" => {
                    println!("Type Skol declarations or statements. Expressions are printed.");
                    println!("Multi-line input continues until brackets balance. Names can be redefined.");
                    println!("Imports load Name.skl from the current directory; `from math use sqrt, pi` is built in.");
                    continue;
                }
                _ => {}
            }
        }
        buf.push_str(&line);
        if open_brackets(&buf) > 0 { continue; }
        let src = std::mem::take(&mut buf);
        match checker.check_repl_chunk(&src, &mut FsLoader) {
            Err(d) => println!("{d}"),
            Ok(init) => {
                if let Err(e) = compile_into(checker.program(), vm.bytecode_mut()) {
                    println!("error: {e}");
                    continue;
                }
                vm.sync_globals();
                if let Err(e) = vm.call(init, vec![]) {
                    let _ = std::io::stdout().flush();
                    println!("{e}");
                }
            }
        }
    }
}
