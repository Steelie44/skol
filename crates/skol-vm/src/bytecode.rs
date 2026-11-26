//! Register bytecode and the IR -> bytecode compiler.
//!
//! Every function gets a window of registers. Registers `0..n_locals` hold
//! the IR's local slots (parameters first); temporaries are allocated above
//! them. Instructions name their operands and destination directly
//! (`AddI(dst, a, b)`), so ordinary expressions don't shuffle values through a
//! stack. Calls place arguments in consecutive registers at the top of the
//! caller's window, which become the callee's first registers: no copying.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::rc::Rc;

use skol_ir::{self as ir, BinOp, Builtin, Cmp, ExprKind, Place, Span, StmtKind, TableOp, Type, TypeNames, UnOp, UpvalSource};
use skol_stdlib::Value;

/// A register index within the current frame.
pub type R = u16;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Op {
    // ---- moves and constants
    Move(R, R),
    LoadK(R, u32),
    LoadInt(R, i32),
    LoadBool(R, bool),
    LoadVoid(R),
    LoadGlobal(R, u32),
    StoreGlobal(u32, R),
    /// Read/write through the heap cell held by a captured local.
    LoadCell(R, R),
    StoreCell(R, R),
    /// Put a fresh empty cell in a local (captured `let` / loop variable).
    NewCell(R),
    /// Move a parameter's value into a new cell (captured parameter).
    WrapCell(R),
    LoadUpval(R, u16),
    StoreUpval(u16, R),
    /// The upvalue's cell itself, for capturing it again.
    LoadUpvalRaw(R, u16),

    // ---- arithmetic: (dst, a, b)
    AddI(R, R, R), SubI(R, R, R), MulI(R, R, R), DivI(R, R, R), ModI(R, R, R),
    /// dst = a + constant
    AddIK(R, R, i32),
    /// dst = a * constant, a / constant (constant != 0), a % constant (constant != 0)
    MulIK(R, R, i32), DivIK(R, R, i32), ModIK(R, R, i32),
    /// dst = a <cmp> constant
    CmpIK(R, R, i32, Cmp),
    AddF(R, R, R), SubF(R, R, R), MulF(R, R, R), DivF(R, R, R), ModF(R, R, R),
    Concat(R, R, R),
    VecAdd(R, R, R), VecSub(R, R, R), VecMul(R, R, R),
    VecScale(R, R, R), ScaleVec(R, R, R), VecDiv(R, R, R),
    CmpI(R, R, R, Cmp), CmpF(R, R, R, Cmp), CmpS(R, R, R, Cmp),
    Eq(R, R, R), Ne(R, R, R),
    NegI(R, R), NegF(R, R), NegV(R, R), Not(R, R), IToF(R, R),

    // ---- vectors
    /// dst = vecN from registers base..base+n
    MakeVec(R, R, u8),
    VecGet(R, R, u8),
    /// dst = v with component i replaced by src
    VecSet(R, R, u8, R),

    // ---- control flow
    Jump(u32),
    JumpIfFalse(R, u32),
    JumpIfTrue(R, u32),
    /// Jump unless `a <cmp> b` (fused loop/branch condition).
    CmpIJump(R, R, Cmp, u32),
    /// Jump unless `a <cmp> constant`.
    CmpIKJump(R, i32, Cmp, u32),
    /// Jump unless `a <cmp> b` for floats.
    CmpFJump(R, R, Cmp, u32),
    /// (iterable, cursor, element dst, exit): load the next element or jump out.
    ForIter(R, R, R, u32),
    /// `for k in range(...)`: (counter, end, element dst, exit). If counter <
    /// end, copy it to dst and increment it; otherwise jump out.
    ForRange(R, R, R, u32),

    // ---- calls: arguments live in consecutive registers starting at `base`
    /// (function, base, argc, dst)
    Call(u32, R, u8, R),
    /// (callee, base, argc, dst): base is reserved for a bound receiver, args start at base+1.
    CallValue(R, R, u8, R),
    /// (name, base, argc, dst): receiver at base, args at base+1.
    CallMethodNamed(u32, R, u8, R),
    /// (builtin, base, argc, dst)
    CallBuiltin(Builtin, R, u8, R),
    /// Hot pure built-ins (vector methods, float math) with operands in
    /// registers: (builtin, dst, a, b). Skips the generic call path.
    FastBuiltin(Builtin, R, R, R),
    Return(R),
    ReturnVoid,

    // ---- objects
    NewInstance(R, u32),
    /// (dst, obj, slot)
    GetField(R, R, u16),
    /// (dst, obj, slot, component): `obj.field.x` for a vector field.
    GetFieldComp(R, R, u16, u8),
    /// (obj, slot, src)
    SetField(R, u16, R),
    GetFieldNamed(R, R, u32),
    SetFieldNamed(R, u32, R),
    /// (dst, func, base, n): captured cells in base..base+n
    Closure(R, u32, R, u8),
    BindMethod(R, R, u32),
    BindMethodNamed(R, R, u32),

    // ---- tables
    /// (dst, type, base, n)
    MakeList(R, u32, R, u32),
    /// (dst, type, base, n pairs): key, value, key, value, ...
    MakeMap(R, u32, R, u32),
    Zero(R, u32),
    /// (op, dst, table, arg1, arg2)
    Table(TableOp, R, R, R, R),

    // ---- signals
    NewSignal(R, u32),
    /// (signal, handler)
    Listen(R, R),
    /// (signal, base, argc)
    Emit(R, R, u8),
}

/// A compiled function.
#[derive(Debug, Clone)]
pub struct Proto {
    pub name: String,
    pub arity: u16,
    pub n_locals: u16,
    /// Registers this function needs (locals + temporaries).
    pub max_regs: u16,
    pub code: Rc<[Op]>,
    /// Source span for each instruction (for errors and stack traces).
    pub spans: Rc<[Span]>,
    pub file: u32,
    pub params: Vec<Type>,
    pub ret: Type,
}

#[derive(Debug, Clone)]
pub struct ClassRt {
    pub name: String,
    pub comps: Vec<u32>,
    pub field_names: Vec<String>,
    pub field_index: HashMap<String, u16>,
    pub methods: HashMap<String, u32>,
    pub default_ctor: u32,
}

/// A whole compiled program.
#[derive(Debug, Clone, Default)]
pub struct Bytecode {
    pub protos: Vec<Proto>,
    pub consts: Vec<Value>,
    pub types: Vec<Type>,
    pub names: Vec<Rc<str>>,
    pub classes: Vec<ClassRt>,
    pub comp_names: Vec<String>,
    pub signal_names: Vec<String>,
    pub global_names: Vec<String>,
    pub files: Vec<ir::SourceFile>,
    pub module_inits: Vec<u32>,
    pub main: Option<u32>,
    str_consts: HashMap<Rc<str>, u32>,
    name_index: HashMap<Rc<str>, u32>,
    type_index: Vec<(Type, u32)>,
}

impl TypeNames for Bytecode {
    fn class_name(&self, id: u32) -> &str { &self.classes[id as usize].name }
    fn comp_name(&self, id: u32) -> &str { &self.comp_names[id as usize] }
    fn signal_name(&self, id: u32) -> &str { &self.signal_names[id as usize] }
}

/// Compile a checked program.
pub fn compile(prog: &ir::Program) -> Result<Bytecode, String> {
    let mut bc = Bytecode::default();
    compile_into(prog, &mut bc)?;
    Ok(bc)
}

/// Compile any functions in `prog` that `bc` doesn't have yet and refresh the
/// program tables. Used by the REPL to grow a running program.
pub fn compile_into(prog: &ir::Program, bc: &mut Bytecode) -> Result<(), String> {
    bc.files = prog.files.clone();
    bc.global_names = prog.globals.iter().map(|g| g.name.clone()).collect();
    bc.comp_names = prog.comps.iter().map(|c| c.name.clone()).collect();
    bc.signal_names = prog.signals.iter().map(|s| s.name.clone()).collect();
    bc.classes = prog.classes.iter().map(|c| ClassRt {
        name: c.name.clone(),
        comps: c.comps.clone(),
        field_names: c.fields.iter().map(|f| f.name.clone()).collect(),
        field_index: c.field_index.iter().map(|(k, v)| (k.clone(), *v as u16)).collect(),
        methods: c.methods.clone(),
        default_ctor: c.default_ctor,
    }).collect();
    bc.module_inits = prog.module_inits.clone();
    bc.main = prog.main;
    for id in bc.protos.len()..prog.functions.len() {
        let proto = FnCompiler::new(&prog.functions[id], bc).compile()
            .map_err(|e| format!("in function '{}': {e}", prog.functions[id].name))?;
        bc.protos.push(proto);
    }
    Ok(())
}

impl Bytecode {
    fn add_const(&mut self, v: Value) -> u32 {
        if let Value::Str(s) = &v {
            if let Some(&i) = self.str_consts.get(s) { return i; }
            self.str_consts.insert(s.clone(), self.consts.len() as u32);
        }
        self.consts.push(v);
        (self.consts.len() - 1) as u32
    }

    fn add_name(&mut self, n: &Rc<str>) -> u32 {
        if let Some(&i) = self.name_index.get(n) { return i; }
        self.names.push(n.clone());
        let i = (self.names.len() - 1) as u32;
        self.name_index.insert(n.clone(), i);
        i
    }

    fn add_type(&mut self, t: &Type) -> u32 {
        if let Some((_, i)) = self.type_index.iter().find(|(x, _)| x == t) { return *i; }
        self.types.push(t.clone());
        let i = (self.types.len() - 1) as u32;
        self.type_index.push((t.clone(), i));
        i
    }

    /// Human-readable listing of every function, for `skol disasm`.
    pub fn disassemble(&self) -> String {
        let mut out = String::new();
        for (id, p) in self.protos.iter().enumerate() {
            let _ = writeln!(out, "fn #{id} {} (arity {}, locals {}, registers {})", p.name, p.arity, p.n_locals, p.max_regs);
            for (ip, op) in p.code.iter().enumerate() {
                let note = match op {
                    Op::LoadK(_, i) => format!("  ; {}", show_const(&self.consts[*i as usize])),
                    Op::Call(f, ..) | Op::Closure(_, f, ..) | Op::BindMethod(_, _, f) =>
                        format!("  ; {}", self.protos.get(*f as usize).map(|p| p.name.as_str()).unwrap_or("?")),
                    Op::GetFieldNamed(_, _, n) | Op::SetFieldNamed(_, n, _) | Op::CallMethodNamed(n, ..) | Op::BindMethodNamed(_, _, n) =>
                        format!("  ; {}", self.names[*n as usize]),
                    Op::LoadGlobal(_, g) | Op::StoreGlobal(g, _) =>
                        format!("  ; {}", self.global_names.get(*g as usize).map(String::as_str).unwrap_or("?")),
                    Op::MakeList(_, t, ..) | Op::MakeMap(_, t, ..) | Op::Zero(_, t) =>
                        format!("  ; {}", ir::DisplayType(&self.types[*t as usize], self)),
                    Op::NewInstance(_, c) => format!("  ; {}", self.classes[*c as usize].name),
                    _ => String::new(),
                };
                let _ = writeln!(out, "  {ip:4}  {op:?}{note}");
            }
            out.push('\n');
        }
        out
    }
}

fn show_const(v: &Value) -> String {
    match v {
        Value::Str(s) => format!("{:?}", &**s),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => skol_stdlib::format::format_float(*f),
        other => format!("{other:?}"),
    }
}

/// An evaluated assignment target: containers have been computed into
/// registers, so the target can be read and written without re-evaluating.
enum Handle {
    /// A non-captured local: its register *is* the value.
    Reg(R),
    Cell(R),
    Global(u32),
    Upval(u16),
    Field(R, u16),
    FieldNamed(R, u32),
    Slot(R, R),
    /// A vector component; `vec` holds the loaded vector.
    VecComp { base: Box<Handle>, vec: R, index: u8 },
}

struct LoopCtx {
    continue_target: u32,
    breaks: Vec<usize>,
}

struct FnCompiler<'a> {
    f: &'a ir::Function,
    bc: &'a mut Bytecode,
    code: Vec<Op>,
    spans: Vec<Span>,
    span: Span,
    loops: Vec<LoopCtx>,
    n_locals: u32,
    top: u32,
    max: u32,
    error: Option<String>,
}

const MAX_REGS: u32 = u16::MAX as u32;

impl<'a> FnCompiler<'a> {
    fn new(f: &'a ir::Function, bc: &'a mut Bytecode) -> Self {
        let n_locals = f.locals.len() as u32;
        FnCompiler {
            f, bc, code: Vec::new(), spans: Vec::new(), span: f.span, loops: Vec::new(),
            n_locals, top: n_locals, max: n_locals, error: None,
        }
    }

    fn compile(mut self) -> Result<Proto, String> {
        if self.n_locals >= MAX_REGS { return Err("too many local variables".into()); }
        for (i, l) in self.f.locals.iter().enumerate().take(self.f.params.len()) {
            if l.captured { self.emit(Op::WrapCell(i as R)); }
        }
        let body = &self.f.body;
        for s in body { self.stmt(s); }
        self.span = self.f.span;
        self.emit(Op::ReturnVoid);
        if let Some(e) = self.error { return Err(e); }
        Ok(Proto {
            name: self.f.name.clone(),
            arity: self.f.params.len() as u16,
            n_locals: self.n_locals as u16,
            max_regs: self.max.max(1) as u16,
            code: self.code.into(),
            spans: self.spans.into(),
            file: self.f.file,
            params: self.f.params.clone(),
            ret: self.f.ret.clone(),
        })
    }

    fn emit(&mut self, op: Op) -> usize {
        self.code.push(op);
        self.spans.push(self.span);
        self.code.len() - 1
    }

    fn here(&self) -> u32 { self.code.len() as u32 }

    fn patch(&mut self, at: usize) {
        let t = self.here();
        self.code[at] = match self.code[at] {
            Op::Jump(_) => Op::Jump(t),
            Op::JumpIfFalse(r, _) => Op::JumpIfFalse(r, t),
            Op::JumpIfTrue(r, _) => Op::JumpIfTrue(r, t),
            Op::CmpIJump(a, b, c, _) => Op::CmpIJump(a, b, c, t),
            Op::CmpIKJump(a, k, c, _) => Op::CmpIKJump(a, k, c, t),
            Op::ForIter(a, b, d, _) => Op::ForIter(a, b, d, t),
            Op::ForRange(a, b, d, _) => Op::ForRange(a, b, d, t),
            Op::CmpFJump(a, b, c, _) => Op::CmpFJump(a, b, c, t),
            other => other,
        };
    }

    /// Allocate `n` consecutive temporary registers.
    fn alloc_n(&mut self, n: u32) -> R {
        let r = self.top;
        self.top += n;
        if self.top >= MAX_REGS {
            if self.error.is_none() { self.error = Some("expression needs too many temporary registers".into()); }
            self.top = self.n_locals;
            return 0;
        }
        self.max = self.max.max(self.top);
        r as R
    }

    fn alloc(&mut self) -> R { self.alloc_n(1) }

    fn captured(&self, l: R) -> bool { self.f.locals[l as usize].captured }

    fn is_local_reg(&self, r: R) -> bool { (r as u32) < self.n_locals }

    // ------------------------------------------------------------ statements

    fn block(&mut self, stmts: &[ir::Stmt]) { for s in stmts { self.stmt(s); } }

    fn stmt(&mut self, s: &ir::Stmt) {
        self.span = s.span;
        self.top = self.n_locals; // temporaries never outlive a statement
        match &s.kind {
            StmtKind::Let { local, value } => {
                if self.captured(*local) {
                    // Fresh cell first, so a lambda can capture itself.
                    self.emit(Op::NewCell(*local));
                    let v = self.expr_any(value);
                    self.emit(Op::StoreCell(*local, v));
                } else {
                    self.expr_to(value, *local);
                }
            }
            StmtKind::Assign { place, op, value } => self.assign(place, *op, value),
            StmtKind::If { cond, then_body, else_body } => {
                let jf = self.cond_jump(cond);
                self.block(then_body);
                if else_body.is_empty() {
                    self.patch_all(jf);
                } else {
                    let j = self.emit(Op::Jump(0));
                    self.patch_all(jf);
                    self.block(else_body);
                    self.patch(j);
                }
            }
            StmtKind::While { cond, body } => {
                let top = self.here();
                let jf = self.cond_jump(cond);
                self.loops.push(LoopCtx { continue_target: top, breaks: vec![] });
                self.block(body);
                self.span = s.span;
                self.emit(Op::Jump(top));
                self.patch_all(jf);
                self.end_loop();
            }
            StmtKind::For { var, iter, index, iterable, body } => {
                // `for k in range(a, b)`: count in registers; no range object.
                let range = match &iterable.kind {
                    ExprKind::CallBuiltin { builtin: Builtin::Range1, args } if args.len() == 1 => Some((None, &args[0])),
                    ExprKind::CallBuiltin { builtin: Builtin::Range2, args } if args.len() == 2 => Some((Some(&args[0]), &args[1])),
                    _ => None,
                };
                match range {
                    Some((start, end)) => {
                        match start {
                            Some(a) => self.expr_to(a, *index),
                            None => { self.emit(Op::LoadInt(*index, 0)); }
                        }
                        self.expr_to(end, *iter);
                    }
                    None => {
                        self.expr_to(iterable, *iter);
                        self.emit(Op::LoadInt(*index, 0));
                    }
                }
                let elem = if self.captured(*var) { self.alloc() } else { *var };
                let top = self.here();
                let fi = if range.is_some() {
                    self.emit(Op::ForRange(*index, *iter, elem, 0))
                } else {
                    self.emit(Op::ForIter(*iter, *index, elem, 0))
                };
                if self.captured(*var) {
                    self.emit(Op::NewCell(*var));
                    self.emit(Op::StoreCell(*var, elem));
                }
                self.loops.push(LoopCtx { continue_target: top, breaks: vec![] });
                self.block(body);
                self.span = s.span;
                self.emit(Op::Jump(top));
                self.patch(fi);
                self.end_loop();
            }
            StmtKind::Return(value) => match value {
                Some(e) => { let r = self.expr_any(e); self.emit(Op::Return(r)); }
                None => { self.emit(Op::ReturnVoid); }
            },
            StmtKind::Break => {
                let j = self.emit(Op::Jump(0));
                if let Some(l) = self.loops.last_mut() { l.breaks.push(j); }
            }
            StmtKind::Continue => {
                let target = self.loops.last().map(|l| l.continue_target).unwrap_or(0);
                self.emit(Op::Jump(target));
            }
            StmtKind::Expr(e) => { self.expr_any(e); }
        }
    }

    fn end_loop(&mut self) {
        if let Some(ctx) = self.loops.pop() {
            for j in ctx.breaks { self.patch(j); }
        }
    }

    fn patch_all(&mut self, jumps: Vec<usize>) { for j in jumps { self.patch(j); } }

    /// Evaluate a condition; returns the jumps taken when it is false.
    /// Comparisons fuse with their jump, and `and` / `or` / `not` become
    /// jump chains instead of materialised booleans.
    fn cond_jump(&mut self, cond: &ir::Expr) -> Vec<usize> {
        let saved = self.span;
        if cond.span != Span::default() { self.span = cond.span; }
        let jumps = self.jumps_false(cond);
        self.span = saved;
        jumps
    }

    fn jumps_false(&mut self, cond: &ir::Expr) -> Vec<usize> {
        self.top_scope(|this| match &cond.kind {
            ExprKind::Binary(BinOp::CmpI(c), l, r) => {
                if let Some(k) = small_int(r) {
                    let a = this.expr_any(l);
                    vec![this.emit(Op::CmpIKJump(a, k, *c, 0))]
                } else {
                    let a = this.expr_any(l);
                    let b = this.expr_any(r);
                    vec![this.emit(Op::CmpIJump(a, b, *c, 0))]
                }
            }
            ExprKind::Binary(BinOp::CmpF(c), l, r) => {
                let a = this.expr_any(l);
                let b = this.expr_any(r);
                vec![this.emit(Op::CmpFJump(a, b, *c, 0))]
            }
            ExprKind::And(l, r) => {
                let mut j = this.jumps_false(l);
                j.extend(this.jumps_false(r));
                j
            }
            ExprKind::Or(l, r) => {
                let jt = this.jumps_true(l);
                let jf = this.jumps_false(r);
                this.patch_all(jt);
                jf
            }
            ExprKind::Unary(UnOp::Not, inner) => this.jumps_true(inner),
            _ => {
                let r = this.expr_any(cond);
                vec![this.emit(Op::JumpIfFalse(r, 0))]
            }
        })
    }

    /// The jumps taken when `cond` is true.
    fn jumps_true(&mut self, cond: &ir::Expr) -> Vec<usize> {
        self.top_scope(|this| match &cond.kind {
            // Integers are totally ordered, so "jump if a < b" is "jump unless a >= b".
            ExprKind::Binary(BinOp::CmpI(c), l, r) => {
                let inv = match c { Cmp::Lt => Cmp::Ge, Cmp::Le => Cmp::Gt, Cmp::Gt => Cmp::Le, Cmp::Ge => Cmp::Lt };
                if let Some(k) = small_int(r) {
                    let a = this.expr_any(l);
                    vec![this.emit(Op::CmpIKJump(a, k, inv, 0))]
                } else {
                    let a = this.expr_any(l);
                    let b = this.expr_any(r);
                    vec![this.emit(Op::CmpIJump(a, b, inv, 0))]
                }
            }
            ExprKind::Or(l, r) => {
                let mut j = this.jumps_true(l);
                j.extend(this.jumps_true(r));
                j
            }
            ExprKind::And(l, r) => {
                let jf = this.jumps_false(l);
                let jt = this.jumps_true(r);
                this.patch_all(jf);
                jt
            }
            ExprKind::Unary(UnOp::Not, inner) => this.jumps_false(inner),
            _ => {
                let r = this.expr_any(cond);
                vec![this.emit(Op::JumpIfTrue(r, 0))]
            }
        })
    }

    /// Run `f` and then free the temporaries it allocated.
    fn top_scope<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let top = self.top;
        let r = f(self);
        self.top = top;
        r
    }

    fn assign(&mut self, place: &Place, op: Option<BinOp>, value: &ir::Expr) {
        let h = self.handle(place);
        match op {
            None => {
                if let Handle::Reg(l) = h {
                    self.expr_to(value, l);
                } else {
                    let v = self.expr_any(value);
                    self.store(&h, v);
                }
            }
            Some(bop) => {
                let cur = self.load(&h);
                // `x += 1` on an int: one instruction.
                if let (BinOp::AddI | BinOp::SubI, Some(k)) = (bop, small_int(value)) {
                    let k = if bop == BinOp::SubI { k.checked_neg() } else { Some(k) };
                    if let Some(k) = k {
                        self.emit(Op::AddIK(cur, cur, k));
                        self.store(&h, cur);
                        return;
                    }
                }
                let v = self.expr_any(value);
                self.emit(binop(bop, cur, cur, v));
                self.store(&h, cur);
            }
        }
    }

    /// Evaluate the containers of an assignment target.
    fn handle(&mut self, place: &Place) -> Handle {
        match place {
            Place::Local(l) => if self.captured(*l) { Handle::Cell(*l) } else { Handle::Reg(*l) },
            Place::Global(g) => Handle::Global(*g),
            Place::Upval(u) => Handle::Upval(*u),
            Place::Field { obj, slot } => { let o = self.expr_any(obj); Handle::Field(o, *slot as u16) }
            Place::FieldNamed { obj, name } => {
                let o = self.expr_any(obj);
                let n = self.bc.add_name(name);
                Handle::FieldNamed(o, n)
            }
            Place::TableSlot { table, key } => {
                let t = self.expr_any(table);
                let k = self.expr_any(key);
                Handle::Slot(t, k)
            }
            Place::VecComp { base, index } => {
                let b = self.handle(base);
                let v = self.load(&b);
                Handle::VecComp { base: Box::new(b), vec: v, index: *index }
            }
        }
    }

    /// Register holding the target's current value (the local itself for `Reg`).
    fn load(&mut self, h: &Handle) -> R {
        match h {
            Handle::Reg(l) => *l,
            Handle::Cell(l) => { let t = self.alloc(); self.emit(Op::LoadCell(t, *l)); t }
            Handle::Global(g) => { let t = self.alloc(); self.emit(Op::LoadGlobal(t, *g)); t }
            Handle::Upval(u) => { let t = self.alloc(); self.emit(Op::LoadUpval(t, *u)); t }
            Handle::Field(o, s) => { let t = self.alloc(); self.emit(Op::GetField(t, *o, *s)); t }
            Handle::FieldNamed(o, n) => { let t = self.alloc(); self.emit(Op::GetFieldNamed(t, *o, *n)); t }
            Handle::Slot(tb, k) => { let t = self.alloc(); self.emit(Op::Table(TableOp::Get, t, *tb, *k, 0)); t }
            Handle::VecComp { vec, index, .. } => { let t = self.alloc(); self.emit(Op::VecGet(t, *vec, *index)); t }
        }
    }

    fn store(&mut self, h: &Handle, src: R) {
        match h {
            Handle::Reg(l) => { if *l != src { self.emit(Op::Move(*l, src)); } }
            Handle::Cell(l) => { self.emit(Op::StoreCell(*l, src)); }
            Handle::Global(g) => { self.emit(Op::StoreGlobal(*g, src)); }
            Handle::Upval(u) => { self.emit(Op::StoreUpval(*u, src)); }
            Handle::Field(o, s) => { self.emit(Op::SetField(*o, *s, src)); }
            Handle::FieldNamed(o, n) => { self.emit(Op::SetFieldNamed(*o, *n, src)); }
            Handle::Slot(tb, k) => { let t = self.alloc(); self.emit(Op::Table(TableOp::Set, t, *tb, *k, src)); }
            Handle::VecComp { base, vec, index } => {
                self.emit(Op::VecSet(*vec, *vec, *index, src));
                self.store(base, *vec);
            }
        }
    }

    // ------------------------------------------------------------ expressions

    /// A register holding the value of `e` (a local's own register when possible).
    fn expr_any(&mut self, e: &ir::Expr) -> R {
        if let ExprKind::Local(l) = &e.kind {
            if !self.captured(*l) { return *l; }
        }
        let t = self.alloc();
        self.expr_to(e, t);
        t
    }

    /// Compile `e` so its value ends up in register `d`. Instructions carry
    /// the expression's own source span.
    fn expr_to(&mut self, e: &ir::Expr, d: R) {
        let saved = self.span;
        if e.span != Span::default() { self.span = e.span; }
        self.expr_inner(e, d);
        self.span = saved;
    }

    fn args_at(&mut self, args: &[ir::Expr], base: R) {
        for (i, a) in args.iter().enumerate() { self.expr_to(a, base + i as R); }
    }

    fn argc(&mut self, n: usize) -> u8 {
        u8::try_from(n).unwrap_or_else(|_| {
            if self.error.is_none() { self.error = Some("too many arguments (at most 255)".into()); }
            0
        })
    }

    fn expr_inner(&mut self, e: &ir::Expr, d: R) {
        match &e.kind {
            ExprKind::Int(n) => match i32::try_from(*n) {
                Ok(small) => { self.emit(Op::LoadInt(d, small)); }
                Err(_) => { let k = self.bc.add_const(Value::Int(*n)); self.emit(Op::LoadK(d, k)); }
            },
            ExprKind::Float(x) => { let k = self.bc.add_const(Value::Float(*x)); self.emit(Op::LoadK(d, k)); }
            ExprKind::Str(s) => { let k = self.bc.add_const(Value::Str(s.clone())); self.emit(Op::LoadK(d, k)); }
            ExprKind::Bool(b) => { self.emit(Op::LoadBool(d, *b)); }
            ExprKind::Void => { self.emit(Op::LoadVoid(d)); }
            ExprKind::Local(l) => {
                if self.captured(*l) { self.emit(Op::LoadCell(d, *l)); }
                else if *l != d { self.emit(Op::Move(d, *l)); }
            }
            ExprKind::Global(g) => { self.emit(Op::LoadGlobal(d, *g)); }
            ExprKind::Upval(u) => { self.emit(Op::LoadUpval(d, *u)); }
            ExprKind::Unary(op, inner) => {
                let a = self.expr_any(inner);
                self.emit(match op { UnOp::NegI => Op::NegI(d, a), UnOp::NegF => Op::NegF(d, a), UnOp::NegV => Op::NegV(d, a), UnOp::Not => Op::Not(d, a) });
            }
            ExprKind::Binary(op, l, r) => {
                // Both sides constant (often after `const` folding): compute now.
                if let (ExprKind::Float(a), ExprKind::Float(b)) = (&l.kind, &r.kind) {
                    let v = match op {
                        BinOp::AddF => Some(a + b), BinOp::SubF => Some(a - b),
                        BinOp::MulF => Some(a * b), BinOp::DivF => Some(a / b),
                        _ => None,
                    };
                    if let Some(v) = v {
                        let k = self.bc.add_const(Value::Float(v));
                        self.emit(Op::LoadK(d, k));
                        return;
                    }
                }
                if let (ExprKind::Int(a), ExprKind::Int(b)) = (&l.kind, &r.kind) {
                    let v = match op {
                        BinOp::AddI => a.checked_add(*b), BinOp::SubI => a.checked_sub(*b),
                        BinOp::MulI => a.checked_mul(*b),
                        _ => None,
                    };
                    if let Some(v) = v {
                        match i32::try_from(v) {
                            Ok(small) => { self.emit(Op::LoadInt(d, small)); }
                            Err(_) => { let k = self.bc.add_const(Value::Int(v)); self.emit(Op::LoadK(d, k)); }
                        }
                        return;
                    }
                }
                if let (BinOp::AddI | BinOp::SubI, Some(k)) = (op, small_int(r)) {
                    let k = if *op == BinOp::SubI { k.checked_neg() } else { Some(k) };
                    if let Some(k) = k {
                        let a = self.expr_any(l);
                        self.emit(Op::AddIK(d, a, k));
                        return;
                    }
                }
                if let Some(k) = small_int(r) {
                    let fused = match op {
                        BinOp::MulI => Some(Op::MulIK(d, 0, k)),
                        BinOp::DivI if k != 0 && k != -1 => Some(Op::DivIK(d, 0, k)),
                        BinOp::ModI if k != 0 => Some(Op::ModIK(d, 0, k)),
                        BinOp::CmpI(c) => Some(Op::CmpIK(d, 0, k, *c)),
                        _ => None,
                    };
                    if let Some(op) = fused {
                        let a = self.expr_any(l);
                        self.emit(match op {
                            Op::MulIK(d, _, k) => Op::MulIK(d, a, k),
                            Op::DivIK(d, _, k) => Op::DivIK(d, a, k),
                            Op::ModIK(d, _, k) => Op::ModIK(d, a, k),
                            Op::CmpIK(d, _, k, c) => Op::CmpIK(d, a, k, c),
                            other => other,
                        });
                        return;
                    }
                }
                let a = self.expr_any(l);
                let b = self.expr_any(r);
                self.emit(binop(*op, d, a, b));
            }
            ExprKind::And(l, r) | ExprKind::Or(l, r) => {
                // Short-circuit writes the destination early, so never target a
                // local directly (the right side might read it).
                let t = if self.is_local_reg(d) { self.alloc() } else { d };
                self.expr_to(l, t);
                let j = if matches!(e.kind, ExprKind::And(..)) { self.emit(Op::JumpIfFalse(t, 0)) } else { self.emit(Op::JumpIfTrue(t, 0)) };
                self.expr_to(r, t);
                self.patch(j);
                if t != d { self.emit(Op::Move(d, t)); }
            }
            ExprKind::IntToFloat(inner) => { let a = self.expr_any(inner); self.emit(Op::IToF(d, a)); }
            ExprKind::MakeVec(items) => {
                let base = self.alloc_n(items.len() as u32);
                self.args_at(items, base);
                self.emit(Op::MakeVec(d, base, items.len() as u8));
            }
            ExprKind::MakeList(items) => {
                let base = self.alloc_n(items.len() as u32);
                self.args_at(items, base);
                let t = self.bc.add_type(&e.ty);
                self.emit(Op::MakeList(d, t, base, items.len() as u32));
            }
            ExprKind::MakeMap(pairs) => {
                let base = self.alloc_n(pairs.len() as u32 * 2);
                for (i, (k, v)) in pairs.iter().enumerate() {
                    self.expr_to(k, base + 2 * i as R);
                    self.expr_to(v, base + 2 * i as R + 1);
                }
                let t = self.bc.add_type(&e.ty);
                self.emit(Op::MakeMap(d, t, base, pairs.len() as u32));
            }
            ExprKind::Zero => { let t = self.bc.add_type(&e.ty); self.emit(Op::Zero(d, t)); }
            ExprKind::NewInstance(c) => { self.emit(Op::NewInstance(d, *c)); }
            ExprKind::NewSignal(s) => { self.emit(Op::NewSignal(d, *s)); }
            ExprKind::GetField { obj, slot } => { let o = self.expr_any(obj); self.emit(Op::GetField(d, o, *slot as u16)); }
            ExprKind::GetFieldNamed { obj, name } => {
                let o = self.expr_any(obj);
                let n = self.bc.add_name(name);
                self.emit(Op::GetFieldNamed(d, o, n));
            }
            ExprKind::VecComp { vec, index } => {
                if let ExprKind::GetField { obj, slot } = &vec.kind {
                    let o = self.expr_any(obj);
                    self.emit(Op::GetFieldComp(d, o, *slot as u16, *index));
                } else {
                    let v = self.expr_any(vec);
                    self.emit(Op::VecGet(d, v, *index));
                }
            }
            ExprKind::CallFunc { func, args } => {
                let argc = self.argc(args.len());
                let base = self.alloc_n(args.len() as u32);
                self.args_at(args, base);
                self.emit(Op::Call(*func, base, argc, d));
            }
            ExprKind::CallMethodNamed { recv, name, args } => {
                let argc = self.argc(args.len());
                let base = self.alloc_n(args.len() as u32 + 1);
                self.expr_to(recv, base);
                self.args_at(args, base + 1);
                let n = self.bc.add_name(name);
                self.emit(Op::CallMethodNamed(n, base, argc, d));
            }
            ExprKind::CallValue { callee, args } => {
                let argc = self.argc(args.len());
                let c = self.expr_any(callee);
                let base = self.alloc_n(args.len() as u32 + 1);
                self.args_at(args, base + 1);
                self.emit(Op::CallValue(c, base, argc, d));
            }
            ExprKind::CallBuiltin { builtin: Builtin::IntToFloat, args } if args.len() == 1 => {
                let a = self.expr_any(&args[0]);
                self.emit(Op::IToF(d, a));
            }
            ExprKind::CallBuiltin { builtin, args } if is_fast_builtin(*builtin) && args.len() <= 2 => {
                let a = match args.first() { Some(x) => self.expr_any(x), None => 0 };
                let b = match args.get(1) { Some(x) => self.expr_any(x), None => 0 };
                self.emit(Op::FastBuiltin(*builtin, d, a, b));
            }
            ExprKind::CallBuiltin { builtin, args } => {
                let argc = self.argc(args.len());
                let base = self.alloc_n(args.len() as u32);
                self.args_at(args, base);
                self.emit(Op::CallBuiltin(*builtin, base, argc, d));
            }
            ExprKind::Closure { func, captures } => {
                let n = self.argc(captures.len());
                let base = self.alloc_n(captures.len() as u32);
                for (i, c) in captures.iter().enumerate() {
                    match c {
                        UpvalSource::Local(l) => { self.emit(Op::Move(base + i as R, *l)); }
                        UpvalSource::Upval(u) => { self.emit(Op::LoadUpvalRaw(base + i as R, *u)); }
                    }
                }
                self.emit(Op::Closure(d, *func, base, n));
            }
            ExprKind::BindMethod { recv, func } => { let r = self.expr_any(recv); self.emit(Op::BindMethod(d, r, *func)); }
            ExprKind::BindMethodNamed { recv, name } => {
                let r = self.expr_any(recv);
                let n = self.bc.add_name(name);
                self.emit(Op::BindMethodNamed(d, r, n));
            }
            ExprKind::Table { op, table, args } => {
                let t = self.expr_any(table);
                let a = match args.first() { Some(x) => self.expr_any(x), None => 0 };
                let b = match args.get(1) { Some(x) => self.expr_any(x), None => 0 };
                self.emit(Op::Table(*op, d, t, a, b));
            }
            ExprKind::SignalListen { signal, handler } => {
                let s = self.expr_any(signal);
                let h = self.expr_any(handler);
                self.emit(Op::Listen(s, h));
                self.emit(Op::LoadVoid(d));
            }
            ExprKind::SignalEmit { signal, args } => {
                let argc = self.argc(args.len());
                let s = self.expr_any(signal);
                let base = self.alloc_n(args.len() as u32);
                self.args_at(args, base);
                self.emit(Op::Emit(s, base, argc));
                self.emit(Op::LoadVoid(d));
            }
        }
    }
}

/// Built-ins the VM evaluates inline (pure, float/vector only, can't fail).
pub fn is_fast_builtin(b: Builtin) -> bool {
    use Builtin::*;
    matches!(b, Sqrt | Sin | Cos | Tan | Atan2 | Pow | AbsF | MinF | MaxF
        | VecLength | VecNormalized | VecDot | VecDistance | VecCross
        | MinI | MaxI | ToStr | StrLen | FloatToInt)
}

fn small_int(e: &ir::Expr) -> Option<i32> {
    match e.kind { ExprKind::Int(n) => i32::try_from(n).ok(), _ => None }
}

fn binop(op: BinOp, d: R, a: R, b: R) -> Op {
    match op {
        BinOp::AddI => Op::AddI(d, a, b), BinOp::SubI => Op::SubI(d, a, b), BinOp::MulI => Op::MulI(d, a, b),
        BinOp::DivI => Op::DivI(d, a, b), BinOp::ModI => Op::ModI(d, a, b),
        BinOp::AddF => Op::AddF(d, a, b), BinOp::SubF => Op::SubF(d, a, b), BinOp::MulF => Op::MulF(d, a, b),
        BinOp::DivF => Op::DivF(d, a, b), BinOp::ModF => Op::ModF(d, a, b),
        BinOp::Concat => Op::Concat(d, a, b),
        BinOp::VecAdd => Op::VecAdd(d, a, b), BinOp::VecSub => Op::VecSub(d, a, b), BinOp::VecMul => Op::VecMul(d, a, b),
        BinOp::VecScale => Op::VecScale(d, a, b), BinOp::ScaleVec => Op::ScaleVec(d, a, b), BinOp::VecDiv => Op::VecDiv(d, a, b),
        BinOp::CmpI(c) => Op::CmpI(d, a, b, c), BinOp::CmpF(c) => Op::CmpF(d, a, b, c), BinOp::CmpS(c) => Op::CmpS(d, a, b, c),
        BinOp::Eq => Op::Eq(d, a, b), BinOp::Ne => Op::Ne(d, a, b),
    }
}
