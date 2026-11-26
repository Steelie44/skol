# Embedding Skol in a game

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
  the `io` module), no environment variables, `exit` or `sleep`
  (`allow_os(true)`; `exit(n)` then returns `Error::Exit(n)`), `input()` reads
  only what `on_input` supplies, `print` goes wherever `on_print` sends it,
  and imports are found only next to the script unless you call
  `add_library_path(dir)`. `set_args` sets what `os.arg` returns.
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

See [LIBRARY.md](../LIBRARY.md#embedding-what-scripts-may-use) for which library functions are sandboxed.
