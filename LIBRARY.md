# Skol library reference

Skol's libraries come in two layers:

- **Built in** (native Rust, fast, always available): the prelude, string
  methods, table operations, and the modules `math`, `random`, `time`, `os`
  and `io`.
- **Standard library** (written in Skol, in the repo's `lib/` folder):
  `timers`, `fsm`, `ease`, `log` and `testing`. Read them, copy them, change
  them; they're ordinary Skol modules.

Everything below is imported the same way: `from <module> use <name>, <name>`.

---

## Prelude (no import needed)

| Function | Returns | |
|---|---|---|
| `print(a, b, ...)` | void | values separated by spaces, then a newline |
| `fmt(template, ...)` | `s` | `{}` inserts the next value; `{.2}` formats a number with 2 decimals; `{{` and `}}` are literal braces |
| `str(x)` `int(x)` `float(x)` | | conversions; `int("x")` is a runtime error, so check with `.is_int()` first |
| `chr(code)` | `s` | one-character string for a Unicode code point (`chr(65)` is `"A"`) |
| `type_of(x)` | `s` | the type name |
| `input()` `input(prompt)` | `s` | one line from the keyboard; `""` at end of input |
| `assert(cond)` `assert(cond, msg)` `error(msg)` | void | stop with a runtime error |
| `range(n)` `range(a, b)` | range | for `for` loops |

```
print(fmt("{} has {.1} hp", enemy.name, enemy.hp))    # orc has 12.5 hp
```

## Strings

`len upper lower trim trim_start trim_end contains starts_with ends_with
replace(from, to) find(sub) rfind(sub) count(sub) substr(start, end)
repeat(n) reverse pad_left(width, fill) pad_right(width, fill) split(sep)
code is_int is_float is_alpha is_digit is_space`

- `find` / `rfind` return the character index of the first / last match, or `-1`.
- `pad_left(3, "0")` turns `"7"` into `"007"`; `fill` is one character.
- `is_int()` / `is_float()` tell you whether `int(x)` / `float(x)` will work.
- `code()` is the code point of the first character (`"A".code()` is 65).
- `split` and the other list-building calls take their capacity from where
  the result goes: `var parts: T16[s] = line.split(",")`.
- `s[i]` is the character at index `i`.

## Tables

All operations go through four dispatchers. **L** = list `T[e]`,
**M** = key/value `T[k, v]`.

### `.iter(...)` — read the whole table

| Call | Works on | Returns |
|---|---|---|
| `len` | L M | `i` |
| `sum` | L M of `i`/`f` | `i` or `f` |
| `min` / `max` | L M of `i`/`f`/`s` | element (error if empty) |
| `count, v` | L M | how many elements (values) equal `v` |
| `contains, v` | L M | `bool` — is `v` an element (value)? |
| `find, v` | L | index of the first `v`, or `-1` |
| `even` | `T[i]` | new list of the even elements |
| `cut, k` | L M | resets slot `k` to the zero value (keeps the slot) |

### `.pos(...)` — one slot

| Call | Works on | Returns |
|---|---|---|
| `get, k` / `set, k, v` / `has, k` | L M | as before |
| `get_or, k, default` | L M | the value, or `default` if `k` is missing |
| `first` / `last` | L | element (error if empty) |

### `.edit(...)` — change the table

| Call | Works on | |
|---|---|---|
| `push, v` / `pop` / `clear` | L (`clear`: L M) | as before |
| `insert, i, v` | L | insert before index `i` (`0..len`), shifting the rest up |
| `remove, k` | L M | list: remove index `k` and shift down; map: delete key `k`. Returns the removed value |
| `swap_remove, i` | L | remove index `i` by moving the last element into it: O(1), changes order |
| `pop_front` | L | remove and return the first element (queue) |
| `extend, other` | L M | append another list / merge another map of the same type |
| `sort` | L of `i`/`f`/`s`/`bool`; M (by key) | ascending, stable |
| `sort_by, fn(e) -> i/f/s` | L | sort by a key function: `enemies.edit(sort_by, lambda(e: Enemy) -> f { return e.hp })` |
| `reverse` | L M | |
| `shuffle` | L | random order (uses the `random` seed) |

### `.view(...)` — make something new

| Call | Works on | Returns |
|---|---|---|
| `is_empty` / `keys` / `vals` | L M | as before |
| `copy` | L M | an independent table of the same type |
| `slice, a, b` | L | elements `a..b` as a new table |
| `filter, fn(e) -> bool` | L | new table with the elements the function keeps |
| `join, sep` | L | `s` — elements joined with `sep` |
| `pick` | L | a random element |

Stack: `push` / `pop`. Queue: `push` / `pop_front`. Deque: add `insert, 0, v`.
To remove things while looping, mark them and clean up after the loop, or
loop backwards with an index and `remove`.

---

## `math`

| Name | |
|---|---|
| `sqrt sin cos tan asin acos atan atan2 pow exp log log10` | `log` is the natural log |
| `abs sign` | keep `i` as `i` and `f` as `f` |
| `floor ceil round` | return `i` |
| `min max clamp` | all-`i` arguments give an `i` |
| `wrap(x, lo, hi)` | wraps into `[lo, hi)`: `wrap(-1, 0, 4)` is 3; works on `i` and `f` (angles: `wrap(a, 0.0, 360.0)`) |
| `lerp(a, b, t)` `inverse_lerp(a, b, v)` `remap(v, in_lo, in_hi, out_lo, out_hi)` | |
| `move_toward(from, to, delta)` | step toward a target without overshooting |
| `smoothstep(edge0, edge1, x)` | smooth 0→1 curve |
| `snapped(x, step)` | round to the nearest multiple of `step` |
| `deg_to_rad rad_to_deg` | |
| `mod(a, b)` | Euclidean remainder of `i`s |
| `random()` `random_int(lo, hi)` `time()` | kept for older scripts; see `random` and `time` |
| `pi tau e inf` | constants |

## `random`

One generator per program, seeded from the clock (or `skol run --seed n`).

| Name | Returns | |
|---|---|---|
| `seed(n)` | void | restart the sequence (replays, tests) |
| `randf()` | `f` | `[0, 1)` |
| `randi()` | `i` | non-negative 63-bit integer |
| `randi_range(lo, hi)` | `i` | `lo..hi` inclusive |
| `randf_range(lo, hi)` | `f` | `[lo, hi)` |
| `chance(p)` | `bool` | true with probability `p` (0.0–1.0) |
| `rand_dir()` | `vec2` | random unit vector |

Tables: `.edit(shuffle)` and `.view(pick)` use the same generator.

## `time`

| Name | Returns | |
|---|---|---|
| `time()` | `f` | seconds since the program started |
| `ticks_msec()` `ticks_usec()` | `i` | milliseconds / microseconds since start |
| `now()` | `f` | Unix time (seconds since 1970-01-01 UTC) |
| `sleep(seconds)` | void | pause (max one day) |
| `date()` / `date(unix)` | `T8[s, i]` | keys `year month day hour minute second weekday yearday` (weekday 0 = Sunday) |
| `date_string()` / `date_string(unix)` | `s` | `"2026-11-25 21:13:00"` |

Dates are **UTC**: Skol can't read your time zone yet. For local time, add
your offset: `date_string(now() - 8.0 * 3600.0)` for UTC−8.

## `os`

| Name | Returns | |
|---|---|---|
| `arg_count()` / `arg(i)` | `i` / `s` | command-line arguments after the script: `skol run game.skl level2 hard` |
| `env(name)` | `s` | an environment variable, or `""` |
| `has_env(name)` | `bool` | |
| `platform()` | `s` | `"windows"`, `"linux"` or `"macos"` |
| `exit(code)` | void | stop the program with an exit code |

To pass arguments that look like `skol` options, put `--` first:
`skol run tool.skl -- --verbose`.

## `io`

Paths are relative to the folder you run `skol` from. Failures (missing
file, no permission) are runtime errors.

| Name | Returns | |
|---|---|---|
| `read_file(path)` | `s` | whole file |
| `read_lines(path)` | `T[s]` | lines without line endings; capacity from the destination: `var lines: T512[s] = read_lines("level.txt")` |
| `write_file(path, text)` / `append_file(path, text)` | void | |
| `file_exists(path)` / `dir_exists(path)` | `bool` | |
| `list_dir(path)` | `T[s]` | names in a folder, sorted; capacity from the destination |
| `make_dir(path)` | void | creates missing parent folders too |
| `delete_file(path)` | void | |
| `delete_dir(path)` | void | only removes an **empty** folder |
| `rename_file(from, to)` | void | also moves files |

---

## Standard library (`lib/`)

`skol` finds these from any folder. Imports are searched in this order:

1. the importing file's own folder,
2. folders given with `--lib <dir>`,
3. folders in the `SKOL_PATH` environment variable (separated by `;` on
   Windows, `:` elsewhere),
4. the `lib` folder next to the `skol` executable, or one or two folders
   above it (so `target\release\skol.exe` finds the repo's `lib`).

A file in your project with the same name as a library module wins, so you
can override any `lib` module by copying it next to your script. The built-in
modules (`math`, `random`, `time`, `os`, `io`) can't be shadowed.

### `timers` — `Timer`, `Cooldown`, `Stopwatch`

```
from timers use Timer, Cooldown

var spawner: Timer = Timer(2.0, true)              # every 2 s, repeating
spawner.on_timeout = lambda() -> void { spawn_enemy() }
spawner.start()

class Gun:(Cooldown) { cooldown: f = 0.25 }
if gun.trigger() { fire() }                         # false while cooling down

# each frame:
spawner.tick(dt)
gun.tick_cooldown(dt)
```

- `Timer(duration, repeat)`: `start stop is_stopped progress tick(dt)`;
  `tick` returns how many times it fired. Fields `time_left`, `running`,
  `on_timeout`.
- `Cooldown` component: fields `cooldown`, `cooldown_left`; methods
  `ready trigger tick_cooldown(dt) cooldown_progress`.
- `Stopwatch()`: real time. `start stop reset elapsed elapsed_ms`.

### `fsm` — `StateMachine`

```
from fsm use StateMachine

var ai: StateMachine = StateMachine()
ai.add_state("patrol")
ai.add_state("chase")
ai.on_update("patrol", lambda(dt: f) -> void {
    if sees_player() { ai.change("chase") }
})
ai.on_enter("chase", lambda() -> void { play_sound("alert") })
ai.start("patrol")

# each frame:
ai.update(dt)
```

`add_state on_enter on_exit on_update start change restart back is_in
has_state update`; fields `current`, `previous`, `time_in_state`,
`on_change(from, to)`. Up to 32 states.

### `ease` — easing curves and `Tween`

Curves (`fn(f) -> f`, 0→1): `ease_linear`, `ease_in/out/in_out_quad`,
`_cubic`, `_sine`, `ease_in_expo`, `ease_out_expo`, `ease_in_back`,
`ease_out_back`, `ease_out_elastic`, `ease_in_bounce`, `ease_out_bounce`.

```
from ease use Tween, ease_out_back

var pop: Tween = Tween(0.0, 1.0, 0.3)       # from, to, seconds
pop.easing = ease_out_back
pop.on_update = lambda(v: f) -> void { button.scale = v }
pop.on_done = lambda() -> void { print("shown") }
pop.start()
# each frame: pop.tick(dt)
```

Also `lerp_vec2(a, b, t)`; combine it with a curve: `lerp_vec2(a, b, ease_out_quad(t))`.

### `log` — `Logger`

```
from log use Logger, LOG_DEBUG

var log: Logger = Logger("game")
log.info("level loaded")
log.warn(fmt("low health: {}", hp))
log.level = LOG_DEBUG           # LOG_DEBUG, LOG_INFO (default), LOG_WARN, LOG_ERROR, LOG_OFF
log.file = "game.log"           # also append to a file
```

Lines look like `[   12.35s] WARN  game: low health: 9`. Set
`relative_time = false` for UTC dates, `to_console = false` to silence the
console, or `on_line` to forward lines to an in-game console.

### `testing` — unit tests

```
from testing use test, expect, expect_eq_i, expect_eq_s, expect_near, test_summary
from math use clamp

fn main() -> i {
    test("clamp keeps hp in range", lambda() -> void {
        expect_eq_i(clamp(150, 0, 100), 100, "upper bound")
    })
    return test_summary()        # exit code 0 when everything passed
}
```

`test(name, fn)`, `expect(cond, msg)`, `expect_eq_i`, `expect_eq_s`,
`expect_eq_b`, `expect_near(actual, expected, tolerance, msg)`,
`test_quiet(true)` (print only failures), `test_summary()`. A failed
`expect_*` is recorded and the test carries on; a runtime error still stops
the program. The library's own tests in `lib/tests` are examples.

---

## Embedding: what scripts may use

`Engine::new()` is sandboxed. Scripts get the prelude, strings, tables,
`math`, `random` and the read-only parts of `time` and `os` (`time`,
`ticks_*`, `now`, `date*`, `platform`, `arg*`). The rest is opt-in:

| Call | Unlocks |
|---|---|
| `engine.allow_file_access(true)` | the whole `io` module |
| `engine.allow_os(true)` | `env`, `has_env`, `exit` (returns `Error::Exit(code)`), `sleep` |
| `engine.set_args(vec![...])` | what `arg_count()` / `arg(i)` return |
| `engine.add_library_path("path/to/lib")` | imports from that folder (e.g. Skol's `lib`) |
