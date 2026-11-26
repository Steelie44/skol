//! Robustness fuzzing: Skol must never panic or overflow the stack, whatever
//! it is fed. Runs random token streams, mutated versions of every real Skol
//! program in the repository, and targeted stress inputs through the whole
//! pipeline (parse -> check -> compile -> run with a step budget).
//!
//! `SKOL_FUZZ_CASES=20000 cargo test --release --test fuzz` runs a longer session.

use std::panic::{self, AssertUnwindSafe};
use std::path::Path;

use skol_sema::{Checker, ModuleLoader};
use skol_vm::{compile, Vm};

struct NoFiles;
impl ModuleLoader for NoFiles {
    fn load(&mut self, _importer: &str, name: &str) -> Option<Result<(String, String), String>> {
        // A tiny in-memory module so imports get exercised too.
        if name == "Lib" {
            return Some(Ok(("Lib.skl".into(), "comp Hp { hp: i = 3 }\nfn twice(x: i) -> i { return x * 2 }\nsignal ping(a)\n".into())));
        }
        None
    }
}

/// Run one program through everything. Errors are fine; panics are not.
fn exercise(src: &str) {
    let mut checker = Checker::new();
    if checker.check_entry("fuzz.skl", src, &mut NoFiles).is_err() { return; }
    let prog = checker.into_program();
    let Ok(bc) = compile(&prog) else { return };
    let _ = bc.disassemble();
    let mut vm = Vm::with_output(bc, Box::new(|_| {}));
    vm.allow_fs = false;
    vm.allow_os = false;
    vm.set_input(Box::new(|| None));
    vm.seed(7);
    vm.limits.max_steps = Some(20_000);
    vm.limits.max_depth = 400;
    let _ = vm.run_program();
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 { self.0 ^= self.0 << 13; self.0 ^= self.0 >> 7; self.0 ^= self.0 << 17; self.0 }
    fn below(&mut self, n: usize) -> usize { if n == 0 { 0 } else { (self.next() % n as u64) as usize } }
    fn pick<'a>(&mut self, xs: &'a [&'a str]) -> &'a str { xs[self.below(xs.len())] }
}

const TOKENS: &[&str] = &[
    "const", "var", "fn", "class", "comp", "signal", "from", "use", "if", "else", "for", "in", "while",
    "return", "pass", "break", "continue", "true", "false", "and", "or", "not", "lambda",
    "i", "f", "s", "bool", "void", "vec2", "vec3", "vec4", "T", "T4", "T1", "T99999999",
    "x", "y", "self", "print", "range", "str", "int", "float", "main", "Lib", "Hp", "twice", "ping",
    "0", "1", "-1", "2", "9223372036854775807", "1.5", "0.0", "\"s\"", "\"\"",
    ":", ":=", ",", ".", "->", "{", "}", "(", ")", "[", "]", "=", "+=", "-=", "+", "-", "*", "/", "%",
    "==", "!=", "<", "<=", ">", ">=", "iter", "pos", "edit", "view", "len", "sum", "get", "set", "push", "pop",
    "\n",
];

const EDGE_NUMBERS: &[&str] = &["0", "-1", "1", "9223372036854775807", "-9223372036854775807", "4294967296", "0.0", "1e308", "65536"];

fn corpus() -> Vec<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    for dir in [root.join("tests/programs"), root.join("../../examples"), root.join("../../examples/game"), root.join("../../lib"), root.join("../../lib/tests")] {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut paths: Vec<_> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
        paths.sort();
        for p in paths {
            if p.extension().and_then(|e| e.to_str()) == Some("skl") {
                if let Ok(s) = std::fs::read_to_string(&p) { out.push(s); }
            }
        }
    }
    out
}

fn mutate(rng: &mut Rng, src: &str) -> String {
    let mut lines: Vec<String> = src.lines().map(String::from).collect();
    for _ in 0..1 + rng.below(3) {
        if lines.is_empty() { break; }
        let i = rng.below(lines.len());
        match rng.below(8) {
            0 => { lines.remove(i); }
            1 => { let l = lines[i].clone(); lines.insert(i, l); }
            2 => { let j = rng.below(lines.len()); lines.swap(i, j); }
            3 => {
                let l = &mut lines[i];
                if !l.is_empty() {
                    let mut a = rng.below(l.len());
                    while !l.is_char_boundary(a) { a -= 1; }
                    let mut b = (a + 1 + rng.below(8)).min(l.len());
                    while !l.is_char_boundary(b) { b -= 1; }
                    l.replace_range(a..b, "");
                }
            }
            4 => {
                let l = &mut lines[i];
                let mut a = rng.below(l.len() + 1);
                while !l.is_char_boundary(a) { a -= 1; }
                let t = rng.pick(TOKENS);
                l.insert_str(a, &format!(" {t} "));
            }
            5 => {
                // Swap a number for an edge value.
                let words: Vec<String> = lines[i].split(' ').map(|w| {
                    if w.chars().all(|c| c.is_ascii_digit()) && !w.is_empty() { rng.pick(EDGE_NUMBERS).to_string() } else { w.to_string() }
                }).collect();
                lines[i] = words.join(" ");
            }
            6 => { lines.truncate(i); }
            _ => { let j = rng.below(lines.len()); let l = lines[j].clone(); lines[i] = l; }
        }
    }
    lines.join("\n")
}

fn stress_inputs() -> Vec<String> {
    let deep = 20_000;
    let mut v = vec![
        format!("var x: i = {}1{}", "(".repeat(deep), ")".repeat(deep)),
        format!("var x: i = {}1", "-".repeat(deep)),
        format!("var b: bool = {}true", "not ".repeat(deep)),
        format!("var t: {}i{} = {{}}", "T2[".repeat(deep), "]".repeat(deep)),
        format!("{}pass{}", "if true { ".repeat(deep), " }".repeat(deep)),
        format!("if true {{ pass }}{}", " else if true { pass }".repeat(deep)),
        format!("var f: {}i = 0", "fn() -> ".repeat(deep)),
        format!("print({})", "1 + ".repeat(deep) + "1"),
        format!("var t: T4[i] = {}", "{".repeat(deep)),
        format!("print({})", "\"a\" + ".repeat(5_000) + "\"b\""),
        format!("print({})", vec!["1"; 70_000].join(", ")),
        format!("fn g({}) {{ pass }}", (0..300).map(|k| format!("p{k}: i")).collect::<Vec<_>>().join(", ")),
        "var s1: s = \"ab\"\nwhile true { s1 = s1 + s1 }".into(),
        "print(\"x\".repeat(9223372036854775807))".into(),
        "print(\"abc\".replace(\"\", \"xxxxxxxxxxxxxxxx\"))".into(),
        "var big: i = 9223372036854775807\nprint(big + 1)".into(),
        "print(-9223372036854775807 - 1 - 1)".into(),
        "fn r(n: i) -> i { return r(n + 1) }\nprint(r(0))".into(),
        "signal e(n)\ne.listen(lambda(n: i) -> void { e(n + 1) })\ne(0)".into(),
        "class A { b: B }\nclass B { a: A }\nvar a: A = A()".into(),
        "var t: T18446744073709551615[i] = {}\nt.edit(push, 1)\nprint(t.view(keys).iter(len))".into(),
        "var n: i = 0\nwhile true { n += 1 }".into(),
        "for k in range(-9223372036854775807, 9223372036854775807) { pass }".into(),
        "print(9223372036854775807 % -1, -9223372036854775807 / -1)".into(),
        "from math use mod\nprint(mod(-9223372036854775807 - 1, -1))".into(),
        "print(int(1e300), int(\"99999999999999999999\"))".into(),
        "from math use floor\nprint(floor(1e300))".into(),
    ];
    // A million-long closure chain, then drop it (must not overflow on drop).
    v.push("var f: fn() -> i = lambda() -> i { return 0 }\nvar k: i = 0\nwhile k < 15000 {\n    const g: fn() -> i = f\n    f = lambda() -> i { return g() + 1 }\n    k += 1\n}\nprint(f())".into());
    v
}

fn run_case(label: &str, src: &str) -> Option<String> {
    if std::env::var("SKOL_FUZZ_VERBOSE").is_ok() { eprintln!("{label}"); }
    let r = panic::catch_unwind(AssertUnwindSafe(|| exercise(src)));
    match r {
        Ok(()) => None,
        Err(e) => {
            let msg = e.downcast_ref::<&str>().map(|s| s.to_string())
                .or_else(|| e.downcast_ref::<String>().cloned()).unwrap_or_default();
            let shown: String = src.chars().take(400).collect();
            Some(format!("{label}: panic: {msg}\n--- input ---\n{shown}\n"))
        }
    }
}

#[test]
fn fuzz_never_panics() {
    // A modest stack, like an embedding host thread, to catch deep recursion.
    let handle = std::thread::Builder::new().stack_size(8 * 1024 * 1024).spawn(|| {
        panic::set_hook(Box::new(|_| {}));
        let cases: usize = std::env::var("SKOL_FUZZ_CASES").ok().and_then(|s| s.parse().ok()).unwrap_or(1500);
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let mut failures = Vec::new();

        for (k, s) in stress_inputs().iter().enumerate() {
            if let Some(f) = run_case(&format!("stress #{k}"), s) { failures.push(f); }
        }
        let corpus = corpus();
        assert!(!corpus.is_empty(), "no corpus programs found");
        for k in 0..cases {
            let src = if k % 4 == 0 {
                (0..rng.below(60)).map(|_| rng.pick(TOKENS)).collect::<Vec<_>>().join(" ")
            } else {
                let base = &corpus[rng.below(corpus.len())];
                mutate(&mut rng, base)
            };
            if let Some(f) = run_case(&format!("case #{k}"), &src) { failures.push(f); }
            if failures.len() >= 5 { break; }
        }
        let _ = panic::take_hook();
        failures
    }).unwrap();
    let failures = handle.join().expect("fuzz thread crashed");
    assert!(failures.is_empty(), "{} failure(s):\n{}", failures.len(), failures.join("\n"));
}

fn run_on_small_stack(stack: usize, src: &'static str) -> Result<(), String> {
    std::thread::Builder::new().stack_size(stack).spawn(move || {
        let mut checker = Checker::new();
        checker.check_entry("t.skl", src, &mut NoFiles).map_err(|d| d.to_string())?;
        let bc = compile(&checker.into_program())?;
        let mut vm = Vm::with_output(bc, Box::new(|_| {}));
        vm.run_program().map(|_| ()).map_err(|e| e.to_string())
    }).unwrap().join().map_err(|_| "thread panicked".to_string())?
}

#[test]
fn dropping_long_value_chains_uses_constant_stack() {
    // 200K nested closures, instances and cells, freed on a 256 KB stack.
    let src = "class Box1 { inner: fn() -> i = lambda() -> i { return 0 } }
var fa: fn() -> i = lambda() -> i { return 0 }
var k: i = 0
while k < 200000 {
    const g: fn() -> i = fa
    var b: Box1 = Box1()
    b.inner = g
    fa = lambda() -> i { return b.inner() + 1 }
    k += 1
}
fa = lambda() -> i { return 1 }
print(\"done\")";
    run_on_small_stack(256 * 1024, src).unwrap();
}

#[test]
fn nested_signals_hit_the_limit_not_the_stack() {
    // Signal handlers re-entering the VM up to the nesting limit must stop
    // with an error, not a stack overflow: 4 MB covers debug builds (release
    // builds need well under 256 KB).
    let src = "signal e(n)
e.listen(lambda(n: i) -> void { e(n + 1) })
e(0)";
    let err = run_on_small_stack(4 * 1024 * 1024, src).unwrap_err();
    assert!(err.contains("nested too deeply"), "{err}");
}

