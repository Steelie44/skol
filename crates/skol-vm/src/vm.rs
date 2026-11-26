//! The Skol virtual machine: a register machine in safe Rust.
//!
//! All frames share one register file (`regs`). A frame's registers are the
//! slots `base..base + max_regs`; a call places its arguments at the top of
//! the caller's window, and that position becomes the callee's `base`, so
//! arguments are never copied. The result is written straight into the
//! caller's destination register.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;
use std::time::Instant;

use skol_ir::{Cmp, DisplayType, TableOp, Type, TypeNames};
use skol_stdlib::{self as rt, Closure, Instance, SignalObj, Table, Value, VecN};

use crate::bytecode::{Bytecode, Op};

#[derive(Debug, Clone)]
pub struct TraceEntry { pub function: String, pub file: String, pub line: usize, pub col: usize }

#[derive(Debug, Clone)]
pub struct RuntimeError {
    pub message: String,
    pub trace: Vec<TraceEntry>,
    /// Set when the script called `exit(code)`: not a failure, a request to stop.
    pub exit_code: Option<i32>,
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.trace.first() {
            Some(t) => write!(f, "{}:{}:{}: runtime error: {}", t.file, t.line, t.col, self.message)?,
            None => write!(f, "runtime error: {}", self.message)?,
        }
        for t in &self.trace {
            write!(f, "\n    in {} ({}:{}:{})", t.function, t.file, t.line, t.col)?;
        }
        Ok(())
    }
}

/// Resource limits for running scripts.
#[derive(Debug, Clone)]
pub struct Limits {
    /// Maximum call depth before reporting a stack overflow.
    pub max_depth: usize,
    /// Maximum loop iterations + calls per top-level call (`None` = unlimited).
    pub max_steps: Option<u64>,
    /// How deeply the VM may re-enter itself on the native stack (a signal
    /// handler that emits a signal, a default value that builds a class).
    pub max_nesting: u32,
}

impl Default for Limits {
    fn default() -> Self { Limits { max_depth: 10_000, max_steps: None, max_nesting: 32 } }
}

/// Where a returning frame's value goes.
const TO_HOST: usize = usize::MAX;

struct Frame {
    proto: u32,
    ip: usize,
    base: usize,
    /// Absolute register index for the result, or `TO_HOST`.
    ret: usize,
    closure: Option<Rc<Closure>>,
}

/// A function the host makes callable from scripts.
pub type HostFn = Box<dyn FnMut(&[Value]) -> Result<Value, String>>;

pub struct Vm {
    bc: Bytecode,
    globals: Vec<Value>,
    regs: Vec<Value>,
    frames: Vec<Frame>,
    out: Box<dyn FnMut(&str)>,
    rng: u64,
    start: Instant,
    scratch: Vec<Value>,
    pub limits: Limits,
    steps_left: u64,
    nesting: u32,
    host_fns: Vec<HostFn>,
    /// Allow the `io` module (file access). On for the CLI, off when embedded.
    pub allow_fs: bool,
    /// Allow `os.env`, `os.exit` and `time.sleep`. On for the CLI, off when embedded.
    pub allow_os: bool,
    /// Arguments the script sees through `os.arg_count()` / `os.arg(i)`.
    pub args: Vec<String>,
    exit_request: Option<i32>,
    input: Option<Box<dyn FnMut() -> Option<String>>>,
    /// Caches for name-based access through component types: index
    /// `name * classes + class` holds the method id / field slot, or
    /// `IC_EMPTY`. Built lazily; replaces a string hash lookup per access.
    ic_methods: Vec<u32>,
    ic_fields: Vec<u32>,
    ic_dims: (usize, usize),
}

const IC_EMPTY: u32 = u32::MAX;
/// Above this many (class, name) pairs the caches aren't worth their memory.
const IC_MAX_ENTRIES: usize = 1 << 20;

impl TypeNames for Vm {
    fn class_name(&self, id: u32) -> &str { self.bc.class_name(id) }
    fn comp_name(&self, id: u32) -> &str { self.bc.comp_name(id) }
    fn signal_name(&self, id: u32) -> &str { self.bc.signal_name(id) }
}

impl rt::Host for Vm {
    fn field_names(&self, class: u32) -> &[String] { &self.bc.classes[class as usize].field_names }
    fn func_name(&self, func: u32) -> &str { &self.bc.protos[func as usize].name }
    fn write_out(&mut self, text: &str) { (self.out)(text) }
    fn rng_state(&mut self) -> &mut u64 { &mut self.rng }
    fn elapsed_secs(&self) -> f64 { self.start.elapsed().as_secs_f64() }
    fn fs_allowed(&self) -> bool { self.allow_fs }
    fn os_allowed(&self) -> bool { self.allow_os }
    fn script_args(&self) -> &[String] { &self.args }
    fn request_exit(&mut self, code: i32) { self.exit_request = Some(code); }
    fn read_line(&mut self) -> Result<Option<String>, String> {
        match &mut self.input {
            Some(f) => Ok(f()),
            None => {
                use std::io::{BufRead, Write};
                let _ = std::io::stdout().flush();
                let mut line = String::new();
                let n = std::io::stdin().lock().read_line(&mut line).map_err(|e| format!("couldn't read input: {e}"))?;
                if n == 0 { return Ok(None); }
                while line.ends_with('\n') || line.ends_with('\r') { line.pop(); }
                Ok(Some(line))
            }
        }
    }
    fn call_host(&mut self, id: u32, args: &[Value]) -> Result<Value, String> {
        match self.host_fns.get_mut(id as usize) {
            Some(f) => f(args),
            None => Err(format!("host function #{id} is not registered")),
        }
    }
}

/// Decimal text of an integer without going through the formatter.
struct IntText { buf: [u8; 20], start: usize }

impl IntText {
    fn as_str(&mut self) -> &str { std::str::from_utf8(&self.buf[self.start..]).unwrap_or("0") }
}

fn itoa_buf(n: i64) -> IntText {
    let mut buf = [0u8; 20];
    let mut i = 20;
    let neg = n < 0;
    let mut m = n.unsigned_abs();
    loop {
        i -= 1;
        buf[i] = b'0' + (m % 10) as u8;
        m /= 10;
        if m == 0 { break; }
    }
    if neg { i -= 1; buf[i] = b'-'; }
    IntText { buf, start: i }
}

fn int_of(v: &Value) -> Option<i64> { if let Value::Int(i) = v { Some(*i) } else { None } }
fn float_of(v: &Value) -> Option<f64> { if let Value::Float(x) = v { Some(*x) } else { None } }

/// Overwrite a register. Skips drop glue when the old value is a scalar.
#[inline(always)]
fn put(slot: &mut Value, v: Value) {
    match slot {
        Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::Void | Value::Vec2(..) => std::mem::forget(std::mem::replace(slot, v)),
        _ => *slot = v,
    }
}

/// Clone with an inline fast path for scalars.
#[inline(always)]
fn fast_clone(v: &Value) -> Value {
    match v {
        Value::Int(n) => Value::Int(*n),
        Value::Float(x) => Value::Float(*x),
        Value::Bool(b) => Value::Bool(*b),
        Value::Vec2(x, y) => Value::Vec2(*x, *y),
        other => other.clone(),
    }
}

#[inline(always)]
fn cmp_i(c: Cmp, x: i64, y: i64) -> bool {
    match c { Cmp::Lt => x < y, Cmp::Le => x <= y, Cmp::Gt => x > y, Cmp::Ge => x >= y }
}

fn cmp_ord(c: Cmp, o: Option<std::cmp::Ordering>) -> bool {
    use std::cmp::Ordering::*;
    matches!((c, o), (Cmp::Lt, Some(Less)) | (Cmp::Le, Some(Less | Equal)) | (Cmp::Gt, Some(Greater)) | (Cmp::Ge, Some(Greater | Equal)))
}

fn line_col(source: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(source.len());
    let mut line = 1;
    let mut col = 1;
    for (i, ch) in source.char_indices() {
        if i >= offset { break; }
        if ch == '\n' { line += 1; col = 1; } else { col += 1; }
    }
    (line, col)
}

impl Vm {
    pub fn new(bc: Bytecode) -> Vm {
        use std::io::Write;
        Vm::with_output(bc, Box::new(|s: &str| {
            let mut out = std::io::stdout().lock();
            let _ = out.write_all(s.as_bytes());
        }))
    }

    pub fn with_output(bc: Bytecode, out: Box<dyn FnMut(&str)>) -> Vm {
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
        let mut vm = Vm {
            bc, globals: Vec::new(), regs: Vec::with_capacity(4096), frames: Vec::with_capacity(64),
            out, rng: seed | 1, start: Instant::now(), scratch: Vec::with_capacity(8),
            limits: Limits::default(), steps_left: u64::MAX, nesting: 0, host_fns: Vec::new(), allow_fs: true,
            allow_os: true, args: Vec::new(), exit_request: None, input: None,
            ic_methods: Vec::new(), ic_fields: Vec::new(), ic_dims: (0, 0),
        };
        vm.sync_globals();
        vm
    }

    pub fn bytecode(&self) -> &Bytecode { &self.bc }
    pub fn bytecode_mut(&mut self) -> &mut Bytecode { &mut self.bc }

    /// Make room for globals added since the last call (REPL).
    pub fn sync_globals(&mut self) { self.globals.resize(self.bc.global_names.len(), Value::Void); }

    /// Seed the random number generator (for reproducible runs and tests).
    pub fn seed(&mut self, seed: u64) { self.rng = seed | 1; }

    /// Replace where `input()` reads from (`None` from the closure = end of input).
    pub fn set_input(&mut self, f: Box<dyn FnMut() -> Option<String>>) { self.input = Some(f); }

    /// Register a host function; returns the id the checker must be told about.
    pub fn add_host_fn(&mut self, f: HostFn) -> u32 {
        self.host_fns.push(f);
        (self.host_fns.len() - 1) as u32
    }

    pub fn global(&self, id: u32) -> Option<&Value> { self.globals.get(id as usize) }

    /// Overwrite a global (the embedding API checks its type first).
    pub fn set_global(&mut self, id: u32, v: Value) {
        if let Some(slot) = self.globals.get_mut(id as usize) { *slot = v; }
    }

    /// Run every module initialiser, then `main` if present. Returns the
    /// value `main` returned if it was an `i`, else 0.
    pub fn run_program(&mut self) -> Result<i64, RuntimeError> {
        self.sync_globals();
        for init in self.bc.module_inits.clone() { self.call(init, vec![])?; }
        if let Some(m) = self.bc.main {
            if let Value::Int(code) = self.call(m, vec![])? { return Ok(code); }
        }
        Ok(0)
    }

    /// First free register above every active frame.
    fn top(&self) -> usize {
        match self.frames.last() {
            Some(f) => f.base + self.bc.protos[f.proto as usize].max_regs as usize,
            None => 0,
        }
    }

    /// Call a function by id and run it to completion.
    pub fn call(&mut self, func: u32, args: Vec<Value>) -> Result<Value, RuntimeError> {
        self.call_inner(func, None, args)
    }

    /// Call a function value (closure or bound method).
    pub fn call_value(&mut self, f: &Value, args: Vec<Value>) -> Result<Value, RuntimeError> {
        let Value::Func(cl) = f else { return Err(self.error("value is not callable".into())) };
        self.call_inner(cl.func, Some(cl.clone()), args)
    }

    fn call_inner(&mut self, func: u32, closure: Option<Rc<Closure>>, args: Vec<Value>) -> Result<Value, RuntimeError> {
        let Some(proto) = self.bc.protos.get(func as usize) else {
            return Err(self.error(format!("no function #{func}")));
        };
        let mut n = args.len();
        if let Some(cl) = &closure { if cl.bound.is_some() { n += 1; } }
        if n != proto.arity as usize {
            return Err(self.error(format!("{} expects {} arguments, got {}", proto.name, proto.arity, n)));
        }
        let outermost = self.frames.is_empty();
        if outermost {
            self.steps_left = self.limits.max_steps.unwrap_or(u64::MAX);
            self.nesting = 0;
        } else if self.nesting >= self.limits.max_nesting {
            return Err(self.error(format!(
                "signals or default values nested too deeply (more than {} levels)", self.limits.max_nesting)));
        }
        let base = self.top();
        let stop = self.frames.len();
        if let Err(m) = self.enter(func, base, TO_HOST, closure.clone()) { return Err(self.error(m)); }
        let mut i = base;
        if let Some(recv) = closure.as_ref().and_then(|c| c.bound.clone()) { self.regs[i] = recv; i += 1; }
        for a in args { self.regs[i] = a; i += 1; }
        self.nesting += 1;
        let r = self.run(stop);
        self.nesting -= 1;
        match r {
            Ok(v) => Ok(v),
            Err(mut e) => {
                self.frames.truncate(stop);
                if outermost {
                    if let Some(code) = self.exit_request.take() { e.exit_code = Some(code); }
                }
                Err(e)
            }
        }
    }

    #[inline(always)]
    fn enter(&mut self, func: u32, base: usize, ret: usize, closure: Option<Rc<Closure>>) -> Result<(), String> {
        if self.frames.len() >= self.limits.max_depth {
            return Err(format!("stack overflow (more than {} nested calls)", self.limits.max_depth));
        }
        if self.steps_left == 0 { return Err("step limit exceeded".into()); }
        self.steps_left -= 1;
        let need = base + self.bc.protos[func as usize].max_regs as usize;
        if self.regs.len() < need { self.regs.resize(need, Value::Void); }
        self.frames.push(Frame { proto: func, ip: 0, base, ret, closure });
        Ok(())
    }

    fn error(&self, message: String) -> RuntimeError {
        let mut trace = Vec::new();
        for f in self.frames.iter().rev() {
            let p = &self.bc.protos[f.proto as usize];
            let span = p.spans.get(f.ip.saturating_sub(1)).copied().unwrap_or_default();
            let Some(file) = self.bc.files.get(p.file as usize) else { continue };
            let (line, col) = line_col(&file.source, span.start);
            trace.push(TraceEntry { function: p.name.clone(), file: file.path.clone(), line, col });
        }
        RuntimeError { message, trace, exit_code: None }
    }

    fn type_name(&self, t: &Type) -> String { DisplayType(t, self).to_string() }

    /// Zero/default value of a type (`.iter(cut, ...)`, fields without defaults).
    fn zero_value(&mut self, ty: &Type) -> Result<Value, RuntimeError> {
        Ok(match ty {
            Type::Int => Value::Int(0),
            Type::Float => Value::Float(0.0),
            Type::Str => Value::str(""),
            Type::Bool => Value::Bool(false),
            Type::Vec(n) => Value::vec(VecN::new(*n)),
            Type::List { cap, elem } => Value::Table(Rc::new(RefCell::new(Table::new_list(*cap, (**elem).clone(), vec![])))),
            Type::Map { cap, key, val } => Value::Table(Rc::new(RefCell::new(Table::new_map(*cap, (**key).clone(), (**val).clone())))),
            Type::Class(c) => {
                let ctor = self.bc.classes[*c as usize].default_ctor;
                self.call(ctor, vec![])?
            }
            other => return Err(self.error(format!("type {} has no zero value", self.type_name(other)))),
        })
    }

    /// Check (and int->float widen) a value passed to a signal handler.
    fn conform(&self, v: Value, ty: &Type) -> Result<Value, String> {
        let ok = match (ty, &v) {
            (Type::Any, _) => true,
            (Type::Float, Value::Int(i)) => return Ok(Value::Float(*i as f64)),
            (Type::Int, Value::Int(_)) | (Type::Float, Value::Float(_)) | (Type::Str, Value::Str(_)) | (Type::Bool, Value::Bool(_)) => true,
            (Type::Vec(n), Value::Vec2(..) | Value::Vec(_)) => v.as_vec().map(|x| x.n) == Some(*n),
            (Type::List { .. } | Type::Map { .. }, Value::Table(t)) => t.borrow().ty() == *ty,
            (Type::Class(c), Value::Instance(i)) => i.borrow().class == *c,
            (Type::Comp(k), Value::Instance(i)) => self.bc.classes[i.borrow().class as usize].comps.contains(k),
            (Type::Func { .. }, Value::Func(_)) => true,
            (Type::Signal(s), Value::Signal(x)) => x.id == *s,
            (Type::Range, Value::Range(..)) => true,
            _ => false,
        };
        if ok { Ok(v) } else {
            Err(format!("expected {}, got {}", self.type_name(ty), rt::format::type_of(&v, self)))
        }
    }

    fn emit_signal(&mut self, sig: &SignalObj, args: Vec<Value>) -> Result<(), RuntimeError> {
        let listeners = sig.listeners.borrow().clone();
        for h in listeners {
            let Value::Func(cl) = &h else { continue };
            let params = &self.bc.protos[cl.func as usize].params;
            let params: Vec<Type> = if cl.bound.is_some() { params.get(1..).unwrap_or(&[]).to_vec() } else { params.clone() };
            if params.len() != args.len() { continue; }
            let mut conformed = Vec::with_capacity(args.len());
            for (i, (a, t)) in args.iter().zip(params.iter()).enumerate() {
                match self.conform(a.clone(), t) {
                    Ok(v) => conformed.push(v),
                    Err(m) => return Err(self.error(format!(
                        "signal '{}' handler {}: parameter {} {m}",
                        self.bc.signal_names[sig.id as usize], self.bc.protos[cl.func as usize].name, i + 1))),
                }
            }
            self.call_value(&h, conformed)?;
        }
        Ok(())
    }

    /// Method id for `name` on `class`, through the cache.
    #[inline]
    fn method_for(&mut self, class: u32, name: u32) -> Option<u32> {
        let idx = self.ic_index(class, name);
        if let Some(i) = idx {
            let v = self.ic_methods[i];
            if v != IC_EMPTY { return Some(v); }
        }
        let n = &self.bc.names[name as usize];
        let f = *self.bc.classes.get(class as usize)?.methods.get(&**n)?;
        if let Some(i) = idx { self.ic_methods[i] = f; }
        Some(f)
    }

    /// Field slot for `name` on `class`, through the cache.
    #[inline]
    fn field_for(&mut self, class: u32, name: u32) -> Option<u32> {
        let idx = self.ic_index(class, name);
        if let Some(i) = idx {
            let v = self.ic_fields[i];
            if v != IC_EMPTY { return Some(v); }
        }
        let n = &self.bc.names[name as usize];
        let slot = *self.bc.classes.get(class as usize)?.field_index.get(&**n)? as u32;
        if let Some(i) = idx { self.ic_fields[i] = slot; }
        Some(slot)
    }

    #[inline]
    fn ic_index(&mut self, class: u32, name: u32) -> Option<usize> {
        let dims = (self.bc.classes.len(), self.bc.names.len());
        if dims != self.ic_dims {
            // New classes or names (REPL): start over.
            let n = dims.0.saturating_mul(dims.1);
            let n = if n <= IC_MAX_ENTRIES { n } else { 0 };
            self.ic_methods = vec![IC_EMPTY; n];
            self.ic_fields = vec![IC_EMPTY; n];
            self.ic_dims = dims;
        }
        let i = name as usize * dims.0 + class as usize;
        if i < self.ic_methods.len() { Some(i) } else { None }
    }

    /// The dispatch loop. Runs until the frame count drops back to `stop`,
    /// then returns the value the last frame returned.
    fn run(&mut self, stop: usize) -> Result<Value, RuntimeError> {
        let (mut code, mut ip, mut base, mut closure) = {
            let fr = self.frames.last().expect("frame");
            (self.bc.protos[fr.proto as usize].code.clone(), fr.ip, fr.base, fr.closure.clone())
        };

        macro_rules! fail {
            ($($t:tt)*) => {{
                if let Some(fr) = self.frames.last_mut() { fr.ip = ip; }
                return Err(self.error(format!($($t)*)));
            }};
        }
        macro_rules! tri { ($e:expr) => { match $e { Ok(v) => v, Err(m) => fail!("{}", m) } }; }
        macro_rules! save { () => { if let Some(fr) = self.frames.last_mut() { fr.ip = ip; } }; }
        macro_rules! reload {
            () => {{
                let fr = self.frames.last().expect("frame");
                code = self.bc.protos[fr.proto as usize].code.clone();
                ip = fr.ip;
                base = fr.base;
                closure = fr.closure.clone();
            }};
        }
        #[allow(unused_macros)]
        macro_rules! nested { ($e:expr) => {{ save!(); match $e { Ok(v) => v, Err(e) => return Err(e) } }}; }
        macro_rules! reg { ($r:expr) => { self.regs[base + $r as usize] }; }
        macro_rules! set { ($r:expr, $v:expr) => {{ let v = $v; put(&mut self.regs[base + $r as usize], v); }}; }
        macro_rules! int {
            ($r:expr) => { match int_of(&reg!($r)) { Some(x) => x, None => fail!("internal error: expected an integer register") } };
        }
        macro_rules! float {
            ($r:expr) => { match float_of(&reg!($r)) { Some(x) => x, None => fail!("internal error: expected a float register") } };
        }
        macro_rules! set_int {
            ($r:expr, $x:expr) => {{
                let x = $x;
                match &mut reg!($r) { Value::Int(slot) => *slot = x, slot => put(slot, Value::Int(x)) }
            }};
        }
        macro_rules! set_float {
            ($r:expr, $x:expr) => {{
                let x = $x;
                match &mut reg!($r) { Value::Float(slot) => *slot = x, slot => put(slot, Value::Float(x)) }
            }};
        }
        macro_rules! int_op {
            ($d:expr, $a:expr, $b:expr, $m:ident, $what:expr) => {{
                let (x, y) = (int!($a), int!($b));
                match x.$m(y) { Some(r) => set_int!($d, r), None => fail!("integer overflow in {}", $what) }
            }};
        }
        macro_rules! float_op {
            ($d:expr, $a:expr, $b:expr, $o:tt) => {{ let (x, y) = (float!($a), float!($b)); set_float!($d, x $o y); }};
        }
        macro_rules! vec_of {
            ($r:expr) => { match reg!($r).as_vec() { Some(v) => v, None => fail!("internal error: expected a vector register") } };
        }
        macro_rules! instance {
            ($r:expr) => { match &reg!($r) { Value::Instance(i) => i.clone(), _ => fail!("internal error: expected an instance") } };
        }

        loop {
            let Some(op) = code.get(ip) else { fail!("internal error: instruction pointer out of range") };
            ip += 1;
            match *op {
                Op::Move(d, s) => { let v = fast_clone(&reg!(s)); set!(d, v); }
                Op::LoadK(d, k) => { let v = fast_clone(&self.bc.consts[k as usize]); set!(d, v); }
                Op::LoadInt(d, n) => set_int!(d, n as i64),
                Op::LoadBool(d, b) => set!(d, Value::Bool(b)),
                Op::LoadVoid(d) => set!(d, Value::Void),
                Op::LoadGlobal(d, g) => {
                    let v = fast_clone(&self.globals[g as usize]);
                    if matches!(v, Value::Void) {
                        fail!("'{}' was used before it was initialized", self.bc.global_names[g as usize]);
                    }
                    set!(d, v);
                }
                Op::StoreGlobal(g, s) => { let v = fast_clone(&reg!(s)); put(&mut self.globals[g as usize], v); }
                Op::LoadCell(d, l) => {
                    let v = match &reg!(l) { Value::Cell(c) => c.borrow().clone(), other => fast_clone(other) };
                    set!(d, v);
                }
                Op::StoreCell(l, s) => {
                    let v = fast_clone(&reg!(s));
                    match &reg!(l) {
                        Value::Cell(c) => { let c = c.clone(); *c.borrow_mut() = v; }
                        _ => set!(l, v),
                    }
                }
                Op::LoadUpval(d, u) => {
                    let Some(cl) = &closure else { fail!("internal error: no closure") };
                    let v = cl.upvals[u as usize].borrow().clone();
                    set!(d, v);
                }
                Op::StoreUpval(u, s) => {
                    let v = fast_clone(&reg!(s));
                    let Some(cl) = &closure else { fail!("internal error: no closure") };
                    *cl.upvals[u as usize].borrow_mut() = v;
                }
                Op::AddI(d, a, b) => int_op!(d, a, b, checked_add, "+"),
                Op::SubI(d, a, b) => int_op!(d, a, b, checked_sub, "-"),
                Op::MulI(d, a, b) => int_op!(d, a, b, checked_mul, "*"),
                Op::DivI(d, a, b) => {
                    if int!(b) == 0 { fail!("division by zero") }
                    int_op!(d, a, b, checked_div, "/")
                }
                Op::ModI(d, a, b) => {
                    if int!(b) == 0 { fail!("modulo by zero") }
                    int_op!(d, a, b, checked_rem_euclid, "%")
                }
                Op::AddIK(d, a, k) => {
                    let x = int!(a);
                    match x.checked_add(k as i64) {
                        Some(r) => set_int!(d, r),
                        None => fail!("integer overflow in {}", if k >= 0 { "+" } else { "-" }),
                    }
                }
                Op::MulIK(d, a, k) => {
                    let x = int!(a);
                    match x.checked_mul(k as i64) { Some(r) => set_int!(d, r), None => fail!("integer overflow in *") }
                }
                Op::DivIK(d, a, k) => { let x = int!(a); set_int!(d, x / k as i64); }
                Op::ModIK(d, a, k) => { let x = int!(a); set_int!(d, x.rem_euclid(k as i64)); }
                Op::CmpIK(d, a, k, c) => { let x = int!(a); set!(d, Value::Bool(cmp_i(c, x, k as i64))); }
                Op::AddF(d, a, b) => float_op!(d, a, b, +),
                Op::SubF(d, a, b) => float_op!(d, a, b, -),
                Op::MulF(d, a, b) => float_op!(d, a, b, *),
                Op::DivF(d, a, b) => float_op!(d, a, b, /),
                Op::ModF(d, a, b) => { let (x, y) = (float!(a), float!(b)); set_float!(d, x.rem_euclid(y)); }
                Op::VecAdd(d, a, b) => {
                    // vec2 fast path: no conversion through VecN.
                    if let (Value::Vec2(x1, y1), Value::Vec2(x2, y2)) = (&reg!(a), &reg!(b)) {
                        let v = Value::Vec2(*x1 + *x2, *y1 + *y2);
                        set!(d, v);
                    } else {
                        let (x, y) = (vec_of!(a), vec_of!(b));
                        let nv = x.map2(y, |p, q| p + q);
                        reg!(d).set_vec(nv);
                    }
                }
                Op::VecSub(d, a, b) => {
                    // vec2 fast path: no conversion through VecN.
                    if let (Value::Vec2(x1, y1), Value::Vec2(x2, y2)) = (&reg!(a), &reg!(b)) {
                        let v = Value::Vec2(*x1 - *x2, *y1 - *y2);
                        set!(d, v);
                    } else {
                        let (x, y) = (vec_of!(a), vec_of!(b));
                        let nv = x.map2(y, |p, q| p - q);
                        reg!(d).set_vec(nv);
                    }
                }
                Op::VecMul(d, a, b) => {
                    // vec2 fast path: no conversion through VecN.
                    if let (Value::Vec2(x1, y1), Value::Vec2(x2, y2)) = (&reg!(a), &reg!(b)) {
                        let v = Value::Vec2(*x1 * *x2, *y1 * *y2);
                        set!(d, v);
                    } else {
                        let (x, y) = (vec_of!(a), vec_of!(b));
                        let nv = x.map2(y, |p, q| p * q);
                        reg!(d).set_vec(nv);
                    }
                }
                Op::VecScale(d, a, b) => {
                    let k = float!(b);
                    if let Value::Vec2(x, y) = &reg!(a) {
                        let v = Value::Vec2(*x * k, *y * k);
                        set!(d, v);
                    } else {
                        let x = vec_of!(a);
                        let nv = x.map(|p| p * k);
                        reg!(d).set_vec(nv);
                    }
                }
                Op::ScaleVec(d, a, b) => { let (k, x) = (float!(a), vec_of!(b)); { let nv = x.map(|p| p * k); reg!(d).set_vec(nv); } }
                Op::VecDiv(d, a, b) => { let (x, k) = (vec_of!(a), float!(b)); { let nv = x.map(|p| p / k); reg!(d).set_vec(nv); } }
                Op::CmpI(d, a, b, c) => { let (x, y) = (int!(a), int!(b)); set!(d, Value::Bool(cmp_i(c, x, y))); }
                Op::CmpF(d, a, b, c) => { let (x, y) = (float!(a), float!(b)); set!(d, Value::Bool(cmp_ord(c, x.partial_cmp(&y)))); }
                Op::Eq(d, a, b) => { let r = reg!(a).equals(&reg!(b)); set!(d, Value::Bool(r)); }
                Op::Ne(d, a, b) => { let r = !reg!(a).equals(&reg!(b)); set!(d, Value::Bool(r)); }
                Op::NegI(d, a) => {
                    let x = int!(a);
                    match x.checked_neg() { Some(r) => set_int!(d, r), None => fail!("integer overflow in negation") }
                }
                Op::NegF(d, a) => { let x = float!(a); set_float!(d, -x); }
                Op::NegV(d, a) => { let x = vec_of!(a); { let nv = x.map(|p| -p); reg!(d).set_vec(nv); } }
                Op::Not(d, a) => {
                    let b = match reg!(a) { Value::Bool(b) => b, _ => fail!("internal error: expected a bool") };
                    set!(d, Value::Bool(!b));
                }
                Op::IToF(d, a) => { let x = int!(a); set_float!(d, x as f64); }

                Op::VecGet(d, v, i) => {
                    let x = match &reg!(v) {
                        Value::Vec2(x, y) => if i == 0 { *x } else { *y },
                        Value::Vec(w) => w.c[i as usize],
                        _ => fail!("internal error: expected a vector register"),
                    };
                    set_float!(d, x);
                }
                Op::VecSet(d, v, i, s) => {
                    let mut x = vec_of!(v);
                    x.c[i as usize] = float!(s);
                    { let nv = x; reg!(d).set_vec(nv); }
                }

                Op::Jump(t) => {
                    // Every loop iteration passes through a backward jump, so
                    // counting here (and on calls) bounds any runaway script.
                    if (t as usize) < ip {
                        if self.steps_left == 0 { fail!("step limit exceeded") }
                        self.steps_left -= 1;
                    }
                    ip = t as usize;
                }
                Op::JumpIfFalse(r, t) => { if let Value::Bool(false) = reg!(r) { ip = t as usize; } }
                Op::JumpIfTrue(r, t) => { if let Value::Bool(true) = reg!(r) { ip = t as usize; } }
                Op::CmpIJump(a, b, c, t) => { if !cmp_i(c, int!(a), int!(b)) { ip = t as usize; } }
                Op::CmpIKJump(a, k, c, t) => { if !cmp_i(c, int!(a), k as i64) { ip = t as usize; } }
                Op::CmpFJump(a, b, c, t) => {
                    let (x, y) = (float!(a), float!(b));
                    if !cmp_ord(c, x.partial_cmp(&y)) { ip = t as usize; }
                }
                Op::ForRange(idx, end, d, exit) => {
                    let i = int!(idx);
                    if i < int!(end) {
                        set_int!(idx, i + 1);
                        set_int!(d, i);
                    } else {
                        ip = exit as usize;
                    }
                }
                Op::ForIter(it, idx, d, exit) => {
                    let i = int!(idx);
                    let next = match &reg!(it) {
                        Value::Table(t) => t.borrow().iter_at(i as usize).map(|v| (v, i + 1)),
                        Value::Str(s) => s.get(i as usize..).and_then(|rest| rest.chars().next())
                            .map(|c| (Value::str(c.encode_utf8(&mut [0u8; 4])), i + c.len_utf8() as i64)),
                        Value::Range(r) => { let v = r.0.saturating_add(i); if v < r.1 { Some((Value::Int(v), i + 1)) } else { None } }
                        _ => None,
                    };
                    match next {
                        Some((v, ni)) => { set_int!(idx, ni); set!(d, v); }
                        None => ip = exit as usize,
                    }
                }

                Op::Call(func, b, _argc, d) => {
                    save!();
                    let nb = base + b as usize;
                    tri!(self.enter(func, nb, base + d as usize, None));
                    // We know the new frame: skip reading it back.
                    code = self.bc.protos[func as usize].code.clone();
                    ip = 0;
                    base = nb;
                    closure = None;
                }
                Op::CallValue(c, b, _argc, d) => {
                    let Value::Func(cl) = reg!(c).clone() else { fail!("value is not callable") };
                    save!();
                    let ret = base + d as usize;
                    if let Some(recv) = &cl.bound {
                        set!(b, recv.clone());
                        tri!(self.enter(cl.func, base + b as usize, ret, Some(cl.clone())));
                    } else {
                        tri!(self.enter(cl.func, base + b as usize + 1, ret, Some(cl.clone())));
                    }
                    reload!();
                }
                Op::CallMethodNamed(name, b, _argc, d) => {
                    let class = match &reg!(b) { Value::Instance(i) => i.borrow().class, _ => fail!("internal error: expected an instance") };
                    let Some(func) = self.method_for(class, name) else {
                        fail!("{} has no method '{}'", self.bc.classes[class as usize].name, self.bc.names[name as usize])
                    };
                    save!();
                    tri!(self.enter(func, base + b as usize, base + d as usize, None));
                    reload!();
                }
                Op::FastBuiltin(bi, d, a, b) => {
                    use skol_ir::Builtin as B;
                    match bi {
                        B::Sqrt | B::Sin | B::Cos | B::Tan | B::AbsF => {
                            let x = float!(a);
                            let r = match bi { B::Sqrt => x.sqrt(), B::Sin => x.sin(), B::Cos => x.cos(), B::Tan => x.tan(), _ => x.abs() };
                            set_float!(d, r);
                        }
                        B::Atan2 | B::Pow | B::MinF | B::MaxF => {
                            let (x, y) = (float!(a), float!(b));
                            let r = match bi { B::Atan2 => x.atan2(y), B::Pow => x.powf(y), B::MinF => x.min(y), _ => x.max(y) };
                            set_float!(d, r);
                        }
                        B::VecLength => { let v = vec_of!(a); set_float!(d, v.length()); }
                        B::VecDot => { let (v, w) = (vec_of!(a), vec_of!(b)); set_float!(d, v.dot(w)); }
                        B::VecDistance => {
                            let r = if let (Value::Vec2(x1, y1), Value::Vec2(x2, y2)) = (&reg!(a), &reg!(b)) {
                                ((x1 - x2) * (x1 - x2) + (y1 - y2) * (y1 - y2)).sqrt()
                            } else {
                                let (v, w) = (vec_of!(a), vec_of!(b));
                                v.map2(w, |p, q| p - q).length()
                            };
                            set_float!(d, r);
                        }
                        B::VecNormalized => {
                            let v = vec_of!(a);
                            let len = v.length();
                            let nv = if len == 0.0 { v } else { v.map(|c| c / len) };
                            reg!(d).set_vec(nv);
                        }
                        B::VecCross => {
                            let (p, q) = (vec_of!(a), vec_of!(b));
                            let mut r = VecN::new(3);
                            r.c[0] = p.c[1] * q.c[2] - p.c[2] * q.c[1];
                            r.c[1] = p.c[2] * q.c[0] - p.c[0] * q.c[2];
                            r.c[2] = p.c[0] * q.c[1] - p.c[1] * q.c[0];
                            reg!(d).set_vec(r);
                        }
                        B::FloatToInt => {
                            let x = float!(a);
                            if !x.is_finite() || x >= 9.2e18 || x <= -9.2e18 {
                                fail!("{} can't be converted to an i", rt::format::format_float(x))
                            }
                            set_int!(d, x as i64);
                        }
                        B::MinI | B::MaxI => {
                            let (x, y) = (int!(a), int!(b));
                            set_int!(d, if bi == B::MinI { x.min(y) } else { x.max(y) });
                        }
                        B::StrLen => {
                            let n = match &reg!(a) { Value::Str(s) => if s.is_ascii() { s.len() } else { s.chars().count() }, _ => fail!("internal error: expected a string") };
                            set_int!(d, n as i64);
                        }
                        B::ToStr => {
                            let v = match &reg!(a) {
                                Value::Int(n) => { let mut buf = itoa_buf(*n); Value::str(buf.as_str()) }
                                Value::Str(s) => Value::Str(s.clone()),
                                other => { let o = other.clone(); Value::str(&rt::format::to_display(&o, &*self)) }
                            };
                            set!(d, v);
                        }
                        _ => fail!("internal error: {:?} is not a fast built-in", bi),
                    }
                }
                Op::Return(s) => {
                    let v = fast_clone(&reg!(s));
                    let done = self.frames.pop().expect("frame");
                    if self.frames.len() == stop { return Ok(v); }
                    put(&mut self.regs[done.ret], v);
                    reload!();
                }
                Op::ReturnVoid => {
                    let done = self.frames.pop().expect("frame");
                    if self.frames.len() == stop { return Ok(Value::Void); }
                    put(&mut self.regs[done.ret], Value::Void);
                    reload!();
                }

                Op::GetField(d, o, slot) => {
                    let v = match &reg!(o) {
                        Value::Instance(i) => match i.borrow().fields.get(slot as usize) { Some(v) => fast_clone(v), None => fail!("internal error: bad field") },
                        _ => fail!("internal error: field read on a non-instance"),
                    };
                    set!(d, v);
                }
                Op::Concat(d, a, b) => {
                    let v = match (&reg!(a), &reg!(b)) {
                        (Value::Str(x), Value::Str(y)) => {
                            let n = x.len() + y.len();
                            if n <= 128 {
                                // Short strings: join on the stack, then one allocation.
                                let mut buf = [0u8; 128];
                                buf[..x.len()].copy_from_slice(x.as_bytes());
                                buf[x.len()..n].copy_from_slice(y.as_bytes());
                                match std::str::from_utf8(&buf[..n]) { Ok(t) => Value::str(t), Err(_) => fail!("internal error: bad string") }
                            } else {
                                if n > rt::MAX_STRING_BYTES { fail!("string too long ({} bytes)", n) }
                                let mut t = String::with_capacity(n);
                                t.push_str(x);
                                t.push_str(y);
                                Value::Str(t.into())
                            }
                        }
                        _ => fail!("internal error: expected strings"),
                    };
                    set!(d, v);
                }
                Op::GetFieldNamed(d, o, name) => {
                    let inst = instance!(o);
                    let class = inst.borrow().class;
                    let Some(slot) = self.field_for(class, name) else { fail!("missing field '{}'", self.bc.names[name as usize]) };
                    let v = match inst.borrow().fields.get(slot as usize) { Some(v) => fast_clone(v), None => fail!("internal error: bad field") };
                    set!(d, v);
                }
                Op::SetFieldNamed(o, name, s) => {
                    let v = fast_clone(&reg!(s));
                    let inst = instance!(o);
                    let class = inst.borrow().class;
                    let Some(slot) = self.field_for(class, name) else { fail!("missing field '{}'", self.bc.names[name as usize]) };
                    let ok = match inst.borrow_mut().fields.get_mut(slot as usize) { Some(f) => { put(f, v); true } None => false };
                    if !ok { fail!("internal error: bad field") }
                }
                Op::GetFieldComp(d, o, slot, c) => {
                    let x = match &reg!(o) {
                        Value::Instance(i) => match i.borrow().fields.get(slot as usize) {
                            Some(Value::Vec2(x, y)) => if c == 0 { *x } else { *y },
                            Some(Value::Vec(v)) => v.c[c as usize],
                            _ => fail!("internal error: bad vector field"),
                        },
                        _ => fail!("internal error: field read on a non-instance"),
                    };
                    set_float!(d, x);
                }
                Op::SetField(o, slot, s) => {
                    let v = fast_clone(&reg!(s));
                    match &reg!(o) {
                        Value::Instance(i) => match i.borrow_mut().fields.get_mut(slot as usize) {
                            Some(f) => put(f, v),
                            None => fail!("internal error: bad field"),
                        },
                        _ => fail!("internal error: field write on a non-instance"),
                    }
                }
                // Hot table operations: no reference-count traffic on the table.
                Op::Table(TableOp::Get, d, t, a, _) => {
                    let v = match &reg!(t) {
                        Value::Table(tb) => {
                            let tb = tb.borrow();
                            match (&tb.data, &reg!(a)) {
                                (rt::TableData::List(items), Value::Int(i)) if *i >= 0 && (*i as usize) < items.len() => fast_clone(&items[*i as usize]),
                                _ => tri!(tb.get(&reg!(a))),
                            }
                        }
                        _ => fail!("internal error: table op on a non-table"),
                    };
                    set!(d, v);
                }
                Op::Table(TableOp::Set, _, t, a, b) => {
                    let v = fast_clone(&reg!(b));
                    let r = match &reg!(t) {
                        Value::Table(tb) => {
                            let mut tb = tb.borrow_mut();
                            match (&mut tb.data, &reg!(a)) {
                                (rt::TableData::List(items), Value::Int(i)) if *i >= 0 && (*i as usize) < items.len() => { put(&mut items[*i as usize], v); Ok(()) }
                                _ => tb.set(&reg!(a), v),
                            }
                        }
                        _ => fail!("internal error: table op on a non-table"),
                    };
                    tri!(r);
                }
                Op::Table(TableOp::Push, _, t, a, _) => {
                    let v = fast_clone(&reg!(a));
                    let r = match &reg!(t) { Value::Table(tb) => tb.borrow_mut().push(v), _ => fail!("internal error: table op on a non-table") };
                    tri!(r);
                }
                Op::Table(TableOp::Len, d, t, _, _) => {
                    let n = match &reg!(t) { Value::Table(tb) => tb.borrow().len(), _ => fail!("internal error: table op on a non-table") };
                    set_int!(d, n as i64);
                }
                Op::Table(TableOp::Has, d, t, a, _) => {
                    let h = match &reg!(t) { Value::Table(tb) => tb.borrow().has(&reg!(a)), _ => fail!("internal error: table op on a non-table") };
                    set!(d, Value::Bool(h));
                }
                other => {
                    save!();
                    self.run_slow(other, base, closure.as_ref())?;
                }
            }
        }
    }


    /// Instructions that are rare in hot loops. Keeping them out of `run`
    /// keeps the dispatch loop small, which leaves the compiler more
    /// registers for the common path. The caller has saved the instruction
    /// pointer, so errors here get the right location.
    #[inline(never)]
    fn run_slow(&mut self, op: Op, base: usize, closure: Option<&Rc<Closure>>) -> Result<(), RuntimeError> {
        macro_rules! fail { ($($t:tt)*) => {{ return Err(self.error(format!($($t)*))); }}; }
        macro_rules! tri { ($e:expr) => { match $e { Ok(v) => v, Err(m) => fail!("{}", m) } }; }
        macro_rules! nested { ($e:expr) => {{ match $e { Ok(v) => v, Err(e) => return Err(e) } }}; }
        // The caller already saved the instruction pointer.
        macro_rules! save { () => {}; }
        macro_rules! reg { ($r:expr) => { self.regs[base + $r as usize] }; }
        macro_rules! set { ($r:expr, $v:expr) => {{ let v = $v; put(&mut self.regs[base + $r as usize], v); }}; }
        macro_rules! instance {
            ($r:expr) => { match &reg!($r) { Value::Instance(i) => i.clone(), _ => fail!("internal error: expected an instance") } };
        }
        match op {
            Op::NewCell(l) => set!(l, Value::Cell(Rc::new(RefCell::new(Value::Void)))),
            Op::WrapCell(l) => {
                let v = std::mem::replace(&mut reg!(l), Value::Void);
                set!(l, Value::Cell(Rc::new(RefCell::new(v))));
            }
            Op::LoadUpvalRaw(d, u) => {
                let Some(cl) = &closure else { fail!("internal error: no closure") };
                let c = cl.upvals[u as usize].clone();
                set!(d, Value::Cell(c));
            }

            Op::CmpS(d, a, b, c) => {
                let r = match (&reg!(a), &reg!(b)) {
                    (Value::Str(x), Value::Str(y)) => cmp_ord(c, x.partial_cmp(y)),
                    _ => fail!("internal error: expected strings"),
                };
                set!(d, Value::Bool(r));
            }
            Op::MakeVec(d, b, n) => {
                let mut v = VecN::new(n);
                for i in 0..n as usize {
                    if let Value::Float(x) = reg!(b as usize + i) { v.c[i] = x; }
                }
                { let nv = v; reg!(d).set_vec(nv); }
            }
            Op::CallBuiltin(bi, b, argc, d) => {
                let mut args = std::mem::take(&mut self.scratch);
                args.clear();
                let start = base + b as usize;
                args.extend(self.regs[start..start + argc as usize].iter().map(fast_clone));
                save!();
                let r = rt::builtins::call(bi, &args, self);
                args.clear();
                self.scratch = args;
                let v = tri!(r);
                set!(d, v);
            }
            Op::NewInstance(d, c) => {
                let n = self.bc.classes[c as usize].field_names.len();
                set!(d, Value::Instance(Rc::new(RefCell::new(Instance { class: c, fields: vec![Value::Void; n] }))));
            }
            Op::Closure(d, func, b, n) => {
                let mut upvals = Vec::with_capacity(n as usize);
                for i in 0..n as usize {
                    match &reg!(b as usize + i) {
                        Value::Cell(c) => upvals.push(c.clone()),
                        other => upvals.push(Rc::new(RefCell::new(other.clone()))),
                    }
                }
                set!(d, Value::Func(Rc::new(Closure { func, upvals, bound: None })));
            }
            Op::BindMethod(d, r, func) => {
                let recv = reg!(r).clone();
                set!(d, Value::Func(Rc::new(Closure { func, upvals: vec![], bound: Some(recv) })));
            }
            Op::BindMethodNamed(d, r, name) => {
                let inst = instance!(r);
                let class = inst.borrow().class;
                let n = &self.bc.names[name as usize];
                let Some(&func) = self.bc.classes[class as usize].methods.get(&**n) else { fail!("missing method '{}'", n) };
                set!(d, Value::Func(Rc::new(Closure { func, upvals: vec![], bound: Some(Value::Instance(inst)) })));
            }

            Op::MakeList(d, t, b, n) => {
                let Type::List { cap, elem } = &self.bc.types[t as usize] else { fail!("internal error: bad list type") };
                let start = base + b as usize;
                let items: Vec<Value> = self.regs[start..start + n as usize].to_vec();
                let table = Table::new_list(*cap, (**elem).clone(), items);
                set!(d, Value::Table(Rc::new(RefCell::new(table))));
            }
            Op::MakeMap(d, t, b, n) => {
                let Type::Map { cap, key, val } = &self.bc.types[t as usize] else { fail!("internal error: bad map type") };
                let mut table = Table::new_map(*cap, (**key).clone(), (**val).clone());
                for i in 0..n as usize {
                    let k = &self.regs[base + b as usize + 2 * i];
                    let v = self.regs[base + b as usize + 2 * i + 1].clone();
                    let key = tri!(rt::Key::from_value(k));
                    tri!(table.map_insert(key, v));
                }
                set!(d, Value::Table(Rc::new(RefCell::new(table))));
            }
            Op::Zero(d, t) => {
                let ty = self.bc.types[t as usize].clone();
                let v = nested!(self.zero_value(&ty));
                set!(d, v);
            }
            Op::Table(top, d, t, a, b) => {
                let Value::Table(tb) = reg!(t).clone() else { fail!("internal error: table op on a non-table") };
                let result = match top {
                    TableOp::Len => Value::Int(tb.borrow().len() as i64),
                    TableOp::Sum => tb.borrow().sum(),
                    TableOp::Even => Value::Table(Rc::new(RefCell::new(tb.borrow().even()))),
                    TableOp::Cut => {
                        let ty = tb.borrow().val_ty.clone();
                        let zero = nested!(self.zero_value(&ty));
                        let k = reg!(a).clone();
                        tri!(tb.borrow_mut().cut(&k, zero));
                        Value::Void
                    }
                    TableOp::Get => tri!(tb.borrow().get(&reg!(a))),
                    TableOp::Set => {
                        let (k, v) = (reg!(a).clone(), reg!(b).clone());
                        tri!(tb.borrow_mut().set(&k, v));
                        Value::Void
                    }
                    TableOp::Has => Value::Bool(tb.borrow().has(&reg!(a))),
                    TableOp::Push => { let v = reg!(a).clone(); tri!(tb.borrow_mut().push(v)); Value::Void }
                    TableOp::Pop => tri!(tb.borrow_mut().pop()),
                    TableOp::Clear => { tb.borrow_mut().clear(); Value::Void }
                    TableOp::IsEmpty => Value::Bool(tb.borrow().is_empty()),
                    TableOp::Keys => Value::Table(Rc::new(RefCell::new(tb.borrow().keys()))),
                    TableOp::Vals => Value::Table(Rc::new(RefCell::new(tb.borrow().vals()))),
                    TableOp::Count => Value::Int(tb.borrow().count(&reg!(a))),
                    TableOp::Find => Value::Int(tb.borrow().find(&reg!(a))),
                    TableOp::Contains => Value::Bool(tb.borrow().contains(&reg!(a))),
                    TableOp::Min => tri!(tb.borrow().extreme(false)),
                    TableOp::Max => tri!(tb.borrow().extreme(true)),
                    TableOp::First => tri!(tb.borrow().first()),
                    TableOp::Last => tri!(tb.borrow().last()),
                    TableOp::GetOr => { let d = reg!(b).clone(); tb.borrow().get_or(&reg!(a), d) }
                    TableOp::Insert => {
                        let (k, v) = (reg!(a).clone(), reg!(b).clone());
                        tri!(tb.borrow_mut().insert(&k, v));
                        Value::Void
                    }
                    TableOp::Remove => { let k = reg!(a).clone(); tri!(tb.borrow_mut().remove(&k)) }
                    TableOp::SwapRemove => { let k = reg!(a).clone(); tri!(tb.borrow_mut().swap_remove(&k)) }
                    TableOp::PopFront => tri!(tb.borrow_mut().pop_front()),
                    TableOp::Extend => {
                        let Value::Table(other) = reg!(a).clone() else { fail!("internal error: extend with a non-table") };
                        // `t.edit(extend, t)` appends a copy of itself.
                        let src = other.borrow().copy();
                        tri!(tb.borrow_mut().extend(&src));
                        Value::Void
                    }
                    TableOp::Sort => { tb.borrow_mut().sort(); Value::Void }
                    TableOp::Reverse => { tb.borrow_mut().reverse(); Value::Void }
                    TableOp::Shuffle => {
                        let mut t = tb.borrow_mut();
                        if let rt::TableData::List(v) = &mut t.data {
                            for i in (1..v.len()).rev() {
                                let j = rt::builtins::random_index(&mut self.rng, i + 1);
                                v.swap(i, j);
                            }
                        }
                        Value::Void
                    }
                    TableOp::Pick => {
                        let t = tb.borrow();
                        let vals = t.values();
                        if vals.is_empty() { fail!("can't pick from an empty table") }
                        vals[rt::builtins::random_index(&mut self.rng, vals.len())].clone()
                    }
                    TableOp::Copy => Value::Table(Rc::new(RefCell::new(tb.borrow().copy()))),
                    TableOp::Slice => {
                        let (x, y) = (reg!(a).clone(), reg!(b).clone());
                        Value::Table(Rc::new(RefCell::new(tri!(tb.borrow().slice(&x, &y)))))
                    }
                    TableOp::Join => {
                        let sep = match &reg!(a) { Value::Str(s) => s.clone(), _ => fail!("internal error: join separator") };
                        let items: Vec<Value> = tb.borrow().values().to_vec();
                        let mut out = String::new();
                        for (i, v) in items.iter().enumerate() {
                            if i > 0 { out.push_str(&sep); }
                            out.push_str(&rt::format::to_display(v, &*self));
                            if out.len() > rt::MAX_STRING_BYTES { fail!("join would build a string that is too long") }
                        }
                        Value::str(&out)
                    }
                    TableOp::SortBy | TableOp::Filter => {
                        let f = reg!(a).clone();
                        // Work on a snapshot: the callback may read or even change the table.
                        let items: Vec<Value> = tb.borrow().values().to_vec();
                        let mut results = Vec::with_capacity(items.len());
                        for v in &items {
                            let r = nested!(self.call_value(&f, vec![v.clone()]));
                            results.push(r);
                        }
                        if top == TableOp::Filter {
                            let kept: Vec<Value> = items.into_iter().zip(results)
                                .filter(|(_, keep)| matches!(keep, Value::Bool(true))).map(|(v, _)| v).collect();
                            let t = tb.borrow();
                            Value::Table(Rc::new(RefCell::new(Table::new_list(t.cap, t.val_ty.clone(), kept))))
                        } else {
                            let mut order: Vec<usize> = (0..items.len()).collect();
                            order.sort_by(|&x, &y| rt::table::cmp_values(&results[x], &results[y]).unwrap_or(std::cmp::Ordering::Equal));
                            let sorted: Vec<Value> = order.into_iter().map(|i| items[i].clone()).collect();
                            tri!(tb.borrow_mut().replace_list(sorted));
                            Value::Void
                        }
                    }
                };
                set!(d, result);
            }

            Op::NewSignal(d, id) => set!(d, Value::Signal(Rc::new(SignalObj { id, listeners: RefCell::new(vec![]) }))),
            Op::Listen(s, h) => {
                let handler = reg!(h).clone();
                let Value::Signal(sig) = &reg!(s) else { fail!("internal error: listen on a non-signal") };
                sig.listeners.borrow_mut().push(handler);
            }
            Op::Emit(s, b, argc) => {
                let Value::Signal(sig) = reg!(s).clone() else { fail!("internal error: emit on a non-signal") };
                let start = base + b as usize;
                let args = self.regs[start..start + argc as usize].to_vec();
                nested!(self.emit_signal(&sig, args));
            }
            _ => fail!("internal error: {:?} is not a slow instruction", op),
        }
        Ok(())
    }

}
