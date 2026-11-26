//! `skol` — the Skol command-line tool.

#![forbid(unsafe_code)]

use std::io::{BufRead, Read, Write};
use std::process::ExitCode;

use skol_sema::{Checker, FsLoader};
use skol_vm::{compile, compile_into, Vm};

const USAGE: &str = "\
usage:
  skol <file.skl> [args...]        run a program (same as `skol run`); use -- before
                                   script arguments that look like skol options
  skol run <file.skl> [args...]    type-check, compile and run; args go to the script (os.arg)
  skol check <file.skl>            type-check only; errors as path:line:col: error: msg
  skol repl                        interactive prompt (also: `skol` with no arguments)
  skol ast <file.skl>              print the parsed syntax tree
  skol tokens <file.skl>           print the token stream
  skol ir <file.skl>               print the typed IR
  skol disasm <file.skl>           print the compiled bytecode

options:
  -                           read the program from stdin instead of a file
  --name <path>               file name to report (and import relative to) for stdin
  --seed <n>                  seed the random number generator
  --lib <dir>                 also look for imported modules in <dir> (repeatable)

Imports are found next to the importing file, then in each --lib folder,
each folder in SKOL_PATH, and the standard library (lib/ beside skol).";

struct Opts { cmd: String, path: Option<String>, name: Option<String>, seed: Option<u64>, libs: Vec<String>, script_args: Vec<String> }

fn parse_args() -> Result<Opts, String> {
    let commands = ["run", "check", "repl", "ast", "tokens", "ir", "disasm"];
    let mut cmd: Option<String> = None;
    let mut path: Option<String> = None;
    let mut name = None;
    let mut seed = None;
    let mut libs = Vec::new();
    let mut script_args = Vec::new();
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        if a == "--" && path.is_some() { script_args.extend(it.by_ref()); break; }
        match a.as_str() {
            "--name" => name = Some(it.next().ok_or("--name needs a value")?),
            "--seed" => seed = Some(it.next().ok_or("--seed needs a value")?.parse().map_err(|_| "--seed needs a number")?),
            "--lib" => libs.push(it.next().ok_or("--lib needs a folder")?),
            "-h" | "--help" => { println!("{USAGE}"); std::process::exit(0); }
            "-V" | "--version" => { println!("skol {}", env!("CARGO_PKG_VERSION")); std::process::exit(0); }
            c if cmd.is_none() && path.is_none() && commands.contains(&c) => cmd = Some(c.to_string()),
            _ if path.is_none() => path = Some(a),
            _ => script_args.push(a),
        }
    }
    let cmd = cmd.unwrap_or_else(|| if path.is_some() { "run".into() } else { "repl".into() });
    if cmd != "repl" && path.is_none() { return Err(format!("skol {cmd} needs a file\n\n{USAGE}")); }
    if cmd != "run" && !script_args.is_empty() { return Err(format!("only `skol run` passes arguments to the script\n\n{USAGE}")); }
    Ok(Opts { cmd, path, name, seed, libs, script_args })
}

fn loader(libs: &[String]) -> FsLoader {
    let std = FsLoader::standard();
    let mut search: Vec<std::path::PathBuf> = libs.iter().map(std::path::PathBuf::from).collect();
    search.extend(std.search);
    FsLoader { search }
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
    if opts.cmd == "repl" { return repl(opts.seed, &opts.libs); }
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
            if let Err(d) = checker.check_entry(&shown, &source, &mut loader(&opts.libs)) {
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
                    vm.args = opts.script_args.clone();
                    match vm.run_program() {
                        Ok(code) => (code & 0xff) as u8,
                        Err(e) if e.exit_code.is_some() => (e.exit_code.unwrap() & 0xff) as u8,
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

fn repl(seed: Option<u64>, libs: &[String]) -> u8 {
    let mut modules = loader(libs);
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
                    println!("Imports load Name.skl from the current directory or the library path; math, random, time, os and io are built in.");
                    continue;
                }
                _ => {}
            }
        }
        buf.push_str(&line);
        if open_brackets(&buf) > 0 { continue; }
        let src = std::mem::take(&mut buf);
        match checker.check_repl_chunk(&src, &mut modules) {
            Err(d) => println!("{d}"),
            Ok(init) => {
                if let Err(e) = compile_into(checker.program(), vm.bytecode_mut()) {
                    println!("error: {e}");
                    continue;
                }
                vm.sync_globals();
                if let Err(e) = vm.call(init, vec![]) {
                    let _ = std::io::stdout().flush();
                    if let Some(code) = e.exit_code { return (code & 0xff) as u8; }
                    println!("{e}");
                }
            }
        }
    }
}
