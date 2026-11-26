# Skol language reference

## Types

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

## Bindings, functions, control flow

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

## Tables

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

More operations — `count find contains min max`, `first last get_or`,
`insert remove swap_remove pop_front extend sort sort_by reverse shuffle`,
`copy slice filter join pick` — are listed in [LIBRARY.md](../LIBRARY.md#tables).
`remove` works on both kinds: `m.edit(remove, "key")` deletes a map entry.

The type checker enforces each operation's argument count and types, and
rejects operations that don't fit the table (e.g. `even` on `T[f]`, `push` on
a key/value table).

## Classes and components

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

## Signals

```
signal died(entity, cause)

died.listen(lambda(who: Health, cause: s) -> void { print(cause) })
died.listen(enemy.on_death)           # bound methods work too
died.emit(e, "fall")                  # or: died(e, "fall")
```

Listeners run in the order they were added. Signal parameters are untyped;
each handler's declared parameter types are checked when the signal fires.

## Function values

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

## Closures

Lambdas capture enclosing variables by reference, so changes are visible
both ways. Each loop iteration gets a fresh copy of the loop variable and of
variables declared inside the body. A lambda bound with `:=` can call itself
recursively.

## Modules

`from Shapes use Circle, hypot` loads `Shapes.skl`, runs its top-level code
once, and brings the named items into scope. Circular imports are an error.
Modules are found next to the importing file, then in `--lib` folders,
`SKOL_PATH`, and the standard library (`lib/` beside the `skol` executable
or up to two folders above it).

Built-in modules: `math`, `random`, `time`, `os`, `io`. Standard library
modules (written in Skol): `timers`, `fsm`, `ease`, `log`, `testing`.
**[LIBRARY.md](../LIBRARY.md) documents every function, method and table
operation.**

## Built-ins

`print(...)`, `fmt(template, ...)`, `input()`, `input(prompt)`, `str(x)`,
`int(x)`, `float(x)`, `chr(code)`, `type_of(x)`, `assert(cond)`,
`assert(cond, msg)`, `error(msg)`, `range(n)`, `range(a, b)`.

`fmt("{} has {.1} hp", name, hp)` fills `{}` placeholders in order; `{.N}`
prints a number with N decimals.

String methods: `len upper lower trim trim_start trim_end contains
starts_with ends_with replace find rfind count substr(start, end) repeat
reverse pad_left pad_right split code is_int is_float is_alpha is_digit
is_space`. `split` takes its capacity from where the result goes:
`var parts: T16[s] = line.split(",")`.
Vector methods: `length normalized dot distance cross` (`cross` is vec3 only).

## Robustness

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

## Errors

Compile errors stop the build and print `file:line:col: error: ...`, pointing at
the exact expression at fault. Runtime
errors (index out of range, full table, division by zero, integer overflow,
stack overflow, failed assert) print the location and a stack trace, and
`skol` exits with code 70.

## Current limits

- **No string interpolation**; use `fmt("{} hp", hp)` or `+` and `str()`.
- **No `*=`, `/=` or `%=`**; only `+=` and `-=`.
- **Statements are newline-insensitive**: `return` followed by an expression
  on the next line returns that expression.
- **No exponent float literals** (`1e6`); write `1000000.0`.
- **No generics**: classes hold one concrete type; the built-in table
  operations work on any element type.
