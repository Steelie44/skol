<p align="center">
  <img src="editors/vscode/icons/skol.png" width="96" alt="Skol logo">
</p>

<h1 align="center">Skol</h1>

<p align="center">
  A statically typed scripting language for games: components, signals and fixed-capacity tables,<br>
  checked before it runs and embeddable in any Rust game.
</p>

<p align="center">
  <a href="#quick-start">Quick start</a> ·
  <a href="docs/LANGUAGE.md">Language</a> ·
  <a href="LIBRARY.md">Library</a> ·
  <a href="docs/EMBEDDING.md">Embedding</a> ·
  <a href="#vs-code">VS Code</a>
</p>

---

```skol
from timers use Cooldown
from random use randi_range

signal died(who)

comp Health {
    hp: i = 100
    fn take_damage(amount: i) -> void {
        self.hp -= amount
        if self.hp <= 0 { died.emit(self) }
    }
}

class Enemy:(Health, Cooldown) {
    name: s = "grunt"
    cooldown: f = 0.5
    fn init(name: s, hp: i) {
        self.name = name
        self.hp = hp
    }
}

fn main() {
    var enemies: T16[Enemy] = {Enemy("orc", 30), Enemy("bat", 5), Enemy("troll", 80)}
    died.listen(lambda(e: Enemy) -> void { print(fmt("{} died", e.name)) })

    enemies.edit(sort_by, lambda(e: Enemy) -> i { return e.hp })
    for e in enemies {
        if e.trigger() { e.take_damage(randi_range(10, 40)) }
    }
    var alive: T16[Enemy] = enemies.view(filter, lambda(e: Enemy) -> bool { return e.hp > 0 })
    print(fmt("{} of {} still standing", alive.iter(len), enemies.iter(len)))
}
```

## Features

- **Caught before it runs.** Every type is checked when the script loads, and
  errors point at the exact line and column, both in the terminal and as you
  type in VS Code.
- **Made for games.** Components (`class Enemy:(Health, Cooldown)`), signals
  (`died.emit(e)`), built-in `vec2`/`vec3`/`vec4`, and tables with a fixed
  capacity so memory use is predictable.
- **Batteries included.** Built-in `math`, `random`, `time`, `os` and `io`
  modules, plus `timers`, `fsm`, `ease`, `log` and `testing` written in Skol
  itself. See [LIBRARY.md](LIBRARY.md).
- **Embeddable and sandboxed.** Load scripts into a Rust game, expose typed
  host functions, and limit steps, recursion and file access.
- **Doesn't crash.** Written in 100% safe Rust (`#![forbid(unsafe_code)]`).
  Script errors, limit hits and API misuse come back as errors, never as a
  crash, and a fuzzer checks this.
- **Fast.** A register-based bytecode VM runs 1M method-heavy entity updates
  in about 0.14 s. On the benchmarks in [`bench/`](bench) it is faster
  than Python, Ruby and V8 with its JIT turned off.

## Quick start

Install [Rust](https://rustup.rs), then:

```sh
git clone https://github.com/Steelie44/skol.git
cd skol
cargo build --release
```

Run a program (on Windows use `target\release\skol`):

```sh
target/release/skol examples/hello.skl
target/release/skol examples/game/main.skl
```

| Command | What it does |
|---|---|
| `skol run <file.skl> [args]` | Type-check and run it. Extra arguments go to the script (`os.arg`). `skol <file.skl>` works too. |
| `skol check <file.skl>` | Type-check only; errors print as `path:line:col: error: message` |
| `skol repl` | Interactive prompt (also just `skol`) |
| `skol ast` · `tokens` · `ir` · `disasm` | Show each stage of the compiler |
| `--seed <n>` · `--lib <dir>` | Fix the random seed · search another folder for imports |

## A taste of the language

```skol
from math use clamp

var hp: i = 100                          # i, f, s, bool, vec2/3/4
const SPEED: f = 4.5
var inventory: T32[s] = {"sword", "potion"}
var prices: T8[s, i] = {"sword": 50, "potion": 5}

fn heal(amount: i) -> i {                # no "-> type" means void
    hp = clamp(hp + amount, 0, 100)
    return hp
}

inventory.edit(push, "shield")           # tables: .iter .pos .edit .view
prices.edit(remove, "potion")
var short: T32[s] = inventory.view(filter, lambda(item: s) -> bool { return item.len() < 6 })
print(fmt("{} items, {} with short names", inventory.iter(len), short.iter(len)))
```

The [language reference](docs/LANGUAGE.md) covers types, classes and
components, signals, closures and modules.

## Standard library

```skol
from fsm use StateMachine
from ease use Tween, ease_out_back
from log use Logger
```

`skol` finds the [`lib/`](lib) folder on its own, from any project folder.
[LIBRARY.md](LIBRARY.md) documents every built-in function, string method
and table operation, plus each `lib` module.

## Embedding in a Rust game

```rust
use skol::{Engine, Vec2};

let mut engine = Engine::new();                        // sandboxed by default
engine.register("game", "spawn", |kind: String, at: Vec2| -> i64 { 1 })?;
engine.load_file("scripts/main.skl")?;                 // type-checked against your API
engine.call::<(), _>("update", (1.0 / 60.0,))?;
```

See [docs/EMBEDDING.md](docs/EMBEDDING.md), and run
`cargo run -p skol --example game_loop` for a working example.

## VS Code

The extension in [`editors/vscode`](editors/vscode) gives you syntax
highlighting, live type errors, a ▶ Run button (Ctrl+F5) and Skol file icons.

```sh
cargo build --release
code --install-extension editors/vscode/skol.vsix --force
```

Then run **Developer: Reload Window**. The extension uses
`target/release/skol` from the open folder, or `skol` on your PATH (setting:
`skol.executablePath`). If you change the extension, rebuild it with
`python editors/vscode/build_vsix.py`.

## Project layout

```
source ─► skol-syntax ─► skol-sema ─► skol-ir ─► skol-vm
          lexer, parser   types, names  typed IR   bytecode + VM
                                                     │
                                       skol-stdlib ◄─┘  values, tables, built-ins
```

| Path | Contents |
|---|---|
| `crates/skol-syntax` | Lexer, parser, AST with source spans |
| `crates/skol-sema` | Name resolution and type checking; all language rules live here |
| `crates/skol-ir` | Typed intermediate representation, shared by every backend |
| `crates/skol-stdlib` | Runtime values, tables, signals, built-in functions |
| `crates/skol-vm` | Bytecode compiler and register VM |
| `crates/skol` | Embedding API (`Engine`) |
| `crates/skol-cli` | The `skol` command, end-to-end tests and the fuzzer |
| `lib` | Standard library written in Skol, with tests in `lib/tests` |
| `editors/vscode` | VS Code extension |
| `examples`, `bench` | Example programs and cross-language benchmarks |

## Testing

```sh
cargo test --release
```

This runs the unit, golden-output and error-message tests, the fuzzer, and
the standard library's own Skol tests. Golden programs live in
`crates/skol-cli/tests/programs`; after an intentional output change, run
`SKOL_BLESS=1 cargo test` to accept it. For a longer fuzzing session:
`SKOL_FUZZ_CASES=50000 cargo test --release --test fuzz`.

## Status and roadmap

Skol is young (v0.1) and the language may still change. Known gaps:

- No string interpolation yet; use `fmt("{} hp", hp)`.
- Only `+=` and `-=`; no `*=`, `/=` or `%=`.
- No exponent float literals (`1e6`); write `1000000.0`.
- Dates are UTC; there's no time zone support yet.
- No generics: library classes hold one concrete type. The built-in table
  operations work on any element type.
- The embedding API passes scalars, strings and vectors, but not tables or
  class instances.

Next up: a Godot integration, faster field access and calls, and an
ahead-of-time compiler backend that reads `skol_ir`.
