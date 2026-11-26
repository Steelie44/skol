use std::cell::RefCell;
use std::rc::Rc;

use crate::table::Table;

/// A runtime value. Scalars and vectors are copied; tables, instances,
/// functions and signals are shared references.
#[derive(Debug, Clone)]
pub enum Value {
    Void,
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(Rc<str>),
    /// vec2 stored inline: no allocation for the most common game vector.
    Vec2(f64, f64),
    /// vec3 / vec4 (boxed so `Value` stays 24 bytes).
    Vec(Rc<VecN>),
    Table(Rc<RefCell<Table>>),
    Instance(Rc<RefCell<Instance>>),
    Func(Rc<Closure>),
    Signal(Rc<SignalObj>),
    /// Half-open integer range produced by `range(...)`.
    Range(Rc<(i64, i64)>),
    /// Heap cell holding a local captured by a closure. Never visible to
    /// Skol code directly.
    Cell(Rc<RefCell<Value>>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VecN { pub n: u8, pub c: [f64; 4] }

impl VecN {
    pub fn new(n: u8) -> Self { VecN { n, c: [0.0; 4] } }
    pub fn comps(&self) -> &[f64] { &self.c[..self.n as usize] }
    pub fn map2(self, o: VecN, f: impl Fn(f64, f64) -> f64) -> VecN {
        let mut r = self;
        for i in 0..self.n as usize { r.c[i] = f(self.c[i], o.c[i]); }
        r
    }
    pub fn map(self, f: impl Fn(f64) -> f64) -> VecN {
        let mut r = self;
        for i in 0..self.n as usize { r.c[i] = f(self.c[i]); }
        r
    }
    pub fn dot(self, o: VecN) -> f64 { (0..self.n as usize).map(|i| self.c[i] * o.c[i]).sum() }
    pub fn length(self) -> f64 { self.dot(self).sqrt() }
}

#[derive(Debug)]
pub struct Instance { pub class: u32, pub fields: Vec<Value> }

/// A function value: a compiled function plus captured cells, optionally
/// bound to a receiver (`enemy.on_hit` used as a value).
#[derive(Debug)]
pub struct Closure {
    pub func: u32,
    pub upvals: Vec<Rc<RefCell<Value>>>,
    pub bound: Option<Value>,
}

#[derive(Debug)]
pub struct SignalObj { pub id: u32, pub listeners: RefCell<Vec<Value>> }

// ---- Iterative destruction -------------------------------------------------
//
// Rust drops nested structures recursively. A script can build a chain of a
// million closures (or instances, tables, cells) and freeing it would then
// overflow the native stack. These Drop impls move children onto an explicit
// worklist instead, so dropping any value graph uses constant stack.

/// Drop values without recursion.
pub fn drop_values(mut stack: Vec<Value>) {
    while let Some(v) = stack.pop() {
        match v {
            Value::Instance(rc) => {
                if let Ok(cell) = Rc::try_unwrap(rc) { stack.append(&mut cell.into_inner().fields); }
            }
            Value::Table(rc) => {
                if let Ok(cell) = Rc::try_unwrap(rc) { cell.into_inner().take_values(&mut stack); }
            }
            Value::Func(rc) => {
                if let Ok(mut cl) = Rc::try_unwrap(rc) { cl.take_values(&mut stack); }
            }
            Value::Cell(rc) => {
                if let Ok(cell) = Rc::try_unwrap(rc) { stack.push(cell.into_inner()); }
            }
            Value::Signal(rc) => {
                if let Ok(sig) = Rc::try_unwrap(rc) { stack.append(&mut sig.listeners.borrow_mut()); }
            }
            _ => {}
        }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        if self.fields.iter().any(|v| matches!(v, Value::Instance(_) | Value::Table(_) | Value::Func(_) | Value::Cell(_) | Value::Signal(_))) {
            drop_values(std::mem::take(&mut self.fields));
        }
    }
}

impl Closure {
    fn take_values(&mut self, out: &mut Vec<Value>) {
        for rc in self.upvals.drain(..) {
            if let Ok(cell) = Rc::try_unwrap(rc) { out.push(cell.into_inner()); }
        }
        if let Some(b) = self.bound.take() { out.push(b); }
    }
}

impl Drop for Closure {
    fn drop(&mut self) {
        if self.upvals.is_empty() && self.bound.is_none() { return; }
        let mut out = Vec::new();
        self.take_values(&mut out);
        drop_values(out);
    }
}

impl Drop for SignalObj {
    fn drop(&mut self) {
        let listeners = std::mem::take(&mut *self.listeners.borrow_mut());
        if !listeners.is_empty() { drop_values(listeners); }
    }
}

impl Value {
    pub fn str(s: &str) -> Value { Value::Str(s.into()) }
    pub fn vec(v: VecN) -> Value {
        if v.n == 2 { Value::Vec2(v.c[0], v.c[1]) } else { Value::Vec(Rc::new(v)) }
    }
    /// The vector held by this value, if it is one.
    #[inline(always)]
    pub fn as_vec(&self) -> Option<VecN> {
        match self {
            Value::Vec2(x, y) => Some(VecN { n: 2, c: [*x, *y, 0.0, 0.0] }),
            Value::Vec(v) => Some(**v),
            _ => None,
        }
    }
    /// Store a vector into this slot, reusing an unshared box when possible.
    #[inline(always)]
    pub fn set_vec(&mut self, v: VecN) {
        if v.n == 2 {
            if let Value::Vec2(a, b) = self { *a = v.c[0]; *b = v.c[1]; return; }
            *self = Value::Vec2(v.c[0], v.c[1]);
            return;
        }
        if let Value::Vec(rc) = self {
            if let Some(m) = Rc::get_mut(rc) { *m = v; return; }
        }
        *self = Value::Vec(Rc::new(v));
    }
    pub fn range(a: i64, b: i64) -> Value { Value::Range(Rc::new((a, b))) }

    /// Structural equality for scalars, strings and vectors; identity for
    /// reference types.
    pub fn equals(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Void, Value::Void) => true,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Vec2(a, b), Value::Vec2(c, d)) => a == c && b == d,
            (Value::Vec(a), Value::Vec(b)) => **a == **b,
            (Value::Range(a), Value::Range(b)) => a == b,
            (Value::Table(a), Value::Table(b)) => Rc::ptr_eq(a, b),
            (Value::Instance(a), Value::Instance(b)) => Rc::ptr_eq(a, b),
            (Value::Func(a), Value::Func(b)) => Rc::ptr_eq(a, b) || (a.func == b.func && a.upvals.is_empty() && b.upvals.is_empty() && a.bound.is_none() && b.bound.is_none()),
            (Value::Signal(a), Value::Signal(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }
}
