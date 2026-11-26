//! End-to-end tests: run `skol` on every program in tests/programs and compare
//! stdout with the matching .out file; run every case in tests/errors.txt and
//! check it fails with the expected message.
//!
//! To accept new output after an intentional change, run with
//! SKOL_BLESS=1 cargo test.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn skol() -> Command { Command::new(env!("CARGO_BIN_EXE_skol")) }

fn tests_dir() -> PathBuf { Path::new(env!("CARGO_MANIFEST_DIR")).join("tests") }

#[test]
fn golden_programs() {
    let dir = tests_dir().join("programs");
    let bless = std::env::var("SKOL_BLESS").is_ok();
    let mut failures = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
    entries.sort();
    for path in entries {
        if path.extension().and_then(|e| e.to_str()) != Some("skl") { continue; }
        let out_path = path.with_extension("out");
        // Files without a .out are modules imported by other tests.
        if !out_path.exists() && !bless { continue; }
        let output = skol().arg("run").arg(&path).arg("--seed").arg("42").current_dir(&dir).output().unwrap();
        let mut actual = String::from_utf8_lossy(&output.stdout).into_owned();
        actual.push_str(&String::from_utf8_lossy(&output.stderr));
        if bless {
            if out_path.exists() || path.file_stem().unwrap() != "Shapes" {
                std::fs::write(&out_path, &actual).unwrap();
            }
            continue;
        }
        let expected = std::fs::read_to_string(&out_path).unwrap().replace("\r\n", "\n");
        if actual.replace("\r\n", "\n") != expected {
            failures.push(format!("{}:\n--- expected\n{expected}\n--- actual\n{actual}", path.display()));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn error_cases() {
    let text = std::fs::read_to_string(tests_dir().join("errors.txt")).unwrap().replace("\r\n", "\n");
    let mut cases = Vec::new();
    let mut current: Option<(String, String)> = None;
    for line in text.lines() {
        if let Some(expect) = line.strip_prefix("=== ") {
            if let Some(c) = current.take() { cases.push(c); }
            current = Some((expect.to_string(), String::new()));
        } else if let Some((_, src)) = current.as_mut() {
            src.push_str(line);
            src.push('\n');
        }
    }
    if let Some(c) = current.take() { cases.push(c); }
    assert!(cases.len() > 20, "expected many error cases, found {}", cases.len());

    let mut failures = Vec::new();
    for (expect, src) in &cases {
        let mut child = skol().args(["run", "-", "--name", "case.skl"])
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
            .current_dir(tests_dir())
            .spawn().unwrap();
        child.stdin.take().unwrap().write_all(src.as_bytes()).unwrap();
        let output = child.wait_with_output().unwrap();
        let mut all = String::from_utf8_lossy(&output.stdout).into_owned();
        all.push_str(&String::from_utf8_lossy(&output.stderr));
        if output.status.success() || !all.contains(expect.as_str()) {
            failures.push(format!("expected failure containing:\n  {expect}\nsource:\n{src}got (exit {:?}):\n{all}", output.status.code()));
        }
    }
    assert!(failures.is_empty(), "{} of {} error cases failed:\n\n{}", failures.len(), cases.len(), failures.join("\n"));
}

#[test]
fn check_reports_type_errors_in_editor_format() {
    let mut child = skol().args(["check", "-", "--name", "C:\\proj\\main.skl"])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(b"var x: i = 1\nvar y: s = x\n").unwrap();
    let out = child.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.starts_with("C:\\proj\\main.skl:2:12: error: initializer of 'y': expected s, found i"), "{text}");
}

#[test]
fn input_reads_lines_from_stdin() {
    let path = std::env::temp_dir().join(format!("skol_input_test_{}.skl", std::process::id()));
    std::fs::write(&path, "var name: s = input(\"name? \")\nvar n: i = int(input())\nprint(\"hi\", name, n * 2)\nprint(\"eof:[\" + input() + \"]\")\n").unwrap();
    let mut child = skol().arg("run").arg(&path)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(b"ada\r\n21\n").unwrap();
    let out = child.wait_with_output().unwrap();
    let _ = std::fs::remove_file(&path);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("name? hi ada 42"), "{text}");
    assert!(text.contains("eof:[]"), "{text}");
}

#[test]
fn repl_keeps_state_and_echoes() {
    let mut child = skol().arg("repl").stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(
        b"var n: i = 40\nfn add(a: i) -> i {\n  return a + n\n}\nadd(2)\nvar bad: s = 1\nn = 1\nadd(2)\n:quit\n",
    ).unwrap();
    let out = child.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("42"), "{text}");
    assert!(text.contains("expected s, found i"), "{text}");
    assert!(text.contains("skol> 3"), "{text}");
}

#[test]
fn standard_library_tests_pass() {
    let lib = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib");
    let mut entries: Vec<_> = std::fs::read_dir(lib.join("tests")).unwrap().map(|e| e.unwrap().path()).collect();
    entries.sort();
    let mut ran = 0;
    for path in entries {
        if path.extension().and_then(|e| e.to_str()) != Some("skl") { continue; }
        let output = skol().arg("run").arg("--lib").arg(&lib).arg(&path).output().unwrap();
        let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        assert!(output.status.success(), "{} failed:\n{text}", path.display());
        assert!(!text.contains("FAIL"), "{}:\n{text}", path.display());
        ran += 1;
    }
    assert!(ran >= 4, "expected library tests, found {ran}");
}

#[test]
fn script_arguments_and_exit_codes() {
    let path = std::env::temp_dir().join(format!("skol_args_test_{}.skl", std::process::id()));
    std::fs::write(&path, "from os use arg_count, arg, exit\nprint(arg_count(), arg(0), arg(1))\nexit(3)\nprint(\"unreachable\")\n").unwrap();
    let out = skol().arg("run").arg(&path).arg("one").arg("--").arg("--seed").output().unwrap();
    let _ = std::fs::remove_file(&path);
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(text, "2 one --seed\n");
    assert_eq!(out.status.code(), Some(3));
}
