"""Compare Skol against other interpreted languages.

    python bench/run.py            # from the repository root
    python bench/run.py --runs 5   # more runs per benchmark (best time is kept)
    python bench/run.py --jit      # also show JIT-compiled reference rows
    python bench/run.py --only game fib   # just these benchmarks

Every benchmark is written idiomatically in each language and prints
`RESULT <checksum> <seconds>`, timing only the work itself (not start-up).
The runner checks that every language computed the same checksum, keeps the
best of N runs, and prints a table plus `bench/results.md`.

Languages are skipped when not installed. Interpreters are run with their
JIT compilers off so the comparison is interpreter against interpreter:
  - Lua 5.4 (`lua`), LuaJIT's interpreter (`luajit -joff`)
  - PHP 8 with OPcache/JIT disabled
  - JavaScript on V8 with `node --jitless` (V8's Ignition interpreter)
  - Ruby 3 (YJIT is off unless --jit)
  - Python 3
  - GDScript via Godot 4 (`godot --headless --script`); set GODOT=path\\to\\godot.exe
    if it isn't on PATH
"""

import argparse
import math
import os
import platform
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)

BENCHMARKS = [
    ("fib", "fib(30), recursion", 832040),
    ("loop", "10M integer loop", 99999980000000),
    ("entities", "1M entity updates", 33262),
    ("tables", "2M indexed writes + map lookups", 2108616),
    ("strings", "200K string builds + counting", 779000),
    ("game", "1000 game frames: AI, physics, collisions, sorting, events, strings, BFS", 842559),
]


def which(*names):
    for n in names:
        p = shutil.which(n)
        if p:
            return p
    return None


def find_languages(include_jit):
    exe = ".exe" if platform.system() == "Windows" else ""
    langs = []

    skol = os.path.join(ROOT, "target", "release", "skol" + exe)
    if os.path.exists(skol):
        langs.append(("Skol", "skl", [skol]))
    else:
        print(f"note: {skol} not found; run `cargo build --release` first", file=sys.stderr)

    lua = which("lua5.4", "lua54", "lua")
    if lua:
        langs.append(("Lua 5.4", "lua", [lua]))
    luajit = which("luajit")
    if luajit:
        langs.append(("LuaJIT (interp)", "lua", [luajit, "-joff"]))
    php = which("php")
    if php:
        langs.append(("PHP 8", "php", [php, "-d", "opcache.enable_cli=0", "-d", "opcache.jit=disable"]))
    node = which("node")
    if node:
        langs.append(("JS (V8 interp)", "js", [node, "--jitless"]))
    ruby = which("ruby")
    if ruby:
        langs.append(("Ruby 3", "rb", [ruby]))
    langs.append(("Python 3", "py", [sys.executable]))
    godot = os.environ.get("GODOT") or which("godot", "godot4", "Godot", "godot.windows.opt.tools.64")
    if godot:
        langs.append(("GDScript", "gd", [godot, "--headless", "--script"]))

    if include_jit:
        if node:
            langs.append(("JS (V8 JIT)", "js", [node]))
        if luajit:
            langs.append(("LuaJIT (JIT)", "lua", [luajit]))
        if ruby:
            langs.append(("Ruby 3 (YJIT)", "rb", [ruby, "--yjit"]))
        if php:
            langs.append(("PHP 8 (JIT)", "php", [php, "-d", "opcache.enable_cli=1", "-d", "opcache.jit=tracing",
                                                 "-d", "opcache.jit_buffer_size=64M"]))
    return langs


def run_once(cmd, path):
    try:
        p = subprocess.run(cmd + [path], capture_output=True, text=True, timeout=300, cwd=HERE)
    except subprocess.TimeoutExpired:
        return None, "timed out"
    except OSError as e:
        return None, str(e)
    for line in (p.stdout + "\n" + p.stderr).splitlines():
        parts = line.strip().split()
        if len(parts) == 3 and parts[0] == "RESULT":
            try:
                return (int(float(parts[1])), float(parts[2])), None
            except ValueError:
                pass
    tail = (p.stderr or p.stdout).strip().splitlines()[-3:]
    return None, " | ".join(tail) or f"exit code {p.returncode}"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--jit", action="store_true", help="also run JIT-compiled reference configurations")
    ap.add_argument("--only", nargs="+", metavar="NAME", help="run only these benchmarks (e.g. game fib)")
    args = ap.parse_args()
    benchmarks = [b for b in BENCHMARKS if not args.only or b[0] in args.only]
    if not benchmarks:
        sys.exit("no benchmark matches --only; choose from: " + ", ".join(b[0] for b in BENCHMARKS))

    langs = find_languages(args.jit)
    print("Languages:", ", ".join(name for name, _, _ in langs))
    results = {}  # (bench, lang) -> seconds or error string
    for bench, desc, expected in benchmarks:
        for name, ext, cmd in langs:
            path = os.path.join(HERE, f"{bench}.{ext}")
            best, err = None, None
            for _ in range(args.runs):
                r, e = run_once(cmd, path)
                if r is None:
                    err = e
                    break
                checksum, secs = r
                if checksum != expected:
                    err = f"wrong answer {checksum} (expected {expected})"
                    break
                best = secs if best is None else min(best, secs)
            results[(bench, name)] = best if err is None else f"ERROR: {err}"
            shown = f"{best:.3f}s" if err is None else results[(bench, name)]
            print(f"  {bench:9} {name:16} {shown}")

    names = [n for n, _, _ in langs]
    lines = []
    header = "| Benchmark | " + " | ".join(names) + " |"
    lines.append(header)
    lines.append("|---|" + "---|" * len(names))
    for bench, desc, _ in benchmarks:
        cells = []
        for n in names:
            v = results[(bench, n)]
            cells.append(f"{v:.3f}s" if isinstance(v, float) else "error")
        lines.append(f"| {desc} | " + " | ".join(cells) + " |")

    # Geometric mean of time relative to Skol (lower is faster).
    if "Skol" in names:
        cells = []
        for n in names:
            ratios = []
            for bench, _, _ in benchmarks:
                a, b = results[(bench, n)], results[(bench, "Skol")]
                if isinstance(a, float) and isinstance(b, float) and b > 0:
                    ratios.append(a / b)
            if ratios:
                g = math.exp(sum(math.log(r) for r in ratios) / len(ratios))
                cells.append("1.00×" if n == "Skol" else f"{g:.2f}×")
            else:
                cells.append("—")
        lines.append("| **Time vs Skol (geomean)** | " + " | ".join(cells) + " |")

    table = "\n".join(lines)
    print()
    print(table)
    print("\nLower is faster. ×1.00 = same speed as Skol, ×2.00 = takes twice as long.")
    with open(os.path.join(HERE, "results.md"), "w", encoding="utf-8") as f:
        f.write(f"# Benchmark results\n\nMachine: {platform.platform()}, {platform.processor() or platform.machine()}\n"
                f"Best of {args.runs} runs.\n\n{table}\n")
    errors = [(k, v) for k, v in results.items() if isinstance(v, str)]
    for (bench, n), e in errors:
        print(f"{n} / {bench}: {e}")


if __name__ == "__main__":
    main()
