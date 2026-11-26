# Skol

A statically typed scripting language for games, with components, signals
and fixed-capacity tables. Implemented in safe Rust (`#![forbid(unsafe_code)]`
in every crate) as a type checker, a register-based bytecode VM, and an
embedding API for running scripts inside a game.

```
source ──► skol-syntax ──► skol-sema ──► skol-ir ──► skol-vm
           lexer, parser,       names & types,     typed IR          bytecode.rs → vm.rs
           AST with spans       components,        (shared by all
                                closures, tables    backends)
                                                         │
                                       skol-stdlib ◄┘  values, tables, signals, built-ins
```

| Crate | What it does |
|---|---|
| `crates/skol-syntax` | Lexer, recursive-descent parser, AST. Every statement and declaration carries a source span. |
| `crates/skol-sema` | Resolves names and imports, checks every type, flattens classes and components, works out closure captures, validates table operations. Produces the typed IR. All language rules live here. |
| `crates/skol-ir` | The typed IR: resolved locals, globals and upvalues, type-specialized operators, direct calls. The VM consumes it today; a C++/Rust ahead-of-time backend can consume it later without re-implementing the language rules. |
| `crates/skol-stdlib` | Runtime value model, unified list/key-value table storage, signal listener lists, built-in functions. Backend-agnostic. |
| `crates/skol-vm` | `bytecode.rs` lowers IR to register-based instructions (`AddI dst, a, b`), a constant pool and a type pool; `vm.rs` runs the dispatch loop over a shared register file, with zero-copy calls. |
| `crates/skol` | The embedding API: `Engine`, typed host functions, sandbox and limits. |
| `crates/skol-cli` | The `skol` command, plus end-to-end tests and the fuzzer. |
| `editors/vscode` | VS Code extension: highlighting, live type errors, Run button. |

## Building and running

Needs Rust (https://rustup.rs). Use `--release` for real speed.

```
cargo build --release
target\release\skol examples\hello.skl
target\release\skol examples\game\main.skl
```

| Command | |
|---|---|
| `skol <file.skl>` / `skol run <file.skl>` | type-check, compile and run. `main()` runs after top-level code; if it returns an `i`, that's the exit code. |
| `skol check <file.skl>` | type-check only; errors print as `path:line:col: error: message` |
| `skol repl` (or just `skol`) | interactive prompt; expressions are echoed, names can be redefined |
| `skol ast` / `skol tokens` / `skol ir` / `skol disasm` | inspect each stage of the pipeline |
| `--seed <n>` | fixed random seed; `-` reads source from stdin; `--name` sets its path |

Tests: `cargo test` (add `--release` to run the fuzzer faster;
`SKOL_FUZZ_CASES=50000 cargo test --release --test fuzz` for a long session).
Golden programs live in `crates/skol-cli/tests/programs`
(each `.skl` is compared with its `.out`; run `SKOL_BLESS=1 cargo test` to
accept intentional changes). Expected compile and runtime errors are in
`crates/skol-cli/tests/errors.txt`.

## Embedding Skol in a game

Add the `skol` crate (path dependency on `crates/skol`) to your game:

```rust
use skol::{Engine, Limits, Vec2};

let mut engine = Engine::new();                       // sandboxed defaults
engine.set_limits(Limits { max_steps: Some(1_000_000), ..Limits::default() });
engine.register("game", "spawn", |kind: String, at: Vec2| -> i64 { /* ... */ 1 })?;
engine.register("game", "key_down", |key: String| -> bool { /* ... */ false })?;
engine.load_file("scripts/main.skl")?;                // checked against your API

loop {
    engine.call::<(), _>("update", (1.0 / 60.0,))?;   // typed call into the script
    let score: i64 = engine.get("score")?;
}
```

The script imports what you registered: `from game use spawn, key_down`.
Calling it with wrong argument types is a compile error at load time.

- **Types across the boundary:** `i64`/`i32` ↔ `i`, `f64`/`f32` ↔ `f`,
  `bool`, `String` ↔ `s`, `()` ↔ `void`, `skol::Vec2/Vec3/Vec4` ↔
  `vec2/vec3/vec4`. Host functions take up to six arguments, and may return
  `Result<T, String>`; an `Err` becomes a runtime error in the script with a
  stack trace.
- **Sandbox defaults:** no file access (`allow_file_access(true)` to enable
  the `io` module), `input()` reads only what `on_input` supplies, `print`
  goes wherever `on_print` sends it, and scripts can call only the functions
  you registered.
- **Limits:** `max_steps` bounds loop iterations + calls per call into the
  script, `max_depth` bounds recursion, `max_nesting` bounds signal handlers
  that emit signals. Hitting any limit is an ordinary `Err`; the engine stays
  usable.
- **No crashes:** script errors, limit hits, API misuse and even a panic
  inside your own host function come back as `skol::Error`, never a panic.
- **Stack:** release builds of a host need well under 256 KB of stack for
  scripts; debug builds can need up to about 4 MB when signal handlers nest
  deeply.

A runnable example: `cargo run -p skol --example game_loop`.

## VS Code

1. Build once: `cargo build` (or `--release`).
2. Install the extension package:
   ```
   code --install-extension editors\vscode\skol.vsix --force
   ```
   (or Extensions panel → **⋯ → Install from VSIX…**), then **Developer: Reload Window**.
   After changing anything in `editors\vscode`, rebuild the package with
   `python editors\vscode\build_vsix.py` and install it again.
3. Open the `skol` folder. In any `.skl` file:
   - full type errors appear as you type (Problems panel too)
   - ▶ in the editor title bar, or Ctrl+F5, runs the file in a terminal
   - Command Palette → **Skol: Show AST / Tokens / Typed IR / Bytecode**

The extension uses `target\debug\skol.exe` or `target\release\skol.exe` from the
open folder, or `skol` on your PATH; set `skol.executablePath` to override.

---

## Language reference (v0.1)

### Types

| Type | Meaning | Zero value |
|---|---|---|
| `i` | 64-bit integer (overflow is a runtime error) | `0` |
| `f` | 64-bit float | `0.0` |
| `s` | string | `""` |
| `bool` | `true` / `false` | `false` |
| `vec2` `vec3` `vec4` | float vectors, value semantics; components `.x .y .z .w` | all zeros |
| `T<cap>[elem]` | list table with fixed capacity, e.g. `T32[s]` | empty |
| `T<cap>[key, val]` | key/value table; keys are `i`, `s` or `bool` | empty |
| `ClassName` | instance of a class (reference semantics) | default instance |
| `CompName` | any instance whose class includes that component | — |
| `fn(i, s) -> bool` | function value; `fn(s)` returns void | — |

`i`, `f` and `s` are keywords, so they can't be used as variable names.

An `i` widens to `f` automatically. Nothing else converts implicitly; use
`int(x)`, `float(x)` and `str(x)`.

### Bindings, functions, control flow

```
const MAX: i = 10                 # immutable
var name: s = "ada"               # mutable
const add := lambda(a: i, b: i) -> i { return a + b }

fn area(w: f, h: f) -> f { return w * h }    # no "-> type" means void

if x > 0 { ... } else if x < 0 { ... } else { pass }
while n < 10 { n += 1 }
for item in table { ... }         # list: elements; key/value: keys
for k in range(5) { ... }         # 0..4;  range(2, 5) is 2..4
for ch in "text" { ... }          # characters
break                             # leave the innermost loop
continue                          # next iteration of the innermost loop
```

Operators: `+ - * / %` (`/` on two `i` is integer division; `%` is Euclidean, so
`-7 % 3` is `2`, same as `math.mod`), `== != < <= > >=`,
`and or not`, `+=` `-=`. `+` joins strings. Vectors support `+ -`,
component-wise `*`, and `* /` by a number. Conditions must be `bool`.

Top-level code runs in order, then `fn main()` if it exists. Functions,
classes and components can be used before they're declared.

### Tables

Tables have a fixed capacity checked at runtime. Table literals take their
type from the binding: `var t: T8[i] = {1, 2}`, `var m: T4[s, i] = {"a": 1}`.
A vector literal is a table literal of the right size: `var p: vec2 = {1.0, 2.0}`.

Indexing is shorthand for `.pos(get/set, ...)`, and works for reads, writes,
compound assignment and nested places:

```
nums[1] = 25          # nums.pos(set, 1, 25)
stock["pear"] += 1
grid[1][0] = 30       # tables of tables
points[0].y = 9.0     # vector inside a table
word[0]               # strings: the character at index 0 (read-only)
```

All table operations go through four dispatchers, with the operation name
first:

| Call | List `T[e]` | Key/value `T[k, v]` | Returns |
|---|---|---|---|
| `t.iter(len)` | element count | entry count | `i` |
| `t.iter(sum)` | sum of `i`/`f` elements | sum of values | `i` or `f` |
| `t.iter(even)` | new table of even elements (`T[i]` only) | — | `T[i]` |
| `t.iter(cut, k)` | reset slot `k` to the zero value (length unchanged) | reset value at key `k` | void |
| `t.pos(get, k)` | element at index `k` | value at key `k` | element |
| `t.pos(set, k, v)` | replace index `k` | insert or update | void |
| `t.pos(has, k)` | index in range? | key present? | `bool` |
| `t.edit(push, v)` | append (error when full) | — (use `pos(set)`) | void |
| `t.edit(pop)` | remove and return last | — | element |
| `t.edit(clear)` | remove all | remove all | void |
| `t.view(is_empty)` | | | `bool` |
| `t.view(keys)` | indices | keys | list table |
| `t.view(vals)` | copy of elements | values | list table |

The type checker enforces each operation's argument count and types, and
rejects operations that don't fit the table (e.g. `even` on `T[f]`, `push` on
a key/value table).

### Classes and components

```
comp Health {
    max_health: i = 100
    current_health: i = 100
    fn take_damage(amount: i) { self.current_health -= amount }
}

class Enemy:(Health, Movement) {
    name: s = "grunt"
    speed: f = 2.5                    # override a component field's default
    fn init(name: s) { self.name = name }     # optional constructor
}

var e: Enemy = Enemy("orc")           # args go to init
e.take_damage(10)
```

- Fields and methods are accessed through `self` inside methods.
- A class gets all fields and methods of its components. It can override a
  component field's default (same type) or a component method (same
  signature). Two components defining the same field or method is an error.
- A component name used as a type accepts any instance that includes it:
  `fn hurt(target: Health, n: i) { target.take_damage(n) }`.
- Fields without a default get their type's zero value. Function-typed and
  component-typed fields need a default.

### Signals

```
signal died(entity, cause)

died.listen(lambda(who: Health, cause: s) -> void { print(cause) })
died.listen(enemy.on_death)           # bound methods work too
died.emit(e, "fall")                  # or: died(e, "fall")
```

Listeners run in the order they were added. Signal parameters are untyped;
each handler's declared parameter types are checked when the signal fires.

### Function values

Named functions, lambdas and bound methods (`enemy.on_hit`) are values of a
function type, and can be passed, returned and stored anywhere:

```
fn apply(op: fn(i) -> i, x: i) -> i { return op(x) }
fn make_adder(n: i) -> fn(i) -> i { return lambda(x: i) -> i { return x + n } }

var ops: T4[fn(i) -> i] = {make_adder(1), make_adder(10)}
class Button { on_click: fn(s) = lambda(who: s) -> void { print(who) } }
```

`:=` stays reserved for binding a lambda directly; for any other function
value, write the type: `const add1: fn(i) -> i = make_adder(1)`.

### Closures

Lambdas capture enclosing variables by reference, so changes are visible
both ways. Each loop iteration gets a fresh copy of the loop variable and of
variables declared inside the body. A lambda bound with `:=` can call itself
recursively.

### Modules

`from Shapes use Circle, hypot` loads `Shapes.skl` from the importing file's
folder, runs its top-level code once, and brings the named items into scope.
Circular imports are an error.

Built-in `math` module: `from math use sqrt, pi`. It provides `sqrt sin cos
tan atan2 pow abs floor ceil round min max clamp lerp mod random random_int
time` and the constants `pi tau e`.

Built-in `io` module: `from io use read_file, write_file, append_file,
file_exists`. Failures (missing file, no permission) are runtime errors.

### Built-ins

`print(...)`, `input()`, `input(prompt)`, `str(x)`, `int(x)`, `float(x)`,
`type_of(x)`, `assert(cond)`, `assert(cond, msg)`, `error(msg)`, `range(n)`,
`range(a, b)`.

`input` reads one line from the keyboard (or piped stdin) without the line
ending, and returns `""` at end of input.

String methods: `len upper lower trim contains starts_with ends_with replace
find substr(start, end) repeat split`. `split` takes its capacity from where
the result goes: `var parts: T16[s] = line.split(",")` (more pieces than that
is a runtime error).
Vector methods: `length normalized dot distance cross` (`cross` is vec3 only).

### Robustness

Nothing a Skol program does, and no input to the toolchain, crashes the
process:

- Deeply nested code (more than 128 levels) is a compile error, not a stack
  overflow.
- Freeing huge object graphs (millions of linked closures or instances) uses
  constant stack.
- Integer overflow, division by zero, out-of-range indices, full tables,
  oversized strings (64 MB cap) and runaway recursion are runtime errors.
- A fuzzer (`crates/skol-cli/tests/fuzz.rs`) feeds random and mutated
  programs plus targeted stress cases through the whole pipeline on a small
  stack and fails on any panic.

Not covered: a program that deliberately allocates unbounded memory (for
example filling many huge tables) can still exhaust RAM.

### Errors

Compile errors stop the build and print `file:line:col: error: ...`, pointing at
the exact expression at fault. Runtime
errors (index out of range, full table, division by zero, integer overflow,
stack overflow, failed assert) print the location and a stack trace, and
`skol` exits with code 70.

---

## Current limits and natural next steps

- **No string interpolation**; build strings with `+` and `str()`.
- **No `*=`, `/=` or `%=`**; only `+=` and `-=`.
- **Statements are newline-insensitive**: `return` followed by an expression
  on the next line returns that expression.
- **No deleting files or listing directories** in `io` yet.
- **Embedding API types:** host functions and `Engine::call` handle scalars,
  strings and vectors; passing tables or class instances across the boundary
  isn't supported yet.
- **Performance:** register VM in safe Rust. On the game benchmark
  (`examples/bench_game.skl`), 1M entity updates take about 0.21 s; fib(30)
  about 0.075 s. Next steps: inline caches for component-typed field access,
  fewer register moves around calls, and an ahead-of-time backend.
- **AOT backend:** a `skol-codegen` crate can walk `skol_ir::Program`
  directly. Every operator is already type-specialized and every call already
  resolved.
#   s k o l  
 